# Cockpit, Connections, and account identity — technical design

GitHub: https://github.com/byte5ai/zaplex/issues/160

## Context and boundaries

The feature separates three projections that currently overlap:

- the SSH registry is the source of configured Connections and favorite host references;
- `RemoteServerManager::connected_daemons()` is the source of live remote Cockpit roots;
- `zaplex_cockpit::Snapshot` is the source of accounts and agent conversations.

The authoritative stores stay unchanged. In particular, this work does not add a second SSH
registry, favorite store, account-override file, mutable PTY tree, or reference-repository runtime
dependency. The existing flat `ProjectNode.sessions` inventory remains authoritative; the fourth
tree level is a pure presentation grouping.

The implementation starts from Zaplex `origin/main` at
`5e094c567ee44634364b6c1910cf92f00c7e5148`. The freshly fetched comparison revisions are recorded
in [`REFERENCE_AUDIT.md`](REFERENCE_AUDIT.md).

## 1. Connection and live-host projections

### Connections

`app/src/ssh_manager/panel.rs` remains the full registry editor and becomes the independent
Connections sidebar element. Extend each server row with two existing store projections:

- `FavoritesStore`, keyed by the stable registry node id, supplies the favorite star;
- `RemoteServerManager::connected_registry_hosts()` supplies every live registry-node-to-host
  mapping, including multiple aliases reaching the same daemon, for the trailing
  connected/disconnected plug state.

The whole row retains its existing selection/connect behavior. Star and plug controls use existing
compact row-action/button components, stop row-click propagation, and do not introduce a one-off
theme. Default rows do not render connection-state prose, open-session counts, or a legend.
Transient error/progress feedback may still be announced accessibly.

For a resilience-enabled host, the existing Zaplex-sessions action owns a subordinate recovery
disclosure. The default host row remains unchanged; the disclosure may auto-open once when that
host first becomes connected and otherwise opens only on request. It queries identity-local daemon
runtimes, renders recoverable native PTYs plus typed tmux/byobu inventory, and carries the exact
daemon route on every native-session action. This is not a second connection store and contributes
no state to the host row itself. The ordinary recovery list contains no buffer/RAM metric. Measured
runtime diagnostics remain available in explicit connection details and use their real measurement
name; an output/replay buffer must not be presented as total host RAM.

The tab `+` menu in `app/src/workspace/view.rs` continues to query `FavoritesStore` and resolve its
stable references against the SSH registry. The host-label hit target directly dispatches the
existing new-tab connection action exactly once. A separate stable `⋯` target opens a side flyout
for New Agent, Edit Connection, and Remove from Favorites; the parent remains visible and no child
rows are inserted into its layout. The flyout reuses the existing menu/overlay, focus-return,
safe-triangle, and edge-placement mechanisms. Both targets retain the stable registry reference
across reactive updates, stop event propagation from crossing targets, and handle stale references
without creating a phantom host. The menu never reads the Cockpit tree or creates a duplicate host.

### Live Cockpit roots

`app/src/cockpit/model.rs` builds remote contributions only from
`RemoteServerManager::connected_daemons()`. Registry reconciliation may update the label or mark an
already-live node removed, but `crates/zaplex_cockpit/src/fleet.rs` must never append an offline
registry-only root.

Each contribution carries `AgentInventoryStatus`:

- `Ready` means inventory retrieval succeeded, including an empty result;
- `Unsupported` means the connected peer lacks the inventory capability;
- `Unavailable` means the request failed for this refresh.

The local contribution is always retained. Every connected remote contribution is retained even
when its session list is empty or its inventory is unsupported/unavailable.

