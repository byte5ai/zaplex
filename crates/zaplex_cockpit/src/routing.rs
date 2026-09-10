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
#[path = "routing_tests.rs"]
mod tests;
