//! Localized text for the values the `zaplex_cockpit` spine computes.
//!
//! The spine stays language-neutral: it returns structured values (cost
//! figures and the like), and this module is the one place that turns them
//! into Fluent messages and the user's number format. New cockpit wording
//! belongs here, not in the crate.

use zaplex_cockpit::{CostFigure, PricingSource};

/// A cost on one line: `~4,20 $`, `~4,20 $ · teilweise ohne Preis`, or
/// `ohne Preis` when nothing could be priced.
pub(crate) fn cost_text(figure: &CostFigure) -> String {
    let amount = cost_amount_text(figure);
    match cost_hint_text(figure) {
        Some(hint) => format!("{amount} · {hint}"),
        None => amount,
    }
}

/// The amount alone (`~4,20 $`), or `ohne Preis` when nothing could be priced.
pub(crate) fn cost_amount_text(figure: &CostFigure) -> String {
    match figure {
        CostFigure::Estimate { usd } | CostFigure::Partial { usd, .. } => amount_text(*usd),
        CostFigure::Unpriced { .. } => crate::t!("cockpit-cost-unpriced"),
    }
}

/// The visible hint that a sum leaves models out (`teilweise ohne Preis`).
pub(crate) fn cost_hint_text(figure: &CostFigure) -> Option<String> {
    match figure {
        CostFigure::Partial { .. } => Some(crate::t!("cockpit-cost-partly-unpriced")),
        CostFigure::Estimate { .. } | CostFigure::Unpriced { .. } => None,
    }
}

/// Names the models without a list price, for a tooltip or an explanation.
pub(crate) fn unpriced_models_text(figure: &CostFigure) -> Option<String> {
    match figure {
        CostFigure::Partial {
            unpriced_models, ..
        }
        | CostFigure::Unpriced { unpriced_models } => Some(crate::t!(
            "cockpit-cost-unpriced-models",
            models = unpriced_models.join(", ")
        )),
        CostFigure::Estimate { .. } => None,
    }
}

/// Where cockpit costs come from: a list-price estimate and the date the
/// bundled prices were verified.
pub(crate) fn cost_source_text() -> String {
    crate::t!(
        "cockpit-cost-source",
        date = PricingSource::BundledListPrice
            .as_of()
            .unwrap_or_default()
            .to_string()
    )
}

/// The estimate marker, amount and currency in the locale's layout. Fluent
/// does not localize decimal separators, so each catalog places the whole
/// dollars and the two cent digits itself.
fn amount_text(usd: f64) -> String {
    let (whole, cents) = dollars_and_cents(usd);
    crate::t!("cockpit-cost-estimate", whole = whole, cents = cents)
}

/// Rounds to whole cents: `4.2` -> `("4", "20")`. Rounds the exact stored
/// value (`19.005` is stored just below it, so `"19", "00"`), as a scaled
/// multiplication would not. Costs are never negative; a negative input is
/// shown as zero rather than as a fictional credit.
fn dollars_and_cents(usd: f64) -> (String, String) {
    let fixed = format!("{:.2}", usd.max(0.0));
    let (whole, cents) = fixed
        .split_once('.')
        .expect("two decimals always print a decimal point");
    (whole.to_string(), cents.to_string())
}

#[cfg(test)]
#[path = "localize_tests.rs"]
mod tests;