The ordinary topology remains one contribution per connected registry host and runtime. Each pane
resolves its own daemon identity, and several panes may reuse one contribution; the containing tab
is not a routing key. Cross-version recovery is the bounded exception: a historical daemon that
still owns PTYs retains a distinct `HostId` and therefore a distinct temporary contribution. The
two contributions may share a registry node and display label, but must remain separately routed;
registry reconciliation binds that node to every live daemon identity rather than selecting one.
Existing-session attach, managed Stop, signals, transcript reads, and file operations bound to an
adopted terminal resolve the exact `HostId` and historical route. New launches, managed Start and
Restart, account/model discovery, directory validation, and standalone file operations such as
SFTP use only the current route. Historical fleet rows cannot Start or Restart; their daemon-local
account identities must not be reused on the current runtime. Disconnecting the Connections row
deregisters every live daemon identity for that registry node. Each contribution is removed when
its final manager connection disappears; ending its last PTY does not by itself mean the transport
is already disconnected. Without live PTYs or connections, the historical daemon can retire after
its idle grace period. Runtime-local diagnostics stay bound to their runtime and explicit
connection-details surface; ordinary navigation does not synthesize an aggregate host-cap metric.

`CockpitModel` subscribes to `RemoteServerManagerEvent`. `HostConnected` and events that complete,
reconnect, disconnect, deregister, or exit start a refresh. `SessionOpened` follows every
authoritative normal or managed open acknowledgement; `SessionInventoryChanged` follows every
managed Stop/Restart RPC result, including detached sessions and partial failures. Both refresh
the inventory after the operation so changes appear without restarting the app. `HostDisconnected`
first removes the stable daemon id synchronously from the visible tree, emits an update, and then
starts a refresh. Existing refresh-generation gating prevents an older in-flight result from
re-adding the disconnected root. Session-level events do not add duplicate roots.

## 1A. Mixed-host pane routing and lifecycle (#459–#462)

### Pane-local launch intent

`app/src/workspace/view.rs` keeps the existing distinction between a favorite/new-tab launch and a
pane-local split launch. `open_ssh_terminal_command` and `open_resolved_ssh_terminal_command` must
not infer a split target from whatever pane happens to be focused when an asynchronous picker or
connection finishes. The split request captures, before opening UI or starting transport:

- the source tab and pane identities;
- the exact requested `Right` or `Down` direction;
- the stable registry reference, or the explicit Local destination;
- same-host working-directory inheritance intent where applicable.

On acceptance, the target tab/pane is revalidated. A missing, moved, or no-longer-compatible target
fails visibly instead of silently choosing another pane. Cancel, Escape, and outside click create no
session and restore a meaningful prior focus. Multi-click protection ensures one accepted intent
creates one session. Same-host launch may carry the current working directory; cross-host and Local
launch resolve the destination profile/default independently and never interpret the source path in
the destination namespace.

`app/src/pane_group/mod.rs` remains the one layout owner. Use its existing target-aware
`add_session`, `add_pane_with_options`, `base_pane_id`, and direction facilities rather than adding
a second pane tree. Preserve the conservative Classic SSH/legacy checks in `ssh_tab_nodes`; use
`node_for_session` or the pane-owned session route whenever an exact daemon already exists. Every
terminal operation—including Agent launch, File Manager, editor/file action, reconnect, and
restore—resolves the pane's `HostId`/daemon/PTY/generation, never a tab-level host assumption.

### Identity, title, geometry, and focus

`app/src/pane_group/pane/terminal_pane.rs` derives the automatic terminal identity from the pane's
actual host plus project or working-directory basename. The full host/path remains in tooltip and
accessibility text. A missing CWD uses an honest host/session fallback. Collision handling adds a
stable restrained suffix only when two displayed short identities really collide; it does not make
a row index, truncated session id, or global project value the primary identity.

`app/src/pane_group/mod.rs::display_title` and `app/src/tab.rs` retain the existing precedence:
explicit custom tab title, otherwise the focused pane's display title. Removing the override
returns to automatic behavior. Focus state is stored per tab; an account pane in one tab must not
alter the focused-pane identity of another tab.

`app/src/pane_group/tree.rs` owns geometry. One remaining pane consumes the whole available pane
area. Splits mutate only the selected leaf and preserve the requested direction subject to existing
minimum-size rejection. Existing header drag in
`app/src/pane_group/pane/view/header/mod.rs` (`calculate_pane_move_direction`) moves the same pane
entity to the chosen edge or tab; it must not create a connection, PTY, or agent. Drop preview uses
existing theme roles. Invalid targets and cancellation are transactional. Header actions, file
drag, text selection, and divider resize retain disjoint hit targets.

