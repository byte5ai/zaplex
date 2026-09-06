//! su password confirmation prompt. Continuously monitors PTY output; when a password prompt is detected
//! after the user enters `su root` / `su - root` or similar commands to switch to root, displays a confirmation
//! menu, allowing the user to inject the root password or share the OneKey password.
//!
//! Only injects for root targets; switching to other users like `su lg` does not trigger this.
//! Waits for shell prompt to appear first (indicating SSH login is complete) before starting detection,
//! avoiding conflicts with login password prompts. Uses `spawn_stream_local` + `stream!` for continuous
//! monitoring; triggers on every `su root` command.

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::{InactiveReceiver, Receiver};
use async_stream::stream;
use futures_lite::Stream;
use lazy_static::lazy_static;
use regex::bytes::Regex;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};
use zeroize::Zeroizing;

use crate::ssh_manager::shell_prompt::bytes_look_like_shell_prompt;
use crate::terminal::TerminalView;

const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
const BUFFER_HARD_LIMIT: usize = 16 * 1024;
/// Phase 1 maximum wait duration for shell prompt. Times out and abandons the entire stream
/// (resets in_flight in `on_done`).
const SHELL_READY_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// su command regex — matches su commands targeting root (at line end):
    /// `su` / `su -` / `su -l` / `su --login` / `su root` / `su - root` /
    /// `su -l root` / `su --login root`. Does not match forms like `su lg` / `su - lg` switching
    /// to other users; `sudo su` still matches the trailing `su` due to word boundary `\bsu`.
    static ref SU_ROOT_CMD_REGEX: Regex =
        Regex::new(r"(?m)\bsu(?:\s+(?:-l?|--login|-))*(?:\s+root)?\s*$")
            .expect("su root cmd regex must compile");
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
    // Set in-flight flag to prevent OneKey credential dialog from appearing while waiting for shell prompt.
    if let Some(view) = terminal_view.upgrade(ctx) {
        view.update(ctx, |view, _| {
            view.set_ssh_secret_auto_injection_in_flight(true);
        });
    }

    let prompt_stream = su_prompt_events(rx, SHELL_READY_TIMEOUT);

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
            if PASSWORD_PROMPT_REGEX.is_match(&buf) && is_su_to_root(&buf) {
                buf.clear();
                yield SuInjectorEvent::PasswordPrompt;
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

/// Check if buffer contains a su command targeting root.
fn is_su_to_root(buf: &[u8]) -> bool {
    SU_ROOT_CMD_REGEX.is_match(buf)
}

pub(crate) fn should_spawn_su_password_injector(root_password: Option<&Zeroizing<String>>) -> bool {
    root_password.is_some_and(|password| !password.is_empty())
}

#[cfg(test)]
#[path = "su_password_injector_tests.rs"]
mod tests;
