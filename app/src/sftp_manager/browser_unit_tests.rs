use super::*;
use std::path::PathBuf;

#[test]
fn dropped_directory_move_preparation_restores_quarantined_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("source")).unwrap();
    std::fs::write(root.path().join("source/file.txt"), b"source").unwrap();
    let backend = Arc::new(super::super::sftp_backend::InMemorySftpBackend::new(
        root.path().to_path_buf(),
    )) as Arc<dyn SftpBackend>;

    {
        let quarantine = QuarantinedDirSource::new(backend, PathBuf::from("/source")).unwrap();
        assert!(!root.path().join("source").exists());
        assert!(root
            .path()
            .join(
                quarantine
                    .quarantine
                    .file_name()
                    .expect("quarantine has a file name")
            )
            .exists());
    }

    assert_eq!(
        std::fs::read(root.path().join("source/file.txt")).unwrap(),
        b"source"
    );
}

#[test]
fn remote_relay_conflict_preserves_destination_and_move_source() {
    let source_root = tempfile::tempdir().unwrap();
    let destination_root = tempfile::tempdir().unwrap();
    std::fs::write(source_root.path().join("file.txt"), b"SOURCE").unwrap();
    std::fs::write(destination_root.path().join("file.txt"), b"DESTINATION").unwrap();
    let source_backend = Arc::new(super::super::sftp_backend::InMemorySftpBackend::new(
        source_root.path().to_path_buf(),
    )) as Arc<dyn SftpBackend>;
    let destination_backend = Arc::new(super::super::sftp_backend::InMemorySftpBackend::new(
        destination_root.path().to_path_buf(),
    )) as Arc<dyn SftpBackend>;
    super::super::transfer_job::run_transfer(
        &super::super::transfer_job::TransferJob {
            source_backend,
            target_backend: destination_backend,
            source_path: PathBuf::from("/file.txt"),
            target_path: PathBuf::from("/file.txt"),
            operation: super::super::transfer_job::TransferOperation::Move,
            conflict: super::super::transfer_job::ConflictDecision::Skip,
        },
        &super::super::transfer_job::TransferControl::default(),
        None,
    )
    .expect("an unconfirmed relay conflict should be skipped safely");

    assert_eq!(
        std::fs::read(destination_root.path().join("file.txt")).unwrap(),
        b"DESTINATION"
    );
    assert_eq!(
        std::fs::read(source_root.path().join("file.txt")).unwrap(),
        b"SOURCE"
    );
}

#[test]
fn resolved_symlink_target_selects_file_open_or_navigation_explicitly() {
    assert_eq!(
        action_for_symlink_target(FileEntryType::File, SymlinkActivationIntent::Open),
        ResolvedSymlinkAction::OpenFile
    );
    assert_eq!(
        action_for_symlink_target(FileEntryType::Directory, SymlinkActivationIntent::Open,),
        ResolvedSymlinkAction::Navigate
    );
    assert_eq!(
        action_for_symlink_target(
            FileEntryType::File,
            SymlinkActivationIntent::EnterDirectoryOnly,
        ),
        ResolvedSymlinkAction::Ignore
    );
    assert_eq!(
        action_for_symlink_target(FileEntryType::Other, SymlinkActivationIntent::Open),
        ResolvedSymlinkAction::Unsupported,
        "devices, sockets and FIFOs must never enter the file-open path"
    );
}

#[test]
fn broken_symlink_uses_the_specific_resolution_error() {
    let message = symlink_target_unresolved_message();
    assert_eq!(message, crate::t!("fm-toast-symlink-target-unresolved"));
    assert!(!message.contains("missing target"));
    assert_ne!(
        message,
        crate::t!("fm-toast-list-dir-failed", err = "missing target")
    );
}

#[test]
fn resolved_symlink_download_uses_target_size() {
    assert_eq!(resolved_download_size(0, Some(4096)), 4096);
    assert_eq!(resolved_download_size(512, None), 512);
}

// ============================================================
// normalize_remote_path tests
// ============================================================