Whole-tab cross-window drag is fail-closed while any contained daemon terminal is pending or while
a managed start remains between input readiness and its final managed acknowledgement. Those
routes, attempt generations, futures, and completion subscriptions remain workspace-owned, so no
`TransferredTab` is produced until Ready or terminal failure, and managed Ready remains blocked
until `ManagedLaunchOpened` or `ManagedLaunchFailed`. Same-workspace pane moves remain available
because their workspace owner does not change. Pending terminal state ends monotonically at the
first authoritative Ready: later Transport, Attach, or Replay phases still gate input but do not
re-arm workspace ownership for an established session. Managed acknowledgement remains a separate
guard. Hidden Undo Close panes and terminals covered by a temporary File Manager replacement
participate in the same initial-start and managed-acknowledgement gates.

Persistence serializes/restores pane topology together with the existing stable session route,
generation, focused pane per tab, custom-title override, pane mode, and pane-owned state. Restore
does not coalesce two hosts with the same label/path. Closing or moving a pane invalidates pending
focus callbacks so late connection/restore events cannot steal focus.

### Shell readiness and reconnect (#456)

`app/src/workspace/view.rs::adopt_daemon_session`,
`app/src/remote_server/headless_connect.rs`, and
`app/src/terminal/daemon_tty/event_loop.rs` keep transport, attach, replay, and input readiness as
distinct evidence. A successful attach is bound to the requested daemon route, PTY id, and
generation; arbitrary replay output or a status line is not readiness. An already-open exact
session is focused instead of attached a second time.

The terminal pane exposes ordinary command input only after the event loop can deliver it to that
attached PTY generation. Text typed or pasted before then is not invisibly queued for later
execution; existing drafts remain intact. Authentication, password, and host-key challenges retain
their explicit secure interaction paths. Every non-corrupt pending phase reaches Ready, a
phase-specific retry/cancel error, or cancellation within its existing bounded contract. Cancel
closes the local attempt without stopping the remote PTY. A corrupt restore is terminal and offers
neither action because no authoritative retry or cancellation target exists. Inventory refresh
independently reaches loaded or an honest retryable error and does not discard still-valid rows
merely to show an unbounded spinner.

Corrupt persisted remote metadata is represented by a dedicated terminal-view phase. Restoration
keeps the original pane tree and constructs a fail-closed daemon-backed surface on every platform;
it never permits `create_session` to select the local-terminal fallback. The footer names the
corrupt identity and omits retry/cancel controls because neither operation has an authoritative
route. This terminal phase remains input-gated and cannot be overwritten by later transport events.
Retry and cancel replacements for a valid daemon restore likewise pass an inert
`DaemonSessionRequest` rather than `None`, including the non-Unix retry branch, so no action can
instantiate a local PTY behind the saved remote identity.

Managed `OpenSession` delivery uses a bounded logical-open state machine. Exact accepted cache hits
and parameter conflicts resolve before mutable account, project, and managed-route validation;
only a cache miss revalidates current launch inputs. In-flight requesters are deduplicated and
bounded. The additive `logical_open_attempt` generation retires a half-open older transport as soon
as its replacement attempt is registered; Abort or connection teardown removes only the current
requesters. When the final current requester leaves, the origin task is cancelled and a delayed
blocking preflight checks that the reservation still exists before any PTY or agent allocation.
Existing-managed results set the additive `SessionOpened.requires_attach` bit and carry the
authoritative foreground `expected_agent_binding`. A fresh managed result learns and binds that
identity from the structured agent lifecycle. Both paths claim the PTY and complete the same exact
generation- and agent-checked attach before terminal readiness can publish launch success.
The client treats missing terminal-view or connected-session claim context as a terminal ownership
failure: it publishes neither `SessionOpened` nor managed success and cannot bind, attach, or become
ready. Spawn-card bulk launches use independent connect-attempt generations for each managed target,
including different accounts on one registry node; ordinary host-opening actions retain their
single-attempt deduplication, and the host remains visually connecting until its final attempt ends.

