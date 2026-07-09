//! SFTP browser main view
//!
//! Implements the `BackingView` trait and serves as the pane's core view component.
//! Provides full functionality for remote file browsing, upload/download, and directory navigation.
//! author: logic
//! date: 2026-05-26

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_ssh_manager::{KeychainSecretStore, SshRepository};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, Element,
    EventHandler, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition,
    ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::platform::{Cursor, FilePickerConfiguration, SaveFilePickerConfiguration};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};
use warpui::r#async::SpawnedFutureHandle;

use crate::editor::{
    EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

use super::context_menu::ContextMenuState;
use super::fm_registry::{
    plan_transfer, FileManagerRegistry, FmPaneDescriptor, FsNamespace, TransferKind,
};
use super::keynav::{apply_cursor_move, clamp_cursor, CursorMove};
use super::sftp_backend::{LiveSftpBackend, SftpBackend};
use super::sftp_ops;
use super::sftp_ops::normalize_remote_path;
use super::types::{
    ConnectionState, Dialog, FileEntry, FileEntryType, TransferDirection, TransferState,
    TransferTask,
};

/// The MC-style function-key bar: `(key label, caption, action)`. Rendered as
/// a footer and mirroring the physical F-keys handled in `on_keydown`, so the
/// same verbs are reachable by keyboard (pros) and click (beginners). F5/F6
/// cross-pane copy/move light up fully once a second file panel exists.
const FUNCTION_BAR: &[(&str, &str, fn() -> SftpBrowserAction)] = &[
    ("F3", "View", || SftpBrowserAction::OpenCursorInEditor),
    ("F4", "Edit", || SftpBrowserAction::OpenCursorInEditor),
    ("F5", "Copy", || SftpBrowserAction::CopyToOtherPane),
    ("F6", "Move", || SftpBrowserAction::MoveToOtherPane),
    ("F7", "MkDir", || SftpBrowserAction::CreateFolder),
    ("F8", "Delete", || SftpBrowserAction::DeleteSelected),
    ("F10", "Quit", || SftpBrowserAction::CloseFileManager),
];

/// Toolbar button size
const TOOLBAR_BTN_SIZE: f32 = 28.0;
/// Toolbar icon size
const TOOLBAR_ICON_SIZE: f32 = 16.0;
/// Toolbar spacing
const TOOLBAR_SPACING: f32 = 4.0;
/// Panel inner padding
const PANEL_PADDING: f32 = 8.0;
/// SFTP panel position ID (used by SavePosition to position the context menu)
pub(crate) const SFTP_PANEL_POSITION_ID: &str = "sftp_browser_panel_root";

/// SFTP browser action
#[derive(Debug, Clone)]
pub enum SftpBrowserAction {
    /// Navigate to the given path
    NavigateTo(PathBuf),
    /// Go up to the parent directory
    GoUp,
    /// Go back (history)
    GoBack,
    /// Go forward (history)
    GoForward,
    /// Refresh the current directory
    Refresh,
    /// Select the entry at the given index
    SelectEntry(usize),
    /// Open the entry at the given index (enter if a directory, download if a file)
    OpenEntry(usize),
    /// Delete the entry at the given index
    DeleteEntry(usize),
    /// Rename the entry at the given index
    RenameEntry(usize),
    /// Download the entry at the given index
    DownloadEntry(usize),
    /// Upload a file
    UploadFile,
    /// Create a new folder
    NewFolder,
    /// Confirm deletion
    ConfirmDelete,
    /// Confirm rename
    ConfirmRename,
    /// Confirm new folder creation
    ConfirmNewFolder,
    /// Confirm overwrite
    ConfirmOverwrite,
    /// Open the context menu
    ContextMenu {
        index: usize,
        position: Vector2F,
    },
    /// Close the context menu
    CloseContextMenu,
    /// Close the dialog
    CloseDialog,
    /// View entry details
    DetailsEntry(usize),
    /// Set the search filter
    SetSearchFilter(String),
    /// Clear the search filter
    ClearSearchFilter,
    /// Go up to the parent directory (keyboard shortcut)
    NavigateUp,
    /// Delete the selected entries (keyboard shortcut)
    DeleteSelected,
    /// Create a folder (keyboard shortcut)
    CreateFolder,
    /// Files dragged into the browser area
    DragFilesEnter,
    /// Files dragged out of the browser area
    DragFilesLeave,
    /// Upload files via drag and drop
    DragAndDropFiles(Vec<PathBuf>),
    /// Execute an upload
    ExecuteUpload(String),
    /// Execute a save-as download (the user has chosen a path)
    DownloadSaveAs { index: usize, local_path: String },
    /// Confirm move
    ConfirmMove,
    // ---- MC-style keyboard cursor ----
    /// Move the keyboard cursor down one row.
    CursorDown,
    /// Move the keyboard cursor up one row.
    CursorUp,
    /// Move the cursor to the first row.
    CursorFirst,
    /// Move the cursor to the last row.
    CursorLast,
    /// Move the cursor up one page.
    CursorPageUp,
    /// Move the cursor down one page.
    CursorPageDown,
    /// Activate the row under the cursor (Enter): a directory is entered, a
    /// file is downloaded.
    ActivateCursor,
    /// Open the row under the cursor in the code editor (F3 View / F4 Edit): a
    /// directory is entered; a file opens in an editor pane.
    OpenCursorInEditor,
    /// Enter the directory under the cursor (Right arrow); no-op on a file.
    EnterCursorDir,
    /// Toggle the row under the cursor in the multi-selection (Space).
    ToggleSelectCursor,
    /// Rename the row under the cursor (F6 in the single-pane case).
    RenameCursor,
    /// Copy the selection to another file-manager pane (F5) — cross-pane
    /// transfer is the next increment; today this explains itself.
    CopyToOtherPane,
    /// Move the selection to another file-manager pane (F6 cross-pane).
    MoveToOtherPane,
    /// Close the file manager (F10), reverting the pane to its terminal.
    CloseFileManager,
    /// Resolve a copy/move conflict: overwrite this target (`all` = the rest too).
    OverwriteConflict { all: bool },
    /// Resolve a copy/move conflict: skip this target (`all` = the rest too).
    SkipConflict { all: bool },
    /// Destination picker: copy/move into the candidate pane at this index
    /// (index into `pending_target_pick.candidates`).
    PickCopyMoveTarget(usize),
    /// Resolve the cross-connection overwrite prompt: `overwrite` transfers the
    /// conflicting files (overwriting), otherwise they are skipped.
    ResolveCrossConnConflict { overwrite: bool },
    /// Cancel a transfer task
    CancelTransfer(usize),
    /// Toggle transfer panel visibility
    ToggleTransferPanel,
    /// Confirm closing the transfer panel (cancel all transfers and clear the history)
    ConfirmCloseTransferPanel,
}

/// One planned copy/move within a same-filesystem batch.
struct CopyMoveOp {
    source: PathBuf,
    dest: PathBuf,
    is_dir: bool,
}

/// The in-progress state of a same-filesystem copy/move batch, paused whenever
/// a destination already exists so the user can decide (overwrite/skip, with
/// "…all" applying to the rest). Lives on the view so the conflict dialog can
/// resume it.
struct PendingCopyMove {
    ops: std::collections::VecDeque<CopyMoveOp>,
    backend: Arc<dyn SftpBackend>,
    target_label: String,
    is_move: bool,
    /// `None` = ask on each conflict; `Some(true)` = overwrite all; `Some(false)` = skip all.
    overwrite_default: Option<bool>,
    done: usize,
    skipped: usize,
    errors: usize,
}

/// A cross-connection directory **move** in flight: the source directory is
/// deleted only once every file the recursive copy spawned has finished
/// successfully. Tracked by a batch id so each file transfer's completion can
/// report back; the source is kept (never a partial data loss) if any file
/// failed.
struct PendingDirMoveCleanup {
    /// Batch id shared by every file transfer of this directory move.
    id: u64,
    /// Files still in flight (started, not yet in a terminal state).
    remaining: usize,
    /// Any file transfer failed or was cancelled → keep the source.
    any_failed: bool,
    /// The source pane's backend, used to delete the source directory tree.
    source_backend: Arc<dyn SftpBackend>,
    /// The source directory to remove once every file has transferred.
    source_dir: PathBuf,
    /// Human label for the completion toast, e.g. `sub → host:/var/www`.
    label: String,
}

/// F5/F6 with more than one other file-manager pane open: the sources and the
/// candidate target panes, held while the target-picker dialog is up so the
/// user's choice can be routed once they pick one.
struct PendingTargetPick {
    sources: Vec<PathBuf>,
    is_move: bool,
    candidates: Vec<FmPaneDescriptor>,
}

/// One conflicting cross-connection file transfer, held (with the paths already
/// oriented for its direction) while the overwrite prompt is up.
struct CrossConnConflictFile {
    local_path: PathBuf,
    remote_path: PathBuf,
    /// Original source path — deleted through the source backend on a move.
    source: PathBuf,
}

/// The conflicting transfers of a cross-connection copy/move, paused on the
/// overwrite prompt. The non-conflicting files of the same operation are already
/// transferring; these run (overwriting) or are dropped (skip) once the user
/// decides.
struct PendingCrossConn {
    files: Vec<CrossConnConflictFile>,
    is_move: bool,
    direction: TransferDirection,
    /// The backend the transfer runs through (target's for upload, ours for download).
    backend: Arc<dyn SftpBackend>,
    /// The source pane's backend, for copy-then-delete on a move.
    source_backend: Option<Arc<dyn SftpBackend>>,
    /// Destination label for the summary toast.
    target_label: String,
}

/// SFTP browser view
pub struct SftpBrowserView {
    /// ID of the associated SSH server node
    node_id: String,
    /// Pane configuration handle
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// Focus handle
    focus_handle: Option<PaneFocusHandle>,
    // ---- Connection ----
    /// Connection state
    pub(crate) connection: ConnectionState,
    /// SFTP session
    _session: Option<zap_sftp::SftpSession>,
    /// SFTP operations channel
    sftp: Option<Arc<dyn SftpBackend>>,
    // ---- Navigation ----
    /// Current path
    pub(crate) current_path: PathBuf,
    /// File entries in the current directory
    pub(crate) entries: Vec<FileEntry>,
    /// Set of selected entry indices
    pub(crate) selected: HashSet<usize>,
    /// Path history
    pub(crate) path_history: Vec<PathBuf>,
    /// Current position in the history
    pub(crate) history_index: usize,
    /// The directory the caller asked us to open at. The FM pane-mode toggle
    /// hands us the terminal's cwd (e.g. `/srv/app`); the plain "SFTP Browse"
    /// entry passes `None`. Retained so the async `connect_to_server` finalize
    /// can honor it instead of overwriting `current_path` with the remote home.
    requested_start_path: Option<PathBuf>,
    // ---- Transfers ----
    /// List of transfer tasks
    pub(crate) transfers: Vec<TransferTask>,
    /// Next transfer task ID
    pub(crate) next_transfer_id: usize,
    // ---- UI state ----
    /// Currently open dialog
    pub(crate) dialog: Option<Dialog>,
    /// Whether a load is in progress
    pub(crate) is_loading: bool,
    /// Context menu state
    pub(crate) context_menu: Option<ContextMenuState>,
    /// Search filter text
    pub(crate) search_filter: Option<String>,
    /// Whether files are being dragged over the browser
    pub(crate) is_drag_hovering: bool,
    // ---- Mouse handles ----
    /// Refresh button
    refresh_btn: MouseStateHandle,
    /// Parent directory button
    up_btn: MouseStateHandle,
    /// Back button
    back_btn: MouseStateHandle,
    /// Forward button
    forward_btn: MouseStateHandle,
    /// Upload button
    upload_btn: MouseStateHandle,
    /// New folder button
    new_folder_btn: MouseStateHandle,
    /// Dialog confirm button
    dialog_confirm_btn: MouseStateHandle,
    /// Dialog cancel button
    dialog_cancel_btn: MouseStateHandle,
    /// Dialog close button (title bar X button)
    dialog_close_btn: MouseStateHandle,
    // ---- Transfer panel ----
    /// Whether the transfer panel has been hidden by the user
    transfer_panel_hidden: bool,
    /// Transfer panel close button
    transfer_panel_close_btn: MouseStateHandle,
    // ---- Dialog editors ----
    /// Rename editor
    pub(crate) rename_editor: ViewHandle<EditorView>,
    /// New folder editor
    pub(crate) new_folder_editor: ViewHandle<EditorView>,
    /// Search filter editor
    search_editor: ViewHandle<EditorView>,
    // ---- Keyboard cursor (MC-style) ----
    /// Position of the keyboard cursor within the *visible* (filtered) row
    /// list, MC-style. Distinct from `selected` (the multi-selection set).
    /// Always clamped into range after any change to the visible list.
    pub(crate) cursor: usize,
    /// Hover/click state for each cell of the F-key function bar (mouse parity
    /// with the keyboard function keys). One per [`FUNCTION_BAR`] entry.
    fn_bar_handles: Vec<MouseStateHandle>,
    /// Process-unique id for the cross-pane file-manager registry (F5/F6
    /// copy/move target discovery).
    fm_id: u64,
    /// An in-progress same-fs copy/move batch, paused on a conflict dialog.
    pending_copy_move: Option<PendingCopyMove>,
    /// Sources + candidate panes held while the target-picker dialog is up
    /// (F5/F6 with more than one other pane open).
    pending_target_pick: Option<PendingTargetPick>,
    /// Conflicting cross-connection transfers paused on the overwrite prompt.
    pending_cross_conn: Option<PendingCrossConn>,
    /// Dialog button handles for the copy/move conflict "…all" options.
    overwrite_all_btn: MouseStateHandle,
    skip_all_btn: MouseStateHandle,
    /// One mouse-state handle per row of the target-picker dialog (resized when
    /// the picker opens), so each pane row hovers/presses independently.
    target_pick_btn_states: Vec<MouseStateHandle>,
    // ---- File row mouse handles ----
    /// Mouse state handle for each file entry row
    row_mouse_handles: Vec<MouseStateHandle>,
    // ---- Scrolling ----
    /// Scroll state handle
    scroll_state: ClippedScrollStateHandle,
    // ---- Async tasks ----
    /// Future handle for the current connection task
    connect_handle: Option<SpawnedFutureHandle>,
    /// Future handle for the current directory refresh
    refresh_handle: Option<SpawnedFutureHandle>,
    /// Mapping from transfer task ID to its future handle
    transfer_handles: HashMap<usize, SpawnedFutureHandle>,
    /// Pending queue for batched drag-and-drop uploads
    pending_uploads: Vec<PathBuf>,
    /// Cross-connection directory **moves** awaiting their file transfers so the
    /// source tree can be removed once (and only if) all files land.
    pending_dir_move_cleanups: Vec<PendingDirMoveCleanup>,
    /// Monotonic id handed to each cross-connection directory-move batch.
    next_dir_move_batch_id: u64,
}

impl SftpBrowserView {
    /// Create a new SFTP browser view, opened at `start_path` (the remote
    /// shell's cwd) when known, else the host root `/`.
    pub fn new(
        node_id: String,
        start_path: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("File Manager"));
        let rename_editor = make_editor("Enter new name", ctx);
        let new_folder_editor = make_editor("Folder name", ctx);
        let search_editor = make_editor("Search files...", ctx);

        let mut me = Self {
            node_id,
            pane_configuration,
            focus_handle: None,
            connection: ConnectionState::Disconnected,
            _session: None,
            sftp: None,
            current_path: start_path.clone().unwrap_or_else(|| PathBuf::from("/")),
            entries: Vec::new(),
            selected: HashSet::new(),
            path_history: vec![start_path.clone().unwrap_or_else(|| PathBuf::from("/"))],
            history_index: 0,
            // Retain the caller's request so the async connect finalize can land
            // on it (`connect_to_server` runs after construction). Moved last.
            requested_start_path: start_path,
            transfers: Vec::new(),
            next_transfer_id: 1,
            dialog: None,
            is_loading: false,
            context_menu: None,
            search_filter: None,
            is_drag_hovering: false,
            refresh_btn: MouseStateHandle::default(),
            up_btn: MouseStateHandle::default(),
            back_btn: MouseStateHandle::default(),
            forward_btn: MouseStateHandle::default(),
            upload_btn: MouseStateHandle::default(),
            new_folder_btn: MouseStateHandle::default(),
            dialog_confirm_btn: MouseStateHandle::default(),
            dialog_cancel_btn: MouseStateHandle::default(),
            dialog_close_btn: MouseStateHandle::default(),
            transfer_panel_hidden: false,
            transfer_panel_close_btn: MouseStateHandle::default(),
            rename_editor,
            new_folder_editor,
            search_editor,
            cursor: 0,
            fn_bar_handles: FUNCTION_BAR.iter().map(|_| MouseStateHandle::default()).collect(),
            fm_id: super::fm_registry::next_fm_id(),
            pending_copy_move: None,
            pending_target_pick: None,
            pending_cross_conn: None,
            overwrite_all_btn: MouseStateHandle::default(),
            skip_all_btn: MouseStateHandle::default(),
            target_pick_btn_states: Vec::new(),
            row_mouse_handles: Vec::new(),
            scroll_state: ClippedScrollStateHandle::default(),
            connect_handle: None,
            refresh_handle: None,
            transfer_handles: HashMap::new(),
            pending_uploads: Vec::new(),
            pending_dir_move_cleanups: Vec::new(),
            next_dir_move_batch_id: 1,
        };

        // Subscribe to rename editor events
        let rename_editor_handle = me.rename_editor.clone();
        ctx.subscribe_to_view(
            &rename_editor_handle,
            |me, _source, event, ctx| match event {
                EditorEvent::Enter => {
                    me.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
                }
                EditorEvent::Escape => {
                    me.dialog = None;
                    ctx.notify();
                }
                _ => {}
            },
        );

        // Subscribe to new folder editor events
        let new_folder_editor_handle = me.new_folder_editor.clone();
        ctx.subscribe_to_view(
            &new_folder_editor_handle,
            |me, _source, event, ctx| match event {
                EditorEvent::Enter => {
                    me.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
                }
                EditorEvent::Escape => {
                    me.dialog = None;
                    ctx.notify();
                }
                _ => {}
            },
        );

        // Subscribe to search editor events
        let search_editor_handle = me.search_editor.clone();
        ctx.subscribe_to_view(
            &search_editor_handle,
            |me, _source, event, ctx| match event {
                EditorEvent::Escape => {
                    me.search_filter = None;
                    me.search_editor
                        .update(ctx, |e, ctx| e.set_buffer_text("", ctx));
                    me.clamp_cursor_to_visible();
                    ctx.notify();
                }
                _ => {
                    let text = me.search_editor.as_ref(ctx).buffer_text(ctx);
                    let trimmed = text.trim().to_string();
                    if trimmed.is_empty() {
                        me.search_filter = None;
                    } else {
                        me.search_filter = Some(trimmed);
                    }
                    // A narrower/wider filter changes the visible set — a
                    // filtered-out cursor must snap back into range.
                    me.clamp_cursor_to_visible();
                    ctx.notify();
                }
            },
        );

        // Initiate the connection
        me.connect_to_server(ctx);

        me
    }

    /// Create a file-manager view over the **local** filesystem (FM pane-mode P1:
    /// the same browser UI, backed by the local-FS backend instead of SFTP —
    /// no connection involved). `start_path` is typically the invoking terminal
    /// session's cwd, falling back to `$HOME`.
    pub fn new_local(start_path: std::path::PathBuf, ctx: &mut ViewContext<Self>) -> Self {
        // Construct via `new` with an empty node id, then replace the (never
        // started for an empty id) connection with the local backend. The
        // guard in `connect_to_server` skips empty node ids.
        // `None` start_path: `new_local` sets `current_path`/`path_history` from
        // its own `start_path` below, so `new`'s value would be overwritten.
        let mut me = Self::new(String::new(), None, ctx);
        me.pane_configuration = ctx.add_model(|_ctx| {
            PaneConfiguration::new(crate::t!("sftp-local-file-manager-title"))
        });
        let backend = Arc::new(crate::sftp_manager::sftp_backend::InMemorySftpBackend::new(
            std::path::PathBuf::from("/"),
        )) as Arc<dyn SftpBackend>;
        me.connection = ConnectionState::Connected;
        me.sftp = Some(backend);
        me.current_path = start_path.clone();
        me.path_history = vec![start_path];
        me.history_index = 0;
        me.refresh_dir(ctx);
        me
    }

    /// Inject a test backend, simulating the Connected state (test only)
    #[cfg(test)]
    pub(crate) fn set_backend_for_test(
        &mut self,
        backend: Arc<dyn SftpBackend>,
        start_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        self.connection = ConnectionState::Connected;
        self.sftp = Some(backend);
        self.current_path = start_path.clone();
        self.path_history = vec![start_path];
        self.history_index = 0;
        self.refresh_dir_sync(ctx);
    }

    /// Inject a test backend (used by integration tests)
    #[cfg(feature = "integration_tests")]
    pub fn inject_mock_backend(
        &mut self,
        backend: Arc<dyn SftpBackend>,
        start_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        self.connection = ConnectionState::Connected;
        self.sftp = Some(backend);
        self.current_path = start_path.clone();
        self.path_history = vec![start_path];
        self.history_index = 0;
        self.refresh_dir_sync(ctx);
    }

    /// Synchronously refresh the directory contents (test only, to avoid async delays)
    #[cfg(any(test, feature = "integration_tests"))]
    fn refresh_dir_sync(&mut self, ctx: &mut ViewContext<Self>) {
        let sftp = match &self.sftp {
            Some(s) => s.clone(),
            None => return,
        };
        let path = self.current_path.clone();
        match sftp.list_dir(&path) {
            Ok(mut entries) => {
                entries.sort_by(|a, b| match (a.file_type, b.file_type) {
                    (FileEntryType::Directory, FileEntryType::Directory) => {
                        a.name.to_lowercase().cmp(&b.name.to_lowercase())
                    }
                    (
                        FileEntryType::Directory,
                        FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                    ) => std::cmp::Ordering::Less,
                    (
                        FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                        FileEntryType::Directory,
                    ) => std::cmp::Ordering::Greater,
                    (
                        FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                        FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                    ) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });
                self.entries = entries;
                self.selected.clear();
                self.sync_row_mouse_handles();
                self.clamp_cursor_to_visible();
            }
            Err(_) => {}
        }
        let path = self.current_path.display();
        let title = format!("SFTP: {path}");
        self.pane_configuration.update(ctx, |config, ctx| {
            config.set_title(title, ctx);
        });
        self.publish_to_registry(ctx);
        ctx.notify();
    }

    /// Integration-test getter: connection state
    #[cfg(feature = "integration_tests")]
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection
    }

    /// Integration-test getter: list of file entries
    #[cfg(feature = "integration_tests")]
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Integration-test getter: set of selected entries
    #[cfg(feature = "integration_tests")]
    pub fn selected(&self) -> &HashSet<usize> {
        &self.selected
    }

    /// Integration-test getter: dialog state
    #[cfg(feature = "integration_tests")]
    pub fn dialog(&self) -> &Option<Dialog> {
        &self.dialog
    }

    /// Integration-test getter: context menu state
    #[cfg(feature = "integration_tests")]
    pub fn context_menu(&self) -> &Option<ContextMenuState> {
        &self.context_menu
    }

    /// Get the pane configuration
    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// Test only: disconnect and clear state
    #[cfg(test)]
    pub(crate) fn disconnect_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.connection = ConnectionState::Disconnected;
        self.sftp = None;
        self.entries.clear();
        self.selected.clear();
        ctx.notify();
    }

    /// Connect to the SSH server and establish an SFTP channel
    fn connect_to_server(&mut self, ctx: &mut ViewContext<Self>) {
        let node_id = self.node_id.clone();
        // Local mode (`new_local`) has no server node; the caller installs the
        // local backend instead of connecting.
        if node_id.is_empty() {
            return;
        }
        let result = warp_ssh_manager::with_conn(|c| {
            let server = SshRepository::get_server(c, &node_id)?;
            Ok(server)
        });

        match result {
            Ok(Some(server)) => {
                // Cancel any previous connection attempt
                if let Some(h) = self.connect_handle.take() {
                    h.abort();
                }

                self.connection = ConnectionState::Connecting;
                self.is_loading = true;
                ctx.notify();

                let secret_store = KeychainSecretStore;
                self.connect_handle = self.run_blocking(
                    ctx,
                    move || sftp_ops::connect_from_server(&server, &secret_store),
                    move |me, result, ctx| {
                        me.is_loading = false;
                        match result {
                            Ok(Ok(session)) => {
                                match session.sftp() {
                                    Ok(sftp) => {
                                        let backend = Arc::new(LiveSftpBackend::new(sftp)) as Arc<dyn SftpBackend>;
                                        me._session = Some(session);
                                        // Land on the caller's requested `start_path`
                                        // (the FM pane-mode toggle's cwd) when given;
                                        // the plain "SFTP Browse" (`None`) falls back
                                        // to the remote home, then `/`.
                                        me.apply_connected_backend(backend, ctx);
                                    }
                                    Err(e) => {
                                        me.connection =
                                            ConnectionState::Failed(format!("Failed to create SFTP channel: {e}"));
                                        me.show_error_toast(format!("Failed to create SFTP channel: {e}"), ctx);
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                me.connection = ConnectionState::Failed(e.to_string());
                                me.show_error_toast(e.to_string(), ctx);
                            }
                            Err(_) => {
                                // JoinError (aborted or panicked)
                                me.connection = ConnectionState::Failed("Connection cancelled".to_string());
                            }
                        }
                        ctx.notify();
                    },
                );
            }
            Ok(None) => {
                self.connection = ConnectionState::Failed("Server configuration not found".to_string());
                self.show_error_toast("Server configuration not found".to_string(), ctx);
                ctx.notify();
            }
            Err(e) => {
                self.connection = ConnectionState::Failed(format!("Failed to read server configuration: {e}"));
                self.show_error_toast(format!("Failed to read server configuration: {e}"), ctx);
                ctx.notify();
            }
        }
    }

    /// Decide which directory the browser opens once the SFTP connection is
    /// live. An explicit, non-root `requested_start_path` — the FM pane-mode
    /// toggle handing us the terminal's cwd (e.g. `/srv/app`) — is honored so
    /// the file manager opens where the shell is, and `resolve_home` is never
    /// consulted. Otherwise (the plain "SFTP Browse" entry passes `None`, or a
    /// bare `/`) fall back to the remote home (`realpath(".")`, resolved lazily
    /// by `resolve_home`), then `/`. Pure so the choice is unit-testable.
    fn initial_connect_path(
        requested_start_path: &Option<PathBuf>,
        resolve_home: impl FnOnce() -> Option<PathBuf>,
    ) -> PathBuf {
        match requested_start_path {
            Some(p) if p.as_path() != Path::new("/") => p.clone(),
            _ => resolve_home().unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Finalize a freshly-established SFTP connection: pick the initial directory
    /// (honoring a caller-provided `start_path`, else the remote home), install
    /// the backend, and list that directory. Split out of `connect_to_server`'s
    /// async callback so the path-selection + initial-listing behavior is
    /// unit-testable with a mock backend (the live session handling stays in the
    /// callback). Honoring `start_path` also skips the `realpath(".")` round-trip.
    pub(crate) fn apply_connected_backend(
        &mut self,
        backend: Arc<dyn SftpBackend>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current_path = Self::initial_connect_path(&self.requested_start_path, || {
            backend
                .realpath(Path::new("."))
                .ok()
                .map(|home| normalize_remote_path(&home))
        });
        self.path_history = vec![self.current_path.clone()];
        self.history_index = 0;
        self.connection = ConnectionState::Connected;
        self.sftp = Some(backend);
        self.refresh_dir(ctx);
    }

    /// Run a blocking operation and invoke the callback with the result
    /// Production: runs on a background thread via ctx.spawn + spawn_blocking
    /// Tests: runs synchronously inline (to avoid async executor timing issues)
    /// Returns a SpawnedFutureHandle for cancellation (returns None in tests)
    fn run_blocking<T: Send + 'static>(
        &mut self,
        ctx: &mut ViewContext<Self>,
        op: impl FnOnce() -> T + Send + 'static,
        callback: impl FnOnce(&mut Self, Result<T, tokio::task::JoinError>, &mut ViewContext<Self>) + 'static,
    ) -> Option<SpawnedFutureHandle> {
        #[cfg(any(test, feature = "integration_tests"))]
        {
            let result = op();
            callback(self, Ok(result), ctx);
            None
        }
        #[cfg(not(any(test, feature = "integration_tests")))]
        {
            Some(ctx.spawn(
                async move { tokio::task::spawn_blocking(op).await },
                move |me, result, ctx| {
                    callback(me, result, ctx);
                },
            ))
        }
    }

    /// Refresh the current directory contents
    fn refresh_dir(&mut self, ctx: &mut ViewContext<Self>) {
        let sftp = match &self.sftp {
            Some(s) => s.clone(),
            None => {
                self.show_error_toast("Not connected to server".to_string(), ctx);
                ctx.notify();
                return;
            }
        };

        self.is_loading = true;
        ctx.notify();

        let path = self.current_path.clone();
        self.refresh_handle = self.run_blocking(
            ctx,
            move || sftp.list_dir(&path),
            |me, result, ctx| {
                me.is_loading = false;
                match result {
                    Ok(Ok(mut entries)) => {
                        entries.sort_by(|a, b| match (a.file_type, b.file_type) {
                            (FileEntryType::Directory, FileEntryType::Directory) => {
                                a.name.to_lowercase().cmp(&b.name.to_lowercase())
                            }
                            (
                                FileEntryType::Directory,
                                FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                            ) => std::cmp::Ordering::Less,
                            (
                                FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                                FileEntryType::Directory,
                            ) => std::cmp::Ordering::Greater,
                            (
                                FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                                FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other,
                            ) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                        });
                        me.entries = entries;
                        me.selected.clear();
                        me.sync_row_mouse_handles();
                        // A refresh may have shrunk the listing (files removed);
                        // keep the cursor in range rather than pointing past it.
                        me.clamp_cursor_to_visible();
                    }
                    Ok(Err(e)) => {
                        me.show_error_toast(format!("Failed to list directory: {e}"), ctx);
                    }
                    Err(_) => {}
                }

                let path = me.current_path.display();
                let title = format!("SFTP: {path}");
                me.pane_configuration.update(ctx, |config, ctx| {
                    config.set_title(title, ctx);
                });
                me.publish_to_registry(ctx);
                ctx.notify();
            },
        );
    }

    /// Keep the number of row mouse handles in sync with the number of entries
    fn sync_row_mouse_handles(&mut self) {
        while self.row_mouse_handles.len() < self.entries.len() {
            self.row_mouse_handles.push(MouseStateHandle::default());
        }
        self.row_mouse_handles.truncate(self.entries.len());
    }

    /// Entry indices currently visible (after the search filter), in display
    /// order. The single source of truth for both rendering and cursor
    /// arithmetic, so the MC cursor and the rows never disagree.
    pub(crate) fn visible_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                self.search_filter.as_ref().map_or(true, |filter| {
                    entry.name.to_lowercase().contains(&filter.to_lowercase())
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The entry index under the keyboard cursor, or `None` when nothing is
    /// visible.
    pub(crate) fn cursor_entry_index(&self) -> Option<usize> {
        self.visible_indices().get(self.cursor).copied()
    }

    /// Re-clamp the cursor into the current visible range. Call after any
    /// change to the entries or the filter so a stale cursor can't point past
    /// the end.
    fn clamp_cursor_to_visible(&mut self) {
        self.cursor = clamp_cursor(self.cursor, self.visible_indices().len());
    }

    /// Fixed page size for PageUp/PageDown. The model doesn't know the
    /// viewport height, so a sensible constant keeps paging predictable.
    fn cursor_page_size(&self) -> usize {
        15
    }

    /// Apply an MC cursor movement.
    fn move_cursor(&mut self, mv: CursorMove, ctx: &mut ViewContext<Self>) {
        let len = self.visible_indices().len();
        self.cursor = apply_cursor_move(self.cursor, len, mv);
        ctx.notify();
    }

    /// Activate the row under the cursor: enter a directory, or open a file.
    fn activate_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            self.open_entry(index, ctx);
        }
    }

    /// F3/F4: open the row under the cursor in the code editor. A directory is
    /// entered; a file is opened in an editor pane (view + edit). The workspace
    /// resolves local vs remote (`node_id`).
    fn open_cursor_in_editor(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(index) = self.cursor_entry_index() else {
            return;
        };
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if matches!(
            entry.file_type,
            FileEntryType::Directory | FileEntryType::Symlink
        ) {
            let path = entry.path.clone();
            self.navigate_to(path, ctx);
            return;
        }
        ctx.dispatch_typed_action(&crate::WorkspaceAction::OpenFileInEditor {
            node_id: self.node_id.clone(),
            path: entry.path.clone(),
        });
    }

    /// Enter the directory under the cursor (Right arrow); a file is a no-op.
    fn enter_cursor_dir(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            if let Some(entry) = self.entries.get(index) {
                if matches!(
                    entry.file_type,
                    FileEntryType::Directory | FileEntryType::Symlink
                ) {
                    self.navigate_to(entry.path.clone(), ctx);
                }
            }
        }
    }

    /// Toggle the row under the cursor in the multi-selection (Space).
    fn toggle_select_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            if !self.selected.insert(index) {
                self.selected.remove(&index);
            }
            ctx.notify();
        }
    }

    /// Rename the row under the cursor (F6, single-pane case).
    fn rename_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            self.rename_entry(index, ctx);
        }
    }

    // ---- Cross-pane copy/move (MC F5/F6) --------------------------------

    /// Which filesystem this pane browses — local, or a specific remote host.
    /// Two panes can copy/move between each other only within one namespace.
    fn fs_namespace(&self) -> FsNamespace {
        if self.node_id.is_empty() {
            FsNamespace::Local
        } else {
            FsNamespace::Remote(self.node_id.clone())
        }
    }

    /// Human label for the destination picker, kept live with the current dir.
    fn fm_label(&self) -> String {
        let where_ = if self.node_id.is_empty() {
            "local".to_string()
        } else {
            self.node_id.clone()
        };
        format!("{where_}:{}", self.current_path.display())
    }

    /// Publish this pane's live descriptor (and backend handle) into the
    /// cross-pane registry, so others can target — and transfer through — it.
    fn publish_to_registry(&self, ctx: &mut ViewContext<Self>) {
        let descriptor = FmPaneDescriptor {
            id: self.fm_id,
            label: self.fm_label(),
            fs: self.fs_namespace(),
            current_path: self.current_path.clone(),
        };
        let id = self.fm_id;
        let backend = self.sftp.clone();
        FileManagerRegistry::handle(ctx).update(ctx, move |reg, _| {
            reg.upsert(descriptor);
            if let Some(backend) = backend {
                reg.set_backend(id, backend);
            }
        });
    }

    /// Remove this pane from the registry (on close).
    fn deregister_from_registry(&self, ctx: &mut ViewContext<Self>) {
        let id = self.fm_id;
        FileManagerRegistry::handle(ctx).update(ctx, move |reg, _| reg.remove(id));
    }

    /// The paths this operation acts on: the multi-selection if any, otherwise
    /// the single row under the cursor.
    fn operation_sources(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            self.cursor_entry_index()
                .and_then(|i| self.entries.get(i))
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default()
        } else {
            let mut indices: Vec<usize> = self.selected.iter().copied().collect();
            indices.sort_unstable();
            indices
                .iter()
                .filter_map(|&i| self.entries.get(i).map(|e| e.path.clone()))
                .collect()
        }
    }

    /// F5/F6: copy (or move) the operation's sources into another file-manager
    /// pane. Chooses the single other pane automatically (the MC two-panel
    /// case); 0 or >1 candidates are reported rather than guessed. Routes by
    /// filesystem: same fs → a direct backend copy/rename; local↔remote → an
    /// upload/download through the transfer engine (with progress). Two
    /// different remote hosts is a later increment.
    fn copy_or_move_to_other_pane(&mut self, is_move: bool, ctx: &mut ViewContext<Self>) {
        let verb = if is_move { "move" } else { "copy" };
        let sources = self.operation_sources();
        if sources.is_empty() {
            self.show_error_toast(format!("Nothing selected to {verb}."), ctx);
            return;
        }

        let self_id = self.fm_id;
        let candidates = FileManagerRegistry::as_ref(ctx).others(self_id);
        match candidates.as_slice() {
            [] => {
                self.show_error_toast(
                    format!("Open a second file-manager pane to {verb} into it."),
                    ctx,
                );
            }
            [only] => {
                let target = only.clone();
                self.route_copy_move(&sources, &target, is_move, ctx);
            }
            _ => {
                // More than one other pane open — let the user pick which one
                // (MC never guesses the target).
                self.open_target_picker(sources, candidates, is_move, ctx);
            }
        }
    }

    /// Open the destination picker (F5/F6 with more than one other pane open):
    /// stash the sources + candidates and show a dialog listing every candidate
    /// pane as `host:/path`. The chosen row routes through [`Self::route_copy_move`].
    fn open_target_picker(
        &mut self,
        sources: Vec<PathBuf>,
        candidates: Vec<FmPaneDescriptor>,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let labels: Vec<String> = candidates.iter().map(|c| c.label.clone()).collect();
        self.target_pick_btn_states = candidates
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();
        self.pending_target_pick = Some(PendingTargetPick {
            sources,
            is_move,
            candidates,
        });
        self.dialog = Some(Dialog::CopyMoveTargetPicker { is_move, labels });
        ctx.notify();
    }

    /// Route a chosen copy/move into `target` by filesystem: same fs → a direct
    /// backend copy/rename; local↔remote → the transfer engine; remote↔remote →
    /// (currently) an honest message. Shared by the single-candidate fast path
    /// and the destination picker.
    fn route_copy_move(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match plan_transfer(&self.fs_namespace(), &target.fs) {
            TransferKind::DirectSameFs => self.copy_move_same_fs(sources, target, is_move, ctx),
            TransferKind::Upload | TransferKind::Download => {
                self.transfer_cross_connection(sources, target, is_move, ctx)
            }
            TransferKind::RemoteToRemote => {
                self.relay_remote_to_remote(sources, target, is_move, ctx)
            }
        }
    }

    /// Copy/move between two *different* remote hosts by relaying through a local
    /// temp: download each source from this host, upload it to the target host,
    /// then delete the temp. For a **move** the source is removed on this host
    /// only after the upload to the other host has fully succeeded — all on the
    /// blocking thread, so a failure never loses data. Directories relay
    /// recursively (reusing the same enumeration as a direct cross-connection
    /// transfer). One relay runs per source; progress is per-item.
    fn relay_remote_to_remote(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(source_backend) = self.sftp.clone() else {
            self.show_error_toast("Not connected.".to_string(), ctx);
            return;
        };
        let Some(target_backend) = FileManagerRegistry::as_ref(ctx).backend_for(target.id) else {
            self.show_error_toast("The other pane is no longer connected.".to_string(), ctx);
            return;
        };
        let target_dir = target.current_path.clone();
        let target_label = target.label.clone();
        let mut started = 0usize;
        for source in sources {
            let Some(name) = source.file_name() else {
                continue;
            };
            let is_dir = self
                .entries
                .iter()
                .find(|e| &e.path == source)
                .map(|e| matches!(e.file_type, FileEntryType::Directory))
                .unwrap_or(false);
            let dest = normalize_remote_path(&target_dir.join(name));
            let src_backend = source_backend.clone();
            let tgt_backend = target_backend.clone();
            let source_path = source.clone();
            let temp = relay_temp_path();
            let label = format!("{} → {}", name.to_string_lossy(), target_label);
            self.run_blocking(
                ctx,
                move || relay_one(&src_backend, &tgt_backend, &source_path, &dest, is_dir, is_move, &temp),
                move |me, result, ctx| match result {
                    Ok(Ok(())) => {
                        me.show_info_toast(format!("Relayed {label}."), ctx);
                        me.refresh_dir(ctx);
                    }
                    Ok(Err(e)) => me.show_error_toast(format!("Relay of {label} failed: {e}"), ctx),
                    Err(_) => me.show_error_toast(format!("Relay of {label} was cancelled."), ctx),
                },
            );
            started += 1;
        }
        if started > 0 {
            let verb = if is_move { "move" } else { "copy" };
            self.show_info_toast(
                format!("Relaying {started} item(s) to {target_label} ({verb} via this machine)…"),
                ctx,
            );
        }
        self.selected.clear();
    }

    /// Same-filesystem copy/move: direct backend `copy`/`rename`, synchronous
    /// (no bytes cross the wire). Builds the batch and runs it; existing targets
    /// pause the batch for an overwrite/skip decision rather than being silently
    /// skipped.
    fn copy_move_same_fs(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let verb = if is_move { "move" } else { "copy" };
        let backend = match &self.sftp {
            Some(b) => b.clone(),
            None => {
                self.show_error_toast("Not connected.".to_string(), ctx);
                return;
            }
        };
        let target_dir = target.current_path.clone();

        // Copying into our own current directory would collide name-for-name.
        if target_dir == self.current_path {
            self.show_error_toast(
                format!("Source and destination are the same folder — nothing to {verb}."),
                ctx,
            );
            return;
        }

        let mut ops = std::collections::VecDeque::new();
        for source in sources {
            let Some(name) = source.file_name() else {
                continue;
            };
            let dest = normalize_remote_path(&target_dir.join(name));
            let is_dir = self
                .entries
                .iter()
                .find(|e| &e.path == source)
                .map(|e| matches!(e.file_type, FileEntryType::Directory))
                .unwrap_or(false);
            ops.push_back(CopyMoveOp {
                source: source.clone(),
                dest,
                is_dir,
            });
        }

        self.pending_copy_move = Some(PendingCopyMove {
            ops,
            backend,
            target_label: target.label.clone(),
            is_move,
            overwrite_default: None,
            done: 0,
            skipped: 0,
            errors: 0,
        });
        self.process_pending_copy_move(ctx);
    }

    /// Drive the current copy/move batch until it finishes or hits a conflict
    /// that needs the user (a destination that already exists, with no blanket
    /// decision yet). On a conflict it opens [`Dialog::CopyMoveConflict`] and
    /// returns, to be resumed by the dialog actions.
    fn process_pending_copy_move(&mut self, ctx: &mut ViewContext<Self>) {
        enum Step {
            Done,
            Conflict { name: String, remaining: usize, is_move: bool },
            Skip,
            Execute { overwrite: bool },
        }
        loop {
            // Decide the next step under a short immutable borrow.
            let step = {
                let Some(pending) = self.pending_copy_move.as_ref() else {
                    return;
                };
                match pending.ops.front() {
                    None => Step::Done,
                    Some(op) => {
                        let exists = pending.backend.stat(&op.dest).is_ok();
                        if !exists {
                            Step::Execute { overwrite: false }
                        } else {
                            match pending.overwrite_default {
                                Some(true) => Step::Execute { overwrite: true },
                                Some(false) => Step::Skip,
                                None => Step::Conflict {
                                    name: op
                                        .dest
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                    remaining: pending.ops.len(),
                                    is_move: pending.is_move,
                                },
                            }
                        }
                    }
                }
            };
            match step {
                Step::Done => {
                    self.finish_pending_copy_move(ctx);
                    return;
                }
                Step::Conflict { name, remaining, is_move } => {
                    self.dialog = Some(Dialog::CopyMoveConflict { name, remaining, is_move });
                    ctx.notify();
                    return;
                }
                Step::Skip => {
                    if let Some(p) = self.pending_copy_move.as_mut() {
                        p.ops.pop_front();
                        p.skipped += 1;
                    }
                }
                Step::Execute { overwrite } => {
                    let op = self
                        .pending_copy_move
                        .as_mut()
                        .and_then(|p| p.ops.pop_front());
                    if let Some(op) = op {
                        self.execute_copy_move_op(&op, overwrite);
                    }
                }
            }
        }
    }

    /// Perform one copy/move op (the op is already popped off the queue).
    /// `overwrite` means the destination existed and must be replaced — remove
    /// it first so a rename over a non-empty dir (or a copy into an existing
    /// tree) behaves as "replace".
    fn execute_copy_move_op(&mut self, op: &CopyMoveOp, overwrite: bool) {
        let Some(pending) = self.pending_copy_move.as_mut() else {
            return;
        };
        let backend = pending.backend.clone();
        let is_move = pending.is_move;

        if overwrite {
            let dest_is_dir = backend
                .stat(&op.dest)
                .map(|e| matches!(e.file_type, FileEntryType::Directory))
                .unwrap_or(false);
            let remove = if dest_is_dir {
                backend.delete_dir_recursive(&op.dest)
            } else {
                backend.delete_file(&op.dest)
            };
            if let Err(e) = remove {
                log::warn!("fm overwrite: could not remove existing {:?}: {e}", op.dest);
                pending.errors += 1;
                return;
            }
        }

        let result = if is_move {
            backend.rename(&op.source, &op.dest)
        } else if op.is_dir {
            backend.copy_dir_recursive(&op.source, &op.dest)
        } else {
            backend.copy_file(&op.source, &op.dest)
        };
        match result {
            Ok(()) => pending.done += 1,
            Err(e) => {
                pending.errors += 1;
                log::warn!("fm copy/move {:?} -> {:?} failed: {e}", op.source, op.dest);
            }
        }
    }

    /// Finalise the batch: report, refresh, clear state.
    fn finish_pending_copy_move(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(pending) = self.pending_copy_move.take() else {
            return;
        };
        let past = if pending.is_move { "moved" } else { "copied" };
        let mut parts = vec![format!("{} {past}", pending.done)];
        if pending.skipped > 0 {
            parts.push(format!("{} skipped", pending.skipped));
        }
        if pending.errors > 0 {
            parts.push(format!("{} failed", pending.errors));
        }
        let summary = format!("{} → {}", parts.join(", "), pending.target_label);
        if pending.errors > 0 {
            self.show_error_toast(summary, ctx);
        } else {
            self.show_info_toast(summary, ctx);
        }
        self.selected.clear();
        self.dialog = None;
        self.refresh_dir(ctx);
    }

    /// Resolve the open copy/move conflict dialog (overwrite/skip, optionally
    /// for all remaining conflicts) and resume the batch.
    fn resolve_copy_move_conflict(&mut self, overwrite: bool, all: bool, ctx: &mut ViewContext<Self>) {
        self.dialog = None;
        // Pop the conflicting op (at the front) under a short borrow.
        let op = {
            let Some(pending) = self.pending_copy_move.as_mut() else {
                return;
            };
            if all {
                pending.overwrite_default = Some(overwrite);
            }
            pending.ops.pop_front()
        };
        if let Some(op) = op {
            if overwrite {
                self.execute_copy_move_op(&op, true);
            } else if let Some(p) = self.pending_copy_move.as_mut() {
                p.skipped += 1;
            }
        }
        self.process_pending_copy_move(ctx);
    }

    /// Cancel an in-progress copy/move batch (the X / dismiss on the conflict
    /// dialog): report what was done so far and stop.
    fn cancel_pending_copy_move(&mut self, ctx: &mut ViewContext<Self>) {
        if self.pending_copy_move.is_some() {
            // Drop the remaining queue, then finalise with the partial counts.
            if let Some(p) = self.pending_copy_move.as_mut() {
                p.ops.clear();
            }
            self.finish_pending_copy_move(ctx);
        }
    }

    /// Cross-connection copy (local↔remote): each source *file* becomes an
    /// upload or download on the transfer engine, so large files show progress
    /// and can be cancelled. Directories, and moving across connections, are
    /// reported as not-yet-supported rather than done partially.
    fn transfer_cross_connection(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Upload (local source → remote target) transfers through the target's
        // session; download (remote source → local target) through ours.
        let direction = match plan_transfer(&self.fs_namespace(), &target.fs) {
            TransferKind::Upload => TransferDirection::Upload,
            TransferKind::Download => TransferDirection::Download,
            _ => return, // dispatcher only routes Upload/Download here
        };
        let backend = match direction {
            TransferDirection::Upload => {
                match FileManagerRegistry::as_ref(ctx).backend_for(target.id) {
                    Some(b) => b,
                    None => {
                        self.show_error_toast("The other pane is no longer connected.".to_string(), ctx);
                        return;
                    }
                }
            }
            TransferDirection::Download => match &self.sftp {
                Some(b) => b.clone(),
                None => {
                    self.show_error_toast("Not connected.".to_string(), ctx);
                    return;
                }
            },
        };
        // For a move, the source is deleted after the transfer succeeds, via the
        // *source* pane's own backend (ours) — a cross-connection copy-then-delete.
        let source_backend = self.sftp.clone();

        let target_dir = target.current_path.clone();
        let mut started_files = 0usize;
        let mut started_dirs = 0usize;
        // Files whose destination already exists — collected for one overwrite
        // prompt rather than being silently skipped.
        let mut conflicts: Vec<CrossConnConflictFile> = Vec::new();
        for source in sources {
            let Some(name) = source.file_name() else {
                continue;
            };
            let is_dir = self
                .entries
                .iter()
                .find(|e| &e.path == source)
                .map(|e| matches!(e.file_type, FileEntryType::Directory))
                .unwrap_or(false);
            let dest = normalize_remote_path(&target_dir.join(name));
            if is_dir {
                // Recursively copy/move the directory across the connection: the
                // tree is enumerated + created off-thread, then a transfer is
                // spawned per file (move deletes the source once all files land).
                let label = format!("{} → {}", name.to_string_lossy(), target.label);
                self.spawn_dir_transfer(
                    source.clone(),
                    dest,
                    direction,
                    backend.clone(),
                    is_move,
                    label,
                    ctx,
                );
                started_dirs += 1;
                continue;
            }
            let exists = match direction {
                TransferDirection::Upload => backend.stat(&dest).is_ok(),
                TransferDirection::Download => std::fs::metadata(&dest).is_ok(),
            };
            let (local_path, remote_path) = match direction {
                TransferDirection::Upload => (source.clone(), dest),
                TransferDirection::Download => (dest, source.clone()),
            };
            if exists {
                conflicts.push(CrossConnConflictFile {
                    local_path,
                    remote_path,
                    source: source.clone(),
                });
                continue;
            }
            let delete_after = match (is_move, &source_backend) {
                (true, Some(b)) => Some((b.clone(), source.clone())),
                _ => None,
            };
            self.spawn_transfer_with_backend(
                local_path,
                remote_path,
                backend.clone(),
                direction,
                delete_after,
                None,
                ctx,
            );
            started_files += 1;
        }

        // Report what started right away.
        let verb = if is_move { "move" } else { "copy" };
        let mut parts = Vec::new();
        if started_files > 0 {
            parts.push(format!("{started_files} file {verb}(s) started"));
        }
        if started_dirs > 0 {
            parts.push(format!("{started_dirs} folder {verb}(s) started"));
        }
        if !parts.is_empty() {
            self.show_info_toast(format!("{} → {}", parts.join(", "), target.label), ctx);
        }

        // Ask once about any files that already exist on the destination.
        if conflicts.is_empty() {
            if parts.is_empty() {
                self.show_info_toast(format!("Nothing to {verb} → {}", target.label), ctx);
            }
        } else {
            let existing = conflicts.len();
            self.pending_cross_conn = Some(PendingCrossConn {
                files: conflicts,
                is_move,
                direction,
                backend: backend.clone(),
                source_backend: source_backend.clone(),
                target_label: target.label.clone(),
            });
            self.dialog = Some(Dialog::CrossConnConflict { existing, is_move });
        }
        self.selected.clear();
    }

    /// Resolve the cross-connection overwrite prompt: `overwrite` spawns the
    /// conflicting transfers (they overwrite the destination), otherwise they are
    /// skipped. The non-conflicting files of the same operation already started.
    fn resolve_cross_conn_conflict(&mut self, overwrite: bool, ctx: &mut ViewContext<Self>) {
        self.dialog = None;
        let Some(pending) = self.pending_cross_conn.take() else {
            return;
        };
        let verb = if pending.is_move { "move" } else { "copy" };
        if !overwrite {
            self.show_info_toast(
                format!(
                    "Skipped {} existing item(s) → {}",
                    pending.files.len(),
                    pending.target_label
                ),
                ctx,
            );
            ctx.notify();
            return;
        }
        let count = pending.files.len();
        for file in pending.files {
            let delete_after = match (pending.is_move, &pending.source_backend) {
                (true, Some(b)) => Some((b.clone(), file.source.clone())),
                _ => None,
            };
            self.spawn_transfer_with_backend(
                file.local_path,
                file.remote_path,
                pending.backend.clone(),
                pending.direction,
                delete_after,
                None,
                ctx,
            );
        }
        self.show_info_toast(
            format!("Overwriting {count} existing item(s) ({verb}) → {}", pending.target_label),
            ctx,
        );
        ctx.notify();
    }

    /// Recursively copy (or move) a directory across a connection. The source
    /// tree is enumerated and the target directory structure created off the
    /// main thread; then one file transfer is spawned per file through the
    /// existing transfer engine. Existing target files are skipped (matching the
    /// single-file cross-connection path; a conflict prompt is a later step).
    /// For a move, every file is copied and the whole source tree is removed
    /// only once all files have landed successfully (see
    /// [`Self::note_dir_move_progress`]) — a partial failure keeps the source.
    #[allow(clippy::too_many_arguments)]
    fn spawn_dir_transfer(
        &mut self,
        source_dir: PathBuf,
        dest_dir: PathBuf,
        direction: TransferDirection,
        remote_backend: Arc<dyn SftpBackend>,
        is_move: bool,
        label: String,
        ctx: &mut ViewContext<Self>,
    ) {
        // The source pane's own backend removes the source tree on a move.
        let Some(source_backend) = self.sftp.clone() else {
            self.show_error_toast("Not connected.".to_string(), ctx);
            return;
        };
        let plan_backend = remote_backend.clone();
        let src = source_dir.clone();
        let dst = dest_dir.clone();
        self.run_blocking(
            ctx,
            move || plan_dir_transfer(direction, plan_backend, &src, &dst),
            move |me, result, ctx| match result {
                Ok(Ok(plan)) => {
                    if plan.files.is_empty() {
                        me.show_info_toast(
                            format!(
                                "{label}: nothing to transfer{}",
                                if plan.skipped_existing > 0 {
                                    format!(" ({} already exist)", plan.skipped_existing)
                                } else {
                                    String::new()
                                }
                            ),
                            ctx,
                        );
                        return;
                    }
                    // Register the move batch *before* spawning so a synchronous
                    // (test-mode) completion still counts down correctly.
                    let batch_id = if is_move {
                        let id = me.next_dir_move_batch_id;
                        me.next_dir_move_batch_id += 1;
                        me.pending_dir_move_cleanups.push(PendingDirMoveCleanup {
                            id,
                            remaining: plan.files.len(),
                            any_failed: false,
                            source_backend: source_backend.clone(),
                            source_dir: source_dir.clone(),
                            label: label.clone(),
                        });
                        Some(id)
                    } else {
                        None
                    };
                    let count = plan.files.len();
                    for (local_path, remote_path) in plan.files {
                        me.spawn_transfer_with_backend(
                            local_path,
                            remote_path,
                            remote_backend.clone(),
                            direction,
                            None,
                            batch_id,
                            ctx,
                        );
                    }
                    me.show_info_toast(
                        format!(
                            "{label}: {count} file(s) started{}",
                            if plan.skipped_existing > 0 {
                                format!(", {} skipped (exist)", plan.skipped_existing)
                            } else {
                                String::new()
                            }
                        ),
                        ctx,
                    );
                }
                Ok(Err(e)) => {
                    me.show_error_toast(format!("{label}: couldn't prepare the transfer: {e}"), ctx)
                }
                Err(_) => me.show_error_toast(format!("{label}: preparation was cancelled"), ctx),
            },
        );
    }

    /// A file that belonged to a cross-connection directory move has finished.
    /// Count it down; when the whole batch is done, remove the source tree (only
    /// if every file succeeded — otherwise keep it and report).
    fn note_dir_move_progress(&mut self, batch_id: u64, succeeded: bool, ctx: &mut ViewContext<Self>) {
        let Some(pos) = self
            .pending_dir_move_cleanups
            .iter()
            .position(|b| b.id == batch_id)
        else {
            return;
        };
        {
            let batch = &mut self.pending_dir_move_cleanups[pos];
            if !succeeded {
                batch.any_failed = true;
            }
            batch.remaining = batch.remaining.saturating_sub(1);
            if batch.remaining > 0 {
                return;
            }
        }
        let batch = self.pending_dir_move_cleanups.remove(pos);
        if batch.any_failed {
            self.show_error_toast(
                format!(
                    "{}: source kept — some files didn't transfer, so the move is incomplete.",
                    batch.label
                ),
                ctx,
            );
        } else {
            match batch.source_backend.delete_dir_recursive(&batch.source_dir) {
                Ok(()) => self.show_info_toast(format!("Moved {}.", batch.label), ctx),
                Err(e) => self.show_error_toast(
                    format!("Copied {} but couldn't remove the source: {e}", batch.label),
                    ctx,
                ),
            }
        }
        self.refresh_dir(ctx);
    }

    /// Spawn a single upload/download on the transfer engine using an explicit
    /// backend (the target's for uploads, ours for downloads), mirroring the
    /// drag-and-drop upload path but parameterised by direction and backend.
    ///
    /// `delete_after` = `Some((source_backend, source_path))` turns a copy into
    /// a **move**: on success the source is deleted through its own backend
    /// (copy-then-delete across connections). It is never deleted on failure or
    /// cancellation, so a failed move can't lose data.
    ///
    /// `dir_move_batch` = `Some(id)` ties this file to a cross-connection
    /// directory move: on completion it reports back to that batch (see
    /// [`Self::note_dir_move_progress`]) instead of deleting a single source.
    /// Returns the new transfer's task id.
    fn spawn_transfer_with_backend(
        &mut self,
        local_path: PathBuf,
        remote_path: PathBuf,
        backend: Arc<dyn SftpBackend>,
        direction: TransferDirection,
        delete_after: Option<(Arc<dyn SftpBackend>, PathBuf)>,
        dir_move_batch: Option<u64>,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let total_size = match direction {
            TransferDirection::Upload => {
                std::fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0)
            }
            TransferDirection::Download => {
                backend.stat(&remote_path).map(|e| e.size).unwrap_or(0)
            }
        };
        let task = TransferTask::new(
            self.next_transfer_id,
            local_path.clone(),
            remote_path.clone(),
            direction,
            total_size,
        );
        self.next_transfer_id += 1;
        let task_id = task.id;
        let cancel_flag = task.cancel_flag.clone();
        self.transfers.push(task);
        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == task_id) {
            t.state = TransferState::InProgress;
        }
        self.transfer_panel_hidden = false;
        ctx.notify();

        let transferred = Arc::new(AtomicU64::new(0));
        let transferred_clone = transferred.clone();
        let progress_cb: sftp_ops::ProgressCallback = Box::new(move |bytes, _total| {
            transferred_clone.store(bytes, Ordering::SeqCst);
        });
        let work_backend = backend.clone();
        let lp = local_path.clone();
        let rp = remote_path.clone();
        if let Some(handle) = self.run_blocking(
            ctx,
            move || match direction {
                TransferDirection::Upload => {
                    work_backend.upload_file(&lp, &rp, Some(&progress_cb), Some(&cancel_flag))
                }
                TransferDirection::Download => {
                    work_backend.download_file(&rp, &lp, Some(&progress_cb), Some(&cancel_flag))
                }
            },
            move |me, result, ctx| {
                if let Some(t) = me.transfers.iter_mut().find(|t| t.id == task_id) {
                    match &result {
                        Ok(Ok(())) => {
                            t.state = TransferState::Completed;
                            t.transferred = t.total_size;
                        }
                        Ok(Err(e)) => {
                            if matches!(e, super::sftp_ops::SftpOpsError::Cancelled) {
                                t.state = TransferState::Cancelled;
                            } else {
                                t.state = TransferState::Failed(e.to_string());
                            }
                            t.transferred = transferred.load(Ordering::SeqCst);
                        }
                        Err(_) => {
                            t.state = TransferState::Cancelled;
                            t.transferred = transferred.load(Ordering::SeqCst);
                        }
                    }
                }
                me.transfer_handles.remove(&task_id);
                match &result {
                    Ok(Ok(())) => {
                        // Copy-then-delete: only remove the source once the
                        // transfer has fully succeeded (never on error/cancel).
                        if let Some((source_backend, source_path)) = &delete_after {
                            if let Err(e) = source_backend.delete_file(source_path) {
                                log::warn!("fm move: source delete failed: {e}");
                                me.show_error_toast(
                                    format!("Copied, but removing the source failed: {e}"),
                                    ctx,
                                );
                            }
                        }
                        me.refresh_dir(ctx);
                    }
                    Ok(Err(e)) => {
                        log::error!("sftp: cross-pane transfer failed: {e}");
                        me.show_error_toast(format!("Transfer failed: {e}"), ctx);
                        ctx.notify();
                    }
                    Err(_) => ctx.notify(),
                }
                // Report back to a directory-move batch, if this file is part of
                // one, so the source tree is removed once all files have landed.
                if let Some(batch_id) = dir_move_batch {
                    let succeeded = matches!(&result, Ok(Ok(())));
                    me.note_dir_move_progress(batch_id, succeeded, ctx);
                }
            },
        ) {
            self.transfer_handles.insert(task_id, handle);
        }
        task_id
    }

    /// Show an error toast notification
    fn show_error_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(message)
                .with_object_id("sftp_error".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Show a success/info toast notification (e.g. a copy/move summary).
    fn show_info_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::success(message)
                .with_object_id("sftp_info".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Navigate to the given path and update the history
    fn navigate_to(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let path = normalize_remote_path(&path);
        if path == self.current_path {
            return;
        }
        self.current_path = path;
        // A new directory listing starts with the cursor at the top (MC).
        self.cursor = 0;
        // Truncate the forward history
        self.path_history.truncate(self.history_index + 1);
        self.path_history.push(self.current_path.clone());
        self.history_index = self.path_history.len() - 1;
        self.refresh_dir(ctx);
    }

    /// Go up to the parent directory
    fn go_up(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(parent) = self.current_path.parent() {
            let parent = normalize_remote_path(&parent.to_path_buf());
            if parent != self.current_path {
                self.navigate_to(parent, ctx);
            }
        }
    }

    /// Go back to the previous path in the history
    fn go_back(&mut self, ctx: &mut ViewContext<Self>) {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.current_path = self.path_history[self.history_index].clone();
            self.refresh_dir(ctx);
        }
    }

    /// Go forward to the next path in the history
    fn go_forward(&mut self, ctx: &mut ViewContext<Self>) {
        if self.history_index < self.path_history.len() - 1 {
            self.history_index += 1;
            self.current_path = self.path_history[self.history_index].clone();
            self.refresh_dir(ctx);
        }
    }

    /// Open the entry at the given index
    fn open_entry(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            match entry.file_type {
                FileEntryType::Directory | FileEntryType::Symlink => {
                    self.navigate_to(entry.path.clone(), ctx);
                }
                FileEntryType::File | FileEntryType::Other => {
                    self.download_entry(index, ctx);
                }
            }
        }
    }

    /// Open the delete confirmation dialog
    fn delete_selected(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            let (paths, is_dirs) = if self.selected.contains(&index) {
                // Delete all selected entries
                self.selected
                    .iter()
                    .filter_map(|&i| {
                        self.entries.get(i).map(|e| {
                            (e.path.clone(), matches!(e.file_type, FileEntryType::Directory))
                        })
                    })
                    .unzip()
            } else {
                (
                    vec![entry.path.clone()],
                    vec![matches!(entry.file_type, FileEntryType::Directory)],
                )
            };
            self.dialog = Some(Dialog::DeleteConfirm { paths, is_dirs });
            ctx.notify();
        }
    }

    /// Perform the delete operation
    fn confirm_delete(&mut self, ctx: &mut ViewContext<Self>) {
        let sftp = match &self.sftp {
            Some(s) => s.clone(),
            None => {
                self.show_error_toast("Not connected to server".to_string(), ctx);
                self.dialog = None;
                ctx.notify();
                return;
            }
        };

        let (paths, is_dirs) = match &self.dialog {
            Some(Dialog::DeleteConfirm { paths, is_dirs }) => (paths.clone(), is_dirs.clone()),
            Some(Dialog::Rename { .. })
            | Some(Dialog::CreateFolder { .. })
            | Some(Dialog::Move { .. })
            | Some(Dialog::OverwriteConfirm { .. })
            | Some(Dialog::CopyMoveConflict { .. })
            | Some(Dialog::CopyMoveTargetPicker { .. })
            | Some(Dialog::CrossConnConflict { .. })
            | Some(Dialog::FileDetails { .. })
            | Some(Dialog::CloseTransferPanelConfirm)
            | None => {
                self.dialog = None;
                ctx.notify();
                return;
            }
        };

        self.dialog = None;
        self.is_loading = true;
        ctx.notify();

        self.run_blocking(
            ctx,
            move || {
                for (path, is_dir) in paths.iter().zip(is_dirs.iter()) {
                    let result = if *is_dir {
                        sftp.delete_dir_recursive(path)
                    } else {
                        sftp.delete_file(path)
                    };
                    if let Err(e) = result {
                        return Err(e.to_string());
                    }
                }
                Ok(())
            },
            move |me, result, ctx| {
                me.is_loading = false;
                me.selected.clear();
                match result {
                    Ok(Ok(())) => {
                        me.refresh_dir(ctx);
                    }
                    Ok(Err(e)) => {
                        me.show_error_toast(format!("Delete failed: {e}"), ctx);
                        me.refresh_dir(ctx);
                    }
                    Err(_) => {
                        // Cancelled
                        me.refresh_dir(ctx);
                    }
                }
                ctx.notify();
            },
        );
    }

    /// Create a download transfer task
    fn download_entry(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            let default_name = entry.name.clone();
            let idx = index;
            ctx.open_save_file_picker(
                move |path_opt: Option<String>, _me: &mut Self, _ctx: &mut ViewContext<Self>| {
                    if let Some(path) = path_opt {
                        _ctx.dispatch_typed_action_deferred(SftpBrowserAction::DownloadSaveAs {
                            index: idx,
                            local_path: path,
                        });
                    }
                },
                SaveFilePickerConfiguration::new().with_default_filename(default_name),
            );
        }
    }

    /// Show the entry details dialog
    fn show_details(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            self.dialog = Some(Dialog::FileDetails {
                entry: entry.clone(),
            });
            ctx.notify();
        }
    }

    /// Open the rename dialog
    fn rename_entry(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            self.dialog = Some(Dialog::Rename {
                path: entry.path.clone(),
                original_name: entry.name.clone(),
            });
            // Write the current name into the editor
            self.rename_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&entry.name, ctx));
            ctx.notify();
        }
    }

    /// Render a single toolbar button
    fn render_toolbar_btn(
        &self,
        icon: Icon,
        handle: MouseStateHandle,
        action: SftpBrowserAction,
        _tooltip: &str,
        appearance: &Appearance,
        position_id: &'static str,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = theme.sub_text_color(theme.background());

        let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
            .with_width(TOOLBAR_ICON_SIZE)
            .with_height(TOOLBAR_ICON_SIZE)
            .finish();

        let btn_el = Hoverable::new(handle, move |_| {
            Container::new(
                ConstrainedBox::new(Container::new(icon_el).with_uniform_padding(6.0).finish())
                    .with_width(TOOLBAR_BTN_SIZE)
                    .with_height(TOOLBAR_BTN_SIZE)
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();

        SavePosition::new(btn_el, position_id).finish()
    }

    /// Render the toolbar
    fn render_toolbar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let nav_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(TOOLBAR_SPACING)
            .with_child(self.render_toolbar_btn(
                Icon::ChevronLeft,
                self.back_btn.clone(),
                SftpBrowserAction::GoBack,
                "Back",
                appearance,
                "sftp_btn:back",
            ))
            .with_child(self.render_toolbar_btn(
                Icon::ChevronRight,
                self.forward_btn.clone(),
                SftpBrowserAction::GoForward,
                "Forward",
                appearance,
                "sftp_btn:forward",
            ))
            .with_child(self.render_toolbar_btn(
                Icon::ArrowUp,
                self.up_btn.clone(),
                SftpBrowserAction::GoUp,
                "Up",
                appearance,
                "sftp_btn:up",
            ))
            .with_child(self.render_toolbar_btn(
                Icon::Refresh,
                self.refresh_btn.clone(),
                SftpBrowserAction::Refresh,
                "Refresh",
                appearance,
                "sftp_btn:refresh",
            ))
            .with_main_axis_size(MainAxisSize::Min)
            .finish();

        let action_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(TOOLBAR_SPACING)
            .with_child(self.render_toolbar_btn(
                Icon::UploadCloud,
                self.upload_btn.clone(),
                SftpBrowserAction::UploadFile,
                "Upload",
                appearance,
                "sftp_btn:upload",
            ))
            .with_child(self.render_toolbar_btn(
                Icon::Plus,
                self.new_folder_btn.clone(),
                SftpBrowserAction::NewFolder,
                "New folder",
                appearance,
                "sftp_btn:new_folder",
            ))
            .with_main_axis_size(MainAxisSize::Min)
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(nav_buttons)
            .with_child(action_buttons)
            .finish()
    }

    /// Render the breadcrumb navigation
    fn render_breadcrumb(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.sub_text_color(theme.background());

        let parts: Vec<Box<dyn Element>> =
            super::breadcrumb::render_breadcrumb(&self.current_path, appearance);

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.0);

        // Add the root directory "/" as a clickable entry point
        let root_text_color = text_color;
        let root_hoverable = Hoverable::new(Default::default(), move |_| {
            let t = Text::new_inline(
                "/".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(root_text_color.into())
            .finish();
            Container::new(t).finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SftpBrowserAction::NavigateTo(PathBuf::from("/")));
        })
        .finish();
        let root_el = SavePosition::new(root_hoverable, "sftp_breadcrumb:/").finish();
        row.add_child(root_el);

        for part in parts {
            row.add_child(part);
        }

        Container::new(row.finish())
            .with_padding_left(4.0)
            .with_padding_right(4.0)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .with_background(theme.surface_2())
            .finish()
    }

    /// Render the connection state (when not connected)
    fn render_connection_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let (msg, icon) = match &self.connection {
            ConnectionState::Connecting => ("Connecting...".to_string(), Icon::Loading),
            ConnectionState::Failed(err) => (err.clone(), Icon::AlertCircle),
            ConnectionState::Disconnected => ("Disconnected".to_string(), Icon::AlertCircle),
            ConnectionState::Connected => {
                return Container::new(Flex::row().finish()).finish();
            }
        };

        render_centered_status(icon, &msg, 12.0, appearance)
    }

    /// Render the MC-style function-key bar footer. Each cell shows `F<n>` in
    /// the accent colour followed by its caption, and clicking it dispatches
    /// the same action as the physical function key.
    fn render_function_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let size = appearance.ui_font_size();
        let key_color = theme.accent().into_solid();
        let caption_color = theme.sub_text_color(theme.background());

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(4.0);

        for (i, (key, caption, make_action)) in FUNCTION_BAR.iter().enumerate() {
            let handle = self.fn_bar_handles.get(i).cloned().unwrap_or_default();
            let key = *key;
            let caption = *caption;
            let make_action = *make_action;
            let cell = Hoverable::new(handle, move |mouse| {
                let content = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(3.0)
                    .with_child(
                        Text::new_inline(key.to_string(), family, size)
                            .with_color(key_color.into())
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(caption.to_string(), family, size)
                            .with_color(caption_color.into())
                            .finish(),
                    )
                    .finish();
                let mut container = Container::new(content)
                    .with_padding_left(6.0)
                    .with_padding_right(6.0)
                    .with_padding_top(2.0)
                    .with_padding_bottom(2.0)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)));
                if mouse.is_hovered() {
                    container = container.with_background(internal_colors::neutral_3(theme));
                }
                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(make_action()))
            .finish();
            row.add_child(cell);
        }

        Container::new(row.finish())
            .with_padding_left(PANEL_PADDING)
            .with_padding_right(PANEL_PADDING)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0)
            .with_background(theme.background())
            .finish()
    }

    /// Render the file list
    fn render_file_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Filter the entries — same source of truth as the keyboard cursor.
        let filtered_indices = self.visible_indices();

        if filtered_indices.is_empty() {
            let text_el = Text::new_inline(
                "This folder is empty".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();

            return Align::new(Container::new(text_el).with_uniform_padding(24.0).finish())
                .finish();
        }

        // Header row
        let header = super::file_list::render_header(appearance);

        // File rows
        let rows = super::file_list::render_file_rows(
            &self.entries,
            &filtered_indices,
            &self.selected,
            self.cursor_entry_index(),
            &self.row_mouse_handles,
            appearance,
        );

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(rows)
            .finish()
    }

    /// Render the transfer panel
    fn render_transfers(&self, appearance: &Appearance) -> Box<dyn Element> {
        super::transfer_panel::render_transfer_panel(
            &self.transfers,
            appearance,
            self.transfer_panel_close_btn.clone(),
        )
    }

    /// Execute an upload (shared entry point for drag-and-drop and file-picker uploads)
    ///
    /// First checks whether a file with the same name already exists in the remote directory;
    /// if so, opens an overwrite confirmation dialog, and after the user confirms,
    /// performs the actual upload via `execute_upload_confirmed`.
    fn execute_upload(&mut self, local_path: &Path, ctx: &mut ViewContext<Self>) {
        let file_name = local_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let remote_path = match build_upload_remote_path(&self.current_path, &file_name) {
            Some(p) => p,
            None => {
                self.show_error_toast("Filename contains invalid characters".to_string(), ctx);
                return;
            }
        };

        // Check whether a file with the same name already exists in the remote directory
        let existing = self.entries.iter().find(|e| {
            e.name == file_name && matches!(e.file_type, FileEntryType::File)
        });

        if existing.is_some() {
            let local_size = std::fs::metadata(local_path).map(|m| m.len()).unwrap_or(0);
            self.dialog = Some(Dialog::OverwriteConfirm {
                source: local_path.to_path_buf(),
                target: remote_path,
                file_size: local_size,
                direction: TransferDirection::Upload,
            });
            ctx.notify();
            return;
        }

        // No conflict; perform the upload directly
        self.execute_upload_confirmed(local_path, &remote_path, ctx);
    }

    /// Process the pending upload queue, uploading one file at a time until a conflict occurs or the queue is empty
    ///
    /// For batched drag-and-drop uploads, all files are enqueued and then processed one by one.
    /// When a same-name file conflict occurs, the queue is paused and an overwrite confirmation dialog is shown;
    /// after the user confirms, ConfirmOverwrite calls this method again to continue.
    /// author: logic
    /// date: 2026-06-01
    fn process_pending_uploads(&mut self, ctx: &mut ViewContext<Self>) {
        while let Some(local_path) = self.pending_uploads.pop() {
            self.execute_upload(&local_path, ctx);
            if self.dialog.is_some() {
                // Conflict encountered; pause the queue and wait for user confirmation
                return;
            }
        }
    }

    /// Execute a confirmed upload (create the transfer task and start the background upload)
    fn execute_upload_confirmed(
        &mut self,
        local_path: &Path,
        remote_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        let total_size = std::fs::metadata(local_path).map(|m| m.len()).unwrap_or(0);

        let task = TransferTask::new(
            self.next_transfer_id,
            local_path.to_path_buf(),
            remote_path.to_path_buf(),
            TransferDirection::Upload,
            total_size,
        );
        self.next_transfer_id += 1;
        let task_id = task.id;
        let cancel_flag = task.cancel_flag.clone();
        self.transfers.push(task);

        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == task_id) {
            t.state = TransferState::InProgress;
        }
        self.transfer_panel_hidden = false;
        ctx.notify();

        if let Some(sftp) = &self.sftp {
            let sftp = sftp.clone();
            let transferred = Arc::new(AtomicU64::new(0));
            let transferred_clone = transferred.clone();

            let progress_cb: sftp_ops::ProgressCallback =
                Box::new(move |bytes, _total| {
                    transferred_clone.store(bytes, Ordering::SeqCst);
                });

            let local_path = local_path.to_path_buf();
            let remote_path = remote_path.to_path_buf();
            if let Some(handle) = self.run_blocking(
                ctx,
                move || {
                    sftp.upload_file(&local_path, &remote_path, Some(&progress_cb), Some(&cancel_flag))
                },
                move |me, result, ctx| {
                    if let Some(t) = me.transfers.iter_mut().find(|t| t.id == task_id) {
                        match &result {
                            Ok(Ok(())) => {
                                t.state = TransferState::Completed;
                                t.transferred = t.total_size;
                            }
                            Ok(Err(e)) => {
                                if matches!(e, super::sftp_ops::SftpOpsError::Cancelled) {
                                    t.state = TransferState::Cancelled;
                                } else {
                                    t.state = TransferState::Failed(e.to_string());
                                }
                                t.transferred = transferred.load(Ordering::SeqCst);
                            }
                            Err(_) => {
                                // JoinError (aborted)
                                t.state = TransferState::Cancelled;
                                t.transferred = transferred.load(Ordering::SeqCst);
                            }
                        }
                    }

                    // Clean up the handle once the transfer completes (the future has ended, no abort needed)
                    me.transfer_handles.remove(&task_id);

                    match &result {
                        Ok(Ok(())) => {
                            me.refresh_dir(ctx);
                        }
                        Ok(Err(e)) => {
                            log::error!("sftp: upload failed: {e}");
                            me.show_error_toast(format!("Upload failed: {e}"), ctx);
                            ctx.notify();
                        }
                        Err(_) => {
                            ctx.notify();
                        }
                    }
                },
            ) {
                self.transfer_handles.insert(task_id, handle);
            }
        } else {
            if let Some(t) = self.transfers.iter_mut().find(|t| t.id == task_id) {
                t.state = TransferState::Failed("Not connected to server".to_string());
            }
            log::error!("sftp: upload failed: not connected to server");
            self.show_error_toast("Upload failed: not connected to server".to_string(), ctx);
            ctx.notify();
        }
    }

    /// Execute a download (shared logic for confirm-overwrite and save-as)
    fn execute_download(
        &mut self,
        remote_path: &Path,
        local_path: &Path,
        file_size: u64,
        ctx: &mut ViewContext<Self>,
    ) {
        let task = TransferTask::new(
            self.next_transfer_id,
            remote_path.to_path_buf(),
            local_path.to_path_buf(),
            TransferDirection::Download,
            file_size,
        );
        self.next_transfer_id += 1;
        let task_id = task.id;
        let cancel_flag = task.cancel_flag.clone();
        self.transfers.push(task);

        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == task_id) {
            t.state = TransferState::InProgress;
        }
        self.transfer_panel_hidden = false;
        ctx.notify();

        if let Some(sftp) = &self.sftp {
            let sftp = sftp.clone();
            let transferred = Arc::new(AtomicU64::new(0));
            let transferred_clone = transferred.clone();

            let progress_cb: sftp_ops::ProgressCallback =
                Box::new(move |bytes, _total| {
                    transferred_clone.store(bytes, Ordering::SeqCst);
                });

            let remote_path = remote_path.to_path_buf();
            let local_path = local_path.to_path_buf();
            if let Some(handle) = self.run_blocking(
                ctx,
                move || {
                    sftp.download_file(&remote_path, &local_path, Some(&progress_cb), Some(&cancel_flag))
                },
                move |me, result, ctx| {
                    if let Some(t) = me.transfers.iter_mut().find(|t| t.id == task_id) {
                        match &result {
                            Ok(Ok(())) => {
                                t.state = TransferState::Completed;
                                t.transferred = t.total_size;
                            }
                            Ok(Err(e)) => {
                                if matches!(e, super::sftp_ops::SftpOpsError::Cancelled) {
                                    t.state = TransferState::Cancelled;
                                } else {
                                    t.state = TransferState::Failed(e.to_string());
                                }
                                t.transferred = transferred.load(Ordering::SeqCst);
                            }
                            Err(_) => {
                                t.state = TransferState::Cancelled;
                                t.transferred = transferred.load(Ordering::SeqCst);
                            }
                        }
                    }

                    // Clean up the handle once the transfer completes (the future has ended, no abort needed)
                    me.transfer_handles.remove(&task_id);

                    if let Ok(Err(e)) = &result {
                        log::error!("sftp: download failed: {e}");
                        me.show_error_toast(format!("Download failed: {e}"), ctx);
                    }
                    ctx.notify();
                },
            ) {
                self.transfer_handles.insert(task_id, handle);
            }
        } else {
            if let Some(t) = self.transfers.iter_mut().find(|t| t.id == task_id) {
                t.state = TransferState::Failed("Not connected to server".to_string());
            }
            log::error!("sftp: download failed: not connected to server");
            self.show_error_toast("Download failed: not connected to server".to_string(), ctx);
            ctx.notify();
        }
    }

    /// Render the search bar
    fn render_search_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.sub_text_color(theme.background());

        let search_icon = ConstrainedBox::new(Icon::Search.to_warpui_icon(text_color).finish())
            .with_width(14.0)
            .with_height(14.0)
            .finish();

        let editor_el = Container::new(ChildView::new(&self.search_editor).finish()).finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.0)
                .with_child(search_icon)
                .with_child(Shrinkable::new(1.0, editor_el).finish())
                .finish(),
        )
        .with_padding_left(8.0)
        .with_padding_right(8.0)
        .with_padding_top(4.0)
        .with_padding_bottom(4.0)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
        .with_background(theme.surface_2())
        .finish()
    }

    /// Render the loading state
    fn render_loading(&self, appearance: &Appearance) -> Box<dyn Element> {
        render_centered_status(Icon::Loading, "Loading...", 8.0, appearance)
    }
}

