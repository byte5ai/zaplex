use std::fs::{self, File};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::*;

fn path_string(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

fn call(
    server: &mut SafeFileServer,
    owner: ConnectionId,
    operation_id: &str,
    operation: safe_file_request::Operation,
) -> safe_file_response::Result {
    server
        .handle(
            owner,
            SafeFileRequest {
                operation_id: operation_id.to_string(),
                operation: Some(operation),
            },
        )
        .result
        .unwrap()
}

fn open_regular(server: &mut SafeFileServer, owner: ConnectionId, path: &Path) -> SafeFileOpened {
    match call(
        server,
        owner,
        "",
        safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
            path: path_string(path),
            expected_kind: SafeFileEntryKind::Regular as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected opened response, got {other:?}"),
    }
}

fn create_regular(
    server: &mut SafeFileServer,
    owner: ConnectionId,
    operation_id: &str,
    path: &Path,
) -> SafeFileOpened {
    match call(
        server,
        owner,
        operation_id,
        safe_file_request::Operation::CreateExclusive(SafeFileCreateExclusive {
            path: path_string(path),
            kind: SafeFileEntryKind::Regular as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected created response, got {other:?}"),
    }
}

#[test]
fn nofollow_open_rejects_symlink_and_descriptor_survives_path_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let moved = directory.path().join("moved.bin");
    let link = directory.path().join("link.bin");
    fs::write(&source, b"original").unwrap();
    symlink(&source, &link).unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    assert!(matches!(
        call(
            &mut server,
            owner,
            "",
            safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
                path: path_string(&link),
                expected_kind: SafeFileEntryKind::Regular as i32,
            }),
        ),
        safe_file_response::Result::Error(_)
    ));

    let opened = open_regular(&mut server, owner, &source);
    fs::rename(&source, &moved).unwrap();
    fs::write(&source, b"replacement").unwrap();
    let read = call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::ReadHandle(SafeFileReadHandle {
            handle_id: opened.handle_id.clone(),
            max_bytes: 64,
        }),
    );
    let safe_file_response::Result::Read(read) = read else {
        panic!("expected read response");
    };
    assert_eq!(read.bytes, b"original");

    let inspected = call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::InspectHandle(SafeFileInspectHandle {
            handle_id: opened.handle_id,
            path: path_string(&moved),
        }),
    );
    let safe_file_response::Result::Inspected(inspected) = inspected else {
        panic!("expected inspected response");
    };
    assert!(inspected.matches_path);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn identity_bound_symlink_delete_removes_only_the_link() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let target = directory.path().join("target-dir");
    let link = directory.path().join("link-dir");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), b"keep").unwrap();
    symlink(&target, &link).unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = match call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
            path: path_string(&link),
            expected_kind: SafeFileEntryKind::Symlink as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected symlink handle, got {other:?}"),
    };
    let result = call(
        &mut server,
        owner,
        "delete-symlink",
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&link),
            expected: opened.identity,
            expected_sha256: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Mutation(_)));
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"keep");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn identity_bound_symlink_delete_preserves_a_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let original_target = directory.path().join("original-target");
    let replacement_target = directory.path().join("replacement-target");
    let link = directory.path().join("link");
    fs::create_dir(&original_target).unwrap();
    fs::create_dir(&replacement_target).unwrap();
    symlink(&original_target, &link).unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = match call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
            path: path_string(&link),
            expected_kind: SafeFileEntryKind::Symlink as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected symlink handle, got {other:?}"),
    };
    fs::remove_file(&link).unwrap();
    symlink(&replacement_target, &link).unwrap();

    let result = call(
        &mut server,
        owner,
        "delete-replaced-symlink",
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&link),
            expected: opened.identity,
            expected_sha256: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read_link(&link).unwrap(), replacement_target);
    assert!(original_target.is_dir());
    assert!(replacement_target.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn private_delete_unlink_detects_a_final_symlink_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let target = directory.path().join("target");
    let replacement_target = directory.path().join("replacement-target");
    let link = directory.path().join("link");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&replacement_target).unwrap();
    symlink(&target, &link).unwrap();
    let observed = Arc::new(Mutex::new(None));

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal.clone());
    let opened = match call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
            path: path_string(&link),
            expected_kind: SafeFileEntryKind::Symlink as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected symlink handle, got {other:?}"),
    };
    server.before_private_delete_unlink = Some(Box::new({
        let observed = observed.clone();
        let replacement_target = replacement_target.clone();
        move |private| {
            let retained = private.with_extension("retained");
            fs::rename(private, &retained).unwrap();
            symlink(&replacement_target, private).unwrap();
            *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
        }
    }));

    let result = call(
        &mut server,
        owner,
        "delete-final-race",
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&link),
            expected: opened.identity,
            expected_sha256: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    let (replacement, retained) = observed.lock().unwrap().clone().unwrap();
    assert!(!replacement.exists());
    assert_eq!(fs::read_link(retained).unwrap(), target);
    assert!(replacement_target.is_dir());

    drop(server);
    let recovered = SafeFileServer::new_for_test(journal);
    let recoveries = recovered.list_recoveries().unwrap().recoveries;
    assert!(recoveries
        .iter()
        .any(|recovery| recovery.operation_id == "delete-final-race"));
}