These wire additions remain protobuf-additive, but their safety contract is capability-gated.
Automatic ambiguous-open retry requires both `logical-open-id-v1` and
`logical-open-attempt-v1`; without both, the client fails closed instead of issuing another open.
Managed start additionally requires `managed-open-attach-v1`, so an older client or daemon cannot
start a managed agent under weaker acknowledgement semantics; the gate also requires
`agent-pty-binding-v2` for the foreground identity check. Older mixed versions must be upgraded.
They may decode and ignore the new fields, but do not provide these ownership guarantees.

The client event loop atomically claims `(daemon, runtime, PTY, generation)` before an authoritative
attach can start. An existing claim is idempotent only for the same connection and terminal view;
`Workspace::index_opened_daemon_session` observes that exact owner. A late competing claim publishes
the acknowledgement only for cleanup, skips attach, retires the losing pane/connection, focuses the
existing owner, and returns before binding or persisting remote identity.

### Account-pane placement (#160/#459)

The Cockpit account action resolves a stable provider/account identity within the current tab. It
focuses the existing matching account pane in that tab or adds one through `PaneGroup`; it never
replaces a terminal or File Manager pane. Deduplication scope is one tab only. The same account may
therefore be open in another tab with independent focus. The aggregate-account action has a visible
localized label.

## 2. Four-level presentation tree

`crates/zaplex_cockpit/src/conductor.rs` groups each project's flat agent snapshots into
`ConductorSession` values:

- snapshots with PTY metadata group by `(pty_session_id, pty_session_generation)`;
- snapshots without PTY metadata each receive a stable fallback key derived from the full agent
  session identity;
- a foreground agent is the representative, otherwise the most recently active child is;
- children sort waiting-first, then by recent activity.

No label, project name, account email, or truncated id may be used as the grouping identity.
Selection still routes the exact child `SessionKey` through the existing local/remote resume path.

`app/src/cockpit/panel.rs` renders `Host → Project → PTY session → Agent` with independent stable
expansion keys. Hosts, projects, and PTY sessions default expanded. The old fleet-size auto-collapse
and host registry actions are absent from this tree.

Presentation rules are encoded in small pure helpers/descriptors where practical so tests do not
depend on pixels:

- expanded containers hide counts; collapsed containers show a count;
- a collapsed count turns amber if it hides waiting attention;
- session containers have hierarchy only, not a duplicated aggregate state glyph;
- agent leaves render state glyph, provider, and optional model only;
- no tree leaf renders state words, context percentage, cost, email, effort, or activity age;
- the section header renders only an amber glyph and numeric count when attention exists.

## 3. Waiting animation and accessibility

Only the tree's amber waiting glyph animates. Reuse the WarpUI frame/repaint mechanism rather than
a model timer: derive a 1.6-second normalized phase from elapsed monotonic time, request the next
repaint, vary core emphasis modestly, and draw a ring whose maximum diameter is approximately twice
the core glyph footprint. Layout bounds remain constant so the pulse never reflows a row.

When `AccessibilitySettings::reduce_motion` is active, render the same amber glyph with static
emphasis and do not schedule animation-only repaints. Every glyph exposes a semantic status label
or tooltip; color is not the sole accessible state mechanism. Working, idle, connection, and host
inventory glyphs remain static.

## 4. Account and session discovery

### Root discovery

Account discovery is independent from session count. Resolve documented default roots plus pinned
environment roots (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) deterministically. On Linux, include
`CLAUDE_CONFIG_DIR` from live Claude processes only after confirming effective same-UID; revalidate
process start time and command after reading the environment so PID reuse cannot cross the trust
boundary. Permission denial, incomplete `/proc` visibility, and unsupported platforms contribute
generic degraded health without exposing environment values or paths. Canonicalize roots when
possible and deduplicate aliases by canonical path or stable discovered identity. An unreadable or
malformed source contributes a degraded/error scan outcome, not an invented account or successful
zero result.

