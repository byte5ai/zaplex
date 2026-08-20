//! `CockpitModel` — the singleton that holds the latest [`CockpitSnapshot`] and keeps
//! it fresh, emitting [`CockpitEvent::Updated`] on change.
//!
//! Refresh is driven by three sources (mirrors `file_mcp_watcher` + the daemon GC
//! timer):
//! - [`HomeDirectoryWatcher`] (top-level home changes) → catches account add/remove
//!   (`~/.claude.json`, `~/.claude`, `~/.codex`).
//! - first/last remote-host connection events → update the live host roots immediately.
//! - a periodic **reconcile tick** → catches usage growth (transcripts append deep in
//!   `projects/**` / `sessions/**`, which the non-recursive home watcher never sees)
//!   and window/reset rollover.
//!
//! The (blocking) disk scan runs on the background executor; results are applied back
//! on the model's thread via the spawner round-trip.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_compat::CompatExt as _;
use chrono::Utc;
use warpui::{Entity, ModelContext, SingletonEntity};
use watcher::HomeDirectoryWatcher;
#[cfg(test)]
use zaplex_cockpit::HostNode;
use zaplex_cockpit::{
    apply_oauth_usage, build_snapshot_with_cache, fold_inventory, session_key, AccountOverrides,
    AgentInventoryStatus, CockpitSnapshot, FleetTree, PricingTable, Provider, RegisteredHost,
    RemoteHost, ScanHealth, SessionSnapshot, TranscriptScanCache,
};
// Cross-host daemon fold is a native-only concern: the `agent_session` module
// (and the whole remote-daemon layer it lives in) is `#[cfg(not(wasm))]`, and a
// WASM build has no daemon connections at all. On WASM the fold degrades to the
// local tree, so these imports — used only by the remote-fetch block below —
// are gated to match.
#[cfg(not(target_family = "wasm"))]
use zaplex_remote_session::types::{
    has_feature, FEATURE_AGENT_INVENTORY, FEATURE_AGENT_PTY_BINDING_V2,
};

use crate::cockpit::oauth::{self, CachedOauth};
use crate::cockpit::settings::CockpitSettings;
#[cfg(not(target_family = "wasm"))]
use crate::remote_server::agent_session::proto_to_snapshot;
use crate::remote_server::manager::{
    ConnectedDaemon, RemoteServerManager, RemoteServerManagerEvent,
};

#[cfg(not(target_family = "wasm"))]
fn retain_negotiated_agent_pty_routes(features: &[String], sessions: &mut [SessionSnapshot]) {
    if has_feature(features, FEATURE_AGENT_PTY_BINDING_V2) {
        return;
    }
    for session in sessions {
        session.pty_session_id = None;
        session.pty_session_generation = None;
        session.pty_foreground = false;
    }
}

/// How often to re-scan transcripts even when no top-level home change fired.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(45);

/// Emitted whenever the snapshot changes.
pub enum CockpitEvent {
    /// One or more sessions transitioned from working to WAITING — they need
    /// the user. Carries "account-label — session/cwd" display strings.
    SessionsBecameWaiting(Vec<String>),
    Updated,
}

/// Where the user's account overrides live. One derivation, so the reader (the
/// off-thread refresh) and the writer (`set_alias`) can never point at different
/// files — which would look exactly like an alias that does not stick.
fn instances_path(home: &std::path::Path) -> PathBuf {
    home.join(".zap").join("instances.json")
}

fn initial_snapshot() -> CockpitSnapshot {
    CockpitSnapshot {
        accounts: Vec::new(),
        generated_at: Utc::now(),
        // No scan has run yet — the UI must show "loading", not "no accounts".
        health: ScanHealth::Pending,
    }
}

fn should_apply_refresh_result(current_generation: u64, completed_generation: u64) -> bool {
    current_generation == completed_generation
}

