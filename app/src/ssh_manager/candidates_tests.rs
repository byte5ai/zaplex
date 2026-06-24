//! Pure function unit tests for `candidates::CandidatesViewModel::rows()`.
//!
//! Tests only verify the "state → row list" mapping; real IO (`load_candidates`), warpui
//! runtime don't participate. Coverage matrix: 5 state types (unloaded / NotFound / Error /
//! Empty / non-empty Loaded) × 2 collapse states × `added_aliases` hit / miss.

use std::collections::HashSet;
use std::path::PathBuf;

use warp_ssh_manager::SshConfigCandidate;

use super::{
    CandidateRow, CandidatesViewModel, fake_load_result_error, fake_load_result_loaded,
    fake_load_result_not_found,
};

fn cand(alias: &str) -> SshConfigCandidate {
    SshConfigCandidate {
        alias: alias.into(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    }
}

fn full_cand() -> SshConfigCandidate {
    SshConfigCandidate {
        alias: "prodbox".into(),
        hostname: Some("prod.example.com".into()),
        user: Some("alice".into()),
        port: Some(2222),
        identity_file: Some(PathBuf::from("/home/alice/.ssh/id_ed25519")),
    }
}

#[test]
fn rows_when_state_is_none_is_empty() {
    // Model just created, not yet refreshed — panel doesn't render section based on this.
    let vm = CandidatesViewModel::new();
    assert_eq!(vm.rows(), Vec::<CandidateRow>::new());
}

#[test]
fn rows_when_not_found_returns_header_plus_not_found() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_not_found("/home/u/.ssh/config")),
        HashSet::new(),
        true,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 2);
    assert!(matches!(
        rows[0],
        CandidateRow::Header {
            count: 0,
            can_refresh: true,
            ..
        }
    ));
    match &rows[1] {
        CandidateRow::NotFound { path_display } => {
            assert_eq!(path_display, "/home/u/.ssh/config");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn rows_when_error_returns_header_plus_error_with_message() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_error(
            "/home/u/.ssh/config",
            "permission denied (os error 13)",
        )),
        HashSet::new(),
        true,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 2);
    match &rows[1] {
        CandidateRow::Error {
            path_display,
            message,
        } => {
            assert_eq!(path_display, "/home/u/.ssh/config");
            assert_eq!(message, "permission denied (os error 13)");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn rows_when_loaded_empty_returns_header_plus_empty() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded("/home/u/.ssh/config", vec![])),
        HashSet::new(),
        true,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0], CandidateRow::Header { count: 0, .. }));
    assert!(matches!(rows[1], CandidateRow::Empty { .. }));
}

#[test]
fn rows_when_loaded_non_empty_returns_header_plus_one_per_candidate() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![cand("a"), cand("b"), cand("c")],
        )),
        HashSet::new(),
        true,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 4);
    match &rows[0] {
        CandidateRow::Header { count, .. } => assert_eq!(*count, 3),
        other => panic!("expected Header, got {other:?}"),
    }
    let aliases: Vec<&str> = rows[1..]
        .iter()
        .map(|r| match r {
            CandidateRow::Candidate { alias, .. } => alias.as_str(),
            other => panic!("expected Candidate, got {other:?}"),
        })
        .collect();
    assert_eq!(aliases, vec!["a", "b", "c"]);
}

#[test]
fn rows_marks_added_when_alias_in_added_set() {
    // PRODUCT.md decision E: imported → `added=true`, UI shows "Added" badge.
    let mut added = HashSet::new();
    added.insert("b".to_string());
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![cand("a"), cand("b"), cand("c")],
        )),
        added,
        true,
    );
    let rows = vm.rows();
    let marks: Vec<bool> = rows[1..]
        .iter()
        .map(|r| match r {
            CandidateRow::Candidate { added, .. } => *added,
            other => panic!("expected Candidate, got {other:?}"),
        })
        .collect();
    assert_eq!(marks, vec![false, true, false]);
}

#[test]
fn rows_propagates_all_candidate_fields() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![full_cand()],
        )),
        HashSet::new(),
        true,
    );
    let rows = vm.rows();
    match &rows[1] {
        CandidateRow::Candidate {
            alias,
            hostname,
            user,
            port,
            identity_file,
            added,
        } => {
            assert_eq!(alias, "prodbox");
            assert_eq!(hostname.as_deref(), Some("prod.example.com"));
            assert_eq!(user.as_deref(), Some("alice"));
            assert_eq!(*port, Some(2222));
            // PathBuf::display() may have different separators across platforms — use contains
            // rather than literal equality; verifying key path segments is sufficient.
            assert!(identity_file.as_deref().unwrap().contains("id_ed25519"));
            assert!(!*added);
        }
        other => panic!("expected Candidate, got {other:?}"),
    }
}

#[test]
fn rows_when_collapsed_returns_header_only() {
    // When collapsed, body (NotFound / Empty / Error / Candidate) doesn't render at all.
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![cand("a"), cand("b")],
        )),
        HashSet::new(),
        false,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 1);
    match &rows[0] {
        CandidateRow::Header { count, .. } => assert_eq!(*count, 2),
        other => panic!("expected Header, got {other:?}"),
    }
}

