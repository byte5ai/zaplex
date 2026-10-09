use super::*;
use std::path::PathBuf;

#[test]
fn only_failed_connections_offer_a_connection_retry() {
    assert!(connection_retry_available(&ConnectionState::Failed(
        "connection failed".to_string()
    )));
    assert!(!connection_retry_available(&ConnectionState::Connecting));
    assert!(!connection_retry_available(&ConnectionState::Connected));
    assert!(!connection_retry_available(&ConnectionState::Disconnected));
}

#[test]
fn only_successfully_installed_navigation_commits_request_a_snapshot() {
    assert!(navigation_commit_needs_snapshot(true, true));
    assert!(!navigation_commit_needs_snapshot(true, false));
    assert!(!navigation_commit_needs_snapshot(false, true));
}

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
    let result = build_rename_path(&original, "new.txt", false);
    assert_eq!(result, Some(PathBuf::from("/home/user/new.txt")));
}

/// Test rename path construction with no parent directory
#[test]
fn test_build_rename_path_no_parent() {
    let original = PathBuf::from("old.txt");
    let result = build_rename_path(&original, "new.txt", false);
    assert_eq!(result, Some(PathBuf::from("new.txt")));
}

/// Test that a rename path with backslashes is normalized
#[test]
fn test_build_rename_path_normalizes() {
    let original = PathBuf::from("/home/user/old.txt");
    let result = build_rename_path(&original, "new.txt", false).unwrap();
    assert!(!result.to_string_lossy().contains('\\'));
}

/// Test that rename path construction rejects path injection
#[test]
fn test_build_rename_path_rejects_traversal() {
    let original = PathBuf::from("/home/user/old.txt");
    assert_eq!(build_rename_path(&original, "../etc/passwd", false), None);
    assert_eq!(build_rename_path(&original, "/etc/passwd", false), None);
    assert_eq!(build_rename_path(&original, "sub/name", false), None);
    assert_eq!(build_rename_path(&original, "", false), None);
}

// ============================================================
// build_new_folder_path tests
// ============================================================

/// Test new folder path construction
#[test]
fn test_build_new_folder_path_basic() {
    let parent = PathBuf::from("/home/user");
    let result = build_new_folder_path(&parent, "new_dir", false);
    assert_eq!(result, Some(PathBuf::from("/home/user/new_dir")));
}

/// Test that a new folder path with backslashes is normalized
#[test]
fn test_build_new_folder_path_normalizes() {
    let parent = PathBuf::from("/home/user");
    let result = build_new_folder_path(&parent, "test", false).unwrap();
    assert!(!result.to_string_lossy().contains('\\'));
}

/// Test that new folder path construction rejects path injection
#[test]
fn test_build_new_folder_path_rejects_traversal() {
    let parent = PathBuf::from("/home/user");
    assert_eq!(build_new_folder_path(&parent, "../etc", false), None);
    assert_eq!(build_new_folder_path(&parent, "/etc", false), None);
    assert_eq!(build_new_folder_path(&parent, "sub/name", false), None);
    assert_eq!(build_new_folder_path(&parent, "", false), None);
}

// ============================================================
// build_upload_remote_path tests
// ============================================================

/// Test upload remote path construction
#[test]
fn test_build_upload_remote_path_basic() {
    let current = PathBuf::from("/home/user");
    let result = build_upload_remote_path(&current, "upload.txt", false);
    assert_eq!(result, Some(PathBuf::from("/home/user/upload.txt")));
}

/// Test that an upload remote path with backslashes is normalized
#[test]
fn test_build_upload_remote_path_normalizes() {
    let current = PathBuf::from("/home/user");
    let result = build_upload_remote_path(&current, "file.txt", false);
    assert!(result.is_some());
    assert!(!result.unwrap().to_string_lossy().contains('\\'));
}

/// Test that upload remote path construction rejects dangerous file names
#[test]
fn test_build_upload_remote_path_rejects_dangerous() {
    let current = PathBuf::from("/home/user");
    assert_eq!(
        build_upload_remote_path(&current, "../etc/passwd", false),
        None
    );
    assert_eq!(build_upload_remote_path(&current, "", false), None);
    assert_eq!(
        build_upload_remote_path(&current, "/etc/passwd", false),
        None
    );
}

