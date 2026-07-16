use super::*;
use crate::types::{Provider, SessionState};
use chrono::Utc;

fn session(name: &str, cwd: &str, pid: u32) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "s1".to_string(),
        cwd: cwd.to_string(),
        name: name.to_string(),
        state: SessionState::Active,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: cwd.to_string(),
        repo_root: cwd.to_string(),
        project_name: "proj".to_string(),
        branch: None,
        worktree: None,
        config_dir: None,
        account_email: None,
        last_activity: Utc::now(),
        pid,
    }
}

#[test]
fn interrupt_maps_to_sigint() {
    assert_eq!(GuardrailSignal::Interrupt.signal_number(), 2);
    assert_eq!(GuardrailSignal::Interrupt.shell_flag(), "INT");
}

#[test]
fn kill_maps_to_sigkill() {
    assert_eq!(GuardrailSignal::Kill.signal_number(), 9);
    assert_eq!(GuardrailSignal::Kill.shell_flag(), "KILL");
}

#[test]
fn remote_kill_command_is_a_plain_kill_invocation() {
    assert_eq!(
        GuardrailSignal::Interrupt.remote_kill_command(4242),
        "kill -INT 4242"
    );
    assert_eq!(
        GuardrailSignal::Kill.remote_kill_command(4242),
        "kill -KILL 4242"
    );
}

#[test]
fn guardrail_target_routes_by_explicit_locality() {
    // Local marker → Local, regardless of the label.
    assert_eq!(guardrail_target(true, "devhost"), GuardrailTarget::Local);
    // Remote marker → Remote, carrying the label for the daemon lookup.
    assert_eq!(
        guardrail_target(false, "macmini"),
        GuardrailTarget::Remote("macmini".to_string())
    );
}

#[test]
fn guardrail_target_label_collision_does_not_route_local() {
    // The exact P1: a REMOTE host whose label equals the local hostname must
    // still route Remote — never Local — because the pid is host-local and a
    // local `libc::kill` would signal an unrelated local process. Locality is
    // decided by `is_local`, not by the label string.
    assert_eq!(
        guardrail_target(false, "devhost"),
        GuardrailTarget::Remote("devhost".to_string()),
        "a remote host colliding with the local label must NOT be Local"
    );
}

#[test]
fn pid_zero_is_never_signalable() {
    assert!(!pid_signalable(0));
    assert!(pid_signalable(1234));
}

#[test]
fn session_label_matches_conductor_row_rule() {
    let named = session("build-agent", "/home/me/zaplex", 1);
    assert_eq!(session_label(&named), "build-agent — zaplex");

    let unnamed = session("", "/home/me/zaplex", 1);
    assert_eq!(session_label(&unnamed), "zaplex");
}

#[test]
fn kill_confirm_message_names_agent_and_project() {
    let (title, body) = kill_confirm_message("build-agent — zaplex", "zaplex");
    assert_eq!(title, "Kill \u{201c}build-agent — zaplex\u{201d}?");
    assert!(body.contains("SIGKILL"));
    assert!(body.contains("zaplex"));
}

#[test]
fn kill_confirm_message_handles_empty_project_name() {
    let (_, body) = kill_confirm_message("agent", "");
    assert!(body.contains("SIGKILL"));
    assert!(!body.contains("  in ")); // no dangling "in" clause
}

#[test]
fn stop_all_confirm_message_pluralizes() {
    let (title, _) = stop_all_confirm_message(1);
    assert_eq!(title, "Stop all 1 agent?");
    let (title, _) = stop_all_confirm_message(5);
    assert_eq!(title, "Stop all 5 agents?");
}

#[test]
fn stop_all_summary_reports_failures_honestly() {
    assert_eq!(stop_all_summary_toast(3, 0), "Stopped 3 agents.");
    assert_eq!(stop_all_summary_toast(1, 0), "Stopped 1 agent.");
    assert_eq!(
        stop_all_summary_toast(2, 1),
        "Stopped 2 agents, 1 could not be reached."
    );
}

#[test]
fn toasts_always_name_the_agent() {
    assert!(unsignalable_toast("agent").contains("agent"));
    assert!(sent_toast("agent", GuardrailSignal::Interrupt).contains("agent"));
    assert!(sent_toast("agent", GuardrailSignal::Kill).contains("Killed"));
    assert!(failed_toast("agent", GuardrailSignal::Interrupt, "boom").contains("boom"));
    assert!(no_remote_connection_toast("agent", "devhost").contains("devhost"));
}