Keep account aliases, color, ordering, and hidden state in the existing
`crates/zaplex_cockpit/src/overrides.rs` store.

### Claude live and dormant sessions

`crates/zaplex_cockpit/src/sessions.rs` accepts both supported registry shapes:

- legacy entries with a valid legacy status;
- current real conversations with a known `kind` of `interactive` or `bg`.

Status-less unknown entries and shell/helper records remain excluded. Transcript matching and
stable session identity remain mandatory for a resumable conversation.

In addition to registry-backed live inventory, scan a bounded recent transcript set for dormant
history. A transcript-only candidate must be valid and substantial (at least two assistant text
turns or one tool use), exclude observer/memory infrastructure, and becomes an idle account-detail
session. It is deliberately excluded from the live Cockpit tree.

### Exact launch metadata binding (#166)

Assign a process-local `LaunchId` before the provider command can execute. The newly opened
terminal is an attach relation for that launch, while the provider session id from the structured
hook remains conversation identity. Buffer either side so terminal attachment and the first hook
event may arrive in either order. Once both exist, rekey model/effort intent to
`Host × Provider × Account × Session-ID` and remove that launch's coordinate entry. Repeated hook
events and daemon rehosting are idempotent; an account mismatch fails closed. Until a provider id
arrives, the legacy `(provider, host, cwd)` fallback remains available and renders effort with a
compact `~` prefix so heuristic data is visibly distinguishable from an exact binding.

### Machine-readable snapshot (#163)

`zaplex cockpit snapshot --json` owns a versioned, secret-free wire schema rather than serializing
internal Rust structs. Stable ids include the full host/provider/account/session coordinate and do
not collapse accounts or sessions that share a basename. Local discovery is always available. When
an authenticated app IPC endpoint can provide the live `CockpitModel` fold, the command uses it for
connected-host truth; otherwise it returns an explicitly degraded local snapshot and exit status 3.
Unavailable usage values serialize as `null`, never invented zeroes.

### Opaque remote account routing (#165)

`agent-account-routing-v1` adds a daemon-owned account inventory and `AgentLaunchRoute`. The daemon
keeps the canonical config root in a short-lived route cache while the client sees only an opaque
account id, provider, display identity, provenance, health, and capacity. New-capability peers stamp
the opaque id onto `AgentSessionInfo`; `SessionSnapshot` carries it through the Cockpit actions so
start, resume, fork, attach, and transcript reads can name the exact account. Missing, stale, or
ambiguous ids fail closed. The legacy `config_dir` field remains decode-compatible for old peers but
is not used as a cross-host route by new peers.

### Bounded transcript projection (#170)

Codex rollout history and Claude JSONL history normalize into the shared `TranscriptTurn` model.
Local parsing and the remote `agent-transcript-read-v1` RPC enforce source-size, line, turn, field,
tool-count, and encoded-response limits. Requests contain provider plus opaque account/session ids;
the daemon resolves paths internally and returns no path or raw record. Only user/assistant display
content, bounded thinking, model, timestamps, and tool names survive projection. Developer/tool
payloads and results, encrypted content, credentials, and unknown metadata are discarded.

Remote responses expose a source revision and `NotModified` result for a later refresh loop; the
initial Cockpit integration opens a one-shot snapshot. Generated projections are parsed off the UI
thread and opened as pathless, selectable in-memory code documents. They are neither editable nor
restorable, never touch a temporary file, and remain valid after the source Cockpit closes.

### Executable parity gate (#169)

`.github/workflows/cockpit-parity-audit.yml` runs on relevant pull-request paths and explicit
dispatch. `script/cockpit-parity-audit` validates a machine-readable matrix, refreshes or verifies
the reference default branches fail-closed, records exact revisions/timestamps, runs the focused
Rust check/test suites in CI, and drives normal, narrow, and reduced-motion Playwright screenshots.
The real two-host lifecycle remains a documented manual smoke because hosted CI cannot truthfully
manufacture the user's authenticated remote environment.

## 5. Provider/account presentation

