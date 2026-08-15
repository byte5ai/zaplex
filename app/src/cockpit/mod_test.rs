use super::*;
use chrono::Utc;
use zaplex_cockpit::SessionState;

/// A Claude session at `cwd` whose transcript effort is `effort`.
fn snap(cwd: &str, effort: Option<String>) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "s".into(),
        cwd: cwd.into(),
        name: String::new(),
        state: SessionState::Active,
        provider: Provider::Claude,
        model: "opus".into(),
        effort,
        ctx_tokens: 0,
        project_root: cwd.into(),
        repo_root: cwd.into(),
        project_name: String::new(),
        branch: None,
        worktree: None,
        config_dir: None,
        account_email: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: 0,
    }
}

#[test]
fn antigravity_provider_maps_to_agy_cli() {
    assert_eq!(agent_of(Provider::Antigravity), CLIAgent::Antigravity);
}

#[test]
fn remote_effort_resolves_from_registry_by_stable_host_id() {
    // A remote Claude launch records its effort keyed by the daemon host_id.
    let cwd = "/remote/proj/effort-remote";
    let host_id = "daemon-host-id-xyz";
    launch_registry::record(
        CLIAgent::Claude,
        Some(host_id),
        Some(Path::new(cwd)),
        Some("opus".into()),
        Some("high".into()),
    );
    let s = snap(cwd, None);
    // Passing the inventory node's stable host_id now recovers the effort
    // that used to be dropped for remote sessions.
    assert_eq!(
        session_effort(&s, false, Some(host_id)).as_deref(),
        Some("high"),
    );
    // A different host id (another daemon at the same cwd) must NOT leak the
    // effort across hosts, and a missing id stays honestly unknown.
    assert_eq!(session_effort(&s, false, Some("other-id")), None);
    assert_eq!(session_effort(&s, false, None), None);
}

#[test]
fn snapshot_effort_wins_over_registry() {
    // When the transcript carried effort, it wins regardless of host.
    let s = snap("/remote/proj/snap-wins", Some("medium".into()));
    assert_eq!(
        session_effort(&s, false, Some("any-id")).as_deref(),
        Some("medium"),
    );
}

#[test]
fn local_effort_still_resolves_with_none_host() {
    // The local path is unchanged: keyed by (agent, None, cwd).
    let cwd = "/local/proj/effort-local";
    launch_registry::record(
        CLIAgent::Claude,
        None,
        Some(Path::new(cwd)),
        Some("sonnet".into()),
        Some("low".into()),
    );
    let s = snap(cwd, None);
    assert_eq!(session_effort(&s, true, None).as_deref(), Some("low"));
}

#[test]
fn default_dir_launch_effort_resolves_when_recorded_at_resolved_cwd() {
    // A local launch with no project selected lands in the shell's default
    // dir ($HOME) and the launch now records under that *resolved* cwd (not
    // `None`). The snapshot later reports the same concrete home path, so the
    // effort resolves — the mismatch that used to drop it (record `None` vs
    // lookup `Some(home)`) is gone.
    let home = "/home/effort-default-dir";
    launch_registry::record(
        CLIAgent::Claude,
        None,
        Some(Path::new(home)),
        Some("opus".into()),
        Some("high".into()),
    );
    let s = snap(home, None);
    assert_eq!(session_effort(&s, true, None).as_deref(), Some("high"));
}
