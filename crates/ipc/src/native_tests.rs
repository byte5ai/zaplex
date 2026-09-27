use super::server::ConnectionListenerImpl;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

#[test]
fn filesystem_socket_is_owner_only_and_removed_on_drop() {
    let parent = test_directory(0o700);
    let path = parent.join("server.sock");
    let listener = ConnectionListenerImpl::new(path.display().to_string().into()).unwrap();

    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(listener);
    assert!(!path.exists());
    std::fs::remove_dir(parent).unwrap();
}

#[test]
fn filesystem_socket_rejects_a_public_parent_directory() {
    let parent = test_directory(0o755);
    let path = parent.join("server.sock");

    assert!(ConnectionListenerImpl::new(path.display().to_string().into()).is_err());
    assert!(!path.exists());
    std::fs::remove_dir(parent).unwrap();
}

#[test]
fn filesystem_socket_creates_and_removes_a_private_parent() {
    let root = test_directory(0o700);
    let parent = root.join("private");
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
    std::fs::remove_dir(root).unwrap();
}

fn test_directory(mode: u32) -> std::path::PathBuf {
    // Keep the complete socket path within macOS's short sockaddr_un limit.
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let directory = std::env::temp_dir().join(format!("zpi-{}", &nonce[..12]));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(mode)).unwrap();
    directory
}
