//! Automatic SSH password injection. Subscribes to the PTY output broadcast of a terminal pane,
//! and upon matching a pre-shell `password:` prompt at line end, **one-time** writes secret + `\n`.
//!
//! ## Key Design Trade-offs
//!
//! - **8KB sliding window + strict whole-line matching**: only recognized OpenSSH, PAM, sudo,
//!   and key-passphrase prompt forms match, avoiding banner lines that merely end in `password:`.
//!   The sliding window keeps memory bounded.
//!
//! - **Shell boundary**: seeing a shell prompt permanently disarms the watcher before it considers later output,
//!   so a post-login `sudo` prompt cannot consume the SSH password.
//!
//! - **Key authentication**: private-key passphrases are never injected into the PTY. They use the
//!   `SSH_ASKPASS` launcher prepared by `warp_ssh_manager`.
//!
//! - **One-time trigger**: immediately break after match, injector future exits → InactiveReceiver
//!   drops → subsequent PTY stream no longer seen by this injector, **prevents double injection**.
//!
//! - **bytes::Regex**: PTY output may contain incomplete UTF-8 bytes, using `regex::bytes` is safe.

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::{InactiveReceiver, Receiver};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use warp_core::SessionId;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};
use zeroize::Zeroizing;

use crate::ssh_manager::password_prompt::bytes_look_like_password_prompt;
use crate::ssh_manager::shell_prompt::bytes_look_like_shell_prompt;
use crate::terminal::{Event, TerminalView};
use warp_ssh_manager::{AuthType, PreparedSshCommand};

/// Injection timeout upper limit.
const INJECT_TIMEOUT: Duration = Duration::from_secs(15);
const ASKPASS_LIFETIME_TIMEOUT: Duration = Duration::from_secs(30);
/// Sliding window retains this many bytes of PTY output for regex matching.
const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
/// When buffer exceeds this value, drain to sliding window size.
const BUFFER_HARD_LIMIT: usize = 16 * 1024;

/// One automatic write belongs to exactly one SSH launch. A later command or
/// cancellation must never redirect it into the terminal's next shell.
#[derive(Default)]
pub(crate) struct SshInjectionAttempt {
    session: Option<SessionId>,
    finished: bool,
}

impl SshInjectionAttempt {
    fn observe_command(&mut self, expected: &str, command: &str, session: SessionId) {
        if self.finished {
            return;
        }
        if self.session.is_none() && command == expected {
            self.session = Some(session);
        } else {
            self.finished = true;
        }
    }

    fn observe_completion(&mut self, session: Option<SessionId>, executed: bool, background: bool) {
        if executed && !background && self.session.is_some() && self.session == session {
            self.finished = true;
        }
    }

    fn observe_input(&mut self, bytes: &[u8]) {
        if bytes.contains(&warp_terminal::model::escape_sequences::C0::ETX)
            || bytes.contains(&warp_terminal::model::escape_sequences::C0::EOT)
        {
            self.finished = true;
        }
    }

    pub(crate) fn finish(&mut self) -> bool {
        let authorized = !self.finished && self.session.is_some();
        self.finished = true;
        authorized
    }
}

