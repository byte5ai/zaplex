//! Auto-execute startup command after successful SSH connection. Wait for shell prompt,
//! then write startup_command to PTY + `\n`, inject once and exit.

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::InactiveReceiver;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};

use crate::ssh_manager::shell_prompt::bytes_look_like_shell_prompt;
use crate::terminal::TerminalView;

const INJECT_TIMEOUT: Duration = Duration::from_secs(30);
const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
const BUFFER_HARD_LIMIT: usize = 16 * 1024;

#[derive(Debug, Eq, PartialEq)]
enum StartupCommandWaitOutcome {
    Ready(String),
    EndOfStream,
    TimedOut,
}

/// Spawn startup command injector task in owner context.
pub fn spawn_startup_command_injector<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    startup_command: String,
    ctx: &mut ViewContext<O>,
) where
    O: warpui::View + 'static,
{
    let Some(rx) = pty_reads_rx else {
        log::debug!("ssh startup command injector: no pty_reads_rx — skip");
        return;
    };
    if startup_command.is_empty() {
        log::debug!("ssh startup command injector: empty command — skip");
        return;
    }

    let future = wait_for_startup_command(rx, startup_command, INJECT_TIMEOUT);
    ctx.spawn(future, move |_owner, outcome, ctx| {
        let Some(view) = terminal_view.upgrade(ctx) else {
            log::debug!("ssh startup command injector: terminal view dropped");
            return;
        };
        let cmd = match outcome {
            StartupCommandWaitOutcome::Ready(cmd) => cmd,
            StartupCommandWaitOutcome::EndOfStream => {
                log::debug!("ssh startup command injector: PTY stream ended before shell ready");
                return;
            }
            StartupCommandWaitOutcome::TimedOut => {
                log::warn!(
                    "ssh startup command injector: shell was not ready within {INJECT_TIMEOUT:?}"
                );
                return;
            }
        };
        view.update(ctx, |view, ctx| {
            let mut bytes = cmd.as_bytes().to_vec();
            bytes.push(b'\n');
            view.write_to_pty(bytes, ctx);
        });
    });
}

async fn wait_for_startup_command(
    rx: InactiveReceiver<Arc<Vec<u8>>>,
    startup_command: String,
    timeout: Duration,
) -> StartupCommandWaitOutcome {
    match wait_for_shell_prompt(rx).with_timeout(timeout).await {
        Ok(true) => StartupCommandWaitOutcome::Ready(startup_command),
        Ok(false) => StartupCommandWaitOutcome::EndOfStream,
        Err(_) => StartupCommandWaitOutcome::TimedOut,
    }
}

async fn wait_for_shell_prompt(rx: InactiveReceiver<Arc<Vec<u8>>>) -> bool {
    let mut active = rx.activate_cloned();
    let mut buf: Vec<u8> = Vec::with_capacity(SLIDING_WINDOW_BYTES);
    while let Ok(chunk) = active.recv().await {
        buf.extend_from_slice(&chunk);
        if buf.len() > BUFFER_HARD_LIMIT {
            let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
            buf.drain(..drop_n);
        }
        if bytes_look_like_shell_prompt(&buf) {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "startup_command_injector_tests.rs"]
mod tests;
