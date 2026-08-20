use super::*;
use crate::cockpit::palette::{CockpitPaletteKind, CockpitPaletteRecord, CockpitPaletteTarget};

fn record(key: &str, label: &str, search_text: &str, waiting: bool) -> CockpitPaletteRecord {
    CockpitPaletteRecord {
        kind: CockpitPaletteKind::Session,
        primary: label.to_string(),
        secondary: "Claude · local · zaplex".to_string(),
        search_text: search_text.to_string(),
        waiting,
        target: CockpitPaletteTarget::Session {
            key: key.to_string(),
            host: "local".to_string(),
            host_id: None,
            session_id: key.to_string(),
            provider: zaplex_cockpit::Provider::Claude,
            config_dir: None,
            account_email: None,
            account_id: None,
            is_local: true,
        },
    }
}

#[test]
fn fuzzy_search_matches_safe_projection_and_preserves_stable_action() {
    let results = search_records(
        vec![
            record("account-a", "Work", "claude work zaplex", false),
            record("account-b", "Personal", "codex personal notes", false),
        ],
        "cld wrk",
    );
    assert_eq!(results.len(), 1);
    match results[0].accept_result() {
        CommandPaletteItemAction::RunCockpitTarget { target } => {
            assert_eq!(target.stable_key(), "account-a")
        }
        action => panic!("unexpected action: {action:?}"),
    }
}

#[test]
fn waiting_bonus_only_breaks_equivalent_matches() {
    let results = search_records(
        vec![
            record("idle", "Same", "same session", false),
            record("waiting", "Same", "same session", true),
        ],
        "same",
    );
    assert_eq!(results.len(), 2);
    let idle = results
        .iter()
        .find(|result| {
            matches!(
                result.accept_result().to_summary(),
                crate::search::command_palette::mixer::ItemSummary::Cockpit { key }
                    if key == "idle"
            )
        })
        .unwrap();
    let waiting = results
        .iter()
        .find(|result| {
            matches!(
                result.accept_result().to_summary(),
                crate::search::command_palette::mixer::ItemSummary::Cockpit { key }
                    if key == "waiting"
            )
        })
        .unwrap();
    assert!(waiting.score() > idle.score());
}

#[test]
fn result_count_is_bounded() {
    let records = (0..MAX_COCKPIT_RESULTS + 20)
        .map(|index| record(&format!("key-{index}"), "Row", "row", false))
        .collect();
    assert_eq!(search_records(records, "").len(), MAX_COCKPIT_RESULTS);
}

#[test]
fn result_bound_keeps_the_best_match_even_when_it_was_indexed_last() {
    let mut records = (0..MAX_COCKPIT_RESULTS + 20)
        .map(|index| {
            record(
                &format!("background-{index}"),
                "Background",
                "needle background session",
                true,
            )
        })
        .collect::<Vec<_>>();
    records.push(record(
        "exact",
        "NeEdLe",
        "Claude local zaplex needle",
        false,
    ));

    let results = search_records(records, "needle");
    assert_eq!(results.len(), MAX_COCKPIT_RESULTS);
    assert!(matches!(
        results[0].accept_result().to_summary(),
        crate::search::command_palette::mixer::ItemSummary::Cockpit { key }
            if key == "exact"
    ));
    assert!(results.iter().any(|result| {
        matches!(
            result.accept_result().to_summary(),
            crate::search::command_palette::mixer::ItemSummary::Cockpit { key }
                if key == "exact"
        )
    }));
}

#[test]
fn case_insensitive_exact_label_outranks_a_waiting_prefix_match() {
    let results = search_records(
        vec![
            record("prefix", "Background", "needle background", true),
            record("exact", "NeEdLe", "Claude local zaplex needle", false),
        ],
        "needle",
    );

    assert_eq!(results.len(), 2);
    assert!(results[0].score() > results[1].score());
    assert!(matches!(
        results[0].accept_result().to_summary(),
        crate::search::command_palette::mixer::ItemSummary::Cockpit { key }
            if key == "exact"
    ));
}

#[test]
fn accessibility_and_keyboard_action_use_the_same_target() {
    let results = search_records(
        vec![record("stable", "Waiting PR", "waiting pr", true)],
        "waiting",
    );
    assert_eq!(
        results[0].accessibility_label(),
        "Agent session: Waiting PR. Claude · local · zaplex"
    );
    assert_eq!(
        results[0].accept_result().to_summary(),
        results[0].execute_result().to_summary()
    );
}

#[test]
fn every_cockpit_object_class_emits_its_exact_keyboard_target() {
    let repository = crate::cockpit::github_flows::RepositoryContext {
        slug: "byte5ai/zaplex".to_string(),
        worktree: "/projects/zaplex".into(),
        display_label: "zaplex".to_string(),
    };
    let targets = vec![
        (
            CockpitPaletteKind::Account,
            CockpitPaletteTarget::Account {
                account_key: "account:claude".to_string(),
            },
        ),
        (
            CockpitPaletteKind::Session,
            CockpitPaletteTarget::Session {
                key: "session:local:claude:one".to_string(),
                host: "local".to_string(),
                host_id: None,
                session_id: "one".to_string(),
                provider: zaplex_cockpit::Provider::Claude,
                config_dir: None,
                account_email: Some("me@example.com".to_string()),
                account_id: None,
                is_local: true,
            },
        ),
        (
            CockpitPaletteKind::Host,
            CockpitPaletteTarget::Host {
                key: "host:local".to_string(),
                registry_node_id: None,
                host_id: None,
                host: "local".to_string(),
                is_local: true,
            },
        ),
        (
            CockpitPaletteKind::Project,
            CockpitPaletteTarget::Project {
                key: "project:local:zaplex".to_string(),
                registry_node_id: None,
                host_id: None,
                host: "local".to_string(),
                is_local: true,
                project_root: "/projects/zaplex".into(),
            },
        ),
        (
            CockpitPaletteKind::GitHubFlow,
            CockpitPaletteTarget::GitHubFlow {
                key: "github:byte5ai/zaplex:quick_issue".to_string(),
                flow_key: crate::cockpit::github_flows::FLOW_QUICK_ISSUE.to_string(),
                repository,
            },
        ),
    ];

    for (kind, target) in targets {
        let key = target.stable_key().to_string();
        let result: QueryResult<CommandPaletteItemAction> = SearchItem::new(
            CockpitPaletteRecord {
                kind,
                primary: key.clone(),
                secondary: String::new(),
                search_text: key.clone(),
                waiting: false,
                target,
            },
            FuzzyMatchResult::no_match(),
        )
        .into();
        assert!(matches!(
            result.accept_result(),
            CommandPaletteItemAction::RunCockpitTarget { target }
                if target.stable_key() == key
        ));
    }
}
