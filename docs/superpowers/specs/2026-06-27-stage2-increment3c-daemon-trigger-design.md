# Stage 2 Increment 3c — Daemon-Session Trigger (Option B)

> **Status:** Design, created 2026-06-27.
> **Parent:** [native-remote-session-layer.md](../plans/2026-06-24-native-remote-session-layer.md) §6/§7 + [Stage 2 design](2026-06-25-stage2-client-attach-design.md).
> **Builds on:** 3a (`daemon_tty` terminal manager, commit 492ebd4e) + 3b (`session_resilience` per-host setting, commit e4406460).

---

## 1. Goal

Make a saved SSH host with `session_resilience.is_enabled()` open **directly** as a
daemon-hosted session (Option B, user-decided): the remote daemon owns the PTY and a
replay buffer; the SSH connection is a pure control/transport channel. `local_tty`
(localhost + plain-vanilla ssh) and the existing non-resilient SSH path stay the
untouched default.

Out of scope (later increments): replay-on-reattach/persistence reconnect (4), the
`session_resilience` settings UI + feature gate (4), mosh-grade UDP transport / B3.

---

## 2. Verified seams

| Concern | Location |
|---|---|
| SSH-host launch (the branch point) | `app/src/workspace/view.rs:5416` `open_ssh_terminal()` — resolves auth, builds `cmd`, opens a **local** tab, spawns password/startup/su injectors, queues `ssh` via `execute_command_or_set_pending` |
| Per-host opt-in (3b) | `SshServerInfo.session_resilience: SessionResilience` (`warp_ssh_manager::types`), `is_enabled()` |
| Daemon terminal (3a) | `app/src/terminal/daemon_tty/` — `TerminalManager::create_model(resources, size, model_event_sender, window_id, input_config, connection_session_id: SessionId, open_params: OpenSessionParams, ctx)` |
| Headless connect: ControlMaster args | `crates/remote_server/src/ssh.rs:17` `ssh_args(socket_path)` (for multiplexed cmds over an existing master) |
| Headless connect: auth machinery | `crates/warp_ssh_manager/src/ssh_command.rs` (`build_ssh_command_line`, `build_password_auth_stdin`, `test_connection` — already does a non-interactive ssh connect with password/key auth) |
| Transport + connect | `app/src/remote_server/ssh_transport.rs` `SshTransport::new(socket_path, auth_context)`; `RemoteServerManager::connect_session(session_id, transport, auth_context, ctx)` (`crates/remote_server/src/manager.rs:706`, **unchanged**) |
| Connected signal | `RemoteServerManagerEvent::SessionConnected { session_id, host_id }` |
| SessionId | `warp_core::SessionId(u64)`, `From<u64>` (allocate a fresh id for the headless session) |

**Feasibility (confirmed by investigation):** headless connect is low-effort — spawn our
own `ssh -N -o ControlMaster=yes -o ControlPath=<local socket> user@host` (with auth),
wait for the socket, build `SshTransport`, call `connect_session`. This bypasses the
interactive DCS bootstrap (`dcs_hooks.rs` → `SshInitShell` → `RemoteServerController`),
which today is what supplies `socket_path`.

---

## 3. Design

### 3.1 Tab/connect ordering (UX)

A `daemon_tty` terminal has **no** local PTY — its bytes only arrive once the daemon
session is open, which needs a connected transport. Chosen model (premium UX, fits the
existing async pattern): **create the daemon tab immediately**, show a "connecting…"
state, and have `daemon_tty` defer `OpenSession` until its connection reaches
`Connected`. (Alternative — connect first, then create the tab — is simpler but adds a
pre-tab delay and a disappearing-action feel; rejected.)

### 3.2 `daemon_tty` waits for `SessionConnected` (3c-i)

Today `EventLoop::start` calls `open_session` eagerly (3a assumed an already-connected
session). Change: stash `(open_params, size_info)` as pending; on start, if the manager
already has a client for `connection_session_id`, open now; otherwise the existing
manager subscription gains a `SessionConnected { session_id } if session_id ==
connection_session_id` arm that triggers the one-shot open. Also handle
`SessionConnectionFailed`/`SessionDisconnected` for our id → surface a clear error into
the terminal model (no silent dead tab). Isolated, `cargo check -p warp`-verifiable.

### 3.3 Headless connect orchestrator (3c-ii)

New, self-contained unit (likely `app/src/remote_server/headless_connect.rs`): given a
resolved `SshServerInfo` + auth + a freshly allocated `SessionId`:

1. Compute a stable local ControlPath socket (e.g. hash of `host:port:user:identity_key`
   under the existing ssh socket dir).
2. Spawn the master `ssh -N -o ControlMaster=yes -o ControlPersist=… -o
   ControlPath=<socket> <auth opts> user@host`, reusing `ssh_command.rs` auth handling
   (key/agent now; password via the existing stdin/askpass machinery).
3. Await the socket file, then `SshTransport::new(socket, auth_context)` →
   `RemoteServerManager::connect_session(session_id, transport, auth_context, ctx)`.
   The binary check/install is already part of `connect_session`'s precondition via the
   manager; reuse `check_binary`/`install_binary` exactly as `RemoteServerController` does
   if needed, or require a preinstalled daemon for v1 (decide during impl).

**Auth scope for v1:** key/agent auth works cleanly headless. Password auth needs the
non-PTY injection path (`build_password_auth_stdin` / `SSH_ASKPASS`); if that proves
involved, v1 falls back to the normal (non-daemon) SSH path for password-only hosts and
logs why — never a broken tab.

### 3.4 `open_ssh_terminal` branch (3c-iii)

At the top of `open_ssh_terminal`, after auth resolution: if
`server.session_resilience.is_enabled()` (and auth is headless-capable), take the daemon
path — allocate `SessionId`, kick off the orchestrator (3c-ii), create the tab via
`daemon_tty::create_model(connection_session_id = that id, open_params = {cwd, shell,
env})` instead of the local-shell-+-queued-ssh path. The injectors and
`execute_command_or_set_pending` are skipped (no local PTY). Otherwise: unchanged.

`create_session`/`add_new_session_tab_internal_*` need a daemon variant that routes to
`daemon_tty::create_model` (additive; `local_tty` stays the cfg/runtime default).

---

## 4. Increments

- **3c-i** — `daemon_tty` defers `OpenSession` until `SessionConnected`; handles
  connect-failure into the model. `cargo check -p warp`.
- **3c-ii** — headless-connect orchestrator (spawn master + `connect_session`). Isolated;
  unit-test the socket-path derivation; runtime-validate via the xl dispatch / a real run.
- **3c-iii** — `open_ssh_terminal` branch + daemon tab creation. End-to-end (output
  renders, input/resize/**mouse** via `send_session_input` — mouse is structurally safe,
  no multiplexer nesting). This is where Stage 2's end-to-end mouse/blocks goal lands.

---

## 5. Risks

- **Headless password auth** — main unknown; mitigated by key-first + graceful fallback.
- **ControlMaster lifecycle** — who owns/tears down the headless master (the daemon
  session outlives the SSH channel by design). Tie master teardown to session close /
  `ssh -O exit` (`ssh.rs`), not to a client tab close (a detached session must survive).
- **Daemon not installed** — reuse the manager's check/install, or require preinstalled
  for v1 with a clear error.
- **No async-model test harness** — app-side increments are `cargo check`-verified +
  exercised via xl dispatch / real run; full behaviour validation is by running the app.