Introduce or reuse one pure identity presentation helper in the Cockpit view layer. Both
`app/src/cockpit/panel.rs` sidebar cards and `app/src/cockpit/pane.rs` detail cards consume it:

- headline: explicit provider name (`Claude`, `Codex`, or supported provider);
- subline: account label/email plus plan when known;
- when label and email are equal, render the value once;
- provider color/icon is supplementary and never the only provider identity.

The sidebar cards retain both five-hour and weekly meters. The large pane may retain detailed
reset, token, price, and provenance facts, but removes a separate provider strip that duplicates
the card headline.

In the large session table, waiting rows receive a subtle whole-row amber background using existing
semantic theme colors with low opacity. Row geometry and columns remain unchanged, and the Status
column keeps its glyph plus explicit status word.

Sidebar account cards stack the five-hour and weekly meters so each meter receives the card's
available text width. Unknown and estimated values retain their semantic state and never render as
measured zero. The Cockpit and Connections panels share existing layout constants/components for
section headers, hierarchy indentation, row heights, flexible identity, and fixed action slots.
Native colors, borders, selection, hover, focus, status, and progress resolve exclusively through
`appearance.theme()` and existing component themes; the HTML artifact defines role relationships,
not literal color values. No provider-colored border or other decorative pane outline is added.

## 5A. File Manager pane mode and transfer identity (#458/#464)

The existing File Manager integration in `app/src/workspace/view.rs` remains a mode of its owning
terminal pane. Entering or leaving the mode does not replace or terminate the terminal session.
Host/daemon/PTY/generation, CWD, draft input, process, and selection remain pane-owned. F10 and the
mode-close action switch back to Terminal mode through the same lifecycle rather than closing the
pane.

`app/src/sftp_manager/browser.rs::render_responsive_function_bar` renders a single non-wrapping
function row with stable F3/F4/F5/F6 slots in every File Manager pane. Enabled state comes only from
the currently focused compatible pane. A terminal, account pane, or overlay focus disables File
Manager actions elsewhere without changing their geometry. Long localized labels use the existing
compact/responsive treatment; they do not create a second row or remove existing commands.

`fm_registry.rs` remains the inventory for eligible pane destinations. Copy/Move captures and shows
the source identity and either the sole valid counterpart or an explicit selected target. A target
coordinate contains stable pane/session identity, exact host/daemon route, generation, mode, and
path; the tab title and a presumed tab host are never routing inputs. Candidates may live in other
tabs. Immediately before transfer, `browser.rs`, `file_list.rs`, and `transfer_queue.rs` re-resolve
and validate source/target identity, mode, CWD, generation, and transport. Any stale coordinate
returns to target selection or a visible safe failure. Existing conflict, symlink, path, streaming,
cancellation, and overwrite guards remain authoritative.

Parent navigation stores the directory identity being left before requesting `..`. After the
successful, possibly delayed local or remote listing, `keynav.rs` selects that identity in the
current sorted/filtered projection and scrolls it into view. It never restores a stale numeric row
index. Root, failed/cancelled navigation, or a removed/filtered child uses the established safe
fallback without altering another pane or pending transfer.

## 6. Localization and documentation

English and German copy cover Connections, the Sessions tree, honest local/ready-empty,
unsupported, unavailable, loading, degraded, split direction/destination, reconnect phase/error,
transfer source/target, and labelled aggregate-account states. Do not use a `0 accounts` header as
a loading/error substitute.

The binding visual contract is
[`docs/ui/cockpit-sidebar-connections.html`](../../docs/ui/cockpit-sidebar-connections.html).
Historical Cockpit documents are amended only where needed to mark the old combined registry/live
tree and three-level hierarchy as superseded by GH-160.

## 7. Verification map

