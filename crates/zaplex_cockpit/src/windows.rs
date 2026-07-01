//! Time-window aggregation: ccusage-style rolling blocks (5h / 7d) + calendar
//! "today", producing [`WindowTotals`] and reset times, plus heat.
//!
//! All functions are pure and take an explicit `now`, so window boundaries are
//! deterministic and unit-testable without touching the clock.

use chrono::{DateTime, Duration, Timelike, Utc};

use crate::pricing::PricingTable;
use crate::types::{Account, AccountUsage, UsageEntry, WindowTotals};

/// Rolling-block window for the "5h block" view.
pub fn window_5h() -> Duration {
    Duration::hours(5)
}
/// Rolling-block window for the "week" view.
pub fn window_week() -> Duration {
    Duration::days(7)
}

/// Flat default budgets (token *work*) used for heat when no per-tier estimate or
/// user override applies (mirrors `claudeplex` instances.ts). Overridable upstream.
pub const DEFAULT_BUDGET_5H: u64 = 20_000_000;
pub const DEFAULT_BUDGET_WEEK: u64 = 300_000_000;

fn floor_to_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
    ts.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(ts)
}

/// A rolling activity block: a start (first activity floored to the hour) and its totals.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Block {
    start: DateTime<Utc>,
    totals: WindowTotals,
}

/// Group time-ordered entries into ccusage-style rolling blocks: a block begins at
/// the first activity (floored to the hour); an entry starts a *new* block when it
/// falls outside `window` from the block start, or when the gap from the previous
/// entry is ≥ `window`. `entries` MUST be sorted ascending by `ts`.
fn rolling_blocks(entries: &[UsageEntry], window: Duration, pricing: &PricingTable) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut last_ts: Option<DateTime<Utc>> = None;
    for e in entries {
        let start_new = match (blocks.last(), last_ts) {
            (Some(b), Some(last)) => e.ts >= b.start + window || e.ts - last >= window,
            _ => true,
        };
        if start_new {
            blocks.push(Block {
                start: floor_to_hour(e.ts),
                totals: WindowTotals::default(),
            });
        }
        blocks
            .last_mut()
            .expect("just pushed or existed")
            .totals
            .add(e, pricing);
        last_ts = Some(e.ts);
    }
    blocks
}

/// Totals + reset time for the block *currently active* at `now`: the most recent
/// block whose `[start, start + window)` contains `now`. If the last activity's
/// window has elapsed (user idle), returns empty totals and no reset.
fn current_window(
    entries: &[UsageEntry],
    now: DateTime<Utc>,
    window: Duration,
    pricing: &PricingTable,
) -> (WindowTotals, Option<DateTime<Utc>>) {
    match rolling_blocks(entries, window, pricing).last() {
        Some(b) if now >= b.start && now < b.start + window => (b.totals, Some(b.start + window)),
        _ => (WindowTotals::default(), None),
    }
}

/// Sum of entries whose timestamp is on the same UTC calendar day as `now`.
fn today_totals(
    entries: &[UsageEntry],
    now: DateTime<Utc>,
    pricing: &PricingTable,
) -> WindowTotals {
    let today = now.date_naive();
    let mut totals = WindowTotals::default();
    for e in entries.iter().filter(|e| e.ts.date_naive() == today) {
        totals.add(e, pricing);
    }
    totals
}

/// Build the full per-account usage view (5h block / today / week + resets + heat).
/// `entries` may be in any order; it is sorted internally.
pub fn build_account_usage(
    account: Account,
    mut entries: Vec<UsageEntry>,
    now: DateTime<Utc>,
    budget_5h: u64,
    pricing: &PricingTable,
) -> AccountUsage {
    entries.sort_by_key(|e| e.ts);
    let (block5h, reset5h) = current_window(&entries, now, window_5h(), pricing);
    let (week, reset_week) = current_window(&entries, now, window_week(), pricing);
    let today = today_totals(&entries, now, pricing);
    let heat = if budget_5h > 0 {
        block5h.work as f64 / budget_5h as f64
    } else {
        0.0
    };
    AccountUsage {
        account,
        block5h,
        today,
        week,
        reset5h,
        reset_week,
        heat,
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