/// Test that backslashes are replaced with forward slashes
#[test]
fn test_normalize_remote_path_backslash() {
    let path = PathBuf::from(r"home\user\docs");
    let result = normalize_remote_path(&path);
    assert_eq!(result, PathBuf::from("home/user/docs"));
}

/// Test that a pure forward-slash path is left unchanged
#[test]
fn test_normalize_remote_path_forward_slash() {
    let path = PathBuf::from("/home/user/docs");
    let result = normalize_remote_path(&path);
    assert_eq!(result, PathBuf::from("/home/user/docs"));
}

/// Test the root path
#[test]
fn test_normalize_remote_path_root() {
    let path = PathBuf::from("/");
    let result = normalize_remote_path(&path);
    assert_eq!(result, PathBuf::from("/"));
}

/// Test the empty path
#[test]
fn test_normalize_remote_path_empty() {
    let path = PathBuf::from("");
    let result = normalize_remote_path(&path);
    assert_eq!(result, PathBuf::from(""));
}

/// Test a path with mixed slashes
#[test]
fn test_normalize_remote_path_mixed() {
    let path = PathBuf::from(r"home/user\docs/file.txt");
    let result = normalize_remote_path(&path);
    assert_eq!(result, PathBuf::from("home/user/docs/file.txt"));
}

// ============================================================
// build_rename_path tests
// ============================================================

/// Test rename path construction
#[test]
fn test_build_rename_path_basic() {
    let original = PathBuf::from("/home/user/old.txt");
    let result = build_rename_path(&original, "new.txt");
    assert_eq!(result, Some(PathBuf::from("/home/user/new.txt")));
}

/// Test rename path construction with no parent directory
#[test]
fn test_build_rename_path_no_parent() {
    let original = PathBuf::from("old.txt");
    let result = build_rename_path(&original, "new.txt");
    assert_eq!(result, Some(PathBuf::from("new.txt")));
}

/// Test that a rename path with backslashes is normalized
#[test]
fn test_build_rename_path_normalizes() {
    let original = PathBuf::from("/home/user/old.txt");
    let result = build_rename_path(&original, "new.txt").unwrap();
    assert!(!result.to_string_lossy().contains('\\'));
}

/// Test that rename path construction rejects path injection
#[test]
fn test_build_rename_path_rejects_traversal() {
    let original = PathBuf::from("/home/user/old.txt");
    assert_eq!(build_rename_path(&original, "../etc/passwd"), None);
    assert_eq!(build_rename_path(&original, "/etc/passwd"), None);
    assert_eq!(build_rename_path(&original, "sub/name"), None);
    assert_eq!(build_rename_path(&original, ""), None);
}

// ============================================================
// build_new_folder_path tests
// ============================================================

/// Test new folder path construction
#[test]
fn test_build_new_folder_path_basic() {
    let parent = PathBuf::from("/home/user");
    let result = build_new_folder_path(&parent, "new_dir");
    assert_eq!(result, Some(PathBuf::from("/home/user/new_dir")));
}

/// Test that a new folder path with backslashes is normalized
#[test]
fn test_build_new_folder_path_normalizes() {
    let parent = PathBuf::from("/home/user");
    let result = build_new_folder_path(&parent, "test").unwrap();
    assert!(!result.to_string_lossy().contains('\\'));
}

/// Test that new folder path construction rejects path injection
#[test]
fn test_build_new_folder_path_rejects_traversal() {
    let parent = PathBuf::from("/home/user");
    assert_eq!(build_new_folder_path(&parent, "../etc"), None);
    assert_eq!(build_new_folder_path(&parent, "/etc"), None);
    assert_eq!(build_new_folder_path(&parent, "sub/name"), None);
    assert_eq!(build_new_folder_path(&parent, ""), None);
}

// ============================================================
// build_upload_remote_path tests
// ============================================================

