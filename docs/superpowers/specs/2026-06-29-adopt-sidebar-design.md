# Adopt-Sidebar — Multi-Session UI (design)

> Branch `feat/stage2-client-attach`. Surfaces a host's running daemon sessions
> in the SSH-manager sidebar and lets the user adopt one (attach + replay) in a
> new tab. Backend (protocol + adopt entry) is done; this is the UI + wiring.

## Goal / key use case

After an app restart or a transport drop, the daemon sessions on a host keep
running. The user opens the SSH-manager sidebar, sees the host's **running
sessions** listed under it (title = cwd basename / shell), clicks one, and it
opens in a new tab attached to the live session with full scrollback replay.
Also covers "open a second view of a running session" while connected.

## What already exists

- **Protocol:** `RemoteServerClient::list_sessions() -> SessionList` and the
  daemon's `handle_list_sessions` (returns `SessionInfo { session_id, title,
  cwd, alive, last_attached_epoch_millis }`). Runtime-tested.
- **Adopt entry:** `Workspace::adopt_daemon_session(server, pty_session_id, ctx)`
  — creates a daemon tab in *adopt* mode (attach + replay) and connects.
- **Routing pattern:** panel emits `SshManagerPanelEvent` → `left_panel.rs`
  re-emits `LeftPanelEvent` → `Workspace` handles (see `OpenSshTerminal`).

## The gap (why it isn't just "render the list")

The panel knows **nodes/servers** (host/user/port). The `RemoteServerManager`
keys connected sessions by **`HostId`** — which is reported by the daemon in the
initialize handshake and is **not derivable** from the saved server. So the
panel cannot, on its own, find a connected client for a node to call
`list_sessions`. And in the primary (post-restart) use case there is **no open
terminal / no existing connection** at all.

## Architecture decision

**Workspace orchestrates; the panel is a thin renderer.** Workspace already owns
the daemon-connect logic (`spawn_daemon_session_connect`: `ensure_control_master`
→ `check/install_binary` → `connect_session`) and learns the `HostId` on
`SessionConnected`. So:

- **List:** the panel requests a listing for a node; Workspace ensures the
  ControlMaster + connects a **list-only** session (no tab) + calls
  `list_sessions`, then pushes the `Vec<SessionInfo>` back into the panel's
  per-node state for rendering. Reuses the existing connect path (the running
  daemon is reused; if none is running the connect spawns it, which is also what
  surfaces zero sessions cleanly).
- **Adopt:** the panel emits the picked `pty_session_id`; Workspace calls
  `adopt_daemon_session`.

Routing (mirrors `OpenSshTerminal`), both directions:
- panel → `SshManagerPanelEvent::{RequestSessionList, AdoptDaemonSession}` →
  `left_panel` → `LeftPanelEvent::…` → `Workspace`.
- Workspace → `panel.update(set host_sessions[node_id] = sessions)` via the
  held panel handle (left_panel owns it).

## Increments (each compiles warning-clean — wired end-to-end)

1. **Routing + render together (vertical slice):** add the two events through
   the chain; Workspace handler for `AdoptDaemonSession` → `adopt_daemon_session`;
   Workspace handler for `RequestSessionList` → connect-list → push back; panel
   gains `host_sessions: HashMap<node_id, Vec<SessionInfo>>` + renders session
   child-rows under an **expanded** host + a per-host refresh affordance; row
   click emits `AdoptDaemonSession`. (Build it as one slice so there's no
   dead-code interim.)
2. **List-only connect path:** factor a `spawn_daemon_session_connect` variant
   that connects without opening a tab and resolves `list_sessions`, used by the
   `RequestSessionList` handler.
3. **Polish:** loading/empty/error states per host; refresh on
   `SessionReconnected`; only show the expander for `session_resilience`-capable
   key-auth hosts.

## Open UX decision (before build)

- **Fetch trigger:** on host-row **expand** (+ a manual refresh icon) — lazy,
  no background connects. Alternative: auto-fetch on host-connect.
- **Scope:** any daemon-capable (key-auth, `session_resilience`) host via the
  connect-to-list path (covers the post-restart case) — vs. only hosts that
  already have an open daemon terminal (simpler, but misses the main use case).

Recommended: **on-expand fetch + connect-to-list for any daemon-capable host**
(serves the post-restart adopt case, no background work).