#[test]
fn find_candidate_returns_match_by_alias() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![cand("a"), full_cand(), cand("c")],
        )),
        HashSet::new(),
        true,
    );
    let got = vm.find_candidate("prodbox").expect("present");
    assert_eq!(got.hostname.as_deref(), Some("prod.example.com"));
    assert_eq!(got.port, Some(2222));
    assert!(vm.find_candidate("does-not-exist").is_none());
}

#[test]
fn path_display_reflects_state_path() {
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded("/etc/ssh/config", vec![])),
        HashSet::new(),
        true,
    );
    assert_eq!(vm.path_display().as_deref(), Some("/etc/ssh/config"));

    // When state is None, path_display returns None
    let empty = CandidatesViewModel::new();
    assert!(empty.path_display().is_none());
}

// ---- Explicit state=None scenario tests ----
// `refresh()` sets `state` to `None` when "auto-discovery is disabled", effect same as uninitialized.
// Tests below explicitly construct this state with `with_state(None, …)` to verify expected behavior of public methods.
// Note: These tests don't go through `refresh()` path, only verify output when `state = None`.

#[test]
fn rows_empty_when_explicit_state_none() {
    // state = None → rows returns empty Vec, panel doesn't render Candidates section based on this.
    let vm = CandidatesViewModel::with_state(None, HashSet::new(), true);
    assert_eq!(vm.rows(), Vec::<CandidateRow>::new());
}

#[test]
fn find_candidate_returns_none_when_explicit_state_none() {
    // state = None → find_candidate always returns None.
    let vm = CandidatesViewModel::with_state(None, HashSet::new(), true);
    assert!(vm.find_candidate("any-host").is_none());
}

#[test]
fn path_display_returns_none_when_explicit_state_none() {
    // state = None → path_display returns None.
    let vm = CandidatesViewModel::with_state(None, HashSet::new(), true);
    assert!(vm.path_display().is_none());
}

#[test]
fn added_aliases_still_works_when_state_loaded() {
    // When auto-discovery is enabled, on_tree_changed correctly updates added_aliases.
    let mut added = HashSet::new();
    added.insert("web-server".to_string());
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_loaded(
            "/home/u/.ssh/config",
            vec![cand("web-server"), cand("db-server")],
        )),
        added,
        true,
    );
    let rows = vm.rows();
    // Row 0 is Header, rows 1+ are Candidates
    match &rows[1] {
        CandidateRow::Candidate { alias, added, .. } => {
            assert_eq!(alias, "web-server");
            assert!(*added);
        }
        other => panic!("expected Candidate, got {other:?}"),
    }
    match &rows[2] {
        CandidateRow::Candidate { alias, added, .. } => {
            assert_eq!(alias, "db-server");
            assert!(!*added);
        }
        other => panic!("expected Candidate, got {other:?}"),
    }
}

#[test]
fn rows_when_collapsed_with_not_found_returns_header_only() {
    // Collapsed + NotFound state → returns only Header, body doesn't render.
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_not_found("/home/u/.ssh/config")),
        HashSet::new(),
        false,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0],
        CandidateRow::Header {
            count: 0,
            can_refresh: true,
            ..
        }
    ));
}

#[test]
fn rows_when_collapsed_with_error_returns_header_only() {
    // Collapsed + Error state → returns only Header.
    let vm = CandidatesViewModel::with_state(
        Some(fake_load_result_error("/home/u/.ssh/config", "io error")),
        HashSet::new(),
        false,
    );
    let rows = vm.rows();
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0],
        CandidateRow::Header {
            count: 0,
            can_refresh: true,
            ..
        }
    ));
}

// ---- refresh() settings branch tests ----
// Tests below actually call `refresh()` through warpui runtime to verify auto-discovery toggle settings branch.
// Unlike the `with_state(None, …)` tests above, these go through the real `refresh()` path.

#[test]
fn refresh_clears_state_when_auto_discovery_disabled() {
    use crate::settings::SshSettings;
    use crate::test_util::settings::initialize_settings_for_tests;
    use settings::Setting;
    use warpui::{App, SingletonEntity};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        // Disable auto-discovery
        SshSettings::handle(&app).update(&mut app, |s, ctx| {
            s.enable_ssh_auto_discovery.set_value(false, ctx).unwrap();
        });

        // Create model with preloaded candidate data (simulate "was enabled and loaded before")
        let handle = app.add_model(|_| {
            CandidatesViewModel::with_state(
                Some(fake_load_result_loaded(
                    "/home/u/.ssh/config",
                    vec![cand("web-server"), cand("db-server")],
                )),
                HashSet::new(),
                true,
            )
        });

        // refresh → auto-discovery disabled → state set to None
        handle.update(&mut app, |vm, ctx| {
            vm.refresh(ctx);
        });

        // rows() returns empty, panel doesn't render Candidates section based on this
        handle.read(&app, |vm, _| {
            assert_eq!(vm.rows(), Vec::<CandidateRow>::new());
        });
    });
}