/// Test upload remote path construction
#[test]
fn test_build_upload_remote_path_basic() {
    let current = PathBuf::from("/home/user");
    let result = build_upload_remote_path(&current, "upload.txt");
    assert_eq!(result, Some(PathBuf::from("/home/user/upload.txt")));
}

/// Test that an upload remote path with backslashes is normalized
#[test]
fn test_build_upload_remote_path_normalizes() {
    let current = PathBuf::from("/home/user");
    let result = build_upload_remote_path(&current, "file.txt");
    assert!(result.is_some());
    assert!(!result.unwrap().to_string_lossy().contains('\\'));
}

/// Test that upload remote path construction rejects dangerous file names
#[test]
fn test_build_upload_remote_path_rejects_dangerous() {
    let current = PathBuf::from("/home/user");
    // file_name() extracts "passwd" from "../etc/passwd", so the path is safe
    assert_eq!(
        build_upload_remote_path(&current, "../etc/passwd"),
        Some(PathBuf::from("/home/user/passwd"))
    );
    assert_eq!(build_upload_remote_path(&current, ""), None);
    // file_name() extracts "passwd" from "/etc/passwd", so the path is safe
    assert_eq!(
        build_upload_remote_path(&current, "/etc/passwd"),
        Some(PathBuf::from("/home/user/passwd"))
    );
}

// ============================================================
// initial_connect_path tests (the connect finalize's directory choice)
// ============================================================

/// An explicit, non-root `start_path` (the FM pane-mode toggle's cwd) is
/// honored verbatim, and the remote home is never resolved.
#[test]
fn test_initial_connect_path_honors_explicit_start_path() {
    let requested = Some(PathBuf::from("/srv/app"));
    let mut home_consulted = false;
    let result = SftpBrowserView::initial_connect_path(&requested, || {
        home_consulted = true;
        Some(PathBuf::from("/home/user"))
    });
    assert_eq!(result, PathBuf::from("/srv/app"));
    assert!(
        !home_consulted,
        "the remote home must not be resolved when an explicit start_path is honored"
    );
}

/// The plain "SFTP Browse" entry (`None`) falls back to the remote home.
#[test]
fn test_initial_connect_path_none_uses_home() {
    let result = SftpBrowserView::initial_connect_path(&None, || Some(PathBuf::from("/home/user")));
    assert_eq!(result, PathBuf::from("/home/user"));
}

/// A bare `/` start_path is treated like the plain entry: fall back to home.
#[test]
fn test_initial_connect_path_root_uses_home() {
    let requested = Some(PathBuf::from("/"));
    let result =
        SftpBrowserView::initial_connect_path(&requested, || Some(PathBuf::from("/home/user")));
    assert_eq!(result, PathBuf::from("/home/user"));
}

/// When the home cannot be resolved (`realpath(".")` failed) and no
/// start_path was given, fall back to `/` — the pre-existing behavior.
#[test]
fn test_initial_connect_path_none_no_home_uses_root() {
    let result = SftpBrowserView::initial_connect_path(&None, || None);
    assert_eq!(result, PathBuf::from("/"));
}

#[test]
fn tab_cycles_fm_panes_clockwise() {
    assert!(matches!(
        pane_cycle_action("tab", false),
        Some(crate::pane_group::PaneGroupAction::NavigateNext)
    ));
}

#[test]
fn shift_tab_cycles_counterclockwise() {
    assert!(matches!(
        pane_cycle_action("tab", true),
        Some(crate::pane_group::PaneGroupAction::NavigatePrev)
    ));
}

#[test]
fn f5_f6_f7_f8_f10_dispatch_documented_actions() {
    assert!(matches!(
        function_key_action("f2"),
        Some(SftpBrowserAction::RenameCursor)
    ));
    assert!(matches!(
        function_key_action("f3"),
        Some(SftpBrowserAction::ViewCursorDetails)
    ));
    assert!(matches!(
        function_key_action("f4"),
        Some(SftpBrowserAction::OpenCursorInEditor)
    ));
    assert!(matches!(
        function_key_action("f5"),
        Some(SftpBrowserAction::CopyToOtherPane)
    ));
    assert!(matches!(
        function_key_action("f6"),
        Some(SftpBrowserAction::MoveToOtherPane)
    ));
    assert!(matches!(
        function_key_action("f7"),
        Some(SftpBrowserAction::CreateFolder)
    ));
    assert!(matches!(
        function_key_action("f8"),
        Some(SftpBrowserAction::DeleteSelected)
    ));
    assert!(matches!(
        function_key_action("f10"),
        Some(SftpBrowserAction::CloseFileManager)
    ));
}

