use std::fs;

#[cfg(unix)]
use remote_server::proto::{client_message, server_message, ServerMessage};
use remote_server::proto::{
    resolve_path_response, FileOperationError, ResolvePathNotFound, ResolvePathResponse,
};
use tokio::io::AsyncWriteExt as _;
#[cfg(unix)]
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};

use super::*;

#[cfg(unix)]
fn spawn_safe_file_test_client(
    journal_path: PathBuf,
) -> (
    Arc<RemoteServerClient>,
    warpui::r#async::executor::Background,
    tokio::task::JoinHandle<()>,
) {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let connection_id = Uuid::new_v4();
    let server_task = tokio::spawn(async move {
        let mut reader = server_read.compat();
        let mut writer = server_write.compat_write();
        let mut server =
            crate::remote_server::safe_file::SafeFileServer::new_for_test(journal_path);
        loop {
            let message = match remote_server::protocol::read_client_message(&mut reader).await {
                Ok(message) => message,
                Err(remote_server::protocol::ProtocolError::UnexpectedEof) => break,
                Err(error) => panic!("safe-file test transport failed: {error}"),
            };
            let Some(client_message::Message::SafeFile(request)) = message.message else {
                panic!("safe-file test received an unexpected request");
            };
            let response = server.handle(connection_id, request);
            remote_server::protocol::write_server_message(
                &mut writer,
                &ServerMessage {
                    request_id: message.request_id,
                    message: Some(server_message::Message::SafeFileResponse(response)),
                },
            )
            .await
            .expect("safe-file test response should be writable");
        }
        server.close_connection(connection_id);
    });

    let executor = warpui::r#async::executor::Background::default();
    let (client, _events) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    (Arc::new(client), executor, server_task)
}

#[cfg(unix)]
#[test]
fn rename_existing_sibling_is_rejected_without_mutation() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("config.yaml.bak");
        let destination = directory.path().join("config.yaml");
        fs::write(&source, b"backup").unwrap();
        fs::write(&destination, b"live").unwrap();
        let (client, executor, server_task) =
            spawn_safe_file_test_client(directory.path().join("journal"));

        let result = rename_remote_path_with_safe_file(
            client.clone(),
            source.to_string_lossy().into_owned(),
            "config.yaml".to_string(),
            SafeFileEntryKind::Regular,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(fs::read(&source).unwrap(), b"backup");
        assert_eq!(fs::read(&destination).unwrap(), b"live");
        drop(client);
        drop(executor);
        server_task.abort();
    });
}

#[cfg(unix)]
#[test]
fn rename_existing_directory_is_rejected_without_nesting_source() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("source.txt"), b"source").unwrap();
        fs::write(destination.join("destination.txt"), b"destination").unwrap();
        let (client, executor, server_task) =
            spawn_safe_file_test_client(directory.path().join("journal"));

        let result = rename_remote_path_with_safe_file(
            client.clone(),
            source.to_string_lossy().into_owned(),
            "destination".to_string(),
            SafeFileEntryKind::Directory,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(fs::read(source.join("source.txt")).unwrap(), b"source");
        assert_eq!(
            fs::read(destination.join("destination.txt")).unwrap(),
            b"destination"
        );
        assert!(!destination.join("source").exists());
        drop(client);
        drop(executor);
        server_task.abort();
    });
}

#[test]
fn incomplete_download_preserves_existing_target_and_removes_its_sidecar() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        fs::write(&destination, b"existing bytes").unwrap();
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        let sidecar = download.temporary.path().to_path_buf();
        download
            .output
            .write_all(b"partial replacement")
            .await
            .unwrap();

        drop(download);

        assert_eq!(fs::read(destination).unwrap(), b"existing bytes");
        assert!(!sidecar.exists());
    });
}