#[test]
fn safe_name_helpers_accept_embedded_double_dots() {
    let parent = PathBuf::from("/home/user");
    let original = parent.join("old.txt");

    assert_eq!(
        safe_join_name(&parent, "notes..txt"),
        Some(parent.join("notes..txt"))
    );
    assert_eq!(
        build_rename_path(&original, "v1..v2", false),
        Some(parent.join("v1..v2"))
    );
    assert_eq!(
        build_new_folder_path(&parent, "archive.tar..gz", false),
        Some(parent.join("archive.tar..gz"))
    );
    assert_eq!(
        build_upload_remote_path(&parent, "notes..txt", false),
        Some(parent.join("notes..txt"))
    );

    for unsafe_name in ["", ".", "..", "child/name", "child\\name", "/absolute"] {
        assert_eq!(safe_join_name(&parent, unsafe_name), None);
        assert_eq!(build_rename_path(&original, unsafe_name, false), None);
        assert_eq!(build_new_folder_path(&parent, unsafe_name, false), None);
        assert_eq!(build_upload_remote_path(&parent, unsafe_name, false), None);
    }
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

/// The layout `SizeConstraintSwitch` picks for a legend `width` wide: the
/// first breakpoint the width falls below, otherwise one row.
fn function_legend_layout_at(width: f32, cell_min_width: f32) -> FunctionLegendLayout {
    function_legend_breakpoints(cell_min_width)
        .into_iter()
        .find(|(below_width, _)| width < *below_width)
        .map_or(FunctionLegendLayout::OneRow, |(_, layout)| layout)
}

#[test]
fn function_legend_cell_fits_keycap_gap_and_caption() {
    // 2x6 cell padding + 20 key text + 2x(4 padding + 1 border) keycap chrome
    // + 4 gap + 50 caption + 2 rounding slack.
    assert_eq!(function_legend_cell_min_width(20.0, 50.0), 98.0);
    // A wider caption widens the cell one for one.
    assert_eq!(function_legend_cell_min_width(20.0, 60.0), 108.0);
}

#[test]
fn function_legend_wraps_rows_instead_of_hiding_captions() {
    // 100 px cells, 4 px between cells, 2x8 px bar padding:
    // eight in a row need 8x100 + 7x4 + 16 = 844 px,
    // four in a row need 4x100 + 3x4 + 16 = 428 px.
    assert_eq!(
        function_legend_breakpoints(100.0),
        [
            (428.0, FunctionLegendLayout::FourRows),
            (844.0, FunctionLegendLayout::TwoRows),
        ]
    );
    assert_eq!(
        function_legend_layout_at(1200.0, 100.0),
        FunctionLegendLayout::OneRow
    );
    assert_eq!(
        function_legend_layout_at(844.0, 100.0),
        FunctionLegendLayout::OneRow
    );
    assert_eq!(
        function_legend_layout_at(843.5, 100.0),
        FunctionLegendLayout::TwoRows
    );
    assert_eq!(
        function_legend_layout_at(428.0, 100.0),
        FunctionLegendLayout::TwoRows
    );
    assert_eq!(
        function_legend_layout_at(427.5, 100.0),
        FunctionLegendLayout::FourRows
    );
    // Far below the four-row minimum the legend still keeps four rows.
    assert_eq!(
        function_legend_layout_at(120.0, 100.0),
        FunctionLegendLayout::FourRows
    );
}

#[test]
fn reported_1024px_pane_keeps_one_row_of_short_captions() {
    // The pane that hid F2/F7/F8/F10 was about 1024 px wide; one row holds
    // cells up to (1024 - 7x4 - 16) / 8 = 122.5 px, well above what an
    // eight-character caption next to an "F10" keycap needs at 12 px.
    assert_eq!(
        function_legend_layout_at(1024.0, 122.5),
        FunctionLegendLayout::OneRow
    );
    assert_eq!(
        function_legend_layout_at(1024.0, 123.0),
        FunctionLegendLayout::TwoRows
    );
}

#[test]
fn every_function_legend_layout_fills_equal_rows() {
    assert_eq!(FUNCTION_BAR.len(), 8);
    for (layout, cells_per_row, rows) in [
        (FunctionLegendLayout::OneRow, 8, 1),
        (FunctionLegendLayout::TwoRows, 4, 2),
        (FunctionLegendLayout::FourRows, 2, 4),
    ] {
        assert_eq!(layout.cells_per_row(), cells_per_row, "{layout:?}");
        assert_eq!(
            FUNCTION_BAR.chunks(layout.cells_per_row()).count(),
            rows,
            "{layout:?}"
        );
    }
}

#[test]
fn function_bar_captions_are_single_short_words_in_every_catalog() {
    let caption_ids = [
        "fm-key-rename",
        "fm-key-view",
        "fm-key-edit",
        "fm-key-copy",
        "fm-key-move",
        "fm-key-mkdir",
        "fm-key-delete",
        "fm-key-terminal",
    ];
    for (locale, catalog) in [
        ("en", include_str!("../../i18n/en/warp.ftl")),
        ("de", include_str!("../../i18n/de/warp.ftl")),
    ] {
        for id in caption_ids {
            let prefix = format!("{id} = ");
            let caption = catalog
                .lines()
                .find_map(|line| line.strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("{locale} catalog lacks {id}"));
            assert!(!caption.is_empty(), "{locale} {id} is empty");
            assert!(
                caption.chars().count() <= 8 && !caption.contains(char::is_whitespace),
                "{locale} {id} = {caption:?} is not one short word"
            );
        }
    }
}

#[test]
fn function_bar_actions_require_the_focused_compatible_pane() {
    let view = SftpBrowserAction::ViewCursorDetails;
    assert!(function_bar_action_enabled(&view, true, false));
    assert!(!function_bar_action_enabled(&view, false, true));

    let move_action = SftpBrowserAction::MoveToOtherPane;
    assert!(!function_bar_action_enabled(&move_action, true, false));
    assert!(function_bar_action_enabled(&move_action, true, true));
}

#[test]
fn parent_navigation_rejects_failed_and_superseded_listings_without_moving_selection() {
    use super::super::browser_integration_tests::{create_connected_view, initialize_app};

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(
            &mut app,
            &[("departed/keep.txt", b"keep"), ("alpha/file.txt", b"a")],
        );
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::NavigateTo(PathBuf::from("/departed")),
                ctx,
            );
            let parent_entries = view
                .sftp
                .as_ref()
                .unwrap()
                .list_dir(Path::new("/"))
                .unwrap();
            let before_path = view.current_path.clone();
            let before_cursor = view.cursor;
            let before_entries = view
                .entries
                .iter()
                .map(FileEntry::entry_identity)
                .collect::<Vec<_>>();
            let before_history = view.path_history.clone();
            let before_history_index = view.history_index;
            let pending = || NavigationCommit {
                path: PathBuf::from("/"),
                history: vec![
                    PathBuf::from("/"),
                    PathBuf::from("/departed"),
                    PathBuf::from("/"),
                ],
                history_index: 2,
                departed_directory: Some(PathBuf::from("/departed")),
            };
            view.refresh_generation = 10;
            view.is_loading = true;
            view.on_dir_listed_with_navigation(
                9,
                Some(pending()),
                Ok(Ok(parent_entries.clone())),
                ctx,
            );
            assert!(
                view.is_loading,
                "a stale response must not finish the current request"
            );
            assert_eq!(view.current_path, before_path);
            assert_eq!(view.cursor, before_cursor);
            assert_eq!(view.path_history, before_history);
            assert_eq!(view.history_index, before_history_index);
            assert_eq!(
                view.entries
                    .iter()
                    .map(FileEntry::entry_identity)
                    .collect::<Vec<_>>(),
                before_entries
            );

            view.on_dir_listed_with_navigation(
                10,
                Some(pending()),
                Ok(Err(super::super::sftp_ops::SftpOpsError::Operation(
                    "listing denied".into(),
                ))),
                ctx,
            );
            assert!(!view.is_loading);
            assert_eq!(view.current_path, before_path);
            assert_eq!(view.cursor, before_cursor);
            assert_eq!(view.path_history, before_history);
            assert_eq!(view.history_index, before_history_index);
            assert_eq!(
                view.entries
                    .iter()
                    .map(FileEntry::entry_identity)
                    .collect::<Vec<_>>(),
                before_entries
            );

            view.refresh_generation = 11;
            view.on_dir_listed_with_navigation(11, Some(pending()), Ok(Ok(parent_entries)), ctx);
            assert_eq!(view.current_path, PathBuf::from("/"));
            let selected = view
                .cursor_entry_index()
                .and_then(|index| view.entries.get(index));
            assert_eq!(selected.map(|entry| entry.name.as_str()), Some("departed"));
        });
    });
}

