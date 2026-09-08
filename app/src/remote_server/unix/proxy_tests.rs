use super::{select_daemon, DaemonSelection};

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
