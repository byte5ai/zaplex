use super::*;
use crate::terminal::cli_agent_sessions::event::CLIAgentEventType;

#[test]
fn codex_parses_any_text_as_stop() {
    let event = CodexSessionHandler::parse_osc9_text("Agent turn complete").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.agent, CLIAgent::Codex);
    assert_eq!(event.payload.query.as_deref(), Some("Agent turn complete"));
}

#[test]
fn codex_body_becomes_query() {
    let event =
        CodexSessionHandler::parse_osc9_text("I've updated the README with the new instructions.")
            .unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(
        event.payload.query.as_deref(),
        Some("I've updated the README with the new instructions.")
    );
}

#[test]
fn codex_approval_text_still_becomes_stop() {
    let event =
        CodexSessionHandler::parse_osc9_text("Approval requested: rm -rf /tmp/foo").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(
        event.payload.query.as_deref(),
        Some("Approval requested: rm -rf /tmp/foo")
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn codewhale_audit_reports_only_actual_approval_lifecycle() {
    let required = codewhale_audit_update(
        br#"{"event":"tool.approval_required","tool_id":"call-1","tool_name":"exec_shell"}"#,
    )
    .unwrap();
    assert_eq!(
        required,
        CodeWhaleAuditUpdate::ApprovalRequired {
            tool_id: "call-1".to_owned(),
            tool_name: Some("exec_shell".to_owned()),
        }
    );

    let decided = codewhale_audit_update(
        br#"{"event":"tool.approval_decision","tool_id":"call-1","tool_name":"exec_shell","decision":"approved"}"#,
    )
    .unwrap();
    assert_eq!(
        decided,
        CodeWhaleAuditUpdate::ApprovalDecided {
            tool_id: "call-1".to_owned(),
        }
    );

    assert!(codewhale_audit_update(
        br#"{"event":"tool.result","tool_id":"call-1","tool_name":"exec_shell"}"#
    )
    .is_none());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn codewhale_audit_reader_waits_for_complete_jsonl_records() {
    use std::io::Write as _;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("audit.jsonl");
    let first =
        br#"{"event":"tool.approval_required","tool_id":"call-1","tool_name":"write_file"}"#;
    let second =
        br#"{"event":"tool.approval_decision","tool_id":"call-1","tool_name":"write_file","decision":"denied"}"#;
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(first).unwrap();
    file.write_all(b"\n").unwrap();
    file.write_all(&second[..20]).unwrap();
    file.flush().unwrap();

    let first_read = read_codewhale_audit_events(&path, 0).unwrap();
    assert_eq!(first_read.updates.len(), 1);
    assert_eq!(
        first_read.updates[0],
        CodeWhaleAuditUpdate::ApprovalRequired {
            tool_id: "call-1".to_owned(),
            tool_name: Some("write_file".to_owned()),
        }
    );

    file.write_all(&second[20..]).unwrap();
    file.write_all(b"\n").unwrap();
    file.flush().unwrap();
    let second_read = read_codewhale_audit_events(&path, first_read.next_offset).unwrap();
    assert_eq!(second_read.updates.len(), 1);
    assert_eq!(
        second_read.updates[0],
        CodeWhaleAuditUpdate::ApprovalDecided {
            tool_id: "call-1".to_owned(),
        }
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn codewhale_audit_does_not_resurrect_an_already_decided_approval() {
    let required = CodeWhaleAuditUpdate::ApprovalRequired {
        tool_id: "call-1".to_owned(),
        tool_name: Some("exec_shell".to_owned()),
    };
    let decided = CodeWhaleAuditUpdate::ApprovalDecided {
        tool_id: "call-1".to_owned(),
    };
    let mut pending = HashSet::new();

    let same_batch = codewhale_events_for_audit_updates(&mut pending, vec![required, decided]);

    assert!(same_batch.is_empty());
    assert!(pending.is_empty());

    let required = CodeWhaleAuditUpdate::ApprovalRequired {
        tool_id: "call-2".to_owned(),
        tool_name: Some("write_file".to_owned()),
    };
    let events = codewhale_events_for_audit_updates(&mut pending, vec![required]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, CLIAgentEventType::PermissionRequest);
    assert_eq!(pending, HashSet::from(["call-2".to_owned()]));

    let decided = CodeWhaleAuditUpdate::ApprovalDecided {
        tool_id: "call-2".to_owned(),
    };
    let events = codewhale_events_for_audit_updates(&mut pending, vec![decided]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, CLIAgentEventType::PermissionReplied);
    assert!(pending.is_empty());
}

#[test]
fn codex_ignores_empty_body() {
    assert!(CodexSessionHandler::parse_osc9_text("").is_none());
    assert!(CodexSessionHandler::parse_osc9_text("   ").is_none());
}

#[test]
fn codex_try_parse_ignores_titled_notifications() {
    let handler = CodexSessionHandler;
    assert!(handler
        .try_parse(Some("some-title"), "Agent turn complete")
        .is_none());
}

#[test]
fn codex_try_parse_handles_osc9() {
    let handler = CodexSessionHandler;
    let event = handler.try_parse(None, "Agent turn complete").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Stop);
}

#[test]
fn grok_osc9_completion_is_detected_without_claiming_rich_status() {
    let handler = GrokSessionHandler;
    let event = handler.try_parse(None, "Grok turn complete").unwrap();
    assert_eq!(event.agent, CLIAgent::Grok);
    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.payload.query.as_deref(), Some("Grok turn complete"));

    let session = CLIAgentSession {
        agent: CLIAgent::Grok,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext::default(),
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };
    assert!(!session_supports_rich_status(&session));
}

#[test]
fn grok_structured_permission_request_preserves_typed_state() {
    let handler = GrokSessionHandler;
    let event = handler
        .try_parse(
            Some(
                crate::terminal::cli_agent_sessions::event::CLI_AGENT_NOTIFICATION_SENTINEL,
            ),
            r#"{"v":1,"agent":"grok","event":"permission_request","session_id":"grok-123","summary":"Approval required"}"#,
        )
        .expect("valid structured Grok event");
    assert_eq!(event.agent, CLIAgent::Grok);
    assert_eq!(event.event, CLIAgentEventType::PermissionRequest);
    assert_eq!(event.session_id.as_deref(), Some("grok-123"));
    assert_eq!(event.payload.summary.as_deref(), Some("Approval required"));
}

#[test]
fn documented_native_grok_hook_payload_reaches_the_typed_listener() {
    let body = super::super::hook_bridge::normalize_hook_input(
        CLIAgent::Grok,
        br#"{
            "sessionId":"grok-123",
            "cwd":"/workspace",
            "hookEventName":"pre_tool_use",
            "toolName":"run_terminal_cmd",
            "toolInput":{"command":"sensitive command"}
        }"#,
    )
    .expect("documented Grok hook payload should normalize");
    let body = String::from_utf8(body).unwrap();
    let handler = GrokSessionHandler;
    let event = handler
        .try_parse(
            Some(crate::terminal::cli_agent_sessions::event::CLI_AGENT_NOTIFICATION_SENTINEL),
            &body,
        )
        .expect("normalized native hook should reach the typed listener");

    assert_eq!(event.agent, CLIAgent::Grok);
    assert_eq!(event.event, CLIAgentEventType::PreToolUse);
    assert_eq!(event.session_id.as_deref(), Some("grok-123"));
    assert_eq!(event.payload.tool_name.as_deref(), Some("run_terminal_cmd"));
    assert!(!body.contains("sensitive command"));
}

#[test]
fn grok_structured_session_supports_rich_status() {
    let session = CLIAgentSession {
        agent: CLIAgent::Grok,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext {
            session_id: Some("grok-session-123".to_owned()),
            ..Default::default()
        },
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };
    assert!(session_supports_rich_status(&session));
}

#[test]
fn codex_legacy_osc9_session_is_not_rich_status() {
    let session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext::default(),
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };

    assert!(!session_supports_rich_status(&session));
}

#[test]
fn codex_structured_session_is_rich_status() {
    let session = CLIAgentSession {
        agent: CLIAgent::Codex,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext {
            session_id: Some("thr_123".to_owned()),
            ..Default::default()
        },
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };

    assert!(session_supports_rich_status(&session));
}

#[test]
fn repeated_session_start_reuses_listener_without_registering_again() {
    assert_eq!(
        listener_registration_action(Some((CLIAgent::Codex, true)), CLIAgent::Codex),
        ListenerRegistrationAction::Reuse
    );
}

#[test]
fn structured_event_rejects_a_second_agent_listener_for_the_same_terminal() {
    assert_eq!(
        listener_registration_action(Some((CLIAgent::Claude, true)), CLIAgent::Codex),
        ListenerRegistrationAction::Reject
    );
}

#[test]
fn structured_event_registers_when_session_has_no_listener() {
    assert_eq!(
        listener_registration_action(Some((CLIAgent::Codex, false)), CLIAgent::Codex),
        ListenerRegistrationAction::Register
    );
}

#[test]
fn auggie_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Auggie));
}

