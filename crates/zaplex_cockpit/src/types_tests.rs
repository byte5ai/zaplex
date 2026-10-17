use super::*;
use crate::pricing::PricingTable;

fn entry(input: u64, output: u64, cache_read: u64, reasoning: u64) -> UsageEntry {
    UsageEntry {
        ts: chrono::Utc::now(),
        provider: Provider::Codex,
        model: "gpt-5-codex".into(),
        input,
        output,
        cache_create: 0,
        cache_read,
        reasoning,
        session_id: String::new(),
    }
}

#[test]
fn totals_saturate_at_u64_maximum() {
    let mut totals = WindowTotals {
        input: u64::MAX,
        output: u64::MAX,
        cache_create: u64::MAX,
        cache_read: u64::MAX,
        reasoning: u64::MAX,
        work: u64::MAX,
        total: u64::MAX,
        has_unpriced_usage: false,
        unpriced_models: BTreeSet::new(),
        cost_usd: 0.0,
        messages: u64::MAX,
    };
    totals.add(&entry(1, 1, 1, 1), &PricingTable::default());

    assert_eq!(totals.input, u64::MAX);
    assert_eq!(totals.output, u64::MAX);
    assert_eq!(totals.cache_read, u64::MAX);
    assert_eq!(totals.reasoning, u64::MAX);
    assert_eq!(totals.work, u64::MAX);
    assert_eq!(totals.total, u64::MAX);
    assert_eq!(totals.messages, u64::MAX);
}

#[test]
fn unpriced_model_marks_the_window_unpriced() {
    let mut totals = WindowTotals::default();
    let mut unknown = entry(10, 10, 0, 0);
    unknown.model = "future-unpriced-model".into();

    totals.add(&unknown, &PricingTable::default());

    assert!(totals.has_unpriced_usage);
    assert_eq!(
        totals.unpriced_models,
        BTreeSet::from(["future-unpriced-model".to_string()])
    );
    assert_eq!(
        totals.cost_usd, 0.0,
        "no fictional amount for unknown spend"
    );

    let encoded = serde_json::to_string(&totals).unwrap();
    let decoded: WindowTotals = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, totals, "unpriced totals survive a JSON round trip");
}

/// One unpriced turn used to turn the whole window into "unpriced" (#515).
/// The priced turns keep their sum; the unpriced model is named beside it.
#[test]
fn partially_unpriced_window_keeps_priced_sum() {
    let pricing = PricingTable::new(vec![(
        "gpt-5-codex".to_string(),
        crate::pricing::ModelPrice {
            input: 1.0,
            output: 2.0,
            cache_write: 0.0,
            cache_read: 0.5,
        },
    )]);
    let mut totals = WindowTotals::default();
    // (10 * 1 + 10 * 2 + 90 * 0.5) / 1e6 = 75 per 1M for each priced turn.
    totals.add(&entry(10, 10, 90, 0), &pricing);
    let mut unknown = entry(1_000, 1_000, 0, 0);
    unknown.model = "gpt-5.5".into();
    totals.add(&unknown, &pricing);
    // A priced turn after the unpriced one still counts.
    totals.add(&entry(10, 10, 90, 0), &pricing);

    assert!(totals.has_unpriced_usage);
    assert_eq!(
        totals.unpriced_models,
        BTreeSet::from(["gpt-5.5".to_string()])
    );
    assert!((totals.cost_usd - 150e-6).abs() < 1e-12);
    assert_eq!(totals.messages, 3);
}

/// A turn that billed no tokens (e.g. a synthetic transcript entry) costs
/// nothing at any rate, so its unknown model is no gap in the estimate.
#[test]
fn a_turn_without_tokens_is_never_unpriced() {
    let mut totals = WindowTotals::default();
    let mut empty = entry(0, 0, 0, 0);
    empty.model = "<synthetic>".into();

    totals.add(&empty, &PricingTable::default());

    assert!(!totals.has_unpriced_usage);
    assert!(totals.unpriced_models.is_empty());
    assert_eq!(totals.cost_usd, 0.0);
    assert_eq!(totals.messages, 1);
}

#[test]
fn account_without_provider_identity_remains_readable() {
    let account: Account = serde_json::from_value(serde_json::json!({
        "provider": "Codex",
        "key": "codex:default",
        "config_dir": "/accounts/codex",
        "label": "developer@example.com",
        "email": "developer@example.com",
        "org": null,
        "role": null,
        "plan_tier": null,
        "is_default": true
    }))
    .unwrap();

    assert_eq!(account.key, "codex:default");
    assert_eq!(account.provider_account_id, None);
}
