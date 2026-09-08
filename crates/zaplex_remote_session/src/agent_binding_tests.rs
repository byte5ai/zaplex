use super::{
    AgentIdentity, AgentPtyBindings, BindingError, BindingRequest, MAX_HISTORICAL_BINDINGS,
};
use std::collections::HashSet;

const HOST: &str = "daemon-host-1";

fn identity(provider: &str, session_id: &str) -> AgentIdentity {
    AgentIdentity {
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        account_email: Some("agent@example.com".to_string()),
        config_dir: Some("/home/agent/.config".to_string()),
        account_id: None,
    }
}

fn request(
    agent: AgentIdentity,
    generation: u64,
    handoff_from: Option<AgentIdentity>,
) -> BindingRequest {
    BindingRequest {
        host_id: HOST.to_string(),
        pty_session_id: "pty-1".to_string(),
        pty_generation: generation,
        agent,
        handoff_from,
    }
}

#[test]
fn closed_binding_history_is_bounded() {
    let mut bindings = AgentPtyBindings::default();
    let history_size = MAX_HISTORICAL_BINDINGS + 5;

    for index in 0..history_size {
        let pty_session_id = format!("pty-{index}");
        let agent = identity("codex", &format!("agent-{index}"));
        bindings.register_pty(&pty_session_id, 1, HOST, 11);
        bindings
            .bind(
                11,
                BindingRequest {
                    host_id: HOST.to_string(),
                    pty_session_id: pty_session_id.clone(),
                    pty_generation: 1,
                    agent,
                    handoff_from: None,
                },
            )
            .unwrap();
        bindings.remove_pty(&pty_session_id, 1);
    }

    assert!(bindings.bindings.len() <= MAX_HISTORICAL_BINDINGS);
    let newest = identity("codex", &format!("agent-{}", history_size - 1));
    assert_eq!(
        bindings.binding_for(&newest).unwrap().pty_session_id,
        format!("pty-{}", history_size - 1)
    );
    assert!(bindings
        .binding_for(&identity("codex", "agent-0"))
        .is_none());
}

#[test]
fn registered_generation_history_is_never_pruned() {
    let mut bindings = AgentPtyBindings::default();
    let protected_history_size = MAX_HISTORICAL_BINDINGS + 5;
    bindings.register_pty("protected-pty", 1, HOST, 11);

    let mut foreground = None;
    for index in 0..protected_history_size {
        let agent = identity("codex", &format!("protected-agent-{index}"));
        bindings
            .bind(
                11,
                BindingRequest {
                    host_id: HOST.to_string(),
                    pty_session_id: "protected-pty".to_string(),
                    pty_generation: 1,
                    agent: agent.clone(),
                    handoff_from: foreground,
                },
            )
            .unwrap();
        foreground = Some(agent);
    }

    for index in 0..MAX_HISTORICAL_BINDINGS + 5 {
        let pty_session_id = format!("closed-pty-{index}");
        bindings.register_pty(&pty_session_id, 1, HOST, 11);
        bindings
            .bind(
                11,
                BindingRequest {
                    host_id: HOST.to_string(),
                    pty_session_id: pty_session_id.clone(),
                    pty_generation: 1,
                    agent: identity("codex", &format!("closed-agent-{index}")),
                    handoff_from: None,
                },
            )
            .unwrap();
        bindings.remove_pty(&pty_session_id, 1);
    }

    assert_eq!(
        bindings.bindings_for_pty("protected-pty", 1).len(),
        protected_history_size
    );
    assert!(bindings.foreground_for_pty("protected-pty", 1).is_some());
}

#[test]
fn bind_and_unbind_follow_agent_and_pty_lifecycle() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert_eq!(
        bindings.binding_for(&agent).unwrap().pty_session_id,
        "pty-1"
    );

    bindings.unbind(11, HOST, &agent, "pty-1", 7).unwrap();
    assert!(!bindings.binding_for(&agent).unwrap().foreground);

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert!(bindings.binding_for(&agent).unwrap().foreground);
    bindings.remove_pty("pty-1", 7);
    let historical = bindings.binding_for(&agent).unwrap();
    assert!(!historical.foreground);
    assert_eq!(historical.pty_generation, 7);
}

