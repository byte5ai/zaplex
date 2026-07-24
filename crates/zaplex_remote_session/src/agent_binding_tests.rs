use super::agent_binding::{AgentIdentity, AgentPtyBindings, BindingError, BindingRequest};

fn identity(provider: &str, session_id: &str) -> AgentIdentity {
    AgentIdentity {
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        account_email: Some("agent@example.com".to_string()),
        config_dir: Some("/home/agent/.config".to_string()),
    }
}

fn request(
    agent: AgentIdentity,
    generation: u64,
    handoff_from: Option<AgentIdentity>,
) -> BindingRequest {
    BindingRequest {
        pty_session_id: "pty-1".to_string(),
        pty_generation: generation,
        agent,
        handoff_from,
    }
}

#[test]
fn bind_and_unbind_follow_session_lifecycle() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert_eq!(
        bindings.binding_for(&agent).unwrap().pty_session_id,
        "pty-1"
    );

    bindings.unbind(11, &agent, "pty-1", 7).unwrap();
    assert!(!bindings.binding_for(&agent).unwrap().foreground);

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert!(bindings.binding_for(&agent).unwrap().foreground);
    bindings.remove_pty("pty-1", 7);
    assert!(bindings.binding_for(&agent).is_none());
}

#[test]
fn new_generation_rejects_stale_pty_binding() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 8, 11);

    assert_eq!(
        bindings
            .bind(11, request(identity("claude", "agent-1"), 7, None))
            .unwrap_err(),
        BindingError::StaleGeneration
    );
}

#[test]
fn foreign_pty_id_is_rejected() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);

    assert_eq!(
        bindings
            .bind(12, request(identity("claude", "agent-1"), 7, None))
            .unwrap_err(),
        BindingError::ForeignConnection
    );
}

#[test]
fn multiple_historical_agents_can_share_pty() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);
    let first = identity("claude", "agent-1");
    let second = identity("claude", "agent-2");

    bindings.bind(11, request(first.clone(), 7, None)).unwrap();
    bindings
        .bind(11, request(second.clone(), 7, Some(first.clone())))
        .unwrap();

    assert!(!bindings.binding_for(&first).unwrap().foreground);
    assert!(bindings.binding_for(&second).unwrap().foreground);
    assert_eq!(bindings.bindings_for_pty("pty-1", 7).len(), 2);
}

#[test]
fn second_live_foreground_binding_requires_explicit_handoff() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);
    let first = identity("codex", "agent-1");
    let second = identity("codex", "agent-2");

    bindings.bind(11, request(first.clone(), 7, None)).unwrap();
    assert_eq!(
        bindings
            .bind(11, request(second.clone(), 7, None))
            .unwrap_err(),
        BindingError::ForegroundConflict
    );
    assert!(bindings.binding_for(&first).unwrap().foreground);
    assert!(bindings.binding_for(&second).is_none());

    bindings
        .bind(11, request(second.clone(), 7, Some(first.clone())))
        .unwrap();
    assert!(!bindings.binding_for(&first).unwrap().foreground);
    assert!(bindings.binding_for(&second).unwrap().foreground);
}

#[test]
fn agent_identity_cannot_bind_to_two_ptys() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);
    bindings.register_pty("pty-2", 8, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert_eq!(
        bindings
            .bind(
                11,
                BindingRequest {
                    pty_session_id: "pty-2".to_string(),
                    pty_generation: 8,
                    agent,
                    handoff_from: None,
                },
            )
            .unwrap_err(),
        BindingError::IdentityAlreadyBound
    );
}

#[test]
fn unbound_identity_can_move_to_another_pty_and_keeps_history() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, 11);
    bindings.register_pty("pty-2", 8, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    bindings.unbind(11, &agent, "pty-1", 7).unwrap();
    bindings
        .bind(
            11,
            BindingRequest {
                pty_session_id: "pty-2".to_string(),
                pty_generation: 8,
                agent: agent.clone(),
                handoff_from: None,
            },
        )
        .unwrap();

    assert_eq!(bindings.bindings_for_pty("pty-1", 7).len(), 1);
    assert!(!bindings.bindings_for_pty("pty-1", 7)[0].foreground);
    assert_eq!(bindings.bindings_for_pty("pty-2", 8).len(), 1);
    assert_eq!(
        bindings.binding_for(&agent).unwrap().pty_session_id,
        "pty-2",
        "inventory must route to the one live foreground association"
    );
}