#[test]
fn escape_dismisses_focused_overlays_through_real_key_routing() {
    use crate::pane_group::focus_state::PaneGroupFocusState;
    use crate::pane_group::pane::PaneId;
    use crate::sftp_manager::browser_integration_tests::{
        create_connected_view, initialize_app, key_down, presenter_for_window, rerender,
    };

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view, _directory) =
            create_connected_view(&mut app, &[("keep.txt", b"keep")]);
        let pane_id = PaneId::dummy_pane_id();
        let focus_state = app.add_model(|_| PaneGroupFocusState::new(pane_id, None, true));
        view.update(&mut app, |view, ctx| {
            view.set_focus_handle(PaneFocusHandle::new(pane_id, focus_state), ctx);
            view.focus_contents(ctx);
            view.dialog = Some(Dialog::CloseTransferPanelConfirm);
            view.context_menu = Some(ContextMenuState::new(
                view.entry_reference(0).unwrap(),
                Vector2F::new(100.0, 100.0),
            ));
        });
        let (presenter, invalidation) = presenter_for_window(&app, window_id);
        rerender(&mut app, presenter.clone(), invalidation.clone());
        key_down(&mut app, window_id, presenter.clone(), "shift-escape");
        view.read(&app, |view, _| {
            assert!(view.context_menu.is_some());
            assert!(view.dialog.is_some());
        });
        assert!(key_down(&mut app, window_id, presenter.clone(), "escape"));
        view.read(&app, |view, _| {
            assert!(view.context_menu.is_none(), "the context menu closes first");
            assert!(
                view.dialog.is_some(),
                "one Escape must not also close the dialog"
            );
        });
        rerender(&mut app, presenter.clone(), invalidation.clone());
        assert!(key_down(&mut app, window_id, presenter.clone(), "escape"));
        view.read(&app, |view, _| assert!(view.dialog.is_none()));

        // Host-key prompts use the separate disconnected rendering branch.
        view.update(&mut app, |view, _| {
            view.connection = ConnectionState::Failed("host key confirmation required".to_string());
            view.dialog = Some(Dialog::ConfirmUnknownHostKey {
                host: "example.invalid".to_string(),
                port: 22,
                fingerprint_sha256: "SHA256:test".to_string(),
                key_type: "ssh-ed25519".to_string(),
            });
            view.has_focus_within = false;
        });
        rerender(&mut app, presenter.clone(), invalidation.clone());
        key_down(&mut app, window_id, presenter.clone(), "escape");
        view.read(&app, |view, _| assert!(view.dialog.is_some()));
        view.update(&mut app, |view, _| view.has_focus_within = true);
        rerender(&mut app, presenter.clone(), invalidation);
        assert!(key_down(&mut app, window_id, presenter, "escape"));
        view.read(&app, |view, _| assert!(view.dialog.is_none()));
        assert_eq!(
            std::fs::read(_directory.path().join("keep.txt")).unwrap(),
            b"keep"
        );
    });
}