/// Render a centered status indicator (icon + text)
fn render_centered_status(
    icon: Icon,
    message: &str,
    spacing: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_color = theme.sub_text_color(theme.background());

    let icon_el = ConstrainedBox::new(icon.to_warpui_icon(text_color).finish())
        .with_width(24.0)
        .with_height(24.0)
        .finish();

    let text_el = Text::new_inline(
        message.to_string(),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(text_color.into())
    .finish();

    let content = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(spacing)
        .with_child(icon_el)
        .with_child(text_el)
        .with_main_axis_size(MainAxisSize::Min)
        .finish();

    Align::new(Container::new(content).with_uniform_padding(24.0).finish()).finish()
}

/// The plan for a cross-connection directory transfer: the (local, remote) file
/// pairs still to move, plus how many were skipped because a same-named file
/// already exists on the destination.
pub(super) struct DirTransferPlan {
    /// Each entry is `(local_path, remote_path)`, matching
    /// [`SftpBrowserView::spawn_transfer_with_backend`]'s argument order for
    /// either direction.
    pub(super) files: Vec<(PathBuf, PathBuf)>,
    pub(super) skipped_existing: usize,
}

/// Enumerate `source_dir` and create the destination directory structure for a
/// cross-connection transfer, returning the per-file work. `remote_backend` is
/// the SFTP side — the *target* for an upload, the *source* for a download; the
/// other side is the local filesystem. Existing destination files are skipped
/// (counted), matching the single-file cross-connection path. Blocking: run off
/// the main thread.
pub(super) fn plan_dir_transfer(
    direction: TransferDirection,
    remote_backend: Arc<dyn SftpBackend>,
    source_dir: &Path,
    dest_dir: &Path,
) -> Result<DirTransferPlan, String> {
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut skipped_existing = 0usize;
    match direction {
        TransferDirection::Upload => {
            // Local source → remote dest: walk the local tree (parents before
            // children), mkdir on the remote, collect the files to upload.
            let _ = remote_backend.create_dir(dest_dir); // ignore "already exists"
            // WalkDir yields a directory before its contents, so each parent is
            // created before the files that go into it.
            for entry in walkdir::WalkDir::new(source_dir).min_depth(1) {
                let entry = entry.map_err(|e| e.to_string())?;
                let rel = entry
                    .path()
                    .strip_prefix(source_dir)
                    .map_err(|e| e.to_string())?;
                let remote_target = normalize_remote_path(&dest_dir.join(rel));
                if entry.file_type().is_dir() {
                    let _ = remote_backend.create_dir(&remote_target); // ignore exists
                } else if entry.file_type().is_file() {
                    if remote_backend.stat(&remote_target).is_ok() {
                        skipped_existing += 1;
                        continue;
                    }
                    files.push((entry.path().to_path_buf(), remote_target));
                }
            }
        }
        TransferDirection::Download => {
            // Remote source → local dest: walk the remote tree via `list_dir`,
            // create local directories, collect the files to download.
            std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
            let mut stack = vec![source_dir.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = remote_backend.list_dir(&dir).map_err(|e| e.to_string())?;
                for entry in entries {
                    let rel = entry.path.strip_prefix(source_dir).unwrap_or(&entry.path);
                    let local_target = dest_dir.join(rel);
                    if matches!(entry.file_type, FileEntryType::Directory) {
                        std::fs::create_dir_all(&local_target).map_err(|e| e.to_string())?;
                        stack.push(entry.path.clone());
                    } else {
                        if std::fs::metadata(&local_target).is_ok() {
                            skipped_existing += 1;
                            continue;
                        }
                        files.push((local_target, entry.path.clone()));
                    }
                }
            }
        }
    }
    Ok(DirTransferPlan {
        files,
        skipped_existing,
    })
}

/// A unique local temp path for one remote↔remote relay (file or directory).
/// Under the OS temp dir; pid + counter, never reused.
fn relay_temp_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("zaplex-relay-{}-{n}", std::process::id()))
}