pub struct CockpitModel {
    snapshot: CockpitSnapshot,
    /// Monotonic identity of the newest requested refresh. Background scans can
    /// complete out of order; only the result matching this generation may
    /// replace the current snapshot.
    refresh_generation: u64,
    /// The unified cross-host Agent-Inventory: local sessions folded together
    /// with every connected daemon's sessions into one Host▸Project▸Session
    /// tree. Rebuilt on every refresh; equals the local-only tree when no
    /// daemon is connected. Read by the attention ambient-bit (`needs_me`) and
    /// the Conductor UI (`inventory`).
    inventory: FleetTree,
    pricing: PricingTable,
    /// Per-account real-usage cache (C3b), keyed by the account's config dir.
    /// Lives here so the 15-min TTL survives across refresh cycles; the token
    /// itself is never stored — only the parsed, secret-free usage numbers.
    oauth_cache: HashMap<PathBuf, CachedOauth>,
    /// Bounded parse cache for structured agent task state and Codex rollout
    /// metadata. Survives reconcile ticks so unchanged transcripts are not
    /// reopened every 45 seconds.
    transcript_cache: TranscriptScanCache,
    /// User overrides (instances.json: label/color/order/hide), applied to every
    /// snapshot. Kept here so the card renderer can look up per-account colors
    /// (color isn't an `Account` field). Empty when the file is absent/broken.
    overrides: AccountOverrides,
    /// Label of the local host in [`Self::inventory`] — the machine hostname (or
    /// `"local"` when unavailable). The Conductor uses it to tell which host node
    /// is *this* machine, so its sessions can be adopted in place (a remote
    /// host's sessions resume on that host, not locally).
    local_label: String,
    /// The account the user has selected in the sidebar (its `account.key`), so
    /// the dashboard pane shows that account as the detail focus and the sidebar
    /// carries a stable highlight (WS4 S5: click → selection → detail in the
    /// pane). `None` = nothing selected. Cleared when the account disappears.
    selected_account: Option<String>,
}

/// Inputs captured on the model thread, moved into the off-thread build.
struct RefreshInputs {
    home: PathBuf,
    codex_home: PathBuf,
    claude_config_dir_env: Option<String>,
    budget_5h: u64,
    budget_week: u64,
    pricing: PricingTable,
    /// `cockpit.oauth_usage` — when off, no usage requests happen at all.
    oauth_enabled: bool,
    /// Cache state moved into the build; the (possibly refreshed) cache comes
    /// back with the snapshot via `apply`.
    oauth_cache: HashMap<PathBuf, CachedOauth>,
    /// Transcript parse cache moved through the background scan and returned
    /// with the accepted refresh result.
    transcript_cache: TranscriptScanCache,
    /// Path to the user's `instances.json` account overrides (read off-thread).
    instances_path: PathBuf,
    /// Live daemon connections captured on the model thread, moved into the
    /// off-thread build so each host's agent-inventory can be fetched and folded
    /// in. Empty when no daemon is connected (fold = local-only tree).
    daemons: Vec<ConnectedDaemon>,
    /// Label for the local host in the folded tree — the machine hostname, or
    /// `"local"` when it can't be determined.
    local_label: String,
}

fn codex_home(home: &Path, configured: Option<std::ffi::OsString>) -> PathBuf {
    configured
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
}

