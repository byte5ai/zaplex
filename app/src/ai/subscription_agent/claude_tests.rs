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

    assert_eq!(capability.installation.account.id, "developer@example.com");
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
}
