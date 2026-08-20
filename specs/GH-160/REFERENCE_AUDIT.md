# GH-160 reference parity audit

This ledger is refreshed four times: baseline before implementation, after account/session
discovery changes, after UI integration, and after the follow-up parity implementation tranche.
Reference repositories are read-only idea and parity sources; they are never runtime dependencies.

## Stage 1 — baseline

Audited: 2026-08-19

All repositories were fetched immediately before inspection. The checked-out reference branches
matched their fetched default branches exactly.

| Repository | Branch | Audited revision |
|---|---|---|
| `zaplex` | `origin/main` | `5e094c567ee44634364b6c1910cf92f00c7e5148` |
| `claudeplex` | `origin/main` | `8c2041ff68d97463aed7aeb01da0f16b708b8e22` |
| `claudeplex-desktop` | `origin/main` | `8c0aad0a944a8f5b6a26636d0827db57ca22d0f3` |

Fresh Git state does not guarantee current external protocol coverage. Both reference repositories
predate the current Claude registry shape, so Zaplex also needs explicit fixtures for observed
legacy and current schemas.

### Baseline matrix

| Area | Reference evidence | Zaplex requirement / baseline classification |
|---|---|---|
| Claude account roots | The TUI merges default Claude directories, the current environment, and config roots observed from live processes; `src/discover.ts:1-10,31-38,59-70,121-182`. The desktop has similar sources but an older default metadata-path assumption in `src/core/discover.ts:36-40,74-77,107-165`. | Use the TUI default-root split as the stronger reference. Preserve deterministic default and `CLAUDE_CONFIG_DIR` pinning, canonical deduplication, and honest errors. Live-process-only custom roots are a parity candidate, not permission to add unsafe process spawning. |
| Codex account roots | Neither reference implements Codex or `CODEX_HOME`. | Zaplex extension: document and test default and pinned Codex roots independently of the Claude reference. |
| Live Claude sessions | Both references join registry entry, live PID, and matching transcript; TUI `src/collect.ts:560-599,636-683`, desktop `src/core/collect.ts:398-438,474-521`. | Keep real live conversation classification and exclude registry helpers without a transcript. Current `kind`-based entries must not be discarded merely because legacy `status` is absent. |
| Dormant Claude history | Both scan transcript history independently of the live registry and keep a bounded recent substantial set; TUI `src/collect.ts:84-90,489-517,685-718`, desktop `src/core/collect.ts:84-90,327-355,523-545`. | Open regression on `origin/main`: transcript history must remain resumable from account detail after its registry row disappears, while staying out of the live tree. |
| Transcript metadata | The TUI derives cwd, branch, title fallback, activity, model, context, thinking, and turn end from JSONL; `src/collect.ts:159-277`. Desktop adoption can recover owner and cwd from a session transcript; `electron/agents.ts:188-232`. | Preserve stable session identity and project grouping. Only provider and model belong in compact tree leaves; context and richer metadata belong in detail. |
| Status semantics | Busy is active, a live background job is monitor, a completed assistant turn is waiting, otherwise monitor; history is stale. TUI `src/collect.ts:602-605,657-716`; desktop `src/core/collect.ts:440-443,495-545`. | Use transcript/process facts rather than age alone. Map into Zaplex waiting/working/idle semantics without copying compact-tree status words. |
| Attention | The TUI tracks transitions and flashes only after first observation; `src/tracker.ts:32-72`, `src/index.ts:36-59,823-826`. Desktop prioritizes waiting rows but its exported tracker is not wired; `src/views/SessionsMain.tsx:5-30`. | Zaplex-specific final design: subtle full detail-row tint and a visible restrained amber glyph pulse; never infer parity from the unused desktop tracker. |
| Status glyphs | Central glyph/color mapping in TUI `src/render.ts:186-192,266-282`; desktop uses `● / ◐ / ◷ / ○` and duplicates words in `src/components/SessionRow.tsx:5-21,63-80`. | Adopt compact glyph semantics but intentionally omit visible status words in the sidebar tree. Keep text in detail tables and accessible labels. |
| Provider/account identity | Both products are Claude-only. The TUI shows login/email over plan/role/org; `src/render.ts:318-329`. Desktop account and detail headers omit provider; `src/views/OverviewMain.tsx:146-166`, `src/views/AccountDetail.tsx:37-45`. | Intentional Zaplex extension: visible `Claude` or `Codex` headline, account/email/plan below, no duplicated identity. |
| Usage and cost provenance | TUI token usage comes from transcript events; cost is a local list-price estimate, `src/usage.ts:1-5,28-47,57-65`. Desktop prefers Anthropic subscription usage with an explicit source and falls back to plan/transcript estimation; `electron/usage.ts:1-13,38-61,83-135`, `electron/main.ts:77-123`. | Keep subscription load, token facts, and estimated cost visibly distinct. Unknown pricing is never exact zero. |
| Tree hierarchy | TUI implements host to nested project/cwd to session, with Claude implicit; `src/index.ts:432-519`. Desktop has no host/project tree. | Final Zaplex hierarchy is `Host → Project → PTY session → Agent`, a deliberate multi-provider extension. |
| Remote host visibility | TUI discovers configured hosts without connecting and leaves remote session collection unfinished; `src/hosts.ts:1-6,106-137`, `src/index.ts:454-539`. Desktop has no remote model. | Do not copy the reference tree. Zaplex keeps registry hosts in Connections and shows a remote Cockpit root only during at least one open Zaplex session. |
| Favorites and launch | References have no host-favorite model. TUI fresh/resume pins account and cwd; `src/agents.ts:75-89`. Desktop can choose the least-loaded account; `src/views/NewAgentWizard.tsx:11-16,36-83,133-195`. | Host favorites, tab projection, and connection state are Zaplex-specific. Existing least-loaded account routing remains separate from host-favorite presentation. |