#[cfg(unix)]
#[test]
fn local_navigation_keeps_literal_backslash_names_distinct_from_nested_paths() {
    use crate::sftp_manager::browser_integration_tests::initialize_app;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let literal = directory.path().join(r"a\b");
        let nested = directory.path().join("a/b");
        std::fs::create_dir_all(literal.join("child")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(literal.join("literal.txt"), b"literal").unwrap();
        std::fs::write(nested.join("nested.txt"), b"nested").unwrap();
        let (_, browser) = app.add_window(warpui::platform::WindowStyle::NotStealFocus, |ctx| {
            SftpBrowserView::new_local(directory.path().to_path_buf(), ctx)
        });
        browser.update(&mut app, |browser, ctx| {
            browser.navigate_to(literal.clone(), ctx);
            assert_eq!(browser.current_path, literal);
            assert!(browser
                .entries
                .iter()
                .any(|entry| entry.name == "literal.txt"));
            assert!(!browser
                .entries
                .iter()
                .any(|entry| entry.name == "nested.txt"));
            browser.navigate_to(literal.join("child"), ctx);
            browser.go_up(ctx);
            assert_eq!(browser.current_path, literal);
            assert_eq!(
                build_new_folder_path(&literal, "new", true),
                Some(literal.join("new"))
            );
            assert_eq!(
                build_rename_path(&literal.join("old"), "new", true),
                Some(literal.join("new"))
            );
            assert_eq!(
                build_upload_remote_path(&literal, "upload", true),
                Some(literal.join("upload"))
            );
        });
        assert_eq!(std::fs::read(nested.join("nested.txt")).unwrap(), b"nested");
    });
}

