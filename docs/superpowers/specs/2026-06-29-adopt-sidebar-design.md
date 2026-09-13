# Adopt-Sidebar — Multi-Session UI (design)

> Branch `feat/stage2-client-attach`. Surfaces a host's running daemon sessions
> in the SSH-manager sidebar and lets the user adopt one (attach + replay) in a
> new tab. Backend (protocol + adopt entry) is done; this is the UI + wiring.

## Goal / key use case

After an app restart or a transport drop, the daemon sessions on a host keep
running. The user opens the SSH-manager sidebar, sees the host's **running
Zaplex sessions** listed under it (title = foreground agent provider plus
task name, falling back to the project/cwd basename where available, otherwise a neutral Zaplex identifier;
never a shell executable name), clicks one, and it
opens in a new tab attached to the live session with full scrollback replay.
Also covers "open a second view of a running session" while connected.

## What already exists

- **Protocol:** `RemoteServerClient::list_sessions() -> SessionList` and the
  daemon's `handle_list_sessions` (returns `SessionInfo { session_id, title,
  cwd, alive, last_attached_epoch_millis }`). Cross-version recovery requires
  CI and signed-client/real-host acceptance evidence for the tested commit.
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

**The panel owns discovery state; Workspace owns adopted tabs.** Discovery must
also work after restart when no Workspace terminal exists, so the panel uses the
headless connect-to-list path and keeps loading/error/inventory state per saved
node. Workspace remains the only owner of opening an adopted terminal tab. So:

- **List:** the panel runs a **list-only** headless connection (no tab), calls
  `list_sessions` on every eligible identity-local daemon runtime, and stores
  the routed result in per-node render state. The default/current route retains
  the normal install/start/self-heal behaviour; historical routes remain
  strictly connect-only. Titles use the generation-matched foreground agent
  provider and task, then project. Failed inventory on a capable daemon may
  retain a cwd-based project title; peers without agent-inventory support use a
  neutral session identifier while retaining cwd metadata for routed operations.
  When multiple runtimes contribute, the combined host-cap row is hidden because
  each daemon enforces its own ring cap; per-session values remain exact.
- **Adopt:** the panel emits the picked PTY id, generation, and historical
  runtime route when present; Workspace calls `adopt_daemon_session`. Historical
  routes are connect-only and remain pinned through reconnect.

Routing for adoption mirrors `OpenSshTerminal`: panel →
`SshManagerPanelEvent::AdoptDaemonSession` → `left_panel` →
`LeftPanelEvent::AdoptDaemonSession` → `Workspace`.

## Increments (each compiles warning-clean — wired end-to-end)

1. **Routing + render together (vertical slice):** wire `AdoptDaemonSession`
   through Workspace; the panel gains per-node inventory state, renders session
   child rows under an **expanded** host, and exposes a per-host refresh action.
2. **List-only connect path:** use the headless connect-to-list helper directly
   from the panel so discovery never depends on an already-open terminal.
3. **Polish:** loading/empty/error states per host; refresh on relevant
   connection/session lifecycle events; only show the expander for
   `session_resilience`-capable key-auth hosts.

## Resolved UX decision (2026-09-13)

- **Fetch trigger:** a persistence-enabled host auto-reveals and fetches its
  Zaplex-session disclosure once when it first becomes connected in an app run.
  Relevant daemon/session lifecycle events refresh each expanded
  persistence-enabled disclosure after each acknowledged normal or managed open,
  on exit, and after every managed Stop/Restart RPC result, including detached
  sessions and partial failures. An event during an in-flight fetch schedules
  one follow-up refresh, so a stale response cannot hide the change;
  the host menu also offers a manual refresh. The user can collapse/reopen it,
  and the app never reopens a section the user already collapsed.
- **Scope:** any daemon-capable key-auth host through the connect-to-list path,
  including the post-restart recovery case and older still-running runtimes.