impl CockpitModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Account add/remove (top-level home entries).
        ctx.subscribe_to_model(&HomeDirectoryWatcher::handle(ctx), |me, _event, ctx| {
            me.spawn_refresh(ctx);
        });
        ctx.subscribe_to_model(
            &RemoteServerManager::handle(ctx),
            |me, event, ctx| match event {
                RemoteServerManagerEvent::HostConnected { .. } => me.spawn_refresh(ctx),
                RemoteServerManagerEvent::HostDisconnected { host_id } => {
                    if remove_disconnected_host(&mut me.inventory, host_id.as_str()) {
                        ctx.emit(CockpitEvent::Updated);
                    }
                    me.spawn_refresh(ctx);
                }
                RemoteServerManagerEvent::SessionConnecting { .. }
                | RemoteServerManagerEvent::SessionConnected { .. }
                | RemoteServerManagerEvent::SessionConnectionFailed { .. }
                | RemoteServerManagerEvent::SessionDisconnected { .. }
                | RemoteServerManagerEvent::SessionReconnected { .. }
                | RemoteServerManagerEvent::SessionDeregistered { .. }
                | RemoteServerManagerEvent::NavigatedToDirectory { .. }
                | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
                | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
                | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
                | RemoteServerManagerEvent::BufferUpdated { .. }
                | RemoteServerManagerEvent::SetupStateChanged { .. }
                | RemoteServerManagerEvent::BinaryCheckComplete { .. }
                | RemoteServerManagerEvent::BinaryInstallComplete { .. }
                | RemoteServerManagerEvent::ClientRequestFailed { .. }
                | RemoteServerManagerEvent::ServerMessageDecodingError { .. }
                | RemoteServerManagerEvent::SessionOutput { .. }
                | RemoteServerManagerEvent::SessionExited { .. }
                | RemoteServerManagerEvent::SessionNotice { .. } => {}
            },
        );

        let mut model = Self {
            snapshot: initial_snapshot(),
            refresh_generation: 0,
            inventory: FleetTree::default(),
            pricing: PricingTable::default(),
            oauth_cache: HashMap::new(),
            transcript_cache: TranscriptScanCache::default(),
            overrides: AccountOverrides::default(),
            local_label: "local".to_string(),
            selected_account: None,
        };
        model.spawn_refresh(ctx);
        model.start_reconcile_timer(ctx);
        model
    }

    /// The latest snapshot (empty until the first background scan completes).
    pub fn snapshot(&self) -> &CockpitSnapshot {
        &self.snapshot
    }

    /// User-triggered re-scan — the empty/degraded-state "try again" action. Re-runs
    /// the same background scan the watchers and `new()` use.
    pub fn rescan(&mut self, ctx: &mut ModelContext<Self>) {
        self.spawn_refresh(ctx);
    }

    /// The account key the user has selected in the sidebar, if any (WS4 S5).
    pub fn selected_account(&self) -> Option<&str> {
        self.selected_account.as_deref()
    }

    /// Select an account: stores its `account.key` and emits
    /// [`CockpitEvent::Updated`] so the sidebar highlight refreshes.
    ///
    /// Clicking the selected account again **keeps** it selected — the click
    /// then focuses its pane (the caller opens/focuses it). It used to toggle
    /// the selection off, which read as the card fighting the user: click to
    /// look at an account, click again because its pane is behind something,
    /// and the highlight vanishes instead (spec v3 §4.1 P1). No-op if nothing
    /// actually changed.
    pub fn select_account(&mut self, key: String, ctx: &mut ModelContext<Self>) {
        let next = Some(key);
        if next != self.selected_account {
            self.selected_account = next;
            ctx.emit(CockpitEvent::Updated);
        }
    }

    /// Gather the inputs for a build, or `None` if the cockpit is disabled or the home
    /// directory is unavailable (in which case no refresh runs).
    fn refresh_inputs(&self, ctx: &mut ModelContext<Self>) -> Option<RefreshInputs> {
        if !*CockpitSettings::as_ref(ctx).enabled {
            return None;
        }
        let home = dirs::home_dir()?;
        // 0 = automatic: the spine estimates per-account budgets from the plan
        // tier (Enterprise/Team/Pro/Max) instead of one flat default.
        let budget_5h = *CockpitSettings::as_ref(ctx).budget_5h as u64;
        let budget_week = *CockpitSettings::as_ref(ctx).budget_week as u64;
        // Snapshot the live daemon connections now, on the model thread; the
        // actual (async) inventory fetch happens off-thread in `spawn_refresh`.
        // `RemoteServerManager` is a singleton registered before `CockpitModel`
        // (see `app/src/lib.rs`), so this read is always available.
        let daemons = RemoteServerManager::as_ref(ctx).connected_daemons();
        let local_label = gethostname::gethostname()
            .into_string()
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "local".to_string());
        Some(RefreshInputs {
            codex_home: codex_home(&home, std::env::var_os("CODEX_HOME")),
            claude_config_dir_env: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            instances_path: instances_path(&home),
            home,
            budget_5h,
            budget_week,
            pricing: self.pricing.clone(),
            oauth_enabled: *CockpitSettings::as_ref(ctx).oauth_usage,
            oauth_cache: self.oauth_cache.clone(),
            transcript_cache: self.transcript_cache.clone(),
            daemons,
            local_label,
        })
    }

    /// Kick off a background disk scan; applies the result on the model thread.
    fn spawn_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        // Advance before reading inputs: disabling the cockpit must invalidate a
        // scan that is already in flight as surely as starting a newer scan does.
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let generation = self.refresh_generation;
        let Some(mut inputs) = self.refresh_inputs(ctx) else {
            // Disabled (or no home dir): blank any stale state instead of
            // silently doing nothing, so the ambient badge and Conductor UI
            // don't hold onto a waiting-count from before the toggle.
            self.clear_for_disabled(ctx);
            return;
        };
        let spawner = ctx.spawner();
        ctx.background_executor()
            .spawn(async move {
                let scan_now = Utc::now();
                let mut snapshot = build_snapshot_with_cache(
                    &inputs.home,
                    &inputs.codex_home,
                    inputs.claude_config_dir_env.as_deref(),
                    scan_now,
                    inputs.budget_5h,
                    inputs.budget_week,
                    &inputs.pricing,
                    &mut inputs.transcript_cache,
                );
                // C3b: overlay real per-account utilization where available.
                // Piggybacks on this refresh (no extra timer); the 15-min TTL
                // inside `refresh_cache` keeps actual requests rare. `.compat()`
                // provides the tokio reactor reqwest needs on this executor.
                let oauth_cache = if inputs.oauth_enabled {
                    let claude_dirs: Vec<PathBuf> = snapshot
                        .accounts
                        .iter()
                        .filter(|a| a.account.provider == Provider::Claude)
                        .map(|a| a.account.config_dir.clone())
                        .collect();
                    let cache = oauth::refresh_cache(
                        claude_dirs,
                        inputs.home.join(".claude"),
                        inputs.oauth_cache,
                    )
                    .compat()
                    .await;
                    apply_oauth_usage(&mut snapshot, &oauth::usable_usage(&cache));
                    cache
                } else {
                    inputs.oauth_cache
                };
                // Apply user overrides (instances.json) last, on the fully-built
                // snapshot: hide / relabel / reorder for display. Read off-thread;
                // a missing or broken file yields no overrides (never blanks the
                // cockpit — see `AccountOverrides::parse`).
                let overrides = AccountOverrides::parse(
                    &std::fs::read_to_string(&inputs.instances_path).unwrap_or_default(),
                );
                snapshot.accounts = overrides.apply(std::mem::take(&mut snapshot.accounts));
                for account in &mut snapshot.accounts {
                    for session in &mut account.sessions {
                        session.effort = super::session_effort(session, true, None);
                    }
                }

                // Cross-host fold: every connected daemon contributes one host
                // root. Agent inventory enriches that root when available; an old
                // daemon or a request error stays visible with an honest status.
                //
                // Native only: the daemon layer (and `list_agent_sessions` /
                // `proto_to_snapshot`) is `#[cfg(not(wasm))]`. On WASM there are
                // no daemon connections, so `remotes` is empty and the fold below
                // degrades to the local tree alone.
                let live_hosts_by_registry_node: HashMap<String, String> = inputs
                    .daemons
                    .iter()
                    .filter_map(|daemon| {
                        daemon
                            .registry_node_id
                            .clone()
                            .map(|node_id| (node_id, daemon.host_id.clone()))
                    })
                    .collect();
                #[cfg(not(target_family = "wasm"))]
                let remotes: Vec<(RemoteHost, Vec<SessionSnapshot>)> = {
                    let mut remotes = Vec::with_capacity(inputs.daemons.len());
                    for daemon in inputs.daemons {
                        if !has_feature(&daemon.features, FEATURE_AGENT_INVENTORY) {
                            remotes.push((
                                RemoteHost {
                                    label: daemon.host_label,
                                    host_id: daemon.host_id,
                                    registry_node_id: daemon.registry_node_id,
                                    inventory_status: AgentInventoryStatus::Unsupported,
                                },
                                Vec::new(),
                            ));
                            continue;
                        }
                        match daemon.client.list_agent_sessions().await {
                            Ok(list) => {
                                let mut sessions: Vec<SessionSnapshot> =
                                    list.sessions.iter().map(proto_to_snapshot).collect();
                                retain_negotiated_agent_pty_routes(&daemon.features, &mut sessions);
                                for session in &mut sessions {
                                    session.effort = super::session_effort(
                                        session,
                                        false,
                                        Some(&daemon.host_id),
                                    );
                                }
                                // Carry the daemon's stable `host_id` alongside its
                                // display label so the folded inventory can route
                                // guardrails/attach by id, not by a collidable label.
                                remotes.push((
                                    RemoteHost {
                                        label: daemon.host_label,
                                        host_id: daemon.host_id,
                                        registry_node_id: daemon.registry_node_id,
                                        inventory_status: AgentInventoryStatus::Ready,
                                    },
                                    sessions,
                                ));
                            }
                            Err(e) => {
                                log::warn!(
                                    "cockpit fold: list_agent_sessions failed for host {:?}: {e} \
                                     — retaining the connected host without inventory",
                                    daemon.host_label
                                );
                                remotes.push((
                                    RemoteHost {
                                        label: daemon.host_label,
                                        host_id: daemon.host_id,
                                        registry_node_id: daemon.registry_node_id,
                                        inventory_status: AgentInventoryStatus::Unavailable,
                                    },
                                    Vec::new(),
                                ));
                            }
                        }
                    }
                    remotes
                };
                #[cfg(target_family = "wasm")]
                let remotes: Vec<(RemoteHost, Vec<SessionSnapshot>)> = {
                    // No daemon connections on WASM; the captured (empty) list is
                    // consumed here so the fold is honestly local-only.
                    let _ = inputs.daemons;
                    Vec::new()
                };
                // Local contribution: every live account session plus
                // Antigravity's per-workspace resume registry. Antigravity is
                // deliberately Idle: its disk state proves a resumable
                // conversation, not a running process. Claude/Codex dormant
                // histories remain on their existing account-detail surfaces;
                // adding all of them here would turn provider enablement into a
                // broad Conductor behavior change.
                let antigravity = zaplex_cockpit::antigravity_idle_sessions(
                    &inputs.home,
                    scan_now,
                    zaplex_cockpit::IDLE_MAX_AGE,
                    zaplex_cockpit::IDLE_SESSION_LIMIT,
                );
                let local: Vec<SessionSnapshot> = snapshot
                    .accounts
                    .iter()
                    .flat_map(|account| account.sessions.iter().cloned())
                    .chain(antigravity)
                    .collect();
                let local_label = inputs.local_label.clone();
                let mut inventory = fold_inventory(inputs.local_label, local, remotes);

                // Validate only the connected roots against the SSH registry.
                // Registry-only/offline hosts belong to Connections and are never
                // appended to the Cockpit tree. Display labels never join identities.
                let registered: Vec<RegisteredHost> = warp_ssh_manager::with_conn(|c| {
                    Ok(warp_ssh_manager::SshRepository::list_nodes(c)?)
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|n| matches!(n.kind, warp_ssh_manager::types::NodeKind::Server))
                .map(|n| {
                    let live_host_id = live_hosts_by_registry_node.get(&n.id).cloned();
                    RegisteredHost {
                        node_id: n.id,
                        label: n.name,
                        live_host_id,
                    }
                })
                .collect();
                zaplex_cockpit::reconcile_connected_hosts(&mut inventory, &registered);

                let _ = spawner
                    .spawn(move |me, ctx| {
                        if !should_apply_refresh_result(me.refresh_generation, generation) {
                            return;
                        }
                        me.apply(
                            snapshot,
                            oauth_cache,
                            inputs.transcript_cache,
                            overrides,
                            inventory,
                            local_label,
                            ctx,
                        )
                    })
                    .await;
            })
            .detach();
    }

    /// The user-overridden display color for an account key, if any (hex string
    /// like `#22C55E`; the renderer parses/validates it).
    /// Set (or clear) an account's alias, persisted to `instances.json` — the
    /// one place overrides live (A1). Returns the IO error so the caller can
    /// toast it: a write that silently did nothing would be the worst outcome.
    ///
    /// The file is watched, so the snapshot reloads on its own and the alias
    /// appears everywhere at once — card, pane title, table, spawn card — without
    /// this having to know about any of those surfaces.
    pub fn set_alias(&self, account_key: &str, alias: Option<&str>) -> std::io::Result<()> {
        let Some(home) = dirs::home_dir() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no home directory — cannot locate instances.json",
            ));
        };
        zaplex_cockpit::set_label_override(&instances_path(&home), account_key, alias)
    }

    pub fn override_color(&self, key: &str) -> Option<&str> {
        self.overrides.color_for(key)
    }

    /// Blank the model's state on the enabled→disabled transition, so every
    /// consumer of `Updated` — the ambient Dock badge (`AttentionDriver`), the
    /// Conductor pane, and the sidebar — reflects "nothing to show" instead of
    /// holding onto whatever snapshot/inventory existed right before the
    /// setting flipped off. `spawn_refresh` calls this on every disabled tick
    /// (timer + home-directory watcher), but it only actually mutates and
    /// emits once: comparing against the *current* state (rather than tracking
    /// a separate "was enabled" flag) means every later disabled tick is
    /// already blank and is a silent no-op — `Updated` never spams while
    /// disabled. Re-enabling resumes normally: the next `spawn_refresh` finds
    /// `enabled` true again and applies a live snapshot as usual.
    fn clear_for_disabled(&mut self, ctx: &mut ModelContext<Self>) {
        // Skip only when already a *settled, authoritative* blank. The extra
        // `health.is_loaded()` matters at startup: a cockpit disabled from the very
        // first tick is blank but still `Pending` (its initial state), and returning
        // early there would leave every open pane showing "loading…" forever.
        if is_blank(&self.snapshot, &self.inventory)
            && self.selected_account.is_none()
            && self.snapshot.health.is_loaded()
        {
            return; // already a settled blank — nothing changed since the last disabled tick
        }
        self.snapshot = CockpitSnapshot {
            accounts: Vec::new(),
            generated_at: Utc::now(),
            // Disabled is a deliberate, authoritative "nothing" — not a load in flight.
            health: ScanHealth::Loaded,
        };
        self.inventory = FleetTree::default();
        self.transcript_cache = TranscriptScanCache::default();
        self.selected_account = None;
        ctx.emit(CockpitEvent::Updated);
    }

    fn apply(
        &mut self,
        snapshot: CockpitSnapshot,
        oauth_cache: HashMap<PathBuf, CachedOauth>,
        transcript_cache: TranscriptScanCache,
        overrides: AccountOverrides,
        inventory: FleetTree,
        local_label: String,
        ctx: &mut ModelContext<Self>,
    ) {
        // Transition detection (claudeplex's most-loved signal): a session that
        // was working (Active/Monitor) and is now Waiting needs the user NOW.
        // Diffed off the unified inventory (old → new); see
        // [`fleet_transitions_to_waiting`] for the identity-keying rationale.
        let became_waiting = fleet_transitions_to_waiting(&self.inventory, &inventory);

        self.snapshot = snapshot;
        self.inventory = inventory;
        self.oauth_cache = oauth_cache;
        self.transcript_cache = transcript_cache;
        self.overrides = overrides;
        self.local_label = local_label;
        // Drop a selection whose account no longer exists, so the highlight never
        // points at a vanished card.
        if let Some(sel) = &self.selected_account {
            if !self.snapshot.accounts.iter().any(|a| &a.account.key == sel) {
                self.selected_account = None;
            }
        }
        ctx.emit(CockpitEvent::Updated);
        if !became_waiting.is_empty() {
            ctx.emit(CockpitEvent::SessionsBecameWaiting(became_waiting));
        }
    }

    /// The unified cross-host Agent-Inventory tree (local + every connected
    /// daemon). Equals the local-only tree when no daemon is connected. Read by
    /// the Conductor UI.
    pub fn inventory(&self) -> &FleetTree {
        &self.inventory
    }

    /// The fleet-wide *needs-me* count — the total number of sessions in
    /// [`SessionState::Waiting`] across every host. Read by the attention
    /// ambient-bit / badge.
    pub fn needs_me(&self) -> usize {
        self.inventory.needs_me
    }

    /// Label of the local host in [`Self::inventory`] (the machine hostname, or
    /// `"local"`). A host node whose `host` equals this is *this* machine, so the
    /// Conductor may adopt its sessions in place; any other host is remote.
    pub fn local_label(&self) -> &str {
        &self.local_label
    }

    /// Periodic reconcile: re-scan on a fixed interval for the model's lifetime.
    fn start_reconcile_timer(&self, ctx: &mut ModelContext<Self>) {
        let spawner = ctx.spawner();
        ctx.background_executor()
            .spawn(async move {
                loop {
                    async_io::Timer::after(RECONCILE_INTERVAL).await;
                    let outcome = spawner.spawn(|me, ctx| me.spawn_refresh(ctx)).await;
                    if outcome.is_err() {
                        break; // model dropped
                    }
                }
            })
            .detach();
    }
}

