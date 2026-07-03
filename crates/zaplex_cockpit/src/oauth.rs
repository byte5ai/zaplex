//! Real subscription utilization from Anthropic's `/api/oauth/usage` endpoint —
//! the *parsing and merging* half (C3b).
//!
//! This module is pure: no network, no token handling. The app wiring
//! (`app/src/cockpit/oauth.rs`) owns the single authenticated GET per account
//! and hands the raw JSON body here; on any schema surprise the caller falls
//! back to the estimate — degradation is the contract, not an error path.
//!
//! Policy line (C3b design §4): the cockpit may ask "how full is my quota?",
//! it may never spend the quota. This module only interprets the answer.
//!
//! Schema verified 2026-07-03 against the claudeplex-desktop reference
//! (`electron/usage.ts`): windows carry `used_percentage` (0..100) or
//! `utilization` (0..1 or 0..100); `resets_at` is epoch seconds, epoch
//! milliseconds, or ISO 8601; `five_hour` is the validity gate; optional
//! `seven_day_opus` / `seven_day_sonnet` plan sublimits.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};

use crate::types::{CockpitSnapshot, Provider, UsageProvenance};

/// One rate-limit window as reported by the endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OauthWindow {
    /// Utilization as a fraction (1.0 == 100% of the window's limit) — same
    /// scale as [`crate::types::AccountUsage::heat`].
    pub fraction: f64,
    /// When the window resets; `None` if the endpoint didn't say.
    pub resets_at: Option<DateTime<Utc>>,
}

/// Parsed `/api/oauth/usage` response for one account.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OauthUsage {
    /// The rolling 5-hour window.
    pub five_hour: OauthWindow,
    /// The rolling 7-day window.
    pub seven_day: OauthWindow,
    /// 7-day Opus sublimit, if the plan has one (fraction, 1.0 == 100%).
    pub opus_fraction: Option<f64>,
    /// 7-day Sonnet sublimit, if the plan has one (fraction, 1.0 == 100%).
    pub sonnet_fraction: Option<f64>,
}

/// Window object → utilization fraction. Handles both observed field variants:
/// `used_percentage` (always 0..100) and `utilization` (0..1 or 0..100 scale).
fn fraction_of(window: &serde_json::Value) -> f64 {
    if let Some(p) = window.get("used_percentage").and_then(|v| v.as_f64()) {
        return p / 100.0;
    }
    if let Some(u) = window.get("utilization").and_then(|v| v.as_f64()) {
        return if u <= 1.0 { u } else { u / 100.0 };
    }
    0.0
}

/// `resets_at` → UTC time. Accepts epoch seconds, epoch milliseconds (split at
/// 2e10, i.e. year ~2603 in seconds), or an ISO-8601 / RFC-3339 string.
fn resets_at_of(window: &serde_json::Value) -> Option<DateTime<Utc>> {
    match window.get("resets_at") {
        Some(serde_json::Value::Number(n)) => {
            let v = n.as_f64()?;
            let ms = if v < 2e10 { v * 1000.0 } else { v };
            Utc.timestamp_millis_opt(ms as i64).single()
        }
        Some(serde_json::Value::String(s)) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

fn window_of(v: &serde_json::Value, key: &str) -> OauthWindow {
    match v.get(key) {
        Some(w) => OauthWindow {
            fraction: fraction_of(w),
            resets_at: resets_at_of(w),
        },
        None => OauthWindow::default(),
    }
}

/// Parse the endpoint's JSON body into [`OauthUsage`]. Returns `None` when the
/// body is not JSON or lacks the `five_hour` window (the same validity gate the
/// reference implementation uses) — the caller then keeps the estimate.
pub fn parse_oauth_usage(body: &str) -> Option<OauthUsage> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("five_hour")?;
    Some(OauthUsage {
        five_hour: window_of(&v, "five_hour"),
        seven_day: window_of(&v, "seven_day"),
        opus_fraction: v.get("seven_day_opus").map(fraction_of),
        sonnet_fraction: v.get("seven_day_sonnet").map(fraction_of),
    })
}

/// Merge real per-account usage into a freshly built snapshot.
///
/// Matching Claude accounts (by `config_dir`) get real heat + reset times and
/// [`UsageProvenance::Real`]; everything else keeps the estimate. Token/cost
/// totals are never touched — they measure spend, not rate-limit position.
/// A real window without a reset time keeps the transcript-derived reset
/// (better an estimated countdown than none).
pub fn apply_oauth_usage(
    snapshot: &mut CockpitSnapshot,
    by_config_dir: &HashMap<PathBuf, OauthUsage>,
) {
    for acct in &mut snapshot.accounts {
        if acct.account.provider != Provider::Claude {
            continue;
        }
        let Some(real) = by_config_dir.get(&acct.account.config_dir) else {
            continue;
        };
        acct.heat = real.five_hour.fraction;
        acct.heat_week = real.seven_day.fraction;
        if real.five_hour.resets_at.is_some() {
            acct.reset5h = real.five_hour.resets_at;
        }
        if real.seven_day.resets_at.is_some() {
            acct.reset_week = real.seven_day.resets_at;
        }
        acct.provenance = UsageProvenance::Real;
    }
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
