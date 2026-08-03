use super::*;

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
}

#[test]
fn substring_matching_picks_the_right_model() {
    let t = PricingTable::default();
    assert_eq!(
        t.price_for("claude-opus-4-8"),
        Some(ModelPrice {
            input: 5.0,
            output: 25.0,
            cache_write: 6.25,
            cache_read: 0.50,
        })
    );
    assert!(t.price_for("claude-sonnet-4-6").is_some());
    assert!(t.price_for("claude-haiku-4-5-20251001").is_some());
    assert!(t.price_for("gpt-5.6-sol").is_some());
    assert!(t.price_for("gpt-5.6-terra").is_some());
    assert!(t.price_for("gpt-5.6-luna").is_some());
    assert!(t.price_for("gpt-5.1-codex").is_some());
    assert!(t.price_for("gpt-5-codex").is_some());
    assert!(t.price_for("gpt-5.5").is_none());
    assert!(t.price_for("claude-opus-5").is_none());
    assert!(t.price_for("totally-unknown-model").is_none());
}

#[test]
fn current_codex_models_use_their_exact_prices() {
    let t = PricingTable::default();
    assert_eq!(
        t.price_for("gpt-5.6-sol"),
        Some(ModelPrice {
            input: 5.0,
            output: 30.0,
            cache_write: 6.25,
            cache_read: 0.50,
        })
    );
    assert_eq!(
        t.price_for("gpt-5.6-terra"),
        Some(ModelPrice {
            input: 2.50,
            output: 15.0,
            cache_write: 3.125,
            cache_read: 0.25,
        })
    );
    assert_eq!(
        t.price_for("gpt-5.6-luna"),
        Some(ModelPrice {
            input: 1.0,
            output: 6.0,
            cache_write: 1.25,
            cache_read: 0.10,
        })
    );
}

#[test]
fn opus_golden_cost_from_verified_transcript_block() {
    // Verified 2026-07-31 against Anthropic's public API list price.
    let t = PricingTable::default();
    let cost = t
        .cost_for("claude-opus-4-8", 19370, 230, 9716, 19748)
        .unwrap();
    // (19370*5 + 230*25 + 9716*6.25 + 19748*0.50) / 1e6
    approx(cost, 0.173199);
}

#[test]
fn one_million_each_opus() {
    let t = PricingTable::default();
    // 1M input @ $5 + 1M output @ $25 = $30.
    approx(
        t.cost_for("claude-opus-4-8", 1_000_000, 1_000_000, 0, 0)
            .unwrap(),
        30.0,
    );
}

#[test]
fn sol_golden_cost_from_current_list_price() {
    let t = PricingTable::default();
    let cost = t
        .cost_for("gpt-5.6-sol", 1_000_000, 1_000_000, 0, 1_000_000)
        .unwrap();
    // 1M input @ $5 + 1M output @ $30 + 1M cached input @ $0.50.
    approx(cost, 35.50);
}

#[test]
fn reasoning_tokens_are_not_billed_twice_for_codex() {
    let t = PricingTable::default();
    // The 1M reasoning-token breakdown is already inside the 1M output total.
    approx(t.cost_for("gpt-5-codex", 0, 1_000_000, 0, 0).unwrap(), 10.0);
}

#[test]
fn unknown_model_price_is_rendered_as_not_priced() {
    let t = PricingTable::default();
    let cost = t.cost_for("some-future-model", 1_000_000, 1_000_000, 0, 0);
    assert_eq!(cost, None);
    assert_eq!(
        crate::format_cost(cost.unwrap_or(crate::types::UNPRICED_COST_USD)),
        "unpriced"
    );
}

#[test]
fn known_estimated_cost_has_provenance_marker() {
    let estimate = PricingTable::default()
        .estimate_for("gpt-5-codex", 100, 10, 0, 90)
        .unwrap();
    assert!(estimate.is_estimate());
    assert_eq!(estimate.source, PricingSource::BundledListPrice);
    assert!(crate::format_cost(estimate.usd).starts_with('~'));
}