/// Whether the model's public state is already the disabled/blank state (no
/// accounts, no inventory). Pure so `clear_for_disabled`'s idempotency — the
/// thing that keeps a disabled cockpit's repeated refresh ticks from spamming
/// `CockpitEvent::Updated` — is unit-testable without the actor/`ModelContext`
/// harness.
fn is_blank(snapshot: &CockpitSnapshot, inventory: &FleetTree) -> bool {
    snapshot.accounts.is_empty() && *inventory == FleetTree::default()
}

/// Remove one disconnected remote root synchronously, before the background
/// disk/inventory refresh completes. The manager emits `HostDisconnected` only
/// after the last session for this stable host id is gone, so retaining another
/// session cannot race this removal. Local never matches a daemon id.
fn remove_disconnected_host(inventory: &mut FleetTree, host_id: &str) -> bool {
    let before = inventory.hosts.len();
    inventory
        .hosts
        .retain(|host| host.is_local || host.host_id.as_deref() != Some(host_id));
    if inventory.hosts.len() == before {
        return false;
    }
    inventory.needs_me = inventory
        .hosts
        .iter()
        .filter(|host| host.is_available())
        .map(|host| host.needs_me)
        .sum();
    true
}

/// Detect working→Waiting transitions across the WHOLE fleet — local and every
/// remote host — by diffing the previous inventory against the next one, and
/// return the display string (`"{host} — {place}"`) for each session that just
/// flipped to Waiting from Active/Monitor. Sessions first seen already-waiting
/// don't fire (no old state).
///
/// Sessions are keyed by their complete [`session_key`], never the display
/// `host` label or raw conversation id. The same id can exist on several hosts
/// and can be copied between provider accounts; either collision could otherwise
/// overwrite an old state and hide or misattribute a waiting transition.
fn fleet_transitions_to_waiting(old: &FleetTree, new: &FleetTree) -> Vec<String> {
    use std::collections::HashMap;
    use zaplex_cockpit::SessionState;
    let old_states: HashMap<String, SessionState> = old
        .hosts
        .iter()
        .filter(|h| h.is_available())
        .flat_map(|h| {
            let is_local = h.is_local;
            let host_id = h.host_id.clone();
            h.projects
                .iter()
                .flat_map(|p| &p.sessions)
                .map(move |s| (session_key(is_local, host_id.as_deref(), s), s.state))
        })
        .collect();
    let mut became_waiting = Vec::new();
    for host in new.hosts.iter().filter(|host| host.is_available()) {
        for session in host.projects.iter().flat_map(|p| &p.sessions) {
            if session.state != SessionState::Waiting {
                continue;
            }
            match old_states.get(&session_key(
                host.is_local,
                host.host_id.as_deref(),
                session,
            )) {
                Some(SessionState::Active) | Some(SessionState::Monitor) => {
                    let place = if session.name.is_empty() {
                        std::path::Path::new(&session.cwd)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| session.cwd.clone())
                    } else {
                        session.name.clone()
                    };
                    became_waiting.push(format!("{} — {place}", host.host));
                }
                _ => {}
            }
        }
    }
    became_waiting
}

impl Entity for CockpitModel {
    type Event = CockpitEvent;
}

impl SingletonEntity for CockpitModel {}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
