# Stage 2 Design — Client Attach + Block/Grid + Mouse

> **Status:** Design + increment 1 in progress, created 2026-06-25.
> **Parent:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) §6 + [Stage 1 spec](2026-06-24-stage1-session-host-design.md).
> **Builds on:** Stage 1 (merged) — the daemon owns PTYs and sends `SessionOutput`/`SessionExited` pushes, accepting `OpenSession`/`AttachSession`/`SessionInput`/`ResizeSession`.

---

## 1. Goal

Make a daemon-hosted session usable as a real terminal in the client: its
output renders into the grid/blocks, keyboard **and mouse** input flow back to
the PTY, and resize propagates — all without an external multiplexer (so the SGR
mouse path works end-to-end, plan §3.5 / §6).

**Stage 2 scope:** one remote session, opened and driven from the client.
Replay-on-reconnect/persistence is Stage 3; multi-session UI/adopt is Stage 4.

---

## 2. Verified seams (from code exploration)

| Concern | Location |
|---|---|
| Client push → event | `crates/remote_server/src/client/mod.rs`: `ClientEvent` enum, `push_message_to_event()` |
| Client request/notify methods | same file (pattern: `initialize()` request, `send_buffer_edit()` notification) |
| Manager event forward | `crates/remote_server/src/manager.rs`: `forward_client_event()` → `RemoteServerManagerEvent` |
| **ANSI feed (core insertion point)** | `app/src/terminal/model/ansi/mod.rs`: `Processor::parse_bytes(&mut handler, bytes, writer)` — `TerminalModel` is the `Handler` |
| Local byte-flow reference | `app/src/terminal/local_tty/event_loop.rs` (feeds `parser.parse_bytes(&mut model, bytes, &mut sink/pty)`) |
| Remote TTY scaffold | `app/src/terminal/remote_tty/terminal_manager.rs` (async-channel eventloop) |
| Session type | `app/src/terminal/model/session.rs` (`SessionType` / `BootstrapSessionType`) |
| User input + mouse → PTY | `app/src/terminal/writeable_pty/pty_controller.rs` (`write_user_bytes_to_pty`), `app/src/terminal/alt_screen/mod.rs` (`should_intercept_mouse`) |

**Core insertion point:** feed `SessionOutput.bytes` into `Processor::parse_bytes(&mut terminal_model, &bytes, &mut io::sink())` — identical to the local path, but with `io::sink()` as the "writer" (no local echo; the shell runs on the daemon). Grid/blocks update immediately; trigger a redraw wakeup.

---

## 3. Increments

1. **Client protocol layer** *(this PR)* — `ClientEvent::SessionOutput`/`SessionExited`, `push_message_to_event` arms, and client methods `open_session`/`attach_session`/`send_session_input`/`send_resize_session`/`send_detach_session`. Isolated to `remote_server`; `cargo check -p remote_server` green. `forward_client_event` has a placeholder arm (no app consumer yet).
2. **Manager → app events** — add `RemoteServerManagerEvent::SessionOutput`/`SessionExited`, emit them from `forward_client_event`; handle the new variants in app subscribers.
3. **Remote terminal byte source** — a `remote_tty` byte sink that feeds `SessionOutput` bytes into the terminal's `ansi::Processor` (`parse_bytes` + redraw). Open a session via `open_session`, route the model's bytes from the manager events.
4. **Input + resize + mouse** — route `write_user_bytes_to_pty` for a remote session to `client.send_session_input`; resize → `send_resize_session`; ensure `should_intercept_mouse` lets SGR mouse reports flow to `send_session_input` (no multiplexer nesting → the Warp tmux mouse bug is structurally absent).
5. **Session lifecycle** — `SessionExited` → close/mark the terminal; a session-type path (`session.rs`) that selects the attached-remote source instead of a local PTY.

---

## 4. Notes / risks

- **Mouse:** the whole point — because the daemon owns the real PTY and there is no multiplexer between, SGR mouse (`DECSET 1006/1000/1002/1003`) round-trips through the normal ansi path; mouse bytes just go back via `send_session_input`.
- **Redraw:** after `parse_bytes`, send the model's wakeup/redraw event (as the local event loop does) so the view updates.
- **Backpressure / ordering:** `SessionOutput` carries monotonic `seq`; Stage 2 consumes in arrival order (replay/seq-gap handling is Stage 3).
- **No async-model test harness** exists, so app-side increments are `cargo check`-verified + exercised via the manual `test-dispatch` job / real run; behaviour validation of the full path is via running the app (Stage 2 is where end-to-end UI testing becomes possible).