| PRODUCT behavior | Verification seam |
|---|---|
| 1–3 Connections/favorites/menu | SSH row projection tests; stable-favorite identity; direct-connect vs `⋯` propagation; parent/flyout mouse, keyboard, focus-return, safe-triangle, and edge-placement tests |
| 4, 8–10, 14 tree identity and grouping | `conductor_tests.rs`, presentation-descriptor tests, exact route assertions |
| 5–7 host lifecycle/inventory | `fleet_tests.rs` and `model_tests.rs` for local empty, first/last connection, unsupported/unavailable, stale generation |
| 11–13 glyphs/pulse | pure state/pulse geometry tests plus static source/UI-spec checks, including reduced motion |
| 15–17 identity/meters/waiting row | pure identity and row-style tests plus HTML visual states |
| 18–22 discovery/history | Claude/Codex root fixtures and `sessions_tests.rs` legacy/current/dormant cases |
| 23–24 parity audit | three timestamped SHA ledgers and evidence matrix in `REFERENCE_AUDIT.md` |
| 25 responsive/accessibility | normal/narrow HTML states, semantic labels, keyboard/focus review |
| 26 machine snapshot | CLI schema fixtures, stable-id/collision tests, degraded/null semantics, IPC capability test |
| 27 opaque remote accounts | proto compatibility, cache ambiguity, capability, launch/resume/fork/attach routing tests |
| 28 exact launch binding | hook-first/terminal-first, mismatch, reuse, rehost, eviction tests |
| 29 transcript projection | Claude/Codex fixtures, redaction, traversal/symlink, limits, revision and pane-lifetime tests |
| 30 continuous parity | audit script validation/self-test, PR workflow, screenshot spec, two-host smoke checklist |
| 31–32 mixed-host tabs/split launch | target-intent tests for tab/pane/direction/host capture; cancel/stale target/double-click; Local, Classic SSH, and daemon routes; usable-input focus only after readiness |
| 33 identity/title | host/CWD/collision fixtures; custom-title precedence/removal; per-tab focus and account-pane cases |
| 34–35 geometry/drag/restore | pane-tree full-area and nested split tests; move-not-clone identity assertions; invalid drop; close/focus callback; pending cross-window tab gate through Ready/failure and final managed acknowledgement, including temporary replacement/Undo Close; persistence round trip with mixed modes/hosts |
| 36 account-pane placement | add-or-focus per-tab tests; no replacement; same account in separate tabs; labelled aggregate action |
| 37–39 File Manager/transfers | mode round trip; one-line focus-gated function bar; exact cross-host target and stale-target tests; local↔local, local↔remote, remote A↔remote B, N=2/N=3, other-tab target |
| 40 parent navigation | identity-based local/remote sorted/filtered/delayed fixtures; root, removed/hidden child, cancel/failure |
| 41 readiness/reconnect | transport/attach/replay/ready/corrupt transitions; cross-platform corrupt and retry/cancel fail-closed restore with sibling retention, daemon-backed replacement, and no inert actions; route/PTY/generation match; input gating/no hidden queue; missing-claim rejection before open/attach success; bounded logical-open waiters and Abort/disconnect cleanup; managed capability rejection before Accepted/InFlight lookup while capable retry classification remains idempotent; delayed-preflight cancellation after both ACK windows; accepted-hit/conflict ordering under project mutation; first-ACK existing-managed attach; same-host multi-account bulk launch attempts; late-claim owner-preservation/focus/no-second-identity; localized terminal and managed-open failure; real Same-SHA client/daemon smoke |
| 42 shared visual language | source checks for theme/component reuse; normal/narrow HTML states; native light/dark/contrast screenshots with long identities and mixed panes |

Run formatting and repository static checks locally. Binding host policy forbids local Cargo
builds/tests; `cargo check` and Rust test execution therefore require separately approved CI. This
constraint must be reported honestly rather than represented as a passing build.

## 8. Implementation order and file ownership

1. Freeze PRODUCT, TECH, visual HTML, and Stage 1 audit before product code.
2. Preserve existing correct behavior with focused tests; then establish pane-local route identity,
   readiness/reconnect truth, full-area geometry, title precedence, and focus/restore seams.
3. Build pane-local mixed-host launch on those seams; integrate existing drag without duplicating
   session or layout ownership.
4. Correct account roots and Claude live/dormant discovery; complete active-host reconciliation and
   four-level grouping with pure tests.
