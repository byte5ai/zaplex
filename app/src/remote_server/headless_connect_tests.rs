use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::sync::Mutex;
use warp_ssh_manager::{AuthType, SshServerInfo};

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
            control_master_args(&invalid, Path::new("/tmp/zaplex-test-control")).is_err(),
            "headless ControlMaster must reject {host:?}:{port}"
        );
    }
}

#[test]
fn headless_control_master_uses_l2_host_key_and_argument_policy() {
    let args = control_master_args(
        &server(AuthType::Key),
        Path::new("/tmp/zaplex-test-control"),
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

    let first = ensure_control_master_with_factory(&test_server, &socket_path, &factory);
    let second = ensure_control_master_with_factory(&test_server, &socket_path, &factory);
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
