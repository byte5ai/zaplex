//! SFTP browser main view
//!
//! Implements the `BackingView` trait and serves as the pane's core view component.
//! Provides full functionality for remote file browsing, upload/download, and directory navigation.
//! author: logic
//! date: 2026-05-26

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::editor::{
    EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_ssh_manager::{KeychainSecretStore, SshRepository};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, Element,
    EventHandler, Expanded, Fill, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    SavePosition, ScrollbarWidth, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Stack,
    Text,
};
use warpui::platform::{Cursor, FilePickerConfiguration, SaveFilePickerConfiguration};
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};
use zaplex_remote_session::types::{has_feature, FEATURE_SAFE_FILE_TRANSACTIONS_V1};

use super::context_menu::ContextMenuState;
use super::fm_registry::{
    plan_transfer, FileManagerRegistry, FmPaneDescriptor, FsNamespace, TransferKind,
};
use super::keynav::{apply_cursor_move, clamp_cursor, CursorMove};
use super::sftp_backend::{LiveSftpBackend, SafeFileClientSlot, SftpBackend};
use super::sftp_ops;
use super::sftp_ops::normalize_remote_path;
use super::types::{
    ConnectionState, Dialog, EntryIdentity, EntryReference, FileEntry, FileEntryType,
    TransferDirection, TransferState, TransferTask,
};

/// The function-key bar: `(key label, action)`. Rendered as a footer and
/// mirroring the physical F-keys handled in `on_keydown`, so the same verbs
/// are reachable by keyboard (pros) and click (beginners). F5/F6 cross-pane
/// copy/move light up fully once a second file panel exists. Captions come
/// from [`function_bar_caption`] — localized, not the raw English the bar
/// used to carry in an otherwise localized app (polish audit FM.3).
const FUNCTION_BAR: &[(&str, fn() -> SftpBrowserAction)] = &[
    ("F2", || SftpBrowserAction::RenameCursor),
    ("F3", || SftpBrowserAction::ViewCursorDetails),
    ("F4", || SftpBrowserAction::OpenCursorInEditor),
    ("F5", || SftpBrowserAction::CopyToOtherPane),
    ("F6", || SftpBrowserAction::MoveToOtherPane),
    ("F7", || SftpBrowserAction::CreateFolder),
    ("F8", || SftpBrowserAction::DeleteSelected),
    ("F10", || SftpBrowserAction::CloseFileManager),
];

fn function_key_action(key: &str) -> Option<SftpBrowserAction> {
    FUNCTION_BAR
        .iter()
        .find(|(label, _)| label.eq_ignore_ascii_case(key))
        .map(|(_, action)| action())
}

fn pane_cycle_action(key: &str, shift: bool) -> Option<crate::pane_group::PaneGroupAction> {
    match (key, shift) {
        ("tab", true) => Some(crate::pane_group::PaneGroupAction::NavigatePrev),
        ("tab", false) => Some(crate::pane_group::PaneGroupAction::NavigateNext),
        _ => None,
    }
}

/// Localized caption for a [`FUNCTION_BAR`] key.
fn function_bar_caption(key: &str) -> String {
    match key {
        "F2" => crate::t!("fm-key-rename"),
        "F3" => crate::t!("fm-key-view"),
        "F4" => crate::t!("fm-key-edit"),
        "F5" => crate::t!("fm-key-copy"),
        "F6" => crate::t!("fm-key-move"),
        "F7" => crate::t!("fm-key-mkdir"),
        "F8" => crate::t!("fm-key-delete"),
        _ => crate::t!("fm-key-quit"),
    }
}

/// A pane-local legend must never compete with file rows for horizontal space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionLegendMode {
    Full,
    Compact,
    Hidden,
}

impl FunctionLegendMode {
    fn position_suffix(self) -> &'static str {
        match self {
            Self::Full => "legend-full",
            Self::Compact => "legend-compact",
            Self::Hidden => "legend-hidden",
        }
    }
}

const FUNCTION_LEGEND_KEYCAP_MIN_WIDTH: f32 = 44.0;
// The widest localized caption is German "Verschieben". Reserve the keycap,
// cell padding, caption, and inter-cell spacing before showing captions.
const FUNCTION_LEGEND_CAPTION_MIN_WIDTH: f32 = 128.0;
const FUNCTION_LEGEND_HORIZONTAL_PADDING: f32 = PANEL_PADDING * 2.0;

fn function_legend_mode(pane_width: f32) -> FunctionLegendMode {
    let (compact_width, full_width) = function_legend_widths();

    if pane_width >= full_width {
        FunctionLegendMode::Full
    } else if pane_width >= compact_width {
        FunctionLegendMode::Compact
    } else {
        FunctionLegendMode::Hidden
    }
}

fn function_legend_widths() -> (f32, f32) {
    (
        FUNCTION_BAR.len() as f32 * FUNCTION_LEGEND_KEYCAP_MIN_WIDTH
            + FUNCTION_LEGEND_HORIZONTAL_PADDING,
        FUNCTION_BAR.len() as f32 * FUNCTION_LEGEND_CAPTION_MIN_WIDTH
            + FUNCTION_LEGEND_HORIZONTAL_PADDING,
    )
}

fn save_layout_position(child: Box<dyn Element>, position_id: &str) -> Box<dyn Element> {
    let mut stack = Stack::new();
    stack.add_child(
        SavePosition::new(child, position_id)
            .for_single_frame()
            .finish(),
    );
    stack.finish()
}

/// A sortable list column (MC's Name/Size/Modified ordering).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
}

/// Which column orders the list and in which direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState {
    pub column: SortColumn,
    pub ascending: bool,
}

impl Default for SortState {
    fn default() -> Self {
        // Name ascending — the order the list has always loaded in.
        Self {
            column: SortColumn::Name,
            ascending: true,
        }
    }
}

impl SortState {
    /// Click on a header: the same column flips direction, a new column starts
    /// ascending (descending first for Size/Modified, where "biggest" and
    /// "newest" are what one actually looks for).
    fn toggled(self, column: SortColumn) -> Self {
        if self.column == column {
            Self {
                column,
                ascending: !self.ascending,
            }
        } else {
            Self {
                column,
                ascending: matches!(column, SortColumn::Name),
            }
        }
    }
}

/// Toolbar button size
const TOOLBAR_BTN_SIZE: f32 = 28.0;
/// Toolbar icon size
const TOOLBAR_ICON_SIZE: f32 = 16.0;
/// Toolbar spacing
const TOOLBAR_SPACING: f32 = 4.0;
/// Panel inner padding
const PANEL_PADDING: f32 = 8.0;

fn render_toolbar_button(
    icon: Icon,
    handle: MouseStateHandle,
    action: SftpBrowserAction,
    appearance: &Appearance,
    position_id: &'static str,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_color = theme.sub_text_color(theme.background());
    let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
        .with_width(TOOLBAR_ICON_SIZE)
        .with_height(TOOLBAR_ICON_SIZE)
        .finish();
    let button = Hoverable::new(handle, move |_| {
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

    SavePosition::new(button, position_id).finish()
}

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
    /// Select the referenced entry if that exact object is still listed.
    SelectEntry(EntryReference),
    /// Toggle the multi-selection mark on the referenced entry — the
    /// mouse counterpart of Insert/Space.
    ToggleMark(EntryReference),
    /// Mark the row under the cursor and advance one row (MC's Insert).
    MarkAndAdvance,
    /// Order the list by this column (or flip the direction if it already is).
    SortBy(SortColumn),
    /// Show / hide dot-files.
    ToggleHidden,
    /// Open the referenced entry (enter if a directory, download if a file)
    OpenEntry(EntryReference),
    /// Delete the referenced entry
    DeleteEntry(EntryReference),
    /// Rename the referenced entry
    RenameEntry(EntryReference),
    /// Download the referenced entry
    DownloadEntry(EntryReference),
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
        entry: EntryReference,
        position: Vector2F,
    },
    /// Close the context menu
    CloseContextMenu,
    /// Close the dialog
    CloseDialog,
    /// View entry details
    DetailsEntry(EntryReference),
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
    /// Execute a save-as download (the user has chosen a path). The remote
    /// identity is captured before the native picker opens, because the
    /// listing may refresh or reorder while that picker is visible.
    DownloadSaveAs {
        entry: EntryReference,
        resolved_target_size: Option<u64>,
        local_path: String,
    },
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
    /// Show metadata for the row under the cursor (F3 View).
    ViewCursorDetails,
    /// Open the row under the cursor in the code editor (F4 Edit): a directory
    /// is entered; a file opens in an editor pane.
    OpenCursorInEditor,
    /// Enter the directory under the cursor (Right arrow); no-op on a file.
    EnterCursorDir,
    /// Toggle the row under the cursor in the multi-selection (Space).
    ToggleSelectCursor,
    /// Pick-mode only (#105): return the current directory to the spawn card via
    /// `WorkspaceAction::RemoteSpawnDirPicked` and close the picker.
    PickCurrentDir,
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
    /// Resolve a copy/move conflict by choosing an available sibling name.
    RenameConflict { all: bool },
    /// Resolve a copy/move conflict only when the source is provably newer.
    NewerOnlyConflict { all: bool },
    /// Destination picker: copy/move into the candidate pane at this index
    /// (index into `pending_target_pick.candidates`).
    PickCopyMoveTarget(usize),
    /// Resolve the cross-connection overwrite prompt: `overwrite` transfers the
    /// conflicting files (overwriting), otherwise they are skipped.
    ResolveCrossConnConflict { overwrite: bool },
    /// Cancel a transfer task
    CancelTransfer(usize, Option<u64>),
    /// Pause a transfer task.
    PauseTransfer(usize, Option<u64>),
    /// Resume a paused transfer task.
    ResumeTransfer(usize, Option<u64>),
    /// Retry a retained, identity-checked cleanup action.
    RetryTransferRecovery(usize),
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
    /// `None` = ask on each conflict; `Some` applies that policy to the rest.
    conflict_default: Option<super::transfer_job::ConflictDecision>,
    queued: usize,
    skipped: usize,
}

pub(super) fn copy_move_submission_summary(queued: usize, skipped: usize, target: &str) -> String {
    let mut parts = vec![crate::t!("fm-summary-queued", count = queued)];
    if skipped > 0 {
        parts.push(crate::t!("fm-summary-skipped", count = skipped));
    }
    crate::t!(
        "fm-summary-to-target",
        parts = parts.join(", "),
        target = target.to_string()
    )
}

fn queue_state_for_outcome(
    outcome: &super::transfer_job::TransferOutcome,
) -> super::transfer_queue::QueuedTransferState {
    match outcome {
        super::transfer_job::TransferOutcome::Completed => {
            super::transfer_queue::QueuedTransferState::Completed
        }
        super::transfer_job::TransferOutcome::PartiallyCompleted {
            transferred,
            published,
            skipped,
            source_kept,
        } => super::transfer_queue::QueuedTransferState::PartiallyCompleted {
            transferred: *transferred,
            published: *published,
            skipped: *skipped,
            source_kept: *source_kept,
        },
        super::transfer_job::TransferOutcome::Skipped => {
            super::transfer_queue::QueuedTransferState::Skipped
        }
    }
}

pub(super) fn partial_transfer_message(
    transferred: usize,
    published: usize,
    skipped: usize,
    source_kept: bool,
) -> String {
    if source_kept {
        crate::t!(
            "fm-toast-partial-source-kept",
            transferred = transferred.to_string(),
            published = published.to_string(),
            skipped = skipped.to_string()
        )
    } else {
        crate::t!(
            "fm-toast-partial",
            transferred = transferred.to_string(),
            published = published.to_string(),
            skipped = skipped.to_string()
        )
    }
}

pub(super) fn transfer_display_priority(task: &TransferTask) -> (u8, std::cmp::Reverse<usize>) {
    let priority = if !task.recovery_paths.is_empty() {
        0
    } else if matches!(
        task.state,
        TransferState::Pending
            | TransferState::InProgress
            | TransferState::Paused
            | TransferState::Cancelling
    ) {
        1
    } else {
        2
    };
    (priority, std::cmp::Reverse(task.id))
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
    /// Original path to restore on an incomplete move. `None` is used only by
    /// focused cleanup tests that seed an already-in-place source.
    original_source_dir: Option<PathBuf>,
    /// Human label for the completion toast, e.g. `sub → host:/var/www`.
    label: String,
}

struct QuarantinedDirSource {
    backend: Arc<dyn SftpBackend>,
    original: PathBuf,
    quarantine: PathBuf,
    restore_on_drop: bool,
}

impl QuarantinedDirSource {
    fn new(
        backend: Arc<dyn SftpBackend>,
        original: PathBuf,
    ) -> Result<Self, sftp_ops::SftpOpsError> {
        let quarantine = unique_backend_sibling(&original, "zaplex_move");
        backend.rename(&original, &quarantine)?;
        Ok(Self {
            backend,
            original,
            quarantine,
            restore_on_drop: true,
        })
    }

    fn disarm(mut self) -> PathBuf {
        self.restore_on_drop = false;
        self.quarantine.clone()
    }

    fn restore(mut self) -> Result<(), (sftp_ops::SftpOpsError, PathBuf)> {
        match self.backend.rename(&self.quarantine, &self.original) {
            Ok(()) => {
                self.restore_on_drop = false;
                Ok(())
            }
            Err(error) => {
                self.restore_on_drop = false;
                Err((error, self.quarantine.clone()))
            }
        }
    }
}