#[cfg(target_os = "linux")]
#[test]
fn delete_restores_a_replacement_from_the_public_isolation_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let target = directory.path().join("target");
    let replacement_target = directory.path().join("replacement-target");
    let retained = directory.path().join("retained-link");
    let link = directory.path().join("link");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&replacement_target).unwrap();
    symlink(&target, &link).unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = match call(
        &mut server,
        owner,
        "",
        safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
            path: path_string(&link),
            expected_kind: SafeFileEntryKind::Symlink as i32,
        }),
    ) {
        safe_file_response::Result::Opened(opened) => opened,
        other => panic!("expected symlink handle, got {other:?}"),
    };
    server.before_delete_isolation = Some(Box::new({
        let link = link.clone();
        let retained = retained.clone();
        let replacement_target = replacement_target.clone();
        move |_| {
            fs::rename(&link, &retained).unwrap();
            symlink(&replacement_target, &link).unwrap();
        }
    }));

    let result = call(
        &mut server,
        owner,
        "delete-public-race",
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&link),
            expected: opened.identity,
            expected_sha256: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read_link(link).unwrap(), replacement_target);
    assert_eq!(fs::read_link(retained).unwrap(), target);
}

#[test]
fn rename_restores_a_replacement_from_the_final_mutation_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let retained = directory.path().join("retained.bin");
    let destination = directory.path().join("destination.bin");
    fs::write(&source, b"original").unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = open_regular(&mut server, owner, &source);
    server.before_rename_mutation = Some(Box::new({
        let source = source.clone();
        let retained = retained.clone();
        move |_, _| {
            fs::rename(&source, &retained).unwrap();
            fs::write(&source, b"replacement").unwrap();
        }
    }));

    let result = call(
        &mut server,
        owner,
        "rename-final-race",
        safe_file_request::Operation::Rename(SafeFileRename {
            handle_id: opened.handle_id,
            old_path: path_string(&source),
            new_path: path_string(&destination),
            mode: SafeFileRenameMode::NoReplace as i32,
            expected_target: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read(source).unwrap(), b"replacement");
    assert_eq!(fs::read(retained).unwrap(), b"original");
    assert!(!destination.exists());
}

#[test]
fn no_replace_rejects_a_target_created_at_the_mutation_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let staging = directory.path().join("staging.bin");
    let destination = directory.path().join("destination.bin");
    fs::write(&staging, b"uploaded").unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = open_regular(&mut server, owner, &staging);
    server.before_rename_mutation = Some(Box::new({
        let destination = destination.clone();
        move |_, _| fs::write(&destination, b"racing target").unwrap()
    }));

    let result = call(
        &mut server,
        owner,
        "rename-target-race",
        safe_file_request::Operation::Rename(SafeFileRename {
            handle_id: opened.handle_id,
            old_path: path_string(&staging),
            new_path: path_string(&destination),
            mode: SafeFileRenameMode::NoReplace as i32,
            expected_target: None,
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read(staging).unwrap(), b"uploaded");
    assert_eq!(fs::read(destination).unwrap(), b"racing target");
}

#[test]
fn exchange_restores_a_target_replacement_from_the_mutation_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let target = directory.path().join("target.bin");
    let retained_target = directory.path().join("retained-target.bin");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"target").unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = open_regular(&mut server, owner, &source);
    let expected_target = identity_for_path(&target).unwrap();
    server.before_rename_mutation = Some(Box::new({
        let target = target.clone();
        let retained_target = retained_target.clone();
        move |_, _| {
            fs::rename(&target, &retained_target).unwrap();
            fs::write(&target, b"replacement").unwrap();
        }
    }));

    let result = call(
        &mut server,
        owner,
        "exchange-final-race",
        safe_file_request::Operation::Rename(SafeFileRename {
            handle_id: opened.handle_id,
            old_path: path_string(&source),
            new_path: path_string(&target),
            mode: SafeFileRenameMode::Exchange as i32,
            expected_target: Some(expected_target),
        }),
    );

    assert!(matches!(result, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(target).unwrap(), b"replacement");
    assert_eq!(fs::read(retained_target).unwrap(), b"target");
}

#[test]
fn recovery_rolls_back_a_boundary_replacement_after_a_post_syscall_crash() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let retained = directory.path().join("retained.bin");
    let destination = directory.path().join("destination.bin");
    fs::write(&source, b"original").unwrap();

    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    let opened = open_regular(&mut server, owner, &source);
    server.before_rename_mutation = Some(Box::new({
        let source = source.clone();
        let retained = retained.clone();
        move |_, _| {
            fs::rename(&source, &retained).unwrap();
            fs::write(&source, b"replacement").unwrap();
        }
    }));
    server.after_rename_mutation = Some(Box::new(|_, _| {
        panic!("simulated crash after the rename syscall")
    }));
    let request = SafeFileRename {
        handle_id: opened.handle_id,
        old_path: path_string(&source),
        new_path: path_string(&destination),
        mode: SafeFileRenameMode::NoReplace as i32,
        expected_target: None,
    };

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        call(
            &mut server,
            owner,
            "rename-post-syscall-crash",
            safe_file_request::Operation::Rename(request.clone()),
        )
    }));
    assert!(crashed.is_err());
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"replacement");

    server.before_rename_mutation = None;
    server.after_rename_mutation = None;
    let recovered = call(
        &mut server,
        owner,
        "rename-post-syscall-crash",
        safe_file_request::Operation::Rename(request),
    );

    assert!(matches!(recovered, safe_file_response::Result::Error(_)));
    assert_eq!(fs::read(source).unwrap(), b"replacement");
    assert_eq!(fs::read(retained).unwrap(), b"original");
    assert!(!destination.exists());
}