## Stage 2 — after discovery and lifecycle changes

Audited: 2026-08-19T19:53:20+02:00

All three remotes were fetched again immediately before this stage. Their default-branch revisions
were unchanged from Stage 1:

| Repository | Branch | Audited revision |
|---|---|---|
| `zaplex` | `origin/main` | `5e094c567ee44634364b6c1910cf92f00c7e5148` |
| `claudeplex` | `origin/main` | `8c2041ff68d97463aed7aeb01da0f16b708b8e22` |
| `claudeplex-desktop` | `origin/main` | `8c0aad0a944a8f5b6a26636d0827db57ca22d0f3` |

The Zaplex implementation under audit is the uncommitted GH-160 worktree based exactly on the
recorded Zaplex revision; no claim below is inferred from a newer local `main`.

| Area | Classification after discovery/lifecycle work | Reproducible Zaplex evidence |
|---|---|---|
| Claude default and pinned roots | **Fixed regression with intentional remaining gap.** Default and sorted sibling roots plus `CLAUDE_CONFIG_DIR` are canonicalized and deduplicated by path/stable identity; unreadable/malformed roots degrade health. The claudeplex live-process source remains a documented gap because adding process inspection would expand this filesystem-only scope. | `crates/zaplex_cockpit/src/claude.rs` (`discover_accounts_with_health`); fixtures in `claude_tests.rs` for pinned, canonical alias, stable identity, malformed, and unreadable roots. |
| Codex default and pinned roots | **Zaplex extension fixed.** Neither reference implements Codex. Zaplex now unions the default root with `CODEX_HOME`, canonicalizes and deduplicates them, and carries discovery issues into `ScanHealth`. | `crates/zaplex_cockpit/src/codex.rs` (`discover_account_roots`), `codex_tests.rs`, and `app/src/cockpit/model.rs` (`codex_home`, `CODEX_HOME`). |
| Accounts independent of sessions | **Fixed regression.** A valid discovered identity produces an account even with zero sessions; pending/degraded source failures remain distinct from an honest loaded zero. | `crates/zaplex_cockpit/src/lib.rs` discovery-before-session loops and health tests; Claude/Codex root fixtures. |
| Current and legacy Claude registry | **Fixed regression / selective parity.** Legacy valid status remains accepted; current status-less `interactive` and `bg` conversations are accepted; status-less unknown/shell helpers remain excluded. | `crates/zaplex_cockpit/src/sessions.rs` (`is_real_reg`) and `sessions_tests.rs` current, legacy, background, shell, and unknown-helper fixtures. |
| Dormant Claude history | **Reference parity restored.** Bounded recent substantial transcript history is discovered independently of a registry row and returned only as idle account detail data. Invalid cwd and observer/memory infrastructure are excluded. | `crates/zaplex_cockpit/src/sessions.rs` (`scan_transcript_history`) and `sessions_tests.rs` registry-cleanup, substance, cwd, and observer cases. |
| Live remote roots | **Intentional Zaplex integration fixed.** Unlike the incomplete reference remote model, Zaplex retains local plus actually connected remote roots only. Empty, unsupported, and unavailable inventory do not impersonate disconnection. | `app/src/cockpit/model.rs`, `crates/zaplex_cockpit/src/fleet.rs`; first/last/stale-generation and inventory-state tests in their paired test files. |
| PTY grouping and exact routing | **Intentional multi-provider extension fixed.** Agents group by full PTY id and generation; missing PTY metadata receives an isolated stable fallback based on exact session identity. | `crates/zaplex_cockpit/src/conductor.rs` and `conductor_tests.rs` grouping, generation, fallback, representative, and stable-order cases. |
| Compact status and account presentation | **Deferred to Stage 3.** Discovery supplies truthful provider/account facts, but the final glyph-only tree, pulse, provider headline, waiting row tint, and Connections projection are classified only after UI integration. | Stage 3 visual/source audit pending. |

