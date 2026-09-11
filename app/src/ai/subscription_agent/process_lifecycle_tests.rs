use super::*;

#[cfg(unix)]
use crate::ai::subscription_agent::{
    AccountIdentity, ApprovalDecision, HostIdentity, InstallationIdentity, ModelCapability,
    SessionIdentity, SubscriptionAgent, SubscriptionEvent, SubscriptionSession, SubscriptionTarget,
    Usage,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn write_executable(path: &std::path::Path, script: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    shell_words::quote(path.to_str().unwrap()).into_owned()
}

#[cfg(unix)]
fn subscription_target(
    agent: SubscriptionAgent,
    executable: std::path::PathBuf,
    working_directory: std::path::PathBuf,
    config_dir: std::path::PathBuf,
) -> SubscriptionTarget {
    let (provider_account_id, model_id) = match agent {
        SubscriptionAgent::ClaudeCode => ("claude-account-42", "default"),
        SubscriptionAgent::Codex => ("codex-account-42", "gpt-test"),
    };
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent,
            host: HostIdentity {
                id: "test-host".to_string(),
                display_name: "Test host".to_string(),
            },
            account: AccountIdentity {
                id: "test-route".to_string(),
                display_name: "developer@example.com".to_string(),
                provider_account_id: Some(provider_account_id.to_string()),
                config_dir: Some(config_dir),
            },
            executable,
            version: "test-version".to_string(),
        },
        working_directory,
        model: ModelCapability {
            id: model_id.to_string(),
            display_name: "Test model".to_string(),
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

#[cfg(unix)]
fn fake_process(script: &str) -> (tempfile::TempDir, JsonLineProcess) {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-subscription-agent");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let launch = ProcessLaunch {
        program: executable,
        args: Vec::new(),
        environment: Vec::new(),
        unset_environment: Vec::new(),
        working_directory: directory.path().to_path_buf(),
        location: ProcessLocation::Local,
    };
    let process = JsonLineProcess::spawn(&launch).unwrap();
    (directory, process)
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn process_exits_gracefully_when_protocol_input_closes() {
    futures_lite::future::block_on(async {
        let (_directory, mut process) = fake_process("cat >/dev/null");

        let termination = process
            .terminate_with_timeouts(Duration::from_secs(1), Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(termination, ProcessTermination::Graceful);
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn process_exit_timeout_escalates_and_reaps_the_child() {
    futures_lite::future::block_on(async {
        let (_directory, mut process) = fake_process("while :; do :; done");
        let pid = process.child.id();

        let termination = process
            .terminate_with_timeouts(Duration::from_millis(20), Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(termination, ProcessTermination::Forced);
        assert!(process.child.try_status().unwrap().is_some());
        assert_eq!(
            std::path::Path::new(&format!("/proc/{pid}")).exists(),
            false
        );
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn local_fake_claude_covers_account_cwd_events_approval_and_completion() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let working_directory = directory.path().join("working directory");
        let config_dir = directory.path().join("claude account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let executable = directory.path().join("fake-claude");
        let observed_cwd = directory.path().join("claude-cwd");
        let observed_config = directory.path().join("claude-config");
        let observed_key = directory.path().join("claude-api-key");
        let observed_approval = directory.path().join("claude-approval");
        let script = r#"
printf '%s' "$PWD" > __CWD__
printf '%s' "${CLAUDE_CONFIG_DIR-unset}" > __CONFIG__
printf '%s' "${ANTHROPIC_API_KEY-unset}" > __KEY__
IFS= read -r initialize
printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"zaplex-session-initialize","response":{"account":{"accountUuid":"claude-account-42","email":"developer@example.com"},"models":[{"value":"default","displayName":"Default"}]}}}'
IFS= read -r prompt
printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-session-real"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"Inspecting"}}}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"README.md"}}]}}'
printf '%s\n' '{"type":"control_request","request_id":"approval-1","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"README.md"}}}'
IFS= read -r approval
printf '%s' "$approval" > __APPROVAL__
printf '%s\n' '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"contents","is_error":false}]}}'
printf '%s\n' '{"type":"result","session_id":"claude-session-real","is_error":false,"usage":{"input_tokens":10,"cache_read_input_tokens":2,"output_tokens":3}}'
cat >/dev/null
"#
        .replace("__CWD__", &shell_quote(&observed_cwd))
        .replace("__CONFIG__", &shell_quote(&observed_config))
        .replace("__KEY__", &shell_quote(&observed_key))
        .replace("__APPROVAL__", &shell_quote(&observed_approval));
        write_executable(&executable, &script);

        let target = subscription_target(
            SubscriptionAgent::ClaudeCode,
            executable,
            working_directory.clone(),
            config_dir.clone(),
        );
        let mut launch = ProcessLaunch::for_session(&target, None, ProcessLocation::Local);
        launch
            .environment
            .push(("ANTHROPIC_API_KEY", "must-be-removed".to_string()));
        let mut session = SubscriptionSession::open_with_launch(target, None, launch)
            .await
            .unwrap();
        session.send_prompt("Hello").await.unwrap();

        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::SessionStarted(
                SessionIdentity::ClaudeCode("claude-session-real".to_string())
            ))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ReasoningDelta("Inspecting".to_string()))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TextDelta("Hello".to_string()))
        );
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ToolStarted { ref id, .. }) if id == "tool-1"
        ));
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ApprovalRequested { ref request_id, .. })
                if request_id == "approval-1"
        ));
        session
            .respond_to_approval("approval-1", ApprovalDecision::Allow)
            .await
            .unwrap();
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ToolOutput { ref id, is_error: false, .. })
                if id == "tool-1"
        ));
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::Usage(_))
        ));
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TurnCompleted {
                session: SessionIdentity::ClaudeCode("claude-session-real".to_string())
            })
        );
        session.end().await.unwrap();

        assert_eq!(
            std::fs::read_to_string(observed_cwd).unwrap(),
            working_directory.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(observed_config).unwrap(),
            config_dir.to_str().unwrap()
        );
        assert_eq!(std::fs::read_to_string(observed_key).unwrap(), "unset");
        let approval = std::fs::read_to_string(observed_approval).unwrap();
        assert!(approval.contains("\"request_id\":\"approval-1\""));
        assert!(approval.contains("\"behavior\":\"allow\""));
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn local_fake_claude_resumes_real_session_and_cancels_its_process() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-claude-resume");
        let observed_args = directory.path().join("claude-args");
        let observed_interrupt = directory.path().join("claude-interrupt");
        let script = r#"