#[test]
fn abandoned_created_artifact_is_reaped_but_a_replacement_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let abandoned = directory.path().join("abandoned.bin");
    let replaced = directory.path().join("replaced.bin");
    let displaced = directory.path().join("displaced.bin");
    let owner = ConnectionId::new_v4();

    let mut server = SafeFileServer::new_for_test(journal.clone());
    create_regular(&mut server, owner, "create-abandoned", &abandoned);
    drop(server);
    assert!(abandoned.exists());
    let server = SafeFileServer::new_for_test(journal.clone());
    assert!(!abandoned.exists());
    drop(server);

    let mut server = SafeFileServer::new_for_test(journal.clone());
    create_regular(&mut server, owner, "create-replaced", &replaced);
    fs::rename(&replaced, &displaced).unwrap();
    fs::write(&replaced, b"replacement").unwrap();
    drop(server);
    let server = SafeFileServer::new_for_test(journal);
    assert_eq!(fs::read(&replaced).unwrap(), b"replacement");
    assert_eq!(fs::read(&displaced).unwrap(), b"");
    assert_eq!(server.list_recoveries().unwrap().recoveries.len(), 1);
}

#[test]
fn isolated_created_artifact_is_reaped_without_touching_its_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let target = directory.path().join("target.bin");
    let tombstone = directory.path().join(".zaplex-owned-create-isolated");
    let owner = ConnectionId::new_v4();

    let mut server = SafeFileServer::new_for_test(journal.clone());
    create_regular(&mut server, owner, "create-isolated", &target);
    fs::rename(&target, &tombstone).unwrap();
    fs::write(&target, b"replacement").unwrap();
    drop(server);

    let server = SafeFileServer::new_for_test(journal);
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    assert!(!tombstone.exists());
    assert!(server.list_recoveries().unwrap().recoveries.is_empty());
}

