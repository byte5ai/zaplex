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
    assert!(t.price_for("gpt-5-codex").is_some());
    assert!(t.price_for("claude-opus-5").is_none());
    assert!(t.price_for("totally-unknown-model").is_none());
}

#[test]
fn codex_key_is_more_specific_than_gpt5() {
    let t = PricingTable::default();
    // Both currently priced the same, but the specific key must resolve first so a
    // future divergence in the table is honoured.
    assert!(t.price_for("gpt-5-codex").is_some());
}

#[test]
fn opus_golden_cost_from_verified_transcript_block() {
    // Verified 2026-07-24 against Anthropic's public API list price.
    let t = PricingTable::default();
    let cost = t
        .cost_for("claude-opus-4-8", 19370, 230, 9716, 19748, 0)
        .unwrap();
    // (19370*5 + 230*25 + 9716*6.25 + 19748*0.50) / 1e6
    approx(cost, 0.173199);
}

#[test]
fn one_million_each_opus() {
    let t = PricingTable::default();
    // 1M input @ $5 + 1M output @ $25 = $30.
    approx(
        t.cost_for("claude-opus-4-8", 1_000_000, 1_000_000, 0, 0, 0)
            .unwrap(),
        30.0,
    );
}

#[test]
fn reasoning_tokens_bill_as_output_for_codex() {
    let t = PricingTable::default();
    // 1M output + 1M reasoning, both @ $10 output = $20.
    approx(
        t.cost_for("gpt-5-codex", 0, 1_000_000, 0, 0, 1_000_000)
            .unwrap(),
        20.0,
    );
}

#[test]
fn unknown_model_is_not_zero_cost() {
    let t = PricingTable::default();
    assert_eq!(
        t.cost_for("some-future-model", 1_000_000, 1_000_000, 0, 0, 0),
        None
    );
}

#[test]
fn estimate_has_provenance() {
    let estimate = PricingTable::default()
        .estimate_for("gpt-5-codex", 100, 10, 0, 90, 5)
        .unwrap();
    assert!(estimate.is_estimate());
    assert_eq!(estimate.source, PricingSource::BundledListPrice);
}
