use super::*;
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
        b"#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$ZAPLEX_HOST_KEY_TEST_LOG\"\ncase \"$1\" in\n  ssh-keygen)\n    printf '256 SHA256:confirmed host (ED25519)\\n' ;;\n  ssh)\n    shift\n    for arg in \"$@\"; do\n      case \"$arg\" in\n        UserKnownHostsFile=*) path=${arg#UserKnownHostsFile=} ;;\n        StrictHostKeyChecking=*) strict=${arg#StrictHostKeyChecking=} ;;\n        BatchMode=*) batch=${arg#BatchMode=} ;;\n        ControlPath=*) control=${arg#ControlPath=} ;;\n      esac\n    done\n    if [ \"$strict\" = yes ]; then\n      if [ -n \"$path\" ] && grep -q 'ssh-ed25519 AAAA' \"$path\"; then\n        : > \"$control\"\n        exit 0\n      fi\n      printf 'Host key verification failed.\\nED25519 key fingerprint is SHA256:confirmed.\\n' >&2\n      exit 255\n    fi\n    if [ \"$strict\" = ask ] && [ \"$batch\" = no ]; then\n      printf 'example.com ssh-ed25519 AAAA\\n' > \"$path\"\n      exit 255\n    fi\n    exit 2 ;;\n  *) exit 2 ;;\nesac\n",
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
