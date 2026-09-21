use super::super::DAEMON_SESSION_ID_BASE;
use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::Mutex;
use warp_ssh_manager::{AuthType, ResolvedSshConnection, SecretKind, SshServerInfo};

fn server(auth: AuthType) -> SshServerInfo {
    let mut s = SshServerInfo::new_default("node-1".to_string());
    s.host = "example.com".to_string();
    s.username = "me".to_string();
    s.port = 22;
    s.auth_type = auth;
    s
}

#[test]
fn headless_capable_only_for_key_auth() {
    assert!(is_headless_capable(&server(AuthType::Key)));
    assert!(!is_headless_capable(&server(AuthType::Password)));
    // OneKey is resolved to Key/Password upstream (resolve_server_auth); the
    // bare OneKey marker is not headless-capable on its own.
    assert!(!is_headless_capable(&server(AuthType::OneKey)));
}

#[test]
fn agent_route_preflight_resolves_onekey_key() {
    let resolved = |auth_type, secret_kind| ResolvedSshConnection {
        server: server(auth_type),
        secret_lookup_id: "cred-1".to_string(),
        secret_kind,
    };

    assert!(is_agent_route_headless_capable(&resolved(
        AuthType::Key,
        SecretKind::Passphrase,
    )));
    assert!(!is_agent_route_headless_capable(&resolved(
        AuthType::Password,
        SecretKind::OneKeyPassword,
    )));
}

#[test]
fn multiplexer_inventory_requires_explicit_daemon_capability() {
    let old_daemon = InitializeResponse::default();
    assert!(!supports_multiplexer_inventory(&old_daemon));

    let capable_daemon = InitializeResponse {
        features: vec![FEATURE_MULTIPLEXER_INVENTORY_V1.to_string()],
        ..Default::default()
    };
    assert!(supports_multiplexer_inventory(&capable_daemon));
}

#[test]
fn agent_inventory_requires_explicit_daemon_capability() {
    let old_daemon = InitializeResponse::default();
    assert!(!supports_agent_inventory(&old_daemon));

    let capable_daemon = InitializeResponse {
        features: vec![FEATURE_AGENT_INVENTORY.to_string()],
        ..Default::default()
    };
    assert!(supports_agent_inventory(&capable_daemon));
}

#[test]
fn historical_recovery_accepts_only_older_semantic_versions() {
    assert!(is_older_release_daemon(Some("v1.0.29"), "v1.0.28"));
    assert!(!is_older_release_daemon(Some("v1.0.29"), "v1.0.29"));
    assert!(!is_older_release_daemon(Some("v1.0.29"), "v1.0.30"));
    assert!(!is_older_release_daemon(None, "v1.0.28"));
    assert!(!is_older_release_daemon(Some("v1.0.29"), "development"));
    assert!(!is_older_release_daemon(Some("v1.1"), "v1.1.rcbad"));
    assert!(!is_older_release_daemon(Some("v1.1"), "v1.1.dev"));
    assert!(is_older_release_daemon(Some("v1.1"), "v1.0.29"));
    assert!(is_older_release_daemon(Some("v1.1.rc1"), "v1.1.dev2"));
    assert!(!is_older_release_daemon(Some("v1.1.dev2"), "v1.1.rc1"));
}

#[test]
fn release_tag_forms_compare_by_numeric_prerelease_precedence() {
    for (newer, older) in [
        ("v1.1", "v1.1.rc10"),
        ("v1.1.rc10", "v1.1.rc2"),
        ("v1.1.dev10", "v1.1.dev2"),
        ("v1.1.0-rc10", "v1.1.0-rc2"),
        ("v1.1.0-beta10", "v1.1.0-beta2"),
        ("v1.1.0-alpha10", "v1.1.0-alpha2"),
        ("v1.1.0-dev10", "v1.1.dev2"),
    ] {
        assert!(
            is_older_release_daemon(Some(newer), older),
            "{older} < {newer}"
        );
        assert!(
            !is_older_release_daemon(Some(older), newer),
            "{newer} > {older}"
        );
    }
    for (left, right) in [
        ("v1.1", "v1.1.0"),
        ("v1.1.rc2", "v1.1.0-rc2"),
        ("v1.1.dev2", "v1.1.0-dev2"),
        ("v1.1.0+build2", "v1.1.0+build1"),
    ] {
        assert!(!is_older_release_daemon(Some(left), right));
        assert!(!is_older_release_daemon(Some(right), left));
    }
}

