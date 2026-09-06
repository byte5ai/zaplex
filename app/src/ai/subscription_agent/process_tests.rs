use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};
#[cfg(target_os = "linux")]
use warpui::r#async::FutureExt as _;

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
            ssh_argv: vec!["ssh".to_string(), "--".to_string(), "host".to_string()],
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

#[cfg(target_os = "linux")]
#[test]
fn remote_version_probe_uses_noninteractive_ssh_options() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("ssh-probe");
    let args_file = directory.path().join("args");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '1.2.3\\n'\n",
            args_file.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let installation = target(SubscriptionAgent::ClaudeCode).installation;
    let version = futures_lite::future::block_on(query_cli_version(
        &installation,
        directory.path().to_path_buf(),
        ProcessLocation::Remote {
            ssh_argv: vec![
                executable.to_string_lossy().into_owned(),
                "--".to_string(),
                "host".to_string(),
            ],
        },
    ))
    .unwrap();
    assert_eq!(version, "1.2.3");

    let args = std::fs::read_to_string(args_file)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let delimiter = args.iter().position(|arg| arg == "--").unwrap();
    assert!(args[..delimiter]
        .windows(2)
        .any(|args| args == ["-o", "BatchMode=yes"]));
    assert!(args[..delimiter]
        .windows(2)
        .any(|args| args == ["-o", "ConnectTimeout=10"]));
    assert!(args[..delimiter]
        .windows(2)
        .any(|args| args == ["-o", "ConnectionAttempts=1"]));
    assert_eq!(args.get(delimiter + 1).map(String::as_str), Some("host"));
}

#[cfg(target_os = "linux")]
#[test]
fn timed_out_version_probe_kills_child() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("version-probe");
    let pid_file = directory.path().join("pid");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
    installation.executable = executable;
    let result = futures_lite::future::block_on(
        query_cli_version(
            &installation,
            directory.path().to_path_buf(),
            ProcessLocation::Local,
        )
        .with_timeout(Duration::from_millis(50)),
    );
    assert!(result.is_err());

    let pid = std::fs::read_to_string(pid_file).unwrap();
    let process_path = std::path::PathBuf::from(format!("/proc/{}", pid.trim()));
    let deadline = Instant::now() + Duration::from_secs(1);
    while process_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(!process_path.exists(), "timed-out child was not reaped");
}
