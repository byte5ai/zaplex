use super::*;
use crate::types::{Account, AccountStatus, Provider, SessionState, UsageEntry};
use chrono::{DateTime, FixedOffset, Utc};

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&Utc)
}

fn entry(t: &str, input: u64, output: u64) -> UsageEntry {
    entry_of("s1", t, input, output)
}

/// A turn attributed to a specific session.
fn entry_of(session_id: &str, t: &str, input: u64, output: u64) -> UsageEntry {
    UsageEntry {
        ts: ts(t),
        provider: Provider::Claude,
        model: "claude-opus-4-8".into(),
        input,
        output,
        cache_create: 0,
        cache_read: 0,
        reasoning: 0,
        session_id: session_id.into(),
    }
}

fn acct() -> Account {
    Account {
        provider: Provider::Claude,
        key: "claude:default".into(),
        config_dir: "/tmp/x".into(),
        label: "test".into(),
        email: None,
        org: None,
        role: None,
        plan_tier: None,
        is_default: true,
    }
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
}

#[test]
fn windows_bucket_correctly_with_fixed_now() {
    let entries = vec![
        entry("2026-06-28T08:00:00Z", 500, 50), // 2 days ago
        entry("2026-06-30T10:00:00Z", 1000, 100),
        entry("2026-06-30T11:00:00Z", 2000, 200),
    ];
    let now = ts("2026-06-30T12:00:00Z");
    let pricing = PricingTable::default();
    let u = build_account_usage(acct(), entries, now, 6600, DEFAULT_BUDGET_WEEK, &pricing);

    // 5h block = the two same-day turns; older turn is a separate expired block.
    assert_eq!(u.block5h.messages, 2);
    assert_eq!(u.block5h.work, 3300);
    assert_eq!(u.block5h.input, 3000);
    assert_eq!(u.block5h.output, 300);
    assert_eq!(u.reset5h, Some(ts("2026-06-30T15:00:00Z")));
    // opus: (3000*15 + 300*75)/1e6
    approx(u.block5h.cost_usd, 0.0675);

    // today (UTC) = same as the 5h block here.
    assert_eq!(u.today.messages, 2);
    assert_eq!(u.today.work, 3300);

    // week = all three turns in one rolling 7d block.
    assert_eq!(u.week.messages, 3);
    assert_eq!(u.week.work, 3850);
    assert_eq!(u.reset_week, Some(ts("2026-07-05T08:00:00Z")));

    // heat = 3300 / 6600.
    approx(u.heat, 0.5);
}

#[test]
fn idle_past_the_window_yields_empty_block_and_no_reset() {
    let entries = vec![
        entry("2026-06-30T10:00:00Z", 1000, 100),
        entry("2026-06-30T11:00:00Z", 2000, 200),
    ];
    // 20:00 is > 5h after the block start (10:00 → resets 15:00).
    let now = ts("2026-06-30T20:00:00Z");
    let pricing = PricingTable::default();
    let u = build_account_usage(acct(), entries, now, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK, &pricing);

    assert_eq!(u.block5h.messages, 0);
    assert_eq!(u.block5h.work, 0);
    assert!(u.reset5h.is_none());
    approx(u.heat, 0.0);

    // Still within the 7d week block, so week stays populated.
    assert_eq!(u.week.messages, 2);
    assert!(u.reset_week.is_some());
}

#[test]
fn a_gap_of_at_least_the_window_starts_a_new_block() {
    let entries = vec![
        entry("2026-06-30T10:00:00Z", 1000, 100),
        entry("2026-06-30T16:00:00Z", 2000, 200), // gap 6h ≥ 5h → new block
    ];
    let now = ts("2026-06-30T16:30:00Z");
    let pricing = PricingTable::default();
    let u = build_account_usage(acct(), entries, now, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK, &pricing);

    // Current 5h block only contains the second turn.
    assert_eq!(u.block5h.messages, 1);
    assert_eq!(u.block5h.work, 2200);
    assert_eq!(u.reset5h, Some(ts("2026-06-30T21:00:00Z")));
}

