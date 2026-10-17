use super::*;
use crate::format::{cost_figure, CostFigure};
use crate::types::{Provider, UsageEntry, WindowTotals};

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
}

fn turn(model: &str, input: u64, output: u64) -> UsageEntry {
    UsageEntry {
        ts: chrono::Utc::now(),
        provider: Provider::Claude,
        model: model.to_string(),
        input,
        output,
        cache_create: 0,
        cache_read: 0,
        reasoning: 0,
        session_id: String::new(),
    }
}

/// A bundled entry prices its own model id and that id's dated snapshots,
/// never a longer id: `claude-opus-5-6` is a different model, not a snapshot
/// of `claude-opus-5`, and must stay unpriced rather than borrow its rate.
#[test]
fn model_ids_match_exactly_or_as_dated_snapshots() {
    let t = PricingTable::default();
    assert_eq!(
        t.price_for("CLAUDE-OPUS-4-8"),
        t.price_for("claude-opus-4-8")
    );
    assert_eq!(
        t.price_for("claude-haiku-4-5-20251001"),
        t.price_for("claude-haiku-4-5")
    );
    assert_eq!(
        t.price_for("gpt-6-sol-2026-04-20"),
        t.price_for("gpt-6-sol")
    );
    assert!(t.price_for("gpt-6-sol").is_some());
    // Opus 5 and Opus 5.5 share a prefix but not a price: each id finds its own.
    assert!(t.price_for("claude-opus-5").is_some());
    assert_ne!(t.price_for("claude-opus-5"), t.price_for("claude-opus-5-5"));
    for unknown in [
        "claude-opus-5-6",
        "claude-fable-5-2",
        "claude-opus-5-5-preview",
        "gpt-5.1-codex-mini",
        "gpt-6-sol-pro",
        "gpt-6-sol-20260",
        "gpt-5.5",
        "totally-unknown-model",
    ] {
        assert_eq!(t.price_for(unknown), None, "{unknown} must stay unpriced");
    }
}

/// Past the long-context threshold the vendor bills the **whole** request at
/// the higher rate, and the prompt it measures includes cached tokens.
#[test]
fn a_prompt_over_the_long_context_threshold_reprices_the_whole_request() {
    let rates = |base: f64| ModelPrice {
        input: base,
        output: 2.0 * base,
        cache_write: 3.0 * base,
        cache_read: 4.0 * base,
    };
    let t = PricingTable::bundled(vec![(
        "tiered-model",
        rates(1.0),
        Some(LongContextPrice {
            above_prompt_tokens: 100,
            price: rates(10.0),
        }),
    )]);
    // 50 input + 25 cache writes + 25 cache reads = a 100-token prompt, not
    // over the threshold: 50 * 1 + 7 * 2 + 25 * 3 + 25 * 4 = 239 per 1M.
    approx(t.cost_for("tiered-model", 50, 7, 25, 25).unwrap(), 239e-6);
    // One more cached token makes 101: every token, output included, now
    // pays the long rate: 50 * 10 + 7 * 20 + 25 * 30 + 26 * 40 = 2430 per 1M.
    approx(t.cost_for("tiered-model", 50, 7, 25, 26).unwrap(), 2430e-6);
}

#[test]
fn custom_keys_are_matched_case_insensitively() {
    let price = ModelPrice {
        input: 1.0,
        output: 2.0,
        cache_write: 3.0,
        cache_read: 4.0,
    };
    let table = PricingTable::new(vec![("Custom-MODEL".to_string(), price)]);

    assert_eq!(table.price_for("prefix-custom-model-suffix"), Some(price));
    assert_eq!(
        table
            .estimate_for("CUSTOM-MODEL", 1_000_000, 0, 0, 0)
            .unwrap()
            .source,
        PricingSource::CustomStatic
    );
}

#[test]
fn unknown_model_price_is_rendered_as_not_priced() {
    let t = PricingTable::default();
    let cost = t.cost_for("some-future-model", 1_000_000, 1_000_000, 0, 0);
    assert_eq!(cost, None);
    let mut totals = WindowTotals::default();
    totals.add(&turn("some-future-model", 1_000_000, 1_000_000), &t);
    assert_eq!(
        cost_figure([&totals]),
        CostFigure::Unpriced {
            unpriced_models: vec!["some-future-model".to_string()],
        }
    );
}

#[test]
fn known_estimated_cost_has_provenance_marker() {
    let t = PricingTable::default();
    let estimate = t.estimate_for("gpt-5-codex", 100, 10, 0, 90).unwrap();
    assert!(estimate.is_estimate());
    assert_eq!(estimate.source, PricingSource::BundledListPrice);
    assert!(
        estimate.source.as_of().is_some(),
        "bundled prices name the date they were verified"
    );
    let mut totals = WindowTotals::default();
    totals.add(&turn("gpt-5-codex", 100, 10), &t);
    assert!(matches!(
        cost_figure([&totals]),
        CostFigure::Estimate { usd } if usd > 0.0
    ));
}
