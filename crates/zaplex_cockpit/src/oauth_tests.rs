//! Fixture-backed tests for the `/api/oauth/usage` parser and the snapshot
//! merge. The fixtures mirror the schema variants the claudeplex-desktop
//! reference implementation handles (C3b design milestone 1).

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{TimeZone, Utc};

use super::{apply_oauth_usage, parse_oauth_usage};
use crate::pricing::PricingTable;
use crate::types::{Account, CockpitSnapshot, Provider, UsageProvenance};
use crate::windows::build_account_usage;

/// The shape claudeplex-desktop observes in the wild: `utilization` fractions
/// (0..1) + ISO-8601 resets + plan sublimits.
const FIXTURE_UTILIZATION_ISO: &str = r#"{
  "five_hour":  { "utilization": 0.34, "resets_at": "2026-07-03T18:00:00Z" },
  "seven_day":  { "utilization": 0.61, "resets_at": "2026-07-08T09:30:00.000Z" },
  "seven_day_opus":   { "utilization": 0.12 },
  "seven_day_sonnet": { "utilization": 0.55 }
}"#;

/// Variant: `used_percentage` (0..100) + epoch-second resets, no sublimits.
const FIXTURE_PERCENTAGE_EPOCH_S: &str = r#"{
  "five_hour": { "used_percentage": 34.0, "resets_at": 1782151200 },
  "seven_day": { "used_percentage": 61.5, "resets_at": 1782551400 }
}"#;

/// Variant: `utilization` on the 0..100 scale + epoch-millisecond resets.
const FIXTURE_UTILIZATION_100_EPOCH_MS: &str = r#"{
  "five_hour": { "utilization": 34.0, "resets_at": 1782151200000 },
  "seven_day": { "utilization": 61.5 }
}"#;

#[test]
fn parses_utilization_fractions_and_iso_resets() {
    let u = parse_oauth_usage(FIXTURE_UTILIZATION_ISO).expect("fixture parses");
    assert!((u.five_hour.fraction - 0.34).abs() < 1e-9);
    assert_eq!(
        u.five_hour.resets_at,
        Some(Utc.with_ymd_and_hms(2026, 7, 3, 18, 0, 0).unwrap())
    );
    assert!((u.seven_day.fraction - 0.61).abs() < 1e-9);
    assert_eq!(
        u.seven_day.resets_at,
        Some(Utc.with_ymd_and_hms(2026, 7, 8, 9, 30, 0).unwrap())
    );
    assert_eq!(u.opus_fraction, Some(0.12));
    assert_eq!(u.sonnet_fraction, Some(0.55));
}

#[test]
fn parses_used_percentage_and_epoch_second_resets() {
    let u = parse_oauth_usage(FIXTURE_PERCENTAGE_EPOCH_S).expect("fixture parses");
    assert!((u.five_hour.fraction - 0.34).abs() < 1e-9);
    assert_eq!(
        u.five_hour.resets_at,
        Some(Utc.timestamp_opt(1_782_151_200, 0).unwrap())
    );
    assert!((u.seven_day.fraction - 0.615).abs() < 1e-9);
    assert_eq!(u.opus_fraction, None);
    assert_eq!(u.sonnet_fraction, None);
}

#[test]
fn parses_percent_scale_utilization_and_epoch_ms_resets() {
    let u = parse_oauth_usage(FIXTURE_UTILIZATION_100_EPOCH_MS).expect("fixture parses");
    assert!((u.five_hour.fraction - 0.34).abs() < 1e-9);
    assert_eq!(
        u.five_hour.resets_at,
        Some(Utc.timestamp_millis_opt(1_782_151_200_000).unwrap())
    );
    // Missing resets_at → None (caller keeps the estimated reset).
    assert_eq!(u.seven_day.resets_at, None);
}