pub(crate) fn watch_injection_attempt<O: warpui::View + 'static>(
    view: &warpui::ViewHandle<TerminalView>,
    expected_ssh_command: String,
    pty_reads_rx: InactiveReceiver<Arc<Vec<u8>>>,
    releases_password_suppression: bool,
    ctx: &mut ViewContext<O>,
) -> (
    Arc<Mutex<SshInjectionAttempt>>,
    oneshot::Receiver<Receiver<Arc<Vec<u8>>>>,
    oneshot::Receiver<()>,
) {
    let (cancel_sender, cancel_receiver) = oneshot::channel();
    let mut cancel_sender = Some(cancel_sender);
    let (start_sender, start_receiver) = oneshot::channel();
    let mut start_sender = Some(start_sender);
    let mut pty_reads_rx = Some(pty_reads_rx);
    let attempt = Arc::new(Mutex::new(SshInjectionAttempt::default()));
    let weak_attempt = Arc::downgrade(&attempt);
    ctx.subscribe_to_view(view, move |_owner, view, event, ctx| {
        let Some(attempt) = weak_attempt.upgrade() else {
            return;
        };
        let mut attempt = attempt.lock();
        if attempt.finished {
            return;
        }
        match event {
            Event::ExecuteCommand(command) => {
                attempt.observe_command(
                    &expected_ssh_command,
                    &command.command,
                    command.session_id,
                );
                if !attempt.finished && attempt.session.is_some() {
                    if let (Some(sender), Some(rx)) = (start_sender.take(), pty_reads_rx.take()) {
                        // Activate at the authorized launch event, before yielding to the
                        // worker. Earlier local/bootstrap output is never prompt evidence.
                        let _ = sender.send(rx.activate());
                    }
                }
            }
            Event::BlockCompleted { block, .. } => {
                attempt.observe_completion(
                    block.session_id,
                    block.did_execute,
                    block.is_background,
                );
            }
            Event::WriteBytesToPty { bytes } => attempt.observe_input(bytes),
            Event::BlockListCleared
            | Event::CtrlD
            | Event::ShutdownPty
            | Event::CloseRequested
            | Event::Exited => attempt.finished = true,
            _ => {}
        }
        let cancelled = attempt.finished;
        if cancelled {
            if let Some(sender) = cancel_sender.take() {
                let _ = sender.send(());
            }
            start_sender.take();
            pty_reads_rx.take();
        }
        drop(attempt);
        if cancelled && releases_password_suppression {
            view.update(ctx, |view, _| {
                view.set_ssh_secret_auto_injection_in_flight(false)
            });
        }
    });
    (attempt, start_receiver, cancel_receiver)
}

/// Spawn a one-time injection task in the owner=Workspace context. The task is automatically
/// cancelled when Workspace is dropped; owner doesn't need to abort.
///
/// Prerequisite: `pty_reads_rx` is obtained from `terminal_view.inactive_pty_reads_rx(ctx)`.
/// The future only actually runs when Some; wasm / remote session gets None and directly no-ops.
pub fn spawn_password_injector<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    secret: Zeroizing<String>,
    effective_auth_type: AuthType,
    expected_ssh_command: String,
    ctx: &mut ViewContext<O>,
) where
    O: warpui::View + 'static,
{
    let Some(rx) = pty_reads_rx else {
        log::debug!("ssh secret injector: no pty_reads_rx (non-local session) — skip");
        return;
    };
    if secret.is_empty() {
        log::debug!("ssh secret injector: empty secret — skip");
        return;
    }
    match effective_auth_type {
        AuthType::Password => {}
        AuthType::Key | AuthType::OneKey => {
            log::debug!("ssh secret injector: PTY injection disabled for non-password auth");
            return;
        }
    }

    // Set in-flight to true immediately, notifying OneKey listener not to show menu
    // before this injection completes. This way, regardless of whether injector finishes first
    // or OneKey sees the bytes first, the semantics are consistent: **injector has priority**.
    let Some(view) = terminal_view.upgrade(ctx) else {
        return;
    };
    let (attempt, started, cancelled) =
        watch_injection_attempt(&view, expected_ssh_command, rx, true, ctx);
    view.update(ctx, |view, _| {
        view.set_ssh_secret_auto_injection_in_flight(true);
    });

    let owned_secret = secret.clone();
    let future = async move {
        let watch = async move {
            let Ok(rx) = started.await else {
                return false;
            };
            watch_for_prompt(rx, effective_auth_type).await
        };
        let cancelled = async move {
            let _ = cancelled.await;
            false
        };
        match futures_lite::future::race(watch, cancelled)
            .with_timeout(INJECT_TIMEOUT)
            .await
        {
            Ok(true) => Some(owned_secret),
            Ok(false) | Err(_) => None, // EOF or timeout → no-op
        }
    };
    ctx.spawn(future, move |_owner, secret_opt, ctx| {
        let Some(view) = terminal_view.upgrade(ctx) else {
            log::debug!("ssh secret injector: terminal view dropped before injection");
            return;
        };
        let mut pending = attempt.lock();
        if pending.finished {
            return;
        }
        let authorized = pending.finish();
        drop(pending);
        let Some(secret) = secret_opt.filter(|_| authorized) else {
            log::debug!("ssh secret injector: no prompt seen within timeout");
            view.update(ctx, |view, _| {
                view.set_ssh_secret_auto_injection_in_flight(false);
            });
            return;
        };
        view.update(ctx, |view, ctx| {
            // Write password + newline as bytes to PTY, equivalent to simulating keyboard keystrokes in response to interactive prompt.
            // At this point SSH is already running (bootstrap completed long ago), write_to_pty direct write is the right approach.
            let mut bytes = secret.as_bytes().to_vec();
            bytes.push(b'\n');
            view.write_to_pty(bytes, ctx);
            view.note_ssh_secret_auto_injected(ctx);
            view.set_ssh_secret_auto_injection_in_flight(false);
        });
    });
}

