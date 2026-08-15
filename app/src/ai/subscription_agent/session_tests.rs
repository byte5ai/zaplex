use super::*;

#[test]
fn resume_session_must_match_selected_agent() {
    assert_eq!(
        validate_resume_agent(
            SubscriptionAgent::ClaudeCode,
            Some(&SessionIdentity::Codex("thread-1".to_string())),
        )
        .unwrap_err()
        .to_string(),
        "cannot resume a Codex thread with Claude Code"
    );
    assert_eq!(
        validate_resume_agent(
            SubscriptionAgent::Codex,
            Some(&SessionIdentity::ClaudeCode("session-1".to_string())),
        )
        .unwrap_err()
        .to_string(),
        "cannot resume a Claude session with Codex"
    );
    assert_eq!(
        validate_resume_agent(
            SubscriptionAgent::Codex,
            Some(&SessionIdentity::Codex("thread-1".to_string())),
        )
        .is_ok(),
        true
    );
}

#[test]
fn app_server_error_preserves_upgrade_context() {
    assert_eq!(
        ensure_success(
            serde_json::json!({
                "id": 1,
                "error": {
                    "code": -32601,
                    "message": "method not found"
                }
            }),
            "Codex app-server initialize",
        )
        .unwrap_err()
        .to_string(),
        "Codex app-server initialize failed: method not found"
    );
}
