//! Time-window aggregation: ccusage-style rolling blocks (5h / 7d) + calendar
//! "today", producing [`WindowTotals`] and reset times, plus heat.
//!
//! Functions take an explicit `now`, so window boundaries are deterministic and
//! unit-testable without touching the clock. The one ambient input is the
//! machine's time zone, which fixes the calendar-day boundary; it is injected
//! into [`today_totals_in`] so that path stays testable against a fixed zone.

use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};

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

/// Sum of entries falling on the same calendar day as `now` **in `tz`**.
///
/// The day boundary is the one the user sees on their own clock, not UTC:
/// east of UTC, work done just after local midnight still carries the previous
/// UTC date, and would otherwise be booked into yesterday's "today".
/// Each instant is converted on its own, so the offset applied is the one in
/// force at that instant. With a DST-aware zone (production passes [`Local`])
/// the local day therefore stays correct across a transition; a `FixedOffset`,
/// as the tests pass, is not DST-aware to begin with. UTC→local is always
/// unambiguous, so this cannot panic even inside a repeated fall-back hour.
fn today_totals_in<Tz: TimeZone>(
    entries: &[UsageEntry],
    now: DateTime<Utc>,
    tz: &Tz,
    pricing: &PricingTable,
) -> WindowTotals {
    let today = now.with_timezone(tz).date_naive();
    let mut totals = WindowTotals::default();
    for e in entries
        .iter()
        .filter(|e| e.ts.with_timezone(tz).date_naive() == today)
    {
        totals.add(e, pricing);
    }
    totals
}

/// Sum of entries on the machine's current local calendar day.
fn today_totals(
    entries: &[UsageEntry],
    now: DateTime<Utc>,
    pricing: &PricingTable,
) -> WindowTotals {
    today_totals_in(entries, now, &Local, pricing)
}

/// Build the full per-account usage view (5h block / today / week + resets + heat).
/// `entries` may be in any order; it is sorted internally.
pub fn build_account_usage(
    account: Account,
    mut entries: Vec<UsageEntry>,
    now: DateTime<Utc>,
    budget_5h: u64,
    budget_week: u64,
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
    let heat_week = if budget_week > 0 {
        week.work as f64 / budget_week as f64
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
        heat_week,
        // Estimates have no sublimit signal — only real OAuth reports these.
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions: Vec::new(),
        status: crate::types::AccountStatus::Offline,
        provenance: crate::types::UsageProvenance::Estimate,
    }
}

/// Plan-tier-based budget estimate (port of claudeplex-desktop's `planBudget`):
/// used when the user set no explicit budget, so a heavily-used Enterprise/Team
/// account isn't shown falsely maxed against the flat Max-5x default. Sensible
/// defaults, not exact plan limits.
pub fn plan_budgets(plan_tier: Option<&str>) -> (u64, u64) {
    let plan = plan_tier.unwrap_or("").to_ascii_lowercase();
    if plan.contains("enterprise") {
        (140_000_000, 2_000_000_000)
    } else if plan.contains("team") || plan.contains("20") {
        (70_000_000, 1_000_000_000)
    } else if plan.contains("pro") {
        (7_000_000, 100_000_000)
    } else {
        // Max 5x / unknown → the flat defaults.
        (DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK)
    }
}

/// Attaches live sessions to a built usage view and derives the account status.
///
/// "Live" asks for a session that is actually *there*, so a dormant one never
/// counts. `sessions` is not supposed to carry [`SessionState::Idle`] at all
/// (that is what [`AccountUsage::idle_sessions`] is for), but the rule is
/// written to be total rather than to trust the invariant: a remote host folds
/// any state string it does not recognise to `Idle`, and an account must not
/// claim to be live on the strength of one of those.
pub fn with_sessions(
    mut usage: AccountUsage,
    sessions: Vec<crate::types::SessionSnapshot>,
) -> AccountUsage {
    use crate::types::{AccountStatus, SessionState};
    usage.status = if sessions.iter().any(|s| s.state == SessionState::Active) {
        AccountStatus::Working
    } else if sessions.iter().any(|s| s.state != SessionState::Idle) {
        AccountStatus::Live
    } else {
        AccountStatus::Offline
    };
    usage.sessions = sessions;
    usage
}

/// Attaches the dormant, resumable sessions. Kept apart from [`with_sessions`]
/// because they answer a different question — "what could I pick back up?"
/// rather than "what is running?" — and must not colour the account's status.
pub fn with_idle_sessions(
    mut usage: AccountUsage,
    idle: Vec<crate::types::SessionSnapshot>,
) -> AccountUsage {
    usage.idle_sessions = idle;
    usage
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