impl Drop for QuarantinedDirSource {
    fn drop(&mut self) {
        if self.restore_on_drop {
            if let Err(error) = self.backend.rename(&self.quarantine, &self.original) {
                log::error!(
                    "fm move recovery required: source remains at {} because restoring {} failed: {error}",
                    self.quarantine.display(),
                    self.original.display()
                );
            }
        }
    }
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

/// State that becomes current only after the requested directory has been
/// listed successfully. Keeping this candidate separate prevents a failed or
/// superseded request from desynchronizing the breadcrumb/title from the rows.
struct NavigationCommit {
    path: PathBuf,
    history: Vec<PathBuf>,
    history_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymlinkActivationIntent {
    Open,
    OpenInEditor,
    EnterDirectoryOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedSymlinkAction {
    Navigate,
    OpenFile,
    Ignore,
    Unsupported,
}

/// Decide what an already-resolved symlink target means for the user's input.
/// Resolution itself is fallible; a broken link never reaches this function.
fn action_for_symlink_target(
    target_type: FileEntryType,
    intent: SymlinkActivationIntent,
) -> ResolvedSymlinkAction {
    match target_type {
        FileEntryType::Directory => ResolvedSymlinkAction::Navigate,
        FileEntryType::File => match intent {
            SymlinkActivationIntent::Open | SymlinkActivationIntent::OpenInEditor => {
                ResolvedSymlinkAction::OpenFile
            }
            SymlinkActivationIntent::EnterDirectoryOnly => ResolvedSymlinkAction::Ignore,
        },
        // Both production backends define `stat` as following links. Seeing a
        // link here therefore means the backend could not resolve its target;
        // do not guess that it is a directory.
        FileEntryType::Other | FileEntryType::Symlink => ResolvedSymlinkAction::Unsupported,
    }
}

fn symlink_target_unresolved_message() -> String {
    crate::t!("fm-toast-symlink-target-unresolved")
}

fn resolved_download_size(entry_size: u64, resolved_target_size: Option<u64>) -> u64 {
    resolved_target_size.unwrap_or(entry_size)
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
    /// Current descriptor-bound daemon capability for this registry node.
    safe_file_client: SafeFileClientSlot,
    // ---- Navigation ----
    /// Current path
    pub(crate) current_path: PathBuf,
    /// File entries in the current directory
    pub(crate) entries: Vec<FileEntry>,
    /// Set of selected filesystem objects. Indices are never persisted because
    /// a refresh may reuse an index for a different object.
    pub(crate) selected: HashSet<EntryIdentity>,
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
    /// Root breadcrumb button
    root_breadcrumb_btn: MouseStateHandle,
    /// Persistent click state for each non-root breadcrumb segment.
    breadcrumb_mouse_handles: HashMap<PathBuf, MouseStateHandle>,
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
    /// Scroll position for the bounded global transfer history.
    transfer_panel_scroll_state: ClippedScrollStateHandle,
    /// Persistent cancel-button state keyed by transfer task id.
    transfer_cancel_handles: HashMap<usize, MouseStateHandle>,
    /// Persistent pause/resume-button state keyed by transfer task id.
    transfer_pause_handles: HashMap<usize, MouseStateHandle>,
    /// Global queue job keyed by the pane-local transfer id.
    transfer_queue_ids: HashMap<usize, super::transfer_queue::TransferId>,
    transfer_progress_poll_handle: Option<SpawnedFutureHandle>,
    // ---- Dialog editors ----
    /// Rename editor
    pub(crate) rename_editor: ViewHandle<EditorView>,
    /// New folder editor
    pub(crate) new_folder_editor: ViewHandle<EditorView>,
    /// Search filter editor
    search_editor: ViewHandle<EditorView>,
    // ---- Keyboard cursor (MC-style) ----
    /// Position of the keyboard cursor within the *visible row* list, MC-style
    /// — that is, over [`Self::row_count`] rows: the `..` parent row (when the
    /// current directory has one) followed by the filtered, sorted entries.
    /// Distinct from `selected` (the multi-selection set). Always clamped into
    /// range after any change to the visible list.
    pub(crate) cursor: usize,
    // ---- List view state (MC parity) ----
    /// Which column the list is ordered by, and in which direction. Applied in
    /// [`Self::visible_indices`] — a *view* concern, so `entries` (and every
    /// index into it: `selected`, the mouse handles) stays stable when the
    /// user re-sorts.
    pub(crate) sort: SortState,
    /// Whether dot-files are listed. Off by default: the file manager opens on
    /// the useful contents of a directory, and `~` full of tool dot-dirs was
    /// exactly the "no overview" complaint. Toggled from the toolbar.
    pub(crate) show_hidden: bool,
    /// Hover/click state per row **mark zone** (the leading icon cell, which
    /// toggles the multi-selection), keyed by path so a refresh cannot reuse
    /// an in-flight click for a different entry.
    mark_handles: HashMap<PathBuf, MouseStateHandle>,
    /// Hover/click state per sortable column header.
    header_handles: HashMap<SortColumn, MouseStateHandle>,
    /// Hover state of the show-hidden toolbar toggle.
    hidden_btn: MouseStateHandle,
    /// Set when a *navigation* (not a refresh) has changed the directory, so
    /// the next listing parks the cursor on the first real entry rather than
    /// on `..`. MC starts on `..`; here the first thing under the cursor
    /// should be something F3/F5/F8 can act on, and `..` stays one Up away.
    cursor_reset_pending: bool,
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
    rename_conflict_btn: MouseStateHandle,
    newer_only_conflict_btn: MouseStateHandle,
    rename_all_btn: MouseStateHandle,
    newer_only_all_btn: MouseStateHandle,
    /// One mouse-state handle per row of the target-picker dialog (resized when
    /// the picker opens), so each pane row hovers/presses independently.
    target_pick_btn_states: Vec<MouseStateHandle>,
    // ---- File row mouse handles ----
    /// Mouse state for each entry path. Path identity prevents a mouse-up after
    /// refresh from completing a click on a replacement at the same row index.
    row_mouse_handles: HashMap<PathBuf, MouseStateHandle>,
    /// Mouse state handle for the virtual `..` parent row. PERSISTENT, like
    /// every other click target's: a `MouseStateHandle::default()` minted per
    /// render records the mouse-down in a state the next render throws away,
    /// so the mouse-up never finds it and the click never fires — the dead
    /// `..` row of the RC acceptance 2026-07-21 (same failure as the T2.3
    /// rescan button).
    parent_row_handle: MouseStateHandle,
    /// Open briefly after an *upward* navigation: row clicks aimed at the old
    /// layout are swallowed until it closes. The `..` row navigates on a
    /// SINGLE click, so the second click of a habitual double click is still
    /// in flight when the rows swap — on a fast backend it lands on the new
    /// listing, and at the root boundary (no `..` row there) the first entry
    /// sits in exactly those pixels, so the stray click would open or mark a
    /// row the user never aimed at. Only `NavigateUp` arms this: every other
    /// row navigates on double click (no tail click exists), and breadcrumb /
    /// toolbar targets are not over the rows. 250 ms is far below any
    /// deliberate see-then-click reaction on a fresh listing.
    suppress_row_clicks_until: Option<std::time::Instant>,
    // ---- Scrolling ----
    /// Scroll state handle
    scroll_state: ClippedScrollStateHandle,
    // ---- Async tasks ----
    /// Future handle for the current connection task
    connect_handle: Option<SpawnedFutureHandle>,
    /// Future handle for the current directory refresh
    refresh_handle: Option<SpawnedFutureHandle>,
    /// Monotonic id bumped on every `refresh_dir`. A refresh's completion only
    /// installs its listing if this id still matches — so a slow list of the old
    /// directory can't land *after* the user has navigated away and overwrite the
    /// new listing (header says one directory, rows are another → open/delete hit
    /// the wrong file).
    pub(crate) refresh_generation: u64,
    /// Mapping from transfer task ID to its future handle
    transfer_handles: HashMap<usize, SpawnedFutureHandle>,
    /// Pending queue for batched drag-and-drop uploads
    pending_uploads: Vec<PathBuf>,
    /// Cross-connection directory **moves** awaiting their file transfers so the
    /// source tree can be removed once (and only if) all files land.
    pending_dir_move_cleanups: Vec<PendingDirMoveCleanup>,
    /// Monotonic id handed to each cross-connection directory-move batch.
    next_dir_move_batch_id: u64,
    /// When true this browser was opened as a directory *picker* (the spawn
    /// card's "Browse…", #105): a pick bar's "Use this folder" returns
    /// `current_path` via `WorkspaceAction::RemoteSpawnDirPicked` and closes.
    pick_mode: bool,
    /// Hover state for the pick bar's "Use this folder" button.
    pick_btn: MouseStateHandle,
    /// Set once a directory was actually picked, so `close()` doesn't also fire
    /// the cancel (which re-shows the card) after a successful pick (#105).
    pick_resolved: bool,
}

/// The display name of an SSH registry node, for the tab title.
///
/// Read from the registry rather than carried in: the browser is opened from
/// several places, and a name threaded through each of them is a name that can
/// be threaded through wrong. `None` when the node is gone or the registry can't
/// be read — the caller then shows a generic title instead of an empty one.
fn host_name_for_node(node_id: &str) -> Option<String> {
    if node_id.is_empty() {
        return None;
    }
    warp_ssh_manager::with_conn(|c| Ok(warp_ssh_manager::SshRepository::list_nodes(c)?))
        .ok()?
        .into_iter()
        .find(|n| n.id == node_id)
        // A blank name is not a title. The editor rejects one, but the
        // repository takes any string, so an entry written another way could
        // hand back "" — and `Some("")` would beat the fallback and leave an
        // empty tab.
        .map(|n| n.name)
        .filter(|n| !n.trim().is_empty())
}

impl SftpBrowserView {
    fn layout_position_id(&self, part: &str) -> String {
        format!("sftp_layout:{}:{part}", self.fm_id)
    }

    #[cfg(test)]
    pub(crate) fn layout_position_id_for_test(&self, part: &str) -> String {
        self.layout_position_id(part)
    }

    /// Keep one mouse state per visible breadcrumb target. Reusing these
    /// handles across renders preserves a mouse-down until the matching
    /// mouse-up arrives.
    fn sync_breadcrumb_mouse_handles(&mut self) {
        let mut accumulated = PathBuf::new();
        let mut visible = HashSet::new();
        for component in self
            .current_path
            .components()
            .filter(|component| !matches!(component, Component::RootDir))
        {
            accumulated.push(component);
            visible.insert(accumulated.clone());
            self.breadcrumb_mouse_handles
                .entry(accumulated.clone())
                .or_default();
        }
        self.breadcrumb_mouse_handles
            .retain(|path, _| visible.contains(path));
    }

    /// Create a new SFTP browser view, opened at `start_path` (the remote
    /// shell's cwd) when known, else the host root `/`.
    pub fn new(node_id: String, start_path: Option<PathBuf>, ctx: &mut ViewContext<Self>) -> Self {
        // The tab keeps saying which HOST this is; that it happens to be showing
        // files rather than a shell is the pane's own obvious business (spec v3
        // FM). Titling it "File Manager" dropped the host entirely — with two
        // browsers open you could not tell which machine either was on, which is
        // the one thing a tab title has to answer here.
        //
        // A host whose registry entry has gone (deleted while open) falls back to
        // the generic title rather than a blank tab.
        let title = host_name_for_node(&node_id)
            .unwrap_or_else(|| crate::t!("sftp-file-manager-title").to_string());
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(title));
        let rename_editor = make_editor(&crate::t!("fm-rename-placeholder"), ctx);
        let new_folder_editor = make_editor(&crate::t!("fm-folder-name-placeholder"), ctx);
        let search_editor = make_editor(&crate::t!("fm-search-placeholder"), ctx);

        let mut me = Self {
            node_id,
            pane_configuration,
            focus_handle: None,
            connection: ConnectionState::Disconnected,
            _session: None,
            sftp: None,
            safe_file_client: SafeFileClientSlot::default(),
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
            root_breadcrumb_btn: MouseStateHandle::default(),
            breadcrumb_mouse_handles: HashMap::new(),
            dialog_confirm_btn: MouseStateHandle::default(),
            dialog_cancel_btn: MouseStateHandle::default(),
            dialog_close_btn: MouseStateHandle::default(),
            transfer_panel_hidden: false,
            transfer_panel_close_btn: MouseStateHandle::default(),
            transfer_panel_scroll_state: ClippedScrollStateHandle::default(),
            transfer_cancel_handles: HashMap::new(),
            transfer_pause_handles: HashMap::new(),
            transfer_queue_ids: HashMap::new(),
            transfer_progress_poll_handle: None,
            rename_editor,
            new_folder_editor,
            search_editor,
            cursor: 0,
            sort: SortState::default(),
            show_hidden: false,
            mark_handles: HashMap::new(),
            header_handles: [SortColumn::Name, SortColumn::Size, SortColumn::Modified]
                .into_iter()
                .map(|c| (c, MouseStateHandle::default()))
                .collect(),
            hidden_btn: MouseStateHandle::default(),
            cursor_reset_pending: true,
            fn_bar_handles: FUNCTION_BAR
                .iter()
                .map(|_| MouseStateHandle::default())
                .collect(),
            fm_id: super::fm_registry::next_fm_id(),
            pending_copy_move: None,
            pending_target_pick: None,
            pending_cross_conn: None,
            overwrite_all_btn: MouseStateHandle::default(),
            skip_all_btn: MouseStateHandle::default(),
            rename_conflict_btn: MouseStateHandle::default(),
            newer_only_conflict_btn: MouseStateHandle::default(),
            rename_all_btn: MouseStateHandle::default(),
            newer_only_all_btn: MouseStateHandle::default(),
            target_pick_btn_states: Vec::new(),
            row_mouse_handles: HashMap::new(),
            parent_row_handle: MouseStateHandle::default(),
            suppress_row_clicks_until: None,
            scroll_state: ClippedScrollStateHandle::default(),
            connect_handle: None,
            refresh_handle: None,
            refresh_generation: 0,
            transfer_handles: HashMap::new(),
            pending_uploads: Vec::new(),
            pending_dir_move_cleanups: Vec::new(),
            next_dir_move_batch_id: 1,
            pick_mode: false,
            pick_btn: MouseStateHandle::default(),
            pick_resolved: false,
        };
        me.sync_breadcrumb_mouse_handles();

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

        ctx.observe(
            &super::transfer_queue::TransferQueue::handle(ctx),
            |me, _, ctx| {
                me.sync_transfer_tracking_with_queue(ctx);
                me.schedule_transfer_progress_poll(ctx);
                ctx.notify();
            },
        );
        me.schedule_transfer_progress_poll(ctx);

        ctx.subscribe(
            &RemoteServerManager::handle(ctx),
            |me, _manager, event, ctx| {
                if matches!(
                    event,
                    RemoteServerManagerEvent::SessionConnected { .. }
                        | RemoteServerManagerEvent::SessionReconnected { .. }
                        | RemoteServerManagerEvent::SessionDisconnected { .. }
                        | RemoteServerManagerEvent::SessionDeregistered { .. }
                ) {
                    me.refresh_safe_file_client(ctx);
                }
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
                    // A narrower/wider filter changes the visible set — the
                    // cursor must snap back into range, and marks must not
                    // survive out of view (they are what F5/F6/F8 act on).
                    me.retain_marks_to_visible();
                    ctx.notify();
                }
            },
        );

        // Initiate the connection
        me.connect_to_server(ctx);

        me
    }

