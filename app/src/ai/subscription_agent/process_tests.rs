use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
};

fn target(agent: SubscriptionAgent) -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local".to_string(),
            },
            account: AccountIdentity {
                id: "account-1".to_string(),
                display_name: "developer@example.com".to_string(),
                config_dir: Some("/accounts/with space".into()),
            },
            executable: match agent {
                SubscriptionAgent::ClaudeCode => "/usr/bin/claude".into(),
                SubscriptionAgent::Codex => "/usr/bin/codex".into(),
            },
            version: "1.0".to_string(),
        },
        working_directory: "/workspace/with space".into(),
        model: ModelCapability {
            id: "reported-model".to_string(),
            display_name: "Reported model".to_string(),
            description: None,
            resolved_model: None,
            is_default: true,
            supported_efforts: Vec::new(),
            default_effort: None,
            context_window: None,
        },
        effort: None,
    }
}

#[test]
fn claude_launch_uses_structured_protocol_and_subscription_environment() {
    let launch = ProcessLaunch::for_session(
        &target(SubscriptionAgent::ClaudeCode),
        Some("session-1"),
        ProcessLocation::Local,
    );

    assert_eq!(
        launch.unset_environment,
        vec!["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
    );
    assert_eq!(
        launch.environment,
        vec![("CLAUDE_CONFIG_DIR", "/accounts/with space".to_string())]
    );
    assert_eq!(launch.args.contains(&"stream-json".to_string()), true);
    assert_eq!(launch.args.contains(&"default".to_string()), true);
    assert_eq!(
        launch
            .args
            .contains(&"--dangerously-skip-permissions".to_string()),
        false
    );
    assert_eq!(
        launch
            .args
            .windows(2)
            .any(|args| args == ["--resume", "session-1"]),
        true
    );
}

#[test]
fn codex_launch_uses_app_server_and_subscription_environment() {
    let launch = ProcessLaunch::for_session(
        &target(SubscriptionAgent::Codex),
        None,
        ProcessLocation::Local,
    );

    assert_eq!(launch.unset_environment, vec!["OPENAI_API_KEY"]);
    assert_eq!(
        launch.environment,
        vec![("CODEX_HOME", "/accounts/with space".to_string())]
    );
    assert_eq!(launch.args, vec!["app-server", "--listen", "stdio://"]);
}

#[test]
fn remote_launch_quotes_working_directory_environment_and_model() {
    let launch = ProcessLaunch::for_session(
        &target(SubscriptionAgent::ClaudeCode),
        None,
        ProcessLocation::Remote {
            ssh_argv: vec!["ssh".to_string(), "host".to_string()],
        },
    );
    let command = launch.remote_command();

    assert_eq!(command.starts_with("cd -- '/workspace/with space'"), true);
    assert_eq!(command.contains("-u ANTHROPIC_API_KEY"), true);
    assert_eq!(
        command.contains("'CLAUDE_CONFIG_DIR=/accounts/with space'"),
        true
    );
    assert_eq!(command.contains("--model reported-model"), true);
}
