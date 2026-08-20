//! Stable, account-precise multi-launch plans for the Spawn Card.
//!
//! The card creates one immutable plan before execution. Results are recorded
//! per stable target, so a partial retry can never repeat a successful launch.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::ManagedLaunchMode;
use crate::terminal::CLIAgent;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LaunchAccountId(pub(crate) String);

impl LaunchAccountId {
    pub(super) fn local(agent: CLIAgent, config_dir: &std::path::Path) -> Self {
        Self(format!(
            "{}:local:{}",
            agent.to_serialized_name(),
            config_dir.to_string_lossy()
        ))
    }

    pub(super) fn remote(agent: CLIAgent, account_id: &str) -> Self {
        Self(format!(
            "{}:remote:{account_id}",
            agent.to_serialized_name()
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchAccountTarget {
    pub(crate) id: LaunchAccountId,
    pub(crate) label: String,
    pub(crate) config_dir: Option<PathBuf>,
    pub(crate) account_email: Option<String>,
    pub(crate) remote_route: Option<remote_server::proto::AgentLaunchRoute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkLaunchTarget {
    pub(crate) account: LaunchAccountTarget,
    pub(crate) agent: CLIAgent,
    pub(crate) node_id: Option<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) managed_mode: ManagedLaunchMode,
    /// Generated once with the immutable plan and retained across partial
    /// retries, making a retried OpenSession idempotent daemon-side.
    pub(crate) managed_launch_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BulkLaunchTargetId(pub(crate) String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BulkLaunchPlanId(pub(crate) u64);

impl BulkLaunchPlanId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BulkLaunchPlan {
    pub(super) id: BulkLaunchPlanId,
    pub(super) targets: BTreeMap<BulkLaunchTargetId, BulkLaunchTarget>,
}

impl BulkLaunchPlan {
    pub(super) fn new(targets: impl IntoIterator<Item = BulkLaunchTarget>) -> Self {
        let id = BulkLaunchPlanId::next();
        let mut unique = BTreeMap::new();
        for target in targets {
            let target_id = BulkLaunchTargetId(target.account.id.0.clone());
            unique.entry(target_id).or_insert(target);
        }
        Self {
            id,
            targets: unique,
        }
    }

    pub(super) fn preview(&self, host_label: &str, directory_label: &str) -> LaunchPreview {
        let provider = self
            .targets
            .values()
            .next()
            .map(|target| target.agent.display_name().to_string())
            .unwrap_or_default();
        LaunchPreview {
            count: self.targets.len(),
            provider,
            host: host_label.to_string(),
            directory: directory_label.to_string(),
            accounts: self
                .targets
                .values()
                .map(|target| target.account.label.clone())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LaunchPreview {
    pub(super) count: usize,
    pub(super) provider: String,
    pub(super) host: String,
    pub(super) directory: String,
    pub(super) accounts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BulkTargetResult {
    Pending,
    InFlight { launch_id: String },
    Succeeded { launch_id: String },
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BulkLaunchLedger {
    pub(super) plan: BulkLaunchPlan,
    results: BTreeMap<BulkLaunchTargetId, BulkTargetResult>,
}

impl BulkLaunchLedger {
    pub(super) fn new(plan: BulkLaunchPlan) -> Self {
        let results = plan
            .targets
            .keys()
            .cloned()
            .map(|target| (target, BulkTargetResult::Pending))
            .collect();
        Self { plan, results }
    }

    pub(super) fn targets_for_attempt(&self) -> Vec<(BulkLaunchTargetId, BulkLaunchTarget)> {
        self.plan
            .targets
            .iter()
            .filter(|(id, _)| {
                !matches!(
                    self.results.get(*id),
                    Some(BulkTargetResult::InFlight { .. } | BulkTargetResult::Succeeded { .. })
                )
            })
            .map(|(id, target)| (id.clone(), target.clone()))
            .collect()
    }

    pub(super) fn apply(
        &mut self,
        plan_id: BulkLaunchPlanId,
        target_id: &BulkLaunchTargetId,
        result: Result<String, String>,
    ) -> bool {
        if self.plan.id != plan_id || !self.results.contains_key(target_id) {
            return false;
        }
        if matches!(
            self.results.get(target_id),
            Some(BulkTargetResult::Succeeded { .. })
        ) {
            return true;
        }
        let result = match result {
            Ok(launch_id) => BulkTargetResult::Succeeded { launch_id },
            Err(message) => BulkTargetResult::Failed { message },
        };
        self.results.insert(target_id.clone(), result);
        true
    }

    pub(super) fn mark_in_flight(
        &mut self,
        plan_id: BulkLaunchPlanId,
        target_id: &BulkLaunchTargetId,
        launch_id: String,
    ) -> bool {
        if self.plan.id != plan_id || !self.results.contains_key(target_id) {
            return false;
        }
        self.results
            .insert(target_id.clone(), BulkTargetResult::InFlight { launch_id });
        true
    }

    pub(super) fn all_succeeded(&self) -> bool {
        !self.results.is_empty()
            && self
                .results
                .values()
                .all(|result| matches!(result, BulkTargetResult::Succeeded { .. }))
    }

    pub(super) fn any_succeeded(&self) -> bool {
        self.results
            .values()
            .any(|result| matches!(result, BulkTargetResult::Succeeded { .. }))
    }

    pub(super) fn failed(&self) -> Vec<(&BulkLaunchTarget, &str)> {
        self.results
            .iter()
            .filter_map(|(id, result)| match result {
                BulkTargetResult::Failed { message } => self
                    .plan
                    .targets
                    .get(id)
                    .map(|target| (target, message.as_str())),
                BulkTargetResult::Pending
                | BulkTargetResult::InFlight { .. }
                | BulkTargetResult::Succeeded { .. } => None,
            })
            .collect()
    }
}

pub(super) fn selected_account_ids(
    all: &[LaunchAccountTarget],
    selected: &BTreeSet<LaunchAccountId>,
    select_all: bool,
) -> Vec<LaunchAccountTarget> {
    all.iter()
        .filter(|account| select_all || selected.contains(&account.id))
        .cloned()
        .collect()
}

#[cfg(test)]
#[path = "bulk_tests.rs"]
mod tests;