5. Integrate Connections, direct favorite-host launch plus side flyout, compact tree, account
   add-or-focus behavior, identity/meters, and waiting-row tint.
6. Integrate File Manager mode/function bar, exact multi-host transfer targets, and `..` selection;
   retain all existing transfer guards.
7. Repeat the reference audit after discovery and after final UI; run static/spec checks and review
   every modified line against #160 and #459–#464.

Parallel work must use non-overlapping file ownership. Shared files are integrated serially after
the owning task finishes.

## Risks and mitigations

- **Connection churn during inventory fetch:** generation-scoped apply plus synchronous last-host
  removal prevents stale reappearance.
- **Old or failing remote peer:** retain the connected root and distinguish unsupported/unavailable
  inventory from disconnection.
- **Missing PTY metadata:** isolate each agent in a stable fallback container; never group by label.
- **Registry/transcript schema drift:** fixture both known shapes, exclude unknown helpers, and make
  source errors visible.
- **Animation noise or reflow:** animate only paint properties inside fixed bounds and cap ring size.
- **Favorite/row click collision:** use the existing compact action component and explicit event
  propagation boundaries.
- **Async split/focus drift:** capture tab/pane/direction/host before opening UI and revalidate before
  mutation; never fall back to current focus.
- **Route loss across drag/restore:** persist and assert the exact host/daemon/PTY/generation and
  move the existing pane entity rather than recreating it.
- **False reconnect readiness:** gate input on protocol/PTy evidence, not replay output; retain
  retry/cancel and phase-specific failure.
- **Ambiguous managed open:** bind liveness to the current delivery attempt, deduplicate logical
  waiters, cancel preflight when its final current requester leaves, and require an authoritative
  generation-and-agent attach before every managed launch reports success.
- **Late PTY ownership collision:** claim before attach, retain the existing owner, and discard the
  provisional surface before any identity write.
- **Stale transfer target:** resolve exact pane/session/host/generation/mode/path immediately before
  execution and fail safely on change.
- **Private audit leakage:** record only public revisions, sanitized schemas, and repository-relative
  evidence; never include local hostnames, credentials, paths, or transcript content.

## Umsetzung der Release-Review-Korrekturen

`CockpitModel` publiziert die lokale Teilansicht zuerst. `FuturesUnordered` sammelt parallele Host-Abfragen in Abschlussreihenfolge; die drei Inventar-RPCs pro Host laufen ebenfalls parallel. Ein Ergebnis ersetzt nur die Wurzel seiner exakten Daemon-ID und deren Managed-Fleet-Daten. Jede Veröffentlichung prüft die aktuelle Refresh-Generation und die Registry-Zuordnung. `RefreshSingleFlight` bleibt bis zum Abschluss aller begrenzten Host-Abfragen reserviert.

Der Remote-Client bietet zeitschrankenfähige Listenmethoden. Die Frist umfasst Ausgangsqueue und Antwort, und eine RAII-Korrelation entfernt Pending-Einträge auch bei Abbruch. Ein Timeout sendet best-effort Abort; spätere Antworten können keine neue Anfrage erfüllen. Die übrigen Aufrufer behalten ihre bisherige Standardfrist.

`ssh_manager::panel::init` registriert ausschließlich im fokussierten Verbindungsbaum aktive Keybindings. Der gemeinsame Session-Renderer hält primäre Identität flexibel und Öffnen-Aktion fest; Statuswechsel erzeugen keine Zusatzzeile. Der Cockpit-Link öffnet die vorhandene Verbindungen-Ansicht. `adopt_daemon_session` reicht denselben echten Installationsfortschrittskanal wie eine neue Verbindung durch.

SFTP lädt bekannte Schlüssel zeilenweise in voneinander getrennte libssh2-Sammlungen. Aliasaufteilung und Hash-/Port-Matching bleiben bei libssh2; Verifikationsansichten schreiben keine normalisierten oder verlustbehafteten Daten zurück. Tests enthalten identische Schlüssel gehashter Endpunkte, kurze Aliase, große fremde Inventare, Ports, Marker und nicht-UTF8-Kommentare.
