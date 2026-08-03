use super::server::ConnectionListenerImpl;
use std::os::unix::fs::PermissionsExt as _;

#[test]
fn filesystem_socket_is_owner_only_and_removed_on_drop() {
    let path = std::env::temp_dir().join(format!(
        "zaplex-ipc-permissions-{}-{}.sock",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let listener = ConnectionListenerImpl::new(path.display().to_string().into()).unwrap();

    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(listener);
    assert!(!path.exists());
}

#[test]
fn filesystem_socket_rejects_a_public_parent_directory() {
    let path = std::env::temp_dir().join(format!(
        "zaplex-ipc-public-parent-{}-{}.sock",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));

    assert!(ConnectionListenerImpl::new(path.display().to_string().into()).is_err());
    assert!(!path.exists());
}

#[test]
fn filesystem_socket_creates_and_removes_a_private_parent() {
    let parent = std::env::temp_dir().join(format!(
        "zaplex-ipc-private-parent-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let path = parent.join("server.sock");

    let listener = ConnectionListenerImpl::new(path.display().to_string().into()).unwrap();

    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(listener);
    assert!(!path.exists());
    assert!(!parent.exists());
}
