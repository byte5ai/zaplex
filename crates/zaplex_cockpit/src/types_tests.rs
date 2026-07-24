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
    assert!(totals.cost_usd < 0.0);
    assert_eq!(crate::format_cost(totals.cost_usd), "unpriced");

    let encoded = serde_json::to_string(&totals).unwrap();
    let decoded: WindowTotals = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, totals, "unpriced totals survive a JSON round trip");
}
