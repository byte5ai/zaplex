//! Tests for instances.json-style account overrides.

use super::*;
use crate::types::{Account, AccountStatus, Provider, UsageProvenance, WindowTotals};

fn account(key: &str) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider: Provider::Claude,
            key: key.into(),
            config_dir: format!("/tmp/{key}").into(),
            label: key.into(),
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: key.ends_with("default"),
        },
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        status: AccountStatus::Live,
        provenance: UsageProvenance::Real,
    }
}

fn keys(v: &[AccountUsage]) -> Vec<String> {
    v.iter().map(|a| a.account.key.clone()).collect()
}

#[test]
fn empty_overrides_pass_accounts_through_unchanged() {
    let ov = AccountOverrides::parse("{}");
    assert!(ov.is_empty());
    let accts = vec![account("claude:a"), account("claude:default")];
    let out = ov.apply(accts);
    assert_eq!(keys(&out), vec!["claude:a", "claude:default"]);
}

#[test]
fn hidden_accounts_are_dropped() {
    let ov = AccountOverrides::parse(r#"{"claude:a": {"hidden": true}}"#);
    let out = ov.apply(vec![account("claude:a"), account("claude:b")]);
    assert_eq!(keys(&out), vec!["claude:b"]);
}

#[test]
fn label_override_is_applied() {
    let ov = AccountOverrides::parse(r#"{"claude:work": {"label": "Work — Max 20x"}}"#);
    let out = ov.apply(vec![account("claude:work")]);
    assert_eq!(out[0].account.label, "Work — Max 20x");
    assert_eq!(out[0].account.key, "claude:work", "key is never rewritten");
}

#[test]
fn order_sorts_ascending_with_unordered_last_stable() {
    // b→order 1, default→order 0; a and c are un-ordered and keep their
    // input relative order after the ordered ones.
    let ov = AccountOverrides::parse(
        r#"{"claude:b": {"order": 1}, "claude:default": {"order": 0}}"#,
    );
    let out = ov.apply(vec![
        account("claude:a"),
        account("claude:b"),
        account("claude:c"),
        account("claude:default"),
    ]);
    assert_eq!(
        keys(&out),
        vec!["claude:default", "claude:b", "claude:a", "claude:c"]
    );
}

#[test]
fn color_lookup_by_key() {
    let ov = AccountOverrides::parse(r##"{"claude:a": {"color": "#22C55E"}}"##);
    assert_eq!(ov.color_for("claude:a"), Some("#22C55E"));
    assert_eq!(ov.color_for("claude:b"), None);
}

#[test]
fn combined_override_hide_relabel_recolor_reorder() {
    let ov = AccountOverrides::parse(
        r##"{
            "claude:old": {"hidden": true},
            "claude:work": {"label": "Work", "color": "#FB923C", "order": 0}
        }"##,
    );
    let out = ov.apply(vec![
        account("claude:default"),
        account("claude:old"),
        account("claude:work"),
    ]);
    // old dropped; work pulled to front by order 0; default un-ordered after.
    assert_eq!(keys(&out), vec!["claude:work", "claude:default"]);
    assert_eq!(out[0].account.label, "Work");
    assert_eq!(ov.color_for("claude:work"), Some("#FB923C"));
}

#[test]
fn broken_json_yields_no_overrides_never_hides_accounts() {
    for bad in ["", "not json", "[1,2,3]", "{\"k\": \"not an object\"}"] {
        let ov = AccountOverrides::parse(bad);
        assert!(ov.is_empty(), "broken input {bad:?} must yield empty overrides");
        // And apply is a pass-through — accounts survive a broken file.
        let out = ov.apply(vec![account("claude:a")]);
        assert_eq!(keys(&out), vec!["claude:a"]);
    }
}
