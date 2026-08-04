use super::*;
use std::sync::atomic::AtomicBool;

fn backend() -> (InMemorySftpBackend, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    (InMemorySftpBackend::new(dir.path().to_path_buf()), dir)
}

#[test]
fn repeated_recovery_scans_report_each_operation_only_once() {
    let mut routes = HashMap::new();
    let first = replace_recovery_routes(
        &mut routes,
        [
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-a".to_string(),
                    source_preserved_after_commit: true,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/a"),
            ),
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-b".to_string(),
                    source_preserved_after_commit: false,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/b"),
            ),
        ],
    );
    assert_eq!(first, vec![PathBuf::from("/a"), PathBuf::from("/b")]);

    let second = replace_recovery_routes(
        &mut routes,
        [
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-a".to_string(),
                    source_preserved_after_commit: true,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/a"),
            ),
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-b".to_string(),
                    source_preserved_after_commit: false,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/b"),
            ),
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-b".to_string(),
                    source_preserved_after_commit: false,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/duplicate"),
            ),
            (
                RemoteRecoveryOperation {
                    operation_id: "operation-c".to_string(),
                    source_preserved_after_commit: true,
                    action: RemoteRecoveryAction::Acknowledge,
                },
                PathBuf::from("/c"),
            ),
        ],
    );
    assert_eq!(second, vec![PathBuf::from("/c")]);
    assert_eq!(
        routes.get(Path::new("/b")),
        Some(&vec![RemoteRecoveryOperation {
            operation_id: "operation-b".to_string(),
            source_preserved_after_commit: false,
            action: RemoteRecoveryAction::Acknowledge,
        }])
    );
    assert!(!routes.contains_key(Path::new("/duplicate")));

    let replay = RemoteRecoveryOperation {
        operation_id: "operation-replay".to_string(),
        source_preserved_after_commit: false,
        action: RemoteRecoveryAction::Delete(SafeFileDelete {
            path: "/replay".to_string(),
            expected: None,
            expected_sha256: None,
        }),
    };
    routes.insert(PathBuf::from("/replay"), vec![replay.clone()]);
    assert!(replace_recovery_routes(&mut routes, []).is_empty());
    assert_eq!(
        routes.get(Path::new("/replay")),
        Some(&vec![replay.clone()])
    );

    let server_record = RemoteRecoveryOperation {
        operation_id: "operation-replay".to_string(),
        source_preserved_after_commit: false,
        action: RemoteRecoveryAction::Acknowledge,
    };
    assert!(replace_recovery_routes(
        &mut routes,
        [(server_record.clone(), PathBuf::from("/server"))]
    )
    .is_empty());
    assert!(!routes.contains_key(Path::new("/replay")));
    assert_eq!(
        routes.get(Path::new("/server")),
        Some(&vec![RemoteRecoveryOperation {
            source_preserved_after_commit: server_record.source_preserved_after_commit,
            ..replay
        }])
    );
}

#[test]
fn local_self_copy_is_rejected_without_truncation() {
    let (backend, root) = backend();
    fs::write(root.path().join("same.txt"), b"original").unwrap();

    let error = backend
        .copy_file(Path::new("/same.txt"), Path::new("/same.txt"))
        .expect_err("self-copy must be rejected");

    assert!(error.to_string().contains("same path"));
    assert_eq!(fs::read(root.path().join("same.txt")).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn recursive_copy_rejects_destination_inside_source_before_mutation() {
    let (backend, root) = backend();
    fs::create_dir(root.path().join("source")).unwrap();
    fs::write(root.path().join("source/file.txt"), b"original").unwrap();

    let error = backend
        .copy_dir_recursive(Path::new("/source"), Path::new("/source/child"))
        .expect_err("copying into a descendant must be rejected");

    assert!(error.to_string().contains("own descendant"));
    assert!(!root.path().join("source/child").exists());
    assert_eq!(
        fs::read(root.path().join("source/file.txt")).unwrap(),
        b"original"
    );
}

/// Renaming onto an existing name must refuse, not silently destroy it.
/// `fs::rename` overwrites on Unix; the remote backend has always used
/// `overwrite: false`, and this is the local path catching up (RC audit).
#[test]
fn local_rename_never_replaces_existing_destination() {
    let (be, dir) = backend();
    fs::write(dir.path().join("victim.txt"), b"PRECIOUS").unwrap();
    fs::write(dir.path().join("source.txt"), b"new").unwrap();

    let err = be
        .rename(Path::new("/source.txt"), Path::new("/victim.txt"))
        .expect_err("renaming onto an existing file must fail");
    assert!(
        matches!(err, SftpOpsError::Operation(ref m) if m.contains("already exists")),
        "expected an already-exists conflict, got {err:?}"
    );
    assert_eq!(
        fs::read(dir.path().join("victim.txt")).unwrap(),
        b"PRECIOUS",
        "the existing file must be untouched"
    );
}

/// A destination created after the initial lookup is still a conflict.
/// The commit itself must be no-replace; a check followed by plain rename
/// loses this race on Unix.
#[test]
fn local_rename_never_replaces_concurrently_created_destination() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("source.txt"), b"source").unwrap();
    let backend =
        InMemorySftpBackend::new(dir.path().to_path_buf()).with_before_rename(|destination| {
            fs::write(destination, b"CONCURRENT").unwrap();
        });

    backend
        .rename(Path::new("/source.txt"), Path::new("/destination.txt"))
        .expect_err("an atomically created destination must block rename");

    assert_eq!(
        fs::read(dir.path().join("destination.txt")).unwrap(),
        b"CONCURRENT",
        "rename must not replace the destination created in the race window"
    );
    assert!(
        dir.path().join("source.txt").exists(),
        "failed rename must preserve its source"
    );
}

