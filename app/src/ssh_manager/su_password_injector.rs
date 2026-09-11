//! su password confirmation prompt. A locally submitted `su` command targeting root arms one
//! short-lived attempt. PTY output can consume that attempt only when a password prompt is detected,
//! then displays a confirmation menu for the stored root password or a OneKey password.
//!
//! Only injects for root targets; switching to other users like `su lg` does not trigger this.
//! Waits for shell prompt to appear first (indicating SSH login is complete) before starting detection,
//! avoiding conflicts with login password prompts. Uses `spawn_stream_local` + `stream!` for continuous
//! monitoring. Remote-controlled PTY output never arms the helper.

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::{InactiveReceiver, Receiver};
use async_stream::stream;
use futures_lite::Stream;
use instant::Instant;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use regex::bytes::Regex;
use warp_core::SessionId;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};
use zeroize::Zeroizing;

use crate::ssh_manager::shell_prompt::bytes_look_like_shell_prompt;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::TerminalView;

const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
const BUFFER_HARD_LIMIT: usize = 16 * 1024;
/// Phase 1 maximum wait duration for shell prompt. Times out and abandons the entire stream
/// (resets in_flight in `on_done`).
const SHELL_READY_TIMEOUT: Duration = Duration::from_secs(30);
/// A genuine `su` password prompt follows command submission immediately. Keeping the local grant
/// short-lived prevents unrelated later prompts from consuming it.
const SU_ROOT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellReadyOutcome {
    Ready,
    EndOfStream,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuInjectorEvent {
    ShellReadyFinished(ShellReadyOutcome),
    PasswordPrompt,
}

#[derive(Clone, Copy, Debug)]
struct ArmedSuRootAttempt {
    generation: u64,
    session_id: SessionId,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct SuRootAttemptGuard {
    armed: Option<ArmedSuRootAttempt>,
    next_generation: u64,
}

impl SuRootAttemptGuard {
    fn observe_command(
        &mut self,
        command: &str,
        session_id: SessionId,
        is_local_user_command: bool,
        now: Instant,
    ) -> Option<u64> {
        self.clear();
        if is_local_user_command && is_su_to_root(command) {
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1);
            self.armed = Some(ArmedSuRootAttempt {
                generation,
                session_id,
                expires_at: now + SU_ROOT_ATTEMPT_TIMEOUT,
            });
            Some(generation)
        } else {
            None
        }
    }

    fn consume(&mut self, now: Instant) -> bool {
        self.armed
            .take()
            .is_some_and(|attempt| now <= attempt.expires_at)
    }

    fn clear_if_session_changed(&mut self, active_session_id: Option<SessionId>) {
        if self
            .armed
            .is_some_and(|attempt| Some(attempt.session_id) != active_session_id)
        {
            self.clear();
        }
    }

    fn expire(&mut self, generation: u64) {
        if self
            .armed
            .is_some_and(|attempt| attempt.generation == generation)
        {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.armed = None;
    }
}

impl SuInjectorEvent {
    fn releases_onekey_suppression(self) -> bool {
        matches!(self, Self::ShellReadyFinished(_))
    }
}

lazy_static! {
    /// Password prompt regex — strictly matches two types:
    /// 1. `password` / `passphrase` (and the localized terms in the regex) with a half-width `:` or full-width `：` colon at line end
    /// 2. Kylin Galaxy V10's colon-less localized "enter password" prompt (see the regex)
    ///
    /// Old implementation made colon optional; any line ending with "password" (e.g.,
    /// `Your password has expired`) would be a false positive.
    static ref PASSWORD_PROMPT_REGEX: Regex = Regex::new(
        r"(?im)(?:(?:password|passphrase|密码)[^\n]*(?::|：)\s*$|输入密码\s*$)"
    )
    .expect("su password prompt regex must compile");

}

/// Spawn su password continuous monitoring stream on the owner context.
pub fn spawn_su_password_injector<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    root_password: Option<Zeroizing<String>>,
    ctx: &mut ViewContext<O>,
) where
    O: warpui::View + 'static,
{
    let Some(rx) = pty_reads_rx else {
        log::debug!("ssh su password injector: no pty_reads_rx — skip");
        return;
    };
    let Some(root_password) = root_password.filter(|password| !password.is_empty()) else {
        log::debug!("ssh su password injector: empty root password - skip");
        return;
    };
    let Some(view) = terminal_view.upgrade(ctx) else {
        log::debug!("ssh su password injector: terminal view dropped before setup");
        return;
    };

    // Set in-flight flag to prevent OneKey credential dialog from appearing while waiting for shell prompt.
    view.update(ctx, |view, _| {
        view.set_ssh_secret_auto_injection_in_flight(true);
    });

    let attempt_guard = Arc::new(Mutex::new(SuRootAttemptGuard::default()));
    let attempt_guard_for_events = attempt_guard.clone();
    ctx.subscribe_to_view(
        &view,
        move |_owner, terminal_view, event, ctx| match event {
            crate::terminal::Event::ExecuteCommand(event) => {
                let generation = attempt_guard_for_events.lock().observe_command(
                    &event.command,
                    event.session_id,
                    matches!(&event.source, CommandExecutionSource::User),
                    Instant::now(),
                );
                if let Some(generation) = generation {
                    let attempt_guard = attempt_guard_for_events.clone();
                    ctx.spawn(
                        warpui::r#async::Timer::after(SU_ROOT_ATTEMPT_TIMEOUT),
                        move |_owner, (), _ctx| {
                            attempt_guard.lock().expire(generation);
                        },
                    );
                }
            }
            crate::terminal::Event::TerminalViewStateChanged => {
                attempt_guard_for_events
                    .lock()
                    .clear_if_session_changed(terminal_view.as_ref(ctx).active_block_session_id());
            }
            crate::terminal::Event::WriteBytesToPty { bytes }
                if bytes.contains(&warp_terminal::model::escape_sequences::C0::ETX)
                    || bytes.contains(&warp_terminal::model::escape_sequences::C0::EOT) =>
            {
                attempt_guard_for_events.lock().clear();
            }
            crate::terminal::Event::BlockCompleted { .. }
            | crate::terminal::Event::BlockListCleared
            | crate::terminal::Event::CtrlD
            | crate::terminal::Event::ShutdownPty
            | crate::terminal::Event::CloseRequested
            | crate::terminal::Event::Exited => attempt_guard_for_events.lock().clear(),
            _ => {}
        },
    );

    let prompt_stream = su_prompt_events(rx, SHELL_READY_TIMEOUT, attempt_guard);

    // on_done remains a final safety net for task abortion or owner teardown. Normal Phase 1
    // completion also emits ShellReadyFinished, so suppression is released immediately at the
    // phase boundary; the final safety write is deliberately idempotent.
    let terminal_view_done = terminal_view.clone();
    let _ = ctx.spawn_stream_local(
        prompt_stream,
        move |_owner, event, ctx| {
            let Some(view) = terminal_view.upgrade(ctx) else {
                return;
            };
            view.update(ctx, |view, ctx| {
                if event.releases_onekey_suppression() {
                    view.set_ssh_secret_auto_injection_in_flight(false);
                }
                match event {
                    SuInjectorEvent::ShellReadyFinished(_) => {}
                    SuInjectorEvent::PasswordPrompt => {
                        view.su_root_password = Some(root_password.clone());
                        view.show_su_root_confirm_menu(ctx);
                    }
                }
            });
        },
        move |_owner, ctx| {
            if let Some(view) = terminal_view_done.upgrade(ctx) {
                view.update(ctx, |view, _| {
                    view.set_ssh_secret_auto_injection_in_flight(false);
                });
            }
        },
    );
}

fn su_prompt_events(
    rx: InactiveReceiver<Arc<Vec<u8>>>,
    shell_ready_timeout: Duration,
    attempt_guard: Arc<Mutex<SuRootAttemptGuard>>,
) -> impl Stream<Item = SuInjectorEvent> {
    stream! {
        let mut active = rx.activate_cloned();
        let mut buf: Vec<u8> = Vec::with_capacity(SLIDING_WINDOW_BYTES);

        // Phase 1 uses one absolute deadline around the whole receive loop. Continuous
        // non-prompt output cannot reset or extend the wait.
        let shell_ready = wait_for_shell_ready(&mut active, &mut buf, shell_ready_timeout).await;
        match shell_ready {
            ShellReadyOutcome::Ready => {
                log::debug!("ssh su password injector: shell ready; monitoring su prompts");
            }
            ShellReadyOutcome::EndOfStream => {
                log::debug!("ssh su password injector: PTY stream ended before shell ready");
            }
            ShellReadyOutcome::TimedOut => {
                log::warn!(
                    "ssh su password injector: shell was not ready within {shell_ready_timeout:?}"
                );
            }
        }
        yield SuInjectorEvent::ShellReadyFinished(shell_ready);
        if !matches!(shell_ready, ShellReadyOutcome::Ready) {
            return;
        }

        // Phase 2: Continuously detect su root + password prompt, continue listening after each yield
        buf.clear();
        while let Ok(chunk) = active.recv().await {
            buf.extend_from_slice(&chunk);
            if buf.len() > BUFFER_HARD_LIMIT {
                let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
                buf.drain(..drop_n);
            }
            if PASSWORD_PROMPT_REGEX.is_match(&buf) {
                buf.clear();
                let authorized = attempt_guard.lock().consume(Instant::now());
                if authorized {
                    yield SuInjectorEvent::PasswordPrompt;
                }
            }
        }
    }
}

async fn wait_for_shell_ready(
    active: &mut Receiver<Arc<Vec<u8>>>,
    buf: &mut Vec<u8>,
    timeout: Duration,
) -> ShellReadyOutcome {
    let receive_until_ready = async {
        while let Ok(chunk) = active.recv().await {
            buf.extend_from_slice(&chunk);
            if buf.len() > BUFFER_HARD_LIMIT {
                let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
                buf.drain(..drop_n);
            }
            if bytes_look_like_shell_prompt(buf) {
                return true;
            }
        }
        false
    };

    match receive_until_ready.with_timeout(timeout).await {
        Ok(true) => ShellReadyOutcome::Ready,
        Ok(false) => ShellReadyOutcome::EndOfStream,
        Err(_) => ShellReadyOutcome::TimedOut,
    }
}

/// Recognizes the deliberately small set of direct root-targeting `su` commands supported by the
/// helper. Shell pipelines, command lists, wrappers other than `sudo`, and non-root targets are
/// rejected so an unrelated local command cannot arm a hostile PTY prompt.
fn is_su_to_root(command: &str) -> bool {
    let mut words = command.split_ascii_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let su_word = if first == "sudo" {
        words.next()
    } else {
        Some(first)
    };
    if !su_word.is_some_and(|word| word == "su" || word.ends_with("/su")) {
        return false;
    }

    let mut target_seen = false;
    for word in words {
        match word {
            "-" | "-l" | "--login" if !target_seen => {}
            "root" if !target_seen => target_seen = true,
            _ => return false,
        }
    }
    true
}

pub(crate) fn should_spawn_su_password_injector(root_password: Option<&Zeroizing<String>>) -> bool {
    root_password.is_some_and(|password| !password.is_empty())
}

#[cfg(test)]
#[path = "su_password_injector_tests.rs"]
mod tests;
