use super::*;

#[test]
fn only_ready_and_completed_states_accept_prompts() {
    let session = SessionIdentity::Codex("thread-1".to_string());
    let states = [
        AgentLifecycle::NoAgentInstalled,
        AgentLifecycle::NotSignedIn {
            agent: SubscriptionAgent::ClaudeCode,
        },
        AgentLifecycle::Starting,
        AgentLifecycle::Responding,
        AgentLifecycle::RunningTool {
            name: "shell".to_string(),
        },
        AgentLifecycle::WaitingForApproval {
            request_id: "approval-1".to_string(),
        },
        AgentLifecycle::SessionEnded,
        AgentLifecycle::RecoverableError {
            message: "disconnected".to_string(),
            session: None,
        },
    ];
    for state in states {
        assert_eq!(state.accepts_prompt(), false, "state: {state:?}");
    }
    assert_eq!(AgentLifecycle::Ready.accepts_prompt(), true);
    assert_eq!(
        AgentLifecycle::TurnCompleted { session }.accepts_prompt(),
        true
    );
}

#[test]
fn only_states_with_native_session_can_resume() {
    let session = SessionIdentity::ClaudeCode("session-1".to_string());
    assert_eq!(
        AgentLifecycle::TurnCompleted {
            session: session.clone()
        }
        .can_resume(),
        true
    );
    assert_eq!(
        AgentLifecycle::RecoverableError {
            message: "host disconnected".to_string(),
            session: Some(session),
        }
        .can_resume(),
        true
    );
    assert_eq!(AgentLifecycle::SessionEnded.can_resume(), false);
}
