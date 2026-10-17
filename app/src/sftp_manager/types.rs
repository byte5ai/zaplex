//! SFTP manager UI layer type definitions
//!
//! author: logic
//! date: 2026-05-26

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// File entry type (UI layer)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

/// Backend-provided identity for one filesystem object and its current revision.
///
/// `object_id` identifies the object independently of its list position, while
/// `revision` changes when the backend can observe that the object changed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StableEntryIdentity {
    pub file_type: FileEntryType,
    pub size: u64,
    pub object_id: String,
    pub revision: String,
}

/// Stable key used by marks across list refreshes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntryIdentity {
    pub path: PathBuf,
    pub backend: StableEntryIdentity,
}

/// Generation-bound reference captured by a rendered row action.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntryReference {
    pub listing_generation: u64,
    pub identity: EntryIdentity,
}

/// File entry (for UI display)
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File name
    pub name: String,
    /// Full path
    pub path: PathBuf,
    /// File type
    pub file_type: FileEntryType,
    /// File size (bytes)
    pub size: u64,
    /// Modification time
    pub modified: Option<String>,
    /// Permission string
    pub permissions: Option<String>,
    /// Stable backend identity captured with this listing entry.
    pub identity: StableEntryIdentity,
}

impl FileEntry {
    pub fn entry_identity(&self) -> EntryIdentity {
        EntryIdentity {
            path: self.path.clone(),
            backend: self.identity.clone(),
        }
    }

    pub fn entry_reference(&self, listing_generation: u64) -> EntryReference {
        EntryReference {
            listing_generation,
            identity: self.entry_identity(),
        }
    }
}

/// Transfer direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
    Copy,
}

/// Transfer state
#[derive(Debug, Clone)]
pub enum TransferState {
    Pending,
    InProgress,
    Paused,
    Cancelling,
    Completed,
    Skipped,
    PartiallyCompleted {
        transferred: usize,
        published: usize,
        skipped: usize,
        source_kept: bool,
    },
    CompletedWithWarning(String),
    Failed(String),
    Cancelled,
}

/// The current transfer phase shown in global activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransferPhase {
    #[default]
    Transferring,
    Verifying,
    Finalizing,
}

/// Transfer task
#[derive(Debug, Clone)]
pub struct TransferTask {
    /// Task ID
    pub id: usize,
    /// Queue control generation captured when this task was rendered.
    pub control_epoch: Option<u64>,
    /// Source path
    pub source_path: PathBuf,
    /// Target path
    pub target_path: PathBuf,
    /// Transfer direction
    pub direction: TransferDirection,
    /// Total size (bytes)
    pub total_size: u64,
    /// Transferred size (bytes)
    pub transferred: u64,
    /// Current average transfer throughput in bytes per second.
    pub bytes_per_second: u64,
    /// Estimated time remaining while the transfer is active.
    pub eta: Option<Duration>,
    /// Current streaming or verification phase.
    pub phase: TransferPhase,
    /// Transfer state
    pub state: TransferState,
    /// Paths retained after an indeterminate or incomplete cleanup.
    pub recovery_paths: Vec<PathBuf>,
    /// Whether the retained cleanup can be retried safely.
    pub recovery_retryable: bool,
    /// Cancel flag
    pub cancel_flag: Arc<AtomicBool>,
}

impl TransferTask {
    /// Create a new transfer task
    pub fn new(
        id: usize,
        source_path: PathBuf,
        target_path: PathBuf,
        direction: TransferDirection,
        total_size: u64,
    ) -> Self {
        Self {
            id,
            control_epoch: None,
            source_path,
            target_path,
            direction,
            total_size,
            transferred: 0,
            bytes_per_second: 0,
            eta: None,
            phase: TransferPhase::Transferring,
            state: TransferState::Pending,
            recovery_paths: Vec::new(),
            recovery_retryable: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Calculate progress percentage (0-100), capped at 100 when exceeded
    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        let calculated = ((self.transferred as f64 / self.total_size as f64) * 100.0) as u8;
        calculated.min(100)
    }

    /// Cancel transfer
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    /// Check if transfer was cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }
}

/// One candidate of the transfer target picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPickerRow {
    /// `host · path`, disambiguated among the listed candidates.
    pub label: String,
    /// The pane lives in another tab than the source pane.
    pub other_tab: bool,
    /// The pane is the source's current transfer target.
    pub selected: bool,
}

/// Dialog type
#[derive(Debug, Clone)]
pub enum Dialog {
    DeleteConfirm {
        /// Generation-bound identities captured when the dialog was opened.
        entries: Vec<EntryReference>,
        paths: Vec<PathBuf>,
        /// Whether each path is a directory, corresponding 1-to-1 with paths
        is_dirs: Vec<bool>,
    },
    Rename {
        /// Generation-bound identity captured when the dialog was opened.
        entry: EntryReference,
        path: PathBuf,
        original_name: String,
    },
    CreateFolder {
        parent_path: PathBuf,
    },
    Move {
        source: PathBuf,
        target_dir: PathBuf,
    },
    OverwriteConfirm {
        source: PathBuf,
        target: PathBuf,
        file_size: u64,
        direction: TransferDirection,
    },
    /// An unrecognized SSH server key that must be approved explicitly.
    ConfirmUnknownHostKey {
        host: String,
        port: u16,
        fingerprint_sha256: String,
        key_type: String,
    },
    /// A known SSH endpoint presented a different key. Replacing the stored
    /// identity requires a separate, explicit warning dialog.
    ConfirmChangedHostKey {
        host: String,
        port: u16,
        fingerprint_sha256: String,
        key_type: String,
    },
    /// A copy/move batch hit an existing target: ask the user per item, with
    /// "…all" options that apply to the rest of the batch.
    CopyMoveConflict {
        /// Name of the conflicting item.
        name: String,
        /// How many conflicts (including this one) are still queued.
        remaining: usize,
        /// Move vs copy (wording only).
        is_move: bool,
    },
    /// Choose the pane to copy/move into: F5/F6 without a single unambiguous
    /// destination, Shift-F5/F6, or the target footer. The candidate
    /// descriptors (with ids, for routing) live on the view in
    /// `pending_target_pick`; this carries only what is displayed.
    CopyMoveTargetPicker {
        /// Move vs copy of the pending operation; `None` when the picker only
        /// chooses the pane's transfer target (wording only).
        is_move: Option<bool>,
        /// What the pending operation transfers and from where.
        source: Option<String>,
        /// One row per candidate pane, in `pending_target_pick` order.
        rows: Vec<TargetPickerRow>,
    },
    /// A cross-connection copy/move found files that already exist on the
    /// destination: ask once whether to overwrite them all or skip them (the
    /// non-conflicting files start transferring immediately). The conflicting
    /// transfers live on the view in `pending_cross_conn`.
    CrossConnConflict {
        /// How many destination files already exist.
        existing: usize,
        /// Move vs copy (wording only).
        is_move: bool,
    },
    FileDetails {
        entry: FileEntry,
    },
    /// Confirm closing transfer panel (when active transfers exist)
    CloseTransferPanelConfirm,
}

/// Connection state
#[derive(Debug)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Failed(String),
}

/// Format file size as human-readable string
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{size} B")
    }
}