#[test]
fn retry_recovery_cleans_an_isolated_create_without_touching_its_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("journal");
    let target = directory.path().join("target.bin");
    let tombstone = directory.path().join(".zaplex-owned-retry-isolated");
    fs::write(&tombstone, b"owned").unwrap();
    fs::write(&target, b"replacement").unwrap();
    let expected = identity_for_path(&tombstone).unwrap();
    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal);
    server
        .journal()
        .unwrap()
        .save(&JournalRecord {
            operation_id: "retry-isolated".to_string(),
            state: JournalState::Recovery,
            operation: JournalOperation::Create {
                path: path_string(&target),
                identity: Some(JournalIdentity::from(&expected)),
            },
            recovery_paths: vec![path_string(&target), path_string(&tombstone)],
            failure: Some("injected cleanup failure".to_string()),
        })
        .unwrap();

    let result = call(
        &mut server,
        owner,
        "retry-isolated",
        safe_file_request::Operation::RetryRecovery(SafeFileRetryRecovery {}),
    );
    assert!(matches!(result, safe_file_response::Result::Mutation(_)));
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    assert!(!tombstone.exists());
    assert!(server.list_recoveries().unwrap().recoveries.is_empty());
}

#[test]
fn started_rename_is_retried_with_the_same_operation_id() {
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    fs::write(&source, b"payload").unwrap();
    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal_path);
    let opened = open_regular(&mut server, owner, &source);
    let identity = opened.identity.clone().unwrap();
    server
        .journal()
        .unwrap()
        .save(&JournalRecord {
            operation_id: "rename-after-crash".to_string(),
            state: JournalState::Started,
            operation: JournalOperation::Rename {
                old_path: path_string(&source),
                new_path: path_string(&destination),
                mode: SafeFileRenameMode::NoReplace as i32,
                source: JournalIdentity::from(&identity),
                target: None,
                boundary: Some(JournalRenameBoundary {
                    old: Some(JournalIdentity::from(&identity)),
                    new: None,
                }),
            },
            recovery_paths: vec![path_string(&source), path_string(&destination)],
            failure: None,
        })
        .unwrap();

    let result = call(
        &mut server,
        owner,
        "rename-after-crash",
        safe_file_request::Operation::Rename(SafeFileRename {
            handle_id: opened.handle_id,
            old_path: path_string(&source),
            new_path: path_string(&destination),
            mode: SafeFileRenameMode::NoReplace as i32,
            expected_target: None,
        }),
    );
    let safe_file_response::Result::Mutation(result) = result else {
        panic!("expected mutation response");
    };
    assert_eq!(
        SafeFileMutationState::try_from(result.state).unwrap(),
        SafeFileMutationState::AlreadyApplied
    );
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"payload");
}

#[test]
fn applied_rename_survives_restart_until_client_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("journal");
    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    fs::write(&source, b"payload").unwrap();
    let owner = ConnectionId::new_v4();

    let mut server = SafeFileServer::new_for_test(journal_path.clone());
    let opened = open_regular(&mut server, owner, &source);
    let operation_id = "rename-response-lost";
    let result = call(
        &mut server,
        owner,
        operation_id,
        safe_file_request::Operation::Rename(SafeFileRename {
            handle_id: opened.handle_id,
            old_path: path_string(&source),
            new_path: path_string(&destination),
            mode: SafeFileRenameMode::NoReplace as i32,
            expected_target: None,
        }),
    );
    assert!(matches!(result, safe_file_response::Result::Mutation(_)));
    drop(server);

    let mut restarted = SafeFileServer::new_for_test(journal_path);
    let recoveries = restarted.list_recoveries().unwrap().recoveries;
    assert_eq!(recoveries.len(), 1);
    assert_eq!(recoveries[0].operation_id, operation_id);
    assert_eq!(recoveries[0].paths[0], path_string(&destination));
    assert_eq!(recoveries[0].paths[1], path_string(&source));
    assert!(recoveries[0].source_preserved_after_commit);
    assert_eq!(fs::read(&destination).unwrap(), b"payload");

    let acknowledged = call(
        &mut restarted,
        ConnectionId::new_v4(),
        operation_id,
        safe_file_request::Operation::RetryRecovery(SafeFileRetryRecovery {}),
    );
    assert!(matches!(
        acknowledged,
        safe_file_response::Result::Mutation(_)
    ));
    assert!(restarted.list_recoveries().unwrap().recoveries.is_empty());
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"payload");
}

