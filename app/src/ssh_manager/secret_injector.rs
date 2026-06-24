//! Automatic SSH password/passphrase injection. Subscribes to the PTY output broadcast of a terminal pane,
//! and upon matching `password:` or `passphrase:` prompts at line end, **one-time** writes secret + `\n`.
//!
//! ## Key Design Trade-offs
//!
//! - **8KB sliding window + strict line-end matching**: regex `(?im)(password|passphrase)[^\n]*:\s*$`
//!   only matches at line end (avoids false positives from "password" in motd/banner) + sliding window ensures memory bound.
//!
//! - **15s timeout**: typical SSH pubkey negotiation < 2s, password prompt < 5s. 15s is a reasonable upper limit
//!   for pubkey auth failure + fallback to password. **Boundary for pubkey-based passwordless login**
//!   (authorized_keys configured + we also have password stored): successful pubkey handshake → no prompt appears
//!   → injector silently times out and exits, **won't mistakenly inject into post-login shell**.
//!
//! - **One-time trigger**: immediately break after match, injector future exits → InactiveReceiver
//!   drops → subsequent PTY stream no longer seen by this injector, **prevents double injection**.
//!
//! - **bytes::Regex**: PTY output may contain incomplete UTF-8 bytes, using `regex::bytes` is safe.

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::InactiveReceiver;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};
use zeroize::Zeroizing;

use crate::ssh_manager::password_prompt::bytes_look_like_password_prompt;
use crate::terminal::TerminalView;

/// Injection timeout upper limit.
const INJECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Sliding window retains this many bytes of PTY output for regex matching.
const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
/// When buffer exceeds this value, drain to sliding window size.
const BUFFER_HARD_LIMIT: usize = 16 * 1024;

/// Spawn a one-time injection task in the owner=Workspace context. The task is automatically
/// cancelled when Workspace is dropped; owner doesn't need to abort.
///
/// Prerequisite: `pty_reads_rx` is obtained from `terminal_view.inactive_pty_reads_rx(ctx)`.
/// The future only actually runs when Some; wasm / remote session gets None and directly no-ops.
pub fn spawn_password_injector<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    secret: Zeroizing<String>,
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

    // Set in-flight to true immediately, notifying OneKey listener not to show menu
    // before this injection completes. This way, regardless of whether injector finishes first
    // or OneKey sees the bytes first, the semantics are consistent: **injector has priority**.
    if let Some(view) = terminal_view.upgrade(ctx) {
        view.update(ctx, |view, _| {
            view.set_ssh_secret_auto_injection_in_flight(true);
        });
    }

    let owned_secret = secret.clone();
    let future = async move {
        match watch_for_prompt(rx).with_timeout(INJECT_TIMEOUT).await {
            Ok(true) => Some(owned_secret),
            Ok(false) | Err(_) => None, // EOF or timeout → no-op
        }
    };
    ctx.spawn(future, move |_owner, secret_opt, ctx| {
        let Some(view) = terminal_view.upgrade(ctx) else {
            log::debug!("ssh secret injector: terminal view dropped before injection");
            return;
        };
        let Some(secret) = secret_opt else {
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

/// Async loop: consumes PTY broadcast, appends to sliding window, **returns true as soon as regex matches line-end prompt**;
/// returns false on EOF. Timeout is wrapped by caller via `with_timeout`.
async fn watch_for_prompt(rx: InactiveReceiver<Arc<Vec<u8>>>) -> bool {
    let mut active = rx.activate_cloned();
    let mut buf: Vec<u8> = Vec::with_capacity(SLIDING_WINDOW_BYTES);
    while let Ok(chunk) = active.recv().await {
        buf.extend_from_slice(&chunk);
        if buf.len() > BUFFER_HARD_LIMIT {
            let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
            buf.drain(..drop_n);
        }
        if bytes_look_like_password_prompt(&buf) {
            return true;
        }
    }
    false
}