#[cfg(windows)]
#[test]
fn local_path_formatting_preserves_windows_verbatim_disk_and_unc_prefixes() {
    for value in [r"\\?\C:\work\child", r"\\?\UNC\server\share\child"] {
        let path = Path::new(value);
        assert_eq!(
            normalize_browser_path(path, true).as_os_str(),
            path.as_os_str()
        );
        assert_eq!(
            build_new_folder_path(path, "new", true),
            Some(path.join("new"))
        );
    }
    let remote = Path::new(r"\srv\work\child");
    assert_eq!(
        normalize_browser_path(remote, false),
        normalize_remote_path(remote)
    );
}

#[test]
fn space_marks_whether_the_platform_spells_it_as_a_blank_or_a_name() {
    // The platform layer reports the space bar as " " (only the keymap spells
    // it "space"); the old `"space"`-only match never fired.
    for key in [" ", "space"] {
        assert!(matches!(
            list_key_action(key, false, false),
            Some(SftpBrowserAction::ToggleSelectCursor)
        ));
    }
    // While the filter field has focus, Space and Escape belong to the text.
    assert!(list_key_action(" ", false, true).is_none());
    assert!(list_key_action("escape", false, true).is_none());
    assert!(matches!(
        list_key_action("escape", false, false),
        Some(SftpBrowserAction::ClearMarks)
    ));
    assert!(matches!(
        list_key_action("insert", false, false),
        Some(SftpBrowserAction::MarkAndAdvance)
    ));
}

#[test]
fn shift_arrows_mark_while_plain_arrows_only_move() {
    assert!(matches!(
        list_key_action("down", true, false),
        Some(SftpBrowserAction::MarkAndStep { down: true })
    ));
    assert!(matches!(
        list_key_action("up", true, false),
        Some(SftpBrowserAction::MarkAndStep { down: false })
    ));
    assert!(matches!(
        list_key_action("down", false, false),
        Some(SftpBrowserAction::CursorDown)
    ));
    assert!(matches!(
        list_key_action("up", false, false),
        Some(SftpBrowserAction::CursorUp)
    ));
}

#[test]
fn mark_all_is_cmd_or_ctrl_a_without_other_modifiers() {
    let chord = |text: &str| warpui::keymap::Keystroke::parse(text).unwrap();
    assert!(is_mark_all_chord(&chord("cmd-a")));
    assert!(is_mark_all_chord(&chord("ctrl-a")));
    assert!(
        !is_mark_all_chord(&chord("a")),
        "a plain `a` is not a chord"
    );
    assert!(!is_mark_all_chord(&chord("cmd-shift-A")));
    assert!(!is_mark_all_chord(&chord("ctrl-alt-a")));
    assert!(!is_mark_all_chord(&chord("cmd-b")));
}