/// Keep the temporary askpass files alive until SSH reaches a shell prompt or the bounded
/// authentication window expires. Dropping the guard deletes the secret, helper, and launcher.
pub fn retain_key_askpass_until_shell_ready<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    prepared_command: PreparedSshCommand,
    expected_ssh_command: String,
    ctx: &mut ViewContext<O>,
) where
    O: warpui::View + 'static,
{
    let Some(view) = terminal_view.upgrade(ctx) else {
        return;
    };
    let watch =
        pty_reads_rx.map(|rx| watch_injection_attempt(&view, expected_ssh_command, rx, false, ctx));
    let future = async move {
        match watch {
            Some((attempt, started, cancelled)) => {
                wait_for_askpass_release(started, cancelled, ASKPASS_LIFETIME_TIMEOUT).await;
                drop(attempt);
            }
            None => {
                warpui::r#async::Timer::after(ASKPASS_LIFETIME_TIMEOUT).await;
            }
        }
        drop(prepared_command);
    };
    ctx.spawn(future, |_owner, (), _ctx| {});
}

async fn wait_for_askpass_release(
    started: oneshot::Receiver<Receiver<Arc<Vec<u8>>>>,
    cancelled: oneshot::Receiver<()>,
    timeout: Duration,
) {
    let ready = async move {
        if let Ok(rx) = started.await {
            wait_for_shell_prompt(rx).await;
        }
    };
    let cancelled = async move {
        let _ = cancelled.await;
    };
    // The same bound includes the local shell bootstrap and remote authentication.
    let _ = futures_lite::future::race(ready, cancelled)
        .with_timeout(timeout)
        .await;
}

async fn wait_for_shell_prompt(mut active: Receiver<Arc<Vec<u8>>>) {
    let mut buf = Vec::with_capacity(SLIDING_WINDOW_BYTES);
    while let Ok(chunk) = active.recv().await {
        buf.extend_from_slice(&chunk);
        if buf.len() > BUFFER_HARD_LIMIT {
            let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
            buf.drain(..drop_n);
        }
        if bytes_look_like_shell_prompt(&buf) {
            return;
        }
    }
}

/// Async loop: consumes PTY broadcast, appends to sliding window, **returns true as soon as regex matches line-end prompt**;
/// returns false on EOF. Timeout is wrapped by caller via `with_timeout`.
async fn watch_for_prompt(
    mut active: Receiver<Arc<Vec<u8>>>,
    effective_auth_type: AuthType,
) -> bool {
    match effective_auth_type {
        AuthType::Password => {}
        AuthType::Key | AuthType::OneKey => return false,
    }
    let mut buf: Vec<u8> = Vec::with_capacity(SLIDING_WINDOW_BYTES);
    while let Ok(chunk) = active.recv().await {
        buf.extend_from_slice(&chunk);
        if buf.len() > BUFFER_HARD_LIMIT {
            let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
            buf.drain(..drop_n);
        }
        if bytes_look_like_shell_prompt(&buf) {
            return false;
        }
        if bytes_look_like_password_prompt(&buf) {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "secret_injector_tests.rs"]
mod tests;
