//! SFTP manager UI layer type definitions
//!
//! author: logic
//! date: 2026-05-26

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// File entry type (UI layer)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEntryType {
    File,
    Directory,
    Symlink,
    Other,
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
}

/// Transfer direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

/// Transfer state
#[derive(Debug, Clone)]
pub enum TransferState {
    Pending,
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

/// Transfer task
#[derive(Debug, Clone)]
pub struct TransferTask {
    /// Task ID
    pub id: usize,
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
    /// Transfer state
    pub state: TransferState,
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
            source_path,
            target_path,
            direction,
            total_size,
            transferred: 0,
            state: TransferState::Pending,
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

/// Dialog type
#[derive(Debug, Clone)]
pub enum Dialog {
    DeleteConfirm {
        paths: Vec<PathBuf>,
        /// Whether each path is a directory, corresponding 1-to-1 with paths
        is_dirs: Vec<bool>,
    },
    Rename {
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
    FileDetails { entry: FileEntry },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    /// Test format_size with zero bytes
    #[test]
    fn test_format_size_zero() {
        assert_eq!(format_size(0), "0 B");
    }

    /// Test format_size with byte-level values
    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    /// Test format_size with KB-level values
    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 512), "512.0 KB");
    }

    /// Test format_size with MB-level values
    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(2 * 1024 * 1024 + 512 * 1024), "2.5 MB");
    }

    /// Test format_size with GB-level values
    #[test]
    fn test_format_size_gb() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    /// Test TransferTask creation
    #[test]
    fn test_transfer_task_new() {
        let task = TransferTask::new(
            1,
            PathBuf::from("/remote/file.txt"),
            PathBuf::from("/local/file.txt"),
            TransferDirection::Download,
            1024,
        );
        assert_eq!(task.id, 1);
        assert_eq!(task.total_size, 1024);
        assert_eq!(task.transferred, 0);
        assert!(matches!(task.state, TransferState::Pending));
        assert!(!task.is_cancelled());
    }

    /// Test TransferTask progress when total_size is zero
    #[test]
    fn test_transfer_task_progress_zero() {
        let task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            0,
        );
        assert_eq!(task.progress_percent(), 0);
    }

    /// Test TransferTask progress at 50%
    #[test]
    fn test_transfer_task_progress_half() {
        let mut task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            1000,
        );
        task.transferred = 500;
        assert_eq!(task.progress_percent(), 50);
    }

    /// Test TransferTask progress at 100%
    #[test]
    fn test_transfer_task_progress_full() {
        let mut task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Download,
            1000,
        );
        task.transferred = 1000;
        assert_eq!(task.progress_percent(), 100);
    }

    /// Test TransferTask progress percentage rounding
    #[test]
    fn test_transfer_task_progress_rounding() {
        let mut task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            3,
        );
        task.transferred = 1;
        assert_eq!(task.progress_percent(), 33);
    }

    /// Test TransferTask cancel operation
    #[test]
    fn test_transfer_task_cancel() {
        let task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            100,
        );
        assert!(!task.is_cancelled());
        task.cancel();
        assert!(task.is_cancelled());
    }

    /// Test TransferTask cancel flag sharing
    #[test]
    fn test_transfer_task_cancel_flag_shared() {
        let task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Download,
            100,
        );
        let flag = task.cancel_flag.clone();
        flag.store(true, Ordering::SeqCst);
        assert!(task.is_cancelled());
    }

    /// Test FileEntryType equality
    #[test]
    fn test_file_entry_type_equality() {
        assert_eq!(FileEntryType::File, FileEntryType::File);
        assert_eq!(FileEntryType::Directory, FileEntryType::Directory);
        assert_ne!(FileEntryType::File, FileEntryType::Directory);
    }

    /// Test TransferDirection equality
    #[test]
    fn test_transfer_direction_equality() {
        assert_eq!(TransferDirection::Upload, TransferDirection::Upload);
        assert_eq!(TransferDirection::Download, TransferDirection::Download);
        assert_ne!(TransferDirection::Upload, TransferDirection::Download);
    }

    /// Test ConnectionState Debug output
    #[test]
    fn test_connection_state_debug() {
        let states = vec![
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Disconnected,
            ConnectionState::Failed("timeout".into()),
        ];
        for state in &states {
            let debug_str = format!("{state:?}");
            assert!(!debug_str.is_empty());
        }
    }

    /// Test Dialog enum variants
    #[test]
    fn test_dialog_variants() {
        let delete = Dialog::DeleteConfirm {
            paths: vec![PathBuf::from("/foo")],
            is_dirs: vec![false],
        };
        assert!(matches!(delete, Dialog::DeleteConfirm { .. }));

        let rename = Dialog::Rename {
            path: PathBuf::from("/old"),
            original_name: "old".into(),
        };
        assert!(matches!(rename, Dialog::Rename { .. }));

        let folder = Dialog::CreateFolder {
            parent_path: PathBuf::from("/home"),
        };
        assert!(matches!(folder, Dialog::CreateFolder { .. }));

        let details = Dialog::FileDetails {
            entry: FileEntry {
                name: "test.txt".into(),
                path: PathBuf::from("/test.txt"),
                file_type: FileEntryType::File,
                size: 100,
                modified: None,
                permissions: None,
            },
        };
        assert!(matches!(details, Dialog::FileDetails { .. }));
    }

    /// Test Dialog::Move variant
    #[test]
    fn test_dialog_move_variant() {
        let dialog = Dialog::Move {
            source: PathBuf::from("/home/user/file.txt"),
            target_dir: PathBuf::from("/home/user/backup"),
        };
        assert!(matches!(dialog, Dialog::Move { .. }));
    }

    /// Test Dialog::OverwriteConfirm variant
    #[test]
    fn test_dialog_overwrite_confirm_variant() {
        let dialog = Dialog::OverwriteConfirm {
            source: PathBuf::from("/home/user/file.txt"),
            target: PathBuf::from("/home/user/file_copy.txt"),
            file_size: 1024,
            direction: TransferDirection::Download,
        };
        assert!(matches!(dialog, Dialog::OverwriteConfirm { .. }));
    }

    /// Test TransferState variants Debug output
    #[test]
    fn test_transfer_state_variants() {
        assert!(matches!(TransferState::Pending, TransferState::Pending));
        assert!(matches!(TransferState::InProgress, TransferState::InProgress));
        assert!(matches!(TransferState::Completed, TransferState::Completed));
        assert!(matches!(TransferState::Cancelled, TransferState::Cancelled));
        let failed = TransferState::Failed("io error".into());
        assert!(matches!(failed, TransferState::Failed(_)));
    }

    /// Test format_size with exactly 1 KB
    #[test]
    fn test_format_size_exact_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
    }

    /// Test format_size with exactly 1 MB
    #[test]
    fn test_format_size_exact_mb() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }

    /// Test format_size with exactly 1 GB
    #[test]
    fn test_format_size_exact_gb() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }

    /// Test format_size with 1 byte
    #[test]
    fn test_format_size_one_byte() {
        assert_eq!(format_size(1), "1 B");
    }

    /// Test format_size with large values
    #[test]
    fn test_format_size_large() {
        let size = 5 * 1024 * 1024 * 1024u64; // 5 GB
        assert_eq!(format_size(size), "5.0 GB");
    }

    /// Test format_size near boundary (1023 B)
    #[test]
    fn test_format_size_near_kb_boundary() {
        assert_eq!(format_size(1023), "1023 B");
    }

    /// Test TransferTask Clone consistency
    #[test]
    fn test_transfer_task_clone() {
        let task = TransferTask::new(
            42,
            PathBuf::from("/src"),
            PathBuf::from("/dst"),
            TransferDirection::Download,
            999,
        );
        let cloned = task.clone();
        assert_eq!(cloned.id, 42);
        assert_eq!(cloned.total_size, 999);
        assert_eq!(cloned.direction, TransferDirection::Download);
    }

    /// Test FileEntry Clone consistency
    #[test]
    fn test_file_entry_clone() {
        let entry = FileEntry {
            name: "doc.txt".into(),
            path: PathBuf::from("/home/doc.txt"),
            file_type: FileEntryType::File,
            size: 2048,
            modified: Some("2026-01-01".into()),
            permissions: Some("rw-r--r--".into()),
        };
        let cloned = entry.clone();
        assert_eq!(cloned.name, "doc.txt");
        assert_eq!(cloned.size, 2048);
        assert_eq!(cloned.modified, Some("2026-01-01".into()));
    }

    // ==================== Supplementary boundary case tests ====================

    /// Test format_size with extreme value (u64::MAX)
    #[test]
    fn test_format_size_u64_max() {
        let result = format_size(u64::MAX);
        assert!(result.contains("GB"), "u64::MAX should be in GB units: {result}");
    }

    /// Test format_size near MB boundary
    #[test]
    fn test_format_size_near_mb_boundary() {
        let just_below_mb = 1024 * 1024 - 1;
        assert_eq!(format_size(just_below_mb), "1024.0 KB");
    }

    /// Test TransferTask progress_percent return value when exceeding range
    #[test]
    fn test_transfer_task_progress_over_100() {
        let mut task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            100,
        );
        task.transferred = 200;
        let pct = task.progress_percent();
        assert_eq!(pct, 100, "when transferred > total_size progress is capped to 100%");
    }

    /// Test TransferTask progress_percent fractional truncation
    #[test]
    fn test_transfer_task_progress_truncation() {
        let mut task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            7,
        );
        task.transferred = 1;
        let pct = task.progress_percent();
        assert_eq!(pct, 14, "1/7 ≈ 14.28%, truncated to 14");
    }

    /// Test TransferTask cancel idempotency
    #[test]
    fn test_transfer_task_cancel_idempotent() {
        let task = TransferTask::new(
            1,
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            TransferDirection::Upload,
            100,
        );
        task.cancel();
        assert!(task.is_cancelled());
        task.cancel();
        assert!(task.is_cancelled());
    }

    /// Test TransferState::Failed with empty string
    #[test]
    fn test_transfer_state_failed_empty() {
        let state = TransferState::Failed(String::new());
        assert!(matches!(state, TransferState::Failed(_)));
        let debug = format!("{state:?}");
        assert!(!debug.is_empty());
    }

    /// Test ConnectionState::Failed with empty string
    #[test]
    fn test_connection_state_failed_empty() {
        let state = ConnectionState::Failed(String::new());
        let debug = format!("{state:?}");
        assert!(!debug.is_empty());
    }

    /// Test Dialog::DeleteConfirm with empty paths list
    #[test]
    fn test_dialog_delete_confirm_empty_paths() {
        let dialog = Dialog::DeleteConfirm { paths: vec![], is_dirs: vec![] };
        assert!(matches!(dialog, Dialog::DeleteConfirm { .. }));
    }

    /// Test FileEntry with all fields empty/zero-valued
    #[test]
    fn test_file_entry_default_values() {
        let entry = FileEntry {
            name: String::new(),
            path: PathBuf::new(),
            file_type: FileEntryType::Other,
            size: 0,
            modified: None,
            permissions: None,
        };
        assert!(entry.name.is_empty());
        assert_eq!(entry.size, 0);
        assert!(entry.modified.is_none());
    }
}