/// "Today" must follow the clock on the user's wall, not UTC. Both assertions
/// below fail under a `now.date_naive()` (UTC-day) implementation.
#[test]
fn today_follows_the_local_calendar_day_east_of_utc() {
    let berlin = FixedOffset::east_opt(2 * 3600).unwrap(); // CEST
    let pricing = PricingTable::default();
    let entries = vec![
        entry("2026-06-30T09:00:00Z", 1000, 100), // local 30 Jun 11:00 → yesterday
        entry("2026-06-30T22:35:00Z", 2000, 200), // local 01 Jul 00:35 → today
    ];
    // 01 Jul 00:30 local, while UTC still says 30 Jun.
    let now = ts("2026-06-30T22:30:00Z");

    let today = today_totals_in(&entries, now, &berlin, &pricing);

    // Only the after-midnight turn counts: the UTC day would have taken both.
    assert_eq!(today.messages, 1);
    assert_eq!(today.work, 2200);
}

#[test]
fn today_follows_the_local_calendar_day_west_of_utc() {
    let new_york = FixedOffset::west_opt(5 * 3600).unwrap(); // EST
    let pricing = PricingTable::default();
    let entries = vec![
        entry("2026-06-30T20:00:00Z", 1000, 100), // local 30 Jun 15:00 → today
        entry("2026-07-01T01:00:00Z", 2000, 200), // local 30 Jun 20:00 → today
    ];
    // 30 Jun 22:00 local, while UTC has already rolled over to 01 Jul.
    let now = ts("2026-07-01T03:00:00Z");

    let today = today_totals_in(&entries, now, &new_york, &pricing);

    // Both turns are on the same local day; the UTC day would have split them.
    assert_eq!(today.messages, 2);
    assert_eq!(today.work, 3300);
}

#[test]
fn empty_entries_are_all_zero() {
    let now = ts("2026-06-30T12:00:00Z");
    let pricing = PricingTable::default();
    let u = build_account_usage(acct(), vec![], now, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK, &pricing);
    assert_eq!(u.block5h, WindowTotals::default());
    assert_eq!(u.today, WindowTotals::default());
    assert_eq!(u.week, WindowTotals::default());
    assert!(u.reset5h.is_none());
    assert!(u.reset_week.is_none());
    approx(u.heat, 0.0);
}

// ── Account status vs. dormant sessions ─────────────────────────────────────

fn snapshot(state: SessionState) -> crate::types::SessionSnapshot {
    crate::types::SessionSnapshot {
        session_id: "s".into(),
        cwd: "/tmp/p".into(),
        name: String::new(),
        state,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/tmp/p".into(),
        project_name: "p".into(),
        branch: None,
        worktree: None,
        config_dir: None,
        last_activity: ts("2026-06-30T12:00:00Z"),
        pid: 0,
    }
}

fn status_of(sessions: Vec<crate::types::SessionSnapshot>) -> AccountStatus {
    let pricing = PricingTable::default();
    let now = ts("2026-06-30T12:00:00Z");
    let usage = build_account_usage(acct(), vec![], now, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK, &pricing);
    with_sessions(usage, sessions).status
}

#[test]
fn account_status_reflects_running_work_only() {
    assert_eq!(status_of(vec![]), AccountStatus::Offline);
    assert_eq!(status_of(vec![snapshot(SessionState::Active)]), AccountStatus::Working);
    assert_eq!(status_of(vec![snapshot(SessionState::Waiting)]), AccountStatus::Live);
    assert_eq!(status_of(vec![snapshot(SessionState::Monitor)]), AccountStatus::Live);
}

/// A finished conversation must not make the account look live. Dormant
/// sessions belong in `idle_sessions`, but a remote host folds any state it
/// cannot parse to `Idle`, so the rule is written to hold even then.
#[test]
fn dormant_sessions_never_make_an_account_live() {
    assert_eq!(
        status_of(vec![snapshot(SessionState::Idle)]),
        AccountStatus::Offline,
        "nothing is running, so the account is not live"
    );
    // Mixed: the running one still decides.
    assert_eq!(
        status_of(vec![snapshot(SessionState::Idle), snapshot(SessionState::Waiting)]),
        AccountStatus::Live
    );
    assert_eq!(
        status_of(vec![snapshot(SessionState::Idle), snapshot(SessionState::Active)]),
        AccountStatus::Working
    );
}

#[test]
fn idle_sessions_are_carried_separately_from_live_ones() {
    let pricing = PricingTable::default();
    let now = ts("2026-06-30T12:00:00Z");
    let usage = build_account_usage(acct(), vec![], now, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK, &pricing);
    let usage = with_sessions(usage, vec![snapshot(SessionState::Waiting)]);
    let usage = with_idle_sessions(usage, vec![snapshot(SessionState::Idle)]);

    assert_eq!(usage.sessions.len(), 1);
    assert_eq!(usage.idle_sessions.len(), 1);
    assert_eq!(
        usage.status,
        AccountStatus::Live,
        "attaching dormant sessions must not disturb the status"
    );
}