printf '%s\n' "$@" > __ARGS__
IFS= read -r initialize
printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"zaplex-session-initialize","response":{"account":{"accountUuid":"claude-account-42"},"models":[{"value":"default"}]}}}'
IFS= read -r prompt
printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-session-resume"}'
IFS= read -r interrupt
printf '%s' "$interrupt" > __INTERRUPT__
printf '%s\n' '{"type":"result","session_id":"claude-session-resume","is_error":true,"result":"Interrupted"}'
cat >/dev/null
"#
        .replace("__ARGS__", &shell_quote(&observed_args))
        .replace("__INTERRUPT__", &shell_quote(&observed_interrupt));
        write_executable(&executable, &script);
        let working_directory = directory.path().join("cwd");
        let config_dir = directory.path().join("account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let target = subscription_target(
            SubscriptionAgent::ClaudeCode,
            executable,
            working_directory,
            config_dir,
        );
        let resume = SessionIdentity::ClaudeCode("claude-session-resume".to_string());
        let mut session =
            SubscriptionSession::open(target, Some(resume.clone()), ProcessLocation::Local)
                .await
                .unwrap();
        assert_eq!(session.identity(), Some(&resume));
        session.send_prompt("Continue").await.unwrap();
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::SessionStarted(resume.clone()))
        );
        session.cancel().await.unwrap();
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::Error { ref session, .. }) if session.as_ref() == Some(&resume)
        ));
        session.end().await.unwrap();

        let args = std::fs::read_to_string(observed_args).unwrap();
        assert!(args
            .lines()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|args| args == ["--resume", "claude-session-resume"]));
        let interrupt = std::fs::read_to_string(observed_interrupt).unwrap();
        assert!(interrupt.contains("\"subtype\":\"interrupt\""));
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn fake_claude_crash_reports_a_recoverable_error_with_real_session_id() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-claude-crash");
        write_executable(
            &executable,
            r#"
IFS= read -r initialize
printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"zaplex-session-initialize","response":{"account":{"accountUuid":"claude-account-42"},"models":[{"value":"default"}]}}}'
IFS= read -r prompt
printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-session-before-crash"}'
exit 17
"#,
        );
        let working_directory = directory.path().join("cwd");
        let config_dir = directory.path().join("account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let target = subscription_target(
            SubscriptionAgent::ClaudeCode,
            executable,
            working_directory,
            config_dir,
        );
        let mut session = SubscriptionSession::open(target, None, ProcessLocation::Local)
            .await
            .unwrap();
        session.send_prompt("Crash").await.unwrap();
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::SessionStarted(
                SessionIdentity::ClaudeCode("claude-session-before-crash".to_string())
            ))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::Error {
                message: "Claude Code exited unexpectedly".to_string(),
                recoverable: true,
                session: Some(SessionIdentity::ClaudeCode(
                    "claude-session-before-crash".to_string()
                )),
            })
        );
        session.end().await.unwrap();
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn remote_fake_claude_uses_remote_host_cwd_account_and_scrubbed_environment() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let working_directory = directory.path().join("remote claude cwd");
        let config_dir = directory.path().join("remote claude account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let executable = directory.path().join("fake-remote-claude");
        let fake_ssh = directory.path().join("fake-ssh");
        let observed_ssh = directory.path().join("claude-ssh-args");
        let observed_cwd = directory.path().join("remote-claude-cwd");
        let observed_config = directory.path().join("remote-claude-config");
        let observed_key = directory.path().join("remote-claude-api-key");
        let observed_token = directory.path().join("remote-claude-auth-token");
        let claude_script = r#"
printf '%s' "$PWD" > __CWD__
printf '%s' "${CLAUDE_CONFIG_DIR-unset}" > __CONFIG__
printf '%s' "${ANTHROPIC_API_KEY-unset}" > __KEY__
printf '%s' "${ANTHROPIC_AUTH_TOKEN-unset}" > __TOKEN__
IFS= read -r initialize
printf '%s\n' '{"type":"control_response","response":{"subtype":"success","request_id":"zaplex-session-initialize","response":{"account":{"accountUuid":"claude-account-42"},"models":[{"value":"default"}]}}}'
IFS= read -r prompt
printf '%s\n' '{"type":"system","subtype":"init","session_id":"claude-remote-session"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"Remote reply"}]}}'
printf '%s\n' '{"type":"result","session_id":"claude-remote-session","is_error":false}'
cat >/dev/null
"#
        .replace("__CWD__", &shell_quote(&observed_cwd))
        .replace("__CONFIG__", &shell_quote(&observed_config))
        .replace("__KEY__", &shell_quote(&observed_key))
        .replace("__TOKEN__", &shell_quote(&observed_token));
        write_executable(&executable, &claude_script);
        let ssh_script = r#"
printf '%s\n' "$@" > __SSH_ARGS__
export ANTHROPIC_API_KEY=ambient-remote-key
export ANTHROPIC_AUTH_TOKEN=ambient-remote-token
last=''
for argument in "$@"; do
    last="$argument"
done
exec /bin/sh -c "$last"
"#
        .replace("__SSH_ARGS__", &shell_quote(&observed_ssh));
        write_executable(&fake_ssh, &ssh_script);

        let target = subscription_target(
            SubscriptionAgent::ClaudeCode,
            executable,
            working_directory.clone(),
            config_dir.clone(),
        );
        let mut session = SubscriptionSession::open(
            target,
            None,
            ProcessLocation::Remote {
                ssh_argv: vec![
                    fake_ssh.to_string_lossy().into_owned(),
                    "--".to_string(),
                    "claude.remote.test".to_string(),
                ],
            },
        )
        .await
        .unwrap();
        session.send_prompt("Run remotely").await.unwrap();

        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::SessionStarted(
                SessionIdentity::ClaudeCode("claude-remote-session".to_string())
            ))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TextDelta("Remote reply".to_string()))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TurnCompleted {
                session: SessionIdentity::ClaudeCode("claude-remote-session".to_string())
            })
        );
        session.end().await.unwrap();

        assert_eq!(
            std::fs::read_to_string(observed_cwd).unwrap(),
            working_directory.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(observed_config).unwrap(),
            config_dir.to_str().unwrap()
        );
        assert_eq!(std::fs::read_to_string(observed_key).unwrap(), "unset");
        assert_eq!(std::fs::read_to_string(observed_token).unwrap(), "unset");
        let ssh_args = std::fs::read_to_string(observed_ssh).unwrap();
        assert!(ssh_args.contains("BatchMode=yes"));
        assert!(ssh_args.contains("claude.remote.test"));
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn remote_fake_codex_resumes_approves_and_cancels_the_addressed_turn() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let working_directory = directory.path().join("remote cwd");
        let config_dir = directory.path().join("remote codex home");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let app_server = directory.path().join("fake-codex");
        let fake_ssh = directory.path().join("fake-ssh");
        let observed_ssh = directory.path().join("ssh-args");
        let observed_cwd = directory.path().join("codex-cwd");
        let observed_config = directory.path().join("codex-config");
        let observed_key = directory.path().join("codex-api-key");
        let observed_thread = directory.path().join("codex-thread");
        let observed_approval = directory.path().join("codex-approval");
        let observed_interrupt = directory.path().join("codex-interrupt");
        let app_server_script = r#"
