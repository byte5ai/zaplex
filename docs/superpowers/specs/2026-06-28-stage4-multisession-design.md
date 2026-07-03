# Stage 4 Design — Multi-Session + Lifecycle

> **Status:** Design, created 2026-06-28.
> **Parent:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) §5/§7/§12 + Stage 1–3.
> **Goal:** several sessions per host, list + adopt them, and bound their lifetime/RAM.

---

## 1. Already done

- Per-host `session_resilience` setting + DB migration + UI toggle (Stage 3b + UI).
- Per-session output ring with a byte ceiling (`RING_CEILING_BYTES = 4 MiB`, Stage 1) — a ceiling already "greift".
- Grace-timer guard so the daemon stays up while sessions live (Stage 3a).
- Proto is ready (Stage 0): `ListSessions {}`, `SessionInfo {session_id, title, cwd, alive, last_attached_epoch_millis}`, `SessionList { sessions }`.

## 2. Increments

- **S4a — ListSessions (server + client).**
  - `Session` gains metadata: `cwd: Option<String>`, `shell: String`, `last_attached_ms: u64` (epoch millis; 0 = never). Set on open (opener counts as attached); `last_attached_ms` refreshed on every `AttachSession`.
  - `handle_list_sessions` → `SessionList` of `SessionInfo` over `self.sessions` (title derived from cwd basename else shell; `alive = true` for every registry entry — exited sessions are removed on reader-EOF/close).
  - Client `list_sessions() -> SessionList` (request/response, mirrors `attach_session`). Dispatch: unix → handler; non-unix → reject.

- **S4b — headless test** (server_model_tests, xl): open N sessions with distinct cwds → `ListSessions` → assert N entries with the right ids/cwds/alive; close one → list shrinks.

- **S4c — detached-idle GC + RAM ceiling setting** (server, follow-up):
  - Periodic sweep reaps sessions that are detached (their `attached` conn is gone) and idle (`last_attached_ms` older than `MAX_DETACHED_AGE`, default 24 h) → kill+reap+`SessionExited`. Bounds host RAM from abandoned sessions; complements the grace guard.
  - Lift the ring ceiling from a constant to the per-host `session_resilience`-adjacent setting + a per-host total-RAM cap (registry-level). Wire from the persisted `ssh_servers` row.

- **Adopt UI (GUI, user E2E):** sidebar lists `list_sessions()`; "Enter on a running session" → `attach_session(id, 0)` → full-history block. The protocol foundation (S4a) is what makes this real; the rendering is part of the GUI E2E.

## 3. Notes

- `alive`: registry membership == alive (the reader-EOF path and `CloseSession`/`handle_close_session` remove dead sessions and emit `SessionExited`). No `try_wait` in the read path (avoids reaping side effects mid-list).
- Timestamps via `SystemTime::now()` server-side (epoch millis); the client renders relative ages.
- S4a/S4b are fully headless-testable; the idle-GC (S4c) is testable with an injected/short max-age; the sidebar/adopt rendering is the GUI E2E.
