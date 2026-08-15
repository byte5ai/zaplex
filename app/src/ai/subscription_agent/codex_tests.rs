use super::*;
use crate::ai::subscription_agent::{AccountIdentity, HostIdentity, SubscriptionAgent};

fn installation() -> InstallationIdentity {
    InstallationIdentity {
        agent: SubscriptionAgent::Codex,
        host: HostIdentity {
            id: "local".to_string(),
            display_name: "Local".to_string(),
        },
        account: AccountIdentity {
            id: "configured-account".to_string(),
            display_name: "Configured account".to_string(),
            config_dir: Some("/accounts/codex".into()),
        },
        executable: "/usr/bin/codex".into(),
        version: "0.146.0".to_string(),
    }
}

fn target() -> SubscriptionTarget {
    SubscriptionTarget {
        installation: installation(),
        working_directory: "/workspace".into(),
        model: ModelCapability {
            id: "gpt-reported-current".to_string(),
            display_name: "Current".to_string(),
            description: None,
            resolved_model: Some("gpt-reported-current".to_string()),
            is_default: true,
            supported_efforts: vec![ModelEffort {
                id: "high".to_string(),
                display_name: "High".to_string(),
            }],
            default_effort: Some("high".to_string()),
            context_window: None,
        },
        effort: Some("high".to_string()),
    }
}

#[test]
fn parses_subscription_account_and_exact_reported_models() {
    let account = json!({
        "id": 2,
        "result": {
            "account": {
                "type": "chatgpt",
                "email": "developer@example.com",
                "planType": "plus"
            },
            "requiresOpenaiAuth": true
        }
    });
    let models = json!({
        "id": 3,
        "result": {
            "data": [{
                "id": "gpt-reported-current",
                "model": "gpt-reported-current",
                "displayName": "Current",
                "description": "Reported by app-server",
                "hidden": false,
                "isDefault": true,
                "defaultReasoningEffort": "medium",
                "supportedReasoningEfforts": [{
                    "reasoningEffort": "low",
                    "description": "Low"
                }, {
                    "reasoningEffort": "medium",
                    "description": "Medium"
                }]
            }, {
                "id": "hidden-model",
                "model": "hidden-model",
                "displayName": "Hidden",
                "description": "Not selectable",
                "hidden": true,
                "isDefault": false,
                "defaultReasoningEffort": "medium",
                "supportedReasoningEfforts": []
            }]
        }
    });

    let capability = CodexProtocol::parse_capability(&account, &models, installation()).unwrap();

    assert_eq!(capability.installation.account.id, "developer@example.com");
    assert_eq!(capability.models.len(), 1);
    assert_eq!(capability.models[0].id, "gpt-reported-current");
    assert_eq!(capability.models[0].is_default, true);
    assert_eq!(
        capability.models[0].default_effort.as_deref(),
        Some("medium")
    );
}

#[test]
fn rejects_api_key_account_in_subscription_path() {
    let result = CodexProtocol::parse_capability(
        &json!({
            "result": {
                "account": {"type": "apiKey"},
                "requiresOpenaiAuth": true
            }
        }),
        &json!({"result": {"data": []}}),
        installation(),
    );

    assert_eq!(
        result.unwrap_err().to_string(),
        "Codex is not using a ChatGPT subscription account"
    );
}

#[test]
fn request_sequence_uses_native_threads_turns_and_manual_approvals() {
    let target = target();

    assert_eq!(
        CodexProtocol::thread_start_request(4, &target),
        json!({
            "id": 4,
            "method": "thread/start",
            "params": {
                "cwd": "/workspace",
                "model": "gpt-reported-current",
                "sandbox": "workspace-write",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "ephemeral": false
            }
        })
    );
    assert_eq!(
        CodexProtocol::turn_start_request(5, &target, "thread-1", "Hello"),
        json!({
            "id": 5,
            "method": "turn/start",
            "params": {
                "threadId": "thread-1",
                "input": [{"type": "text", "text": "Hello"}],
                "cwd": "/workspace",
                "model": "gpt-reported-current",
                "effort": "high",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user"
            }
        })
    );
}

#[test]
fn parses_text_reasoning_tool_approval_usage_diff_and_completion() {
    assert_eq!(
        CodexProtocol::parse_event(&json!({
            "method": "item/agentMessage/delta",
            "params": {"delta": "Hello", "threadId": "thread-1", "turnId": "turn-1"}
        }))
        .unwrap(),
        vec![SubscriptionEvent::TextDelta("Hello".to_string())]
    );
    assert_eq!(
        CodexProtocol::parse_event(&json!({
            "method": "item/commandExecution/requestApproval",
            "id": 41,
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "command": "cargo check",
                "reason": "Run validation"
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::ApprovalRequested {
            request_id: "41".to_string(),
            kind: "item/commandExecution/requestApproval".to_string(),
            description: "Run validation".to_string(),
            input: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "command": "cargo check",
                "reason": "Run validation"
            }),
        }]
    );
    assert_eq!(
        CodexProtocol::parse_event(&json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "tokenUsage": {
                    "last": {
                        "inputTokens": 12,
                        "cachedInputTokens": 5,
                        "outputTokens": 7
                    }
                }
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::Usage(Usage {
            input_tokens: 12,
            cached_input_tokens: 5,
            output_tokens: 7,
        })]
    );
    assert_eq!(
        CodexProtocol::parse_event(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "completed",
                    "items": []
                }
            }
        }))
        .unwrap(),
        vec![SubscriptionEvent::TurnCompleted {
            session: SessionIdentity::Codex("thread-1".to_string())
        }]
    );
}

#[test]
fn approval_responses_preserve_json_rpc_request_id() {
    assert_eq!(
        CodexProtocol::approval_response(
            json!(41),
            "item/commandExecution/requestApproval",
            &json!({}),
            ApprovalDecision::AllowForSession,
        ),
        json!({
            "id": 41,
            "result": {"decision": "acceptForSession"}
        })
    );
    assert_eq!(
        CodexProtocol::approval_response(
            json!(42),
            "item/fileChange/requestApproval",
            &json!({}),
            ApprovalDecision::Cancel,
        ),
        json!({
            "id": 42,
            "result": {"decision": "cancel"}
        })
    );
}

#[test]
fn permission_approval_echoes_only_the_requested_profile() {
    assert_eq!(
        CodexProtocol::approval_response(
            json!(43),
            "item/permissions/requestApproval",
            &json!({
                "permissions": {
                    "network": {"enabled": true}
                },
                "reason": "Download dependencies"
            }),
            ApprovalDecision::AllowForSession,
        ),
        json!({
            "id": 43,
            "result": {
                "permissions": {
                    "network": {"enabled": true}
                },
                "scope": "session"
            }
        })
    );
}
