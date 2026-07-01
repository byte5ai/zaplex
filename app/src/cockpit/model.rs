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

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use warpui::{Entity, ModelContext, SingletonEntity};
use watcher::HomeDirectoryWatcher;
use zaplex_cockpit::{build_snapshot, CockpitSnapshot, PricingTable, DEFAULT_BUDGET_5H};

use crate::cockpit::settings::CockpitSettings;

/// How often to re-scan transcripts even when no top-level home change fired.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(45);

/// Emitted whenever the snapshot changes.
pub enum CockpitEvent {
    Updated,
}

pub struct CockpitModel {
    snapshot: CockpitSnapshot,
    pricing: PricingTable,
}

/// Inputs captured on the model thread, moved into the off-thread build.
struct RefreshInputs {
    home: PathBuf,
    codex_home: PathBuf,
    claude_config_dir_env: Option<String>,
    budget_5h: u64,
    pricing: PricingTable,
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
        let budget_override = *CockpitSettings::as_ref(ctx).budget_5h as u64;
        let budget_5h = if budget_override > 0 {
            budget_override
        } else {
            DEFAULT_BUDGET_5H
        };
        Some(RefreshInputs {
            codex_home: home.join(".codex"),
            claude_config_dir_env: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            home,
            budget_5h,
            pricing: self.pricing.clone(),
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
                let snapshot = build_snapshot(
                    &inputs.home,
                    &inputs.codex_home,
                    inputs.claude_config_dir_env.as_deref(),
                    Utc::now(),
                    inputs.budget_5h,
                    &inputs.pricing,
                );
                let _ = spawner
                    .spawn(move |me, ctx| me.apply(snapshot, ctx))
                    .await;
            })
            .detach();
    }

    fn apply(&mut self, snapshot: CockpitSnapshot, ctx: &mut ModelContext<Self>) {
        self.snapshot = snapshot;
        ctx.emit(CockpitEvent::Updated);
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