No Cargo build or Rust test execution is claimed: binding host policy forbids local builds. The
evidence above is fixture/source coverage plus `rustfmt --check` and `git diff --check`; executable
verification requires separately approved CI.

## Stage 3 — after UI integration

Audited: 2026-08-19T20:06:01+02:00

All three remotes were fetched with pruning immediately before this review. The reference default
branches and Zaplex `origin/main` were still unchanged:

| Repository | Branch | Audited revision |
|---|---|---|
| `zaplex` | `origin/main` | `5e094c567ee44634364b6c1910cf92f00c7e5148` |
| `claudeplex` | `origin/main` | `8c2041ff68d97463aed7aeb01da0f16b708b8e22` |
| `claudeplex-desktop` | `origin/main` | `8c0aad0a944a8f5b6a26636d0827db57ca22d0f3` |

The implementation reviewed below remains the uncommitted GH-160 worktree based exactly on the
recorded Zaplex revision.

| Area | Final classification | Reproducible Zaplex evidence |
|---|---|---|
| Connections and favorites | **Intentional Zaplex extension completed.** Registered hosts, favorite state, and live connection state remain separate projections. Every connected registry alias is preserved even when two aliases reach one daemon. The tab menu resolves favorite registered hosts only and leaves curation in Connections. | `RemoteServerManager::connected_registry_hosts`; `app/src/ssh_manager/panel.rs`; `app/src/workspace/view.rs` and focused menu/sidebar tests. |
| Live host lifecycle | **Intentional Zaplex extension completed.** Local remains present. Remote roots are derived from live daemon connections, survive honest empty/unsupported/unavailable inventory, and disappear after the last disconnect with stale refresh generations gated. | `app/src/cockpit/model.rs`, `crates/zaplex_cockpit/src/fleet.rs`, and their first/last/stale/inventory fixtures. |
| Four-level hierarchy | **Intentional multi-provider extension completed.** The reference TUI stops at Claude sessions. Zaplex groups exact PTY id plus generation into `Host → Project → PTY Session → Agent`; missing PTY metadata is isolated by full session identity. | `crates/zaplex_cockpit/src/conductor.rs`, `conductor_tests.rs`, and `app/src/cockpit/panel.rs`. |
| Compact status presentation | **Intentional divergence completed.** Unlike the reference surfaces, the expanded tree omits visible waiting/working/idle words and fixed metric columns. Agent leaves contain only the state glyph, provider, and optional model; collapsed counts alone carry hidden attention. | `app/src/cockpit/panel.rs` presentation helpers/tests and the normal/narrow states in `docs/ui/cockpit-sidebar-connections.html`. |
| Waiting attention | **Reference idea adapted.** Waiting-first ordering is retained, while Zaplex adds a restrained 1.6-second amber glyph pulse capped at twice the core diameter and a static reduced-motion state. Detail rows use a low-opacity whole-row amber tint while retaining explicit Status text. | `waiting_pulse_frame` tests in `panel_tests.rs`; optional table row background API and test; `table_row_needs_attention` test; HTML animated and row-tint states. |
| Provider/account identity | **Zaplex multi-provider regression fixed.** Both compact and large cards use the provider as the headline and one account-or-email-plus-plan subline. Provider color is supplementary. Generic pane chrome prevents a duplicate provider strip. | `account_identity` and tests; consumers in `panel.rs` and `pane.rs`; HTML Claude and Codex cards. |
| Usage windows | **Preserved.** Sidebar cards retain both five-hour and weekly meters; the large detail keeps richer usage/cost facts without duplicating identity. | `panel.rs::render_card`, `pane.rs::render_account_detail`, and HTML account states. |
| Claude live/dormant discovery | **Reference parity restored selectively.** Current and legacy real registry conversations remain eligible; recent substantial transcript-only history remains resumable in account detail and never enters the live tree. | `crates/zaplex_cockpit/src/sessions.rs` and live/helper/history fixtures in `sessions_tests.rs`. |
| Account roots | **Fixed with one remaining reference gap.** Default, sibling, and pinned Claude roots plus default/pinned Codex roots are deterministic and deduplicated. Zaplex does not yet inspect live process environments for additional Claude config roots as claudeplex does; that remains outside this filesystem-only change. | `claude.rs`, `codex.rs`, `lib.rs`, `model.rs`, and paired fixtures; remaining gap recorded explicitly rather than treated as parity. |
| Responsive/accessibility contract | **Implemented at source/spec level.** Normal and narrow layouts preserve hierarchy and controls; state has distinct shapes/tooltips, and reduced motion schedules no animation repaint. | `docs/ui/cockpit-sidebar-connections.html`, `panel.rs`, `AccessibilitySettings::reduce_motion`, and pure pulse/count tests. |

