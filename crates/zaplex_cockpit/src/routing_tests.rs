use super::*;
use crate::types::{Account, WindowTotals};

fn usage(
    provider: Provider,
    key: &str,
    heat: f64,
    status: AccountStatus,
    provenance: UsageProvenance,
) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider,
            key: key.into(),
            config_dir: format!("/tmp/{key}").into(),
            label: key.into(),
            provider_account_id: None,
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: key.ends_with("default"),
        },
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        today_by_session: Default::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat,
        heat_week: heat,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions: Vec::new(),
        status,
        provenance,
    }
}

fn claude(key: &str, heat: f64) -> AccountUsage {
    usage(
        Provider::Claude,
        key,
        heat,
        AccountStatus::Live,
        UsageProvenance::Real,
    )
}

#[test]
fn filters_by_provider() {
    let accts = vec![
        usage(
            Provider::Codex,
            "codex:default",
            0.1,
            AccountStatus::Live,
            UsageProvenance::Estimate,
        ),
        claude("claude:default", 0.8),
    ];
    // Even though the Codex account is cooler, a Claude launch never picks it.
    let pick = pick_freest(Provider::Claude, &accts).unwrap();
    assert_eq!(pick.account.provider, Provider::Claude);
    assert_eq!(pick.account.key, "claude:default");
}

#[test]
fn real_usage_beats_estimate_at_equal_heat_and_status() {
    let est = usage(
        Provider::Claude,
        "claude:est",
        0.5,
        AccountStatus::Live,
        UsageProvenance::Estimate,
    );
    let real = usage(
        Provider::Claude,
        "claude:real",
        0.5,
        AccountStatus::Live,
        UsageProvenance::Real,
    );
    let accts = vec![est, real];
    let pick = pick_freest(Provider::Claude, &accts).unwrap();
    assert_eq!(
        pick.account.key, "claude:real",
        "real provenance breaks the tie"
    );
}

#[test]
fn rank_orders_freest_first_and_filters_provider() {
    let accts = vec![
        claude("claude:hot", 0.9),
        usage(
            Provider::Codex,
            "codex:default",
            0.1,
            AccountStatus::Live,
            UsageProvenance::Estimate,
        ),
        claude("claude:cool", 0.2),
        claude("claude:mid", 0.5),
    ];
    let ranked = rank_by_freeness(Provider::Claude, &accts);
    let keys: Vec<&str> = ranked.iter().map(|u| u.account.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["claude:cool", "claude:mid", "claude:hot"],
        "freest-first, no codex"
    );
}

fn snapshot(accounts: Vec<AccountUsage>, health: crate::types::ScanHealth) -> CockpitSnapshot {
    CockpitSnapshot {
        accounts,
        generated_at: chrono::Utc::now(),
        health,
    }
}

#[test]
fn failed_account_scan_is_excluded_from_freest_routing() {
    use crate::types::ScanHealth;
    let accts = vec![claude("claude:default", 0.3)];
    // Authoritative scan → auto-route works.
    assert!(
        pick_freest_checked(
            Provider::Claude,
            &snapshot(accts.clone(), ScanHealth::Loaded)
        )
        .is_some(),
        "a loaded snapshot routes normally"
    );
    // First scan not done yet — do not silently auto-route on a "not loaded" list.
    assert!(
        pick_freest_checked(
            Provider::Claude,
            &snapshot(accts.clone(), ScanHealth::Pending)
        )
        .is_none(),
        "a pending snapshot must not auto-route"
    );
    // A config/dir failed to load — a scan-failed account looks maximally free and
    // would be picked wrongly. Refuse to guess.
    assert!(
        pick_freest_checked(
            Provider::Claude,
            &snapshot(accts, ScanHealth::Degraded("codex auth unreadable".into()))
        )
        .is_none(),
        "a degraded snapshot must not auto-route"
    );
}