#[test]
fn auggie_uses_default_handler_with_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::Auggie));
}

#[test]
fn auggie_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn auggie_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn pi_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Pi));
}

#[test]
fn pi_uses_default_handler_with_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::Pi));
}

#[test]
fn pi_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn pi_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn antigravity_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Antigravity));
}

#[test]
fn antigravity_uses_default_handler_with_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::Antigravity));
}

#[test]
fn antigravity_claims_rich_status_only_after_a_structured_session_identity_arrives() {
    let mut session = CLIAgentSession {
        agent: CLIAgent::Antigravity,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext::default(),
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
    };
    assert!(!session_supports_rich_status(&session));

    session.session_context.session_id = Some("agy-123".to_owned());
    assert!(session_supports_rich_status(&session));
}

#[test]
fn documented_antigravity_status_payload_drives_blocked_and_active_state() {
    let pending = super::super::hook_bridge::normalize_antigravity_status_input(
        br#"{
            "conversation_id":"agy-123",
            "workspace":{"current_dir":"/workspace"},
            "agent_state":"tool_use",
            "tool_confirmation_pending":true
        }"#,
    )
    .unwrap();
    let handler = DefaultSessionListener;
    let body = String::from_utf8(pending[0].clone()).unwrap();
    let event = handler
        .try_parse(
            Some(crate::terminal::cli_agent_sessions::event::CLI_AGENT_NOTIFICATION_SENTINEL),
            &body,
        )
        .unwrap();
    assert_eq!(event.agent, CLIAgent::Antigravity);
    assert_eq!(event.event, CLIAgentEventType::PermissionRequest);
    assert_eq!(event.session_id.as_deref(), Some("agy-123"));

    let mut session = CLIAgentSession {
        agent: CLIAgent::Antigravity,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext::default(),
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
    };
    assert!(matches!(
        session.apply_event(&event),
        Some(super::super::CLIAgentSessionStatus::Blocked { .. })
    ));

    let active = super::super::hook_bridge::normalize_antigravity_status_input(
        br#"{
            "conversation_id":"agy-123",
            "workspace":{"current_dir":"/workspace"},
            "agent_state":"tool_use",
            "tool_confirmation_pending":false
        }"#,
    )
    .unwrap();
    for body in active {
        let body = String::from_utf8(body).unwrap();
        let event = handler
            .try_parse(
                Some(crate::terminal::cli_agent_sessions::event::CLI_AGENT_NOTIFICATION_SENTINEL),
                &body,
            )
            .unwrap();
        session.apply_event(&event);
    }
    assert_eq!(
        session.status,
        super::super::CLIAgentSessionStatus::InProgress
    );
}

