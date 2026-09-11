use super::*;
use crate::ai::subscription_agent::{AccountIdentity, HostIdentity, SubscriptionAgent};

fn installation() -> InstallationIdentity {
    InstallationIdentity {
        agent: SubscriptionAgent::ClaudeCode,
        host: HostIdentity {
            id: "local".to_string(),
            display_name: "Local".to_string(),
        },
        account: AccountIdentity {
            id: "configured-account".to_string(),
            display_name: "Configured account".to_string(),
            provider_account_id: Some("account-42".to_string()),
            config_dir: Some("/accounts/claude".into()),
        },
        executable: "/usr/bin/claude".into(),
        version: "2.1.220".to_string(),
    }
}

#[test]
fn parses_models_and_account_from_initialize_response() {
    let frame = json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": "init-1",
            "response": {
                "account": {
                    "accountUuid": " ACCOUNT-42 ",
                    "email": "developer@example.com",
                    "organizationId": "org-1",
                    "subscriptionType": "pro"
                },
                "models": [{
                    "value": "default",
                    "displayName": "Default (recommended)",
                    "description": "Uses the account default",
                    "resolvedModel": "claude-reported-current",
                    "supportedEffortLevels": ["low", "high"],
                    "defaultEffort": "high"
                }, {
                    "value": "claude-reported-fast",
                    "displayName": "Fast",
                    "supportsEffort": false
                }]
            }
        }
    });

    let capability = ClaudeProtocol::parse_capability(&frame, installation()).unwrap();

    assert_eq!(capability.installation.account.id, "configured-account");
    assert_eq!(
        capability
            .installation
            .account
            .provider_account_id
            .as_deref(),
        Some("account-42")
    );
    assert_eq!(
        capability.installation.account.display_name,
        "developer@example.com"
    );
    assert_eq!(capability.models.len(), 2);
    assert_eq!(capability.models[0].id, "default");
    assert_eq!(
        capability.models[0].resolved_model.as_deref(),
        Some("claude-reported-current")
    );
    assert_eq!(capability.models[0].is_default, true);
    assert_eq!(
        capability.models[0]
            .supported_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "high"]
    );
}

#[test]
fn rejects_same_display_name_when_provider_account_id_differs() {
    let frame = json!({
        "response": {
            "account": {
                "accountUuid": "different-account",
                "email": "Configured account"
            },
            "models": [{"value": "default"}]
        }
    });

    let error = ClaudeProtocol::parse_capability(&frame, installation()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Claude Code authenticated account does not match selected account Configured account; sign in to that account in the selected CLI profile and retry"
    );
}

#[test]
fn rejects_selected_account_when_structured_identity_is_missing() {
    let frame = json!({
        "response": {
            "account": {"email": "Configured account"},
            "models": [{"value": "default"}]
        }
    });

    let error = ClaudeProtocol::parse_capability(&frame, installation()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Claude Code did not report an account ID for selected account Configured account; refresh that CLI login and retry"
    );
}

#[test]
fn parses_session_text_tool_approval_usage_and_completion() {
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "system",
            "subtype": "init",
            "session_id": "session-1"
        }))
        .unwrap(),
        vec![SubscriptionEvent::SessionStarted(
            SessionIdentity::ClaudeCode("session-1".to_string())
        )]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "text",
                    "text": "Hello"
                }, {
                    "type": "tool_use",
                    "id": "tool-1",
                    "name": "Read",
                    "input": {"file_path": "README.md"}
                }]
            }
        }))
        .unwrap(),
        vec![
            SubscriptionEvent::TextDelta("Hello".to_string()),
            SubscriptionEvent::ToolStarted {
                id: "tool-1".to_string(),
                name: "Read".to_string(),
                input: json!({"file_path": "README.md"}),
            },
        ]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "thinking_delta", "thinking": "Checking the file"}
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::ReasoningDelta(
            "Checking the file".to_string()
        )]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool-1",
                    "content": "permission denied",
                    "is_error": true
                }]
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::ToolOutput {
            id: "tool-1".to_string(),
            output: "permission denied".to_string(),
            is_error: true,
        }]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "control_request",
            "request_id": "approval-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": {"command": "cargo check"}
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::ApprovalRequested {
            request_id: "approval-1".to_string(),
            kind: "Bash".to_string(),
            description: "Bash".to_string(),
            input: json!({"command": "cargo check"}),
        }]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "result",
            "session_id": "session-1",
            "is_error": false,
            "usage": {
                "input_tokens": 10,
                "cache_read_input_tokens": 4,
                "output_tokens": 6
            }
        }))
        .unwrap(),
        vec![
            SubscriptionEvent::Usage(Usage {
                input_tokens: 10,
                cached_input_tokens: 4,
                output_tokens: 6,
            }),
            SubscriptionEvent::TurnCompleted {
                session: SessionIdentity::ClaudeCode("session-1".to_string()),
            },
        ]
    );
    assert_eq!(
        ClaudeProtocol::parse_event(&json!({
            "type": "result",
            "session_id": "session-1",
            "is_error": true,
            "result": "tool execution failed"
        }))
        .unwrap(),
        vec![SubscriptionEvent::Error {
            message: "tool execution failed".to_string(),
            recoverable: true,
            session: Some(SessionIdentity::ClaudeCode("session-1".to_string())),
        }]
    );
}

#[test]
fn approval_response_never_bypasses_the_protocol() {
    assert_eq!(
        ClaudeProtocol::approval_response(
            "approval-1",
            ApprovalDecision::Allow,
            &json!({"command": "pwd"}),
        ),
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "approval-1",
                "response": {
                    "behavior": "allow",
                    "updatedInput": {"command": "pwd"}
                }
            }
        })
    );

    assert_eq!(
        ClaudeProtocol::approval_response(
            "approval-2",
            ApprovalDecision::Deny,
            &json!({"command": "rm file"}),
        ),
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "approval-2",
                "response": {
                    "behavior": "deny",
                    "message": "Denied by user"
                }
            }
        })
    );
}