#[test]
fn commit_without_overwrite_consent_preserves_existing_target() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        fs::write(&destination, b"existing bytes").unwrap();
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        let sidecar = download.temporary.path().to_path_buf();
        download
            .output
            .write_all(b"complete replacement")
            .await
            .unwrap();

        assert!(download.commit(&destination).await.is_err());

        assert_eq!(fs::read(destination).unwrap(), b"existing bytes");
        assert!(!sidecar.exists());
    });
}

#[test]
fn explicit_overwrite_revalidates_and_replaces_existing_target() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        fs::write(&destination, b"existing bytes").unwrap();
        let identity = local_download_target_identity(&destination)
            .unwrap()
            .unwrap();
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        download
            .output
            .write_all(b"complete replacement")
            .await
            .unwrap();

        download
            .commit_overwriting(&destination, &identity)
            .await
            .unwrap();

        assert_eq!(fs::read(destination).unwrap(), b"complete replacement");
    });
}

#[test]
fn explicit_overwrite_rejects_a_changed_target() {
    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        fs::write(&destination, b"original").unwrap();
        let identity = local_download_target_identity(&destination)
            .unwrap()
            .unwrap();
        fs::remove_file(&destination).unwrap();
        fs::write(&destination, b"replacement target").unwrap();
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        download.output.write_all(b"downloaded").await.unwrap();

        assert!(download
            .commit_overwriting(&destination, &identity)
            .await
            .is_err());
        assert_eq!(fs::read(destination).unwrap(), b"replacement target");
    });
}

#[cfg(unix)]
#[test]
fn completed_download_replaces_destination_symlink_without_touching_referent() {
    use std::os::unix::fs::symlink;

    warpui::r#async::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let referent = directory.path().join("outside.bin");
        let destination = directory.path().join("download.bin");
        fs::write(&referent, b"outside bytes").unwrap();
        symlink(&referent, &destination).unwrap();
        let conflicts = scan_local_download_conflicts(&[PendingDownloadFile {
            remote_path: "/remote/download.bin".to_string(),
            local_path: destination.clone(),
            display_name: "download.bin".to_string(),
            total_bytes: 16,
        }])
        .unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].identity.kind, LocalDownloadTargetKind::Symlink);
        let identity = local_download_target_identity(&destination)
            .unwrap()
            .unwrap();
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        download
            .output
            .write_all(b"downloaded bytes")
            .await
            .unwrap();

        download
            .commit_overwriting(&destination, &identity)
            .await
            .unwrap();

        assert_eq!(fs::read(&referent).unwrap(), b"outside bytes");
        assert!(!fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(destination).unwrap(), b"downloaded bytes");
    });
}

#[cfg(unix)]
#[test]
fn download_rejects_a_symlink_destination_directory() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    let destination_directory = directory.path().join("destination");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &destination_directory).unwrap();

    let destination = destination_directory.join("download.bin");
    assert!(AtomicDownloadFile::new(&destination).is_err());
    assert!(!outside.join("download.bin").exists());
}

#[cfg(unix)]
#[test]
fn upload_preflight_rejects_selected_and_nested_non_regular_entries() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let directory = tempfile::tempdir().unwrap();
    let regular = directory.path().join("regular.txt");
    let link = directory.path().join("link.txt");
    let socket = directory.path().join("socket");
    fs::write(&regular, b"data").unwrap();
    symlink(&regular, &link).unwrap();
    let _listener = UnixListener::bind(&socket).unwrap();

    assert!(collect_upload_tasks(vec![link], "/remote".to_string(), false).is_err());
    assert!(collect_upload_tasks(vec![socket], "/remote".to_string(), false).is_err());
    assert!(collect_upload_tasks(
        vec![directory.path().to_path_buf()],
        "/remote".to_string(),
        true,
    )
    .is_err());
}

#[test]
fn remote_entry_names_cannot_escape_the_download_directory() {
    for name in ["", ".", "..", "/absolute", "child/name", "child\\name"] {
        assert!(
            validate_remote_entry_name(name).is_err(),
            "accepted {name:?}"
        );
    }
    assert!(validate_remote_entry_name("ordinary.txt").is_ok());
}

