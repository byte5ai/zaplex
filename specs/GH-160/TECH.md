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

The tab `+` menu in `app/src/workspace/view.rs` continues to query `FavoritesStore` and resolve its
stable references against the SSH registry. Its first level is a quiet favorite-host entry; launch
commands stay in the submenu. It never reads the Cockpit tree and never creates a duplicate host.

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

`CockpitModel` subscribes to `RemoteServerManagerEvent`. `HostConnected` starts a refresh.
`HostDisconnected` first removes the stable daemon id synchronously from the visible tree, emits an
update, and then starts a refresh. Existing refresh-generation gating prevents an older in-flight
result from re-adding the disconnected root. Session-level events do not add duplicate roots.

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

## 6. Localization and documentation

English and German copy cover Connections, the Sessions tree, honest local/ready-empty,
unsupported, unavailable, loading, and degraded states. Do not use a `0 accounts` header as a
loading/error substitute.

The binding visual contract is
[`docs/ui/cockpit-sidebar-connections.html`](../../docs/ui/cockpit-sidebar-connections.html).
Historical Cockpit documents are amended only where needed to mark the old combined registry/live
tree and three-level hierarchy as superseded by GH-160.

## 7. Verification map

| PRODUCT behavior | Verification seam |
|---|---|
| 1–3 Connections/favorites/menu | SSH row projection tests and existing stable-favorite/menu tests |
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

Run formatting and repository static checks locally. Binding host policy forbids local Cargo
builds/tests; `cargo check` and Rust test execution therefore require separately approved CI. This
constraint must be reported honestly rather than represented as a passing build.

## 8. Implementation order and file ownership

1. Freeze PRODUCT, TECH, visual HTML, and Stage 1 audit.
2. Correct account roots and Claude live/dormant discovery.
3. Complete active-host reconciliation and four-level grouping with pure tests.
4. Integrate Connections and favorite/menu projections.
5. Integrate compact tree, pulse, account identity, meters, and waiting-row tint.
6. Repeat the reference audit after discovery and after final UI; run static/spec checks and review
   every modified line against GH-160.

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
- **Private audit leakage:** record only public revisions, sanitized schemas, and repository-relative
  evidence; never include local hostnames, credentials, paths, or transcript content.