#[test]
fn stale_binding_generation_cannot_replace_current_binding() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 8, HOST, 11);

    assert_eq!(
        bindings
            .bind(11, request(identity("claude", "agent-1"), 7, None))
            .unwrap_err(),
        BindingError::StaleGeneration
    );
}

#[test]
fn foreign_host_or_daemon_binding_is_rejected() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);
    let mut foreign = request(identity("claude", "agent-1"), 7, None);
    foreign.host_id = "daemon-host-2".to_string();

    assert_eq!(
        bindings.bind(11, foreign).unwrap_err(),
        BindingError::ForeignDaemon
    );
}

#[test]
fn foreign_connection_is_rejected_independently_of_daemon_identity() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);

    assert_eq!(
        bindings
            .bind(12, request(identity("claude", "agent-1"), 7, None))
            .unwrap_err(),
        BindingError::ForeignConnection
    );
}

#[test]
fn unknown_pty_binding_is_never_attached() {
    let mut bindings = AgentPtyBindings::default();

    assert_eq!(
        bindings
            .bind(11, request(identity("claude", "agent-1"), 7, None))
            .unwrap_err(),
        BindingError::PtyNotFound
    );
}

#[test]
fn multiple_agent_sessions_can_reference_the_same_pty() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);
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
fn second_live_agent_binding_to_same_pty_is_rejected() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);
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
    bindings.register_pty("pty-1", 7, HOST, 11);
    bindings.register_pty("pty-2", 8, HOST, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    assert_eq!(
        bindings
            .bind(
                11,
                BindingRequest {
                    host_id: HOST.to_string(),
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
    bindings.register_pty("pty-1", 7, HOST, 11);
    bindings.register_pty("pty-2", 8, HOST, 11);
    let agent = identity("codex", "agent-1");

    bindings.bind(11, request(agent.clone(), 7, None)).unwrap();
    bindings.unbind(11, HOST, &agent, "pty-1", 7).unwrap();
    bindings
        .bind(
            11,
            BindingRequest {
                host_id: HOST.to_string(),
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

#[test]
fn ended_agent_is_demoted_but_keeps_its_historical_pty() {
    let mut bindings = AgentPtyBindings::default();
    bindings.register_pty("pty-1", 7, HOST, 11);
    let ended = identity("codex", "agent-ended");
    bindings.bind(11, request(ended.clone(), 7, None)).unwrap();

    bindings.reconcile_live_agents(&HashSet::new());

    let historical = bindings.binding_for(&ended).unwrap();
    assert!(!historical.foreground);
    assert_eq!(historical.pty_session_id, "pty-1");
    assert_eq!(historical.pty_generation, 7);
    assert!(bindings.foreground_for_pty("pty-1", 7).is_none());
}

#[test]
fn reused_pty_id_keeps_generations_distinct_and_old_history_non_attachable() {
    let mut bindings = AgentPtyBindings::default();
    let old = identity("codex", "agent-old");
    let new = identity("codex", "agent-new");
    bindings.register_pty("pty-1", 7, HOST, 11);
    bindings.bind(11, request(old.clone(), 7, None)).unwrap();

    bindings.register_pty("pty-1", 8, HOST, 11);
    bindings.bind(11, request(new.clone(), 8, None)).unwrap();

    assert_eq!(bindings.bindings_for_pty("pty-1", 7).len(), 1);
    assert!(!bindings.bindings_for_pty("pty-1", 7)[0].foreground);
    assert_eq!(bindings.bindings_for_pty("pty-1", 8).len(), 1);
    assert!(bindings.bindings_for_pty("pty-1", 8)[0].foreground);
    assert!(!bindings.binding_for(&old).unwrap().foreground);
    assert!(bindings.binding_for(&new).unwrap().foreground);
}
