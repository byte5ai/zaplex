//! zaplex cockpit — the read-only **data spine** for the "plex" half of the product.
//!
//! Discovers Claude Code + Codex accounts/subscriptions, aggregates their own
//! transcript token usage into rolling windows (5h block / today / week), and
//! derives cost (per-model pricing) and heat (load vs. budget).
//!
//! This crate is a **pure, headless-testable data layer**: no GUI, no network, and —
//! a hard privacy invariant — it reads only **token counts and account metadata**,
//! never token strings or transcript content. The `CockpitModel` / file-watch wiring
//! that surfaces this into the app lives in `app/src/cockpit/`.
//!
//! See `docs/superpowers/specs/2026-06-30-cockpit-increment1-account-usage-design.md`.

pub mod claude;
pub mod codex;
pub mod pricing;
pub mod types;
pub mod windows;

pub use pricing::{ModelPrice, PricingTable};
pub use types::{Account, AccountUsage, CockpitSnapshot, Provider, UsageEntry, WindowTotals};
pub use windows::{
    build_account_usage, window_5h, window_week, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK,
};

use std::path::Path;

use chrono::{DateTime, Utc};

/// Build a full cockpit snapshot from disk: discover Claude + Codex accounts, parse
/// their transcripts within the widest (week) window, and aggregate per-account
/// usage / cost / heat.
///
/// `now` is explicit so windowing is deterministic and testable. `budget_5h` sizes
/// heat (0 = disable heat). This is the crate's single I/O entry point; the app's
/// `CockpitModel` calls it off the main thread on file-watch/reconcile ticks.
pub fn build_snapshot(
    home: &Path,
    codex_home: &Path,
    claude_config_dir_env: Option<&str>,
    now: DateTime<Utc>,
    budget_5h: u64,
    pricing: &PricingTable,
) -> CockpitSnapshot {
    let since = now - window_week();
    let mut accounts = Vec::new();

    for account in claude::discover_accounts(home, claude_config_dir_env) {
        let entries = claude::usage_for_account(&account, since);
        accounts.push(build_account_usage(account, entries, now, budget_5h, pricing));
    }
    for account in codex::discover_accounts(codex_home) {
        let entries = codex::usage_for_account(&account, since);
        accounts.push(build_account_usage(account, entries, now, budget_5h, pricing));
    }

    CockpitSnapshot {
        accounts,
        generated_at: now,
    }
}
