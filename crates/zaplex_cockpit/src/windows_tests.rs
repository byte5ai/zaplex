use super::*;
use crate::types::{Account, Provider, UsageEntry};
use chrono::{DateTime, Utc};

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&Utc)
}

fn entry(t: &str, input: u64, output: u64) -> UsageEntry {
    UsageEntry {
        ts: ts(t),
        provider: Provider::Claude,
        model: "claude-opus-4-8".into(),
        input,
        output,
        cache_create: 0,
        cache_read: 0,
        reasoning: 0,
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
    let u = build_account_usage(acct(), entries, now, 6600, &pricing);

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
    let u = build_account_usage(acct(), entries, now, DEFAULT_BUDGET_5H, &pricing);

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
    let u = build_account_usage(acct(), entries, now, DEFAULT_BUDGET_5H, &pricing);

    // Current 5h block only contains the second turn.
    assert_eq!(u.block5h.messages, 1);
    assert_eq!(u.block5h.work, 2200);
    assert_eq!(u.reset5h, Some(ts("2026-06-30T21:00:00Z")));
}

#[test]
fn empty_entries_are_all_zero() {
    let now = ts("2026-06-30T12:00:00Z");
    let pricing = PricingTable::default();
    let u = build_account_usage(acct(), vec![], now, DEFAULT_BUDGET_5H, &pricing);
    assert_eq!(u.block5h, WindowTotals::default());
    assert_eq!(u.today, WindowTotals::default());
    assert_eq!(u.week, WindowTotals::default());
    assert!(u.reset5h.is_none());
    assert!(u.reset_week.is_none());
    approx(u.heat, 0.0);
}