// ── Per-session spend ───────────────────────────────────────────────────────

#[test]
fn todays_spend_is_attributed_to_the_session_that_incurred_it() {
    let pricing = PricingTable::default();
    let now = ts("2026-06-30T12:00:00Z");
    let entries = vec![
        entry_of("alpha", "2026-06-30T10:00:00Z", 1000, 100),
        entry_of("beta", "2026-06-30T10:30:00Z", 2000, 200),
        entry_of("alpha", "2026-06-30T11:00:00Z", 500, 50),
        // Yesterday: must not land in today's split at all.
        entry_of("alpha", "2026-06-29T11:00:00Z", 9000, 900),
    ];
    let by = today_by_session_in(&entries, now, &Utc, &pricing);

    assert_eq!(by.len(), 2, "one bucket per session that spent today");
    assert_eq!(by["alpha"].messages, 2);
    assert_eq!(by["alpha"].work, 1650);
    assert_eq!(by["beta"].messages, 1);
    assert_eq!(by["beta"].work, 2200);
}

/// The account figure and the rows beneath it are one fold, not two estimates:
/// the parts must sum to the whole exactly, or the table quietly contradicts the
/// header above it.
#[test]
fn per_session_spend_sums_exactly_to_the_account_total() {
    let pricing = PricingTable::default();
    let now = ts("2026-06-30T12:00:00Z");
    let entries = vec![
        entry_of("alpha", "2026-06-30T10:00:00Z", 1000, 100),
        entry_of("beta", "2026-06-30T10:30:00Z", 2000, 200),
        entry_of("alpha", "2026-06-30T11:00:00Z", 500, 50),
        entry_of("gamma", "2026-06-30T11:30:00Z", 700, 70),
        entry_of("alpha", "2026-06-29T11:00:00Z", 9000, 900), // yesterday
    ];
    let today = today_totals_in(&entries, now, &Utc, &pricing);
    let by = today_by_session_in(&entries, now, &Utc, &pricing);

    assert_eq!(by.values().map(|t| t.work).sum::<u64>(), today.work);
    assert_eq!(by.values().map(|t| t.total).sum::<u64>(), today.total);
    assert_eq!(by.values().map(|t| t.messages).sum::<u64>(), today.messages);
    approx(by.values().map(|t| t.cost_usd).sum::<f64>(), today.cost_usd);
}

/// A turn whose transcript names no session still belongs to the account. It
/// groups under the empty id rather than being dropped — losing it would make
/// the rows stop summing to the header.
#[test]
fn unattributable_spend_still_counts_towards_the_account() {
    let pricing = PricingTable::default();
    let now = ts("2026-06-30T12:00:00Z");
    let entries = vec![
        entry_of("alpha", "2026-06-30T10:00:00Z", 1000, 100),
        entry_of("", "2026-06-30T10:30:00Z", 2000, 200),
    ];
    let today = today_totals_in(&entries, now, &Utc, &pricing);
    let by = today_by_session_in(&entries, now, &Utc, &pricing);

    assert_eq!(by[""].work, 2200, "kept under the empty id");
    assert_eq!(by.values().map(|t| t.work).sum::<u64>(), today.work);
}

/// The split follows the same local-day rule as the total it must sum to — a
/// second, UTC-based day boundary here would silently break that.
#[test]
fn the_per_session_split_uses_the_same_day_boundary_as_the_total() {
    let berlin = FixedOffset::east_opt(2 * 3600).unwrap();
    let pricing = PricingTable::default();
    let entries = vec![
        entry_of("alpha", "2026-06-30T09:00:00Z", 1000, 100), // local: yesterday
        entry_of("beta", "2026-06-30T22:35:00Z", 2000, 200),  // local: today 00:35
    ];
    let now = ts("2026-06-30T22:30:00Z");

    let today = today_totals_in(&entries, now, &berlin, &pricing);
    let by = today_by_session_in(&entries, now, &berlin, &pricing);

    assert_eq!(by.len(), 1);
    assert_eq!(by["beta"].work, 2200);
    assert_eq!(by.values().map(|t| t.work).sum::<u64>(), today.work);
}
