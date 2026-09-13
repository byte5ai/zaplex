# Stage 1 Design — Daemon-Side Session Host (PTY ownership + output ring + streaming)

> **Status:** Design (read-only; no code yet), created 2026-06-24.
> **Parent plan:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) — this turns that plan's **Stage 1** row into an implementation-ready spec.
> **Builds on:** Stage 0 (merged) — crate `zaplex_remote_session`, additive protocol messages, `InitializeResponse.features` capability handshake, and the daemon dispatch stubs that currently reject the session messages with `INVALID_REQUEST`.
> **Directive:** Sustainable, technically clean solutions only — no quick wins (project memory `no-quick-wins-sustainable-only`).

---

## 1. Scope

**Stage 1 delivers** a daemon that owns PTYs and shells and streams their output, decoupled from the SSH channel:

- `OpenSession` → daemon allocates a PTY, spawns the user login shell, registers a `SessionId`, returns `SessionOpened`.
- A per-session **reader task** pumps PTY bytes into a bounded **output ring buffer** (monotonic `seq`) and pushes `SessionOutput` to the currently attached connection.
- `SessionInput` → write bytes into the PTY.
- `ResizeSession` → `TIOCSWINSZ` on the PTY (closes today's missing remote-resize gap).
- `CloseSession` → terminate + reap the shell; emit `SessionExited`.
- Shell exit (unsolicited) → `SessionExited`.
- Daemon advertises `FEATURE_SESSION_HOST` via `supported_features()`.

**Deferred (later stages, by the plan's phase table):**

- `AttachSession` + `last_seq` replay, `DetachSession`, persistent re-attach window — **Stage 3** (the ring buffer is built here in Stage 1, but replay-on-attach semantics and the manager re-attach wiring are Stage 3).
- `ListSessions`/adopt UI, per-host RAM governor, `session_resilience` per-host setting — **Stage 4**.
- Client Block/grid integration and the mouse path — **Stage 2**.
- UDP/mosh transport — **Stage 5**.

> Stage 1 keeps the **stub handlers** for `AttachSession`/`DetachSession`/`ListSessions` (still `INVALID_REQUEST` or a benign no-op) so the dispatch stays exhaustive; only the Stage 1 verbs get real handlers.

**Acceptance (from the plan):** headless test — open a session, run a command, observe correctly streamed bytes; resize takes effect (`stty size` in the remote reflects it); shell exit yields `SessionExited`.

---

## 2. Where the code lives (touchpoint discipline)

Substance goes into the new crate; inherited crates get **minimal, additive** edits so upstream rebases stay cheap (plan §8.3 / §14).

| Concern | Location |
|---|---|
| Session registry, `Session`, `OutputRing`, reader task, PTY core | `crates/zaplex_remote_session/src/server.rs` (+ submodules) behind the `server` feature |
| Shared PTY primitives (open/spawn/resize/read) | new `zaplex_remote_session::pty` (or a small shared module) — see §4 |
| Daemon wiring (construct registry, route messages) | `app/src/remote_server/unix/mod.rs` (`run_daemon`) + `app/src/remote_server/server_model.rs` dispatch |
| Capability flip | `zaplex_remote_session::types::supported_features()` returns `[FEATURE_SESSION_HOST]` when built with the `server` feature |
| Proto | already defined in Stage 0 (`remote_server.proto`); no proto changes expected in Stage 1 |

The app must depend on `zaplex_remote_session` with the `server` feature for the daemon build (Stage 0 added the default-feature dependency; Stage 1 enables `features = ["server"]`).

---

## 3. Server-side data model

```text
SessionRegistry {
    sessions: HashMap<SessionId, Session>,
    total_ring_bytes: usize,          // for the per-host ceiling (basic in Stage 1)
}

Session {
    pty:           PtyMaster,         // owns the master fd
    child:         Child,            // the spawned login shell
    ring:          OutputRing,
    size:          Winsize,
    seq:           u64,              // last appended seq (monotonic)
    title:         String,
    cwd:           PathBuf,
    attached:      Option<ConnectionId>,
    last_attached: Instant,
}

OutputRing {
    buf:        VecDeque<u8> (or a fixed ring), bounded by max_bytes
    base_seq:   u64,                 // seq of the oldest retained byte
    // replay_from(last_seq) -> (base_seq, bytes) for Stage 3
}
```

**Ownership:** the daemon allocates the PTY and spawns the shell itself, so an SSH drop does **not** kill the session (the whole point). The registry lives for the daemon's lifetime (in-memory; a daemon crash or host reboot ends sessions — explicitly **not** survived, plan §2/§5, communicated honestly).

**Locking discipline:** the registry is shared across the daemon's connection handlers and the reader tasks. Guard it with a `Mutex`/`RwLock` (or an actor/channel). **Never hold the registry lock across an `await`** or across a PTY write that could block — take the per-session handle, drop the lock, then do I/O. (Mirrors AGENTS.md §5.3's lock caution, applied to the registry rather than `TerminalModel`.)

---

## 4. Shared PTY core (the real refactor — plan §14)

Today PTY setup lives client-side in `app/src/terminal/local_tty/unix.rs` (plan §15 cites `:639` for PTY setup / `TIOCSWINSZ`). The daemon needs the same primitives. **Do not duplicate** — extract a small, dependency-light PTY module that both the client TTY path and the daemon can use.

Minimal surface to extract / provide:

- `open_pty(size: Winsize) -> (PtyMaster, PtySlave)` — `posix_openpt` / `grantpt` / `unlockpt` / `ptsname`.
- `spawn_shell(slave, cwd, shell, env) -> Child` — child with the slave as controlling terminal (`setsid`, `TIOCSCTTY`, dup to 0/1/2), via `crates/command` (AGENTS.md §5.7 — never raw `std::process::Command`).
- `set_winsize(master, Winsize)` — `TIOCSWINSZ`.
- async read half over the master fd (the daemon uses `async-io`/`tokio`; the crate already builds on tokio 1.47).

Open decision (resolve at implementation time): **where the shared module lives.** Options: (a) a new tiny `zaplex_pty` crate, (b) a `pty` module inside `zaplex_remote_session` that the client side also imports, (c) lift the existing code in place and re-export. Prefer the smallest change that removes duplication; lean to (b) unless the client path's coupling makes (a) cleaner. This is the highest-effort part of Stage 1 — budget for it.

Windows remote host stays out of scope (plan §1: v1 is bash/zsh, glibc ≥ 2.31). Gate the PTY module `#[cfg(unix)]`.

---

## 5. Reader task & streaming

One task per live session:

```text
loop {
    n = pty.read(buf).await        // EOF => shell exited
    seq = registry.append(session_id, &buf[..n])   // ring.push + seq++
    if let Some(conn) = attached_conn(session_id) {
        push SessionOutput { session_id, seq, bytes }   // empty request_id
    }
    // no attach => bytes still accumulate in the ring (session stays productive)
}
on EOF/err: reap child, emit SessionExited { session_id, exit_code }, mark !alive
```

**Push path:** reuse the daemon's existing server→client send with an empty `request_id` (push convention, `protocol.rs`/`server_model` `send_server_message`). The message is `server_message::Message::SessionOutput`. Confirm the daemon's per-connection writer can be addressed by `ConnectionId` (it already targets specific connections for responses).

**Backpressure:** if the attached connection's writer is slow, do not block the reader (which would stall the shell). Bound the per-connection outbound queue; on overflow, rely on the ring (the client re-syncs via Stage 3 replay). Document the chosen bound.

---

## 6. Handler wiring (replace Stage 0 stubs)

In `server_model.rs` dispatch, replace the Stage 0 `INVALID_REQUEST` arm for the Stage 1 verbs with real handlers (keep `AttachSession`/`DetachSession`/`ListSessions` stubbed):

- `OpenSession{cwd,shell,env,size}` → `open_pty` + `spawn_shell` + register + start reader task → `SessionOpened{session_id}`.
- `SessionInput{session_id,bytes}` → look up session, write bytes to PTY master (notification; no response). Mouse SGR bytes flow through unchanged (Stage 2 closes the client loop).
- `ResizeSession{session_id,size}` → `set_winsize` (notification).
- `CloseSession{session_id}` → signal the shell (SIGHUP/SIGTERM), reader task observes EOF → `SessionExited`.

Keep the match **exhaustive**, no `_` wildcard (AGENTS.md §5.2).

`handle_initialize` already returns `features`; once the `server` feature is compiled in, `supported_features()` returns `[FEATURE_SESSION_HOST]` so capability-aware clients (Stage 2+) take the session path.

---

## 7. Lifecycle, limits

- **End conditions:** shell exit, `CloseSession`, or (Stage 3/4) detached-idle timeout. Stage 1: a session ends only on shell exit or `CloseSession`.
- **Ring ceiling:** `OutputRing` is byte-bounded per session (a setting in Stage 4; a sane constant in Stage 1, e.g. a few MB, anchored to but independent of the client scrollback limit `BlockSize::max_block_scroll_limit`). Track `total_ring_bytes` for the future per-host governor (plan §7/§10), even if only logged in Stage 1.
- **Reaping:** always `wait()` the child to avoid zombies; map exit/signal to `SessionExited.exit_code` (absent when signal-killed, per the proto).

---

## 8. Tests

- **Unit (`OutputRing`):** seq monotonicity, byte-cap eviction advancing `base_seq`, `replay_from(seq)` correctness (the replay API is exercised fully in Stage 3 but built and unit-tested here).
- **Integration (headless daemon)** — reuse `crates/integration` patterns (existing SSH/tmux integration tests):
  - open session → run `echo`/`printf` → assert streamed bytes match.
  - `ResizeSession` → run `stty size` in the session → assert rows/cols.
  - shell `exit` → assert `SessionExited` with the right code.
  - `SessionInput` round-trip (type a command, see its echo/output).
- **Lock/concurrency smoke:** open N sessions, stream concurrently, assert no deadlock and no cross-session byte interleaving.

---

## 9. Risks / open questions

- **PTY core extraction (§4)** is the main effort and the main upstream touchpoint — keep it additive and `#[cfg(unix)]`; decide the module home before coding.
- **Push addressing:** confirm the daemon can push to a specific `ConnectionId` outside a request/response turn (needed for `SessionOutput`). If not, add a minimal per-connection outbound channel — additively.
- **Backpressure** policy (slow client vs shell) — pick and document a bound.
- **Feature gating:** the daemon and the same client binary share one build; ensure enabling `zaplex_remote_session/server` for the daemon does not pull server-only deps into the client/WASM targets (`#[cfg(unix)]` + feature gates).
- **Version lockstep:** normal routes enforce client==daemon tag; the later recovery amendment permits only an explicitly inventoried older daemon, connect-only for its existing sessions. `features` negotiation still controls graceful degradation (plan §11).

---

## 10. Definition of done (Stage 1)

- `cargo check` green (app + `remote_server` + `zaplex_remote_session` with `server`).
- Headless integration tests in §8 pass.
- Daemon advertises `FEATURE_SESSION_HOST`; no behaviour change for clients that ignore it.
- No duplicated PTY code; inherited-crate edits minimal and additive.
- Comments in new code in English (repo policy).