/// The exchange primitive swaps unlike entry types without unlinking
/// either object.
#[test]
fn local_atomic_exchange_swaps_file_and_directory_without_data_loss() {
    let (be, dir) = backend();
    fs::write(dir.path().join("source.txt"), b"replacement").unwrap();
    fs::create_dir(dir.path().join("destination")).unwrap();
    fs::write(dir.path().join("destination/precious.txt"), b"PRECIOUS").unwrap();

    be.replace(Path::new("/source.txt"), Path::new("/destination"))
        .expect("the platform exchange primitive must swap both entries");

    assert_eq!(
        fs::read(dir.path().join("destination")).unwrap(),
        b"replacement",
        "the source file must move to the destination path"
    );
    assert_eq!(
        fs::read(dir.path().join("source.txt/precious.txt")).unwrap(),
        b"PRECIOUS",
        "the displaced directory must remain intact at the source path"
    );
}

/// A cancelled copy must leave the destination exactly as it was — the old
/// code wrote straight into it, so a cancel truncated a good file.
#[test]
fn local_copy_failure_keeps_existing_destination_intact() {
    let (be, dir) = backend();
    let dest = dir.path().join("dest.bin");
    fs::write(&dest, b"ORIGINAL-CONTENT").unwrap();
    // Source big enough that the cancel is observed inside the copy loop.
    fs::write(dir.path().join("src.bin"), vec![b'x'; 512 * 1024]).unwrap();

    let cancel = AtomicBool::new(true);
    let err = be
        .upload_file(
            &dir.path().join("src.bin"),
            Path::new("/dest.bin"),
            None,
            Some(&cancel),
        )
        .expect_err("a pre-cancelled copy must fail");
    assert!(matches!(err, SftpOpsError::Cancelled), "got {err:?}");
    assert_eq!(
        fs::read(&dest).unwrap(),
        b"ORIGINAL-CONTENT",
        "a cancelled copy must not touch the destination"
    );
    // And it must not litter: no partial left behind.
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("zaplex_partial"))
        .collect();
    assert!(leftovers.is_empty(), "partial left behind: {leftovers:?}");
}

/// A successful copy still replaces the destination completely.
#[test]
fn successful_copy_replaces_the_destination() {
    let (be, dir) = backend();
    fs::write(dir.path().join("dest.bin"), b"OLD").unwrap();
    fs::write(dir.path().join("src.bin"), b"NEW-CONTENT").unwrap();

    be.upload_file(
        &dir.path().join("src.bin"),
        Path::new("/dest.bin"),
        None,
        None,
    )
    .expect("copy should succeed");
    assert_eq!(
        fs::read(dir.path().join("dest.bin")).unwrap(),
        b"NEW-CONTENT"
    );
}

/// Independent transfers targeting the same final name must never share
/// their in-progress file. Otherwise either transfer can truncate, rename
/// or clean up the other transfer's bytes.
#[test]
fn concurrent_copies_reserve_distinct_temporary_paths() {
    let destination = Path::new("/tmp/destination.bin");

    let first = temp_sibling(destination).unwrap();
    let second = temp_sibling(destination).unwrap();

    assert_ne!(
        first, second,
        "each transfer needs an exclusive temporary sibling"
    );
}

#[test]
fn local_no_replace_copy_preserves_existing_destination() {
    let (backend, dir) = backend();
    let source = dir.path().join("source.bin");
    fs::write(&source, b"NEW").unwrap();
    fs::write(dir.path().join("destination.bin"), b"EXISTING").unwrap();

    backend
        .upload_file_no_replace(&source, Path::new("/destination.bin"), None, None)
        .expect_err("an unconfirmed copy must not replace its destination");

    assert_eq!(
        fs::read(dir.path().join("destination.bin")).unwrap(),
        b"EXISTING"
    );
}