#[test]
fn rejects_bodies_without_the_five_hour_gate() {
    assert_eq!(parse_oauth_usage(r#"{ "seven_day": {} }"#), None);
    assert_eq!(parse_oauth_usage(r#"{ "error": "unauthorized" }"#), None);
    assert_eq!(parse_oauth_usage("not json at all"), None);
    assert_eq!(parse_oauth_usage(""), None);
}

#[test]
fn missing_window_fields_degrade_to_zero_not_error() {
    let u = parse_oauth_usage(r#"{ "five_hour": {} }"#).expect("gate present");
    assert_eq!(u.five_hour.fraction, 0.0);
    assert_eq!(u.five_hour.resets_at, None);
    assert_eq!(u.seven_day.fraction, 0.0);
}

fn account(provider: Provider, key: &str, dir: &str) -> Account {
    Account {
        provider,
        key: key.to_string(),
        config_dir: PathBuf::from(dir),
        label: key.to_string(),
        email: None,
        org: None,
        role: None,
        plan_tier: None,
        is_default: false,
    }
}

fn snapshot_with(accounts: Vec<Account>) -> CockpitSnapshot {
    let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
    let pricing = PricingTable::default();
    CockpitSnapshot {
        accounts: accounts
            .into_iter()
            .map(|a| build_account_usage(a, Vec::new(), now, 1_000, 10_000, &pricing))
            .collect(),
        generated_at: now,
        health: crate::types::ScanHealth::Loaded,
    }
}

#[test]
fn apply_marks_matching_claude_accounts_real_and_leaves_the_rest_estimated() {
    let mut snapshot = snapshot_with(vec![
        account(Provider::Claude, "claude:default", "/home/u/.claude"),
        account(Provider::Claude, "claude:work", "/home/u/claude-work"),
        account(Provider::Codex, "codex:default", "/home/u/.codex"),
    ]);
    let real = parse_oauth_usage(FIXTURE_UTILIZATION_ISO).unwrap();
    let by_dir: HashMap<PathBuf, _> = [(PathBuf::from("/home/u/.claude"), real)].into();

    apply_oauth_usage(&mut snapshot, &by_dir);

    let matched = &snapshot.accounts[0];
    assert_eq!(matched.provenance, UsageProvenance::Real);
    assert!((matched.heat - 0.34).abs() < 1e-9);
    assert!((matched.heat_week - 0.61).abs() < 1e-9);
    assert_eq!(
        matched.reset5h,
        Some(Utc.with_ymd_and_hms(2026, 7, 3, 18, 0, 0).unwrap())
    );
    // The 7-day per-model sublimits are carried onto the account (Codex #6),
    // not dropped — they are often the binding constraint on Max plans.
    assert_eq!(matched.heat_opus, Some(0.12));
    assert_eq!(matched.heat_sonnet, Some(0.55));

    // Unmatched Claude account and the Codex account keep the estimate.
    assert_eq!(snapshot.accounts[1].provenance, UsageProvenance::Estimate);
    assert_eq!(snapshot.accounts[2].provenance, UsageProvenance::Estimate);
}

#[test]
fn apply_without_reset_keeps_the_estimated_reset() {
    let mut snapshot = snapshot_with(vec![account(
        Provider::Claude,
        "claude:default",
        "/home/u/.claude",
    )]);
    // Give the estimate a reset to preserve.
    let estimated_reset = Some(Utc.with_ymd_and_hms(2026, 7, 3, 15, 0, 0).unwrap());
    snapshot.accounts[0].reset5h = estimated_reset;

    let real = parse_oauth_usage(r#"{ "five_hour": { "utilization": 0.9 } }"#).unwrap();
    let by_dir: HashMap<PathBuf, _> = [(PathBuf::from("/home/u/.claude"), real)].into();

    apply_oauth_usage(&mut snapshot, &by_dir);

    assert_eq!(snapshot.accounts[0].provenance, UsageProvenance::Real);
    assert!((snapshot.accounts[0].heat - 0.9).abs() < 1e-9);
    assert_eq!(snapshot.accounts[0].reset5h, estimated_reset);
}

#[test]
fn apply_never_touches_token_or_cost_totals() {
    let mut snapshot = snapshot_with(vec![account(
        Provider::Claude,
        "claude:default",
        "/home/u/.claude",
    )]);
    let before = snapshot.accounts[0].clone();
    let real = parse_oauth_usage(FIXTURE_UTILIZATION_ISO).unwrap();
    let by_dir: HashMap<PathBuf, _> = [(PathBuf::from("/home/u/.claude"), real)].into();

    apply_oauth_usage(&mut snapshot, &by_dir);

    let after = &snapshot.accounts[0];
    assert_eq!(after.block5h, before.block5h);
    assert_eq!(after.today, before.today);
    assert_eq!(after.week, before.week);
    assert_eq!(after.sessions, before.sessions);
}

#[test]
fn binding_window_surfaces_a_full_opus_sublimit_over_a_calm_5h() {
    // The exact case Codex #6 was filed for: a calm 5h (71%) while the Opus
    // weekly sublimit sits at 91%. The headline must be the Opus limit — the
    // window the user actually hits first — not the 5h that reads "fine".
    let mut snapshot = snapshot_with(vec![account(
        Provider::Claude,
        "claude:default",
        "/home/u/.claude",
    )]);
    let real = parse_oauth_usage(
        r#"{ "five_hour": { "utilization": 0.71 },
             "seven_day": { "utilization": 0.40 },
             "seven_day_opus": { "utilization": 0.91 } }"#,
    )
    .unwrap();
    let by_dir: HashMap<PathBuf, _> = [(PathBuf::from("/home/u/.claude"), real)].into();
    apply_oauth_usage(&mut snapshot, &by_dir);

    let (frac, label) = crate::binding_window(&snapshot.accounts[0]);
    assert!((frac - 0.91).abs() < 1e-9, "binding window is the Opus sublimit");
    assert_eq!(label, "opus");
}

#[test]
fn binding_window_prefers_week_over_5h_when_week_is_fuller() {
    // With no sublimits (estimate-style), the binding window degrades to the
    // fuller of 5h vs. week.
    let mut snapshot = snapshot_with(vec![account(
        Provider::Claude,
        "claude:default",
        "/home/u/.claude",
    )]);
    snapshot.accounts[0].heat = 0.30;
    snapshot.accounts[0].heat_week = 0.80;
    let (frac, label) = crate::binding_window(&snapshot.accounts[0]);
    assert!((frac - 0.80).abs() < 1e-9);
    assert_eq!(label, "wk");
}