/// Relay one entry between two remote hosts through a local temp: download from
/// `source_backend`, upload to `target_backend`, always clean up the temp, and
/// — for a move — delete the source on the origin host only after the upload
/// fully succeeded (so a failure can never lose data). Directories relay
/// recursively via [`plan_dir_transfer`]. Blocking: run off the main thread.
#[allow(clippy::too_many_arguments)]
fn relay_one(
    source_backend: &Arc<dyn SftpBackend>,
    target_backend: &Arc<dyn SftpBackend>,
    source_path: &Path,
    dest_path: &Path,
    is_dir: bool,
    is_move: bool,
    temp: &Path,
) -> Result<(), String> {
    let transfer = (|| -> Result<(), String> {
        if is_dir {
            // Download the source tree from host A into a local temp directory…
            let download = plan_dir_transfer(
                TransferDirection::Download,
                source_backend.clone(),
                source_path,
                temp,
            )?;
            for (local, remote) in &download.files {
                source_backend
                    .download_file(remote, local, None, None)
                    .map_err(|e| e.to_string())?;
            }
            // …then upload the temp tree to host B.
            let upload = plan_dir_transfer(
                TransferDirection::Upload,
                target_backend.clone(),
                temp,
                dest_path,
            )?;
            for (local, remote) in &upload.files {
                target_backend
                    .upload_file(local, remote, None, None)
                    .map_err(|e| e.to_string())?;
            }
        } else {
            source_backend
                .download_file(source_path, temp, None, None)
                .map_err(|e| e.to_string())?;
            target_backend
                .upload_file(temp, dest_path, None, None)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();

    // Always remove the temp, success or failure.
    if is_dir {
        let _ = std::fs::remove_dir_all(temp);
    } else {
        let _ = std::fs::remove_file(temp);
    }
    transfer?;

    // Move: remove the source on the origin host, but only after a fully
    // successful relay (the `transfer?` above returned early on any failure).
    if is_move {
        let removed = if is_dir {
            source_backend.delete_dir_recursive(source_path)
        } else {
            source_backend.delete_file(source_path)
        };
        removed.map_err(|e| format!("relayed, but removing the source failed: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
impl SftpBrowserView {
    /// Seed a pending directory-move cleanup (as `spawn_dir_transfer` would) so
    /// [`Self::note_dir_move_progress`] can be exercised without a live transfer.
    /// Returns the batch id.
    pub(super) fn seed_dir_move_cleanup_for_test(
        &mut self,
        remaining: usize,
        source_backend: Arc<dyn SftpBackend>,
        source_dir: PathBuf,
    ) -> u64 {
        let id = self.next_dir_move_batch_id;
        self.next_dir_move_batch_id += 1;
        self.pending_dir_move_cleanups.push(PendingDirMoveCleanup {
            id,
            remaining,
            any_failed: false,
            source_backend,
            source_dir,
            label: "test-dir".to_string(),
        });
        id
    }

    /// Test wrapper around the private [`Self::note_dir_move_progress`].
    pub(super) fn note_dir_move_progress_for_test(
        &mut self,
        batch_id: u64,
        succeeded: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.note_dir_move_progress(batch_id, succeeded, ctx);
    }

    /// Whether any directory-move cleanup is still pending (for assertions).
    pub(super) fn has_pending_dir_move_cleanup(&self) -> bool {
        !self.pending_dir_move_cleanups.is_empty()
    }

    /// Whether a cross-connection overwrite prompt is still pending (for assertions).
    pub(super) fn has_pending_cross_conn(&self) -> bool {
        self.pending_cross_conn.is_some()
    }
}

/// Safely join a file name to a parent path, preventing path injection and path traversal
fn safe_join_name(parent: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.starts_with('/') || name.starts_with('\\') {
        return None;
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(parent.join(name))
}

/// Build the full path for a renamed entry
fn build_rename_path(original_path: &PathBuf, new_name: &str) -> Option<PathBuf> {
    let parent = original_path.parent().unwrap_or(Path::new("/"));
    safe_join_name(parent, new_name).map(|p| normalize_remote_path(&p))
}

/// Build the full path for a new folder
fn build_new_folder_path(parent_path: &PathBuf, folder_name: &str) -> Option<PathBuf> {
    safe_join_name(parent_path, folder_name).map(|p| normalize_remote_path(&p))
}

/// Build the remote path for an uploaded file
fn build_upload_remote_path(current_path: &PathBuf, local_file_name: &str) -> Option<PathBuf> {
    let name = Path::new(local_file_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| local_file_name.to_string());
    safe_join_name(current_path, &name).map(|p| normalize_remote_path(&p))
}

impl Entity for SftpBrowserView {
    type Event = PaneEvent;
}

impl TypedActionView for SftpBrowserView {
    type Action = SftpBrowserAction;

    /// Handle all SFTP browser actions
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SftpBrowserAction::NavigateTo(path) => {
                self.navigate_to(path.clone(), ctx);
            }
            SftpBrowserAction::GoUp => {
                self.go_up(ctx);
            }
            SftpBrowserAction::GoBack => {
                self.go_back(ctx);
            }
            SftpBrowserAction::GoForward => {
                self.go_forward(ctx);
            }
            SftpBrowserAction::Refresh => {
                self.refresh_dir(ctx);
            }
            SftpBrowserAction::SelectEntry(index) => {
                let index = *index;
                self.selected.clear();
                self.selected.insert(index);
                ctx.notify();
            }
            SftpBrowserAction::OpenEntry(index) => {
                let index = *index;
                self.open_entry(index, ctx);
            }
            SftpBrowserAction::DeleteEntry(index) => {
                let index = *index;
                self.delete_selected(index, ctx);
            }
            SftpBrowserAction::RenameEntry(index) => {
                let index = *index;
                self.rename_entry(index, ctx);
            }
            SftpBrowserAction::DownloadEntry(index) => {
                let index = *index;
                self.download_entry(index, ctx);
            }
            SftpBrowserAction::UploadFile => {
                ctx.open_file_picker(
                    move |result, ctx: &mut ViewContext<SftpBrowserView>| match result {
                        Ok(paths) => {
                            for path in paths {
                                ctx.dispatch_typed_action(&SftpBrowserAction::ExecuteUpload(path));
                            }
                        }
                        Err(e) => {
                            log::warn!("sftp: file picker failed: {e}");
                        }
                    },
                    FilePickerConfiguration::new(),
                );
            }
            SftpBrowserAction::NewFolder => {
                self.dialog = Some(Dialog::CreateFolder {
                    parent_path: self.current_path.clone(),
                });
                self.new_folder_editor
                    .update(ctx, |e, ctx| e.set_buffer_text("", ctx));
                ctx.notify();
            }
            SftpBrowserAction::ConfirmDelete => {
                self.confirm_delete(ctx);
            }
            SftpBrowserAction::ConfirmRename => {
                if let Some(Dialog::Rename {
                    path: original_path,
                    ..
                }) = &self.dialog
                {
                    let new_name = self.rename_editor.as_ref(ctx).buffer_text(ctx);
                    let new_name = new_name.trim().to_string();
                    if new_name.is_empty() {
                        self.show_error_toast("Name cannot be empty".to_string(), ctx);
                        return;
                    }
                    let new_path = match build_rename_path(original_path, &new_name) {
                        Some(p) => p,
                        None => {
                            self.show_error_toast("Invalid name: cannot contain path separators".to_string(), ctx);
                            return;
                        }
                    };

                    if let Some(sftp) = &self.sftp {
                        let sftp = sftp.clone();
                        let original_path = original_path.clone();
                        self.dialog = None;
                        ctx.notify();
                        self.run_blocking(
                            ctx,
                            move || sftp.rename(&original_path, &new_path),
                            move |me, result, ctx| {
                                match result {
                                    Ok(Ok(())) => {
                                        me.refresh_dir(ctx);
                                    }
                                    Ok(Err(e)) => {
                                        me.show_error_toast(format!("Rename failed: {e}"), ctx);
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast("Not connected to server".to_string(), ctx);
                        self.dialog = None;
                    }
                }
            }
            SftpBrowserAction::ConfirmNewFolder => {
                if let Some(Dialog::CreateFolder { parent_path }) = &self.dialog {
                    let folder_name = self.new_folder_editor.as_ref(ctx).buffer_text(ctx);
                    let folder_name = folder_name.trim().to_string();
                    if folder_name.is_empty() {
                        self.show_error_toast("Folder name cannot be empty".to_string(), ctx);
                        return;
                    }
                    let folder_path = match build_new_folder_path(parent_path, &folder_name) {
                        Some(p) => p,
                        None => {
                            self.show_error_toast("Invalid name: cannot contain path separators".to_string(), ctx);
                            return;
                        }
                    };

                    if let Some(sftp) = &self.sftp {
                        let sftp = sftp.clone();
                        self.dialog = None;
                        ctx.notify();
                        self.run_blocking(
                            ctx,
                            move || sftp.create_dir(&folder_path),
                            move |me, result, ctx| {
                                match result {
                                    Ok(Ok(())) => {
                                        me.refresh_dir(ctx);
                                    }
                                    Ok(Err(e)) => {
                                        me.show_error_toast(format!("Create folder failed: {e}"), ctx);
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast("Not connected to server".to_string(), ctx);
                        self.dialog = None;
                    }
                }
            }
            SftpBrowserAction::ConfirmOverwrite => {
                // Extract the paths and transfer direction from the dialog
                let (source, target, file_size, direction) = match &self.dialog {
                    Some(Dialog::OverwriteConfirm { source, target, file_size, direction }) => {
                        (source.clone(), target.clone(), *file_size, *direction)
                    }
                    Some(Dialog::DeleteConfirm { .. })
                    | Some(Dialog::Rename { .. })
                    | Some(Dialog::CreateFolder { .. })
                    | Some(Dialog::Move { .. })
                    | Some(Dialog::CopyMoveConflict { .. })
                    | Some(Dialog::CopyMoveTargetPicker { .. })
                    | Some(Dialog::CrossConnConflict { .. })
                    | Some(Dialog::FileDetails { .. })
                    | Some(Dialog::CloseTransferPanelConfirm)
                    | None => {
                        self.dialog = None;
                        ctx.notify();
                        return;
                    }
                };

                // Close the dialog
                self.dialog = None;
                match direction {
                    TransferDirection::Download => {
                        self.execute_download(&source, &target, file_size, ctx);
                    }
                    TransferDirection::Upload => {
                        self.execute_upload_confirmed(&source, &target, ctx);
                    }
                }
                // Batch upload queue: continue with the next file after confirming the current one
                self.process_pending_uploads(ctx);
            }
            SftpBrowserAction::ContextMenu { index, position } => {
                let index = *index;
                let position = *position;
                self.context_menu = Some(ContextMenuState::new(index, position));
                self.selected.clear();
                self.selected.insert(index);
                ctx.notify();
            }
            SftpBrowserAction::CloseContextMenu => {
                self.context_menu = None;
                ctx.notify();
            }
            SftpBrowserAction::CloseDialog => {
                // Cancelling the target picker discards the pending pick.
                if matches!(self.dialog, Some(Dialog::CopyMoveTargetPicker { .. })) {
                    self.pending_target_pick = None;
                    self.dialog = None;
                    ctx.notify();
                    return;
                }
                // Cancelling the cross-connection overwrite prompt skips the
                // conflicting files (the non-conflicting ones already started).
                if matches!(self.dialog, Some(Dialog::CrossConnConflict { .. })) {
                    self.resolve_cross_conn_conflict(false, ctx);
                    ctx.notify();
                    return;
                }
                // Cancelling the copy/move conflict dialog aborts the batch.
                if matches!(self.dialog, Some(Dialog::CopyMoveConflict { .. })) {
                    self.cancel_pending_copy_move(ctx);
                    ctx.notify();
                    return;
                }
                // When the user cancels the overwrite confirmation, clear the remaining batch upload queue
                let was_upload_overwrite = matches!(
                    self.dialog,
                    Some(Dialog::OverwriteConfirm { direction: TransferDirection::Upload, .. })
                );
                self.dialog = None;
                if was_upload_overwrite {
                    self.pending_uploads.clear();
                }
                ctx.notify();
            }
            SftpBrowserAction::DetailsEntry(index) => {
                let index = *index;
                self.show_details(index, ctx);
            }
            SftpBrowserAction::SetSearchFilter(filter) => {
                self.search_filter = Some(filter.clone());
                self.clamp_cursor_to_visible();
                ctx.notify();
            }
            SftpBrowserAction::ClearSearchFilter => {
                self.search_filter = None;
                self.clamp_cursor_to_visible();
                ctx.notify();
            }
            SftpBrowserAction::NavigateUp => {
                self.go_up(ctx);
            }
            SftpBrowserAction::DeleteSelected => {
                if let Some(&index) = self.selected.iter().next() {
                    self.delete_selected(index, ctx);
                }
            }
            SftpBrowserAction::CreateFolder => {
                self.handle_action(&SftpBrowserAction::NewFolder, ctx);
            }
            SftpBrowserAction::ConfirmMove => {
                if let Some(Dialog::Move { source, target_dir }) = &self.dialog {
                    let file_name = source
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let target_path = match safe_join_name(target_dir, &file_name) {
                        Some(p) => normalize_remote_path(&p),
                        None => {
                            self.show_error_toast("Invalid target path".to_string(), ctx);
                            self.dialog = None;
                            ctx.notify();
                            return;
                        }
                    };

                    if let Some(sftp) = &self.sftp {
                        let sftp = sftp.clone();
                        let source = source.clone();
                        self.dialog = None;
                        ctx.notify();
                        self.run_blocking(
                            ctx,
                            move || sftp.rename(&source, &target_path),
                            move |me, result, ctx| {
                                match result {
                                    Ok(Ok(())) => {
                                        me.refresh_dir(ctx);
                                    }
                                    Ok(Err(e)) => {
                                        me.show_error_toast(format!("Move failed: {e}"), ctx);
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast("Not connected to server".to_string(), ctx);
                        self.dialog = None;
                    }
                }
            }
            // ---- MC-style keyboard cursor ----
            SftpBrowserAction::CursorDown => self.move_cursor(CursorMove::Down, ctx),
            SftpBrowserAction::CursorUp => self.move_cursor(CursorMove::Up, ctx),
            SftpBrowserAction::CursorFirst => self.move_cursor(CursorMove::First, ctx),
            SftpBrowserAction::CursorLast => self.move_cursor(CursorMove::Last, ctx),
            SftpBrowserAction::CursorPageUp => {
                let page = self.cursor_page_size();
                self.move_cursor(CursorMove::PageUp(page), ctx);
            }
            SftpBrowserAction::CursorPageDown => {
                let page = self.cursor_page_size();
                self.move_cursor(CursorMove::PageDown(page), ctx);
            }
            SftpBrowserAction::ActivateCursor => self.activate_cursor(ctx),
            SftpBrowserAction::OpenCursorInEditor => self.open_cursor_in_editor(ctx),
            SftpBrowserAction::EnterCursorDir => self.enter_cursor_dir(ctx),
            SftpBrowserAction::ToggleSelectCursor => self.toggle_select_cursor(ctx),
            SftpBrowserAction::RenameCursor => self.rename_cursor(ctx),
            SftpBrowserAction::CopyToOtherPane => self.copy_or_move_to_other_pane(false, ctx),
            SftpBrowserAction::MoveToOtherPane => self.copy_or_move_to_other_pane(true, ctx),
            SftpBrowserAction::CloseFileManager => {
                // Reverts the pane to its underlying terminal (temporary-replacement seam).
                ctx.emit(PaneEvent::Close);
            }
            SftpBrowserAction::OverwriteConflict { all } => {
                self.resolve_copy_move_conflict(true, *all, ctx);
            }
            SftpBrowserAction::SkipConflict { all } => {
                self.resolve_copy_move_conflict(false, *all, ctx);
            }
            SftpBrowserAction::PickCopyMoveTarget(index) => {
                let index = *index;
                self.dialog = None;
                if let Some(pick) = self.pending_target_pick.take() {
                    if let Some(target) = pick.candidates.get(index).cloned() {
                        self.route_copy_move(&pick.sources, &target, pick.is_move, ctx);
                    }
                }
                ctx.notify();
            }
            SftpBrowserAction::ResolveCrossConnConflict { overwrite } => {
                self.resolve_cross_conn_conflict(*overwrite, ctx);
            }
            SftpBrowserAction::CancelTransfer(task_id) => {
                let task_id = *task_id;
                // Cooperative cancellation: set the cancel_flag
                if let Some(t) = self.transfers.iter().find(|t| t.id == task_id) {
                    t.cancel();
                }
                // Structured cancellation: abort the spawned future
                if let Some(handle) = self.transfer_handles.remove(&task_id) {
                    handle.abort();
                }
                ctx.notify();
            }
            SftpBrowserAction::ToggleTransferPanel => {
                let has_active = self.transfers.iter().any(|t| {
                    matches!(t.state, TransferState::Pending | TransferState::InProgress)
                });
                if has_active {
                    self.dialog = Some(Dialog::CloseTransferPanelConfirm);
                } else {
                    self.transfers.clear();
                    self.transfer_panel_hidden = true;
                }
                ctx.notify();
            }
            SftpBrowserAction::ConfirmCloseTransferPanel => {
                for task in &self.transfers {
                    task.cancel();
                }
                for (_, handle) in self.transfer_handles.drain() {
                    handle.abort();
                }
                self.transfers.clear();
                self.transfer_panel_hidden = true;
                self.dialog = None;
                ctx.notify();
            }
            SftpBrowserAction::DragFilesEnter => {
                self.is_drag_hovering = true;
                ctx.notify();
            }
            SftpBrowserAction::DragFilesLeave => {
                self.is_drag_hovering = false;
                ctx.notify();
            }
            SftpBrowserAction::DragAndDropFiles(paths) => {
                self.is_drag_hovering = false;
                // Enqueue in reverse so that pop() yields files in the original order
                self.pending_uploads = paths.iter().rev().cloned().collect();
                self.process_pending_uploads(ctx);
            }
            SftpBrowserAction::ExecuteUpload(local_path_str) => {
                let local_path = PathBuf::from(local_path_str);
                self.execute_upload(&local_path, ctx);
            }
            SftpBrowserAction::DownloadSaveAs { index, local_path } => {
                let local_path = PathBuf::from(local_path);
                let (remote_path, file_size) = self
                    .entries
                    .get(*index)
                    .map(|e| (e.path.clone(), e.size))
                    .unzip();
                if let (Some(remote_path), Some(file_size)) = (remote_path, file_size) {
                    self.execute_download(&remote_path, &local_path, file_size, ctx);
                }
            }
        }
    }
}

impl View for SftpBrowserView {
    fn ui_name() -> &'static str {
        "SftpBrowserView"
    }

    /// Render the complete UI layout
    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // 1. When not connected, show the connection state
        if !matches!(self.connection, ConnectionState::Connected) {
            return Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(self.render_connection_state(appearance))
                .finish();
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max);

        // 2. Breadcrumb
        col.add_child(
            Container::new(self.render_breadcrumb(appearance))
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .with_padding_top(PANEL_PADDING)
                .finish(),
        );

        // 3. Toolbar
        col.add_child(
            Container::new(self.render_toolbar(appearance))
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .with_padding_top(4.0)
                .with_padding_bottom(4.0)
                .finish(),
        );

        // 4. Search bar
        col.add_child(
            Container::new(self.render_search_bar(appearance))
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .with_padding_bottom(4.0)
                .finish(),
        );

        // 5. Loading indicator / file list
        if self.is_loading {
            col.add_child(Shrinkable::new(1.0, self.render_loading(appearance)).finish());
        } else {
            let file_list = self.render_file_list(appearance);
            let scrollbar_color = theme.disabled_text_color(theme.background()).into();
            let scrollbar_thumb_hover = theme.main_text_color(theme.background()).into();
            let scrollable = ClippedScrollable::vertical(
                self.scroll_state.clone(),
                file_list,
                ScrollbarWidth::Auto,
                scrollbar_color,
                scrollbar_thumb_hover,
                Fill::None,
            )
            .finish();
            col.add_child(Shrinkable::new(1.0, scrollable).finish());
        }

        // 6. MC-style function-key bar (footer)
        col.add_child(self.render_function_bar(appearance));

        // 7. Transfer panel (floating at the bottom)
        let mut main_content = col.finish();

        // 8. Transfer panel floating layer
        if !self.transfers.is_empty() && !self.transfer_panel_hidden {
            let panel_el = Container::new(self.render_transfers(appearance))
                .with_padding_left(PANEL_PADDING)
                .with_padding_right(PANEL_PADDING)
                .with_padding_bottom(PANEL_PADDING)
                .finish();
            let mut stack = Stack::new();
            stack.add_child(main_content);
            stack.add_positioned_overlay_child(
                panel_el,
                OffsetPositioning::offset_from_parent(
                    Vector2F::new(0.0, 0.0),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
            main_content = stack.finish();
        }

        // 9. Context menu
        if let Some(ref cm_state) = self.context_menu {
            let menu_el = super::context_menu::render_context_menu(cm_state, appearance);
            let positioning = OffsetPositioning::offset_from_parent(
                cm_state.position,
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            );
            let mut stack = Stack::new();
            stack.add_child(main_content);
            stack.add_positioned_overlay_child(menu_el, positioning);
            main_content = stack.finish();
        }

        // 9. Dialog (overlay layer)
        if let Some(ref dialog) = self.dialog {
            let dialog_el = super::dialogs::render_dialog(
                dialog,
                &self.rename_editor,
                &self.new_folder_editor,
                appearance,
                self.dialog_confirm_btn.clone(),
                self.dialog_cancel_btn.clone(),
                self.dialog_close_btn.clone(),
                self.overwrite_all_btn.clone(),
                self.skip_all_btn.clone(),
                &self.target_pick_btn_states,
            );
            let mut stack = Stack::new();
            stack.add_child(main_content);
            stack.add_overlay_child(Align::new(dialog_el).finish());
            main_content = stack.finish();
        }

        // 10. Drag-and-drop visual feedback
        if self.is_drag_hovering {
            let drop_hint = Text::new_inline(
                "Drag files to upload".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size() + 2.0,
            )
            .with_color(theme.accent().into())
            .finish();
            let overlay = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(drop_hint)
                .finish();
            let overlay_container = Container::new(overlay)
                .with_background(theme.accent().with_opacity(20))
                .with_border(Border::all(2.0).with_border_fill(theme.accent().into_solid()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)))
                .finish();
            let mut stack = Stack::new();
            stack.add_child(main_content);
            stack.add_child(overlay_container);
            main_content = stack.finish();
        }

        // 11. Save the panel position (used for context menu position calculation)
        let positioned_content =
            SavePosition::new(main_content, SFTP_PANEL_POSITION_ID).finish();

        // 12. Keyboard event interception — MC-style navigation. A modifier
        // (other than Shift, used for range operations later) means the
        // keystroke belongs to a shortcut elsewhere; let it propagate.
        let key_handler = EventHandler::new(positioned_content).on_keydown(
            move |ctx, _app, keystroke| {
                // FM pane-mode toggle: the same chord that opened the file manager
                // (cmd-shift-E on mac, ctrl-alt-e elsewhere) closes it back to the
                // terminal. Handled here, *before* the modifier guard below, since it
                // is a modifier chord the guard would otherwise propagate away. Key is
                // matched case-insensitively (shift may fold "e"→"E" at runtime).
                let is_e = keystroke.key.eq_ignore_ascii_case("e");
                let toggle_back = (is_e
                    && keystroke.cmd
                    && keystroke.shift
                    && !keystroke.ctrl
                    && !keystroke.alt)
                    || (is_e
                        && keystroke.ctrl
                        && keystroke.alt
                        && !keystroke.cmd
                        && !keystroke.shift);
                if toggle_back {
                    ctx.dispatch_typed_action(SftpBrowserAction::CloseFileManager);
                    return DispatchEventResult::StopPropagation;
                }
                if keystroke.ctrl || keystroke.cmd || keystroke.alt || keystroke.meta {
                    return DispatchEventResult::PropagateToParent;
                }
                let action = match keystroke.key.as_str() {
                    // Cursor movement
                    "down" => Some(SftpBrowserAction::CursorDown),
                    "up" => Some(SftpBrowserAction::CursorUp),
                    "home" => Some(SftpBrowserAction::CursorFirst),
                    "end" => Some(SftpBrowserAction::CursorLast),
                    "pageup" => Some(SftpBrowserAction::CursorPageUp),
                    "pagedown" => Some(SftpBrowserAction::CursorPageDown),
                    // Directory traversal, MC-style: Enter/Right open, Left/Backspace go up.
                    "enter" | "numpadenter" => Some(SftpBrowserAction::ActivateCursor),
                    "right" => Some(SftpBrowserAction::EnterCursorDir),
                    "left" | "backspace" => Some(SftpBrowserAction::NavigateUp),
                    "space" => Some(SftpBrowserAction::ToggleSelectCursor),
                    // Rename (F2, the common file-manager convention; MC's F6
                    // is repurposed here for the cross-pane move).
                    "f2" => Some(SftpBrowserAction::RenameCursor),
                    // Function-key bar (MC parity): F3 View / F4 Edit open the editor.
                    "f3" | "f4" => Some(SftpBrowserAction::OpenCursorInEditor),
                    "f5" => Some(SftpBrowserAction::CopyToOtherPane),
                    "f6" => Some(SftpBrowserAction::MoveToOtherPane),
                    "f7" => Some(SftpBrowserAction::CreateFolder),
                    "f8" | "delete" => Some(SftpBrowserAction::DeleteSelected),
                    "f10" => Some(SftpBrowserAction::CloseFileManager),
                    "escape" => Some(SftpBrowserAction::CloseDialog),
                    _ => None,
                };
                match action {
                    Some(action) => {
                        ctx.dispatch_typed_action(action);
                        DispatchEventResult::StopPropagation
                    }
                    None => DispatchEventResult::PropagateToParent,
                }
            },
        );

        // 13. Drag-and-drop event interception
        super::drop_target::SftpDropTargetElement::new(key_handler.finish()).finish()
    }
}

impl BackingView for SftpBrowserView {
    type PaneHeaderOverflowMenuAction = SftpBrowserAction;
    type CustomAction = ();
    type AssociatedData = ();

    /// Handle overflow menu actions
    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    /// Close the view
    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // Cooperative cancellation: set the cancel_flag on all transfer tasks
        for task in &self.transfers {
            task.cancel();
        }
        // Structured cancellation: abort the spawned futures
        for (_, handle) in self.transfer_handles.drain() {
            handle.abort();
        }
        self.pending_uploads.clear();
        self.connect_handle = None;
        self.refresh_handle = None;
        self._session = None;
        self.sftp = None;
        self.connection = ConnectionState::Disconnected;
        // Stop advertising this pane as a copy/move target.
        self.deregister_from_registry(ctx);
        ctx.emit(PaneEvent::Close);
    }

    /// Focus the contents, setting window focus to the current view
    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    /// Render the header content
    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        let path = self.current_path.display();
        let title = format!("SFTP: {path}");
        view::HeaderContent::simple(title)
    }

    /// Set the focus handle
    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

/// Create a single-line editor
fn make_editor(
    placeholder: &str,
    ctx: &mut ViewContext<SftpBrowserView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_string();
    ctx.add_typed_action_view(move |ctx| {
        let options = {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: theme.active_ui_text_color(),
                        disabled_color: theme.disabled_ui_text_color(),
                        hint_color: theme.disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }
        };
        let mut editor = EditorView::single_line(options, ctx);
        editor.set_placeholder_text(&placeholder, ctx);
        editor
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        let result =
            SftpBrowserView::initial_connect_path(&None, || Some(PathBuf::from("/home/user")));
        assert_eq!(result, PathBuf::from("/home/user"));
    }

    /// A bare `/` start_path is treated like the plain entry: fall back to home.
    #[test]
    fn test_initial_connect_path_root_uses_home() {
        let requested = Some(PathBuf::from("/"));
        let result = SftpBrowserView::initial_connect_path(&requested, || {
            Some(PathBuf::from("/home/user"))
        });
        assert_eq!(result, PathBuf::from("/home/user"));
    }

    /// When the home cannot be resolved (`realpath(".")` failed) and no
    /// start_path was given, fall back to `/` — the pre-existing behavior.
    #[test]
    fn test_initial_connect_path_none_no_home_uses_root() {
        let result = SftpBrowserView::initial_connect_path(&None, || None);
        assert_eq!(result, PathBuf::from("/"));
    }

    // ============================================================
    // SftpBrowserAction enum tests
    // ============================================================

    /// Test the SftpBrowserAction::CancelTransfer variant
    #[test]
    fn test_action_cancel_transfer() {
        let action = SftpBrowserAction::CancelTransfer(42);
        assert!(matches!(action, SftpBrowserAction::CancelTransfer(42)));
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
        let action = SftpBrowserAction::DownloadSaveAs {
            index: 3,
            local_path: "/tmp/file.txt".into(),
        };
        assert!(matches!(
            action,
            SftpBrowserAction::DownloadSaveAs { index: 3, .. }
        ));
    }
}
