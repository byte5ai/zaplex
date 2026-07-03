use super::*;
use chrono::{DateTime, Utc};

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn heat_level_thresholds() {
    assert_eq!(HeatLevel::from_fraction(0.0), HeatLevel::Ok);
    assert_eq!(HeatLevel::from_fraction(0.34), HeatLevel::Ok);
    assert_eq!(HeatLevel::from_fraction(0.35), HeatLevel::Elevated);
    assert_eq!(HeatLevel::from_fraction(0.59), HeatLevel::Elevated);
    assert_eq!(HeatLevel::from_fraction(0.60), HeatLevel::High);
    assert_eq!(HeatLevel::from_fraction(0.84), HeatLevel::High);
    assert_eq!(HeatLevel::from_fraction(0.85), HeatLevel::Critical);
    assert_eq!(HeatLevel::from_fraction(0.99), HeatLevel::Critical);
    assert_eq!(HeatLevel::from_fraction(1.0), HeatLevel::Over);
    assert_eq!(HeatLevel::from_fraction(2.5), HeatLevel::Over);
}

#[test]
fn heat_hex_matches_reference_palette() {
    assert_eq!(HeatLevel::Ok.hex(), "#22c55e");
    assert_eq!(HeatLevel::Over.hex(), "#ef4444");
}

#[test]
fn heat_fill_clamps_but_label_does_not() {
    assert_eq!(heat_fill(0.5), 0.5);
    assert_eq!(heat_fill(1.3), 1.0);
    assert_eq!(heat_fill(-0.2), 0.0);
    assert_eq!(heat_pct_label(1.3), "130%");
    assert_eq!(heat_pct_label(0.615), "62%");
}

#[test]
fn cost_format() {
    assert_eq!(format_cost(4.2), "$4.20");
    assert_eq!(format_cost(0.0), "$0.00");
    assert_eq!(format_cost(19.005), "$19.00");
}

#[test]
fn token_humanization() {
    assert_eq!(format_tokens(42), "42");
    assert_eq!(format_tokens(999), "999");
    assert_eq!(format_tokens(3_400), "3.4k");
    assert_eq!(format_tokens(300_000), "300k");
    assert_eq!(format_tokens(1_200_000), "1.2M");
    assert_eq!(format_tokens(6_000_000), "6M");
}

#[test]
fn reset_countdown() {
    let now = ts("2026-06-30T12:00:00Z");
    assert_eq!(format_reset(None, now), "");
    assert_eq!(format_reset(Some(ts("2026-06-30T11:59:00Z")), now), "resetting");
    assert_eq!(format_reset(Some(ts("2026-06-30T12:45:00Z")), now), "45m");
    assert_eq!(format_reset(Some(ts("2026-06-30T14:13:00Z")), now), "2h13m");
    assert_eq!(format_reset(Some(ts("2026-07-04T13:00:00Z")), now), "4d1h");
}