    /// Mark this browser as a directory *picker* opened from the spawn card's
    /// "Browse…" (#105): it shows a pick bar whose "Use this folder" returns the
    /// current directory and closes.
    pub fn with_pick_mode(mut self) -> Self {
        self.pick_mode = true;
        self
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
        me.pane_configuration = ctx
            .add_model(|_ctx| PaneConfiguration::new(crate::t!("sftp-local-file-manager-title")));
        let backend = Arc::new(crate::sftp_manager::sftp_backend::InMemorySftpBackend::new(
            std::path::PathBuf::from("/"),
        )) as Arc<dyn SftpBackend>;
        me.connection = ConnectionState::Connected;
        me.sftp = Some(backend);
        me.current_path = start_path.clone();
        me.path_history = vec![start_path];
        me.history_index = 0;
        me.sync_breadcrumb_mouse_handles();
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
        self.sync_breadcrumb_mouse_handles();
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
        self.sync_breadcrumb_mouse_handles();
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
        let title = crate::t!("fm-title-sftp", path = path.to_string());
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
    pub fn selected(&self) -> &HashSet<EntryIdentity> {
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
                                        me.refresh_safe_file_client(ctx);
                                        let backend =
                                            Arc::new(LiveSftpBackend::new_with_safe_file_slot(
                                                sftp,
                                                me.safe_file_client.clone(),
                                            ))
                                                as Arc<dyn SftpBackend>;
                                        super::transfer_queue::TransferQueue::as_ref(ctx)
                                            .register_backend_recoveries_async(backend.clone());
                                        me._session = Some(session);
                                        // Land on the caller's requested `start_path`
                                        // (the FM pane-mode toggle's cwd) when given;
                                        // the plain "SFTP Browse" (`None`) falls back
                                        // to the remote home, then `/`.
                                        me.apply_connected_backend(backend, ctx);
                                    }
                                    Err(e) => {
                                        let error = super::sftp_ops::SftpOpsError::from(e);
                                        me.connection = ConnectionState::Failed(crate::t!(
                                            "fm-toast-sftp-channel-failed",
                                            err = error.user_message()
                                        ));
                                        me.show_error_toast(
                                            crate::t!(
                                                "fm-toast-sftp-channel-failed",
                                                err = error.user_message()
                                            ),
                                            ctx,
                                        );
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                let message = e.user_message();
                                me.connection = ConnectionState::Failed(message.clone());
                                me.show_error_toast(message, ctx);
                            }
                            Err(_) => {
                                // JoinError (aborted or panicked)
                                me.connection = ConnectionState::Failed(crate::t!(
                                    "fm-status-connection-cancelled"
                                ));
                            }
                        }
                        ctx.notify();
                    },
                );
            }
            Ok(None) => {
                self.connection =
                    ConnectionState::Failed(crate::t!("fm-status-server-config-not-found"));
                self.show_error_toast(crate::t!("fm-status-server-config-not-found"), ctx);
                ctx.notify();
            }
            Err(e) => {
                log::warn!("Failed to read SFTP server configuration: {e}");
                let message = crate::t!("fm-toast-server-config-read-failed");
                self.connection = ConnectionState::Failed(message.clone());
                self.show_error_toast(message, ctx);
                ctx.notify();
            }
        }
    }

    fn refresh_safe_file_client(&mut self, ctx: &AppContext) {
        let client = RemoteServerManager::as_ref(ctx)
            .connected_daemons()
            .into_iter()
            .find(|daemon| {
                daemon.registry_node_id.as_deref() == Some(self.node_id.as_str())
                    && has_feature(&daemon.features, FEATURE_SAFE_FILE_TRANSACTIONS_V1)
            })
            .map(|daemon| daemon.client);
        if self.safe_file_client.set(client) {
            if let Some(backend) = self.sftp.clone() {
                super::transfer_queue::TransferQueue::as_ref(ctx)
                    .register_backend_recoveries_async(backend);
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
        self.sync_breadcrumb_mouse_handles();
        self.path_history = vec![self.current_path.clone()];
        self.history_index = 0;
        // A freshly connected listing parks the cursor on the first real entry
        // once the listing lands (see `cursor_reset_pending`).
        self.cursor = 0;
        self.cursor_reset_pending = true;
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
        callback: impl FnOnce(&mut Self, Result<T, tokio::task::JoinError>, &mut ViewContext<Self>)
            + 'static,
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
        self.load_directory(self.current_path.clone(), None, ctx);
    }

    /// Load `path`, optionally committing a navigation only after its listing
    /// succeeds. The current view remains a coherent, fully actionable snapshot
    /// while the request is in flight and after any error.
    fn load_directory(
        &mut self,
        path: PathBuf,
        navigation: Option<NavigationCommit>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Close any open context menu the moment a refresh is initiated —
        // navigation (Backspace/Left) or a post-op refresh changes the listing
        // under it, and its raw entry index would otherwise still address the
        // *old* directory's file (delete/rename the wrong one). Doing it here —
        // before the connection check and regardless of success/failure/
        // supersession — closes the stale-index window entirely.
        self.context_menu = None;

        let sftp = match &self.sftp {
            Some(s) => s.clone(),
            None => {
                self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
                ctx.notify();
                return;
            }
        };

        self.is_loading = true;
        ctx.notify();

        // Supersede any in-flight refresh: bump the generation so its late
        // completion is discarded, and abort its task to stop wasting a worker.
        // Dropping the handle does *not* cancel it (`SpawnedFutureHandle` has no
        // cancel-on-drop), so the generation check in `on_dir_listed` is the
        // correctness backstop; the abort is only an optimization.
        if let Some(handle) = &self.refresh_handle {
            handle.abort();
        }
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let generation = self.refresh_generation;

        self.refresh_handle = self.run_blocking(
            ctx,
            move || sftp.list_dir(&path),
            move |me, result, ctx| {
                me.on_dir_listed_with_navigation(generation, navigation, result, ctx)
            },
        );
    }

    /// Install the listing produced by the refresh identified by `generation`.
    ///
    /// Discarded if a newer refresh has since superseded this one (the user
    /// navigated away, or a second refresh started): letting a slow listing of
    /// the *old* directory land would leave the header naming one directory while
    /// the rows are another — and every index-addressed verb (open, download,
    /// delete) would then hit the wrong file. The newer refresh owns the view and
    /// the `is_loading` flag, so a stale result is dropped untouched.
    pub(crate) fn on_dir_listed(
        &mut self,
        generation: u64,
        result: Result<
            Result<Vec<FileEntry>, super::sftp_ops::SftpOpsError>,
            tokio::task::JoinError,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        self.on_dir_listed_with_navigation(generation, None, result, ctx);
    }

    fn on_dir_listed_with_navigation(
        &mut self,
        generation: u64,
        navigation: Option<NavigationCommit>,
        result: Result<
            Result<Vec<FileEntry>, super::sftp_ops::SftpOpsError>,
            tokio::task::JoinError,
        >,
        ctx: &mut ViewContext<Self>,
    ) {
        if generation != self.refresh_generation {
            return;
        }
        self.is_loading = false;
        let installed = match result {
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
                // Marks survive a refresh only when the backend reports the
                // exact same object and revision. A reused path is not enough.
                let marked_entries = if navigation.is_none() {
                    self.selected.clone()
                } else {
                    HashSet::new()
                };
                if let Some(navigation) = navigation {
                    self.current_path = navigation.path;
                    self.sync_breadcrumb_mouse_handles();
                    self.path_history = navigation.history;
                    self.history_index = navigation.history_index;
                    self.cursor_reset_pending = true;
                }
                self.entries = entries;
                let listed_identities: HashSet<EntryIdentity> =
                    self.entries.iter().map(FileEntry::entry_identity).collect();
                self.selected = marked_entries
                    .intersection(&listed_identities)
                    .cloned()
                    .collect();
                // (Any open context menu was already closed when this refresh
                // started — see `refresh_dir` — so a stale entry index can't be
                // actioned against the new listing.)
                self.sync_row_mouse_handles();
                // A refresh may have shrunk the listing (files removed); keep the
                // cursor in range rather than pointing past it.
                self.clamp_cursor_to_visible();
                true
            }
            Ok(Err(e)) => {
                self.show_error_toast(
                    crate::t!("fm-toast-list-dir-failed", err = e.user_message()),
                    ctx,
                );
                false
            }
            Err(_) => false,
        };

        if installed {
            let path = self.current_path.display();
            let title = crate::t!("fm-title-sftp", path = path.to_string());
            self.pane_configuration.update(ctx, |config, ctx| {
                config.set_title(title, ctx);
            });
            self.publish_to_registry(ctx);
        }
        ctx.notify();
    }

    /// Rebuild row mouse state for each accepted listing generation.
    ///
    /// Ordinary rerenders keep these handles, but a refresh invalidates every
    /// pending press. A path may disappear and be recreated as a different
    /// object while the refresh is in flight; reusing its old handle would let
    /// the original mouse-up act on that replacement.
    fn sync_row_mouse_handles(&mut self) {
        self.row_mouse_handles.clear();
        self.mark_handles.clear();
        for path in self.entries.iter().map(|entry| entry.path.clone()) {
            self.row_mouse_handles.entry(path.clone()).or_default();
            self.mark_handles.entry(path).or_default();
        }
    }

    /// Entry indices currently visible (after the search filter), in display
    /// order. The single source of truth for both rendering and cursor
    /// arithmetic, so the MC cursor and the rows never disagree.
    pub(crate) fn visible_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                // Dot-files only when the user asked for them.
                if !self.show_hidden && entry.name.starts_with('.') {
                    return false;
                }
                self.search_filter.as_ref().map_or(true, |filter| {
                    entry.name.to_lowercase().contains(&filter.to_lowercase())
                })
            })
            .map(|(i, _)| i)
            .collect();

        // Sorting lives here, on the *view's* index list, rather than by
        // re-ordering `entries`: `selected` and the per-row mouse handles are
        // keyed by entry index, so shuffling the entries themselves would
        // silently move the user's marks onto other files.
        //
        // Directories always lead, in both directions — MC's rule, and the one
        // that makes a re-sort still navigable.
        let sort = self.sort;
        indices.sort_by(|&a, &b| {
            let (ea, eb) = (&self.entries[a], &self.entries[b]);
            let dir_a = matches!(ea.file_type, FileEntryType::Directory);
            let dir_b = matches!(eb.file_type, FileEntryType::Directory);
            if dir_a != dir_b {
                return if dir_a {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            let ordering = match sort.column {
                SortColumn::Name => ea.name.to_lowercase().cmp(&eb.name.to_lowercase()),
                // Directories carry no meaningful size; keep them by name so
                // the leading block stays readable when sorting by size.
                SortColumn::Size if dir_a => ea.name.to_lowercase().cmp(&eb.name.to_lowercase()),
                SortColumn::Size => ea.size.cmp(&eb.size),
                // `modified` is a preformatted string; entries without one sort
                // last. Ties fall back to the name so the order is total (a
                // non-total comparator would render in an arbitrary order).
                SortColumn::Modified => match (&ea.modified, &eb.modified) {
                    (Some(x), Some(y)) => x.cmp(y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| ea.name.to_lowercase().cmp(&eb.name.to_lowercase())),
            };
            if sort.ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
        indices
    }

    /// Whether the list shows a leading `..` row — i.e. whether the current
    /// directory has a parent to go up to (MC always offers this row; a click
    /// target in the list is how one navigates up without hunting for the
    /// toolbar arrow).
    pub(crate) fn has_parent_row(&self) -> bool {
        self.current_path.parent().is_some()
    }

    /// Number of navigable rows: the `..` row (if any) plus the visible
    /// entries. The keyboard cursor indexes THIS space.
    pub(crate) fn row_count(&self) -> usize {
        self.visible_indices().len() + usize::from(self.has_parent_row())
    }

    /// Whether the cursor currently rests on the `..` row.
    pub(crate) fn cursor_on_parent_row(&self) -> bool {
        self.has_parent_row() && self.cursor == 0
    }

    /// The entry index under the keyboard cursor, or `None` when the cursor is
    /// on the `..` row (or nothing is visible).
    ///
    /// Returning `None` for the parent row is what keeps `..` out of every
    /// destructive verb: delete, rename, copy, move and download all resolve
    /// their target through here, so none of them can address it.
    pub(crate) fn cursor_entry_index(&self) -> Option<usize> {
        if self.cursor_on_parent_row() {
            return None;
        }
        let offset = usize::from(self.has_parent_row());
        self.visible_indices().get(self.cursor - offset).copied()
    }

    pub(crate) fn entry_reference(&self, index: usize) -> Option<EntryReference> {
        self.entries
            .get(index)
            .map(|entry| entry.entry_reference(self.refresh_generation))
    }

    fn resolve_entry_reference(&self, reference: &EntryReference) -> Option<usize> {
        if reference.listing_generation > self.refresh_generation {
            return None;
        }
        self.entries
            .iter()
            .position(|entry| entry.entry_identity() == reference.identity)
    }

    pub(crate) fn is_index_marked(&self, index: usize) -> bool {
        self.entries
            .get(index)
            .is_some_and(|entry| self.selected.contains(&entry.entry_identity()))
    }

    fn toggle_mark_index(&mut self, index: usize) {
        let Some(identity) = self.entries.get(index).map(FileEntry::entry_identity) else {
            return;
        };
        if !self.selected.insert(identity.clone()) {
            self.selected.remove(&identity);
        }
    }

    #[cfg(test)]
    pub(crate) fn mark_index_for_test(&mut self, index: usize) {
        if let Some(identity) = self.entries.get(index).map(FileEntry::entry_identity) {
            self.selected.insert(identity);
        }
    }

    /// Drop marks on rows the list no longer shows, and re-clamp the cursor.
    ///
    /// Marks are the target set of F5/F6/F8 (`operation_sources`), so a mark
    /// that survives out of view is a file the user can copy, move or DELETE
    /// without seeing it. Anything that narrows the visible set — the search
    /// filter, the hidden-files toggle — must come through here.
    fn retain_marks_to_visible(&mut self) {
        let visible: HashSet<EntryIdentity> = self
            .visible_indices()
            .into_iter()
            .filter_map(|index| self.entries.get(index).map(FileEntry::entry_identity))
            .collect();
        self.selected.retain(|identity| visible.contains(identity));
        self.clamp_cursor_to_visible();
    }

    /// Re-clamp the cursor into the current visible range. Call after any
    /// change to the entries or the filter so a stale cursor can't point past
    /// the end.
    ///
    /// When a navigation is pending, the cursor also *lands* here: on the
    /// first real entry of the new listing, so the row under the cursor is one
    /// the verbs can act on. A plain refresh never moves it — the user's
    /// position in the list is theirs to keep.
    fn clamp_cursor_to_visible(&mut self) {
        if self.cursor_reset_pending {
            self.cursor_reset_pending = false;
            // First entry row: past `..` when there is one, unless the
            // directory has no listable entries at all.
            self.cursor = if self.has_parent_row() && !self.visible_indices().is_empty() {
                1
            } else {
                0
            };
        }
        self.cursor = clamp_cursor(self.cursor, self.row_count());
    }

    /// True while the post-`NavigateUp` stray-click window is open. Row-level
    /// mouse actions that *do* something (open, mark) check this; pure cursor
    /// moves need not.
    fn row_clicks_suppressed(&self) -> bool {
        self.suppress_row_clicks_until
            .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// Test-only fast-forward past the stray-click window — tests dispatch
    /// clicks faster than any human could.
    #[cfg(test)]
    pub(crate) fn expire_click_guard(&mut self) {
        self.suppress_row_clicks_until = None;
    }

    /// Fixed page size for PageUp/PageDown. The model doesn't know the
    /// viewport height, so a sensible constant keeps paging predictable.
    fn cursor_page_size(&self) -> usize {
        15
    }

    /// Apply an MC cursor movement.
    fn move_cursor(&mut self, mv: CursorMove, ctx: &mut ViewContext<Self>) {
        let len = self.row_count();
        self.cursor = apply_cursor_move(self.cursor, len, mv);
        ctx.notify();
    }

    /// Activate the row under the cursor: go up on `..`, enter a directory, or
    /// open a file.
    fn activate_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if self.cursor_on_parent_row() {
            self.go_up(ctx);
            return;
        }
        if let Some(index) = self.cursor_entry_index() {
            self.open_entry(index, ctx);
        }
    }

    /// Resolve a symlink target without changing the current location, then
    /// perform the action appropriate for the resolved file type. `stat`
    /// follows links; destructive operations continue to use the lstat-style
    /// type from the directory listing and never call this helper.
    fn resolve_and_activate_symlink(
        &mut self,
        index: usize,
        intent: SymlinkActivationIntent,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let path = entry.path.clone();
        let sftp = match &self.sftp {
            Some(sftp) => sftp.clone(),
            None => {
                self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
                return;
            }
        };
        let stat_path = path.clone();
        let _ = self.run_blocking(
            ctx,
            move || sftp.stat(&stat_path),
            move |me, result, ctx| {
                // A stat response can arrive after a refresh/navigation. Raw
                // indices are not stable across listings, so act only if the
                // same link is still at the same index.
                let link_is_current = me.entries.get(index).is_some_and(|entry| {
                    entry.path == path && entry.file_type == FileEntryType::Symlink
                });
                if !link_is_current {
                    return;
                }

                match result {
                    Ok(Ok(target)) => match action_for_symlink_target(target.file_type, intent) {
                        ResolvedSymlinkAction::Navigate => me.navigate_to(path, ctx),
                        ResolvedSymlinkAction::OpenFile => match intent {
                            SymlinkActivationIntent::Open => {
                                me.begin_download(index, Some(target.size), ctx)
                            }
                            SymlinkActivationIntent::OpenInEditor => {
                                ctx.dispatch_typed_action(
                                    &crate::WorkspaceAction::OpenFileInEditor {
                                        node_id: me.node_id.clone(),
                                        path,
                                    },
                                );
                            }
                            SymlinkActivationIntent::EnterDirectoryOnly => {}
                        },
                        ResolvedSymlinkAction::Ignore => {}
                        ResolvedSymlinkAction::Unsupported => {
                            me.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx)
                        }
                    },
                    Ok(Err(error)) => {
                        log::warn!("Failed to resolve SFTP symlink target: {error}");
                        me.show_error_toast(symlink_target_unresolved_message(), ctx);
                    }
                    Err(error) => {
                        log::warn!("SFTP symlink target resolution task failed: {error}");
                        me.show_error_toast(symlink_target_unresolved_message(), ctx);
                    }
                }
            },
        );
    }

    /// F4: open the row under the cursor in the code editor. A directory is
    /// entered; a file is opened in an editor pane. The workspace resolves
    /// local vs remote (`node_id`).
    fn open_cursor_in_editor(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(index) = self.cursor_entry_index() else {
            return;
        };
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        match entry.file_type {
            FileEntryType::Directory => {
                let path = entry.path.clone();
                self.navigate_to(path, ctx);
            }
            FileEntryType::Symlink => {
                self.resolve_and_activate_symlink(
                    index,
                    SymlinkActivationIntent::OpenInEditor,
                    ctx,
                );
            }
            FileEntryType::File => {
                ctx.dispatch_typed_action(&crate::WorkspaceAction::OpenFileInEditor {
                    node_id: self.node_id.clone(),
                    path: entry.path.clone(),
                });
            }
            FileEntryType::Other => {
                self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
            }
        }
    }

    /// F3: view the stable entry currently under the cursor without entering
    /// the editable code path used by F4.
    fn view_cursor_details(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            self.show_details(index, ctx);
        }
    }

    /// Enter the directory under the cursor (Right arrow); a file is a no-op.
    fn enter_cursor_dir(&mut self, ctx: &mut ViewContext<Self>) {
        if self.cursor_on_parent_row() {
            self.go_up(ctx);
            return;
        }
        if let Some(index) = self.cursor_entry_index() {
            if let Some(entry) = self.entries.get(index) {
                match entry.file_type {
                    FileEntryType::Directory => self.navigate_to(entry.path.clone(), ctx),
                    FileEntryType::Symlink => self.resolve_and_activate_symlink(
                        index,
                        SymlinkActivationIntent::EnterDirectoryOnly,
                        ctx,
                    ),
                    FileEntryType::File | FileEntryType::Other => {}
                }
            }
        }
    }

    /// Toggle the row under the cursor in the multi-selection (Space).
    fn toggle_select_cursor(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            self.toggle_mark_index(index);
            ctx.notify();
        }
    }

    /// MC's Insert: mark the row under the cursor, then step down one row, so
    /// a run of files can be marked by holding the key. On the last row the
    /// cursor stays put (movement never wraps).
    fn mark_and_advance(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.cursor_entry_index() {
            self.toggle_mark_index(index);
        }
        // Advance even from the `..` row: Insert walks the list either way.
        self.cursor = apply_cursor_move(self.cursor, self.row_count(), CursorMove::Down);
        ctx.notify();
    }

    /// Total byte size of the marked entries — the number MC puts in its
    /// selection status line. Directories contribute nothing (their size is
    /// unknown without walking them).
    pub(crate) fn marked_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| self.selected.contains(&entry.entry_identity()))
            .filter(|entry| !matches!(entry.file_type, FileEntryType::Directory))
            .map(|entry| entry.size)
            .sum()
    }

    /// Rename the row under the cursor (F2).
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
            crate::t!("fm-label-local")
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
            self.entries
                .iter()
                .filter(|entry| self.selected.contains(&entry.entry_identity()))
                .map(|entry| entry.path.clone())
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
        let sources = self.operation_sources();
        if sources.is_empty() {
            let msg = if is_move {
                crate::t!("fm-toast-nothing-to-move")
            } else {
                crate::t!("fm-toast-nothing-to-copy")
            };
            self.show_error_toast(msg, ctx);
            return;
        }

        let self_id = self.fm_id;
        let candidates = FileManagerRegistry::as_ref(ctx).others(self_id);
        match candidates.as_slice() {
            [] => {
                let msg = if is_move {
                    crate::t!("fm-toast-open-second-pane-move")
                } else {
                    crate::t!("fm-toast-open-second-pane-copy")
                };
                self.show_error_toast(msg, ctx);
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

    /// Copy/move between two different remote hosts as one bounded-memory stream.
    /// A move removes its source only after the target has been published,
    /// verified, and the source identity has been revalidated.
    fn relay_remote_to_remote(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(source_backend) = self.sftp.clone() else {
            self.show_error_toast(crate::t!("fm-toast-not-connected"), ctx);
            return;
        };
        let Some(target_backend) = FileManagerRegistry::as_ref(ctx).backend_for(target.id) else {
            self.show_error_toast(crate::t!("fm-toast-other-pane-disconnected"), ctx);
            return;
        };
        let target_dir = target.current_path.clone();
        let target_label = target.label.clone();
        let mut started = 0usize;
        for source in sources {
            let Some(name) = source.file_name() else {
                continue;
            };
            let Some(source_type) = self
                .entries
                .iter()
                .find(|e| &e.path == source)
                .map(|entry| entry.file_type)
            else {
                continue;
            };
            let is_dir = match source_type {
                FileEntryType::Directory => true,
                FileEntryType::File => false,
                FileEntryType::Symlink | FileEntryType::Other => {
                    self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
                    continue;
                }
            };
            let dest = normalize_remote_path(&target_dir.join(name));
            let src_backend = source_backend.clone();
            let tgt_backend = target_backend.clone();
            let source_path = source.clone();
            let label = format!("{} → {}", name.to_string_lossy(), target_label);
            self.spawn_backend_queue_job(
                src_backend,
                tgt_backend,
                source_path,
                dest,
                is_dir,
                is_move,
                if is_dir {
                    super::transfer_job::ConflictDecision::MergeSkip
                } else {
                    super::transfer_job::ConflictDecision::Skip
                },
                true,
                label,
                ctx,
            );
            started += 1;
        }
        if started > 0 {
            let msg = if is_move {
                crate::t!(
                    "fm-toast-relaying-move",
                    count = started,
                    target = target_label.clone()
                )
            } else {
                crate::t!(
                    "fm-toast-relaying-copy",
                    count = started,
                    target = target_label.clone()
                )
            };
            self.show_info_toast(msg, ctx);
        }
        self.selected.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_backend_queue_job(
        &mut self,
        source_backend: Arc<dyn SftpBackend>,
        target_backend: Arc<dyn SftpBackend>,
        source_path: PathBuf,
        target_path: PathBuf,
        is_dir: bool,
        is_move: bool,
        conflict: super::transfer_job::ConflictDecision,
        is_relay: bool,
        label: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let total = source_backend
            .lstat(&source_path)
            .map(|entry| entry.size)
            .unwrap_or(0);
        let Some(next_task_id) = self.next_transfer_id.checked_add(1) else {
            self.show_error_toast(
                crate::t!(
                    "fm-toast-transfer-failed",
                    err = "transfer task ID exhausted"
                ),
                ctx,
            );
            return;
        };
        let task_id = self.next_transfer_id;
        let workspace_id = ctx.window_id().to_string();
        let operation = if is_move {
            super::transfer_job::TransferOperation::Move
        } else {
            super::transfer_job::TransferOperation::Copy
        };
        let topology = if is_relay {
            super::transfer_queue::TransferTopology::RemoteRelay
        } else {
            super::transfer_queue::TransferTopology::SameFilesystem
        };
        let queue_id = super::transfer_queue::TransferQueue::handle(ctx).update(ctx, {
            let source = source_path.clone();
            let target = target_path.clone();
            move |queue, ctx| {
                let id = queue.enqueue_job_with_audit(
                    workspace_id,
                    source,
                    target,
                    TransferDirection::Copy,
                    operation,
                    conflict,
                    topology,
                    total,
                );
                ctx.notify();
                id
            }
        });
        let queue_id = match queue_id {
            Ok(queue_id) => queue_id,
            Err(error) => {
                self.show_error_toast(
                    crate::t!("fm-toast-transfer-failed", err = error.user_message()),
                    ctx,
                );
                return;
            }
        };
        self.next_transfer_id = next_task_id;
        self.transfers.push(TransferTask::new(
            task_id,
            source_path.clone(),
            target_path.clone(),
            TransferDirection::Copy,
            total,
        ));
        self.transfer_cancel_handles
            .insert(task_id, MouseStateHandle::default());
        self.transfer_pause_handles
            .insert(task_id, MouseStateHandle::default());
        self.transfer_queue_ids.insert(task_id, queue_id);
        let activity = super::transfer_queue::TransferQueue::as_ref(ctx)
            .activity_handle(queue_id)
            .expect("new relay queue activity must exist");
        let control = activity.control();
        let job = super::transfer_job::TransferJob {
            source_backend,
            target_backend,
            source_path,
            target_path,
            operation: if is_move {
                super::transfer_job::TransferOperation::Move
            } else {
                super::transfer_job::TransferOperation::Copy
            },
            conflict,
        };
        let worker_activity = activity.clone();
        if let Some(handle) = self.run_blocking(
            ctx,
            move || {
                let mut on_progress = |progress| worker_activity.update_progress(progress);
                let result = if is_dir {
                    super::transfer_job::run_directory_transfer(
                        &job,
                        &control,
                        Some(&mut on_progress),
                    )
                } else {
                    super::transfer_job::run_transfer(&job, &control, Some(&mut on_progress))
                };
                match &result {
                    Ok(outcome) => worker_activity.set_state(queue_state_for_outcome(outcome)),
                    Err(super::sftp_ops::SftpOpsError::Cancelled) => worker_activity
                        .set_state(super::transfer_queue::QueuedTransferState::Cancelled),
                    Err(error) => worker_activity.set_error(error),
                }
                result
            },
            move |me, result, ctx| {
                me.transfer_handles.remove(&task_id);
                match result {
                    Ok(Ok(super::transfer_job::TransferOutcome::Completed)) => {
                        let message = if is_relay {
                            crate::t!("fm-toast-relayed", name = label.clone())
                        } else if is_move {
                            crate::t!("fm-toast-moved", label = label.clone())
                        } else {
                            crate::t!("fm-toast-copied", label = label.clone())
                        };
                        me.show_info_toast(message, ctx);
                        me.refresh_dir(ctx);
                    }
                    Ok(Ok(super::transfer_job::TransferOutcome::PartiallyCompleted {
                        transferred,
                        published,
                        skipped,
                        source_kept,
                    })) => {
                        me.show_info_toast(
                            partial_transfer_message(transferred, published, skipped, source_kept),
                            ctx,
                        );
                        me.refresh_dir(ctx);
                    }
                    Ok(Ok(super::transfer_job::TransferOutcome::Skipped)) => {
                        me.show_neutral_toast(
                            crate::t!(
                                "fm-toast-skipped-existing",
                                count = 1,
                                target = label.clone()
                            ),
                            ctx,
                        );
                    }
                    Ok(Err(error)) => {
                        let message = if is_relay {
                            crate::t!(
                                "fm-toast-relay-failed",
                                name = label.clone(),
                                err = error.user_message()
                            )
                        } else {
                            crate::t!("fm-toast-transfer-failed", err = error.user_message())
                        };
                        me.show_error_toast(message, ctx);
                    }
                    Err(_) => me.show_error_toast(
                        if is_relay {
                            crate::t!("fm-toast-relay-cancelled", name = label.clone())
                        } else {
                            crate::t!("fm-transfer-cancelled")
                        },
                        ctx,
                    ),
                }
            },
        ) {
            self.transfer_handles.insert(task_id, handle);
        }
    }

    /// Same-filesystem copy/move. Every item is submitted to the process-wide
    /// transfer queue after conflict resolution, including same-host SFTP
    /// operations, so progress and cancellation survive pane changes.
    fn copy_move_same_fs(
        &mut self,
        sources: &[PathBuf],
        target: &FmPaneDescriptor,
        is_move: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let backend = match &self.sftp {
            Some(b) => b.clone(),
            None => {
                self.show_error_toast(crate::t!("fm-toast-not-connected"), ctx);
                return;
            }
        };
        let target_dir = target.current_path.clone();

        // Copying into our own current directory would collide name-for-name.
        if target_dir == self.current_path {
            let msg = if is_move {
                crate::t!("fm-toast-same-folder-move")
            } else {
                crate::t!("fm-toast-same-folder-copy")
            };
            self.show_error_toast(msg, ctx);
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
            conflict_default: None,
            queued: 0,
            skipped: 0,
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
            Conflict {
                name: String,
                remaining: usize,
                is_move: bool,
            },
            Skip,
            Execute {
                conflict: super::transfer_job::ConflictDecision,
            },
            ProbeError(super::sftp_ops::SftpOpsError),
        }
        loop {
            // Decide the next step under a short immutable borrow.
            let step = {
                let Some(pending) = self.pending_copy_move.as_ref() else {
                    return;
                };
                match pending.ops.front() {
                    None => Step::Done,
                    Some(op) => match pending.backend.entry_exists(&op.dest) {
                        Ok(false) => Step::Execute {
                            conflict: super::transfer_job::ConflictDecision::Overwrite,
                        },
                        Ok(true) => match pending.conflict_default {
                            Some(super::transfer_job::ConflictDecision::Skip) => Step::Skip,
                            Some(conflict) => Step::Execute { conflict },
                            None => Step::Conflict {
                                name: op
                                    .dest
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default(),
                                remaining: pending.ops.len(),
                                is_move: pending.is_move,
                            },
                        },
                        Err(error) => Step::ProbeError(error),
                    },
                }
            };
            match step {
                Step::Done => {
                    self.finish_pending_copy_move(ctx);
                    return;
                }
                Step::Conflict {
                    name,
                    remaining,
                    is_move,
                } => {
                    self.dialog = Some(Dialog::CopyMoveConflict {
                        name,
                        remaining,
                        is_move,
                    });
                    ctx.notify();
                    return;
                }
                Step::Skip => {
                    if let Some(p) = self.pending_copy_move.as_mut() {
                        p.ops.pop_front();
                        p.skipped += 1;
                    }
                }
                Step::Execute { conflict } => {
                    let op = self
                        .pending_copy_move
                        .as_mut()
                        .and_then(|p| p.ops.pop_front());
                    if let Some(op) = op {
                        self.execute_copy_move_op(&op, conflict, ctx);
                    }
                }
                Step::ProbeError(error) => {
                    self.pending_copy_move = None;
                    self.dialog = None;
                    self.show_error_toast(
                        crate::t!("fm-toast-transfer-failed", err = error.user_message()),
                        ctx,
                    );
                    ctx.notify();
                    return;
                }
            }
        }
    }

    /// Submit one already-resolved copy/move operation to the global queue.
    fn execute_copy_move_op(
        &mut self,
        op: &CopyMoveOp,
        conflict: super::transfer_job::ConflictDecision,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pending) = self.pending_copy_move.as_ref() else {
            return;
        };
        let backend = pending.backend.clone();
        let is_move = pending.is_move;
        let label = format!(
            "{} → {}",
            op.source
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            pending.target_label
        );
        self.spawn_backend_queue_job(
            backend.clone(),
            backend,
            op.source.clone(),
            op.dest.clone(),
            op.is_dir,
            is_move,
            conflict,
            false,
            label,
            ctx,
        );
        if let Some(pending) = self.pending_copy_move.as_mut() {
            pending.queued += 1;
        }
    }

    /// Finalise submission without claiming that asynchronous jobs completed.
    fn finish_pending_copy_move(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(pending) = self.pending_copy_move.take() else {
            return;
        };
        let summary =
            copy_move_submission_summary(pending.queued, pending.skipped, &pending.target_label);
        self.show_info_toast(summary, ctx);
        self.selected.clear();
        self.dialog = None;
        self.refresh_dir(ctx);
    }

    /// Resolve the open copy/move conflict dialog, optionally
    /// for all remaining conflicts) and resume the batch.
    fn resolve_copy_move_conflict(
        &mut self,
        conflict: super::transfer_job::ConflictDecision,
        all: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dialog = None;
        // Pop the conflicting op (at the front) under a short borrow.
        let op = {
            let Some(pending) = self.pending_copy_move.as_mut() else {
                return;
            };
            if all {
                pending.conflict_default = Some(conflict);
            }
            pending.ops.pop_front()
        };
        if let Some(op) = op {
            match conflict {
                super::transfer_job::ConflictDecision::Skip => {
                    if let Some(pending) = self.pending_copy_move.as_mut() {
                        pending.skipped += 1;
                    }
                }
                super::transfer_job::ConflictDecision::Overwrite
                | super::transfer_job::ConflictDecision::Rename
                | super::transfer_job::ConflictDecision::NewerOnly => {
                    self.execute_copy_move_op(&op, conflict, ctx);
                }
                super::transfer_job::ConflictDecision::MergeSkip => {
                    unreachable!("same-filesystem conflict dialog does not emit merge-skip")
                }
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
                        self.show_error_toast(crate::t!("fm-toast-other-pane-disconnected"), ctx);
                        return;
                    }
                }
            }
            TransferDirection::Download => match &self.sftp {
                Some(b) => b.clone(),
                None => {
                    self.show_error_toast(crate::t!("fm-toast-not-connected"), ctx);
                    return;
                }
            },
            TransferDirection::Copy => {
                unreachable!("copy direction is only used by backend-to-backend queue jobs")
            }
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
            let Some(source_type) = self
                .entries
                .iter()
                .find(|e| &e.path == source)
                .map(|entry| entry.file_type)
            else {
                continue;
            };
            let is_dir = match source_type {
                FileEntryType::Directory => true,
                FileEntryType::File => false,
                FileEntryType::Symlink | FileEntryType::Other => {
                    self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
                    continue;
                }
            };
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
                TransferDirection::Upload => backend.entry_exists(&dest),
                TransferDirection::Download => local_entry_exists(&dest),
                TransferDirection::Copy => {
                    unreachable!("copy direction is not used by upload/download preparation")
                }
            };
            let exists = match exists {
                Ok(exists) => exists,
                Err(error) => {
                    self.show_error_toast(
                        crate::t!("fm-toast-transfer-failed", err = error.user_message()),
                        ctx,
                    );
                    continue;
                }
            };
            let (local_path, remote_path) = match direction {
                TransferDirection::Upload => (source.clone(), dest),
                TransferDirection::Download => (dest, source.clone()),
                TransferDirection::Copy => {
                    unreachable!("copy direction is not used by upload/download preparation")
                }
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
                false,
                delete_after,
                None,
                ctx,
            );
            started_files += 1;
        }

        // Report what started right away.
        let mut parts = Vec::new();
        if started_files > 0 {
            parts.push(if is_move {
                crate::t!("fm-toast-files-move-started", count = started_files)
            } else {
                crate::t!("fm-toast-files-copy-started", count = started_files)
            });
        }
        if started_dirs > 0 {
            parts.push(if is_move {
                crate::t!("fm-toast-folders-move-started", count = started_dirs)
            } else {
                crate::t!("fm-toast-folders-copy-started", count = started_dirs)
            });
        }
        if !parts.is_empty() {
            self.show_info_toast(
                crate::t!(
                    "fm-summary-to-target",
                    parts = parts.join(", "),
                    target = target.label.clone()
                ),
                ctx,
            );
        }

        // Ask once about any files that already exist on the destination.
        if conflicts.is_empty() {
            if parts.is_empty() {
                let msg = if is_move {
                    crate::t!(
                        "fm-toast-nothing-to-move-target",
                        target = target.label.clone()
                    )
                } else {
                    crate::t!(
                        "fm-toast-nothing-to-copy-target",
                        target = target.label.clone()
                    )
                };
                self.show_info_toast(msg, ctx);
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
        if !overwrite {
            self.show_info_toast(
                crate::t!(
                    "fm-toast-skipped-existing",
                    count = pending.files.len(),
                    target = pending.target_label.clone()
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
                true,
                delete_after,
                None,
                ctx,
            );
        }
        self.show_info_toast(
            if pending.is_move {
                crate::t!(
                    "fm-toast-overwriting-move",
                    count = count,
                    target = pending.target_label.clone()
                )
            } else {
                crate::t!(
                    "fm-toast-overwriting-copy",
                    count = count,
                    target = pending.target_label.clone()
                )
            },
            ctx,
        );
        ctx.notify();
    }

    /// Recursively copy or move a directory through the process-wide queue.
    /// The complete source tree is validated before any target is created; a
    /// move removes the source only after every file and identity is verified.
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
        self.spawn_directory_queue_job(
            source_dir,
            dest_dir,
            direction,
            remote_backend,
            is_move,
            label,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_directory_queue_job(
        &mut self,
        source_dir: PathBuf,
        dest_dir: PathBuf,
        direction: TransferDirection,
        remote_backend: Arc<dyn SftpBackend>,
        is_move: bool,
        label: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(own_backend) = self.sftp.clone() else {
            self.show_error_toast(crate::t!("fm-toast-not-connected"), ctx);
            return;
        };
        let local_backend: Arc<dyn SftpBackend> = Arc::new(
            super::sftp_backend::InMemorySftpBackend::new(PathBuf::from("/")),
        );
        let (source_backend, target_backend) = match direction {
            TransferDirection::Upload => (own_backend, remote_backend),
            TransferDirection::Download => (remote_backend, local_backend),
            TransferDirection::Copy => {
                unreachable!("copy direction is not used by directory upload/download jobs")
            }
        };
        let Some(next_task_id) = self.next_transfer_id.checked_add(1) else {
            self.show_error_toast(
                crate::t!(
                    "fm-toast-transfer-failed",
                    err = "transfer task ID exhausted"
                ),
                ctx,
            );
            return;
        };
        let task_id = self.next_transfer_id;
        let workspace_id = ctx.window_id().to_string();
        let operation = if is_move {
            super::transfer_job::TransferOperation::Move
        } else {
            super::transfer_job::TransferOperation::Copy
        };
        let topology = match direction {
            TransferDirection::Upload => super::transfer_queue::TransferTopology::LocalToRemote,
            TransferDirection::Download => super::transfer_queue::TransferTopology::RemoteToLocal,
            TransferDirection::Copy => {
                unreachable!("copy direction is not used by directory upload/download jobs")
            }
        };
        let queue_id = super::transfer_queue::TransferQueue::handle(ctx).update(ctx, {
            let source = source_dir.clone();
            let target = dest_dir.clone();
            move |queue, ctx| {
                let id = queue.enqueue_job_with_audit(
                    workspace_id,
                    source,
                    target,
                    direction,
                    operation,
                    super::transfer_job::ConflictDecision::MergeSkip,
                    topology,
                    0,
                );
                ctx.notify();
                id
            }
        });
        let queue_id = match queue_id {
            Ok(queue_id) => queue_id,
            Err(error) => {
                self.show_error_toast(
                    crate::t!("fm-toast-transfer-failed", err = error.user_message()),
                    ctx,
                );
                return;
            }
        };
        self.next_transfer_id = next_task_id;
        self.transfers.push(TransferTask::new(
            task_id,
            source_dir.clone(),
            dest_dir.clone(),
            direction,
            0,
        ));
        self.transfer_cancel_handles
            .insert(task_id, MouseStateHandle::default());
        self.transfer_pause_handles
            .insert(task_id, MouseStateHandle::default());
        self.transfer_queue_ids.insert(task_id, queue_id);
        let activity = super::transfer_queue::TransferQueue::as_ref(ctx)
            .activity_handle(queue_id)
            .expect("new directory transfer activity must exist");
        let control = activity.control();
        let job = super::transfer_job::TransferJob {
            source_backend,
            target_backend,
            source_path: source_dir,
            target_path: dest_dir,
            operation: if is_move {
                super::transfer_job::TransferOperation::Move
            } else {
                super::transfer_job::TransferOperation::Copy
            },
            conflict: super::transfer_job::ConflictDecision::MergeSkip,
        };
        let worker_activity = activity.clone();
        if let Some(handle) = self.run_blocking(
            ctx,
            move || {
                let mut on_progress = |progress| worker_activity.update_progress(progress);
                let result = super::transfer_job::run_directory_transfer(
                    &job,
                    &control,
                    Some(&mut on_progress),
                );
                match &result {
                    Ok(outcome) => worker_activity.set_state(queue_state_for_outcome(outcome)),
                    Err(super::sftp_ops::SftpOpsError::Cancelled) => worker_activity
                        .set_state(super::transfer_queue::QueuedTransferState::Cancelled),
                    Err(error) => worker_activity.set_error(error),
                }
                result
            },
            move |me, result, ctx| {
                me.transfer_handles.remove(&task_id);
                match result {
                    Ok(Ok(super::transfer_job::TransferOutcome::Completed)) => {
                        me.show_info_toast(
                            if is_move {
                                crate::t!("fm-toast-moved", label = label.clone())
                            } else {
                                crate::t!("fm-toast-copied", label = label.clone())
                            },
                            ctx,
                        );
                        me.refresh_dir(ctx);
                    }
                    Ok(Ok(super::transfer_job::TransferOutcome::PartiallyCompleted {
                        transferred,
                        published,
                        skipped,
                        source_kept,
                    })) => {
                        me.show_info_toast(
                            partial_transfer_message(transferred, published, skipped, source_kept),
                            ctx,
                        );
                        me.refresh_dir(ctx);
                    }
                    Ok(Ok(super::transfer_job::TransferOutcome::Skipped)) => {
                        me.show_neutral_toast(
                            crate::t!(
                                "fm-toast-dir-nothing-to-transfer-existing",
                                label = label.clone(),
                                count = 1
                            ),
                            ctx,
                        );
                    }
                    Ok(Err(error)) => me.show_error_toast(
                        crate::t!(
                            "fm-toast-dir-prepare-failed",
                            label = label.clone(),
                            err = error.user_message()
                        ),
                        ctx,
                    ),
                    Err(_) => ctx.notify(),
                }
            },
        ) {
            self.transfer_handles.insert(task_id, handle);
        }
    }

    /// A file that belonged to a cross-connection directory move has finished.
    /// Count it down; when the whole batch is done, remove the source tree (only
    /// if every file succeeded — otherwise keep it and report).
    fn note_dir_move_progress(
        &mut self,
        batch_id: u64,
        succeeded: bool,
        ctx: &mut ViewContext<Self>,
    ) {
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
            let mut restore_failure = None;
            if let Some(original_source_dir) = &batch.original_source_dir {
                if let Err(error) = batch
                    .source_backend
                    .rename(&batch.source_dir, original_source_dir)
                {
                    log::error!(
                        "fm move: restoring {} to {} failed: {error}",
                        batch.source_dir.display(),
                        original_source_dir.display()
                    );
                    restore_failure = Some(error);
                }
            }
            if restore_failure.is_some() {
                self.show_error_toast(
                    crate::t!(
                        "fm-toast-dir-prepare-failed",
                        label = batch.label.clone(),
                        err = crate::t!("fm-error-recovery-required")
                    ),
                    ctx,
                );
            } else {
                self.show_error_toast(
                    crate::t!("fm-toast-move-source-kept", label = batch.label.clone()),
                    ctx,
                );
            }
        } else {
            match delete_quarantined_dir_source(&*batch.source_backend, &batch.source_dir) {
                Ok(()) => self.show_info_toast(
                    crate::t!("fm-toast-moved", label = batch.label.clone()),
                    ctx,
                ),
                Err(error) => self.show_error_toast(
                    crate::t!(
                        "fm-toast-copied-source-remove-failed",
                        label = batch.label.clone(),
                        err = error.user_message()
                    ),
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
        overwrite_destination: bool,
        delete_after: Option<(Arc<dyn SftpBackend>, PathBuf)>,
        dir_move_batch: Option<u64>,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let total_size = match direction {
            TransferDirection::Upload => {
                std::fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0)
            }
            TransferDirection::Download => backend.stat(&remote_path).map(|e| e.size).unwrap_or(0),
            TransferDirection::Copy => {
                unreachable!("copy direction is not used by individual upload/download jobs")
            }
        };
        let Some(next_task_id) = self.next_transfer_id.checked_add(1) else {
            self.show_error_toast(
                crate::t!(
                    "fm-toast-transfer-failed",
                    err = "transfer task ID exhausted"
                ),
                ctx,
            );
            return self.next_transfer_id;
        };
        let task_id = self.next_transfer_id;
        let workspace_id = ctx.window_id().to_string();
        let operation = if delete_after.is_some() {
            super::transfer_job::TransferOperation::Move
        } else {
            super::transfer_job::TransferOperation::Copy
        };
        let conflict = if overwrite_destination {
            super::transfer_job::ConflictDecision::Overwrite
        } else {
            super::transfer_job::ConflictDecision::Skip
        };
        let topology = match direction {
            TransferDirection::Upload => super::transfer_queue::TransferTopology::LocalToRemote,
            TransferDirection::Download => super::transfer_queue::TransferTopology::RemoteToLocal,
            TransferDirection::Copy => {
                unreachable!("copy direction is not used by individual upload/download jobs")
            }
        };
        let queue_id = super::transfer_queue::TransferQueue::handle(ctx).update(ctx, {
            let source_path = match direction {
                TransferDirection::Upload => local_path.clone(),
                TransferDirection::Download => remote_path.clone(),
                TransferDirection::Copy => {
                    unreachable!("copy direction is not used by individual upload/download jobs")
                }
            };
            let target_path = match direction {
                TransferDirection::Upload => remote_path.clone(),
                TransferDirection::Download => local_path.clone(),
                TransferDirection::Copy => {
                    unreachable!("copy direction is not used by individual upload/download jobs")
                }
            };
            move |queue, ctx| {
                let id = queue.enqueue_job_with_audit(
                    workspace_id,
                    source_path,
                    target_path,
                    direction,
                    operation,
                    conflict,
                    topology,
                    total_size,
                );
                ctx.notify();
                id
            }
        });
        let queue_id = match queue_id {
            Ok(queue_id) => queue_id,
            Err(error) => {
                self.show_error_toast(
                    crate::t!("fm-toast-transfer-failed", err = error.user_message()),
                    ctx,
                );
                return task_id;
            }
        };
        self.next_transfer_id = next_task_id;
        self.transfer_cancel_handles
            .insert(task_id, MouseStateHandle::default());
        self.transfer_pause_handles
            .insert(task_id, MouseStateHandle::default());
        let mut task = TransferTask::new(
            task_id,
            local_path.clone(),
            remote_path.clone(),
            direction,
            total_size,
        );
        task.state = TransferState::InProgress;
        self.transfers.push(task);
        self.transfer_panel_hidden = false;
        self.transfer_queue_ids.insert(task_id, queue_id);
        let activity = super::transfer_queue::TransferQueue::as_ref(ctx)
            .activity_handle(queue_id)
            .expect("new transfer queue activity must exist");
        let control = activity.control();
        let local_backend: Arc<dyn SftpBackend> = Arc::new(
            super::sftp_backend::InMemorySftpBackend::new(PathBuf::from("/")),
        );
        let (source_backend, target_backend, source_path, target_path) = match direction {
            TransferDirection::Upload => (
                delete_after
                    .as_ref()
                    .map(|(source, _)| source.clone())
                    .or_else(|| {
                        matches!(self.fs_namespace(), FsNamespace::Local)
                            .then(|| self.sftp.clone())
                            .flatten()
                    })
                    .unwrap_or_else(|| local_backend.clone()),
                backend,
                local_path,
                remote_path,
            ),
            TransferDirection::Download => (backend, local_backend, remote_path, local_path),
            TransferDirection::Copy => {
                unreachable!("copy direction is not used by individual upload/download jobs")
            }
        };
        let job = super::transfer_job::TransferJob {
            source_backend,
            target_backend,
            source_path,
            target_path,
            operation: if delete_after.is_some() && dir_move_batch.is_none() {
                super::transfer_job::TransferOperation::Move
            } else {
                super::transfer_job::TransferOperation::Copy
            },
            conflict,
        };
        let worker_activity = activity.clone();
        ctx.notify();
        if let Some(handle) = self.run_blocking(
            ctx,
            move || {
                let mut on_progress = |progress| worker_activity.update_progress(progress);
                let result =
                    super::transfer_job::run_transfer(&job, &control, Some(&mut on_progress));
                match &result {
                    Ok(outcome) => worker_activity.set_state(queue_state_for_outcome(outcome)),
                    Err(super::sftp_ops::SftpOpsError::Cancelled) => worker_activity
                        .set_state(super::transfer_queue::QueuedTransferState::Cancelled),
                    Err(error) => worker_activity.set_error(error),
                }
                result
            },
            move |me, result, ctx| {
                let progress = super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activity(queue_id)
                    .map(|activity| activity.progress)
                    .unwrap_or_default();
                if let Some(t) = me.transfers.iter_mut().find(|t| t.id == task_id) {
                    match &result {
                        Ok(Ok(
                            super::transfer_job::TransferOutcome::Completed
                            | super::transfer_job::TransferOutcome::PartiallyCompleted { .. }
                            | super::transfer_job::TransferOutcome::Skipped,
                        )) => {
                            t.state = TransferState::Completed;
                            t.transferred = progress.transferred;
                        }
                        Ok(Err(e)) => {
                            if matches!(e, super::sftp_ops::SftpOpsError::Cancelled) {
                                t.state = TransferState::Cancelled;
                            } else {
                                t.state = TransferState::Failed(e.user_message());
                            }
                            t.transferred = progress.transferred;
                        }
                        Err(_) => {
                            t.state = TransferState::Cancelled;
                            t.transferred = progress.transferred;
                        }
                    }
                    t.bytes_per_second = progress.bytes_per_second;
                    t.eta = progress.eta;
                    t.phase = progress.phase;
                }
                me.transfer_handles.remove(&task_id);
                match &result {
                    Ok(Ok(
                        super::transfer_job::TransferOutcome::Completed
                        | super::transfer_job::TransferOutcome::PartiallyCompleted { .. }
                        | super::transfer_job::TransferOutcome::Skipped,
                    )) => {
                        me.refresh_dir(ctx);
                    }
                    Ok(Err(e)) => {
                        log::error!("sftp: cross-pane transfer failed: {e}");
                        me.show_error_toast(
                            crate::t!("fm-toast-transfer-failed", err = e.user_message()),
                            ctx,
                        );
                        ctx.notify();
                    }
                    Err(_) => ctx.notify(),
                }
                // Report back to a directory-move batch, if this file is part of
                // one, so the source tree is removed once all files have landed.
                if let Some(batch_id) = dir_move_batch {
                    let succeeded = matches!(
                        &result,
                        Ok(Ok(super::transfer_job::TransferOutcome::Completed))
                    );
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
            let toast = DismissibleToast::error(message).with_object_id("sftp_error".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Show a success/info toast notification (e.g. a copy/move summary).
    fn show_info_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::success(message).with_object_id("sftp_info".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    fn show_neutral_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::default(message).with_object_id("sftp_info".to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Request a path and commit it to the view/history only after listing it.
    fn navigate_to(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let path = normalize_remote_path(&path);
        if path == self.current_path {
            return;
        }
        let mut history = self.path_history.clone();
        history.truncate(self.history_index + 1);
        history.push(path.clone());
        let history_index = history.len() - 1;
        self.load_directory(
            path.clone(),
            Some(NavigationCommit {
                path,
                history,
                history_index,
            }),
            ctx,
        );
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
            let history_index = self.history_index - 1;
            let path = self.path_history[history_index].clone();
            self.load_directory(
                path.clone(),
                Some(NavigationCommit {
                    path,
                    history: self.path_history.clone(),
                    history_index,
                }),
                ctx,
            );
        }
    }

    /// Go forward to the next path in the history
    fn go_forward(&mut self, ctx: &mut ViewContext<Self>) {
        if self.history_index < self.path_history.len() - 1 {
            let history_index = self.history_index + 1;
            let path = self.path_history[history_index].clone();
            self.load_directory(
                path.clone(),
                Some(NavigationCommit {
                    path,
                    history: self.path_history.clone(),
                    history_index,
                }),
                ctx,
            );
        }
    }

    /// Open the entry at the given index
    fn open_entry(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            match entry.file_type {
                FileEntryType::Directory => {
                    self.navigate_to(entry.path.clone(), ctx);
                }
                FileEntryType::Symlink => {
                    self.resolve_and_activate_symlink(index, SymlinkActivationIntent::Open, ctx)
                }
                FileEntryType::File => {
                    self.download_entry(index, ctx);
                }
                FileEntryType::Other => {
                    self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
                }
            }
        }
    }

    /// Open the delete confirmation dialog
    fn delete_selected(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.entries.get(index) {
            let (paths, is_dirs) = if self.selected.contains(&entry.entry_identity()) {
                // Delete all selected entries
                self.entries
                    .iter()
                    .filter(|entry| self.selected.contains(&entry.entry_identity()))
                    .map(|entry| {
                        (
                            entry.path.clone(),
                            matches!(entry.file_type, FileEntryType::Directory),
                        )
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
                self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
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
                        return Err(e);
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
                        me.show_error_toast(
                            crate::t!("fm-toast-delete-failed", err = e.user_message()),
                            ctx,
                        );
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

    /// Resolve link targets and reject non-regular special files before a
    /// download picker can start a potentially blocking read.
    fn download_entry(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        match entry.file_type {
            FileEntryType::File => self.begin_download(index, None, ctx),
            FileEntryType::Symlink => {
                self.resolve_and_activate_symlink(index, SymlinkActivationIntent::Open, ctx)
            }
            FileEntryType::Directory | FileEntryType::Other => {
                self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
            }
        }
    }

    /// Open the save picker for an entry already proven to be a regular file.
    fn begin_download(
        &mut self,
        index: usize,
        resolved_target_size: Option<u64>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(entry) = self.entries.get(index) {
            let default_name = entry.name.clone();
            let entry_reference = entry.entry_reference(self.refresh_generation);
            ctx.open_save_file_picker(
                move |path_opt: Option<String>, _me: &mut Self, _ctx: &mut ViewContext<Self>| {
                    if let Some(path) = path_opt {
                        _ctx.dispatch_typed_action_deferred(SftpBrowserAction::DownloadSaveAs {
                            entry: entry_reference.clone(),
                            resolved_target_size,
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
        render_toolbar_button(icon, handle, action, appearance, position_id)
    }

    /// A toolbar button that carries an on/off state: `active` tints the icon
    /// with the accent and seats it on the shared hover surface, so the
    /// toolbar answers "are hidden files showing?" without a click.
    fn render_toolbar_toggle(
        &self,
        icon: Icon,
        active: bool,
        handle: MouseStateHandle,
        action: SftpBrowserAction,
        appearance: &Appearance,
        position_id: &'static str,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = if active {
            theme.accent()
        } else {
            theme.sub_text_color(theme.background())
        };

        let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
            .with_width(TOOLBAR_ICON_SIZE)
            .with_height(TOOLBAR_ICON_SIZE)
            .finish();

        let btn_el = Hoverable::new(handle, move |mouse| {
            let mut c = Container::new(
                ConstrainedBox::new(Container::new(icon_el).with_uniform_padding(6.0).finish())
                    .with_width(TOOLBAR_BTN_SIZE)
                    .with_height(TOOLBAR_BTN_SIZE)
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if active || mouse.is_hovered() {
                c = c.with_background(internal_colors::fg_overlay_1(theme));
            }
            c.finish()
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
            .with_child(self.render_toolbar_toggle(
                Icon::Eye,
                self.show_hidden,
                self.hidden_btn.clone(),
                SftpBrowserAction::ToggleHidden,
                appearance,
                "sftp_btn:hidden",
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

        let parts: Vec<Box<dyn Element>> = super::breadcrumb::render_breadcrumb(
            &self.current_path,
            &self.breadcrumb_mouse_handles,
            appearance,
        );

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.0);

        // Add the root directory "/" as a clickable entry point
        let root_text_color = text_color;
        let root_hoverable = Hoverable::new(self.root_breadcrumb_btn.clone(), move |_| {
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

    fn render_responsive_function_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let (compact_width, full_width) = function_legend_widths();
        SizeConstraintSwitch::new(
            self.render_function_bar(appearance, FunctionLegendMode::Full),
            vec![
                (
                    SizeConstraintCondition::WidthLessThan(compact_width),
                    self.render_function_bar(appearance, FunctionLegendMode::Hidden),
                ),
                (
                    SizeConstraintCondition::WidthLessThan(full_width),
                    self.render_function_bar(appearance, FunctionLegendMode::Compact),
                ),
            ],
        )
        .finish()
    }

    /// Render the connection state (when not connected)
    fn render_connection_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let (msg, icon) = match &self.connection {
            ConnectionState::Connecting => (crate::t!("fm-status-connecting"), Icon::Loading),
            ConnectionState::Failed(err) => (err.clone(), Icon::AlertCircle),
            ConnectionState::Disconnected => {
                (crate::t!("fm-status-disconnected"), Icon::AlertCircle)
            }
            ConnectionState::Connected => {
                return Container::new(Flex::row().finish()).finish();
            }
        };

        render_centered_status(icon, &msg, 12.0, appearance)
    }

    /// The pick-mode banner (#105): the current directory + a "Use this folder"
    /// button that returns it to the spawn card and closes the picker.
    fn render_pick_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let size = appearance.ui_font_size();
        let main = theme.main_text_color(theme.background()).into_solid();
        let accent_color = theme.accent().into_solid();
        let rest_bg = theme.surface_2();
        let hover_bg = theme.surface_3();

        let label = Text::new_inline(
            crate::t!(
                "fm-pickbar-label",
                path = self.current_path.display().to_string()
            ),
            family,
            size,
        )
        .with_color(main)
        .finish();

        let btn = Hoverable::new(self.pick_btn.clone(), move |mouse| {
            let bg = if mouse.is_hovered() {
                hover_bg
            } else {
                rest_bg
            };
            Container::new(
                Text::new_inline(crate::t!("fm-pickbar-use"), family, size)
                    .with_color(accent_color)
                    .finish(),
            )
            .with_horizontal_padding(12.0)
            .with_vertical_padding(6.0)
            .with_background(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SftpBrowserAction::PickCurrentDir))
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0)
            .with_child(Shrinkable::new(1.0, label).finish())
            .with_child(btn)
            .finish()
    }

    /// Render the MC-style function-key bar footer. Each cell shows `F<n>` in
    /// the accent colour followed by its caption, and clicking it dispatches
    /// the same action as the physical function key.
    fn render_function_bar(
        &self,
        appearance: &Appearance,
        mode: FunctionLegendMode,
    ) -> Box<dyn Element> {
        if mode == FunctionLegendMode::Hidden {
            let position_id = self.layout_position_id(mode.position_suffix());
            return save_layout_position(Flex::row().finish(), &position_id);
        }

        let theme = appearance.theme();
        let family = appearance.ui_font_family();
        let size = appearance.ui_font_size();
        let key_color = theme.sub_text_color(theme.background());
        let caption_color = theme.sub_text_color(theme.background());

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(4.0);

        for (i, (key, make_action)) in FUNCTION_BAR.iter().enumerate() {
            let handle = self.fn_bar_handles.get(i).cloned().unwrap_or_default();
            let key = *key;
            let make_action = *make_action;
            let cell = Hoverable::new(handle, move |mouse| {
                // The key renders as a quiet keycap chip (surface_2, hairline
                // border, small radius) instead of a bare accent "F3" shouting
                // DOS at the bottom of the pane (polish audit FM.4). Muted at
                // rest, the shared hover fill on approach; the VERBS stay
                // captions.
                let keycap = Container::new(
                    Text::new_inline(key.to_string(), family, size)
                        .with_color(key_color.into())
                        .finish(),
                )
                .with_padding_left(4.0)
                .with_padding_right(4.0)
                .with_padding_top(1.0)
                .with_padding_bottom(1.0)
                .with_background(theme.surface_2())
                .with_border(Border::all(1.0).with_border_fill(theme.split_pane_border_color()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)))
                .finish();
                let mut content = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.0)
                    .with_child(keycap);
                if mode == FunctionLegendMode::Full {
                    content.add_child(
                        Text::new_inline(function_bar_caption(key), family, size)
                            .with_color(caption_color.into())
                            .finish(),
                    );
                }
                let mut container = Container::new(content.finish())
                    .with_padding_left(6.0)
                    .with_padding_right(6.0)
                    .with_padding_top(2.0)
                    .with_padding_bottom(2.0)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
                if mouse.is_hovered() {
                    container = container.with_background(internal_colors::fg_overlay_1(theme));
                }
                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(make_action()))
            .finish();
            row.add_child(Expanded::new(1.0, cell).finish());
        }

        // A hairline seats the bar against the list above it — footer chrome,
        // not another list row.
        let bar = Container::new(row.finish())
            .with_padding_left(PANEL_PADDING)
            .with_padding_right(PANEL_PADDING)
            .with_padding_top(4.0)
            .with_padding_bottom(4.0)
            .with_background(theme.background())
            .with_border(Border::top(1.0).with_border_fill(theme.split_pane_border_color()))
            .finish();
        let position_id = self.layout_position_id(mode.position_suffix());
        save_layout_position(bar, &position_id)
    }

    /// Render the file list
    fn render_file_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Filter the entries — same source of truth as the keyboard cursor.
        let filtered_indices = self.visible_indices();

        // A directory with no listable entries still offers `..`: an empty
        // folder you cannot leave by clicking would be a trap.
        if filtered_indices.is_empty() && !self.has_parent_row() {
            let text_el = Text::new_inline(
                crate::t!("fm-empty-folder"),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();

            return Align::new(Container::new(text_el).with_uniform_padding(24.0).finish())
                .finish();
        }

        // Header row — its columns sort the list on click.
        let header = super::file_list::render_header(self.sort, &self.header_handles, appearance);

        // File rows (the `..` row first, when there is a parent)
        let row_position_prefix = self.layout_position_id("row");
        let panel_position_id = self.layout_position_id("panel");
        let rows = super::file_list::render_file_rows(
            &self.entries,
            &filtered_indices,
            &self.selected,
            self.refresh_generation,
            self.cursor,
            self.has_parent_row(),
            &row_position_prefix,
            &panel_position_id,
            &self.row_mouse_handles,
            &self.mark_handles,
            self.parent_row_handle.clone(),
            appearance,
        );

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(rows);
        // MC's selection status line: what is marked, and how much of it.
        if !self.selected.is_empty() {
            col = col.with_child(super::file_list::render_selection_status(
                self.selected.len(),
                self.marked_size(),
                appearance,
            ));
        }
        col.finish()
    }

    /// Render the transfer panel
    fn render_transfers(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let queue = super::transfer_queue::TransferQueue::as_ref(app);
        let mut transfers = self
            .transfers
            .iter()
            .filter(|task| !self.transfer_queue_ids.contains_key(&task.id))
            .cloned()
            .collect::<Vec<_>>();
        let mut cancel_handles = self.transfer_cancel_handles.clone();
        let mut pause_handles = self.transfer_pause_handles.clone();
        let mut retry_handles = HashMap::new();
        for activity in queue.activities() {
            let Ok(task_id) = usize::try_from(activity.id) else {
                continue;
            };
            let mut task = TransferTask::new(
                task_id,
                activity.source_path,
                activity.target_path,
                activity.direction,
                activity.progress.total,
            );
            task.transferred = activity.progress.transferred;
            task.bytes_per_second = activity.progress.bytes_per_second;
            task.control_epoch = Some(activity.control_epoch);
            task.eta = activity.progress.eta;
            task.phase = activity.progress.phase;
            task.recovery_paths = activity.recovery_paths;
            task.recovery_retryable = activity.recovery_retryable;
            task.state = super::transfer_panel::transfer_state_from_queue_state(activity.state);
            transfers.push(task);
            if let Some(handle) = queue.cancel_handle(activity.id) {
                cancel_handles.insert(task_id, handle);
            }
            if let Some(handle) = queue.pause_handle(activity.id) {
                pause_handles.insert(task_id, handle);
            }
            if let Some(handle) = queue.retry_handle(activity.id) {
                retry_handles.insert(task_id, handle);
            }
        }
        transfers.sort_by_key(transfer_display_priority);
        super::transfer_panel::render_transfer_panel(
            &transfers,
            &cancel_handles,
            &pause_handles,
            &retry_handles,
            appearance,
            self.transfer_panel_close_btn.clone(),
            self.transfer_panel_scroll_state.clone(),
            super::transfer_panel::TransferPanelTarget::Browser,
        )
    }

    fn schedule_transfer_progress_poll(&mut self, ctx: &mut ViewContext<Self>) {
        if self.transfer_progress_poll_handle.is_some()
            || super::transfer_queue::TransferQueue::as_ref(ctx)
                .summary()
                .active
                == 0
        {
            return;
        }
        self.transfer_progress_poll_handle = Some(ctx.spawn_abortable(
            Timer::after(std::time::Duration::from_millis(100)),
            |me, _, ctx| {
                me.transfer_progress_poll_handle = None;
                ctx.notify();
                me.schedule_transfer_progress_poll(ctx);
            },
            |_, _| {},
        ));
    }

    fn sync_transfer_tracking_with_queue(&mut self, app: &AppContext) {
        let queue_ids = super::transfer_queue::TransferQueue::as_ref(app)
            .activities()
            .map(|activity| activity.id)
            .collect::<HashSet<_>>();
        let removed_task_ids = self
            .transfer_queue_ids
            .iter()
            .filter_map(|(task_id, queue_id)| (!queue_ids.contains(queue_id)).then_some(*task_id))
            .collect::<Vec<_>>();
        self.transfer_queue_ids
            .retain(|_, queue_id| queue_ids.contains(queue_id));
        for task_id in removed_task_ids {
            self.transfers.retain(|task| task.id != task_id);
            self.transfer_cancel_handles.remove(&task_id);
            self.transfer_pause_handles.remove(&task_id);
            self.transfer_handles.remove(&task_id);
        }
    }

    /// Execute an upload (shared entry point for drag-and-drop and file-picker uploads)
    ///
    /// First checks whether a file with the same name already exists in the remote directory;
    /// if so, opens an overwrite confirmation dialog, and after the user confirms,
    /// performs the actual upload via `execute_upload_confirmed`.
    fn execute_upload(&mut self, local_path: &Path, ctx: &mut ViewContext<Self>) {
        if std::fs::symlink_metadata(local_path)
            .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
        {
            self.show_error_toast(crate::t!("fm-toast-unsupported-file-type"), ctx);
            return;
        }
        let file_name = local_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let remote_path = match build_upload_remote_path(&self.current_path, &file_name) {
            Some(p) => p,
            None => {
                self.show_error_toast(crate::t!("fm-toast-invalid-filename-chars"), ctx);
                return;
            }
        };

        // Check whether a file with the same name already exists in the remote directory
        let existing = self
            .entries
            .iter()
            .find(|e| e.name == file_name && matches!(e.file_type, FileEntryType::File));

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
        self.execute_upload_confirmed(local_path, &remote_path, false, ctx);
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
        overwrite_destination: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(backend) = self.sftp.clone() {
            self.spawn_transfer_with_backend(
                local_path.to_path_buf(),
                remote_path.to_path_buf(),
                backend,
                TransferDirection::Upload,
                overwrite_destination,
                None,
                None,
                ctx,
            );
        } else {
            log::error!("sftp: upload failed: not connected to server");
            self.show_error_toast(crate::t!("fm-toast-upload-failed-not-connected"), ctx);
        }
    }

    /// Execute a download (shared logic for confirm-overwrite and save-as)
    fn execute_download(
        &mut self,
        remote_path: &Path,
        local_path: &Path,
        _file_size: u64,
        overwrite_destination: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(backend) = self.sftp.clone() {
            self.spawn_transfer_with_backend(
                local_path.to_path_buf(),
                remote_path.to_path_buf(),
                backend,
                TransferDirection::Download,
                overwrite_destination,
                None,
                None,
                ctx,
            );
        } else {
            log::error!("sftp: download failed: not connected to server");
            self.show_error_toast(crate::t!("fm-toast-download-failed-not-connected"), ctx);
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
        render_centered_status(
            Icon::Loading,
            &crate::t!("fm-status-loading"),
            8.0,
            appearance,
        )
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

fn validated_relative_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe relative transfer path: {}", path.display()));
    }
    Ok(path.to_path_buf())
}

fn validated_listing_relative_path(
    listed_dir: &Path,
    source_root: &Path,
    entry: &FileEntry,
) -> Result<PathBuf, String> {
    let name_path = Path::new(&entry.name);
    validated_relative_path(name_path)?;
    let expected_path = normalize_remote_path(&listed_dir.join(name_path));
    if entry.path != expected_path {
        return Err(format!(
            "directory listing returned unexpected path {} for {}",
            entry.path.display(),
            expected_path.display()
        ));
    }
    let relative = entry.path.strip_prefix(source_root).map_err(|_| {
        format!(
            "directory listing escaped source root {}: {}",
            source_root.display(),
            entry.path.display()
        )
    })?;
    validated_relative_path(relative)
}

fn ensure_remote_directory(backend: &dyn SftpBackend, path: &Path) -> Result<(), String> {
    match backend.lstat(path) {
        Ok(entry) if matches!(entry.file_type, FileEntryType::Directory) => Ok(()),
        Ok(_) => Err(format!(
            "remote directory target is not a directory: {}",
            path.display()
        )),
        Err(_) => backend.create_dir(path).map_err(|error| error.to_string()),
    }
}

fn ensure_local_directory(path: &Path, create_parents: bool) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "local directory target is a symbolic link: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "local directory target is not a directory: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let result = if create_parents {
                std::fs::create_dir_all(path)
            } else {
                std::fs::create_dir(path)
            };
            result.map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
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
            let source_metadata =
                std::fs::symlink_metadata(source_dir).map_err(|error| error.to_string())?;
            if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
                return Err(format!(
                    "upload source is not a regular directory: {}",
                    source_dir.display()
                ));
            }

            // Validate the complete local tree before creating anything on the
            // destination. A late symlink/FIFO must not leave a partial plan
            // that a move could mistake for success.
            let mut directories = Vec::new();
            let mut regular_files = Vec::new();
            for entry in walkdir::WalkDir::new(source_dir).min_depth(1) {
                let entry = entry.map_err(|e| e.to_string())?;
                let relative = entry
                    .path()
                    .strip_prefix(source_dir)
                    .map_err(|e| e.to_string())?;
                let relative = validated_relative_path(relative)?;
                let remote_target = normalize_remote_path(&dest_dir.join(&relative));
                if entry.file_type().is_dir() {
                    directories.push(remote_target);
                } else if entry.file_type().is_file() {
                    regular_files.push((entry.path().to_path_buf(), remote_target));
                } else if entry.file_type().is_symlink() {
                    return Err(format!(
                        "refusing to recursively upload symbolic link {}",
                        entry.path().display()
                    ));
                } else {
                    return Err(format!(
                        "refusing to recursively upload special file {}",
                        entry.path().display()
                    ));
                }
            }

            ensure_remote_directory(remote_backend.as_ref(), dest_dir)?;
            for directory in directories {
                ensure_remote_directory(remote_backend.as_ref(), &directory)?;
            }
            for (local_path, remote_path) in regular_files {
                match remote_backend.lstat(&remote_path) {
                    Ok(entry) if matches!(entry.file_type, FileEntryType::File) => {
                        skipped_existing += 1;
                    }
                    Ok(_) => {
                        return Err(format!(
                            "remote file target is not a regular file: {}",
                            remote_path.display()
                        ));
                    }
                    Err(_) => files.push((local_path, remote_path)),
                }
            }
        }
        TransferDirection::Download => {
            let source_root = normalize_remote_path(source_dir);
            let source_metadata = remote_backend
                .lstat(&source_root)
                .map_err(|error| error.to_string())?;
            if !matches!(source_metadata.file_type, FileEntryType::Directory) {
                return Err(format!(
                    "download source is not a regular directory: {}",
                    source_root.display()
                ));
            }

            // First enumerate and validate every server-provided path and type.
            // Only after the complete tree is trusted may local directories be
            // created.
            let mut stack = vec![source_root.clone()];
            let mut visited = HashSet::new();
            let mut directories = Vec::new();
            let mut regular_files = Vec::new();
            while let Some(dir) = stack.pop() {
                if !visited.insert(dir.clone()) {
                    return Err(format!("directory listing repeated path {}", dir.display()));
                }
                let entries = remote_backend.list_dir(&dir).map_err(|e| e.to_string())?;
                for entry in entries {
                    let relative = validated_listing_relative_path(&dir, &source_root, &entry)?;
                    let local_target = dest_dir.join(&relative);
                    match entry.file_type {
                        FileEntryType::Directory => {
                            directories.push(local_target);
                            stack.push(entry.path);
                        }
                        FileEntryType::File => {
                            regular_files.push((local_target, entry.path));
                        }
                        FileEntryType::Symlink => {
                            return Err(format!(
                                "refusing to recursively download symbolic link {}",
                                entry.path.display()
                            ));
                        }
                        FileEntryType::Other => {
                            return Err(format!(
                                "refusing to recursively download special file {}",
                                entry.path.display()
                            ));
                        }
                    }
                }
            }

            ensure_local_directory(dest_dir, true)?;
            directories.sort_by_key(|path| path.components().count());
            for directory in directories {
                ensure_local_directory(&directory, false)?;
            }
            for (local_path, remote_path) in regular_files {
                match std::fs::symlink_metadata(&local_path) {
                    Ok(metadata) if metadata.is_file() => skipped_existing += 1,
                    Ok(_) => {
                        return Err(format!(
                            "local file target is not a regular file: {}",
                            local_path.display()
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        files.push((local_path, remote_path));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
        TransferDirection::Copy => {
            unreachable!("copy direction is not used by upload/download conflict checks")
        }
    }
    Ok(DirTransferPlan {
        files,
        skipped_existing,
    })
}

fn remove_backend_entry(
    backend: &dyn SftpBackend,
    path: &Path,
) -> Result<(), sftp_ops::SftpOpsError> {
    match backend.lstat(path)?.file_type {
        FileEntryType::Directory => backend.delete_dir_recursive(path),
        FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other => {
            backend.delete_file(path)
        }
    }
}

fn delete_quarantined_dir_source(
    backend: &dyn SftpBackend,
    quarantine: &Path,
) -> Result<(), sftp_ops::SftpOpsError> {
    backend.delete_dir_recursive(quarantine).map_err(|error| {
        sftp_ops::SftpOpsError::Operation(format!(
            "{error}; source cleanup may be incomplete; remaining source recovery path: {}",
            quarantine.display()
        ))
    })
}

fn unique_backend_sibling(destination: &Path, marker: &str) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "entry".to_string());
    destination.with_file_name(format!(".{name}.{marker}-{}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
fn replace_backend_entry(
    backend: &dyn SftpBackend,
    replacement: &Path,
    destination: &Path,
) -> Result<(), sftp_ops::SftpOpsError> {
    backend.replace(replacement, destination)
}

#[cfg(test)]
fn execute_backend_copy_move(
    backend: &dyn SftpBackend,
    source: &Path,
    destination: &Path,
    is_dir: bool,
    is_move: bool,
    overwrite: bool,
) -> Result<(), sftp_ops::SftpOpsError> {
    super::sftp_backend::validate_copy_destination(source, destination, is_dir)?;
    let source_type = backend.lstat(source)?.file_type;
    match (is_dir, source_type) {
        (true, FileEntryType::Directory) | (false, FileEntryType::File) => {}
        (_, FileEntryType::Symlink) => {
            return Err(sftp_ops::SftpOpsError::Operation(format!(
                "Refusing to copy or move symbolic link {}",
                source.display()
            )));
        }
        (_, FileEntryType::Other) => {
            return Err(sftp_ops::SftpOpsError::Operation(format!(
                "Refusing to copy or move special file {}",
                source.display()
            )));
        }
        (true, FileEntryType::File) | (false, FileEntryType::Directory) => {
            return Err(sftp_ops::SftpOpsError::Operation(format!(
                "Source type changed before transfer: {}",
                source.display()
            )));
        }
    }

    if !is_move && is_dir {
        let staged = unique_backend_sibling(destination, "zaplex_staged");
        if let Err(error) = backend.copy_dir_recursive(source, &staged) {
            if backend.lstat(&staged).is_ok() {
                let _ = remove_backend_entry(backend, &staged);
            }
            return Err(error);
        }
        let result = if overwrite {
            match backend.lstat(destination) {
                Ok(_) => replace_backend_entry(backend, &staged, destination),
                Err(error) => Err(error),
            }
        } else {
            backend.rename(&staged, destination)
        };
        if result.is_err() && backend.lstat(&staged).is_ok() {
            let _ = remove_backend_entry(backend, &staged);
        }
        return result;
    }

    if !overwrite {
        if is_move {
            return backend.rename(source, destination);
        }
        let staged = unique_backend_sibling(destination, "zaplex_staged");
        if let Err(error) = backend.copy_file(source, &staged) {
            if backend.lstat(&staged).is_ok() {
                let _ = remove_backend_entry(backend, &staged);
            }
            return Err(error);
        }
        let result = backend.rename(&staged, destination);
        if result.is_err() && backend.lstat(&staged).is_ok() {
            let _ = remove_backend_entry(backend, &staged);
        }
        return result;
    }

    backend.lstat(destination)?;
    if is_move {
        return replace_backend_entry(backend, source, destination);
    }
    let staged = unique_backend_sibling(destination, "zaplex_staged");
    if let Err(error) = backend.copy_file(source, &staged) {
        if backend.lstat(&staged).is_ok() {
            let _ = remove_backend_entry(backend, &staged);
        }
        return Err(error);
    }
    let result = replace_backend_entry(backend, &staged, destination);
    if result.is_err() && backend.lstat(&staged).is_ok() {
        let _ = remove_backend_entry(backend, &staged);
    }
    result
}

#[cfg(test)]
impl SftpBrowserView {
    pub(super) fn has_transfer_progress_poll_for_test(&self) -> bool {
        self.transfer_progress_poll_handle.is_some()
    }

    pub(super) fn tracked_queue_transfers_for_test(&self) -> usize {
        self.transfer_queue_ids.len()
    }

    pub(super) fn render_refresh_button_for_test(
        handle: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        render_toolbar_button(
            Icon::Refresh,
            handle,
            SftpBrowserAction::Refresh,
            appearance,
            "sftp_btn:refresh",
        )
    }

    /// Test wrapper around the private [`Self::spawn_dir_transfer`].
    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_dir_transfer_for_test(
        &mut self,
        source_dir: PathBuf,
        dest_dir: PathBuf,
        direction: TransferDirection,
        remote_backend: Arc<dyn SftpBackend>,
        is_move: bool,
        label: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.spawn_dir_transfer(
            source_dir,
            dest_dir,
            direction,
            remote_backend,
            is_move,
            label,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_backend_queue_job_for_test(
        &mut self,
        source_backend: Arc<dyn SftpBackend>,
        target_backend: Arc<dyn SftpBackend>,
        source_path: PathBuf,
        target_path: PathBuf,
        is_dir: bool,
        conflict: super::transfer_job::ConflictDecision,
        label: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.spawn_backend_queue_job(
            source_backend,
            target_backend,
            source_path,
            target_path,
            is_dir,
            false,
            conflict,
            false,
            label,
            ctx,
        );
    }

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
            original_source_dir: None,
            label: "test-dir".to_string(),
        });
        id
    }

    pub(super) fn seed_quarantined_dir_move_cleanup_for_test(
        &mut self,
        remaining: usize,
        source_backend: Arc<dyn SftpBackend>,
        source_dir: PathBuf,
        original_source_dir: PathBuf,
    ) -> u64 {
        let id = self.next_dir_move_batch_id;
        self.next_dir_move_batch_id += 1;
        self.pending_dir_move_cleanups.push(PendingDirMoveCleanup {
            id,
            remaining,
            any_failed: false,
            source_backend,
            source_dir,
            original_source_dir: Some(original_source_dir),
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

    pub(super) fn close_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        <Self as BackingView>::close(self, ctx);
    }

    /// Whether any directory-move cleanup is still pending (for assertions).
    pub(super) fn has_pending_dir_move_cleanup(&self) -> bool {
        !self.pending_dir_move_cleanups.is_empty()
    }

    /// Whether a cross-connection overwrite prompt is still pending (for assertions).
    pub(super) fn has_pending_cross_conn(&self) -> bool {
        self.pending_cross_conn.is_some()
    }

    pub(super) fn attach_queue_transfer_for_test(
        &mut self,
        task: TransferTask,
        queue_id: super::transfer_queue::TransferId,
    ) {
        self.transfer_queue_ids.insert(task.id, queue_id);
        self.transfer_cancel_handles
            .insert(task.id, MouseStateHandle::default());
        self.transfer_pause_handles
            .insert(task.id, MouseStateHandle::default());
        self.transfers.push(task);
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

fn local_entry_exists(path: &Path) -> Result<bool, sftp_ops::SftpOpsError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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
            SftpBrowserAction::ToggleMark(entry) => {
                if self.row_clicks_suppressed() {
                    return;
                }
                let Some(index) = self.resolve_entry_reference(entry) else {
                    return;
                };
                // Mouse parity for Insert/Space. The cursor follows the click,
                // so keyboard and mouse never disagree about "the current row".
                if let Some(pos) = self.visible_indices().iter().position(|&i| i == index) {
                    self.cursor = pos + usize::from(self.has_parent_row());
                }
                self.toggle_mark_index(index);
                ctx.notify();
            }
            SftpBrowserAction::MarkAndAdvance => self.mark_and_advance(ctx),
            SftpBrowserAction::SortBy(column) => {
                // Resolve which FILE the cursor is on BEFORE reordering: after
                // `self.sort` changes, the same row index resolves to a
                // different file, and re-anchoring on that would silently move
                // the cursor onto the wrong entry.
                let cursor_entry = self.cursor_entry_index();
                self.sort = self.sort.toggled(*column);
                // Marks are entry-keyed, so they survive a re-sort untouched;
                // only the cursor's ROW position is meaningless afterwards.
                match cursor_entry {
                    Some(entry) => {
                        if let Some(pos) = self.visible_indices().iter().position(|&i| i == entry) {
                            self.cursor = pos + usize::from(self.has_parent_row());
                        }
                    }
                    None => self.clamp_cursor_to_visible(),
                }
                ctx.notify();
            }
            SftpBrowserAction::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                // Rows appear/disappear underneath the cursor and the marks.
                self.retain_marks_to_visible();
                ctx.notify();
            }
            SftpBrowserAction::SelectEntry(entry) => {
                let Some(index) = self.resolve_entry_reference(entry) else {
                    return;
                };
                // A mouse click also moves the MC keyboard cursor onto the
                // clicked row (the accent bar), so the click highlight and the
                // keyboard cursor never disagree. `cursor` indexes the *visible*
                // (filtered) list, so translate the entry index into that space.
                if let Some(pos) = self.visible_indices().iter().position(|&i| i == index) {
                    self.cursor = pos + usize::from(self.has_parent_row());
                }
                self.selected.clear();
                self.selected.insert(entry.identity.clone());
                ctx.notify();
            }
            SftpBrowserAction::OpenEntry(entry) => {
                // Swallow a stray double-click tail aimed at the previous
                // listing (see `suppress_row_clicks_until`). Keyboard opens go
                // through `ActivateCursor`, not here.
                if self.row_clicks_suppressed() {
                    return;
                }
                if let Some(index) = self.resolve_entry_reference(entry) {
                    self.open_entry(index, ctx);
                }
            }
            SftpBrowserAction::DeleteEntry(entry) => {
                if let Some(index) = self.resolve_entry_reference(entry) {
                    self.delete_selected(index, ctx);
                }
            }
            SftpBrowserAction::RenameEntry(entry) => {
                if let Some(index) = self.resolve_entry_reference(entry) {
                    self.rename_entry(index, ctx);
                }
            }
            SftpBrowserAction::DownloadEntry(entry) => {
                if let Some(index) = self.resolve_entry_reference(entry) {
                    self.download_entry(index, ctx);
                }
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
                        self.show_error_toast(crate::t!("fm-toast-name-empty"), ctx);
                        return;
                    }
                    let new_path = match build_rename_path(original_path, &new_name) {
                        Some(p) => p,
                        None => {
                            self.show_error_toast(
                                crate::t!("fm-toast-name-invalid-separators"),
                                ctx,
                            );
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
                                        me.show_error_toast(
                                            crate::t!(
                                                "fm-toast-rename-failed",
                                                err = e.user_message()
                                            ),
                                            ctx,
                                        );
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
                        self.dialog = None;
                    }
                }
            }
            SftpBrowserAction::ConfirmNewFolder => {
                if let Some(Dialog::CreateFolder { parent_path }) = &self.dialog {
                    let folder_name = self.new_folder_editor.as_ref(ctx).buffer_text(ctx);
                    let folder_name = folder_name.trim().to_string();
                    if folder_name.is_empty() {
                        self.show_error_toast(crate::t!("fm-toast-folder-name-empty"), ctx);
                        return;
                    }
                    let folder_path = match build_new_folder_path(parent_path, &folder_name) {
                        Some(p) => p,
                        None => {
                            self.show_error_toast(
                                crate::t!("fm-toast-name-invalid-separators"),
                                ctx,
                            );
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
                                        me.show_error_toast(
                                            crate::t!(
                                                "fm-toast-create-folder-failed",
                                                err = e.user_message()
                                            ),
                                            ctx,
                                        );
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
                        self.dialog = None;
                    }
                }
            }
            SftpBrowserAction::ConfirmOverwrite => {
                // Extract the paths and transfer direction from the dialog
                let (source, target, file_size, direction) = match &self.dialog {
                    Some(Dialog::OverwriteConfirm {
                        source,
                        target,
                        file_size,
                        direction,
                    }) => (source.clone(), target.clone(), *file_size, *direction),
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
                        self.execute_download(&source, &target, file_size, true, ctx);
                    }
                    TransferDirection::Upload => {
                        self.execute_upload_confirmed(&source, &target, true, ctx);
                    }
                    TransferDirection::Copy => {
                        unreachable!("copy direction is not used by overwrite dialogs")
                    }
                }
                // Batch upload queue: continue with the next file after confirming the current one
                self.process_pending_uploads(ctx);
            }
            SftpBrowserAction::ContextMenu { entry, position } => {
                let position = *position;
                let Some(index) = self.resolve_entry_reference(entry) else {
                    return;
                };
                self.context_menu = Some(ContextMenuState::new(entry.clone(), position));
                self.selected.clear();
                self.selected.insert(self.entries[index].entry_identity());
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
                    Some(Dialog::OverwriteConfirm {
                        direction: TransferDirection::Upload,
                        ..
                    })
                );
                self.dialog = None;
                if was_upload_overwrite {
                    self.pending_uploads.clear();
                }
                ctx.notify();
            }
            SftpBrowserAction::DetailsEntry(entry) => {
                if let Some(index) = self.resolve_entry_reference(entry) {
                    self.show_details(index, ctx);
                }
            }
            SftpBrowserAction::SetSearchFilter(filter) => {
                self.search_filter = Some(filter.clone());
                // A narrowing filter must not leave marks on rows it hid.
                self.retain_marks_to_visible();
                ctx.notify();
            }
            SftpBrowserAction::ClearSearchFilter => {
                self.search_filter = None;
                self.clamp_cursor_to_visible();
                ctx.notify();
            }
            SftpBrowserAction::NavigateUp => {
                // Arm the stray-click window BEFORE navigating: the `..` row
                // navigates on a single click, so a habitual double click's
                // second click is still in flight when the rows swap (see
                // `suppress_row_clicks_until`).
                self.suppress_row_clicks_until =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(250));
                self.go_up(ctx);
            }
            SftpBrowserAction::DeleteSelected => {
                // Prefer the multi-selection; fall back to the row under the MC
                // cursor so a keyboard-only user (who never pressed Space) can
                // still delete the highlighted row. Mirrors `operation_sources`.
                let index = self
                    .entries
                    .iter()
                    .position(|entry| self.selected.contains(&entry.entry_identity()))
                    .or_else(|| self.cursor_entry_index());
                if let Some(index) = index {
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
                            self.show_error_toast(crate::t!("fm-toast-invalid-target-path"), ctx);
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
                                        me.show_error_toast(
                                            crate::t!(
                                                "fm-toast-move-failed",
                                                err = e.user_message()
                                            ),
                                            ctx,
                                        );
                                    }
                                    Err(_) => {}
                                }
                                ctx.notify();
                            },
                        );
                    } else {
                        self.show_error_toast(crate::t!("fm-toast-not-connected-server"), ctx);
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
            SftpBrowserAction::ViewCursorDetails => self.view_cursor_details(ctx),
            SftpBrowserAction::OpenCursorInEditor => self.open_cursor_in_editor(ctx),
            SftpBrowserAction::EnterCursorDir => self.enter_cursor_dir(ctx),
            SftpBrowserAction::ToggleSelectCursor => self.toggle_select_cursor(ctx),
            SftpBrowserAction::RenameCursor => self.rename_cursor(ctx),
            SftpBrowserAction::CopyToOtherPane => self.copy_or_move_to_other_pane(false, ctx),
            SftpBrowserAction::MoveToOtherPane => self.copy_or_move_to_other_pane(true, ctx),
            SftpBrowserAction::PickCurrentDir => {
                // Return the browsed directory to the spawn card and close (#105).
                self.pick_resolved = true;
                ctx.dispatch_typed_action(&crate::WorkspaceAction::RemoteSpawnDirPicked {
                    path: self.current_path.clone(),
                });
                ctx.emit(PaneEvent::Close);
            }
            SftpBrowserAction::CloseFileManager => {
                // Reverts the pane to its underlying terminal (temporary-replacement seam).
                ctx.emit(PaneEvent::Close);
            }
            SftpBrowserAction::OverwriteConflict { all } => {
                self.resolve_copy_move_conflict(
                    super::transfer_job::ConflictDecision::Overwrite,
                    *all,
                    ctx,
                );
            }
            SftpBrowserAction::SkipConflict { all } => {
                self.resolve_copy_move_conflict(
                    super::transfer_job::ConflictDecision::Skip,
                    *all,
                    ctx,
                );
            }
            SftpBrowserAction::RenameConflict { all } => {
                self.resolve_copy_move_conflict(
                    super::transfer_job::ConflictDecision::Rename,
                    *all,
                    ctx,
                );
            }
            SftpBrowserAction::NewerOnlyConflict { all } => {
                self.resolve_copy_move_conflict(
                    super::transfer_job::ConflictDecision::NewerOnly,
                    *all,
                    ctx,
                );
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
            SftpBrowserAction::CancelTransfer(task_id, control_epoch) => {
                let task_id = *task_id;
                if let Some(control_epoch) = control_epoch {
                    let target = super::transfer_queue::TransferActionTarget {
                        id: task_id as u64,
                        control_epoch: *control_epoch,
                    };
                    super::transfer_queue::TransferQueue::handle(ctx).update(
                        ctx,
                        move |queue, ctx| {
                            queue.cancel(target);
                            ctx.notify();
                        },
                    );
                } else if let Some(task) = self.transfers.iter().find(|task| task.id == task_id) {
                    task.cancel();
                }
                ctx.notify();
            }
            SftpBrowserAction::PauseTransfer(task_id, control_epoch) => {
                if let Some(control_epoch) = control_epoch {
                    let target = super::transfer_queue::TransferActionTarget {
                        id: *task_id as u64,
                        control_epoch: *control_epoch,
                    };
                    super::transfer_queue::TransferQueue::handle(ctx).update(
                        ctx,
                        move |queue, ctx| {
                            queue.pause(target);
                            ctx.notify();
                        },
                    );
                }
                ctx.notify();
            }
            SftpBrowserAction::ResumeTransfer(task_id, control_epoch) => {
                if let Some(control_epoch) = control_epoch {
                    let target = super::transfer_queue::TransferActionTarget {
                        id: *task_id as u64,
                        control_epoch: *control_epoch,
                    };
                    super::transfer_queue::TransferQueue::handle(ctx).update(
                        ctx,
                        move |queue, ctx| {
                            queue.resume(target);
                            ctx.notify();
                        },
                    );
                }
                ctx.notify();
            }
            SftpBrowserAction::RetryTransferRecovery(task_id) => {
                let Some(queue_id) = u64::try_from(*task_id)
                    .ok()
                    .filter(|id| {
                        super::transfer_queue::TransferQueue::as_ref(ctx)
                            .activity(*id)
                            .is_some()
                    })
                    .or_else(|| self.transfer_queue_ids.get(task_id).copied())
                else {
                    return;
                };
                let result = super::transfer_queue::TransferQueue::handle(ctx).update(
                    ctx,
                    move |queue, ctx| {
                        let result = queue.retry_recovery_in_background(queue_id);
                        ctx.notify();
                        result
                    },
                );
                match result {
                    Ok(()) => self.show_info_toast(crate::t!("fm-transfer-recovery-started"), ctx),
                    Err(error) => self.show_error_toast(
                        crate::t!("fm-transfer-recovery-failed", err = error.user_message()),
                        ctx,
                    ),
                }
                ctx.notify();
            }
            SftpBrowserAction::ToggleTransferPanel => {
                let has_active = super::transfer_queue::TransferQueue::as_ref(ctx)
                    .summary()
                    .active
                    > 0
                    || self.transfers.iter().any(|task| {
                        !self.transfer_queue_ids.contains_key(&task.id)
                            && matches!(
                                task.state,
                                TransferState::Pending
                                    | TransferState::InProgress
                                    | TransferState::Paused
                                    | TransferState::Cancelling
                            )
                    });
                let has_recovery = super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activities()
                    .any(|activity| !activity.recovery_paths.is_empty());
                if has_active {
                    self.dialog = Some(Dialog::CloseTransferPanelConfirm);
                } else if has_recovery {
                    self.transfer_panel_hidden = false;
                } else {
                    super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                        queue.clear_terminal();
                        ctx.notify();
                    });
                    self.transfers.clear();
                    self.transfer_cancel_handles.clear();
                    self.transfer_pause_handles.clear();
                    self.transfer_queue_ids.clear();
                    self.transfer_panel_hidden = true;
                }
                ctx.notify();
            }
            SftpBrowserAction::ConfirmCloseTransferPanel => {
                for task in &self.transfers {
                    task.cancel();
                }
                let queue_ids = super::transfer_queue::TransferQueue::as_ref(ctx)
                    .activities()
                    .filter(|activity| {
                        matches!(
                            activity.state,
                            super::transfer_queue::QueuedTransferState::Running
                                | super::transfer_queue::QueuedTransferState::Paused
                                | super::transfer_queue::QueuedTransferState::Cancelling
                        )
                    })
                    .map(|activity| activity.id)
                    .collect::<Vec<_>>();
                super::transfer_queue::TransferQueue::handle(ctx).update(ctx, move |queue, ctx| {
                    for id in queue_ids {
                        queue.cancel_and_clear(id);
                    }
                    queue.clear_terminal();
                    ctx.notify();
                });
                self.transfers.clear();
                self.transfer_cancel_handles.clear();
                self.transfer_pause_handles.clear();
                self.transfer_queue_ids.clear();
                self.transfer_panel_hidden = false;
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
            SftpBrowserAction::DownloadSaveAs {
                entry,
                resolved_target_size,
                local_path,
            } => {
                let local_path = PathBuf::from(local_path);
                if self.sftp.is_some() {
                    if let Some(index) = self.resolve_entry_reference(entry) {
                        let current = &self.entries[index];
                        let remote_path = current.path.clone();
                        let file_size = resolved_download_size(current.size, *resolved_target_size);
                        self.execute_download(&remote_path, &local_path, file_size, false, ctx);
                    }
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

        // 1. When not connected, show the connection state.
        //
        // NB: the view root is wrapped in a `Container`, never returned as a bare
        // `Flex(MainAxisSize::Max)`. `PaneView` mounts every child view as a
        // `Shrinkable` flex child, so a bare `Flex(Max)` root is measured with an
        // infinite main axis and trips the flex assert → SIGABRT (the crash hit
        // when opening the file manager on an SSH pane, which lands here first in
        // the "Connecting…" state). The tight `Container` wrapper is the same
        // remedy `CodeView` uses.
        if !matches!(self.connection, ConnectionState::Connected) {
            let mut content = Container::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(self.render_connection_state(appearance))
                    .finish(),
            )
            .finish();
            if self
                .focus_handle
                .as_ref()
                .is_some_and(|handle| handle.is_focused(app))
            {
                content = Container::new(content)
                    .with_border(Border::all(2.0).with_border_fill(theme.accent()))
                    .finish();
            }
            return content;
        }

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max);

        // Pick-mode banner (#105): a prominent "use this folder" action on top.
        if self.pick_mode {
            col.add_child(
                Container::new(self.render_pick_bar(appearance))
                    .with_padding_left(PANEL_PADDING)
                    .with_padding_right(PANEL_PADDING)
                    .with_padding_top(PANEL_PADDING)
                    .finish(),
            );
        }

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
            col.add_child(Expanded::new(1.0, self.render_loading(appearance)).finish());
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
            let body_position_id = self.layout_position_id("body");
            let positioned_body = save_layout_position(scrollable, &body_position_id);
            col.add_child(Expanded::new(1.0, positioned_body).finish());
        }

        // 6. MC-style function-key footer. It belongs to this browser view,
        // never to a shared pane-group container. Captions are dropped before
        // a narrow split could overlap them.
        col.add_child(self.render_responsive_function_bar(appearance));

        // 7. Transfer panel (floating at the bottom)
        // Wrap the `Flex(Max)` body in a tight `Container` before it becomes the
        // view root (same reason as the not-connected branch above): the pane
        // mounts this as a `Shrinkable` flex child, and a bare `Flex(Max)` root
        // would be measured with an infinite main axis and crash.
        let mut main_content = Container::new(col.finish()).finish();
        if self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(app))
        {
            main_content = Container::new(main_content)
                .with_border(Border::all(2.0).with_border_fill(theme.accent()))
                .finish();
        }
        let root_position_id = self.layout_position_id("pane-root");
        main_content = save_layout_position(main_content, &root_position_id);

        // 8. Transfer panel floating layer
        let global_activities = super::transfer_queue::TransferQueue::as_ref(app)
            .activities()
            .collect::<Vec<_>>();
        let has_global_transfers = !global_activities.is_empty();
        let force_global_panel = global_activities.iter().any(|activity| {
            matches!(
                activity.state,
                super::transfer_queue::QueuedTransferState::Running
                    | super::transfer_queue::QueuedTransferState::Paused
                    | super::transfer_queue::QueuedTransferState::Cancelling
            ) || !activity.recovery_paths.is_empty()
        });
        if (!self.transfers.is_empty() || has_global_transfers)
            && (!self.transfer_panel_hidden || force_global_panel)
        {
            let panel_el = Container::new(self.render_transfers(appearance, app))
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
                self.rename_conflict_btn.clone(),
                self.newer_only_conflict_btn.clone(),
                self.rename_all_btn.clone(),
                self.newer_only_all_btn.clone(),
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
                crate::t!("fm-drag-hint"),
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
        let panel_position_id = self.layout_position_id("panel");
        let positioned_content = save_layout_position(main_content, &panel_position_id);

        // 12. Keyboard event interception — MC-style navigation. A modifier
        // (other than Shift, used for range operations later) means the
        // keystroke belongs to a shortcut elsewhere; let it propagate.
        let focus_handle = self.focus_handle.clone();
        let key_handler =
            EventHandler::new(positioned_content).on_keydown(move |ctx, app, keystroke| {
                if focus_handle
                    .as_ref()
                    .is_some_and(|handle| !handle.is_focused(app))
                {
                    return DispatchEventResult::PropagateToParent;
                }
                // FM pane-mode toggle: the same chord that opened the file manager
                // (cmd-shift-E on mac, ctrl-alt-e elsewhere) closes it back to the
                // terminal. Handled here, *before* the modifier guard below, since it
                // is a modifier chord the guard would otherwise propagate away. Key is
                // matched case-insensitively (shift may fold "e"→"E" at runtime).
                let is_e = keystroke.key.eq_ignore_ascii_case("e");
                let toggle_back =
                    (is_e && keystroke.cmd && keystroke.shift && !keystroke.ctrl && !keystroke.alt)
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
                if let Some(action) = pane_cycle_action(&keystroke.key, keystroke.shift) {
                    ctx.dispatch_typed_action(action);
                    return DispatchEventResult::StopPropagation;
                }
                let action =
                    function_key_action(&keystroke.key).or_else(|| match keystroke.key.as_str() {
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
                        // MC's Insert: mark and step down. Space marks in place.
                        "insert" => Some(SftpBrowserAction::MarkAndAdvance),
                        "delete" => Some(SftpBrowserAction::DeleteSelected),
                        "escape" => Some(SftpBrowserAction::CloseDialog),
                        _ => None,
                    });
                match action {
                    Some(action) => {
                        ctx.dispatch_typed_action(action);
                        DispatchEventResult::StopPropagation
                    }
                    None => DispatchEventResult::PropagateToParent,
                }
            });

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
        // Legacy, pane-owned transfers are cancelled. Global queue jobs keep
        // running and remain visible from other panes/workspaces.
        for task in &self.transfers {
            if !self.transfer_queue_ids.contains_key(&task.id) {
                task.cancel();
            }
        }
        // Restore directory-move sources while this view still owns the backend.
        for batch in self.pending_dir_move_cleanups.drain(..) {
            if let Some(original_source_dir) = batch.original_source_dir {
                if let Err(error) = batch
                    .source_backend
                    .rename(&batch.source_dir, &original_source_dir)
                {
                    log::error!(
                        "fm close recovery required: source remains at {} because restoring {} failed: {error}",
                        batch.source_dir.display(),
                        original_source_dir.display()
                    );
                }
            }
        }
        self.pending_uploads.clear();
        self.connect_handle = None;
        self.refresh_handle = None;
        self._session = None;
        self.sftp = None;
        self.connection = ConnectionState::Disconnected;
        // Stop advertising this pane as a copy/move target.
        self.deregister_from_registry(ctx);
        // Pick mode closed without a pick (cancel / pane-close): re-show the
        // hidden spawn card so its selections aren't stranded (#105). Covers every
        // teardown path (F10, pane-X, dismiss) since they all route through
        // `close()`; skipped after a successful pick (`pick_resolved`).
        if self.pick_mode && !self.pick_resolved {
            ctx.dispatch_typed_action(&crate::WorkspaceAction::RemoteSpawnDirPickCanceled);
        }
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
        let title = crate::t!("fm-title-sftp", path = path.to_string());
        view::HeaderContent::simple(title)
    }

    /// Set the focus handle
    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>) {
        ctx.subscribe_to_model(focus_handle.focus_state_handle(), |_, _, _, ctx| {
            ctx.notify()
        });
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
#[path = "browser_unit_tests.rs"]
mod tests;
