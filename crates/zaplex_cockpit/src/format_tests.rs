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

fn window(cost_usd: f64, unpriced_models: &[&str]) -> WindowTotals {
    WindowTotals {
        has_unpriced_usage: !unpriced_models.is_empty(),
        unpriced_models: unpriced_models
            .iter()
            .map(|model| model.to_string())
            .collect(),
        cost_usd,
        ..WindowTotals::default()
    }
}

/// An unpriced share never voids the priced part (#515): the fleet total sums
/// every account's priced spend and names each unpriced model once.
#[test]
fn cost_figure_keeps_the_priced_part_beside_unpriced_models() {
    assert_eq!(
        cost_figure([&WindowTotals::default()]),
        CostFigure::Estimate { usd: 0.0 },
        "a window without spend is a real zero, not an unknown one"
    );
    assert_eq!(
        cost_figure([&window(0.0, &["gpt-5.5"])]),
        CostFigure::Unpriced {
            unpriced_models: vec!["gpt-5.5".to_string()],
        }
    );
    let accounts = [
        window(1.25, &[]),
        window(2.5, &["gpt-5.5", "claude-next"]),
        window(0.0, &["gpt-5.5"]),
    ];
    assert_eq!(
        cost_figure(&accounts),
        CostFigure::Partial {
            usd: 3.75,
            unpriced_models: vec!["claude-next".to_string(), "gpt-5.5".to_string()],
        }
    );
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
fn token_humanization_promotes_rounded_unit_boundaries() {
    assert_eq!(format_tokens(999_949), "999.9k");
    assert_eq!(format_tokens(999_950), "1M");
    assert_eq!(format_tokens(999_999), "1M");
    assert_eq!(format_tokens(1_000_000), "1M");
    assert_eq!(format_tokens(999_950_000), "1B");
    assert_eq!(format_tokens(1_000_000_000), "1B");
    assert_eq!(format_tokens(u64::MAX), "18446744073.7B");
}

#[test]
fn reset_countdown_steps_from_minutes_to_hours_to_days() {
    let now = ts("2026-06-30T12:00:00Z");
    assert_eq!(reset_countdown(None, now), None);
    assert_eq!(
        reset_countdown(Some(ts("2026-06-30T11:59:00Z")), now),
        Some(ResetCountdown::Resetting)
    );
    assert_eq!(
        reset_countdown(Some(ts("2026-06-30T12:45:00Z")), now),
        Some(ResetCountdown::Minutes(45))
    );
    assert_eq!(
        reset_countdown(Some(ts("2026-06-30T14:13:00Z")), now),
        Some(ResetCountdown::HoursMinutes {
            hours: 2,
            minutes: 13
        })
    );
    assert_eq!(
        reset_countdown(Some(ts("2026-07-04T13:00:00Z")), now),
        Some(ResetCountdown::DaysHours { days: 4, hours: 1 })
    );
}

/// "Last" reads in the same units as "resets in" — a row and a meter should not
/// speak two dialects of the same clock.
#[test]
fn relative_time_is_short_and_matches_the_reset_units() {
    let now = DateTime::parse_from_rfc3339("2026-07-17T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let ago = |secs: i64| relative_age(now - chrono::Duration::seconds(secs), now);

    assert_eq!(
        ago(5),
        RelativeAge::Now,
        "under a minute is not worth a number"
    );
    assert_eq!(ago(59), RelativeAge::Now);
    assert_eq!(ago(60), RelativeAge::Minutes(1));
    assert_eq!(ago(3599), RelativeAge::Minutes(59));
    assert_eq!(ago(3600), RelativeAge::Hours(1));
    assert_eq!(ago(86_399), RelativeAge::Hours(23));
    assert_eq!(ago(86_400), RelativeAge::Days(1));
    assert_eq!(ago(10 * 86_400), RelativeAge::Days(10));
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
        relative_age(now + chrono::Duration::hours(3), now),
        RelativeAge::Now
    );
}
