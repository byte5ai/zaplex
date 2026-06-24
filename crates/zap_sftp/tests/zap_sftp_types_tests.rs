//! Unit tests for zap_sftp::types module
//!
//! author: logic
//! date: 2026/05/26

use zap_sftp::types::*;

// ============================================================
// FileType::from_mode tests
// ============================================================

/// Verify that 0o040000 is parsed as Dir
#[test]
fn test_file_type_from_mode_dir() {
    let ft = FileType::from_mode(0o040000);
    assert_eq!(ft, FileType::Dir);
}

/// Verify that 0o100000 is parsed as File
#[test]
fn test_file_type_from_mode_file() {
    let ft = FileType::from_mode(0o100000);
    assert_eq!(ft, FileType::File);
}

/// Verify that 0o120000 is parsed as Symlink
#[test]
fn test_file_type_from_mode_symlink() {
    let ft = FileType::from_mode(0o120000);
    assert_eq!(ft, FileType::Symlink);
}

/// Verify that 0o000000 is parsed as Other
#[test]
fn test_file_type_from_mode_other() {
    let ft = FileType::from_mode(0o000000);
    assert_eq!(ft, FileType::Other);
}

/// Verify that unknown type 0o050000 is also parsed as Other
#[test]
fn test_file_type_from_mode_unknown() {
    let ft = FileType::from_mode(0o050000);
    assert_eq!(ft, FileType::Other);
}

// ============================================================
// FilePermissions::from_mode tests
// ============================================================

/// Verify that 0o755 => rwxr-xr-x
#[test]
fn test_file_permissions_from_mode_755() {
    let p = FilePermissions::from_mode(0o755);
    assert!(p.owner_read, "owner_read should be true");
    assert!(p.owner_write, "owner_write should be true");
    assert!(p.owner_exec, "owner_exec should be true");
    assert!(p.group_read, "group_read should be true");
    assert!(!p.group_write, "group_write should be false");
    assert!(p.group_exec, "group_exec should be true");
    assert!(p.other_read, "other_read should be true");
    assert!(!p.other_write, "other_write should be false");
    assert!(p.other_exec, "other_exec should be true");
}

/// Verify that 0o644 => rw-r--r--
#[test]
fn test_file_permissions_from_mode_644() {
    let p = FilePermissions::from_mode(0o644);
    assert!(p.owner_read, "owner_read should be true");
    assert!(p.owner_write, "owner_write should be true");
    assert!(!p.owner_exec, "owner_exec should be false");
    assert!(p.group_read, "group_read should be true");
    assert!(!p.group_write, "group_write should be false");
    assert!(!p.group_exec, "group_exec should be false");
    assert!(p.other_read, "other_read should be true");
    assert!(!p.other_write, "other_write should be false");
    assert!(!p.other_exec, "other_exec should be false");
}

/// Verify that 0o777 => all bits are true
#[test]
fn test_file_permissions_from_mode_777() {
    let p = FilePermissions::from_mode(0o777);
    assert!(p.owner_read && p.owner_write && p.owner_exec, "owner bits should all be true");
    assert!(p.group_read && p.group_write && p.group_exec, "group bits should all be true");
    assert!(p.other_read && p.other_write && p.other_exec, "other bits should all be true");
}

/// Verify that 0o000 => all bits are false
#[test]
fn test_file_permissions_from_mode_000() {
    let p = FilePermissions::from_mode(0o000);
    assert!(!p.owner_read && !p.owner_write && !p.owner_exec, "owner bits should all be false");
    assert!(!p.group_read && !p.group_write && !p.group_exec, "group bits should all be false");
    assert!(!p.other_read && !p.other_write && !p.other_exec, "other bits should all be false");
}

/// Verify that 0o111 => only execute bits are true
#[test]
fn test_file_permissions_from_mode_exec_only() {
    let p = FilePermissions::from_mode(0o111);
    assert!(!p.owner_read, "owner_read should be false");
    assert!(!p.owner_write, "owner_write should be false");
    assert!(p.owner_exec, "owner_exec should be true");
    assert!(!p.group_read, "group_read should be false");
    assert!(!p.group_write, "group_write should be false");
    assert!(p.group_exec, "group_exec should be true");
    assert!(!p.other_read, "other_read should be false");
    assert!(!p.other_write, "other_write should be false");
    assert!(p.other_exec, "other_exec should be true");
}

// ============================================================
// OpenOptions constructor tests
// ============================================================

/// Verify the field values of OpenOptions constructed by read()
#[test]
fn test_open_options_read() {
    let opts = OpenOptions::read();
    assert!(opts.read, "read should be true");
    assert!(opts.write.is_none(), "write should be None");
    assert!(!opts.create, "create should be false");
    assert!(!opts.truncate, "truncate should be false");
    assert_eq!(opts.file_type, OpenFileType::File);
}

/// Verify the field values of OpenOptions constructed by write()
#[test]
fn test_open_options_write() {
    let opts = OpenOptions::write();
    assert!(!opts.read, "read should be false");
    assert_eq!(opts.write, Some(WriteMode::Write), "write should be Some(Write)");
    assert!(opts.create, "create should be true");
    assert!(opts.truncate, "truncate should be true");
    assert_eq!(opts.mode, Some(0o644), "mode should be Some(0o644)");
    assert_eq!(opts.file_type, OpenFileType::File);
}

/// Verify the field values of OpenOptions constructed by append()
#[test]
fn test_open_options_append() {
    let opts = OpenOptions::append();
    assert!(!opts.read, "read should be false");
    assert_eq!(opts.write, Some(WriteMode::Append), "write should be Some(Append)");
    assert!(opts.create, "create should be true");
    assert!(!opts.truncate, "truncate should be false");
}

/// Verify the field values of OpenOptions constructed by create_new()
#[test]
fn test_open_options_create_new() {
    let opts = OpenOptions::create_new();
    assert!(!opts.read, "read should be false");
    assert_eq!(opts.write, Some(WriteMode::Write), "write should be Some(Write)");
    assert!(opts.create, "create should be true");
    assert!(!opts.truncate, "truncate should be false");
}

// ============================================================
// RenameOptions Default tests
// ============================================================

/// Verify that RenameOptions default values are all false
#[test]
fn test_rename_options_default() {
    let opts = RenameOptions::default();
    assert!(!opts.overwrite, "overwrite should be false");
    assert!(!opts.atomic, "atomic should be false");
    assert!(!opts.native, "native should be false");
}
