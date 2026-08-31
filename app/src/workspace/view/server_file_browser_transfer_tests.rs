use std::fs;

use remote_server::proto::{
    resolve_path_response, FileOperationError, ResolvePathNotFound, ResolvePathResponse,
};
use tokio::io::AsyncWriteExt as _;

use super::*;

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
fn completed_download_atomically_replaces_existing_target() {
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

        download.commit(&destination).await.unwrap();

        assert_eq!(fs::read(destination).unwrap(), b"complete replacement");
        assert!(!sidecar.exists());
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
        let mut download = AtomicDownloadFile::new(&destination).unwrap();
        download
            .output
            .write_all(b"downloaded bytes")
            .await
            .unwrap();

        download.commit(&destination).await.unwrap();

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
fn directory_upload_promotes_the_staged_root_as_one_object() {
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
    assert_eq!(promotions[0].kind, SafeFileEntryKind::Directory);
    assert_eq!(
        promotions[0].staging_path,
        "/remote/.zap-upload-staging/batch/folder"
    );
    assert_eq!(promotions[0].final_path, "/remote/folder");
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