#[test]
fn conflict_scan_treats_only_typed_not_found_as_absent() {
    let path = "/remote/target";
    let missing = ResolvePathResponse {
        result: Some(resolve_path_response::Result::NotFound(
            ResolvePathNotFound {
                message: "missing".to_string(),
            },
        )),
    };
    let denied = ResolvePathResponse {
        result: Some(resolve_path_response::Result::Error(FileOperationError {
            message: "permission denied".to_string(),
        })),
    };
    let empty = ResolvePathResponse { result: None };

    assert!(decode_remote_path_conflict(path, missing)
        .unwrap()
        .is_none());
    assert_eq!(
        decode_remote_path_conflict(path, denied).unwrap_err(),
        "permission denied"
    );
    assert!(decode_remote_path_conflict(path, empty).is_err());
}

#[test]
fn existing_directory_overwrite_promotes_children_not_root() {
    let batch = ServerFileUploadBatch {
        staging_root: "/remote/.zap-upload-staging/batch".to_string(),
        remote_directory: "/remote".to_string(),
        conflict_policy: UploadConflictPolicy::OverwriteAll,
        directory_roots: vec!["/remote/folder".to_string()],
        phase: UploadBatchPhase::Promoting,
        tasks: vec![ServerFileUploadTask {
            local_path: PathBuf::from("/local/folder/file.txt"),
            file_name: "folder/file.txt".to_string(),
            final_remote_path: "/remote/folder/file.txt".to_string(),
            staging_remote_path: "/remote/.zap-upload-staging/batch/folder/file.txt".to_string(),
            total_bytes: 4,
            uploaded_bytes: Arc::new(AtomicU64::new(4)),
            status: UploadTaskStatus::Completed,
        }],
        next_task_index: 1,
    };

    let promotions = build_pending_promotions(&batch);

    assert_eq!(promotions.len(), 1);
    assert_eq!(promotions[0].kind, SafeFileEntryKind::Regular);
    assert_eq!(
        promotions[0].staging_path,
        "/remote/.zap-upload-staging/batch/folder/file.txt"
    );
    assert_eq!(promotions[0].final_path, "/remote/folder/file.txt");
}

#[test]
fn directory_upload_manifest_includes_empty_and_nested_directories() {
    let local = tempfile::tempdir().unwrap();
    let project = local.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::create_dir(project.join("empty")).unwrap();
    fs::create_dir_all(project.join("nested/deep")).unwrap();
    fs::write(project.join("nested/file.txt"), b"data").unwrap();

    let (files, directories) =
        collect_upload_tasks(vec![project], "/remote".to_string(), true).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].final_remote_path,
        "/remote/project/nested/file.txt"
    );
    assert_eq!(
        directories,
        vec![
            "/remote/project".to_string(),
            "/remote/project/empty".to_string(),
            "/remote/project/nested".to_string(),
            "/remote/project/nested/deep".to_string(),
        ]
    );
}

#[test]
fn skip_and_overwrite_keep_their_conflict_semantics() {
    let files = vec![
        PendingUploadFile {
            local_path: PathBuf::from("/local/existing.txt"),
            final_remote_path: "/remote/existing.txt".to_string(),
            display_name: "existing.txt".to_string(),
            total_bytes: 1,
        },
        PendingUploadFile {
            local_path: PathBuf::from("/local/new.txt"),
            final_remote_path: "/remote/new.txt".to_string(),
            display_name: "new.txt".to_string(),
            total_bytes: 1,
        },
    ];
    let conflicts = HashSet::from(["/remote/existing.txt".to_string()]);

    let skipped = filter_upload_tasks_by_policy(
        files.clone(),
        UploadConflictPolicy::SkipExisting,
        &conflicts,
    );
    let overwritten =
        filter_upload_tasks_by_policy(files, UploadConflictPolicy::OverwriteAll, &conflicts);

    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].final_remote_path, "/remote/new.txt");
    assert_eq!(overwritten.len(), 2);
}
