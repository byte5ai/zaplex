//! Pure display formatting shared by the cockpit sidebar + pane: heat band/colour,
//! cost, humanized tokens, reset countdown. Warpui-free and fully unit-tested, so the
//! visual layers stay thin and the numbers-to-text logic is verified headlessly.

use chrono::{DateTime, Utc};

/// Heat band, matching the claudeplex-desktop `LoadBar` thresholds. Input is the heat
/// *fraction* (work / budget), where 1.0 == 100% of budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeatLevel {
    /// < 35%
    Ok,
    /// 35–60%
    Elevated,
    /// 60–85%
    High,
    /// 85–100%
    Critical,
    /// >= 100% (over budget)
    Over,
}

impl HeatLevel {
    pub fn from_fraction(fraction: f64) -> Self {
        let pct = fraction * 100.0;
        if pct >= 100.0 {
            HeatLevel::Over
        } else if pct >= 85.0 {
            HeatLevel::Critical
        } else if pct >= 60.0 {
            HeatLevel::High
        } else if pct >= 35.0 {
            HeatLevel::Elevated
        } else {
            HeatLevel::Ok
        }
    }

    /// Reference hex from claudeplex `LoadBar` (green→red). The app maps this to a
    /// `ColorU`; kept here so the palette is single-sourced + testable.
    pub fn hex(self) -> &'static str {
        match self {
            HeatLevel::Ok => "#22c55e",
            HeatLevel::Elevated => "#eab308",
            HeatLevel::High => "#fb923c",
            HeatLevel::Critical => "#f97316",
            HeatLevel::Over => "#ef4444",
        }
    }
}

/// Bar-fill fraction, clamped to 0..=1 (the % *label* may exceed 100%, the bar can't).
pub fn heat_fill(fraction: f64) -> f64 {
    fraction.clamp(0.0, 1.0)
}

/// Rounded percent label (not clamped): 0.62 -> "62%", 1.3 -> "130%".
pub fn heat_pct_label(fraction: f64) -> String {
    format!("{}%", (fraction * 100.0).round() as i64)
}

/// Percent label with provenance (C3b): real numbers get no extra chrome,
/// estimates carry a subtle `~` prefix — 0.62 -> "62%" (real) / "~62%" (estimate).
pub fn heat_pct_label_with_provenance(
    fraction: f64,
    provenance: crate::types::UsageProvenance,
) -> String {
    match provenance {
        crate::types::UsageProvenance::Real => heat_pct_label(fraction),
        crate::types::UsageProvenance::Estimate => format!("~{}", heat_pct_label(fraction)),
    }
}

/// The model's usable context window in tokens: 1M for the Opus/Sonnet 1M-beta,
/// 272k for the Codex/OpenAI GPT-5 family, 200k otherwise. Turns a raw
/// context-token count into a fill fraction.
pub fn context_window(model: &str) -> u64 {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") || m.contains("sonnet") {
        1_000_000
    } else if m.contains("gpt") || m.contains("codex") || m.contains("o3") || m.contains("o4") {
        // Codex agent models (GPT-5 family / o-series): 272k-token context
        // window. The rollout transcript also carries the exact
        // `model_context_window`, but the Conductor renders fill from the model
        // id alone (like Claude's family defaults), so we single-source it here.
        272_000
    } else {
        200_000
    }
}

/// Context-window fill fraction (0.0..) for a turn's token count + model. Feed
/// to [`HeatLevel::from_fraction`] for the fill color, like claudeplex's
/// context meter.
pub fn context_fill(model: &str, ctx_tokens: u64) -> f64 {
    let max = context_window(model);
    if max == 0 {
        0.0
    } else {
        ctx_tokens as f64 / max as f64
    }
}

/// Short model family for display (`claude-opus-4-8` -> `opus`), else the raw id
/// (empty stays empty). Case-insensitive.
pub fn model_family(model: &str) -> &str {
    let lower = model.to_ascii_lowercase();
    for fam in ["opus", "sonnet", "haiku", "fable"] {
        if lower.contains(fam) {
            return fam;
        }
    }
    model
}

/// USD cost with 2 decimals: 4.2 -> "$4.20".
pub fn format_cost(usd: f64) -> String {
    format!("${usd:.2}")
}

/// Humanized token count: 42 -> "42", 3400 -> "3.4k", 300000 -> "300k",
/// 1_200_000 -> "1.2M", 6_000_000 -> "6M".
pub fn format_tokens(n: u64) -> String {
    let (val, unit) = if n >= 1_000_000 {
        (n as f64 / 1_000_000.0, "M")
    } else if n >= 1_000 {
        (n as f64 / 1_000.0, "k")
    } else {
        return n.to_string();
    };
    let s = format!("{val:.1}");
    let s = s.strip_suffix(".0").unwrap_or(&s);
    format!("{s}{unit}")
}

/// Relative reset countdown (claudeplex `resetIn`): "45m", "2h13m", "4d1h";
/// "resetting" once past; "" when there is no active window.
pub fn format_reset(reset: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(reset) = reset else {
        return String::new();
    };
    let ms = (reset - now).num_milliseconds();
    if ms <= 0 {
        return "resetting".to_string();
    }
    let total_min = ((ms as f64) / 60_000.0).round() as i64;
    if total_min < 60 {
        return format!("{total_min}m");
    }
    let h = total_min / 60;
    if h < 24 {
        return format!("{}h{}m", h, total_min % 60);
    }
    let d = h / 24;
    format!("{}d{}h", d, h % 24)
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
