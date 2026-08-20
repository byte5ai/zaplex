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
    // Number and unit are joined by a narrow no-break space (U+202F): typographic
    // convention, and it keeps "130 %" from ever wrapping across a line.
    assert_eq!(heat_pct_label(1.3), "130\u{202f}%");
    assert_eq!(heat_pct_label(0.615), "62\u{202f}%");
}

#[test]
fn provenance_marks_estimates_only() {
    use crate::types::UsageProvenance;
    assert_eq!(
        heat_pct_label_with_provenance(0.62, UsageProvenance::Real),
        "62\u{202f}%"
    );
    assert_eq!(
        heat_pct_label_with_provenance(0.62, UsageProvenance::Estimate),
        "~62\u{202f}%"
    );
    assert_eq!(
        heat_pct_label_with_provenance(1.3, UsageProvenance::Estimate),
        "~130\u{202f}%"
    );
}

#[test]
fn cost_format() {
    assert_eq!(format_cost(4.2), "~$4.20");
    assert_eq!(format_cost(0.0), "~$0.00");
    assert_eq!(format_cost(19.005), "~$19.00");
    assert_eq!(format_cost(-1.0), "unpriced");
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
    assert_eq!(
        format_reset(Some(ts("2026-06-30T11:59:00Z")), now),
        "resetting"
    );
    assert_eq!(format_reset(Some(ts("2026-06-30T12:45:00Z")), now), "45m");
    assert_eq!(format_reset(Some(ts("2026-06-30T14:13:00Z")), now), "2h13m");
    assert_eq!(format_reset(Some(ts("2026-07-04T13:00:00Z")), now), "4d1h");
}

#[test]
fn context_window_is_1m_for_opus_sonnet_else_200k() {
    assert_eq!(context_window("claude-opus-4-8"), 1_000_000);
    assert_eq!(context_window("claude-sonnet-4-6"), 1_000_000);
    assert_eq!(context_window("claude-haiku-4-5"), 200_000);
    assert_eq!(context_window("some-other"), 200_000);
}

#[test]
fn context_fill_is_fraction_of_window() {
    // 50k of a 1M opus window = 5%.
    assert!((context_fill("claude-opus-4-8", 50_000) - 0.05).abs() < 1e-9);
    // 100k of a 200k haiku window = 50%.
    assert!((context_fill("claude-haiku-4-5", 100_000) - 0.5).abs() < 1e-9);
    assert_eq!(context_fill("m", 0), 0.0);
}

#[test]
fn model_family_shortens_or_passes_through() {
    assert_eq!(model_family("claude-opus-4-8"), "opus");
    assert_eq!(model_family("CLAUDE-SONNET-4-6"), "sonnet");
    assert_eq!(model_family("gpt-5-codex"), "gpt-5-codex");
    assert_eq!(model_family(""), "");
}

/// "Last" reads in the same units as "resets in" — a row and a meter should not
/// speak two dialects of the same clock.
#[test]
fn relative_time_is_short_and_matches_the_reset_units() {
    let now = DateTime::parse_from_rfc3339("2026-07-17T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let ago = |secs: i64| format_relative(now - chrono::Duration::seconds(secs), now);

    assert_eq!(ago(5), "now", "under a minute is not worth a number");
    assert_eq!(ago(59), "now");
    assert_eq!(ago(60), "1m");
    assert_eq!(ago(3599), "59m");
    assert_eq!(ago(3600), "1h");
    assert_eq!(ago(86_399), "23h");
    assert_eq!(ago(86_400), "1d");
    assert_eq!(ago(10 * 86_400), "10d");
}

/// A timestamp from the future means two clocks disagree — the host that wrote
/// it and this one. That is not news to put in a row, so it reads as `now`
/// rather than as a negative age.
#[test]
fn a_future_timestamp_reads_as_now_rather_than_negative() {
    let now = DateTime::parse_from_rfc3339("2026-07-17T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        format_relative(now + chrono::Duration::hours(3), now),
        "now"
    );
}
