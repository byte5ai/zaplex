//! Launch routing — the C4 "plexing" brain (pure, headless).
//!
//! Given the cockpit's per-account usage, choose which subscription a new agent
//! should launch onto: the *freest* one (least likely to hit a rate limit), or
//! the whole roster ranked freest-first for a manual pick. No I/O, no GUI — this
//! is the decision function the launcher (C4-3) and the "launch on freest"
//! setting build on. Mirrors claudeplex's `instances.ts` load routing.

use std::cmp::Ordering;

use crate::types::{AccountStatus, AccountUsage, CockpitSnapshot, Provider, UsageProvenance};

/// 5-hour heat at/above which an account counts as **over budget** (at or over
/// its 5h budget). Used only to rank under-budget accounts ahead of over-budget
/// ones — launching is never *blocked*: if every account is hot, the least-loaded
/// is still returned so the caller can warn ("even the freest is at 96 %") and
/// proceed. `heat` may exceed 1.0 (see [`AccountUsage::heat`]).
pub const OVER_BUDGET_HEAT: f64 = 1.0;

/// Pick the freest account of `provider` to launch onto. Ranking, best first:
/// 1. **under budget** (binding-window heat < [`OVER_BUDGET_HEAT`]) before over-budget,
/// 2. **not actively working** before working,
/// 3. **lower binding-window heat** (fullest of 5h / week / Opus / Sonnet),
/// 4. **real** usage before local **estimate** (more trustworthy).
///
/// Returns `None` only when no account for `provider` exists. The returned
/// [`AccountUsage`] carries `heat` / `provenance` so the caller can decide
/// whether to warn before launching onto an over-budget account.
pub fn pick_freest(provider: Provider, accounts: &[AccountUsage]) -> Option<&AccountUsage> {
    accounts
        .iter()
        .filter(|a| a.account.provider == provider)
        .min_by(|a, b| cmp_freeness(a, b))
}

/// [`pick_freest`], but only from an authoritative snapshot. Returns `None` while the
/// snapshot is still loading or if it degraded (a config/dir failed to load) — an
/// account whose usage scan silently failed has `heat = 0` and would otherwise win as
/// "freest", so the launcher would auto-route onto a possibly-broken account. On
/// `None` the caller falls back to its default/selected account instead of guessing.
pub fn pick_freest_checked(
    provider: Provider,
    snapshot: &CockpitSnapshot,
) -> Option<&AccountUsage> {
    if !snapshot.health.is_loaded() {
        return None;
    }
    pick_freest(provider, &snapshot.accounts)
}

/// Every account of `provider`, ranked freest-first — the same order
/// [`pick_freest`] chooses from. For the manual "show the ranked list" launcher
/// mode (the `launch_routing = show_ranked` setting).
pub fn rank_by_freeness(provider: Provider, accounts: &[AccountUsage]) -> Vec<&AccountUsage> {
    let mut ranked: Vec<&AccountUsage> = accounts
        .iter()
        .filter(|a| a.account.provider == provider)
        .collect();
    ranked.sort_by(|a, b| cmp_freeness(a, b));
    ranked
}

/// Whether an account is at or over its **binding** budget (the caller's warn
/// signal) — the fullest of its 5h / week / Opus / Sonnet windows, not just 5h,
/// so an account with a calm 5h but a maxed weekly/Opus sublimit still counts.
pub fn is_over_budget(usage: &AccountUsage) -> bool {
    binding_heat(usage) >= OVER_BUDGET_HEAT
}

/// The account's binding-window utilization — the fullest of 5h / week / Opus /
/// Sonnet — i.e. the fraction that actually gates a launch. Ranking on 5h alone
/// would pick an account that is calm short-term but already at its weekly/Opus
/// cap (Codex gate).
fn binding_heat(u: &AccountUsage) -> f64 {
    crate::binding_window(u).0
}

/// Total ordering used by both [`pick_freest`] and [`rank_by_freeness`]. Lower =
/// freer = better. `f64` heat can't be `Ord`, so this is a hand-rolled comparator
/// (heat is never NaN here — it is a computed ratio, defaulting to 0.0).
fn cmp_freeness(a: &AccountUsage, b: &AccountUsage) -> Ordering {
    let working = |u: &AccountUsage| matches!(u.status, AccountStatus::Working);
    // 1. under budget before over budget (false < true)
    is_over_budget(a)
        .cmp(&is_over_budget(b))
        // 2. not-working before working
        .then_with(|| working(a).cmp(&working(b)))
        // 3. lower binding-window heat first (fullest of 5h / week / Opus / Sonnet)
        .then_with(|| {
            binding_heat(a)
                .partial_cmp(&binding_heat(b))
                .unwrap_or(Ordering::Equal)
        })
        // 4. real before estimate
        .then_with(|| prov_rank(a.provenance).cmp(&prov_rank(b.provenance)))
}

fn prov_rank(p: UsageProvenance) -> u8 {
    match p {
        UsageProvenance::Real => 0,
        UsageProvenance::Estimate => 1,
    }
}

#[cfg(test)]
mod tests {
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
}