#[test]
fn modified_clicks_toggle_or_extend_and_plain_clicks_stay_plain() {
    use super::super::file_list::{modified_click, ModifiedClick};
    use warpui::event::ModifiersState;

    let with = |cmd, ctrl, shift, alt| ModifiersState {
        cmd,
        ctrl,
        shift,
        alt,
        ..Default::default()
    };
    assert_eq!(
        modified_click(&with(true, false, false, false)),
        Some(ModifiedClick::Toggle)
    );
    assert_eq!(
        modified_click(&with(false, true, false, false)),
        Some(ModifiedClick::Toggle)
    );
    assert_eq!(
        modified_click(&with(false, false, true, false)),
        Some(ModifiedClick::Range)
    );
    assert_eq!(
        modified_click(&with(true, false, true, false)),
        Some(ModifiedClick::Toggle),
        "Cmd wins over Shift"
    );
    assert_eq!(modified_click(&with(false, false, false, false)), None);
    assert_eq!(modified_click(&with(false, false, false, true)), None);
}

#[test]
fn marked_rows_cursor_and_inactive_pane_stay_distinct() {
    use super::super::file_list::row_look;

    let appearance = Appearance::mock();
    let theme = appearance.theme();
    let plain = row_look(theme, false, false, true, false);
    let marked = row_look(theme, true, false, true, false);
    let cursor = row_look(theme, false, true, true, false);
    let both = row_look(theme, true, true, true, false);
    let inactive_marked = row_look(theme, true, false, false, false);
    let inactive_cursor = row_look(theme, false, true, false, false);

    // A mark is colour (accent-tinted text and fill) plus a bold name.
    assert_eq!(plain.background, None);
    assert!(marked.bold && !plain.bold && !cursor.bold);
    assert_eq!(marked.name, internal_colors::accent_fg_strong(theme));
    assert_eq!(
        marked.detail, marked.name,
        "the whole line carries the mark"
    );
    assert_eq!(
        marked.background,
        Some(internal_colors::accent_overlay_2(theme))
    );
    assert_ne!(marked.name, plain.name);

    // The cursor has its own channel: the accent outline.
    assert_eq!(cursor.outline, Some(theme.accent()));
    assert_eq!(marked.outline, None);
    assert_ne!(cursor.background, marked.background);
    assert_eq!(cursor.name, plain.name);

    // Cursor on a marked row shows both at once.
    assert_eq!(both.outline, Some(theme.accent()));
    assert_eq!(both.background, marked.background);
    assert!(both.bold);

    // An inactive pane dims, but keeps the marks distinguishable; its cursor
    // turns into a neutral outline.
    assert!(inactive_marked.bold);
    assert_eq!(inactive_marked.name, marked.name);
    assert_eq!(
        inactive_marked.background,
        Some(internal_colors::accent_overlay_1(theme))
    );
    assert_ne!(inactive_marked.background, marked.background);
    assert_eq!(
        inactive_cursor.outline,
        Some(internal_colors::fg_overlay_3(theme))
    );
    assert_eq!(inactive_cursor.background, None);

    // Hover only fills an otherwise plain row (spec A4).
    assert_eq!(
        row_look(theme, false, false, true, true).background,
        Some(internal_colors::fg_overlay_1(theme))
    );
    assert_eq!(row_look(theme, true, false, true, true), marked);
}

#[test]
fn transfer_targets_name_the_user_the_connection_authenticates_as() {
    use diesel::Connection as _;
    use diesel_migrations::MigrationHarness as _;
    use warp_ssh_manager::{AuthType, OneKeyCredentialKind};

    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    conn.run_pending_migrations(persistence::MIGRATIONS)
        .unwrap();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared-key",
        "deploy",
        OneKeyCredentialKind::Key,
        Some("/home/deploy/.ssh/id_ed25519"),
    )
    .unwrap();
    let mut info = SshServerInfo::new_default(String::new());
    info.host = "edge.example.com".into();
    info.port = 2222;
    info.auth_type = AuthType::OneKey;
    info.username = "ignored-local-user".into();
    info.credential_id = Some(credential.id);
    let node = SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();
    let server = SshRepository::get_server(&mut conn, &node.id)
        .unwrap()
        .unwrap();

    assert_eq!(
        registry_login_identity(&mut conn, &server),
        "deploy@edge.example.com:2222"
    );
    // Like the pane header: the SSH host, the node's name only without one.
    assert_eq!(server_host_label(&mut conn, &server), "edge.example.com");
    let without_host = SshServerInfo {
        host: String::new(),
        ..server
    };
    assert_eq!(server_host_label(&mut conn, &without_host), "edge");
}
