use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};
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
                provider_account_id: None,
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
        CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES
    );
    assert_eq!(
        launch.environment,
        vec![
            (
                CLAUDE_PROVIDER_MANAGED_BY_HOST.0,
                CLAUDE_PROVIDER_MANAGED_BY_HOST.1.to_string(),
            ),
            ("CLAUDE_CONFIG_DIR", "/accounts/with space".to_string()),
        ]
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
fn claude_discovery_scrubs_all_provider_environment_variables() {
    let installation = target(SubscriptionAgent::ClaudeCode).installation;
    let launch =
        ProcessLaunch::for_discovery(&installation, "/workspace".into(), ProcessLocation::Local);

    assert_eq!(
        launch.unset_environment,
        CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES
    );
    assert_eq!(
        launch.environment,
        vec![
            (
                CLAUDE_PROVIDER_MANAGED_BY_HOST.0,
                CLAUDE_PROVIDER_MANAGED_BY_HOST.1.to_string(),
            ),
            ("CLAUDE_CONFIG_DIR", "/accounts/with space".to_string()),
        ]
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
            environment_path: None,
            ssh_argv: vec!["ssh".to_string(), "--".to_string(), "host".to_string()],
        },
    );
    let command = launch.remote_command();

    assert_eq!(command.starts_with("cd -- '/workspace/with space'"), true);
    for name in CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES {
        assert!(command.contains(&format!("-u {name}")));
    }
    assert!(command.contains("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=zaplex"));
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
            environment_path: None,
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
#[serial_test::serial]
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

#[test]
fn remote_executable_probe_accepts_only_a_framed_absolute_path() {
    let (executable, path) =
        parse_remote_executable(b"/opt/cli with spaces/claude\0/opt/node/bin:/usr/bin\0").unwrap();
    assert_eq!(executable, PathBuf::from("/opt/cli with spaces/claude"));
    assert_eq!(path, "/opt/node/bin:/usr/bin");
    for invalid in [
        b"claude\0/usr/bin\0".as_slice(),
        b"banner\n/opt/claude\0/usr/bin\0".as_slice(),
        b"/opt/claude\0\0".as_slice(),
        b"/opt/claude\0/usr/bin\0extra".as_slice(),
        b"/opt/claude\0/usr/bin\nextra\0".as_slice(),
    ] {
        assert!(parse_remote_executable(invalid).is_err());
    }
}

#[test]
fn process_errors_classify_diagnostics_without_disclosing_raw_stderr() {
    let stderr = b"ANTHROPIC_AUTH_TOKEN=secret-value\nHost key verification failed.\n";
    let message = process_diagnostic(stderr).unwrap();
    assert!(message.contains("SSH host verification failed"));
    assert!(!message.contains("secret-value"));
    assert!(process_diagnostic(b"arbitrary secret-value").is_none());
    assert!(
        process_diagnostic(b"Control socket connect failed: No such file")
            .unwrap()
            .contains("reconnect the host")
    );
}

#[cfg(target_os = "linux")]
fn executable_script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn remote_cli_resolution_preserves_login_path_for_cli_and_interpreter() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("login-only bin");
    std::fs::create_dir(&bin).unwrap();
    let cli = bin.join("claude");
    std::fs::write(&cli, "#!/usr/bin/env zaplex-test-node\nprintf '4.5.6\\n'\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    executable_script(&bin.join("zaplex-test-node"), "exec /bin/sh \"$@\"");

    let login_shell = directory.path().join("login shell");
    executable_script(
        &login_shell,
        &format!(
            "test \"$1\" = -lic || exit 2\ntest \"$BYOBU_DISABLE\" = 1 || exit 3\ntest \"$LC_BYOBU\" = 0 || exit 4\nprintf 'shell startup banner\\n'\nexport PATH={}:/usr/bin:/bin\nexec /bin/sh -c \"$2\"",
            shell_words::quote(&bin.to_string_lossy())
        ),
    );
    let ssh = directory.path().join("ssh");
    executable_script(
        &ssh,
        &format!(
            "export PATH=/usr/bin:/bin\nexport BYOBU_DISABLE=0 LC_BYOBU=1\nexport SHELL={}\nfor argument in \"$@\"; do last=\"$argument\"; done\nexec /bin/sh -c \"$last\"",
            shell_words::quote(&login_shell.to_string_lossy())
        ),
    );
    let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
    installation.executable = "claude".into();
    let mut location = ProcessLocation::Remote {
        ssh_argv: vec![
            ssh.to_string_lossy().into_owned(),
            "--".into(),
            "host".into(),
        ],
        environment_path: None,
    };
    futures_lite::future::block_on(async {
        resolve_remote_executable(&mut installation, &mut location)
            .await
            .unwrap();
        assert_eq!(installation.executable, cli);
        assert_eq!(
            query_cli_version(&installation, directory.path().to_path_buf(), location)
                .await
                .unwrap(),
            "4.5.6"
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn protocol_stderr_is_drained_without_blocking_and_keeps_only_bounded_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("noisy-cli");
    executable_script(
        &executable,
        r#"i=0
while [ "$i" -lt 4096 ]; do
    printf '%s\n' 'startup diagnostic text which must not block the structured stdout stream' >&2
    i=$((i + 1))
done
printf '%s\n' '{"ready":true}'"#,
    );
    let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
    installation.executable = executable;
    let launch = ProcessLaunch::for_discovery(
        &installation,
        directory.path().to_path_buf(),
        ProcessLocation::Local,
    );
    futures_lite::future::block_on(async {
        let mut process = JsonLineProcess::spawn(&launch).unwrap();
        let frame = process
            .receive()
            .with_timeout(Duration::from_secs(3))
            .await
            .expect("stderr pipe must not block stdout")
            .unwrap();
        assert_eq!(frame, Some(serde_json::json!({"ready": true})));
        assert_eq!(process.diagnostics.len(), MAX_DIAGNOSTIC_BYTES);
        process.terminate().await.unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn failed_protocol_start_surfaces_safe_actionable_ssh_error() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("failed-cli");
    executable_script(
        &executable,
        "printf 'private-token-secret\\nHost key verification failed.\\n' >&2\nexit 255",
    );
    let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
    installation.executable = executable;
    let launch = ProcessLaunch::for_discovery(
        &installation,
        directory.path().to_path_buf(),
        ProcessLocation::Local,
    );
    let message = futures_lite::future::block_on(async {
        JsonLineProcess::spawn(&launch)
            .unwrap()
            .receive()
            .await
            .unwrap_err()
            .to_string()
    });
    assert!(message.contains("SSH host verification failed"));
    assert!(!message.contains("private-token-secret"));
}

#[cfg(target_os = "linux")]
#[test]
fn cli_version_probe_rejects_unbounded_shell_output() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("noisy-version");
    executable_script(&executable, "exec yes unexpected-shell-output");
    let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
    installation.executable = executable;
    let message = futures_lite::future::block_on(query_cli_version(
        &installation,
        directory.path().to_path_buf(),
        ProcessLocation::Local,
    ))
    .unwrap_err();
    assert!(format!("{message:#}").contains("excessive output"));
}

#[cfg(target_os = "linux")]
#[test]
fn final_cli_diagnostic_survives_a_large_login_banner_with_bounded_storage() {
    for (diagnostic, expected) in [
        ("ZAPLEX_CLI_NOT_FOUND", "The CLI is not executable"),
        (
            "env: node: No such file or directory",
            "The CLI runtime is unavailable",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("noisy-failing-cli");
        executable_script(
            &executable,
            &format!(
                r#"printf '%s\n' 'private-token-secret at start' >&2
i=0
while [ "$i" -lt 1024 ]; do
    printf '%s\n' 'login banner output exceeding the diagnostic storage limit' >&2
    i=$((i + 1))
done
printf '%s\n' {diagnostic} >&2
exit 127"#,
                diagnostic = shell_words::quote(diagnostic)
            ),
        );
        let mut installation = target(SubscriptionAgent::ClaudeCode).installation;
        installation.executable = executable;
        let launch = ProcessLaunch::for_discovery(
            &installation,
            directory.path().to_path_buf(),
            ProcessLocation::Local,
        );
        futures_lite::future::block_on(async {
            let mut process = JsonLineProcess::spawn(&launch).unwrap();
            let message = process
                .receive()
                .with_timeout(Duration::from_secs(3))
                .await
                .expect("a large stderr banner must not block startup")
                .unwrap_err()
                .to_string();
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("private-token-secret"));
            assert_eq!(process.diagnostics.len(), MAX_DIAGNOSTIC_BYTES);
            assert!(process
                .diagnostics
                .starts_with(b"private-token-secret at start\n"));
            assert!(process
                .diagnostics
                .ends_with(format!("{diagnostic}\n").as_bytes()));
            process.terminate().await.unwrap();
        });
    }
}
