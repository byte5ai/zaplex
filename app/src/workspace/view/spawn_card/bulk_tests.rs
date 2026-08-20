use super::*;

fn account(id: &str) -> LaunchAccountTarget {
    LaunchAccountTarget {
        id: LaunchAccountId(id.to_string()),
        label: id.to_uppercase(),
        config_dir: Some(PathBuf::from(format!("/accounts/{id}"))),
        account_email: Some(format!("{id}@example.com")),
        remote_route: None,
    }
}

fn target(id: &str) -> BulkLaunchTarget {
    BulkLaunchTarget {
        account: account(id),
        agent: CLIAgent::Claude,
        node_id: None,
        cwd: Some(PathBuf::from("/work/zaplex")),
        model: Some("opus".to_string()),
        effort: None,
        prompt: None,
        managed_mode: ManagedLaunchMode::Ordinary,
        managed_launch_id: None,
    }
}

#[test]
fn plan_deduplicates_accounts_and_preview_is_explicit() {
    let plan = BulkLaunchPlan::new([target("a"), target("b"), target("a")]);
    assert_eq!(plan.targets.len(), 2);
    let preview = plan.preview("Local", "/work/zaplex");
    assert_eq!(preview.count, 2);
    assert_eq!(preview.provider, "Claude Code");
    assert_eq!(preview.host, "Local");
    assert_eq!(preview.directory, "/work/zaplex");
    assert_eq!(preview.accounts, vec!["A", "B"]);
}

#[test]
fn retry_never_repeats_a_successful_account() {
    let plan = BulkLaunchPlan::new([target("a"), target("b")]);
    let plan_id = plan.id;
    let mut ledger = BulkLaunchLedger::new(plan);
    let first = ledger.targets_for_attempt();
    assert_eq!(first.len(), 2);
    ledger.apply(plan_id, &first[0].0, Ok("launch-a".to_string()));
    ledger.apply(plan_id, &first[1].0, Err("offline".to_string()));

    let retry = ledger.targets_for_attempt();
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].1.account.id, first[1].1.account.id);
    assert!(ledger.any_succeeded());
    assert!(!ledger.all_succeeded());
}

#[test]
fn duplicate_or_stale_completion_is_idempotent() {
    let plan = BulkLaunchPlan::new([target("a")]);
    let plan_id = plan.id;
    let mut ledger = BulkLaunchLedger::new(plan);
    let target_id = ledger.targets_for_attempt()[0].0.clone();
    assert!(ledger.apply(plan_id, &target_id, Ok("first".to_string())));
    assert!(ledger.apply(plan_id, &target_id, Ok("second".to_string())));
    assert!(ledger.all_succeeded());
    assert!(!ledger.apply(
        BulkLaunchPlanId(plan_id.0 + 1),
        &target_id,
        Err("stale".to_string())
    ));
}

#[test]
fn managed_attempt_is_not_successful_or_retryable_before_daemon_ack() {
    let plan = BulkLaunchPlan::new([target("managed")]);
    let plan_id = plan.id;
    let mut ledger = BulkLaunchLedger::new(plan);
    let target_id = ledger.targets_for_attempt()[0].0.clone();

    assert!(ledger.mark_in_flight(plan_id, &target_id, "managed-launch-1".to_string()));
    assert!(ledger.targets_for_attempt().is_empty());
    assert!(!ledger.any_succeeded());
    assert!(!ledger.all_succeeded());

    assert!(ledger.apply(plan_id, &target_id, Ok("managed:pty-1:9".to_string())));
    assert!(ledger.all_succeeded());
}

#[test]
fn managed_daemon_rejection_becomes_retryable_without_a_false_success() {
    let plan = BulkLaunchPlan::new([target("managed-failure")]);
    let plan_id = plan.id;
    let mut ledger = BulkLaunchLedger::new(plan);
    let target_id = ledger.targets_for_attempt()[0].0.clone();

    assert!(ledger.mark_in_flight(plan_id, &target_id, "managed-launch-2".to_string()));
    assert!(ledger.apply(plan_id, &target_id, Err("headroom-blocked".to_string())));
    assert!(!ledger.any_succeeded());
    assert_eq!(ledger.targets_for_attempt().len(), 1);
}

#[test]
fn account_selection_is_stable_under_discovery_reordering() {
    let accounts = vec![account("b"), account("a"), account("c")];
    let selected = BTreeSet::from([
        LaunchAccountId("a".to_string()),
        LaunchAccountId("c".to_string()),
    ]);
    let picked = selected_account_ids(&accounts, &selected, false);
    assert_eq!(
        picked
            .into_iter()
            .map(|account| account.id.0)
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    assert_eq!(
        selected_account_ids(&accounts, &BTreeSet::new(), true).len(),
        3
    );
}
