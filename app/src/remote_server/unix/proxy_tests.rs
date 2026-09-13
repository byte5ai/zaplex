use super::{connect_existing_daemon, runtime_filenames_in_dir, select_daemon, DaemonSelection};

#[test]
fn live_unrelated_pid_without_socket_is_not_a_live_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let pid_path = directory.path().join("daemon.pid");
    let socket_path = directory.path().join("daemon.sock");
    std::fs::write(pid_path, std::process::id().to_string()).unwrap();

    assert!(matches!(
        select_daemon(&socket_path),
        Ok(DaemonSelection::StartDaemon)
    ));
}

#[test]
fn existing_daemon_connection_is_reused() {
    let directory = tempfile::tempdir().unwrap();
    let socket_path = directory.path().join("daemon.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();

    let selection = select_daemon(&socket_path).unwrap();
    let DaemonSelection::Connected(stream) = selection else {
        panic!("listening daemon socket was not reused");
    };
    let (accepted, _) = listener.accept().unwrap();

    assert!(stream.peer_addr().is_ok());
    assert!(accepted.peer_addr().is_ok());
}

#[test]
fn explicit_historical_runtime_missing_does_not_create_a_socket() {
    let directory = tempfile::tempdir().unwrap();
    let socket_path = directory.path().join("server-v0.9.sock");

    assert!(connect_existing_daemon(&socket_path).is_err());
    assert!(!socket_path.exists());
}

#[test]
fn runtime_inventory_contains_only_daemon_sockets_with_current_first() {
    use std::os::unix::net::UnixListener;

    let directory = tempfile::tempdir().unwrap();
    let _legacy = UnixListener::bind(directory.path().join("server-v0.9.sock")).unwrap();
    let current_name = remote_server::setup::daemon_runtime_filename("sock");
    let _current = UnixListener::bind(directory.path().join(&current_name)).unwrap();
    let _unversioned = UnixListener::bind(directory.path().join("server.sock")).unwrap();
    std::fs::write(directory.path().join("server-v0.8.sock"), "not a socket").unwrap();
    let _unrelated = UnixListener::bind(directory.path().join("unrelated.sock")).unwrap();

    let runtimes = runtime_filenames_in_dir(directory.path()).unwrap();

    assert_eq!(runtimes.first(), Some(&current_name));
    assert!(runtimes.contains(&"server-v0.9.sock".to_string()));
    assert!(runtimes.contains(&"server.sock".to_string()));
    assert!(!runtimes.contains(&"server-v0.8.sock".to_string()));
    assert!(!runtimes.contains(&"unrelated.sock".to_string()));
}