No Cargo build or Rust test execution is claimed. Binding host policy forbids local builds; this
stage used fresh-source comparison, focused fixture review, `rustfmt --check`, `git diff --check`,
SVG/static checks, and rendered HTML inspection. Executable verification still requires separately
approved CI.

## Stage 4 — follow-up parity implementation

Audited: 2026-08-20T01:38:35+02:00

All three remotes were fetched with pruning immediately before this stage. The implementation
worktree is still based exactly on current `zaplex` `origin/main`; both reference checkouts match
their freshly fetched default branches:

| Repository | Branch | Audited revision |
|---|---|---|
| `zaplex` | `origin/main` | `5e094c567ee44634364b6c1910cf92f00c7e5148` |
| `claudeplex` | `origin/main` | `8c2041ff68d97463aed7aeb01da0f16b708b8e22` |
| `claudeplex-desktop` | `origin/main` | `8c0aad0a944a8f5b6a26636d0827db57ca22d0f3` |

| Area | Stage-4 classification | Reproducible Zaplex evidence |
|---|---|---|
| Live-process Claude roots | **Reference parity restored on Linux with a stricter trust boundary.** Same-UID Claude processes contribute config roots through bounded `/proc` inspection with PID/start-time revalidation. Permission, partial, and unsupported-platform states remain visible as degraded health rather than silently inventing completeness. | `crates/zaplex_cockpit/src/claude.rs` and process-race/UID/permission fixtures in `claude_tests.rs`. |
| Machine-readable state | **Zaplex extension completed.** `zaplex cockpit snapshot --json` exports the same loaded runtime fold through the authenticated per-surface IPC service; offline local fallback is explicitly degraded and unknown usage remains `null`. | `crates/warp_cli/src/cockpit.rs`, `app/src/control_surface.rs`, and CLI/runtime/auth fixtures. |
| Exact launch binding | **Zaplex extension completed.** Launch intent is created before command execution and promoted to exact host/provider/account/session identity regardless of terminal-first or hook-first event order; stale transport state is bounded and cleared. | `app/src/cockpit/launch_registry.rs`, terminal/workspace bridge code, and ordering/reuse/eviction fixtures. |
| Remote account routing | **Zaplex extension completed beyond both references.** New peers exchange opaque daemon account ids only. Start, resume, fork, slash, and attach revalidate the exact route; missing, stale, ambiguous, or cross-provider ids fail closed. Host config paths remain daemon-local, with capability-gated legacy compatibility only. | Additive remote proto fields, `app/src/remote_server/agent_account.rs`, Workspace/Spawn routing, and opaque/collision/legacy fixtures. |
| Local and remote transcripts | **Multi-provider extension completed at source level.** Claude/Codex remote actions require an exact connected capable daemon and are revalidated in the handler. Local Codex and remote sources use bounded, symlink-resistant handle-bound reads; remote concurrency is capped. Only provider-neutral display fields reach a pathless read-only document, with distinct missing/empty/unsupported/malformed/too-large/unavailable states. | `codex_sessions.rs`, `transcript_rpc.rs`, Cockpit pane/client/proto integration, and traversal/replacement/limit/status/redaction fixtures. |
| Transcript document lifecycle | **Regression fixed during independent review.** Generated documents are selectable, non-editable, non-restorable, never filesystem-backed, show their real provider title, and cannot be merged or dragged into a file-backed CodeView where save/restore semantics would be corrupted. A rejected merge never removes the source pane. | `CodeSource::GeneratedReadOnly`, `code/view.rs`, Workspace drop guard, and generated-title/non-persistence/no-merge fixtures. |
| Continuous parity | **Gate implemented; executable result pending CI.** The PR workflow refreshes both references fail-closed, validates 47 named source tests across nine scenarios, runs focused Cargo suites, and renders normal/narrow/reduced-motion screenshots. The real authenticated two-host lifecycle remains a manual smoke by design. | `.github/workflows/cockpit-parity-audit.yml`, `script/cockpit-parity-audit`, `specs/parity/cockpit-matrix.json`, screenshot spec, and runtime checklist. Local contract validation and all seven negative self-tests pass. |

No local Cargo build/test or real two-host smoke is claimed. Binding policy reserves executable Rust
verification for CI. Remote transcript revision polling is protocol-ready but the current Cockpit
action intentionally opens a one-shot snapshot; this is not a reference regression because neither
reference implements remote multi-provider transcripts, but it remains a possible UX follow-up.

## Repeat procedure

1. Fetch all three remotes and record default branch, exact revision, and audit time.
2. Re-run the matrix against code, not documentation alone.
3. Inspect only schema keys and file relationships in sanitized fixtures; never record credentials,
   personal paths, or transcript content.
4. Classify every row as parity, fixed regression, intentional divergence, or remaining gap.
5. Map Zaplex claims to focused tests or a screenshot/spec check before closing the stage.