printf '%s' "$PWD" > __CWD__
printf '%s' "${CODEX_HOME-unset}" > __CONFIG__
printf '%s' "${OPENAI_API_KEY-unset}" > __KEY__
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r initialized
IFS= read -r account
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"developer@example.com","planType":"plus"},"requiresOpenaiAuth":true}}'
IFS= read -r rate_limits
printf '%s\n' '{"id":3,"result":{"accountId":"codex-account-42","rateLimits":{}}}'
IFS= read -r models
printf '%s\n' '{"id":4,"result":{"data":[{"id":"gpt-test","displayName":"Test","isDefault":true,"supportedReasoningEfforts":[]}],"nextCursor":null}}'
IFS= read -r thread
printf '%s' "$thread" > __THREAD__
printf '%s\n' '{"id":5,"result":{"thread":{"id":"codex-thread-real"}}}'
IFS= read -r turn
printf '%s\n' '{"id":6,"result":{"turn":{"id":"codex-turn-real"}}}'
printf '%s\n' '{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"codex-thread-real","turnId":"codex-turn-real","delta":"Inspecting"}}'
printf '%s\n' '{"method":"item/started","params":{"threadId":"codex-thread-real","turnId":"codex-turn-real","item":{"id":"item-1","type":"commandExecution","command":"pwd"}}}'
printf '%s\n' '{"method":"turn/diff/updated","params":{"threadId":"codex-thread-real","turnId":"codex-turn-real","diff":"@@ test @@"}}'
printf '%s\n' '{"id":91,"method":"item/commandExecution/requestApproval","params":{"threadId":"codex-thread-real","turnId":"codex-turn-real","itemId":"item-1","command":"pwd","reason":"verify cwd"}}'
IFS= read -r approval
printf '%s' "$approval" > __APPROVAL__
printf '%s\n' '{"method":"item/completed","params":{"threadId":"codex-thread-real","turnId":"codex-turn-real","item":{"id":"item-1","type":"commandExecution","aggregatedOutput":"ok","status":"completed"}}}'
IFS= read -r interrupt
printf '%s' "$interrupt" > __INTERRUPT__
printf '%s\n' '{"method":"turn/completed","params":{"threadId":"codex-thread-real","turn":{"id":"codex-turn-real","status":"completed"}}}'
cat >/dev/null
"#
        .replace("__CWD__", &shell_quote(&observed_cwd))
        .replace("__CONFIG__", &shell_quote(&observed_config))
        .replace("__KEY__", &shell_quote(&observed_key))
        .replace("__THREAD__", &shell_quote(&observed_thread))
        .replace("__APPROVAL__", &shell_quote(&observed_approval))
        .replace("__INTERRUPT__", &shell_quote(&observed_interrupt));
        write_executable(&app_server, &app_server_script);
        let ssh_script = r#"
