# Dynamic Cockpit Command Palette source

## Context

The required behavior is in [PRODUCT.md](./PRODUCT.md). The existing palette combines model-backed `SyncDataSource` and `AsyncDataSource` instances in `app/src/search/command_palette/data_sources.rs`; results emit `CommandPaletteItemAction` from `mixer.rs` and are handled in `view.rs`. `CockpitModel` in `app/src/cockpit/model.rs` owns the generation-checked `CockpitSnapshot` plus `FleetTree` and emits `CockpitEvent::Updated`. Stable cross-host session identity already exists as `zaplex_cockpit::session_key`.

The desktop reference's palette indexes accounts and GitHub flows but does not provide Zaplex's cross-host/session identity guarantees. The Zaplex design extends it to all four requested object classes.

## Proposed changes

1. Add `app/src/cockpit/palette.rs` as a UI-independent index builder. It defines:
   - `CockpitPaletteTarget`, a serializable/debuggable stable target enum for Account, Session, Host, Project, and GitHubFlow;
   - `CockpitPaletteRecord`, containing safe search text, display labels, capability, priority, and target;
   - `build_palette_index`, which folds the current snapshot/fleet and optional repository context without reading UI state;
   - explicit stable-key functions and waiting-aware rank tiers.

2. Session records use `session_key(is_local, host_id, session)` and retain the routing tuple required by existing `WorkspaceAction::AttachFleetSession`. Dormant sessions come only from local account snapshots and dedupe against live keys. Config paths are retained only inside the execution target, never in indexed text or display strings.

3. Host/project keys use `host_key`/stable daemon id plus project root. Only available roots are indexed. GitHub flow keys combine repository slug, absolute worktree identity, and the stable flow key from `github_flows.rs`.

4. Add `app/src/search/command_palette/cockpit/` with a synchronous source and result renderer. The source fuzzy-matches the safe text, applies bounded score boosts for exact class/label matches, then applies the waiting tier without overriding unrelated higher-quality matches. The renderer reuses the Command Palette's shared icon and text primitives.

5. `DataSourceStore` registers the Cockpit source for unfiltered and Actions searches and lets the source runtime-gate itself to an empty result while Cockpit is disabled. Keeping it registered lets an enable/rescan event populate an already-open palette. The store subscribes to `CockpitEvent::Updated`, retains the active mixer handle, and reruns the current query. `SearchMixer`'s monotonic query generation discards stale async completions, satisfying the live generation contract.

6. `CommandPaletteItemAction` receives one `RunCockpitTarget` variant, and `ItemSummary` stores only its stable target key for recents. The palette view dispatches a single `WorkspaceAction::RunCockpitPaletteTarget` and closes normally.

7. `WorkspaceView::run_cockpit_palette_target` re-resolves the stable target against the current `CockpitModel` immediately before execution. It then uses existing methods: account pane focus, `AttachFleetSession`, or `OpenSpawnCard`. GitHub flows additionally revalidate repository context through GH-161.

8. Capability evaluation is shared between indexing and execution. A stale result accepted during a refresh is rejected visibly after revalidation rather than retargeted or silently ignored.

## Testing and validation

- `app/src/cockpit/palette_tests.rs` maps Behavior 2-8 and 11-12: duplicate labels, copied session IDs across hosts/accounts, dormant/live dedupe, safe searchable text, disconnected-host removal, capability gating, and stable GitHub keys.
- `app/src/search/command_palette/cockpit/data_source_tests.rs` maps Behavior 1, 5, and 9-10: fuzzy search, waiting-before-idle for equivalent matches, bounded results, and newer generation replacement.
- Existing Command Palette view tests are extended for Behavior 13-14: Up/Down/Enter acceptance and accessibility text for each object class.
- A workspace unit test verifies that target revalidation dispatches the exact existing Account/Attach/Spawn/GitHub path and rejects a removed host/session.
- CI runs `cargo check` plus focused Cockpit and Command Palette suites. Manual verification keeps the palette open while connecting/disconnecting a remote host and confirms rows appear/disappear without reopening.

## Risks and mitigations

- The same session can appear in snapshot and fleet. Stable-key deduplication keeps one live row and adds only genuinely dormant rows.
- Palette refreshes can race file-search sources. Re-running the current query increments the existing mixer generation, which already rejects stale asynchronous callbacks.
- Route fields contain local config paths. Search/display projections are built separately from execution targets and tests assert those paths never enter index text or accessibility labels.
