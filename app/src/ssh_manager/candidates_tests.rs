//! Pure function unit tests for `candidates::CandidatesViewModel::rows()`.
//!
//! Tests only verify the "state → row list" mapping; real IO (`load_candidates`), warpui
//! runtime don't participate. Coverage matrix: 5 state types (unloaded / NotFound / Error /
//! Empty / non-empty Loaded) × 2 collapse states × `added_aliases` hit / miss.

use std::collections::HashSet;
use std::path::PathBuf;

use warp_ssh_manager::SshConfigCandidate;

use super::{fake_load_result_loaded, CandidateRow, CandidatesViewModel};

fn cand(alias: &str) -> SshConfigCandidate {
    SshConfigCandidate {
        alias: alias.into(),
        hostname: None,
        user: None,
        port: None,
        invalid_port: None,
        identity_file: None,
    }
}

fn full_cand() -> SshConfigCandidate {
    SshConfigCandidate {
        alias: "prodbox".into(),
        hostname: Some("prod.example.com".into()),
        user: Some("alice".into()),
        port: Some(2222),
        invalid_port: None,
        identity_file: Some(PathBuf::from("/home/alice/.ssh/id_ed25519")),
    }
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
fn find_candidate_returns_none_when_explicit_state_none() {
    // state = None → find_candidate always returns None.
    let vm = CandidatesViewModel::with_state(None, HashSet::new(), true);
    assert!(vm.find_candidate("any-host").is_none());
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
