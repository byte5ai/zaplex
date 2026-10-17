//! Unit tests for zap_sftp::types::Metadata::from_ssh2 module
//!
//! Validates logic for creating Metadata from ssh2::FileStat,
//! with emphasis on symlink detection fix and Some/None fallbacks for each field.
//! author: logic
//! date: 2026-05-27

use std::time::{Duration, SystemTime};

use zap_sftp::types::*;

/// Construct an empty ssh2::FileStat with all fields as None
fn empty_stat() -> ssh2::FileStat {
    ssh2::FileStat {
        size: None,
        uid: None,
        gid: None,
        perm: None,
        atime: None,
        mtime: None,
    }
}

// ============================================================
// Metadata::from_ssh2 — File type detection
// ============================================================

/// Verify file_type is Symlink when perm contains symlink mode bits (fix verification)
#[test]
fn test_metadata_from_ssh2_symlink() {
    let stat = ssh2::FileStat {
        perm: Some(0o120755),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert_eq!(meta.file_type, FileType::Symlink);
}

// ============================================================
// Metadata::from_ssh2 — Permission fields
// ============================================================

/// Verify permission bits are parsed correctly
#[test]
fn test_metadata_from_ssh2_permissions() {
    let stat = ssh2::FileStat {
        perm: Some(0o100755),
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert!(meta.permissions.owner_read);
    assert!(meta.permissions.owner_write);
    assert!(meta.permissions.owner_exec);
    assert!(meta.permissions.group_read);
    assert!(!meta.permissions.group_write);
    assert!(meta.permissions.group_exec);
    assert_eq!(meta.mode, Some(0o755));
}

/// Verify all permissions are false when perm is None
#[test]
fn test_metadata_from_ssh2_permissions_none() {
    let stat = ssh2::FileStat {
        perm: None,
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    assert!(!meta.permissions.owner_read);
    assert!(!meta.permissions.owner_write);
    assert_eq!(meta.mode, None);
}

// ============================================================
// Metadata::from_ssh2 — Timestamps
// ============================================================

/// Verify atime/mtime are correctly converted to SystemTime
#[test]
fn test_metadata_from_ssh2_timestamps_present() {
    let stat = ssh2::FileStat {
        perm: Some(0o100644),
        atime: Some(1609459200), // 2021-01-01 00:00:00 UTC
        mtime: Some(1609545600), // 2021-01-02 00:00:00 UTC
        ..empty_stat()
    };
    let meta = Metadata::from_ssh2(stat);
    let expected_atime = SystemTime::UNIX_EPOCH + Duration::from_secs(1609459200);
    let expected_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1609545600);
    assert_eq!(meta.accessed, Some(expected_atime));
    assert_eq!(meta.modified, Some(expected_mtime));
}
