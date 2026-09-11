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
fn picks_lowest_heat_of_the_provider() {
    let accts = vec![
        claude("claude:a", 0.6),
        claude("claude:default", 0.2),
        claude("claude:b", 0.9),
    ];
    let pick = pick_freest(Provider::Claude, &accts).expect("a claude account");
    assert_eq!(
        pick.account.key, "claude:default",
        "the least-loaded is freest"
    );
}

#[test]
fn ranks_by_binding_window_not_5h_alone() {
    // `a` has a calm 5h (0.20) but a nearly-full Opus weekly sublimit (0.95);
    // `b` has a busier 5h (0.60) with room overall. The launch must pick `b` —
    // the binding window, not the 5h, is what actually gates a launch (Codex).
    let mut a = claude("claude:a", 0.20);
    a.heat_opus = Some(0.95);
    let b = claude("claude:b", 0.60);
    let accts = vec![a, b];
    let pick = pick_freest(Provider::Claude, &accts).expect("a claude account");
    assert_eq!(
        pick.account.key, "claude:b",
        "freest ranks on the fullest window (Opus sublimit), not 5h alone"
    );
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
fn none_when_no_account_for_provider() {
    let accts = vec![claude("claude:default", 0.3)];
    assert!(pick_freest(Provider::Codex, &accts).is_none());
}

#[test]
fn under_budget_beats_a_cooler_but_over_budget_account() {
    // b is numerically cooler (0.95) but a is under budget (0.99 < 1.0)…
    let accts = vec![claude("claude:over", 1.05), claude("claude:under", 0.99)];
    let pick = pick_freest(Provider::Claude, &accts).unwrap();
    assert_eq!(
        pick.account.key, "claude:under",
        "under-budget outranks over-budget"
    );
}

#[test]
fn all_over_budget_still_returns_the_least_loaded() {
    let accts = vec![
        claude("claude:a", 1.4),
        claude("claude:b", 1.1),
        claude("claude:c", 1.9),
    ];
    let pick = pick_freest(Provider::Claude, &accts).expect("launch is never blocked");
    assert_eq!(pick.account.key, "claude:b", "least over-budget");
    assert!(is_over_budget(pick));
}

#[test]
fn idle_account_beats_a_working_one_at_equal_heat() {
    let working = usage(
        Provider::Claude,
        "claude:busy",
        0.4,
        AccountStatus::Working,
        UsageProvenance::Real,
    );
    let idle = usage(
        Provider::Claude,
        "claude:idle",
        0.4,
        AccountStatus::Live,
        UsageProvenance::Real,
    );
    let accts = vec![working, idle];
    let pick = pick_freest(Provider::Claude, &accts).unwrap();
    assert_eq!(pick.account.key, "claude:idle", "not-working wins the tie");
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

#[test]
fn empty_input_is_none_and_empty_rank() {
    assert!(pick_freest(Provider::Claude, &[]).is_none());
    assert!(rank_by_freeness(Provider::Claude, &[]).is_empty());
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