#[test]
fn published_no_replace_copy_stays_successful_when_temp_cleanup_fails() {
    let dir = tempfile::tempdir().unwrap();
    let temp = dir.path().join("partial.bin");
    let destination = dir.path().join("destination.bin");
    fs::write(&temp, b"COMPLETE").unwrap();

    publish_copy_without_replacement_with_cleanup(&temp, &destination, |_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected cleanup failure",
        ))
    })
    .expect("published destination must not be reported as a failed copy");

    assert_eq!(fs::read(destination).unwrap(), b"COMPLETE");
    assert!(temp.exists(), "the injected cleanup failure must be real");
}

/// Exclusive temp creation must reject symlink collisions instead of
/// truncating the symlink target through `File::create`.
#[cfg(unix)]
#[test]
fn local_copy_temp_creation_never_follows_existing_symlink() {
    use std::os::unix::fs::symlink;

    let (backend, dir) = backend();
    let source = dir.path().join("source.bin");
    let destination = dir.path().join("destination.bin");
    let victim = dir.path().join("victim.bin");
    fs::write(&source, b"NEW").unwrap();
    fs::write(&victim, b"PRECIOUS").unwrap();

    let first_sequence = COPY_TEMP_COUNTER.load(Ordering::Relaxed);
    for sequence in first_sequence..first_sequence + 32 {
        let candidate = destination.with_file_name(format!(
            ".destination.bin.zaplex_partial-{}-{sequence}",
            std::process::id()
        ));
        symlink(&victim, candidate).unwrap();
    }

    backend
        .upload_file(&source, Path::new("/destination.bin"), None, None)
        .expect("copy should retry after colliding with symlinks");

    assert_eq!(
        fs::read(&victim).unwrap(),
        b"PRECIOUS",
        "temporary-file creation must never follow a symlink"
    );
    assert_eq!(fs::read(destination).unwrap(), b"NEW");
}

#[cfg(unix)]
#[test]
fn stat_follows_a_symlink_to_a_file() {
    use std::os::unix::fs::symlink;

    let (be, dir) = backend();
    fs::write(dir.path().join("target.txt"), b"target").unwrap();
    symlink("target.txt", dir.path().join("link.txt")).unwrap();

    let entry = be
        .stat(Path::new("/link.txt"))
        .expect("a valid file symlink should resolve");

    assert_eq!(entry.file_type, FileEntryType::File);
    assert_eq!(entry.size, 6);
}

#[cfg(unix)]
#[test]
fn stat_follows_a_symlink_to_a_directory() {
    use std::os::unix::fs::symlink;

    let (be, dir) = backend();
    fs::create_dir(dir.path().join("target-dir")).unwrap();
    symlink("target-dir", dir.path().join("link-dir")).unwrap();

    let entry = be
        .stat(Path::new("/link-dir"))
        .expect("a valid directory symlink should resolve");

    assert_eq!(entry.file_type, FileEntryType::Directory);
}

#[cfg(unix)]
#[test]
fn stat_rejects_a_broken_symlink() {
    use std::os::unix::fs::symlink;

    let (be, dir) = backend();
    symlink("missing-target", dir.path().join("broken-link")).unwrap();

    assert_eq!(
        be.lstat(Path::new("/broken-link")).unwrap().file_type,
        FileEntryType::Symlink,
        "lstat must still see a broken link for overwrite/delete checks"
    );
    be.stat(Path::new("/broken-link"))
        .expect_err("a broken symlink must not masquerade as a usable entry");
}

#[cfg(unix)]
#[test]
fn deleting_a_directory_symlink_never_deletes_its_target() {
    use std::os::unix::fs::symlink;

    let (be, dir) = backend();
    fs::create_dir(dir.path().join("target-dir")).unwrap();
    fs::write(dir.path().join("target-dir/keep.txt"), b"keep").unwrap();
    symlink("target-dir", dir.path().join("link-dir")).unwrap();

    let listed = be.list_dir(Path::new("/")).unwrap();
    let link = listed
        .iter()
        .find(|entry| entry.name == "link-dir")
        .expect("the symlink should be listed");
    assert_eq!(
        link.file_type,
        FileEntryType::Symlink,
        "directory listings must retain lstat semantics for destructive decisions"
    );

    be.delete_file(Path::new("/link-dir"))
        .expect("deleting the link should succeed");

    assert_eq!(dir.path().join("link-dir").exists(), false);
    assert_eq!(
        fs::read(dir.path().join("target-dir/keep.txt")).unwrap(),
        b"keep",
        "deleting the symlink must not recurse into its directory target"
    );
}