printf '%s\n' "$@" > __SSH_ARGS__
export OPENAI_API_KEY=ambient-remote-key
last=''
for argument in "$@"; do
    last="$argument"
done
exec /bin/sh -c "$last"
"#
        .replace("__SSH_ARGS__", &shell_quote(&observed_ssh));
        write_executable(&fake_ssh, &ssh_script);

        let target = subscription_target(
            SubscriptionAgent::Codex,
            app_server,
            working_directory.clone(),
            config_dir.clone(),
        );
        let resume = SessionIdentity::Codex("codex-thread-real".to_string());
        let mut session = SubscriptionSession::open(
            target,
            Some(resume.clone()),
            ProcessLocation::Remote {
                ssh_argv: vec![
                    fake_ssh.to_string_lossy().into_owned(),
                    "--".to_string(),
                    "remote.test".to_string(),
                ],
            },
        )
        .await
        .unwrap();
        assert_eq!(session.identity(), Some(&resume));
        session.send_prompt("Continue").await.unwrap();
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ReasoningDelta("Inspecting".to_string()))
        );
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ToolStarted { ref id, .. }) if id == "item-1"
        ));
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::Diff("@@ test @@".to_string()))
        );
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ApprovalRequested { ref request_id, .. }) if request_id == "91"
        ));
        session
            .respond_to_approval("91", ApprovalDecision::AllowForSession)
            .await
            .unwrap();
        assert!(matches!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::ToolOutput { ref id, is_error: false, .. }) if id == "item-1"
        ));
        session.cancel().await.unwrap();
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TurnCompleted {
                session: SessionIdentity::Codex("codex-thread-real".to_string())
            })
        );
        session.end().await.unwrap();

        assert_eq!(
            std::fs::read_to_string(observed_cwd).unwrap(),
            working_directory.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(observed_config).unwrap(),
            config_dir.to_str().unwrap()
        );
        assert_eq!(std::fs::read_to_string(observed_key).unwrap(), "unset");
        let ssh_args = std::fs::read_to_string(observed_ssh).unwrap();
        assert!(ssh_args.contains("BatchMode=yes"));
        assert!(ssh_args.contains("remote.test"));
        let thread = std::fs::read_to_string(observed_thread).unwrap();
        assert!(thread.contains("\"method\":\"thread/resume\""));
        assert!(thread.contains("\"threadId\":\"codex-thread-real\""));
        let approval = std::fs::read_to_string(observed_approval).unwrap();
        assert!(approval.contains("\"id\":91"));
        assert!(approval.contains("acceptForSession"));
        let interrupt = std::fs::read_to_string(observed_interrupt).unwrap();
        assert!(interrupt.contains("\"method\":\"turn/interrupt\""));
        assert!(interrupt.contains("\"threadId\":\"codex-thread-real\""));
        assert!(interrupt.contains("\"turnId\":\"codex-turn-real\""));
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn local_fake_codex_uses_local_cwd_account_and_scrubbed_environment() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let working_directory = directory.path().join("local codex cwd");
        let config_dir = directory.path().join("local codex account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let executable = directory.path().join("fake-local-codex");
        let observed_cwd = directory.path().join("local-codex-cwd");
        let observed_config = directory.path().join("local-codex-config");
        let observed_key = directory.path().join("local-codex-api-key");
        let observed_thread = directory.path().join("local-codex-thread");
        let app_server_script = r#"
printf '%s' "$PWD" > __CWD__
printf '%s' "${CODEX_HOME-unset}" > __CONFIG__
printf '%s' "${OPENAI_API_KEY-unset}" > __KEY__
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r initialized
IFS= read -r account
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"developer@example.com","planType":"plus"}}}'
IFS= read -r rate_limits
printf '%s\n' '{"id":3,"result":{"accountId":"codex-account-42","rateLimits":{}}}'
IFS= read -r models
printf '%s\n' '{"id":4,"result":{"data":[{"id":"gpt-test","displayName":"Test","isDefault":true,"supportedReasoningEfforts":[]}],"nextCursor":null}}'
IFS= read -r thread
printf '%s' "$thread" > __THREAD__
printf '%s\n' '{"id":5,"result":{"thread":{"id":"codex-local-thread"}}}'
IFS= read -r turn
printf '%s\n' '{"id":6,"result":{"turn":{"id":"codex-local-turn"}}}'
printf '%s\n' '{"method":"item/agentMessage/delta","params":{"threadId":"codex-local-thread","turnId":"codex-local-turn","delta":"Local reply"}}'
printf '%s\n' '{"method":"thread/tokenUsage/updated","params":{"threadId":"codex-local-thread","turnId":"codex-local-turn","tokenUsage":{"last":{"inputTokens":8,"cachedInputTokens":3,"outputTokens":5}}}}'
printf '%s\n' '{"method":"turn/completed","params":{"threadId":"codex-local-thread","turn":{"id":"codex-local-turn","status":"completed"}}}'
cat >/dev/null
"#
        .replace("__CWD__", &shell_quote(&observed_cwd))
        .replace("__CONFIG__", &shell_quote(&observed_config))
        .replace("__KEY__", &shell_quote(&observed_key))
        .replace("__THREAD__", &shell_quote(&observed_thread));
        write_executable(&executable, &app_server_script);

        let target = subscription_target(
            SubscriptionAgent::Codex,
            executable,
            working_directory.clone(),
            config_dir.clone(),
        );
        let mut launch = ProcessLaunch::for_session(&target, None, ProcessLocation::Local);
        launch
            .environment
            .push(("OPENAI_API_KEY", "must-be-removed".to_string()));
        let mut session = SubscriptionSession::open_with_launch(target, None, launch)
            .await
            .unwrap();
        assert_eq!(
            session.identity(),
            Some(&SessionIdentity::Codex("codex-local-thread".to_string()))
        );
        session.send_prompt("Run locally").await.unwrap();

        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TextDelta("Local reply".to_string()))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::Usage(Usage {
                input_tokens: 8,
                cached_input_tokens: 3,
                output_tokens: 5,
            }))
        );
        assert_eq!(
            session.next_event().await.unwrap(),
            Some(SubscriptionEvent::TurnCompleted {
                session: SessionIdentity::Codex("codex-local-thread".to_string())
            })
        );
        session.end().await.unwrap();

        assert_eq!(
            std::fs::read_to_string(observed_cwd).unwrap(),
            working_directory.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(observed_config).unwrap(),
            config_dir.to_str().unwrap()
        );
        assert_eq!(std::fs::read_to_string(observed_key).unwrap(), "unset");
        let thread = std::fs::read_to_string(observed_thread).unwrap();
        assert!(thread.contains("\"method\":\"thread/start\""));
        assert!(thread.contains(working_directory.to_str().unwrap()));
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn fake_codex_account_mismatch_fails_closed_before_thread_start() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex-mismatch");
        write_executable(
            &executable,
            r#"
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r initialized
IFS= read -r account
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"developer@example.com","planType":"plus"}}}'
IFS= read -r rate_limits
printf '%s\n' '{"id":3,"result":{"accountId":"different-account","rateLimits":{}}}'
IFS= read -r models
printf '%s\n' '{"id":4,"result":{"data":[{"id":"gpt-test"}],"nextCursor":null}}'
cat >/dev/null
"#,
        );
        let working_directory = directory.path().join("cwd");
        let config_dir = directory.path().join("account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let target = subscription_target(
            SubscriptionAgent::Codex,
            executable,
            working_directory,
            config_dir,
        );

        let error = SubscriptionSession::open(target, None, ProcessLocation::Local)
            .await
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "Codex authenticated account does not match selected account developer@example.com; sign in to that account in the selected CLI profile and retry"
        );
    });
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn fake_old_codex_app_server_requires_an_upgrade_without_fallback() {
    futures_lite::future::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-old-codex");
        write_executable(
            &executable,
            r#"
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{}}'
IFS= read -r initialized
IFS= read -r account
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"developer@example.com","planType":"plus"}}}'
IFS= read -r rate_limits
printf '%s\n' '{"id":3,"error":{"code":-32601,"message":"method not found"}}'
cat >/dev/null
"#,
        );
        let working_directory = directory.path().join("cwd");
        let config_dir = directory.path().join("account");
        std::fs::create_dir_all(&working_directory).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let target = subscription_target(
            SubscriptionAgent::Codex,
            executable,
            working_directory,
            config_dir,
        );

        let error = SubscriptionSession::open(target, None, ProcessLocation::Local)
            .await
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "Codex account verification (upgrade Codex if unavailable) failed: method not found"
        );
    });
}
