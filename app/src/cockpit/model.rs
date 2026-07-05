//! `CockpitModel` — the singleton that holds the latest [`CockpitSnapshot`] and keeps
//! it fresh, emitting [`CockpitEvent::Updated`] on change.
//!
//! Refresh is driven by two sources (mirrors `file_mcp_watcher` + the daemon GC
//! timer):
//! - [`HomeDirectoryWatcher`] (top-level home changes) → catches account add/remove
//!   (`~/.claude.json`, `~/.claude`, `~/.codex`).
//! - a periodic **reconcile tick** → catches usage growth (transcripts append deep in
//!   `projects/**` / `sessions/**`, which the non-recursive home watcher never sees)
//!   and window/reset rollover.
//!
//! The (blocking) disk scan runs on the background executor; results are applied back
//! on the model's thread via the spawner round-trip.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_compat::CompatExt as _;
use chrono::Utc;
use warpui::{Entity, ModelContext, SingletonEntity};
use watcher::HomeDirectoryWatcher;
use zaplex_cockpit::{
    apply_oauth_usage, build_snapshot, AccountOverrides, CockpitSnapshot, PricingTable, Provider,
};

use crate::cockpit::oauth::{self, CachedOauth};
use crate::cockpit::settings::CockpitSettings;

/// How often to re-scan transcripts even when no top-level home change fired.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(45);

/// Emitted whenever the snapshot changes.
pub enum CockpitEvent {
    /// One or more sessions transitioned from working to WAITING — they need
    /// the user. Carries "account-label — session/cwd" display strings.
    SessionsBecameWaiting(Vec<String>),
    Updated,
}

pub struct CockpitModel {
    snapshot: CockpitSnapshot,
    pricing: PricingTable,
    /// Per-account real-usage cache (C3b), keyed by the account's config dir.
    /// Lives here so the 15-min TTL survives across refresh cycles; the token
    /// itself is never stored — only the parsed, secret-free usage numbers.
    oauth_cache: HashMap<PathBuf, CachedOauth>,
    /// User overrides (instances.json: label/color/order/hide), applied to every
    /// snapshot. Kept here so the card renderer can look up per-account colors
    /// (color isn't an `Account` field). Empty when the file is absent/broken.
    overrides: AccountOverrides,
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
    /// Path to the user's `instances.json` account overrides (read off-thread).
    instances_path: PathBuf,
}

impl CockpitModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Account add/remove (top-level home entries).
        ctx.subscribe_to_model(&HomeDirectoryWatcher::handle(ctx), |me, _event, ctx| {
            me.spawn_refresh(ctx);
        });

        let model = Self {
            snapshot: CockpitSnapshot {
                accounts: Vec::new(),
                generated_at: Utc::now(),
            },
            pricing: PricingTable::default(),
            oauth_cache: HashMap::new(),
            overrides: AccountOverrides::default(),
        };
        model.spawn_refresh(ctx);
        model.start_reconcile_timer(ctx);
        model
    }

    /// The latest snapshot (empty until the first background scan completes).
    pub fn snapshot(&self) -> &CockpitSnapshot {
        &self.snapshot
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
        Some(RefreshInputs {
            codex_home: home.join(".codex"),
            claude_config_dir_env: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            instances_path: home.join(".zap").join("instances.json"),
            home,
            budget_5h,
            budget_week,
            pricing: self.pricing.clone(),
            oauth_enabled: *CockpitSettings::as_ref(ctx).oauth_usage,
            oauth_cache: self.oauth_cache.clone(),
        })
    }

    /// Kick off a background disk scan; applies the result on the model thread.
    fn spawn_refresh(&self, ctx: &mut ModelContext<Self>) {
        let Some(inputs) = self.refresh_inputs(ctx) else {
            return;
        };
        let spawner = ctx.spawner();
        ctx.background_executor()
            .spawn(async move {
                let mut snapshot = build_snapshot(
                    &inputs.home,
                    &inputs.codex_home,
                    inputs.claude_config_dir_env.as_deref(),
                    Utc::now(),
                    inputs.budget_5h,
                    inputs.budget_week,
                    &inputs.pricing,
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
                let _ = spawner
                    .spawn(move |me, ctx| me.apply(snapshot, oauth_cache, overrides, ctx))
                    .await;
            })
            .detach();
    }

    /// The user-overridden display color for an account key, if any (hex string
    /// like `#22C55E`; the renderer parses/validates it).
    pub fn override_color(&self, key: &str) -> Option<&str> {
        self.overrides.color_for(key)
    }

    fn apply(
        &mut self,
        snapshot: CockpitSnapshot,
        oauth_cache: HashMap<PathBuf, CachedOauth>,
        overrides: AccountOverrides,
        ctx: &mut ModelContext<Self>,
    ) {
        // Transition detection (claudeplex's most-loved signal): a session that
        // was working (Active/Monitor) and is now Waiting needs the user NOW.
        // Sessions first seen already-waiting don't fire (no old state).
        use std::collections::HashMap;
        use zaplex_cockpit::SessionState;
        let old_states: HashMap<&str, SessionState> = self
            .snapshot
            .accounts
            .iter()
            .flat_map(|a| a.sessions.iter())
            .map(|s| (s.session_id.as_str(), s.state))
            .collect();
        let mut became_waiting = Vec::new();
        for account in &snapshot.accounts {
            for session in &account.sessions {
                if session.state != SessionState::Waiting {
                    continue;
                }
                match old_states.get(session.session_id.as_str()) {
                    Some(SessionState::Active) | Some(SessionState::Monitor) => {
                        let place = if session.name.is_empty() {
                            std::path::Path::new(&session.cwd)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| session.cwd.clone())
                        } else {
                            session.name.clone()
                        };
                        became_waiting.push(format!("{} — {place}", account.account.label));
                    }
                    _ => {}
                }
            }
        }

        self.snapshot = snapshot;
        self.oauth_cache = oauth_cache;
        self.overrides = overrides;
        ctx.emit(CockpitEvent::Updated);
        if !became_waiting.is_empty() {
            ctx.emit(CockpitEvent::SessionsBecameWaiting(became_waiting));
        }
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

impl Entity for CockpitModel {
    type Event = CockpitEvent;
}

impl SingletonEntity for CockpitModel {}