#[test]
fn antigravity_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Antigravity,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn antigravity_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        v: 1,
        agent: CLIAgent::Antigravity,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn deepseek_handler_supports_structured_rich_status() {
    assert!(agent_supports_rich_status(&CLIAgent::DeepSeek));
}

#[test]
fn deepseek_osc9_completion_does_not_claim_prompt_text() {
    let handler = DeepSeekSessionHandler;
    let event = handler
        .try_parse(None, "deepseek: turn complete")
        .expect("DeepSeek OSC 9 body should parse");

    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.payload.query, None);
    assert_eq!(
        event.payload.response.as_deref(),
        Some("deepseek: turn complete")
    );
}

#[test]
fn deepseek_osc9_response_text_becomes_notification_title() {
    let handler = DeepSeekSessionHandler;
    let event = handler
        .try_parse(
            None,
            "最新回复内容\ndeepseek: turn complete (1m 15s, $0.01)",
        )
        .expect("DeepSeek OSC 9 body should parse");

    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.payload.query.as_deref(), Some("最新回复内容"));
    assert_eq!(
        event.payload.response.as_deref(),
        Some("最新回复内容\ndeepseek: turn complete (1m 15s, $0.01)")
    );
}

#[test]
fn deepseek_osc9_plain_response_text_becomes_notification_title() {
    let handler = DeepSeekSessionHandler;
    let event = handler
        .try_parse(None, "最新回复内容")
        .expect("DeepSeek OSC 9 body should parse");

    assert_eq!(event.event, CLIAgentEventType::Stop);
    assert_eq!(event.payload.query.as_deref(), Some("最新回复内容"));
    assert_eq!(event.payload.response.as_deref(), Some("最新回复内容"));
}

#[test]
fn deepseek_legacy_osc9_session_is_not_rich_status() {
    let session = CLIAgentSession {
        agent: CLIAgent::DeepSeek,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext::default(),
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };

    assert!(!session_supports_rich_status(&session));
}

#[test]
fn deepseek_structured_session_is_rich_status() {
    let session = CLIAgentSession {
        agent: CLIAgent::DeepSeek,
        status: super::super::CLIAgentSessionStatus::InProgress,
        session_context: super::super::CLIAgentSessionContext {
            session_id: Some("sess_1234".to_owned()),
            ..Default::default()
        },
        input_state: super::super::CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
    };

    assert!(session_supports_rich_status(&session));
}