#[test]
fn exchange_refuses_a_replaced_destination_identity() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    let retained_destination = directory.path().join("retained-destination.bin");
    fs::write(&source, b"source").unwrap();
    fs::write(&destination, b"old target").unwrap();
    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(directory.path().join("journal"));
    let opened = open_regular(&mut server, owner, &source);
    let expected_target = identity_for_path(&destination).unwrap();
    fs::rename(&destination, &retained_destination).unwrap();
    fs::write(&destination, b"new target").unwrap();

    assert!(matches!(
        call(
            &mut server,
            owner,
            "exchange",
            safe_file_request::Operation::Rename(SafeFileRename {
                handle_id: opened.handle_id,
                old_path: path_string(&source),
                new_path: path_string(&destination),
                mode: SafeFileRenameMode::Exchange as i32,
                expected_target: Some(expected_target),
            }),
        ),
        safe_file_response::Result::Error(_)
    ));
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(destination).unwrap(), b"new target");
    assert_eq!(fs::read(retained_destination).unwrap(), b"old target");
}

#[test]
fn resumed_delete_removes_only_the_isolated_expected_object() {
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("journal");
    let target = directory.path().join("target.bin");
    let tombstone = directory.path().join(".zaplex-delete-resumed-delete");
    fs::write(&target, b"expected").unwrap();
    let expected = identity_for_path(&target).unwrap();
    let expected_digest = {
        let file = File::open(&target).unwrap();
        sha256_file(&file).unwrap()
    };
    fs::rename(&target, &tombstone).unwrap();
    fs::write(&target, b"replacement").unwrap();
    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(journal_path);
    server
        .journal()
        .unwrap()
        .save(&JournalRecord {
            operation_id: "resumed-delete".to_string(),
            state: JournalState::Started,
            operation: JournalOperation::Delete {
                path: path_string(&target),
                tombstone: path_string(&tombstone),
                expected: JournalIdentity::from(&expected),
                expected_sha256: Some(expected_digest),
            },
            recovery_paths: vec![path_string(&target), path_string(&tombstone)],
            failure: None,
        })
        .unwrap();

    let result = call(
        &mut server,
        owner,
        "resumed-delete",
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&target),
            expected: Some(expected),
            expected_sha256: Some(format!("{:x}", Sha256::digest(b"expected"))),
        }),
    );
    assert!(matches!(result, safe_file_response::Result::Mutation(_)));
    assert_eq!(fs::read(target).unwrap(), b"replacement");
    assert!(!tombstone.exists());
}

#[test]
fn fresh_delete_is_applied_once_and_then_reported_as_already_applied() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.bin");
    fs::write(&target, b"payload").unwrap();
    let expected = identity_for_path(&target).unwrap();
    let expected_sha256 = Some(format!("{:x}", Sha256::digest(b"payload")));
    let owner = ConnectionId::new_v4();
    let mut server = SafeFileServer::new_for_test(directory.path().join("journal"));
    let operation_id = "delete-once";
    let delete = || {
        safe_file_request::Operation::Delete(SafeFileDelete {
            path: path_string(&target),
            expected: Some(expected.clone()),
            expected_sha256: expected_sha256.clone(),
        })
    };

    let first = call(&mut server, owner, operation_id, delete());
    let safe_file_response::Result::Mutation(first) = first else {
        panic!("expected first delete mutation response");
    };
    assert_eq!(
        SafeFileMutationState::try_from(first.state).unwrap(),
        SafeFileMutationState::Applied
    );
    assert!(!target.exists());

    let repeated = call(&mut server, owner, operation_id, delete());
    let safe_file_response::Result::Mutation(repeated) = repeated else {
        panic!("expected repeated delete mutation response");
    };
    assert_eq!(
        SafeFileMutationState::try_from(repeated.state).unwrap(),
        SafeFileMutationState::AlreadyApplied
    );
}

#[test]
fn terminal_journal_records_are_capped() {
    let directory = tempfile::tempdir().unwrap();
    let journal = Journal::new_at(directory.path().join("journal")).unwrap();
    for index in 0..=MAX_TERMINAL_JOURNAL_RECORDS {
        journal
            .save(&JournalRecord {
                operation_id: format!("rejected-{index:04}"),
                state: JournalState::Rejected,
                operation: JournalOperation::Create {
                    path: format!("/unused/{index}"),
                    identity: None,
                },
                recovery_paths: Vec::new(),
                failure: Some("expected test rejection".to_string()),
            })
            .unwrap();
    }
    drop(journal.try_lock("prune-trigger").unwrap());
    let record_count = fs::read_dir(&journal.directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .count();
    assert_eq!(record_count, MAX_TERMINAL_JOURNAL_RECORDS);
}