#[test]
fn agent_inventory_replaces_shell_name_with_agent_identity() {
    let mut daemon = remote_server::proto::SessionList {
        sessions: vec![remote_server::proto::SessionInfo {
            session_id: "pty-1".to_string(),
            title: "bash".to_string(),
            generation: 7,
            ..Default::default()
        }],
        ..Default::default()
    };
    let agents = remote_server::proto::AgentSessionList {
        sessions: vec![remote_server::proto::AgentSessionInfo {
            name: "release checks".to_string(),
            provider: "codex".to_string(),
            pty_session_id: "pty-1".to_string(),
            pty_session_generation: 7,
            pty_foreground: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    enrich_daemon_session_titles(&mut daemon, &agents);

    assert_eq!(daemon.sessions[0].title, "Codex · release checks");
}

#[test]
fn unmatched_agent_inventory_never_preserves_a_shell_title() {
    let mut daemon = SessionList {
        sessions: vec![remote_server::proto::SessionInfo {
            session_id: "pty-1".to_string(),
            title: "tcsh".to_string(),
            cwd: "/srv/project".to_string(),
            generation: 7,
            ..Default::default()
        }],
        ..Default::default()
    };

    enrich_daemon_session_titles(&mut daemon, &AgentSessionList::default());

    assert_eq!(daemon.sessions[0].title, "project");
}

#[test]
fn legacy_daemon_without_agent_inventory_uses_neutral_session_identity() {
    let mut daemon = SessionList {
        sessions: vec![remote_server::proto::SessionInfo {
            session_id: "pty-legacy".to_string(),
            title: "bash".to_string(),
            cwd: "/srv/project".to_string(),
            managed: Some(remote_server::proto::ManagedSessionInfo {
                provider: "codex".to_string(),
                project_root: "/srv/project".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    neutralize_legacy_session_titles(&mut daemon);

    assert!(daemon.sessions[0].title.is_empty());
    assert_eq!(daemon.sessions[0].cwd, "/srv/project");
    assert!(daemon.sessions[0].managed.is_none());
}

#[test]
fn merged_daemon_inventory_preserves_exact_route_per_session() {
    let old_route =
        DaemonRuntimeRoute::new("server-v1.0.28.sock".to_string(), "v1.0.28".to_string()).unwrap();
    let current_route =
        DaemonRuntimeRoute::new("server-v1.0.29.sock".to_string(), "v1.0.29".to_string()).unwrap();
    let sessions = || SessionList {
        sessions: vec![remote_server::proto::SessionInfo {
            session_id: "same-id".to_string(),
            generation: 7,
            ..Default::default()
        }],
        host_ring_cap_bytes: 1024,
        ..Default::default()
    };
    let mut inventory = HostSessionInventory::default();

    merge_daemon_inventory(
        &mut inventory,
        sessions(),
        MultiplexerSessionList::default(),
        Some(current_route.clone()),
        0,
    );
    merge_daemon_inventory(
        &mut inventory,
        sessions(),
        MultiplexerSessionList::default(),
        Some(old_route.clone()),
        1,
    );

    assert_eq!(inventory.sessions.len(), 2);
    assert_eq!(inventory.sessions[0].route.as_ref(), Some(&current_route));
    assert_eq!(inventory.sessions[1].route.as_ref(), Some(&old_route));
    assert_eq!(inventory.daemon.host_ring_cap_bytes, 0);
}

#[test]
fn control_socket_path_is_stable_and_per_host() {
    let a1 = control_socket_path(&server(AuthType::Key));
    let a2 = control_socket_path(&server(AuthType::Key));
    assert_eq!(a1, a2, "same host → same socket path (run-to-run stable)");

    let mut other = server(AuthType::Key);
    other.host = "other.example.com".to_string();
    assert_ne!(
        a1,
        control_socket_path(&other),
        "different host → different socket"
    );
    assert!(a1.to_string_lossy().contains(".ssh/zaplex-daemon-"));
}

#[test]
fn daemon_session_ids_are_unique_and_in_top_half() {
    let a = alloc_daemon_session_id();
    let b = alloc_daemon_session_id();
    assert_ne!(a, b, "each allocation is unique");
    assert!(
        a.as_u64() >= DAEMON_SESSION_ID_BASE,
        "top-half id (no collision with shell ids)"
    );
    assert!(b.as_u64() >= DAEMON_SESSION_ID_BASE);
}

#[test]
fn headless_control_master_rejects_invalid_endpoints_before_spawn() {
    for (host, port) in [
        ("", 22),
        ("   ", 22),
        ("-oProxyCommand=malicious", 22),
        ("host with spaces", 22),
        ("example.com", 0),
    ] {
        let mut invalid = server(AuthType::Key);
        invalid.host = host.to_string();
        invalid.port = port;
        assert!(
            control_master_args(&invalid, Path::new("/tmp/zaplex-test-control"), None).is_err(),
            "headless ControlMaster must reject {host:?}:{port}"
        );
    }
}

#[test]
fn headless_control_master_uses_l2_host_key_and_argument_policy() {
    let args = control_master_args(
        &server(AuthType::Key),
        Path::new("/tmp/zaplex-test-control"),
        None,
    )
    .expect("valid endpoint should produce arguments");
    let destination_delimiter = args
        .iter()
        .position(|arg| arg == "--")
        .expect("destination must be separated from options");

    assert_eq!(
        &args[destination_delimiter + 1..],
        &["me@example.com".to_string()]
    );
    assert!(args[..destination_delimiter]
        .iter()
        .any(|arg| arg == "StrictHostKeyChecking=ask"));
    assert!(!args
        .iter()
        .any(|arg| arg == "StrictHostKeyChecking=accept-new" || arg == "StrictHostKeyChecking=no"));
    assert!(args[..destination_delimiter]
        .iter()
        .any(|arg| arg == "ControlMaster=auto"));
}

#[test]
fn control_master_failure_message_has_one_context_prefix() {
    let error = anyhow!("ControlMaster setup failed: Connection closed by remote port 22");

    assert_eq!(
        format_control_master_setup_error(error),
        "ControlMaster setup failed: Connection closed by remote port 22"
    );
    assert_eq!(
        format_control_master_setup_error(anyhow!(HOST_KEY_CHANGED)),
        HOST_KEY_CHANGED
    );
}

#[cfg(unix)]
struct HostKeyCommandFactory {
    script: PathBuf,
    log_path: PathBuf,
}

#[cfg(unix)]
impl WorkspaceCommandFactory for HostKeyCommandFactory {
    fn async_command(&self, program: &str) -> command::r#async::Command {
        let mut command = command::r#async::Command::new(&self.script);
        command
            .arg(program)
            .env("ZAPLEX_HOST_KEY_TEST_LOG", &self.log_path);
        command
    }

    fn blocking_command(&self, program: &str) -> command::blocking::Command {
        let mut command = command::blocking::Command::new(&self.script);
        command
            .arg(program)
            .env("ZAPLEX_HOST_KEY_TEST_LOG", &self.log_path);
        command
    }
}

#[cfg(unix)]
#[test]
fn unknown_host_key_requires_confirmation_before_control_master() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("fake-ssh-tools");
    let log_path = directory.path().join("argv.log");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(
        b"#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$ZAPLEX_HOST_KEY_TEST_LOG\"\ncase \"$1\" in\n  ssh-keygen)\n    printf '256 SHA256:confirmed host (ED25519)\\n' ;;\n  ssh)\n    shift\n    for arg in \"$@\"; do\n      case \"$arg\" in\n        UserKnownHostsFile=*) path=${arg#UserKnownHostsFile=} ;;\n        StrictHostKeyChecking=*) strict=${arg#StrictHostKeyChecking=} ;;\n        BatchMode=*) batch=${arg#BatchMode=} ;;\n        ControlPath=*) control=${arg#ControlPath=} ;;\n      esac\n    done\n    if [ \"$1\" = -O ]; then\n      test -f \"$control\"\n      exit $?\n    fi\n    if [ \"$strict\" = yes ]; then\n      if [ -n \"$path\" ] && grep -q 'ssh-ed25519 AAAA' \"$path\"; then\n        : > \"$control\"\n        exit 0\n      fi\n      printf 'Host key verification failed.\\nED25519 key fingerprint is SHA256:confirmed.\\n' >&2\n      exit 255\n    fi\n    if [ \"$strict\" = ask ] && [ \"$batch\" = no ]; then\n      printf 'example.com ssh-ed25519 AAAA\\n' > \"$path\"\n      exit 255\n    fi\n    exit 2 ;;\n  *) exit 2 ;;\nesac\n",
    )
    .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = HostKeyCommandFactory { script, log_path };
    let managed_known_hosts = directory.path().join("known_hosts");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let outcome = runtime
        .block_on(preflight_control_master_host_key_with_factory(
            &server(AuthType::Key),
            &managed_known_hosts,
            &factory,
        ))
        .unwrap();
    let HostKeyPreflight::ConfirmationRequired(host_key) = outcome else {
        panic!("unknown host key must require confirmation");
    };
    crate::i18n::init(Some("en"));
    assert_eq!(
        require_daemon_inventory_ready(DaemonPreflight::HostKeyConfirmationRequired(
            host_key.clone(),
        )),
        Err(crate::t!(
            "workspace-left-panel-ssh-manager-sessions-confirm-host-key",
            host = host_key.host.clone(),
            port = host_key.port,
            fingerprint = host_key.fingerprint.clone()
        ))
    );
    let invocations = std::fs::read_to_string(&factory.log_path).unwrap();
    assert!(
        !invocations
            .lines()
            .any(|line| line.split_whitespace().any(|arg| arg == "-f")),
        "ControlMaster started before confirmation: {invocations}"
    );

    confirm_host_key_at(&server(AuthType::Key), &host_key, &managed_known_hosts).unwrap();
    assert_eq!(
        std::fs::metadata(&managed_known_hosts)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let args = control_master_args(
        &server(AuthType::Key),
        Path::new("/tmp/zaplex-test-control"),
        Some(&managed_known_hosts),
    )
    .unwrap();
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(args
        .iter()
        .any(|arg| { arg == &format!("UserKnownHostsFile={}", managed_known_hosts.display()) }));
    let control_socket = directory.path().join("control");
    runtime
        .block_on(ensure_control_master_with_factory(
            &server(AuthType::Key),
            &control_socket,
            Some(&managed_known_hosts),
            &factory,
        ))
        .unwrap();
    assert!(control_socket.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn concurrent_ensure_control_master_spawns_once() {
    struct RecordingCommandFactory {
        script: PathBuf,
        programs: Mutex<Vec<String>>,
    }

    impl WorkspaceCommandFactory for RecordingCommandFactory {
        fn async_command(&self, program: &str) -> command::r#async::Command {
            self.programs.lock().unwrap().push(program.to_string());
            command::r#async::Command::new(&self.script)
        }

        fn blocking_command(&self, program: &str) -> command::blocking::Command {
            panic!("unexpected blocking command: {program}")
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("fake-ssh");
    let starts = directory.path().join("starts");
    let release = directory.path().join("release");
    let live = directory.path().join("live");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = '-O' ]; then\n  test -f '{}'\n  exit $?\nfi\nprintf 'start\\n' >> '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\ntouch '{}'\n",
            live.display(),
            starts.display(),
            release.display(),
            live.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    let factory = RecordingCommandFactory {
        script,
        programs: Mutex::new(Vec::new()),
    };
    let socket_path = directory.path().join("control.sock");
    let test_server = server(AuthType::Key);

    let first = ensure_control_master_with_factory(&test_server, &socket_path, None, &factory);
    let second = ensure_control_master_with_factory(&test_server, &socket_path, None, &factory);
    let release_first = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !starts.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(starts.exists(), "first ControlMaster setup did not start");
        std::fs::write(&release, "release").unwrap();
    };

    let (first, second, ()) = tokio::join!(first, second, release_first);
    first.unwrap();
    second.unwrap();
    assert_eq!(std::fs::read_to_string(starts).unwrap(), "start\n");
    assert!(factory
        .programs
        .lock()
        .unwrap()
        .iter()
        .all(|program| program == "ssh"));
}

#[tokio::test]
async fn inventory_deadline_preserves_completed_results() {
    let mut inventory = HostSessionInventory::default();
    inventory.daemon.host_ring_cap_bytes = 42;
    let result = inventory_with_timeout(async { Ok(inventory) }, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(result.daemon.host_ring_cap_bytes, 42);

    let error = inventory_with_timeout(
        async { Err("daemon authentication failed".to_string()) },
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "daemon authentication failed");
}

#[tokio::test]
async fn inventory_deadline_drops_pending_scan_resources_and_releases_lock() {
    crate::i18n::init(Some("en"));
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let resource = DropFlag(dropped.clone());
    let lock = AsyncMutex::new(());
    let guard = lock.lock().await;
    let scan = async move {
        let _resource = resource;
        let _guard = guard;
        futures::future::pending::<std::result::Result<HostSessionInventory, String>>().await
    };

    let timeout = Duration::from_millis(10);
    let error = inventory_with_timeout(scan, timeout).await.unwrap_err();
    assert_eq!(
        error,
        crate::t!(
            "workspace-left-panel-ssh-manager-sessions-timeout",
            seconds = timeout.as_secs()
        )
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert!(lock.try_lock().is_some());
}

#[tokio::test]
async fn inventory_deadline_includes_waiting_for_control_master_lock() {
    crate::i18n::init(Some("en"));
    let lock = AsyncMutex::new(());
    let guard = lock.lock().await;
    let scan = async {
        let _guard = lock.lock().await;
        Ok(HostSessionInventory::default())
    };

    let timeout = Duration::from_millis(10);
    let error = inventory_with_timeout(scan, timeout).await.unwrap_err();
    assert_eq!(
        error,
        crate::t!(
            "workspace-left-panel-ssh-manager-sessions-timeout",
            seconds = timeout.as_secs()
        )
    );
    drop(guard);
    assert!(lock.try_lock().is_some());
}

#[test]
fn inventory_preflight_accepts_an_existing_daemon() {
    assert!(require_daemon_inventory_ready(DaemonPreflight::Ready).is_ok());
}

#[test]
fn inventory_preflight_requires_explicit_connection_before_installing() {
    crate::i18n::init(Some("en"));
    assert_eq!(
        require_daemon_inventory_ready(DaemonPreflight::NeedsInstall),
        Err(crate::t!(
            "workspace-left-panel-ssh-manager-sessions-needs-install"
        ))
    );
}

#[test]
fn subscription_agent_reuses_verified_master_without_fresh_connection_fallback() {
    let host = server(AuthType::Key);
    let args = managed_agent_ssh_args(&host).unwrap();
    let delimiter = args.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(&args[delimiter + 1..], &["me@example.com"]);
    assert!(args[..delimiter].windows(2).any(|pair| {
        pair[0] == "-S" && pair[1] == control_socket_path(&host).to_string_lossy()
    }));
    for option in [
        "ControlMaster=no",
        "ProxyCommand=false",
        "StrictHostKeyChecking=yes",
        "RequestTTY=no",
    ] {
        assert!(args[..delimiter]
            .windows(2)
            .any(|pair| pair == ["-o", option]));
    }
    assert!(!args.iter().any(|arg| arg == "StrictHostKeyChecking=ask"));
}

#[test]
fn subscription_agent_route_rejects_an_invalid_endpoint_before_spawn() {
    let mut invalid = server(AuthType::Key);
    invalid.host = "-oProxyCommand=malicious".to_string();
    assert!(managed_agent_ssh_args(&invalid).is_err());
}
