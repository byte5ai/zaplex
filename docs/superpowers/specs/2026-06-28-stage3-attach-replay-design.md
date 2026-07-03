# Stage 3 Design — Attach / Replay / Reconnect

> **Status:** Design, created 2026-06-28.
> **Parent:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) §6/§9 + Stage 1/2.
> **Goal:** the actual "survives the drop" payoff — a daemon session keeps running while the client is gone, and on reconnect the client re-attaches and replays the missed output so the grid/blocks reconstruct.

---

## 1. What already holds (Stage 1/2)

- The daemon owns the PTY + a per-session `OutputRing` (byte-capped, monotonic `seq`, `replay_from(from_seq) -> (base_seq, bytes)`).
- `deregister_connection` does **not** kill sessions — a dropped connection leaves the session running; the reader keeps appending output to the ring (the dead conn's pushes are harmlessly dropped). So output accumulates while detached. ✓
- Client already has `attach_session(session_id, last_seq) -> SessionAttached{session_id, size, base_seq, replay}` and the `daemon_tty` event loop.

## 2. Gaps Stage 3 closes

| Gap | Fix |
|---|---|
| Daemon `AttachSession`/`DetachSession` are stubbed (reject `InvalidRequest`) | Real handlers (server, unix). |
| **Grace timer kills persistent sessions** — when the last conn drops, the daemon shuts down after `GRACE_PERIOD` (10 min), taking surviving sessions with it | Don't start the grace timer while live sessions exist. |
| Client doesn't re-attach after a transport reconnect | `daemon_tty` tracks `last_seq`, and on `SessionReconnected` calls `attach_session(last_seq)` + feeds `replay` through `parse_bytes`. |

Out of scope (Stage 4): `ListSessions`/adopt, app-restart rehydration, per-host `session_resilience` res_idle GC / RAM ceiling settings.

---

## 3. Increments

- **S3a — server attach/detach + grace guard.**
  - `handle_attach_session(conn_id, AttachSession{session_id, last_seq})`: look up the session; `(base_seq, replay) = ring.replay_from(last_seq)`; set `session.attached = conn_id` (re-route live output to the reconnected connection); reply `SessionAttached{session_id, size, base_seq, replay}`. Unknown id → `InvalidRequest`.
  - `handle_detach_session(DetachSession{session_id})`: `session.attached = Uuid::nil()` (session keeps running; output buffers in the ring, live pushes become harmless no-ops). Notification, no reply.
  - Grace guard: `deregister_connection` starts the grace timer only when **no live sessions** remain (a `has_live_sessions()` helper, cfg-gated since `sessions` is unix-only). Persistent sessions keep the daemon alive. (Detached-idle GC is Stage 4.)
  - `ListSessions` stays rejected (Stage 4).

- **S3b — headless replay test** (server_model_tests, unix, runs on xl): open a session; capture some SessionOutput; `deregister_connection` (simulate a drop) while writing more input; register a *new* connection; `AttachSession{last_seq}`; assert `SessionAttached.replay` contains the output produced while detached, and that live output now flows to the new connection. Runtime-proves the survives-drop core.

- **S3c — client re-attach** (`daemon_tty`, compile-verified; runtime is part of the GUI/real-host E2E): track `last_seq` (`= out.seq + out.bytes.len()` of the latest SessionOutput); add a `SessionReconnected{session_id == connection_session_id}` arm that calls `attach_session(pty_session_id, last_seq)` and feeds the returned `replay` through `process_pty_bytes`. Per §9, keep persistent-session bootstrap state across disconnect rather than clearing it. NOTE: re-establishing the headless ControlMaster on an SSH drop (the daemon_tty transport) may need the orchestrator to re-run `ensure_control_master` — flagged for the E2E.

---

## 4. Notes / risks

- **seq is a byte offset** (not a message counter); `last_seq` the client sends is "everything I've already rendered". `replay_from` clamps to `base_seq` if the requested start was already evicted (ring overflow) — the client then has a gap (acceptable; the ring ceiling bounds it).
- **Re-route via `attached`**: `on_session_output` reads `session.attached` each chunk, so updating it on attach immediately redirects the live stream — no reader-task restart needed.
- Server pieces (S3a/S3b) are fully headless-testable; the client reconnect (S3c) needs the real transport-reconnect path and is validated in the GUI/real-host E2E.