#[test]
fn shift_f5_f6_open_the_target_picker() {
    assert!(matches!(
        shifted_function_key_action("f5", true),
        Some(SftpBrowserAction::ChooseCopyTarget)
    ));
    assert!(matches!(
        shifted_function_key_action("F6", true),
        Some(SftpBrowserAction::ChooseMoveTarget)
    ));
    assert!(shifted_function_key_action("f5", false).is_none());
}

#[test]
fn pane_function_legend_drops_captions_before_overlap() {
    let full_width = FUNCTION_BAR.len() as f32 * FUNCTION_LEGEND_CAPTION_MIN_WIDTH
        + FUNCTION_LEGEND_HORIZONTAL_PADDING;
    let compact_width = FUNCTION_BAR.len() as f32 * FUNCTION_LEGEND_KEYCAP_MIN_WIDTH
        + FUNCTION_LEGEND_HORIZONTAL_PADDING;
    assert_eq!(function_legend_mode(full_width), FunctionLegendMode::Full);
    assert_eq!(
        function_legend_mode(full_width - 1.0),
        FunctionLegendMode::Compact
    );
    assert_eq!(
        function_legend_mode(compact_width - 1.0),
        FunctionLegendMode::Hidden
    );
}

#[test]
fn each_pane_owns_optional_compact_function_legend() {
    assert_eq!(function_legend_mode(400.0), FunctionLegendMode::Compact);
    assert_eq!(function_legend_mode(200.0), FunctionLegendMode::Hidden);
}

// ============================================================
// SftpBrowserAction enum tests
// ============================================================

/// Test the SftpBrowserAction::CancelTransfer variant
#[test]
fn test_action_cancel_transfer() {
    let action = SftpBrowserAction::CancelTransfer(42, None);
    assert!(matches!(
        action,
        SftpBrowserAction::CancelTransfer(42, None)
    ));
}

/// Test the SftpBrowserAction::ConfirmMove variant
#[test]
fn test_action_confirm_move() {
    let action = SftpBrowserAction::ConfirmMove;
    assert!(matches!(action, SftpBrowserAction::ConfirmMove));
}

/// Test the SftpBrowserAction::SetSearchFilter variant
#[test]
fn test_action_set_search_filter() {
    let action = SftpBrowserAction::SetSearchFilter("test".into());
    assert!(matches!(action, SftpBrowserAction::SetSearchFilter(_)));
}

/// Test the SftpBrowserAction::ClearSearchFilter variant
#[test]
fn test_action_clear_search_filter() {
    let action = SftpBrowserAction::ClearSearchFilter;
    assert!(matches!(action, SftpBrowserAction::ClearSearchFilter));
}

/// Test the SftpBrowserAction::DownloadSaveAs variant
#[test]
fn test_action_download_save_as() {
    let entry = EntryReference {
        listing_generation: 1,
        identity: EntryIdentity {
            path: PathBuf::from("/remote/file.txt"),
            backend: super::super::types::StableEntryIdentity {
                file_type: FileEntryType::File,
                size: 42,
                object_id: "file".to_string(),
                revision: "1".to_string(),
            },
        },
    };
    let action = SftpBrowserAction::DownloadSaveAs {
        entry: entry.clone(),
        resolved_target_size: None,
        local_path: "/tmp/file.txt".into(),
    };
    assert!(matches!(
        action,
        SftpBrowserAction::DownloadSaveAs {
            entry: actual,
            ..
        } if actual == entry
    ));
}
