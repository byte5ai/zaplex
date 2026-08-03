//! SFTP backend operation abstraction layer.
//!
//! Defines the SftpBackend trait to decouple the UI layer from the protocol layer.
//! LiveSftpBackend delegates to a real SFTP connection, and InMemorySftpBackend uses the local filesystem for testing.
//! author: logic
//! date: 2026-05-30

use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dunce;
use parking_lot::RwLock;
use remote_server::client::RemoteServerClient;
use remote_server::proto::{
    safe_file_request, safe_file_response, SafeFileCreateExclusive, SafeFileDelete,
    SafeFileEntryKind, SafeFileFlushHandle, SafeFileIdentity, SafeFileInspectHandle,
    SafeFileInspectResult, SafeFileListRecoveries, SafeFileOpenExisting, SafeFileReadHandle,
    SafeFileRename, SafeFileRenameMode, SafeFileRequest, SafeFileRetryRecovery,
    SafeFileWriteHandle,
};
use sha2::{Digest, Sha256};

use super::sftp_ops::{self, ProgressCallback, SftpOpsError};
pub use super::types::StableEntryIdentity;
use super::types::{FileEntry, FileEntryType};

pub(crate) const fn local_safe_rename_primitives_available() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

const DIRECTORY_RESERVATION_NAMESPACE_PREFIX: &str = ".zaplex-directory-reservations-v1";
const DIRECTORY_RESERVATION_NAMESPACE_MARKER: &str = ".zaplex-owned-namespace-v1";
const DIRECTORY_RESERVATION_MARKER_SUFFIX: &str = ".zaplex-owned-directory-v1";
const DIRECTORY_RESERVATION_REGISTRY_VERSION: &str = "zaplex-directory-reservation-registry-v2";
const LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION: &str =
    "zaplex-directory-reservation-registry-v1";
const TRANSFER_ARTIFACT_REGISTRY_VERSION: &str = "zaplex-transfer-artifact-registry-v2";
const LEGACY_TRANSFER_ARTIFACT_REGISTRY_VERSION: &str = "zaplex-transfer-artifact-registry-v1";
const TRANSFER_EXCHANGE_REGISTRY_VERSION: &str = "zaplex-transfer-exchange-registry-v2";
const LEGACY_TRANSFER_EXCHANGE_REGISTRY_VERSION: &str = "zaplex-transfer-exchange-registry-v1";
const NAMESPACE_RECORD_TEMPORARY: &str = ".namespace-write.tmp";
const NAMESPACE_MIGRATION_TEMPORARY: &str = ".namespace-migration-write.tmp";
const EXCHANGE_RECORD_TEMPORARY: &str = ".exchange-write.tmp";
const ARTIFACT_RECORD_TEMPORARY: &str = ".artifact-write.tmp";

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hmac_sha256(secret: &[u8], message: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;

    let mut key = [0_u8; BLOCK_SIZE];
    if secret.len() > BLOCK_SIZE {
        key[..32].copy_from_slice(&Sha256::digest(secret));
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= key[index];
        outer_pad[index] ^= key[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    hex_bytes(&outer.finalize())
}

fn secure_compare(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

/// Opaque handle that keeps an exclusively reserved backend object alive and
/// can prove that a later path still names that exact object.
pub trait BackendOwnershipAnchor: Send + Sync {
    fn identity(&self) -> Result<StableEntryIdentity, SftpOpsError>;
    fn matches_path(&self, path: &Path) -> Result<bool, SftpOpsError>;
    fn link_count(&self) -> Result<Option<u64>, SftpOpsError> {
        Ok(None)
    }
    fn matches_local_path(&self, _path: &Path) -> Result<bool, SftpOpsError> {
        Err(SftpOpsError::Operation(
            "The backend ownership anchor cannot validate host-local paths".to_string(),
        ))
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryReservationFailure {
    Open,
    Identity,
    Match,
    Publish,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamespaceScanFailure {
    ReadDirectory,
    DirectoryEntry,
    MarkerFileType,
    UnclaimedFileType,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamespaceProbeFailure {
    Record,
    NamespacePath,
    Parent,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SiblingRecoveryFailure {
    ReadDirectory,
    DirectoryEntry,
    AnchorProbe,
    RegistryWrite,
}

fn has_immutable_object_token(identity: &StableEntryIdentity) -> bool {
    !identity.object_id.is_empty()
}

fn same_immutable_object(expected: &StableEntryIdentity, actual: &StableEntryIdentity) -> bool {
    expected.file_type == actual.file_type
        && has_immutable_object_token(expected)
        && expected.object_id == actual.object_id
}

/// Chunk reader used by the cross-backend transfer engine.
pub trait BackendFileReader: Send {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, SftpOpsError>;
}

/// Chunk writer used by the cross-backend transfer engine.
pub trait BackendFileWriter: Send {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError>;
    fn flush(&mut self) -> Result<(), SftpOpsError>;

    /// Returns a live handle to the exclusively reserved file. Path-only
    /// implementations must return `None` rather than reconstructing ownership
    /// after reservation.
    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        Ok(None)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_cstring(path: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

#[cfg(target_os = "linux")]
fn rename_noreplace(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    let old_path = path_cstring(old_path)?;
    let new_path = path_cstring(new_path)?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    let old_path = path_cstring(old_path)?;
    let new_path = path_cstring(new_path)?;
    let result =
        unsafe { libc::renamex_np(old_path.as_ptr(), new_path.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_noreplace(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    fs::rename(old_path, new_path)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_noreplace(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(old_path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic no-replace directory rename is unsupported on this platform",
        ));
    }
    fs::hard_link(old_path, new_path)?;
    fs::remove_file(old_path)
}

#[cfg(not(any(unix, windows)))]
fn rename_noreplace(_old_path: &Path, _new_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    let old_path_c = path_cstring(old_path)?;
    let new_path_c = path_cstring(new_path)?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old_path_c.as_ptr(),
            libc::AT_FDCWD,
            new_path_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    let old_path_c = path_cstring(old_path)?;
    let new_path_c = path_cstring(new_path)?;
    let result =
        unsafe { libc::renamex_np(old_path_c.as_ptr(), new_path_c.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "atomic exchange is unsupported for {} <-> {}",
            old_path.display(),
            new_path.display()
        ),
    ))
}

#[cfg(windows)]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "atomic exchange is unsupported for {} <-> {}",
            old_path.display(),
            new_path.display()
        ),
    ))
}

fn validated_child_path(parent: &Path, entry: &FileEntry) -> Result<PathBuf, SftpOpsError> {
    let mut components = Path::new(&entry.name).components();
    let is_single_name = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    let expected = parent.join(&entry.name);
    if !is_single_name || entry.path != expected {
        return Err(SftpOpsError::Operation(format!(
            "Refusing unsafe directory entry {} at {}",
            entry.name,
            entry.path.display()
        )));
    }
    Ok(expected)
}

fn validate_copy_tree<B: SftpBackend + ?Sized>(
    backend: &B,
    source: &Path,
) -> Result<(), SftpOpsError> {
    for entry in backend.list_dir(source)? {
        validated_child_path(source, &entry)?;
        let actual_type = backend.lstat(&entry.path)?.file_type;
        if actual_type != entry.file_type {
            return Err(SftpOpsError::Operation(format!(
                "Source type changed during validation at {}",
                entry.path.display()
            )));
        }
        match entry.file_type {
            FileEntryType::Directory => validate_copy_tree(backend, &entry.path)?,
            FileEntryType::File => {}
            FileEntryType::Symlink => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to recursively copy symbolic link {}",
                    entry.path.display()
                )));
            }
            FileEntryType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to recursively copy special file {}",
                    entry.path.display()
                )));
            }
        }
    }
    Ok(())
}

fn copy_validated_tree<B: SftpBackend + ?Sized>(
    backend: &B,
    source: &Path,
    destination: &Path,
) -> Result<(), SftpOpsError> {
    backend.create_dir(destination)?;
    for entry in backend.list_dir(source)? {
        validated_child_path(source, &entry)?;
        let actual_type = backend.lstat(&entry.path)?.file_type;
        if actual_type != entry.file_type {
            return Err(SftpOpsError::Operation(format!(
                "Source type changed after validation at {}",
                entry.path.display()
            )));
        }
        let child_destination = destination.join(&entry.name);
        match entry.file_type {
            FileEntryType::Directory => {
                copy_validated_tree(backend, &entry.path, &child_destination)?
            }
            FileEntryType::File => backend.copy_file(&entry.path, &child_destination)?,
            FileEntryType::Symlink | FileEntryType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Source tree changed after validation at {}",
                    entry.path.display()
                )));
            }
        }
    }
    Ok(())
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

pub(crate) fn validate_copy_destination(
    source: &Path,
    destination: &Path,
    source_is_directory: bool,
) -> Result<(), SftpOpsError> {
    let source = lexical_path(source);
    let destination = lexical_path(destination);
    if source == destination {
        return Err(SftpOpsError::Operation(format!(
            "Source and destination are the same path: {}",
            source.display()
        )));
    }
    if source_is_directory && destination.starts_with(&source) {
        return Err(SftpOpsError::Operation(format!(
            "A directory cannot be copied into its own descendant: {}",
            destination.display()
        )));
    }
    Ok(())
}

/// SFTP backend operation abstraction to decouple the UI layer from the protocol layer.
pub trait SftpBackend: Send + Sync {
    /// Whether the backend can atomically exchange two existing paths.
    fn supports_atomic_exchange(&self) -> bool {
        false
    }

    /// Whether cleanup can be bound to a stable identity without path races.
    fn supports_identity_bound_cleanup(&self) -> bool {
        false
    }

    /// Returns an identity for a backend-owned cleanup artifact retained after
    /// an uncertain operation. Tokenless backends must leave the path
    /// unresolved rather than guessing ownership.
    fn cleanup_recovery_identity(&self, _path: &Path) -> Option<StableEntryIdentity> {
        None
    }

    /// Returns the live handle retained with a backend-owned cleanup artifact.
    fn cleanup_recovery_anchor(&self, _path: &Path) -> Option<Arc<dyn BackendOwnershipAnchor>> {
        None
    }

    /// Returns restart-discovered cleanup artifacts. Each path is opaque and
    /// must be resolved through the same backend instance. Unauthenticated
    /// artifacts may be reported for visibility but never receive a cleanup
    /// identity or anchor.
    fn startup_recovery_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Retries a backend-specific unresolved discovery action. A returned
    /// replacement list supersedes the supplied logical recovery path.
    fn retry_unresolved_recovery(
        &self,
        _path: &Path,
    ) -> Result<Option<Vec<PathBuf>>, SftpOpsError> {
        Ok(None)
    }

    /// Reports that a recovered backend rename committed its destination while
    /// the higher-level move source remained intact. The flag is consumed once.
    fn take_recovery_source_preserved(&self, _path: &Path) -> bool {
        false
    }

    /// Reports that recovery proved the destination did not commit and the
    /// higher-level move source therefore remains the authoritative copy.
    fn take_recovery_source_restored(&self, _path: &Path) -> bool {
        false
    }

    /// Opens a live ownership handle for an existing source entry. Backends
    /// without an immutable handle must return `None`; move operations then
    /// fail before creating a destination writer.
    fn existing_entry_ownership_anchor(
        &self,
        _path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        Ok(None)
    }

    /// Releases a backend-owned cleanup identity after it has been transferred
    /// into the process-wide recovery registry.
    fn forget_cleanup_recovery_identity(&self, _path: &Path) {}

    /// Releases backend-private path routing after a recovery unit has reached
    /// a terminal state.
    fn release_cleanup_recovery_path(&self, _path: &Path) -> Result<(), SftpOpsError> {
        Ok(())
    }

    /// Verifies the required atomic rename primitives on the destination
    /// filesystem before any transfer writer is created.
    fn preflight_safe_mutation(
        &self,
        path: &Path,
        require_exchange: bool,
    ) -> Result<(), SftpOpsError> {
        if !self.supports_identity_bound_cleanup() {
            return Err(SftpOpsError::Operation(format!(
                "Identity-bound cleanup is unsupported for {}",
                path.display()
            )));
        }
        if require_exchange && !self.supports_atomic_exchange() {
            return Err(SftpOpsError::Operation(format!(
                "Atomic exchange is unsupported for {}",
                path.display()
            )));
        }
        Ok(())
    }

    /// Lists directory contents and returns a list of file entries.
    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError>;

    /// Deletes a remote file.
    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Recursively deletes a remote directory.
    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Creates a remote directory.
    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Creates a directory and returns a live handle proving ownership of the
    /// created object. Backends that cannot provide that proof return `None`.
    fn create_dir_with_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        self.create_dir(path)?;
        Ok(None)
    }

    /// Renames a remote file or directory.
    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError>;

    /// Renames an entry only when the live ownership anchor still names the
    /// source at the final backend mutation boundary.
    fn rename_if_matches(
        &self,
        old_path: &Path,
        new_path: &Path,
        _anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Identity-bound rename is unsupported for {} -> {}",
            old_path.display(),
            new_path.display()
        )))
    }

    /// Atomically exchanges two existing paths.
    ///
    /// On success `new_path` contains the candidate and `old_path` retains the
    /// displaced destination. The safe default refuses the operation.
    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Atomic replacement is unsupported for {} -> {}",
            old_path.display(),
            new_path.display()
        )))
    }

    /// Deletes a regular file only if the backend can bind the deletion to the
    /// supplied object identity and SHA-256 content digest.
    fn delete_file_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
        expected_sha256: &str,
    ) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Identity-bound file deletion is unsupported for {} ({}, {})",
            path.display(),
            expected.object_id,
            expected_sha256
        )))
    }

    /// Deletes an empty directory only if it is still the supplied object.
    fn delete_empty_dir_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
    ) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Identity-bound directory deletion is unsupported for {} ({})",
            path.display(),
            expected.object_id
        )))
    }

    /// Resolves the real path.
    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError>;

    /// Gets file/directory details, following symbolic links to their target.
    /// Directory listings retain link metadata so destructive callers never
    /// mistake a link to a directory for the directory itself.
    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError>;

    /// Gets metadata for the path itself without following symbolic links.
    /// Use this for existence checks and every decision that may delete or
    /// overwrite the path.
    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError>;

    /// Modification time used by the "newer only" conflict policy. `None`
    /// means the backend cannot prove which side is newer.
    fn modification_time(
        &self,
        _path: &Path,
    ) -> Result<Option<std::time::SystemTime>, SftpOpsError> {
        Ok(None)
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, SftpOpsError> {
        match self.lstat(path) {
            Ok(_) => Ok(true),
            Err(SftpOpsError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Capture the identity used to revalidate a move source. Backends should
    /// override this when they expose metadata more precise than `FileEntry`.
    fn stable_identity(&self, path: &Path) -> Result<StableEntryIdentity, SftpOpsError> {
        let entry = self.lstat(path)?;
        Ok(StableEntryIdentity {
            file_type: entry.file_type,
            size: entry.size,
            object_id: entry.modified.clone().unwrap_or_default(),
            revision: entry.modified.unwrap_or_default(),
        })
    }

    /// Open an existing regular file for bounded chunk reads.
    fn open_file_reader(&self, path: &Path) -> Result<Box<dyn BackendFileReader>, SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Streaming reads are unsupported for {}",
            path.display()
        )))
    }

    /// Exclusively create a regular file for bounded chunk writes.
    fn create_file_writer(&self, path: &Path) -> Result<Box<dyn BackendFileWriter>, SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Streaming writes are unsupported for {}",
            path.display()
        )))
    }

    /// Uploads a local file to remote via streaming.
    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError>;

    /// Downloads a remote file to local via streaming.
    fn download_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError>;

    fn upload_file_no_replace(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        if self.lstat(remote_path).is_ok() {
            return Err(SftpOpsError::Operation(format!(
                "{} already exists",
                remote_path.display()
            )));
        }
        self.upload_file(local_path, remote_path, progress_cb, cancel_flag)
    }

    fn download_file_no_replace(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        if std::fs::symlink_metadata(local_path).is_ok() {
            return Err(SftpOpsError::Operation(format!(
                "{} already exists",
                local_path.display()
            )));
        }
        self.download_file(remote_path, local_path, progress_cb, cancel_flag)
    }

    /// Copies a single file *within this backend* (same filesystem namespace),
    /// e.g. between two local file-manager panes or two panes on the same host.
    /// The operation must publish transactionally: a failure leaves an existing
    /// destination unchanged. Cross-connection copy (local↔remote) is a
    /// separate transfer path.
    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError>;

    /// Recursively copies a directory within this backend. The default walks
    /// with `list_dir` + `create_dir` + `copy_file`; backends may override with
    /// a native recursive copy.
    fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        validate_copy_destination(src, dst, true)?;
        if !matches!(self.lstat(src)?.file_type, FileEntryType::Directory) {
            return Err(SftpOpsError::Operation(format!(
                "Recursive copy source is not a directory: {}",
                src.display()
            )));
        }
        validate_copy_tree(self, src)?;
        copy_validated_tree(self, src, dst)
    }
}

fn isolate_cleanup_entry(
    backend: &dyn SftpBackend,
    path: &Path,
    tombstone: &Path,
    expected: &StableEntryIdentity,
    rename_error: Option<SftpOpsError>,
) -> Result<(), SftpOpsError> {
    let same_object = |actual: &StableEntryIdentity| {
        actual.file_type == expected.file_type
            && if expected.file_type == FileEntryType::Directory && !expected.object_id.is_empty() {
                actual.object_id == expected.object_id
            } else {
                actual.size == expected.size
                    && if expected.object_id.is_empty() {
                        actual.revision == expected.revision
                    } else {
                        actual.object_id == expected.object_id
                    }
            }
    };
    let source = if backend.entry_exists(path)? {
        Some(backend.stable_identity(path)?)
    } else {
        None
    };
    let isolated = if backend.entry_exists(tombstone)? {
        Some(backend.stable_identity(tombstone)?)
    } else {
        None
    };
    match (&source, &isolated) {
        (None, Some(actual)) if same_object(actual) => Ok(()),
        (Some(actual), None) if same_object(actual) => Err(rename_error.unwrap_or_else(|| {
            SftpOpsError::Operation(format!(
                "Cleanup isolation was not committed for {}",
                path.display()
            ))
        })),
        (None, None) => Err(rename_error.unwrap_or_else(|| {
            SftpOpsError::Operation(format!(
                "Cleanup isolation paths disappeared for {}",
                path.display()
            ))
        })),
        (Some(_), None) => Err(SftpOpsError::RecoveryRequired {
            message: format!(
                "Cleanup isolation left an unexpected entry at {}",
                path.display()
            ),
            recovery_id: None,
            paths: vec![path.to_path_buf()],
            committed: false,
        }),
        (None, Some(_)) => Err(SftpOpsError::RecoveryRequired {
            message: format!(
                "Cleanup isolation retained an unexpected tombstone at {}",
                tombstone.display()
            ),
            recovery_id: None,
            paths: vec![tombstone.to_path_buf()],
            committed: false,
        }),
        (Some(_), Some(_)) => Err(SftpOpsError::RecoveryRequired {
            message: format!(
                "{}; cleanup isolation has entries at both possible paths",
                rename_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "Cleanup isolation acknowledgement is ambiguous".to_string())
            ),
            recovery_id: None,
            paths: vec![path.to_path_buf(), tombstone.to_path_buf()],
            committed: false,
        }),
    }
}

fn restore_isolated_cleanup_entry(
    backend: &dyn SftpBackend,
    path: &Path,
    tombstone: &Path,
    primary: &SftpOpsError,
) -> SftpOpsError {
    let restore_error = backend.rename(tombstone, path).err();
    let source_exists = backend.entry_exists(path);
    let tombstone_exists = backend.entry_exists(tombstone);
    match (source_exists, tombstone_exists) {
        (Ok(true), Ok(false)) => SftpOpsError::Operation(primary.to_string()),
        (source_exists, tombstone_exists) => {
            let mut paths = Vec::new();
            if !matches!(source_exists, Ok(false)) {
                paths.push(path.to_path_buf());
            }
            if !matches!(tombstone_exists, Ok(false)) {
                paths.push(tombstone.to_path_buf());
            }
            if paths.is_empty() {
                paths.extend([path.to_path_buf(), tombstone.to_path_buf()]);
            }
            SftpOpsError::RecoveryRequired {
                message: format!(
                    "{primary}; restoring isolated cleanup entry is indeterminate: {}",
                    restore_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "restore path probes disagree".to_string())
                ),
                recovery_id: None,
                paths,
                committed: false,
            }
        }
    }
}

#[cfg(unix)]
fn open_local_cleanup_anchor(path: &Path) -> Result<fs::File, SftpOpsError> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = fs::OpenOptions::new();
    #[cfg(target_os = "linux")]
    {
        options
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW);
    }
    #[cfg(not(target_os = "linux"))]
    {
        options.read(true).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|error| {
        SftpOpsError::Operation(format!(
            "Failed to anchor cleanup identity for {}: {error}",
            path.display()
        ))
    })
}

#[cfg(unix)]
/// Removes an entry that is already isolated below an authenticated private
/// cleanup namespace. Callers must never pass a user-visible parent: public
/// paths require a persisted exchange/isolation transition first.
fn unlink_from_anchored_directory(
    directory: &fs::File,
    parent: &Path,
    name: &std::ffi::OsStr,
    expected_anchor: &Arc<dyn BackendOwnershipAnchor>,
    expected_identity: &StableEntryIdentity,
    directory_entry: bool,
) -> Result<(), SftpOpsError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
        SftpOpsError::Operation("Private cleanup entry name contains NUL".to_string())
    })?;
    let physical = parent.join(std::ffi::OsStr::from_bytes(name.as_bytes()));
    let anchored_identity = expected_anchor.identity()?;
    if !same_immutable_object(expected_identity, &anchored_identity)
        || !expected_anchor.matches_local_path(&physical)?
    {
        return Err(SftpOpsError::Operation(format!(
            "Private cleanup entry changed before isolation at {}",
            physical.display()
        )));
    }
    if !directory_entry && expected_anchor.link_count()? != Some(1) {
        return Err(SftpOpsError::Operation(format!(
            "Private cleanup file has multiple links at {}",
            physical.display()
        )));
    }

    // The object is already atomically isolated below a random,
    // authenticated, mode-restricted namespace with a durable recovery
    // record. Same-UID mutation inside that namespace is outside the local
    // threat model, so another name-based exchange would only create a new
    // crash state without strengthening public-path safety.
    let removal_flags = if directory_entry {
        libc::AT_REMOVEDIR
    } else {
        0
    };
    let removed = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), removal_flags) };
    if removed != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

struct DirectoryAnchorCreateError {
    source: std::io::Error,
    reserved_identity: Option<StableEntryIdentity>,
}

#[cfg(unix)]
fn create_local_directory_with_anchor(
    path: &Path,
    #[cfg(test)] after_create: Option<&Arc<dyn Fn(&Path) + Send + Sync>>,
) -> Result<fs::File, DirectoryAnchorCreateError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::create_dir(path).map_err(|source| DirectoryAnchorCreateError {
        source,
        reserved_identity: None,
    })?;
    let reserved =
        stable_identity_from_local_metadata(&fs::symlink_metadata(path).map_err(|source| {
            DirectoryAnchorCreateError {
                source,
                reserved_identity: None,
            }
        })?);
    #[cfg(test)]
    if let Some(after_create) = after_create {
        after_create(path);
    }
    let mut options = fs::OpenOptions::new();
    #[cfg(target_os = "linux")]
    options
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW);
    #[cfg(not(target_os = "linux"))]
    options.read(true).custom_flags(libc::O_NOFOLLOW);
    let anchor = options
        .open(path)
        .map_err(|source| DirectoryAnchorCreateError {
            source,
            reserved_identity: Some(reserved.clone()),
        })?;
    let anchored = stable_identity_from_local_metadata(&anchor.metadata().map_err(|source| {
        DirectoryAnchorCreateError {
            source,
            reserved_identity: Some(reserved.clone()),
        }
    })?);
    let named =
        stable_identity_from_local_metadata(&fs::symlink_metadata(path).map_err(|source| {
            DirectoryAnchorCreateError {
                source,
                reserved_identity: Some(reserved.clone()),
            }
        })?);
    if !same_immutable_object(&reserved, &anchored)
        || !same_immutable_object(&anchored, &named)
        || reserved.revision != anchored.revision
    {
        return Err(DirectoryAnchorCreateError {
            source: std::io::Error::other(format!(
                "Created directory changed before ownership could be anchored: {}",
                path.display()
            )),
            reserved_identity: Some(reserved),
        });
    }
    Ok(anchor)
}

#[cfg(not(unix))]
fn create_local_directory_with_anchor(
    path: &Path,
    #[cfg(test)] _after_create: Option<&Arc<dyn Fn(&Path) + Send + Sync>>,
) -> Result<fs::File, DirectoryAnchorCreateError> {
    Err(DirectoryAnchorCreateError {
        source: std::io::Error::other(format!(
            "Atomic directory reservation is unsupported on this platform: {}",
            path.display()
        )),
        reserved_identity: None,
    })
}

#[cfg(target_os = "linux")]
fn open_confined_new_file(root: &Path, path: &Path) -> Result<fs::File, SftpOpsError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }

    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;

    let relative = path.strip_prefix("/").unwrap_or(path);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(SftpOpsError::Operation(format!(
            "Writer path is not a confined relative path: {}",
            path.display()
        )));
    }
    let relative = CString::new(relative.as_os_str().as_bytes()).map_err(|_| {
        SftpOpsError::Operation(format!("Writer path contains NUL: {}", path.display()))
    })?;
    let mut root_options = fs::OpenOptions::new();
    root_options
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW);
    let root = root_options.open(root)?;
    let how = OpenHow {
        flags: (libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            as u64,
        mode: 0o600,
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    let descriptor = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            relative.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { fs::File::from_raw_fd(descriptor as i32) })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn open_confined_new_file(root: &Path, path: &Path) -> Result<fs::File, SftpOpsError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    let relative = path.strip_prefix("/").unwrap_or(path);
    let components = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(component) => CString::new(component.as_bytes())
                .map_err(|_| SftpOpsError::Operation("Writer path contains NUL".to_string())),
            std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir
            | std::path::Component::Prefix(_) => Err(SftpOpsError::Operation(format!(
                "Writer path escapes the backend root: {}",
                path.display()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (leaf, parents) = components.split_last().ok_or_else(|| {
        SftpOpsError::Operation(format!("Writer path has no file name: {}", path.display()))
    })?;
    let mut root_options = fs::OpenOptions::new();
    root_options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    let mut directory = root_options.open(root)?;
    for parent in parents {
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                parent.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        directory = unsafe { fs::File::from_raw_fd(descriptor) };
    }
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[cfg(not(unix))]
fn open_confined_new_file(_root: &Path, path: &Path) -> Result<fs::File, SftpOpsError> {
    Err(SftpOpsError::Operation(format!(
        "Confined transfer writers are unsupported on this platform: {}",
        path.display()
    )))
}

#[cfg(unix)]
fn anchored_object_id(anchor: &fs::File) -> Result<String, SftpOpsError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = anchor.metadata()?;
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
fn anchored_link_count(anchor: &fs::File) -> Result<u64, SftpOpsError> {
    use std::os::unix::fs::MetadataExt;

    anchor
        .metadata()
        .map(|metadata| metadata.nlink())
        .map_err(Into::into)
}

// ============================================================
// LiveSftpBackend — Delegates to real SFTP connection
// ============================================================

/// Real SFTP backend that wraps zap_sftp::Sftp.
pub struct LiveSftpBackend {
    sftp: zap_sftp::Sftp,
    safe_files: SafeFileClientSlot,
    recovery_operations: Arc<Mutex<HashMap<PathBuf, Vec<RemoteRecoveryOperation>>>>,
    recovered_source_preserved: Arc<Mutex<HashSet<PathBuf>>>,
    recovered_source_restored: Arc<Mutex<HashSet<PathBuf>>>,
}

#[derive(Clone, Debug, PartialEq)]
struct RemoteRecoveryOperation {
    operation_id: String,
    source_preserved_after_commit: bool,
    action: RemoteRecoveryAction,
}

#[derive(Clone, Debug, PartialEq)]
enum RemoteRecoveryAction {
    Acknowledge,
    Rename {
        old_path: PathBuf,
        new_path: PathBuf,
        mode: i32,
        expected_target: Option<SafeFileIdentity>,
        source: StableEntryIdentity,
        source_is_owned_artifact: bool,
    },
    Delete(SafeFileDelete),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteRecoveryResolution {
    MutationApplied,
    DestinationCommittedSourcePreserved,
    SourceRestored,
}

/// Live safe-file capability for one SFTP pane.
///
/// The daemon transport can connect or reconnect after the SFTP channel. The
/// shared slot lets future transfer operations pick up the current negotiated
/// client without rebuilding the pane or disturbing transfers that already
/// hold their own descriptor-bound client.
#[derive(Clone, Default)]
pub(crate) struct SafeFileClientSlot {
    client: Arc<RwLock<Option<Arc<RemoteServerClient>>>>,
}

impl SafeFileClientSlot {
    fn with_client(client: Arc<RemoteServerClient>) -> Self {
        let slot = Self::default();
        slot.set(Some(client));
        slot
    }

    /// Replaces the live client and reports a transition from unavailable to
    /// available, which is when durable remote recovery records need scanning.
    pub(crate) fn set(&self, client: Option<Arc<RemoteServerClient>>) -> bool {
        let mut current = self.client.write();
        let became_available = current.is_none() && client.is_some();
        *current = client;
        became_available
    }

    fn get(&self) -> Option<Arc<RemoteServerClient>> {
        self.client.read().clone()
    }

    fn is_available(&self) -> bool {
        self.client.read().is_some()
    }
}

impl LiveSftpBackend {
    /// Creates a backend from an Sftp instance.
    pub fn new(sftp: zap_sftp::Sftp) -> Self {
        Self {
            sftp,
            safe_files: SafeFileClientSlot::default(),
            recovery_operations: Arc::new(Mutex::new(HashMap::new())),
            recovered_source_preserved: Arc::new(Mutex::new(HashSet::new())),
            recovered_source_restored: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Creates a backend with a negotiated descriptor-bound remote file service.
    pub fn new_with_safe_files(sftp: zap_sftp::Sftp, safe_files: Arc<RemoteServerClient>) -> Self {
        Self {
            sftp,
            safe_files: SafeFileClientSlot::with_client(safe_files),
            recovery_operations: Arc::new(Mutex::new(HashMap::new())),
            recovered_source_preserved: Arc::new(Mutex::new(HashSet::new())),
            recovered_source_restored: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Creates a backend whose safe-file capability follows daemon reconnects.
    pub(crate) fn new_with_safe_file_slot(
        sftp: zap_sftp::Sftp,
        safe_files: SafeFileClientSlot,
    ) -> Self {
        Self {
            sftp,
            safe_files,
            recovery_operations: Arc::new(Mutex::new(HashMap::new())),
            recovered_source_preserved: Arc::new(Mutex::new(HashSet::new())),
            recovered_source_restored: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Gets a reference to the internal Sftp instance (used for realpath calls in connect_to_server).
    pub fn inner(&self) -> &zap_sftp::Sftp {
        &self.sftp
    }

    fn safe_client(&self) -> Result<Arc<RemoteServerClient>, SftpOpsError> {
        self.safe_files.get().ok_or_else(|| {
            SftpOpsError::CapabilityRequired(
                "Secure remote file transactions are unavailable for this connection".to_string(),
            )
        })
    }

    fn open_safe_handle(
        &self,
        path: &Path,
        kind: FileEntryType,
    ) -> Result<Arc<RemoteSafeHandle>, SftpOpsError> {
        let client = self.safe_client()?;
        let path = remote_path_string(path)?;
        let response = safe_file_call(
            &client,
            String::new(),
            safe_file_request::Operation::OpenExisting(SafeFileOpenExisting {
                path,
                expected_kind: safe_kind(kind)? as i32,
            }),
        )?;
        let safe_file_response::Result::Opened(opened) = response else {
            return Err(unexpected_safe_file_response("open"));
        };
        let identity = opened.identity.ok_or_else(|| {
            SftpOpsError::Operation("Safe-file open returned no identity".to_string())
        })?;
        if identity.object_id.is_empty() {
            return Err(SftpOpsError::Operation(
                "Safe-file open returned no immutable object identity".to_string(),
            ));
        }
        Ok(Arc::new(RemoteSafeHandle {
            client,
            handle_id: opened.handle_id,
            kind,
            owned_artifact: false,
        }))
    }

    fn create_safe_handle(
        &self,
        path: &Path,
        kind: FileEntryType,
    ) -> Result<Arc<RemoteSafeHandle>, SftpOpsError> {
        let client = self.safe_client()?;
        let response = journaled_safe_file_call(
            &client,
            uuid::Uuid::new_v4().to_string(),
            safe_file_request::Operation::CreateExclusive(SafeFileCreateExclusive {
                path: remote_path_string(path)?,
                kind: safe_kind(kind)? as i32,
            }),
        )?;
        let safe_file_response::Result::Opened(opened) = response else {
            return Err(unexpected_safe_file_response("create"));
        };
        let identity = opened.identity.ok_or_else(|| {
            SftpOpsError::Operation("Safe-file create returned no identity".to_string())
        })?;
        if identity.object_id.is_empty() {
            return Err(SftpOpsError::Operation(
                "Safe-file create returned no immutable object identity".to_string(),
            ));
        }
        Ok(Arc::new(RemoteSafeHandle {
            client,
            handle_id: opened.handle_id,
            kind,
            owned_artifact: true,
        }))
    }

    fn rename_safe_handle(
        &self,
        handle: &RemoteSafeHandle,
        old_path: &Path,
        new_path: &Path,
        mode: SafeFileRenameMode,
        expected_target: Option<SafeFileIdentity>,
    ) -> Result<(), SftpOpsError> {
        let operation_id = uuid::Uuid::new_v4().to_string();
        let recovery_path = new_path.to_path_buf();
        let source = handle.identity()?;
        let rename = SafeFileRename {
            handle_id: handle.handle_id.clone(),
            old_path: remote_path_string(old_path)?,
            new_path: remote_path_string(new_path)?,
            mode: mode as i32,
            expected_target: expected_target.clone(),
        };
        let replay = RemoteRecoveryOperation {
            operation_id: operation_id.clone(),
            source_preserved_after_commit: true,
            action: RemoteRecoveryAction::Rename {
                old_path: old_path.to_path_buf(),
                new_path: new_path.to_path_buf(),
                mode: mode as i32,
                expected_target,
                source,
                source_is_owned_artifact: handle.owned_artifact,
            },
        };
        let response = journaled_safe_file_call(
            &handle.client,
            operation_id.clone(),
            safe_file_request::Operation::Rename(rename),
        );
        let response = match response {
            Ok(response) => response,
            Err(SftpOpsError::Connection(message)) => {
                self.retain_remote_recovery(recovery_path.clone(), replay);
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!("Remote rename acknowledgement was lost: {message}"),
                    recovery_id: None,
                    paths: vec![recovery_path],
                    committed: false,
                });
            }
            Err(error) => {
                if let Some((operation, path)) =
                    pending_remote_recovery(&handle.client, &operation_id)?
                {
                    self.retain_remote_recovery(path.clone(), operation);
                    return Err(SftpOpsError::RecoveryRequired {
                        message: error.to_string(),
                        recovery_id: None,
                        paths: vec![path],
                        committed: false,
                    });
                }
                return Err(error);
            }
        };
        match response {
            safe_file_response::Result::Mutation(_) => {
                if let Err(error) = acknowledge_safe_file_mutation(&handle.client, &operation_id) {
                    self.retain_remote_recovery(
                        recovery_path.clone(),
                        RemoteRecoveryOperation {
                            operation_id,
                            source_preserved_after_commit: true,
                            action: RemoteRecoveryAction::Acknowledge,
                        },
                    );
                    return Err(SftpOpsError::RecoveryRequired {
                        message: format!(
                            "Remote rename committed but acknowledgement was lost: {error}"
                        ),
                        recovery_id: None,
                        paths: vec![recovery_path],
                        committed: true,
                    });
                }
                Ok(())
            }
            _ => Err(unexpected_safe_file_response("rename")),
        }
    }

    fn retain_remote_recovery(&self, path: PathBuf, operation: RemoteRecoveryOperation) {
        let mut routes = self
            .recovery_operations
            .lock()
            .expect("safe-file recovery route lock poisoned");
        let operations = routes.entry(path).or_default();
        if operations
            .iter()
            .all(|existing| existing.operation_id != operation.operation_id)
        {
            operations.push(operation);
        }
    }

    fn replay_remote_recovery(
        &self,
        client: &RemoteServerClient,
        operation: &RemoteRecoveryOperation,
    ) -> Result<RemoteRecoveryResolution, SftpOpsError> {
        let (request, _retained_handle) = match &operation.action {
            RemoteRecoveryAction::Acknowledge => {
                return Ok(RemoteRecoveryResolution::MutationApplied)
            }
            RemoteRecoveryAction::Rename {
                old_path,
                new_path,
                mode,
                expected_target,
                source,
                source_is_owned_artifact,
            } => {
                let entry = match self.lstat(old_path) {
                    Ok(entry) => entry,
                    Err(SftpOpsError::NotFound(_)) => {
                        return match self.stable_identity(new_path) {
                            Ok(actual) if same_immutable_object(source, &actual) => {
                                Ok(RemoteRecoveryResolution::DestinationCommittedSourcePreserved)
                            }
                            Err(SftpOpsError::NotFound(_)) if *source_is_owned_artifact => {
                                Ok(RemoteRecoveryResolution::SourceRestored)
                            }
                            Err(SftpOpsError::NotFound(_)) => {
                                Err(SftpOpsError::Operation(format!(
                                    "Remote recovery source and destination are both missing: {}, {}",
                                    old_path.display(),
                                    new_path.display()
                                )))
                            }
                            Ok(_) => Err(SftpOpsError::Operation(format!(
                                "Remote recovery destination identity changed at {}",
                                new_path.display()
                            ))),
                            Err(error) => Err(error),
                        };
                    }
                    Err(error) => return Err(error),
                };
                if entry.file_type != source.file_type {
                    return Err(SftpOpsError::Operation(format!(
                        "Remote recovery source type changed at {}",
                        old_path.display()
                    )));
                }
                let handle = self.open_safe_handle(old_path, source.file_type)?;
                let actual = handle.identity()?;
                if !same_immutable_object(source, &actual) {
                    return Err(SftpOpsError::Operation(format!(
                        "Remote recovery source identity changed at {}",
                        old_path.display()
                    )));
                }
                (
                    safe_file_request::Operation::Rename(SafeFileRename {
                        handle_id: handle.handle_id.clone(),
                        old_path: remote_path_string(old_path)?,
                        new_path: remote_path_string(new_path)?,
                        mode: *mode,
                        expected_target: expected_target.clone(),
                    }),
                    Some(handle),
                )
            }
            RemoteRecoveryAction::Delete(delete) => {
                (safe_file_request::Operation::Delete(delete.clone()), None)
            }
        };
        let response = journaled_safe_file_call(client, operation.operation_id.clone(), request)?;
        if !matches!(response, safe_file_response::Result::Mutation(_)) {
            return Err(unexpected_safe_file_response("replayed recovery"));
        }
        acknowledge_safe_file_mutation(client, &operation.operation_id)?;
        Ok(match &operation.action {
            RemoteRecoveryAction::Rename { .. } => {
                RemoteRecoveryResolution::DestinationCommittedSourcePreserved
            }
            RemoteRecoveryAction::Acknowledge | RemoteRecoveryAction::Delete(_) => {
                RemoteRecoveryResolution::MutationApplied
            }
        })
    }

    fn metadata_to_entry(path: &Path, metadata: zap_sftp::types::Metadata) -> FileEntry {
        let identity = stable_identity_from_remote_metadata(&metadata);
        let file_type = match metadata.file_type {
            zap_sftp::types::FileType::Dir => FileEntryType::Directory,
            zap_sftp::types::FileType::File => FileEntryType::File,
            zap_sftp::types::FileType::Symlink => FileEntryType::Symlink,
            zap_sftp::types::FileType::Other => FileEntryType::Other,
        };
        let modified = metadata.modified.map(|t| {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.format("%Y-%m-%d %H:%M").to_string()
        });
        let perms = &metadata.permissions;
        let owner = sftp_ops::bool_to_rwx(perms.owner_read, perms.owner_write, perms.owner_exec);
        let group = sftp_ops::bool_to_rwx(perms.group_read, perms.group_write, perms.group_exec);
        let other = sftp_ops::bool_to_rwx(perms.other_read, perms.other_write, perms.other_exec);
        let permissions = Some(format!("{owner}{group}{other}"));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        FileEntry {
            name,
            path: path.to_path_buf(),
            file_type,
            size: metadata.size,
            modified,
            permissions,
            identity,
        }
    }
}

fn remote_path_string(path: &Path) -> Result<String, SftpOpsError> {
    path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
        SftpOpsError::Operation(format!(
            "Secure remote file transactions require a UTF-8 path: {}",
            path.display()
        ))
    })
}

fn replace_recovery_routes(
    routes: &mut HashMap<PathBuf, Vec<RemoteRecoveryOperation>>,
    recoveries: impl IntoIterator<Item = (RemoteRecoveryOperation, PathBuf)>,
) -> Vec<PathBuf> {
    let recoveries = recoveries.into_iter().collect::<Vec<_>>();
    let server_operation_ids = recoveries
        .iter()
        .map(|(operation, _)| operation.operation_id.clone())
        .collect::<HashSet<_>>();
    let local_replays = routes
        .values()
        .flatten()
        .filter(|operation| !matches!(operation.action, RemoteRecoveryAction::Acknowledge))
        .map(|operation| (operation.operation_id.clone(), operation.clone()))
        .collect::<HashMap<_, _>>();
    let previously_reported = routes
        .values()
        .flatten()
        .map(|operation| operation.operation_id.clone())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut replacement = HashMap::<PathBuf, Vec<RemoteRecoveryOperation>>::new();
    let mut newly_reported = Vec::new();
    for (path, operations) in routes.iter() {
        for operation in operations {
            if !matches!(operation.action, RemoteRecoveryAction::Acknowledge)
                && !server_operation_ids.contains(&operation.operation_id)
                && seen.insert(operation.operation_id.clone())
            {
                replacement
                    .entry(path.clone())
                    .or_default()
                    .push(operation.clone());
            }
        }
    }
    for (mut operation, path) in recoveries {
        if operation.operation_id.is_empty() || !seen.insert(operation.operation_id.clone()) {
            continue;
        }
        if let Some(local) = local_replays.get(&operation.operation_id) {
            operation.action = local.action.clone();
        }
        replacement
            .entry(path.clone())
            .or_default()
            .push(operation.clone());
        if !previously_reported.contains(&operation.operation_id) {
            newly_reported.push(path);
        }
    }
    *routes = replacement;
    newly_reported
}

fn safe_kind(file_type: FileEntryType) -> Result<SafeFileEntryKind, SftpOpsError> {
    match file_type {
        FileEntryType::File => Ok(SafeFileEntryKind::Regular),
        FileEntryType::Directory => Ok(SafeFileEntryKind::Directory),
        FileEntryType::Symlink | FileEntryType::Other => Err(SftpOpsError::Operation(
            "Secure remote file transactions refuse links and special files".to_string(),
        )),
    }
}

fn stable_identity_from_safe(identity: SafeFileIdentity) -> StableEntryIdentity {
    let file_type = match SafeFileEntryKind::try_from(identity.kind).ok() {
        Some(SafeFileEntryKind::Regular) => FileEntryType::File,
        Some(SafeFileEntryKind::Directory) => FileEntryType::Directory,
        Some(SafeFileEntryKind::Unspecified) | None => FileEntryType::Other,
    };
    StableEntryIdentity {
        file_type,
        size: identity.size,
        object_id: identity.object_id,
        revision: identity.revision,
    }
}

fn safe_identity_from_stable(
    identity: &StableEntryIdentity,
) -> Result<SafeFileIdentity, SftpOpsError> {
    if identity.object_id.is_empty() {
        return Err(SftpOpsError::Operation(
            "Secure remote mutation requires an immutable object identity".to_string(),
        ));
    }
    Ok(SafeFileIdentity {
        kind: safe_kind(identity.file_type)? as i32,
        size: identity.size,
        object_id: identity.object_id.clone(),
        revision: identity.revision.clone(),
    })
}

fn unexpected_safe_file_response(operation: &str) -> SftpOpsError {
    SftpOpsError::Operation(format!(
        "Remote safe-file service returned an unexpected {operation} response"
    ))
}

fn safe_file_call(
    client: &RemoteServerClient,
    operation_id: String,
    operation: safe_file_request::Operation,
) -> Result<safe_file_response::Result, SftpOpsError> {
    let response = warpui::r#async::block_on(client.safe_file(SafeFileRequest {
        operation_id,
        operation: Some(operation),
    }))
    .map_err(|error| SftpOpsError::Connection(error.to_string()))?;
    match response.result {
        Some(safe_file_response::Result::Error(error)) => {
            Err(SftpOpsError::Operation(error.message))
        }
        Some(result) => Ok(result),
        None => Err(SftpOpsError::Operation(
            "Remote safe-file service returned an empty response".to_string(),
        )),
    }
}

fn journaled_safe_file_call(
    client: &RemoteServerClient,
    operation_id: String,
    operation: safe_file_request::Operation,
) -> Result<safe_file_response::Result, SftpOpsError> {
    let first = safe_file_call(client, operation_id.clone(), operation.clone());
    if matches!(first, Err(SftpOpsError::Connection(_))) {
        safe_file_call(client, operation_id, operation)
    } else {
        first
    }
}

fn acknowledge_safe_file_mutation(
    client: &RemoteServerClient,
    operation_id: &str,
) -> Result<(), SftpOpsError> {
    let response = safe_file_call(
        client,
        operation_id.to_string(),
        safe_file_request::Operation::RetryRecovery(SafeFileRetryRecovery {}),
    )?;
    match response {
        safe_file_response::Result::Mutation(_) => Ok(()),
        _ => Err(unexpected_safe_file_response("mutation acknowledgement")),
    }
}

fn pending_remote_recovery(
    client: &RemoteServerClient,
    operation_id: &str,
) -> Result<Option<(RemoteRecoveryOperation, PathBuf)>, SftpOpsError> {
    let response = safe_file_call(
        client,
        String::new(),
        safe_file_request::Operation::ListRecoveries(SafeFileListRecoveries {}),
    )?;
    let safe_file_response::Result::Recoveries(recoveries) = response else {
        return Err(unexpected_safe_file_response("recovery inventory"));
    };
    Ok(recoveries
        .recoveries
        .into_iter()
        .find(|recovery| recovery.operation_id == operation_id)
        .and_then(|recovery| {
            recovery.paths.first().map(|path| {
                (
                    RemoteRecoveryOperation {
                        operation_id: recovery.operation_id,
                        source_preserved_after_commit: recovery.source_preserved_after_commit,
                        action: RemoteRecoveryAction::Acknowledge,
                    },
                    PathBuf::from(path),
                )
            })
        }))
}

struct RemoteSafeHandle {
    client: Arc<RemoteServerClient>,
    handle_id: String,
    kind: FileEntryType,
    owned_artifact: bool,
}

impl RemoteSafeHandle {
    fn inspect(&self, path: Option<&Path>) -> Result<SafeFileInspectResult, SftpOpsError> {
        let response = safe_file_call(
            &self.client,
            String::new(),
            safe_file_request::Operation::InspectHandle(SafeFileInspectHandle {
                handle_id: self.handle_id.clone(),
                path: path
                    .map(remote_path_string)
                    .transpose()?
                    .unwrap_or_default(),
            }),
        )?;
        match response {
            safe_file_response::Result::Inspected(inspected) => Ok(inspected),
            _ => Err(unexpected_safe_file_response("inspect")),
        }
    }
}

impl Drop for RemoteSafeHandle {
    fn drop(&mut self) {
        if let Err(error) = self.client.close_safe_file_handle(self.handle_id.clone()) {
            log::warn!("Closing remote safe-file handle failed: {error}");
        }
    }
}

impl BackendOwnershipAnchor for RemoteSafeHandle {
    fn identity(&self) -> Result<StableEntryIdentity, SftpOpsError> {
        let identity = self.inspect(None)?.identity.ok_or_else(|| {
            SftpOpsError::Operation("Safe-file inspect returned no identity".to_string())
        })?;
        let identity = stable_identity_from_safe(identity);
        if identity.file_type != self.kind || identity.object_id.is_empty() {
            return Err(SftpOpsError::Operation(
                "Safe-file handle identity is invalid".to_string(),
            ));
        }
        Ok(identity)
    }

    fn matches_path(&self, path: &Path) -> Result<bool, SftpOpsError> {
        Ok(self.inspect(Some(path))?.matches_path)
    }

    fn link_count(&self) -> Result<Option<u64>, SftpOpsError> {
        Ok(self.inspect(None)?.link_count)
    }
}

struct RemoteSafeFileReader {
    handle: Arc<RemoteSafeHandle>,
    eof: bool,
}

impl BackendFileReader for RemoteSafeFileReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, SftpOpsError> {
        if buffer.is_empty() || self.eof {
            return Ok(0);
        }
        let response = safe_file_call(
            &self.handle.client,
            String::new(),
            safe_file_request::Operation::ReadHandle(SafeFileReadHandle {
                handle_id: self.handle.handle_id.clone(),
                max_bytes: buffer.len() as u64,
            }),
        )?;
        let safe_file_response::Result::Read(read) = response else {
            return Err(unexpected_safe_file_response("read"));
        };
        if read.bytes.len() > buffer.len() {
            return Err(SftpOpsError::Operation(
                "Remote safe-file read exceeded the requested chunk size".to_string(),
            ));
        }
        buffer[..read.bytes.len()].copy_from_slice(&read.bytes);
        self.eof = read.eof;
        Ok(read.bytes.len())
    }
}

struct RemoteSafeFileWriter {
    handle: Arc<RemoteSafeHandle>,
}

impl BackendFileWriter for RemoteSafeFileWriter {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError> {
        let response = safe_file_call(
            &self.handle.client,
            String::new(),
            safe_file_request::Operation::WriteHandle(SafeFileWriteHandle {
                handle_id: self.handle.handle_id.clone(),
                bytes: buffer.to_vec(),
            }),
        )?;
        match response {
            safe_file_response::Result::Mutation(_) => Ok(()),
            _ => Err(unexpected_safe_file_response("write")),
        }
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        let response = safe_file_call(
            &self.handle.client,
            String::new(),
            safe_file_request::Operation::FlushHandle(SafeFileFlushHandle {
                handle_id: self.handle.handle_id.clone(),
            }),
        )?;
        match response {
            safe_file_response::Result::Mutation(_) => Ok(()),
            _ => Err(unexpected_safe_file_response("flush")),
        }
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        Ok(Some(self.handle.clone()))
    }
}

impl SftpBackend for LiveSftpBackend {
    fn supports_atomic_exchange(&self) -> bool {
        self.safe_files.is_available()
    }

    fn supports_identity_bound_cleanup(&self) -> bool {
        self.safe_files.is_available()
    }

    fn startup_recovery_paths(&self) -> Vec<PathBuf> {
        let Some(client) = self.safe_files.get() else {
            return Vec::new();
        };
        let response = safe_file_call(
            &client,
            String::new(),
            safe_file_request::Operation::ListRecoveries(SafeFileListRecoveries {}),
        );
        let Ok(safe_file_response::Result::Recoveries(recoveries)) = response else {
            return Vec::new();
        };
        let recoveries = recoveries.recoveries.into_iter().filter_map(|recovery| {
            recovery.paths.first().map(PathBuf::from).map(|path| {
                (
                    RemoteRecoveryOperation {
                        operation_id: recovery.operation_id,
                        source_preserved_after_commit: recovery.source_preserved_after_commit,
                        action: RemoteRecoveryAction::Acknowledge,
                    },
                    path,
                )
            })
        });
        let mut routes = self
            .recovery_operations
            .lock()
            .expect("safe-file recovery route lock poisoned");
        replace_recovery_routes(&mut routes, recoveries)
    }

    fn retry_unresolved_recovery(&self, path: &Path) -> Result<Option<Vec<PathBuf>>, SftpOpsError> {
        let operation = self
            .recovery_operations
            .lock()
            .expect("safe-file recovery route lock poisoned")
            .get(path)
            .and_then(|operations| operations.first())
            .cloned();
        let Some(operation) = operation else {
            return Ok(None);
        };
        let client = self.safe_client()?;
        let response = safe_file_call(
            &client,
            operation.operation_id.clone(),
            safe_file_request::Operation::RetryRecovery(SafeFileRetryRecovery {}),
        );
        let resolution = match response {
            Ok(safe_file_response::Result::Mutation(_)) => {
                if operation.source_preserved_after_commit {
                    RemoteRecoveryResolution::DestinationCommittedSourcePreserved
                } else {
                    RemoteRecoveryResolution::MutationApplied
                }
            }
            Ok(_) => return Err(unexpected_safe_file_response("recovery retry")),
            Err(error) => match pending_remote_recovery(&client, &operation.operation_id) {
                Ok(Some(_)) => return Err(error),
                Ok(None) => self.replay_remote_recovery(&client, &operation)?,
                Err(_) => return Err(error),
            },
        };
        let mut routes = self
            .recovery_operations
            .lock()
            .expect("safe-file recovery route lock poisoned");
        if let Some(operations) = routes.get_mut(path) {
            if operations.first() == Some(&operation) {
                operations.remove(0);
            }
            if operations.is_empty() {
                routes.remove(path);
            }
        }
        match resolution {
            RemoteRecoveryResolution::MutationApplied => {}
            RemoteRecoveryResolution::DestinationCommittedSourcePreserved => {
                self.recovered_source_preserved
                    .lock()
                    .expect("safe-file recovered-source lock poisoned")
                    .insert(path.to_path_buf());
            }
            RemoteRecoveryResolution::SourceRestored => {
                self.recovered_source_restored
                    .lock()
                    .expect("safe-file restored-source lock poisoned")
                    .insert(path.to_path_buf());
            }
        }
        Ok(Some(Vec::new()))
    }

    fn take_recovery_source_preserved(&self, path: &Path) -> bool {
        self.recovered_source_preserved
            .lock()
            .expect("safe-file recovered-source lock poisoned")
            .remove(path)
    }

    fn take_recovery_source_restored(&self, path: &Path) -> bool {
        self.recovered_source_restored
            .lock()
            .expect("safe-file restored-source lock poisoned")
            .remove(path)
    }

    fn existing_entry_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        let entry = self.lstat(path)?;
        self.open_safe_handle(path, entry.file_type)
            .map(|handle| Some(handle as Arc<dyn BackendOwnershipAnchor>))
    }

    fn preflight_safe_mutation(
        &self,
        path: &Path,
        _require_exchange: bool,
    ) -> Result<(), SftpOpsError> {
        self.safe_client().map(|_| ()).map_err(|_| {
            SftpOpsError::CapabilityRequired(format!(
                "Secure remote file transactions are unavailable for {}",
                path.display()
            ))
        })
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
        sftp_ops::list_dir(&self.sftp, path)
    }

    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError> {
        sftp_ops::delete_file(&self.sftp, path)
    }

    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError> {
        sftp_ops::delete_dir_recursive(&self.sftp, path)
    }

    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError> {
        sftp_ops::create_dir(&self.sftp, path)
    }

    fn create_dir_with_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        self.create_safe_handle(path, FileEntryType::Directory)
            .map(|handle| Some(handle as Arc<dyn BackendOwnershipAnchor>))
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let entry = self.lstat(old_path)?;
        let handle = self.open_safe_handle(old_path, entry.file_type)?;
        self.rename_safe_handle(
            &handle,
            old_path,
            new_path,
            SafeFileRenameMode::NoReplace,
            None,
        )
    }

    fn rename_if_matches(
        &self,
        old_path: &Path,
        new_path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        if !anchor.matches_path(old_path)? {
            return Err(SftpOpsError::Operation(format!(
                "Remote rename source ownership changed at {}",
                old_path.display()
            )));
        }
        let expected = anchor.identity()?;
        let handle = self.open_safe_handle(old_path, expected.file_type)?;
        let actual = handle.identity()?;
        if !same_immutable_object(&expected, &actual) {
            return Err(SftpOpsError::Operation(format!(
                "Remote rename source identity changed at {}",
                old_path.display()
            )));
        }
        self.rename_safe_handle(
            &handle,
            old_path,
            new_path,
            SafeFileRenameMode::NoReplace,
            None,
        )
    }

    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let source_entry = self.lstat(old_path)?;
        let target_identity = self.stable_identity(new_path)?;
        let source = self.open_safe_handle(old_path, source_entry.file_type)?;
        self.rename_safe_handle(
            &source,
            old_path,
            new_path,
            SafeFileRenameMode::Exchange,
            Some(safe_identity_from_stable(&target_identity)?),
        )
    }

    fn delete_file_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
        expected_sha256: &str,
    ) -> Result<(), SftpOpsError> {
        let client = self.safe_client()?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let delete = SafeFileDelete {
            path: remote_path_string(path)?,
            expected: Some(safe_identity_from_stable(expected)?),
            expected_sha256: Some(expected_sha256.to_string()),
        };
        let replay = RemoteRecoveryOperation {
            operation_id: operation_id.clone(),
            source_preserved_after_commit: false,
            action: RemoteRecoveryAction::Delete(delete.clone()),
        };
        let response = journaled_safe_file_call(
            &client,
            operation_id.clone(),
            safe_file_request::Operation::Delete(delete),
        );
        let response = match response {
            Ok(response) => response,
            Err(SftpOpsError::Connection(message)) => {
                self.retain_remote_recovery(path.to_path_buf(), replay);
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!("Remote delete acknowledgement was lost: {message}"),
                    recovery_id: None,
                    paths: vec![path.to_path_buf()],
                    committed: false,
                });
            }
            Err(error) => {
                if let Some((operation, recovery_path)) =
                    pending_remote_recovery(&client, &operation_id)?
                {
                    self.retain_remote_recovery(recovery_path.clone(), operation);
                    return Err(SftpOpsError::RecoveryRequired {
                        message: error.to_string(),
                        recovery_id: None,
                        paths: vec![recovery_path],
                        committed: false,
                    });
                }
                return Err(error);
            }
        };
        match response {
            safe_file_response::Result::Mutation(_) => {
                if let Err(error) = acknowledge_safe_file_mutation(&client, &operation_id) {
                    self.retain_remote_recovery(
                        path.to_path_buf(),
                        RemoteRecoveryOperation {
                            operation_id,
                            source_preserved_after_commit: false,
                            action: RemoteRecoveryAction::Acknowledge,
                        },
                    );
                    return Err(SftpOpsError::RecoveryRequired {
                        message: format!(
                            "Remote delete committed but acknowledgement was lost: {error}"
                        ),
                        recovery_id: None,
                        paths: vec![path.to_path_buf()],
                        committed: true,
                    });
                }
                Ok(())
            }
            _ => Err(unexpected_safe_file_response("delete")),
        }
    }

    fn delete_empty_dir_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
    ) -> Result<(), SftpOpsError> {
        let client = self.safe_client()?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let delete = SafeFileDelete {
            path: remote_path_string(path)?,
            expected: Some(safe_identity_from_stable(expected)?),
            expected_sha256: None,
        };
        let replay = RemoteRecoveryOperation {
            operation_id: operation_id.clone(),
            source_preserved_after_commit: false,
            action: RemoteRecoveryAction::Delete(delete.clone()),
        };
        let response = journaled_safe_file_call(
            &client,
            operation_id.clone(),
            safe_file_request::Operation::Delete(delete),
        );
        let response = match response {
            Ok(response) => response,
            Err(SftpOpsError::Connection(message)) => {
                self.retain_remote_recovery(path.to_path_buf(), replay);
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!("Remote directory-delete acknowledgement was lost: {message}"),
                    recovery_id: None,
                    paths: vec![path.to_path_buf()],
                    committed: false,
                });
            }
            Err(error) => {
                if let Some((operation, recovery_path)) =
                    pending_remote_recovery(&client, &operation_id)?
                {
                    self.retain_remote_recovery(recovery_path.clone(), operation);
                    return Err(SftpOpsError::RecoveryRequired {
                        message: error.to_string(),
                        recovery_id: None,
                        paths: vec![recovery_path],
                        committed: false,
                    });
                }
                return Err(error);
            }
        };
        match response {
            safe_file_response::Result::Mutation(_) => {
                if let Err(error) = acknowledge_safe_file_mutation(&client, &operation_id) {
                    self.retain_remote_recovery(
                        path.to_path_buf(),
                        RemoteRecoveryOperation {
                            operation_id,
                            source_preserved_after_commit: false,
                            action: RemoteRecoveryAction::Acknowledge,
                        },
                    );
                    return Err(SftpOpsError::RecoveryRequired {
                        message: format!(
                            "Remote directory delete committed but acknowledgement was lost: {error}"
                        ),
                        recovery_id: None,
                        paths: vec![path.to_path_buf()],
                        committed: true,
                    });
                }
                Ok(())
            }
            _ => Err(unexpected_safe_file_response("directory delete")),
        }
    }

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        self.sftp
            .realpath(path)
            .map_err(|e| SftpOpsError::Operation(e.to_string()))
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let metadata = self.sftp.stat(path)?;
        Ok(Self::metadata_to_entry(path, metadata))
    }

    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let metadata = self.sftp.lstat(path)?;
        let mut entry = Self::metadata_to_entry(path, metadata);
        if self.safe_files.is_available()
            && matches!(
                entry.file_type,
                FileEntryType::File | FileEntryType::Directory
            )
        {
            entry.identity = self.stable_identity(path)?;
        }
        Ok(entry)
    }

    fn modification_time(
        &self,
        path: &Path,
    ) -> Result<Option<std::time::SystemTime>, SftpOpsError> {
        Ok(self.sftp.lstat(path)?.modified)
    }

    fn stable_identity(&self, path: &Path) -> Result<StableEntryIdentity, SftpOpsError> {
        let metadata = self.sftp.lstat(path)?;
        let kind = match metadata.file_type {
            zap_sftp::types::FileType::File => FileEntryType::File,
            zap_sftp::types::FileType::Dir => FileEntryType::Directory,
            zap_sftp::types::FileType::Symlink => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to identify remote symbolic link {}",
                    path.display()
                )))
            }
            zap_sftp::types::FileType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to identify remote special file {}",
                    path.display()
                )))
            }
        };
        self.open_safe_handle(path, kind)?.identity()
    }

    fn open_file_reader(&self, path: &Path) -> Result<Box<dyn BackendFileReader>, SftpOpsError> {
        Ok(Box::new(RemoteSafeFileReader {
            handle: self.open_safe_handle(path, FileEntryType::File)?,
            eof: false,
        }))
    }

    fn create_file_writer(&self, path: &Path) -> Result<Box<dyn BackendFileWriter>, SftpOpsError> {
        Ok(Box::new(RemoteSafeFileWriter {
            handle: self.create_safe_handle(path, FileEntryType::File)?,
        }))
    }

    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);
        let flag = cancel_flag.unwrap_or(&NEVER_CANCEL);
        sftp_ops::upload_file_streaming(&self.sftp, local_path, remote_path, progress_cb, flag)
    }

    fn download_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);
        let flag = cancel_flag.unwrap_or(&NEVER_CANCEL);
        sftp_ops::download_file_streaming(&self.sftp, remote_path, local_path, progress_cb, flag)
    }

    fn upload_file_no_replace(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);
        let flag = cancel_flag.unwrap_or(&NEVER_CANCEL);
        sftp_ops::upload_file_streaming_no_replace(
            &self.sftp,
            local_path,
            remote_path,
            progress_cb,
            flag,
        )
    }

    fn download_file_no_replace(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);
        let flag = cancel_flag.unwrap_or(&NEVER_CANCEL);
        sftp_ops::download_file_streaming_no_replace(
            &self.sftp,
            remote_path,
            local_path,
            progress_cb,
            flag,
        )
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        validate_copy_destination(src, dst, false)?;
        let mut reader = self.open_file_reader(src)?;
        let mut writer = self.create_file_writer(dst)?;
        let mut buffer = vec![0_u8; super::transfer_job::STREAM_CHUNK_SIZE];
        loop {
            let read = reader.read_chunk(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_chunk(&buffer[..read])?;
        }
        writer.flush()
    }
}

// ============================================================
// InMemorySftpBackend — Local filesystem-based test implementation
// ============================================================

/// SFTP backend based on memory (local temp directory) for testing.
pub struct InMemorySftpBackend {
    /// Root directory that simulates the remote filesystem root.
    root: PathBuf,
    directory_reservation_registry: Option<DirectoryReservationRegistry>,
    safe_mutation_capabilities: Mutex<HashMap<(u64, bool), Result<(), String>>>,
    cleanup_recovery_identities:
        Mutex<HashMap<PathBuf, (StableEntryIdentity, Arc<dyn BackendOwnershipAnchor>)>>,
    directory_reservation_namespaces: Mutex<HashMap<PathBuf, DirectoryReservationNamespace>>,
    reserved_directory_namespace_paths: Mutex<HashMap<PathBuf, usize>>,
    #[cfg(test)]
    force_in_tree_directory_reservation_namespace: bool,
    opaque_recovery_paths: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
    opaque_recovery_markers: Mutex<HashMap<PathBuf, OwnedReservationMarker>>,
    artifact_lifecycle: Mutex<()>,
    persistent_artifact_records: Mutex<HashMap<PathBuf, PersistentArtifactRecord>>,
    persistent_exchange_records: Mutex<HashMap<PathBuf, PersistentExchangeRecord>>,
    startup_unresolved_paths: Mutex<HashSet<PathBuf>>,
    #[cfg(test)]
    before_rename: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_guarded_rename_check_before_mutation: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    before_guarded_rename_restore: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    before_placeholder_isolation: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_placeholder_tombstone_cleanup: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_placeholder_final_check_before_delete: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_private_placeholder_unlink: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_private_namespace_unlink: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_private_placeholder_isolation: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_placeholder_isolation_before_classification:
        Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_guarded_exchange_before_classification: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_guarded_cleanup_verification: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_rename: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    forced_lstat_error: Option<PathBuf>,
    #[cfg(test)]
    fail_staged_identity: bool,
    #[cfg(test)]
    fail_published_identity: Option<PathBuf>,
    #[cfg(test)]
    published_identity_calls: AtomicU64,
    #[cfg(test)]
    fail_delete_after_apply: Option<PathBuf>,
    #[cfg(test)]
    fail_delete_matching: Option<String>,
    #[cfg(test)]
    fail_delete_matching_once: bool,
    #[cfg(test)]
    delete_matching_failed: AtomicBool,
    #[cfg(test)]
    fail_replace_after_apply: Option<PathBuf>,
    #[cfg(test)]
    before_replace: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_replace: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    fail_rename_after_apply: Option<PathBuf>,
    #[cfg(test)]
    fail_writer_on_create: Option<u64>,
    #[cfg(test)]
    fail_writer_create_after_apply: Option<u64>,
    #[cfg(test)]
    corrupt_writer_on_create: Option<u64>,
    #[cfg(test)]
    writer_creates: AtomicU64,
    #[cfg(test)]
    fail_directory_create_after_apply: Option<u64>,
    #[cfg(test)]
    directory_creates: AtomicU64,
    #[cfg(test)]
    after_directory_create_before_anchor: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_directory_anchor_before_publish: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_namespace_create_before_anchor: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_writer_validation_before_open: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    directory_reservation_failure: Option<DirectoryReservationFailure>,
    #[cfg(test)]
    ignore_noreplace_probe_semantics: bool,
    #[cfg(test)]
    fail_preflight_cleanup: bool,
    #[cfg(test)]
    preflight_collision_suffix: Option<char>,
    #[cfg(test)]
    preflight_collision: Mutex<Option<(PathBuf, u64, u64)>>,
    #[cfg(test)]
    preflight_rename_copy_unlink: bool,
    #[cfg(test)]
    preflight_exchange_content_swap: bool,
    #[cfg(test)]
    replace_preflight_source_before_rename: bool,
    #[cfg(test)]
    replace_preflight_source_before_exchange: bool,
    #[cfg(test)]
    replace_preflight_sources_before_reject: bool,
    #[cfg(test)]
    preflight_mutation_replacement: Mutex<Option<PathBuf>>,
    #[cfg(test)]
    preflight_reject_replacements: Mutex<Vec<PathBuf>>,
    #[cfg(test)]
    fail_preflight_rename_after_apply: bool,
    #[cfg(test)]
    fail_preflight_create_after_apply: Option<char>,
    #[cfg(test)]
    preflight_uncertain_create: Mutex<Option<PathBuf>>,
    #[cfg(test)]
    replace_preflight_owned_before_cleanup: Option<char>,
    #[cfg(test)]
    preflight_cleanup_replacement: Mutex<Option<(PathBuf, u64, u64)>>,
    #[cfg(test)]
    replace_preflight_owned_after_check: Option<char>,
    #[cfg(test)]
    preflight_cleanup_replacement_after_check: Mutex<Option<(PathBuf, u64, u64)>>,
    #[cfg(test)]
    observe_preflight_cleanup_anchor: Option<char>,
    #[cfg(test)]
    preflight_cleanup_anchor_observed: AtomicBool,
    #[cfg(test)]
    force_preflight_inode_reuse: Option<char>,
    #[cfg(test)]
    preflight_inode_reuse_observation: Mutex<Option<(PathBuf, u64, u64, bool, i64, i64)>>,
    #[cfg(test)]
    fail_recursive_delete_partially: bool,
    #[cfg(test)]
    after_stable_identity: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_guarded_delete: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    fail_isolated_delete_before_apply: bool,
    #[cfg(test)]
    fail_directory_marker_cleanup: bool,
    #[cfg(test)]
    namespace_scan_failure: Mutex<Option<NamespaceScanFailure>>,
    #[cfg(test)]
    sibling_recovery_failure: Mutex<Option<SiblingRecoveryFailure>>,
    #[cfg(test)]
    before_owned_candidate_anchor_open: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_sibling_registry_commit: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_sibling_registry_write: Option<Arc<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    before_sibling_rescan_iteration: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_sibling_recovery_anchor_open: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    after_artifact_retirement_generation_check:
        Option<Arc<dyn Fn(&InMemorySftpBackend, &Path) + Send + Sync>>,
    #[cfg(test)]
    at_artifact_association_cutpoint:
        Option<Arc<dyn Fn(&InMemorySftpBackend, &Path) + Send + Sync>>,
    #[cfg(test)]
    at_failed_directory_candidate_association_cutpoint:
        Option<Arc<dyn Fn(&InMemorySftpBackend, &Path, bool) + Send + Sync>>,
    #[cfg(test)]
    at_opaque_cleanup_sibling_publication_cutpoint:
        Option<Arc<dyn Fn(&InMemorySftpBackend, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_opaque_cleanup_source_read_before_lifecycle:
        Option<Arc<dyn Fn(&InMemorySftpBackend, &Path) + Send + Sync>>,
}

#[derive(Clone)]
struct DirectoryReservationNamespace {
    path: PathBuf,
    anchor: Arc<dyn BackendOwnershipAnchor>,
    device: u64,
    namespace_id: String,
    generation: String,
}

#[derive(Clone)]
struct DirectoryReservationRegistry {
    // This secret separates backend-visible path occupants from objects
    // created by Zaplex. A process that can read and modify Zaplex's private
    // state directory is inside the local account trust boundary; path names,
    // marker bytes, UID, and mode alone never establish cleanup ownership.
    root: PathBuf,
    secret: Arc<[u8; 32]>,
    backend_key: String,
    #[cfg(test)]
    fail_artifact_writes: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_write_on: Arc<AtomicU64>,
    #[cfg(test)]
    artifact_write_calls: Arc<AtomicU64>,
    #[cfg(test)]
    fail_artifact_transitions: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_removals: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_retirement_sync: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_retirement_unlink: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_retirement_final_sync: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_legacy_migration: Arc<AtomicBool>,
    #[cfg(test)]
    replace_artifact_before_retirement: Arc<AtomicBool>,
    #[cfg(test)]
    namespace_probe_failure: Arc<Mutex<Option<NamespaceProbeFailure>>>,
    #[cfg(test)]
    fail_namespace_migration_after_marker_replace: Arc<AtomicBool>,
    #[cfg(test)]
    fail_namespace_migration_after_record_replace: Arc<AtomicBool>,
    #[cfg(test)]
    fail_exchange_retirement_unlink: Arc<AtomicBool>,
    #[cfg(test)]
    fail_exchange_retirement_final_sync: Arc<AtomicBool>,
    #[cfg(test)]
    fail_temporary_cleanup: Arc<AtomicBool>,
    #[cfg(test)]
    fail_exchange_temporary_write: Arc<AtomicBool>,
    #[cfg(test)]
    fail_artifact_temporary_write: Arc<AtomicBool>,
    #[cfg(test)]
    fail_namespace_migration_marker_temporary_write: Arc<AtomicBool>,
    #[cfg(test)]
    fail_namespace_migration_record_temporary_write: Arc<AtomicBool>,
    #[cfg(test)]
    exchange_create_probe_error: Arc<Mutex<Option<i32>>>,
    #[cfg(test)]
    fail_exchange_create_sync: Arc<AtomicBool>,
}

#[derive(Clone)]
struct OwnedReservationMarker {
    path: PathBuf,
    identity: StableEntryIdentity,
    anchor: Arc<dyn BackendOwnershipAnchor>,
}

struct LocalFileReader(fs::File);

impl BackendFileReader for LocalFileReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, SftpOpsError> {
        self.0.read(buffer).map_err(Into::into)
    }
}

struct LocalOwnershipAnchor {
    file: fs::File,
    root: PathBuf,
    opaque_paths: Option<Arc<Mutex<HashMap<PathBuf, PathBuf>>>>,
}

impl BackendOwnershipAnchor for LocalOwnershipAnchor {
    fn identity(&self) -> Result<StableEntryIdentity, SftpOpsError> {
        Ok(stable_identity_from_local_metadata(&self.file.metadata()?))
    }

    fn matches_path(&self, path: &Path) -> Result<bool, SftpOpsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let anchored = self.file.metadata()?;
            let local = self
                .opaque_paths
                .as_ref()
                .and_then(|paths| {
                    paths
                        .lock()
                        .expect("opaque recovery path lock poisoned")
                        .get(path)
                        .cloned()
                })
                .unwrap_or_else(|| self.root.join(path.strip_prefix("/").unwrap_or(path)));
            let current = match fs::symlink_metadata(local) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            Ok(!current.file_type().is_symlink()
                && anchored.dev() == current.dev()
                && anchored.ino() == current.ino()
                && anchored.file_type() == current.file_type())
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(false)
        }
    }

    fn link_count(&self) -> Result<Option<u64>, SftpOpsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            Ok(Some(self.file.metadata()?.nlink()))
        }
        #[cfg(not(unix))]
        {
            Ok(None)
        }
    }

    fn matches_local_path(&self, path: &Path) -> Result<bool, SftpOpsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let anchored = self.file.metadata()?;
            let current = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            Ok(!current.file_type().is_symlink()
                && anchored.dev() == current.dev()
                && anchored.ino() == current.ino()
                && anchored.file_type() == current.file_type())
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(false)
        }
    }
}

fn local_ownership_anchor(
    file: &fs::File,
    root: &Path,
) -> Result<Arc<dyn BackendOwnershipAnchor>, SftpOpsError> {
    Ok(Arc::new(LocalOwnershipAnchor {
        file: file.try_clone()?,
        root: root.to_path_buf(),
        opaque_paths: None,
    }))
}

struct LocalFileWriter {
    file: fs::File,
    root: PathBuf,
}

impl BackendFileWriter for LocalFileWriter {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError> {
        self.file.write_all(buffer).map_err(Into::into)
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        self.file.flush().map_err(Into::into)
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        local_ownership_anchor(&self.file, &self.root).map(Some)
    }
}

#[cfg(test)]
struct FailingFileWriter {
    file: fs::File,
    root: PathBuf,
}

#[cfg(test)]
impl BackendFileWriter for FailingFileWriter {
    fn write_chunk(&mut self, _buffer: &[u8]) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(
            "injected streaming writer failure".to_string(),
        ))
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        Ok(())
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        local_ownership_anchor(&self.file, &self.root).map(Some)
    }
}

#[cfg(test)]
struct CorruptingFileWriter {
    file: fs::File,
    root: PathBuf,
    corrupted: bool,
}

#[cfg(test)]
impl BackendFileWriter for CorruptingFileWriter {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError> {
        let mut bytes = buffer.to_vec();
        if !self.corrupted && !bytes.is_empty() {
            bytes[0] ^= 0xff;
            self.corrupted = true;
        }
        self.file.write_all(&bytes).map_err(Into::into)
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        self.file.flush().map_err(Into::into)
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        local_ownership_anchor(&self.file, &self.root).map(Some)
    }
}

fn stable_identity_from_remote_metadata(
    metadata: &zap_sftp::types::Metadata,
) -> StableEntryIdentity {
    let file_type = match metadata.file_type {
        zap_sftp::types::FileType::Dir => FileEntryType::Directory,
        zap_sftp::types::FileType::File => FileEntryType::File,
        zap_sftp::types::FileType::Symlink => FileEntryType::Symlink,
        zap_sftp::types::FileType::Other => FileEntryType::Other,
    };
    let modified = metadata
        .modified
        .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    StableEntryIdentity {
        file_type,
        size: metadata.size,
        object_id: String::new(),
        revision: format!("{}:{}:{modified}", metadata.uid, metadata.gid),
    }
}

fn stable_identity_from_local_metadata(metadata: &fs::Metadata) -> StableEntryIdentity {
    let file_type = if metadata.file_type().is_symlink() {
        FileEntryType::Symlink
    } else if metadata.is_dir() {
        FileEntryType::Directory
    } else if metadata.is_file() {
        FileEntryType::File
    } else {
        FileEntryType::Other
    };
    #[cfg(unix)]
    let (object_id, revision) = {
        use std::os::unix::fs::MetadataExt;
        (
            format!("{}:{}", metadata.dev(), metadata.ino()),
            format!(
                "{}:{}:{}:{}:{}:{}",
                metadata.dev(),
                metadata.ino(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec()
            ),
        )
    };
    #[cfg(not(unix))]
    let revision = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_default();
    #[cfg(not(unix))]
    let object_id = revision.clone();
    StableEntryIdentity {
        file_type,
        size: metadata.len(),
        object_id,
        revision,
    }
}

#[derive(Clone)]
struct DirectoryNamespaceRecord {
    path: PathBuf,
    device: u64,
    namespace_id: String,
    object_id: String,
    generation: String,
    legacy: bool,
}

#[derive(Clone)]
struct PersistentArtifactRecord {
    /// Stable logical ID used by the queue and as the registry record key.
    path: PathBuf,
    /// Concrete backend-local path, when the logical ID is intentionally opaque.
    physical_path: Option<PathBuf>,
    role: String,
    identity: Option<StableEntryIdentity>,
    /// HMAC-signed lifecycle nonce used for exact CAS transitions.
    generation: String,
    /// Durable terminal state. Retired records remain as bounded tombstones so
    /// a failed directory fsync can never turn into silent recovery loss.
    retired: bool,
    legacy: bool,
}

impl PersistentArtifactRecord {
    fn active(
        path: PathBuf,
        physical_path: Option<PathBuf>,
        role: String,
        identity: Option<StableEntryIdentity>,
    ) -> Self {
        Self {
            path,
            physical_path,
            role,
            identity,
            generation: uuid::Uuid::new_v4().to_string(),
            retired: false,
            legacy: false,
        }
    }

    fn transition(
        &self,
        physical_path: Option<PathBuf>,
        role: String,
        identity: Option<StableEntryIdentity>,
    ) -> Self {
        Self::active(self.path.clone(), physical_path, role, identity)
    }

    fn retired(&self) -> Self {
        let mut retired = self.clone();
        retired.retired = true;
        retired
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PersistentExchangePhase {
    Prepared,
    Applied,
}

impl PersistentExchangePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Applied => "applied",
        }
    }
}

#[derive(Clone)]
struct PersistentExchangeCandidate {
    physical_path: PathBuf,
    role: String,
    identity: Option<StableEntryIdentity>,
}

#[derive(Clone)]
struct PersistentExchangeRecord {
    path: PathBuf,
    first: PersistentExchangeCandidate,
    second: PersistentExchangeCandidate,
    phase: PersistentExchangePhase,
    generation: String,
    legacy: bool,
}

impl PersistentExchangeRecord {
    fn active(
        path: PathBuf,
        first: PersistentExchangeCandidate,
        second: PersistentExchangeCandidate,
        phase: PersistentExchangePhase,
    ) -> Self {
        Self {
            path,
            first,
            second,
            phase,
            generation: uuid::Uuid::new_v4().to_string(),
            legacy: false,
        }
    }
}

fn same_optional_identity(
    first: Option<&StableEntryIdentity>,
    second: Option<&StableEntryIdentity>,
) -> bool {
    match (first, second) {
        (Some(first), Some(second)) => same_immutable_object(first, second),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn same_persistent_artifact_record(
    first: &PersistentArtifactRecord,
    second: &PersistentArtifactRecord,
) -> bool {
    first.path == second.path
        && first.physical_path == second.physical_path
        && first.role == second.role
        && first.generation == second.generation
        && first.retired == second.retired
        && first.legacy == second.legacy
        && same_optional_identity(first.identity.as_ref(), second.identity.as_ref())
}

struct NamespacePathReservation<'a> {
    reservations: &'a Mutex<HashMap<PathBuf, usize>>,
    path: PathBuf,
    committed: bool,
}

impl NamespacePathReservation<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for NamespacePathReservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut reservations = self
            .reservations
            .lock()
            .expect("reserved directory namespace lock poisoned");
        let Some(count) = reservations.get_mut(&self.path) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            reservations.remove(&self.path);
        }
    }
}

struct OwnedTemporaryFile {
    path: PathBuf,
    committed: bool,
    #[cfg(test)]
    fail_cleanup: Option<Arc<AtomicBool>>,
}

impl OwnedTemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
            #[cfg(test)]
            fail_cleanup: None,
        }
    }

    #[cfg(test)]
    fn with_cleanup_failure(mut self, failure: Arc<AtomicBool>) -> Self {
        self.fail_cleanup = Some(failure);
        self
    }

    fn commit(mut self) {
        self.committed = true;
    }

    fn cleanup(mut self) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if self
            .fail_cleanup
            .as_ref()
            .is_some_and(|failure| failure.load(Ordering::SeqCst))
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.committed = true;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.committed = true;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn open_owned_temporary_file(path: &Path) -> Result<fs::File, SftpOpsError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o777 != 0o600
        {
            return Err(SftpOpsError::Operation(format!(
                "Trusted transfer registry temporary is not an owned mode-0600 file at {}",
                path.display()
            )));
        }
    }
    Ok(file)
}

fn temporary_failure(guard: OwnedTemporaryFile, operation_error: SftpOpsError) -> SftpOpsError {
    match guard.cleanup() {
        Ok(()) => operation_error,
        Err(cleanup_error) => SftpOpsError::Operation(format!(
            "{operation_error}; cleaning the bounded trusted temporary failed: {cleanup_error}"
        )),
    }
}

impl Drop for OwnedTemporaryFile {
    fn drop(&mut self) {
        if !self.committed {
            #[cfg(test)]
            if self
                .fail_cleanup
                .as_ref()
                .is_some_and(|failure| failure.load(Ordering::SeqCst))
            {
                return;
            }
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn same_exchange_candidate(
    first: &PersistentExchangeCandidate,
    second: &PersistentExchangeCandidate,
) -> bool {
    first.physical_path == second.physical_path
        && first.role == second.role
        && same_optional_identity(first.identity.as_ref(), second.identity.as_ref())
}

fn same_persistent_exchange_record(
    first: &PersistentExchangeRecord,
    second: &PersistentExchangeRecord,
) -> bool {
    first.path == second.path
        && first.phase == second.phase
        && first.generation == second.generation
        && first.legacy == second.legacy
        && same_exchange_candidate(&first.first, &second.first)
        && same_exchange_candidate(&first.second, &second.second)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, SftpOpsError> {
    if value.len() % 2 != 0 {
        return Err(SftpOpsError::Operation(
            "Trusted transfer registry contains invalid hex".to_string(),
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Trusted transfer registry contains invalid UTF-8 hex: {error}"
                ))
            })?;
            u8::from_str_radix(pair, 16).map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Trusted transfer registry contains invalid hex: {error}"
                ))
            })
        })
        .collect()
}

fn path_registry_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.as_os_str().to_string_lossy().as_bytes().to_vec()
    }
}

fn path_from_registry_bytes(bytes: Vec<u8>) -> Result<PathBuf, SftpOpsError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Trusted transfer registry path is not UTF-8: {error}"
                ))
            })
    }
}

fn validate_private_registry_file(path: &Path) -> Result<fs::Metadata, SftpOpsError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SftpOpsError::Operation(format!(
            "Trusted transfer registry entry is not a regular file: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(SftpOpsError::Operation(format!(
                "Trusted transfer registry entry has unsafe ownership or permissions: {}",
                path.display()
            )));
        }
    }
    Ok(metadata)
}

#[cfg(unix)]
fn lock_registry_root(root: &Path) -> Result<RegistryLock, SftpOpsError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).mode(0o600);
    let file = options.open(root.join(".registry.lock"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(RegistryLock(file))
}

#[cfg(not(unix))]
fn lock_registry_root(_root: &Path) -> Result<RegistryLock, SftpOpsError> {
    Err(SftpOpsError::Operation(
        "Crash-safe transfer registry locking is unsupported on this platform".to_string(),
    ))
}

impl DirectoryReservationRegistry {
    fn open(root: &Path) -> Result<Self, SftpOpsError> {
        let canonical_root = dunce::canonicalize(root)?;
        let backend_key = hex_bytes(&Sha256::digest(path_registry_bytes(&canonical_root)));
        #[cfg(test)]
        let registry_root = std::env::temp_dir()
            .join("zaplex-transfer-reservation-registry-tests")
            .join(&backend_key);
        #[cfg(not(test))]
        let registry_root = warp_core::paths::secure_state_dir()
            .unwrap_or_else(warp_core::paths::state_dir)
            .join("file-manager-transfer-reservations-v1")
            .join(&backend_key);
        fs::create_dir_all(&registry_root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&registry_root, fs::Permissions::from_mode(0o700))?;
        }
        let registry_lock = lock_registry_root(&registry_root)?;
        Self::prune_owned_temporaries_locked(&registry_root)?;
        let secret_path = registry_root.join("secret-v1");
        let secret = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&secret_path)
        {
            Ok(mut file) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    file.set_permissions(fs::Permissions::from_mode(0o600))?;
                }
                let material = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
                let digest = Sha256::digest(material.as_bytes());
                file.write_all(hex_bytes(&digest).as_bytes())?;
                file.sync_all()?;
                digest.to_vec()
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                validate_private_registry_file(&secret_path)?;
                decode_hex(
                    std::str::from_utf8(&fs::read(&secret_path)?).map_err(|error| {
                        SftpOpsError::Operation(format!(
                            "Trusted transfer registry secret is not UTF-8: {error}"
                        ))
                    })?,
                )?
            }
            Err(error) => return Err(error.into()),
        };
        let secret: [u8; 32] = secret.try_into().map_err(|_| {
            SftpOpsError::Operation(
                "Trusted transfer registry secret has an invalid length".to_string(),
            )
        })?;
        let registry = Self {
            root: registry_root,
            secret: Arc::new(secret),
            backend_key,
            #[cfg(test)]
            fail_artifact_writes: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_write_on: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            artifact_write_calls: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            fail_artifact_transitions: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_removals: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_retirement_sync: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_retirement_unlink: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_retirement_final_sync: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_legacy_migration: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            replace_artifact_before_retirement: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            namespace_probe_failure: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            fail_namespace_migration_after_marker_replace: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_namespace_migration_after_record_replace: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_exchange_retirement_unlink: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_exchange_retirement_final_sync: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_temporary_cleanup: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_exchange_temporary_write: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_artifact_temporary_write: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_namespace_migration_marker_temporary_write: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_namespace_migration_record_temporary_write: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            exchange_create_probe_error: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            fail_exchange_create_sync: Arc::new(AtomicBool::new(false)),
        };
        registry.sync_root()?;
        drop(registry_lock);
        Ok(registry)
    }

    fn prune_owned_temporaries_locked(root: &Path) -> Result<(), SftpOpsError> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let current_slot = matches!(
                name.as_ref(),
                NAMESPACE_RECORD_TEMPORARY
                    | NAMESPACE_MIGRATION_TEMPORARY
                    | EXCHANGE_RECORD_TEMPORARY
                    | ARTIFACT_RECORD_TEMPORARY
            );
            // Review26 used record-specific bounded slots. Keep recognizing
            // those exact legacy names so an upgrade converges without
            // broadening cleanup to arbitrary private-registry files.
            let legacy_namespace = name
                .strip_prefix(".namespace-")
                .and_then(|suffix| suffix.strip_suffix(".tmp"))
                .is_some_and(|body| {
                    let device = body.strip_suffix(".migration").unwrap_or(body);
                    !device.is_empty() && device.chars().all(|character| character.is_ascii_digit())
                });
            let legacy_artifact = name
                .strip_prefix(".artifact-artifact-")
                .and_then(|suffix| suffix.strip_suffix(".record.tmp"))
                .is_some_and(|hash| {
                    hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit())
                });
            let legacy_exchange = name
                .strip_prefix(".exchange-exchange-")
                .and_then(|suffix| suffix.strip_suffix(".record.tmp"))
                .is_some_and(|hash| {
                    hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit())
                });
            if !current_slot && !legacy_namespace && !legacy_artifact && !legacy_exchange {
                continue;
            }
            validate_private_registry_file(&entry.path())?;
            fs::remove_file(entry.path())?;
        }
        fs::File::open(root)?.sync_all()?;
        Ok(())
    }

    fn record_path(&self, device: u64) -> PathBuf {
        self.root.join(format!("namespace-{device}.record"))
    }

    fn artifact_record_path(&self, path: &Path) -> PathBuf {
        self.root.join(format!(
            "artifact-{}.record",
            hex_bytes(&Sha256::digest(path_registry_bytes(path)))
        ))
    }

    fn exchange_record_path(&self, path: &Path) -> PathBuf {
        self.root.join(format!(
            "exchange-{}.record",
            hex_bytes(&Sha256::digest(path_registry_bytes(path)))
        ))
    }

    fn sync_root(&self) -> Result<(), SftpOpsError> {
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    #[cfg(unix)]
    fn lock(&self) -> Result<RegistryLock, SftpOpsError> {
        lock_registry_root(&self.root)
    }

    #[cfg(not(unix))]
    fn lock(&self) -> Result<RegistryLock, SftpOpsError> {
        Err(SftpOpsError::Operation(
            "Crash-safe transfer registry locking is unsupported on this platform".to_string(),
        ))
    }

    fn signed_payload(&self, payload: &str) -> String {
        format!(
            "{payload}\nmac={}",
            hmac_sha256(&*self.secret, payload.as_bytes())
        )
    }

    fn verify_payload(&self, contents: &str) -> Result<String, SftpOpsError> {
        let (payload, mac) = contents.rsplit_once("\nmac=").ok_or_else(|| {
            SftpOpsError::Operation(
                "Trusted transfer registry record has no authenticator".to_string(),
            )
        })?;
        let expected = hmac_sha256(&*self.secret, payload.as_bytes());
        if !secure_compare(mac.trim(), &expected) {
            return Err(SftpOpsError::Operation(
                "Trusted transfer registry authentication failed".to_string(),
            ));
        }
        Ok(payload.to_string())
    }

    fn namespace_payload(
        &self,
        path: &Path,
        device: u64,
        namespace_id: &str,
        object_id: &str,
        generation: &str,
    ) -> String {
        format!(
            "{DIRECTORY_RESERVATION_REGISTRY_VERSION}\nbackend={}\ndevice={device}\nnamespace={namespace_id}\npath={}\nobject={object_id}\ngeneration={generation}",
            self.backend_key,
            hex_bytes(&path_registry_bytes(path))
        )
    }

    fn probe_path(path: &Path) -> Result<Option<fs::Metadata>, SftpOpsError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn probe_namespace_record(&self, path: &Path) -> Result<Option<fs::Metadata>, SftpOpsError> {
        #[cfg(test)]
        if self
            .namespace_probe_failure
            .lock()
            .expect("namespace probe failure lock poisoned")
            .as_ref()
            == Some(&NamespaceProbeFailure::Record)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        Self::probe_path(path)
    }

    fn probe_namespace_path(&self, path: &Path) -> Result<Option<fs::Metadata>, SftpOpsError> {
        #[cfg(test)]
        if self
            .namespace_probe_failure
            .lock()
            .expect("namespace probe failure lock poisoned")
            .as_ref()
            == Some(&NamespaceProbeFailure::NamespacePath)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EACCES).into());
        }
        Self::probe_path(path)
    }

    fn probe_namespace_parent(&self, path: &Path) -> Result<Option<fs::Metadata>, SftpOpsError> {
        #[cfg(test)]
        if self
            .namespace_probe_failure
            .lock()
            .expect("namespace probe failure lock poisoned")
            .as_ref()
            == Some(&NamespaceProbeFailure::Parent)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        Self::probe_path(path)
    }

    fn write_namespace_record(
        &self,
        record: &DirectoryNamespaceRecord,
    ) -> Result<(), SftpOpsError> {
        let _lock = self.lock()?;
        let record_path = self.record_path(record.device);
        let contents = self.signed_payload(&self.namespace_payload(
            &record.path,
            record.device,
            &record.namespace_id,
            &record.object_id,
            &record.generation,
        ));
        // Registry writers hold the cross-process lock, so one slot per
        // record class bounds failed cleanup independently of transfer count.
        let temporary = self.root.join(NAMESPACE_RECORD_TEMPORARY);
        let mut file = open_owned_temporary_file(&temporary)?;
        let temporary_guard = OwnedTemporaryFile::new(temporary.clone());
        #[cfg(test)]
        let temporary_guard =
            temporary_guard.with_cleanup_failure(self.fail_temporary_cleanup.clone());
        let result = (|| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            if self.probe_namespace_record(&record_path)?.is_some() {
                let existing = self.read_and_migrate_namespace_record_locked(&record_path)?;
                let same_record = existing.path == record.path
                    && existing.device == record.device
                    && existing.namespace_id == record.namespace_id
                    && existing.object_id == record.object_id
                    && existing.generation == record.generation;
                if !same_record {
                    if self.probe_namespace_path(&existing.path)?.is_some() {
                        return Err(SftpOpsError::Operation(format!(
                            "Trusted transfer namespace record conflicts at {}",
                            record_path.display()
                        )));
                    }
                    // The registry lock serializes Zaplex writers. Reprobe the
                    // former public path immediately before the record CAS so an
                    // external recreation cannot orphan authenticated recovery.
                    if self.probe_namespace_path(&existing.path)?.is_some() {
                        return Err(SftpOpsError::Operation(format!(
                            "Trusted transfer namespace reappeared before record replacement at {}",
                            existing.path.display()
                        )));
                    }
                }
            }
            fs::rename(&temporary, &record_path)?;
            self.sync_root()
        })();
        match result {
            Ok(()) => {
                temporary_guard.commit();
                Ok(())
            }
            Err(error) => Err(temporary_failure(temporary_guard, error)),
        }
    }

    fn artifact_payload(&self, record: &PersistentArtifactRecord) -> String {
        let (file_type, size, object_id, revision) = match &record.identity {
            Some(identity) => {
                let file_type = match identity.file_type {
                    FileEntryType::File => "file",
                    FileEntryType::Directory => "directory",
                    FileEntryType::Symlink => "symlink",
                    FileEntryType::Other => "other",
                };
                (
                    file_type,
                    identity.size,
                    identity.object_id.as_str(),
                    identity.revision.as_str(),
                )
            }
            None => ("unresolved", 0, "", ""),
        };
        format!(
            "{TRANSFER_ARTIFACT_REGISTRY_VERSION}\nbackend={}\nrole={}\npath={}\nphysical={}\ntype={file_type}\nsize={size}\nobject={}\nrevision={}\ngeneration={}\nretired={}",
            self.backend_key,
            record.role,
            hex_bytes(&path_registry_bytes(&record.path)),
            record
                .physical_path
                .as_ref()
                .map(|path| hex_bytes(&path_registry_bytes(path)))
                .unwrap_or_default(),
            hex_bytes(object_id.as_bytes()),
            hex_bytes(revision.as_bytes()),
            record.generation,
            if record.retired { "true" } else { "false" }
        )
    }

    fn exchange_candidate_payload(prefix: &str, candidate: &PersistentExchangeCandidate) -> String {
        let (file_type, size, object_id, revision) = match &candidate.identity {
            Some(identity) => {
                let file_type = match identity.file_type {
                    FileEntryType::File => "file",
                    FileEntryType::Directory => "directory",
                    FileEntryType::Symlink => "symlink",
                    FileEntryType::Other => "other",
                };
                (
                    file_type,
                    identity.size,
                    identity.object_id.as_str(),
                    identity.revision.as_str(),
                )
            }
            None => ("absent", 0, "", ""),
        };
        format!(
            "{prefix}_physical={}\n{prefix}_role={}\n{prefix}_type={file_type}\n{prefix}_size={size}\n{prefix}_object={}\n{prefix}_revision={}",
            hex_bytes(&path_registry_bytes(&candidate.physical_path)),
            candidate.role,
            hex_bytes(object_id.as_bytes()),
            hex_bytes(revision.as_bytes())
        )
    }

    fn exchange_payload(&self, record: &PersistentExchangeRecord) -> String {
        format!(
            "{TRANSFER_EXCHANGE_REGISTRY_VERSION}\nbackend={}\npath={}\nphase={}\ngeneration={}\n{}\n{}",
            self.backend_key,
            hex_bytes(&path_registry_bytes(&record.path)),
            record.phase.as_str(),
            record.generation,
            Self::exchange_candidate_payload("first", &record.first),
            Self::exchange_candidate_payload("second", &record.second)
        )
    }

    fn write_exchange_record(&self, record: &PersistentExchangeRecord) -> Result<(), SftpOpsError> {
        let _lock = self.lock()?;
        let record_path = self.exchange_record_path(&record.path);
        #[cfg(test)]
        if let Some(errno) = *self
            .exchange_create_probe_error
            .lock()
            .expect("exchange create probe failure lock poisoned")
        {
            return Err(std::io::Error::from_raw_os_error(errno).into());
        }
        if Self::probe_path(&record_path)?.is_some() {
            let existing = self.read_and_migrate_exchange_record_locked(&record_path)?;
            if same_persistent_exchange_record(&existing, record) {
                return self.sync_exchange_create_root();
            }
            return Err(SftpOpsError::Operation(format!(
                "Transfer exchange record already exists at {}",
                record_path.display()
            )));
        }
        self.replace_exchange_record_file(&record_path, record, true)
    }

    fn sync_exchange_create_root(&self) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if self.fail_exchange_create_sync.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        self.sync_root()
    }

    fn transition_exchange_record(
        &self,
        expected: &PersistentExchangeRecord,
        next: &PersistentExchangeRecord,
    ) -> Result<(), SftpOpsError> {
        let _lock = self.lock()?;
        let record_path = self.exchange_record_path(&expected.path);
        let current = self.read_and_migrate_exchange_record_locked(&record_path)?;
        if !same_persistent_exchange_record(&current, expected)
            || next.path != expected.path
            || next.generation != expected.generation
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer exchange record changed before transition at {}",
                record_path.display()
            )));
        }
        self.replace_exchange_record_file(&record_path, next, false)
    }

    fn replace_exchange_record_file(
        &self,
        record_path: &Path,
        record: &PersistentExchangeRecord,
        create: bool,
    ) -> Result<(), SftpOpsError> {
        let temporary = self.root.join(EXCHANGE_RECORD_TEMPORARY);
        let contents = self.signed_payload(&self.exchange_payload(record));
        let mut file = open_owned_temporary_file(&temporary)?;
        let temporary_guard = OwnedTemporaryFile::new(temporary.clone());
        #[cfg(test)]
        let temporary_guard =
            temporary_guard.with_cleanup_failure(self.fail_temporary_cleanup.clone());
        let result = (|| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(test)]
            if self.fail_exchange_temporary_write.load(Ordering::SeqCst) {
                return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
            }
            if create {
                rename_noreplace(&temporary, record_path)?;
            } else {
                fs::rename(&temporary, record_path)?;
            }
            if create {
                self.sync_exchange_create_root()
            } else {
                self.sync_root()
            }
        })();
        match result {
            Ok(()) => {
                temporary_guard.commit();
                Ok(())
            }
            Err(error) => Err(temporary_failure(temporary_guard, error)),
        }
    }

    fn remove_exchange_record_if_matches(
        &self,
        expected: &PersistentExchangeRecord,
    ) -> Result<(), SftpOpsError> {
        let _lock = self.lock()?;
        let record_path = self.exchange_record_path(&expected.path);
        let current = match Self::probe_path(&record_path)? {
            Some(_) => self.read_and_migrate_exchange_record_locked(&record_path)?,
            None => {
                self.sync_root()?;
                return Ok(());
            }
        };
        if !same_persistent_exchange_record(&current, expected) {
            return Err(SftpOpsError::Operation(format!(
                "Transfer exchange generation changed before retirement at {}",
                record_path.display()
            )));
        }
        #[cfg(test)]
        if self.fail_exchange_retirement_unlink.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        match fs::remove_file(&record_path) {
            Ok(()) => {
                #[cfg(test)]
                if self
                    .fail_exchange_retirement_final_sync
                    .load(Ordering::SeqCst)
                {
                    return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
                }
                self.sync_root()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn write_artifact_record(&self, record: &PersistentArtifactRecord) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if self.fail_artifact_writes.load(Ordering::SeqCst)
            || self.fail_artifact_write_on.load(Ordering::SeqCst)
                == self.artifact_write_calls.fetch_add(1, Ordering::SeqCst) + 1
        {
            return Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into());
        }
        let _lock = self.lock()?;
        let record_path = self.artifact_record_path(&record.path);
        if Self::probe_path(&record_path)?.is_some() {
            let existing = self.read_and_migrate_artifact_record_locked(&record_path)?;
            if !existing.retired && !same_persistent_artifact_record(&existing, record) {
                return Err(SftpOpsError::Operation(format!(
                    "Transfer artifact record changed concurrently at {}",
                    record_path.display()
                )));
            }
        }
        self.replace_artifact_record_file(&record_path, record, false)
    }

    fn replace_artifact_record_file(
        &self,
        record_path: &Path,
        record: &PersistentArtifactRecord,
        _retiring: bool,
    ) -> Result<(), SftpOpsError> {
        let temporary = self.root.join(ARTIFACT_RECORD_TEMPORARY);
        let contents = self.signed_payload(&self.artifact_payload(record));
        let mut file = open_owned_temporary_file(&temporary)?;
        let temporary_guard = OwnedTemporaryFile::new(temporary.clone());
        #[cfg(test)]
        let temporary_guard =
            temporary_guard.with_cleanup_failure(self.fail_temporary_cleanup.clone());
        let result = (|| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(test)]
            if self.fail_artifact_temporary_write.load(Ordering::SeqCst) {
                return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
            }
            fs::rename(&temporary, record_path)?;
            #[cfg(test)]
            if _retiring && self.fail_artifact_retirement_sync.load(Ordering::SeqCst) {
                return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
            }
            self.sync_root()
        })();
        match result {
            Ok(()) => {
                temporary_guard.commit();
                Ok(())
            }
            Err(error) => Err(temporary_failure(temporary_guard, error)),
        }
    }

    fn transition_artifact_record(
        &self,
        expected: &PersistentArtifactRecord,
        next: &PersistentArtifactRecord,
    ) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if self.fail_artifact_transitions.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into());
        }
        let _lock = self.lock()?;
        let record_path = self.artifact_record_path(&expected.path);
        let current = self.read_and_migrate_artifact_record_locked(&record_path)?;
        if !same_persistent_artifact_record(&current, expected)
            || next.path != expected.path
            || next.retired
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact record changed before transition at {}",
                record_path.display()
            )));
        }
        self.replace_artifact_record_file(&record_path, next, false)
    }

    fn remove_artifact_record_if_matches(
        &self,
        expected: &PersistentArtifactRecord,
    ) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if self.fail_artifact_removals.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into());
        }
        let _lock = self.lock()?;
        let record_path = self.artifact_record_path(&expected.path);
        let current = match Self::probe_path(&record_path)? {
            Some(_) => self.read_and_migrate_artifact_record_locked(&record_path)?,
            None => {
                self.sync_root()?;
                return Ok(());
            }
        };
        #[cfg(test)]
        let current = if self
            .replace_artifact_before_retirement
            .swap(false, Ordering::SeqCst)
        {
            let replacement = current.transition(
                current.physical_path.clone(),
                current.role.clone(),
                current.identity.clone(),
            );
            self.replace_artifact_record_file(&record_path, &replacement, false)?;
            replacement
        } else {
            current
        };
        let retired = if current.retired && current.generation == expected.generation {
            current
        } else if same_persistent_artifact_record(&current, expected) {
            let retired = current.retired();
            self.replace_artifact_record_file(&record_path, &retired, true)?;
            retired
        } else {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact record changed before retirement at {}",
                record_path.display()
            )));
        };
        let current = self.read_and_migrate_artifact_record_locked(&record_path)?;
        if !same_persistent_artifact_record(&current, &retired) {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact retired generation changed before unlink at {}",
                record_path.display()
            )));
        }
        #[cfg(test)]
        if self.fail_artifact_retirement_unlink.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        fs::remove_file(&record_path)?;
        #[cfg(test)]
        if self
            .fail_artifact_retirement_final_sync
            .load(Ordering::SeqCst)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        self.sync_root()
    }

    fn read_artifact_record(
        &self,
        record_path: &Path,
    ) -> Result<PersistentArtifactRecord, SftpOpsError> {
        let _lock = self.lock()?;
        self.read_and_migrate_artifact_record_locked(record_path)
    }

    fn read_and_migrate_artifact_record_locked(
        &self,
        record_path: &Path,
    ) -> Result<PersistentArtifactRecord, SftpOpsError> {
        let current = self.read_artifact_record_unlocked(record_path)?;
        if !current.legacy {
            return Ok(current);
        }
        let mut migrated = current;
        migrated.generation = uuid::Uuid::new_v4().to_string();
        migrated.legacy = false;
        #[cfg(test)]
        if self.fail_artifact_legacy_migration.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into());
        }
        self.replace_artifact_record_file(record_path, &migrated, false)?;
        Ok(migrated)
    }

    fn read_artifact_record_unlocked(
        &self,
        record_path: &Path,
    ) -> Result<PersistentArtifactRecord, SftpOpsError> {
        validate_private_registry_file(record_path)?;
        let contents = fs::read_to_string(record_path)?;
        let payload = self.verify_payload(&contents)?;
        let mut lines = payload.lines();
        let version = lines.next();
        if version != Some(TRANSFER_ARTIFACT_REGISTRY_VERSION)
            && version != Some(LEGACY_TRANSFER_ARTIFACT_REGISTRY_VERSION)
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact record version is invalid at {}",
                record_path.display()
            )));
        }
        let mut backend = None;
        let mut role = None;
        let mut path = None;
        let mut physical_path = None;
        let mut file_type = None;
        let mut size = None;
        let mut object_id = None;
        let mut revision = None;
        let mut generation = None;
        let mut retired = None;
        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "backend" => backend = Some(value.to_string()),
                "role" => role = Some(value.to_string()),
                "path" => path = Some(path_from_registry_bytes(decode_hex(value)?)?),
                "physical" if !value.is_empty() => {
                    physical_path = Some(path_from_registry_bytes(decode_hex(value)?)?)
                }
                "type" => file_type = Some(value.to_string()),
                "size" => size = value.parse::<u64>().ok(),
                "object" => {
                    object_id = Some(String::from_utf8(decode_hex(value)?).map_err(|error| {
                        SftpOpsError::Operation(format!(
                            "Transfer artifact object ID is not UTF-8: {error}"
                        ))
                    })?)
                }
                "revision" => {
                    revision = Some(String::from_utf8(decode_hex(value)?).map_err(|error| {
                        SftpOpsError::Operation(format!(
                            "Transfer artifact revision is not UTF-8: {error}"
                        ))
                    })?)
                }
                "generation" => generation = Some(value.to_string()),
                "retired" => {
                    retired = match value {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        if backend.as_deref() != Some(&self.backend_key) {
            return Err(SftpOpsError::Operation(
                "Transfer artifact record belongs to a different backend root".to_string(),
            ));
        }
        let identity = match file_type.as_deref() {
            Some("unresolved") => None,
            Some(kind) => Some(StableEntryIdentity {
                file_type: match kind {
                    "file" => FileEntryType::File,
                    "directory" => FileEntryType::Directory,
                    "symlink" => FileEntryType::Symlink,
                    "other" => FileEntryType::Other,
                    _ => {
                        return Err(SftpOpsError::Operation(format!(
                            "Transfer artifact record has invalid type at {}",
                            record_path.display()
                        )));
                    }
                },
                size: size.ok_or_else(|| {
                    SftpOpsError::Operation("Transfer artifact record has no size".to_string())
                })?,
                object_id: object_id.unwrap_or_default(),
                revision: revision.unwrap_or_default(),
            }),
            None => {
                return Err(SftpOpsError::Operation(
                    "Transfer artifact record has no type".to_string(),
                ));
            }
        };
        Ok(PersistentArtifactRecord {
            path: path.ok_or_else(|| {
                SftpOpsError::Operation("Transfer artifact record has no path".to_string())
            })?,
            physical_path,
            role: role.ok_or_else(|| {
                SftpOpsError::Operation("Transfer artifact record has no role".to_string())
            })?,
            identity,
            generation: generation.unwrap_or_else(|| {
                format!("legacy-{}", hex_bytes(&Sha256::digest(contents.as_bytes())))
            }),
            retired: retired.unwrap_or(false),
            legacy: version == Some(LEGACY_TRANSFER_ARTIFACT_REGISTRY_VERSION),
        })
    }

    fn read_exchange_candidate(
        fields: &HashMap<String, String>,
        prefix: &str,
    ) -> Result<PersistentExchangeCandidate, SftpOpsError> {
        let value = |suffix: &str| {
            fields
                .get(&format!("{prefix}_{suffix}"))
                .map(String::as_str)
        };
        let physical_path = value("physical")
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Transfer exchange record has no {prefix} physical path"
                ))
            })
            .and_then(|value| path_from_registry_bytes(decode_hex(value)?))?;
        let role = value("role")
            .ok_or_else(|| {
                SftpOpsError::Operation(format!("Transfer exchange record has no {prefix} role"))
            })?
            .to_string();
        let identity = match value("type") {
            Some("absent") => None,
            Some(kind) => Some(StableEntryIdentity {
                file_type: match kind {
                    "file" => FileEntryType::File,
                    "directory" => FileEntryType::Directory,
                    "symlink" => FileEntryType::Symlink,
                    "other" => FileEntryType::Other,
                    _ => {
                        return Err(SftpOpsError::Operation(format!(
                            "Transfer exchange record has invalid {prefix} type"
                        )));
                    }
                },
                size: value("size")
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(|| {
                        SftpOpsError::Operation(format!(
                            "Transfer exchange record has no {prefix} size"
                        ))
                    })?,
                object_id: String::from_utf8(decode_hex(value("object").unwrap_or_default())?)
                    .map_err(|error| {
                        SftpOpsError::Operation(format!(
                            "Transfer exchange {prefix} object ID is not UTF-8: {error}"
                        ))
                    })?,
                revision: String::from_utf8(decode_hex(value("revision").unwrap_or_default())?)
                    .map_err(|error| {
                        SftpOpsError::Operation(format!(
                            "Transfer exchange {prefix} revision is not UTF-8: {error}"
                        ))
                    })?,
            }),
            None => {
                return Err(SftpOpsError::Operation(format!(
                    "Transfer exchange record has no {prefix} type"
                )));
            }
        };
        Ok(PersistentExchangeCandidate {
            physical_path,
            role,
            identity,
        })
    }

    fn read_exchange_record(
        &self,
        record_path: &Path,
    ) -> Result<PersistentExchangeRecord, SftpOpsError> {
        let _lock = self.lock()?;
        self.read_and_migrate_exchange_record_locked(record_path)
    }

    fn read_and_migrate_exchange_record_locked(
        &self,
        record_path: &Path,
    ) -> Result<PersistentExchangeRecord, SftpOpsError> {
        let current = self.read_exchange_record_unlocked(record_path)?;
        if !current.legacy {
            return Ok(current);
        }
        let mut migrated = current;
        migrated.generation = uuid::Uuid::new_v4().to_string();
        migrated.legacy = false;
        self.replace_exchange_record_file(record_path, &migrated, false)?;
        Ok(migrated)
    }

    fn read_exchange_record_unlocked(
        &self,
        record_path: &Path,
    ) -> Result<PersistentExchangeRecord, SftpOpsError> {
        validate_private_registry_file(record_path)?;
        let contents = fs::read_to_string(record_path)?;
        let payload = self.verify_payload(&contents)?;
        let mut lines = payload.lines();
        let version = lines.next();
        if version != Some(TRANSFER_EXCHANGE_REGISTRY_VERSION)
            && version != Some(LEGACY_TRANSFER_EXCHANGE_REGISTRY_VERSION)
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer exchange record version is invalid at {}",
                record_path.display()
            )));
        }
        let fields = lines
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        if fields.get("backend").map(String::as_str) != Some(&self.backend_key) {
            return Err(SftpOpsError::Operation(
                "Transfer exchange record belongs to a different backend root".to_string(),
            ));
        }
        let path = fields
            .get("path")
            .ok_or_else(|| {
                SftpOpsError::Operation("Transfer exchange record has no path".to_string())
            })
            .and_then(|value| path_from_registry_bytes(decode_hex(value)?))?;
        let phase = match fields.get("phase").map(String::as_str) {
            Some("prepared") => PersistentExchangePhase::Prepared,
            Some("applied") => PersistentExchangePhase::Applied,
            Some(_) | None => {
                return Err(SftpOpsError::Operation(
                    "Transfer exchange record has an invalid phase".to_string(),
                ));
            }
        };
        Ok(PersistentExchangeRecord {
            path,
            first: Self::read_exchange_candidate(&fields, "first")?,
            second: Self::read_exchange_candidate(&fields, "second")?,
            phase,
            generation: fields.get("generation").cloned().unwrap_or_else(|| {
                format!("legacy-{}", hex_bytes(&Sha256::digest(contents.as_bytes())))
            }),
            legacy: version == Some(LEGACY_TRANSFER_EXCHANGE_REGISTRY_VERSION),
        })
    }

    fn artifact_records(
        &self,
    ) -> Result<Vec<(PathBuf, Result<PersistentArtifactRecord, SftpOpsError>)>, SftpOpsError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("artifact-") && name.ends_with(".record") {
                let record_path = entry.path();
                match self.read_artifact_record(&record_path) {
                    Ok(record) if record.retired => {
                        if let Err(error) = self.remove_artifact_record_if_matches(&record) {
                            records.push((record_path, Err(error)));
                        }
                    }
                    result => records.push((record_path, result)),
                }
            }
        }
        Ok(records)
    }

    fn exchange_records(
        &self,
    ) -> Result<Vec<(PathBuf, Result<PersistentExchangeRecord, SftpOpsError>)>, SftpOpsError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("exchange-") && name.ends_with(".record") {
                records.push((entry.path(), self.read_exchange_record(&entry.path())));
            }
        }
        Ok(records)
    }

    fn read_namespace_record(
        &self,
        record_path: &Path,
    ) -> Result<DirectoryNamespaceRecord, SftpOpsError> {
        let _lock = self.lock()?;
        self.read_and_migrate_namespace_record_locked(record_path)
    }

    fn read_and_migrate_namespace_record_locked(
        &self,
        record_path: &Path,
    ) -> Result<DirectoryNamespaceRecord, SftpOpsError> {
        let current = self.read_namespace_record_unlocked(record_path)?;
        if !current.legacy {
            return Ok(current);
        }
        let mut migrated = current;
        migrated.generation = uuid::Uuid::new_v4().to_string();
        migrated.legacy = false;
        let marker_path = migrated.path.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
        let marker_payload = self.verify_marker(&fs::read_to_string(&marker_path)?)?;
        let marker_fields = marker_payload
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once('='))
            .collect::<HashMap<_, _>>();
        let marker_matches = marker_fields.get("backend").copied()
            == Some(self.backend_key.as_str())
            && marker_fields
                .get("device")
                .and_then(|value| value.parse::<u64>().ok())
                == Some(migrated.device)
            && marker_fields.get("namespace").copied() == Some(migrated.namespace_id.as_str())
            && marker_fields
                .get("path")
                .map(|value| decode_hex(value).and_then(path_from_registry_bytes))
                .transpose()?
                .as_ref()
                == Some(&migrated.path)
            && marker_fields.get("object").copied() == Some(migrated.object_id.as_str());
        if !marker_matches {
            return Err(SftpOpsError::Operation(format!(
                "Legacy namespace marker does not match its authenticated record at {}",
                marker_path.display()
            )));
        }
        let marker_temporary = migrated
            .path
            .join(format!(".{DIRECTORY_RESERVATION_NAMESPACE_MARKER}.tmp"));
        let marker_contents = self.signed_payload(&self.namespace_payload(
            &migrated.path,
            migrated.device,
            &migrated.namespace_id,
            &migrated.object_id,
            &migrated.generation,
        ));
        let mut marker_file = open_owned_temporary_file(&marker_temporary)?;
        let marker_guard = OwnedTemporaryFile::new(marker_temporary.clone());
        #[cfg(test)]
        let marker_guard = marker_guard.with_cleanup_failure(self.fail_temporary_cleanup.clone());
        let marker_result: Result<(), SftpOpsError> = (|| {
            marker_file.write_all(marker_contents.as_bytes())?;
            marker_file.sync_all()?;
            drop(marker_file);
            #[cfg(test)]
            if self
                .fail_namespace_migration_marker_temporary_write
                .load(Ordering::SeqCst)
            {
                return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
            }
            fs::rename(&marker_temporary, &marker_path)?;
            fs::File::open(&migrated.path)?.sync_all()?;
            Ok(())
        })();
        match marker_result {
            Ok(()) => marker_guard.commit(),
            Err(error) => return Err(temporary_failure(marker_guard, error)),
        }
        #[cfg(test)]
        if self
            .fail_namespace_migration_after_marker_replace
            .load(Ordering::SeqCst)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        let contents = self.signed_payload(&self.namespace_payload(
            &migrated.path,
            migrated.device,
            &migrated.namespace_id,
            &migrated.object_id,
            &migrated.generation,
        ));
        let temporary = self.root.join(NAMESPACE_MIGRATION_TEMPORARY);
        let mut file = open_owned_temporary_file(&temporary)?;
        let temporary_guard = OwnedTemporaryFile::new(temporary.clone());
        #[cfg(test)]
        let temporary_guard =
            temporary_guard.with_cleanup_failure(self.fail_temporary_cleanup.clone());
        let record_result: Result<(), SftpOpsError> = (|| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(test)]
            if self
                .fail_namespace_migration_record_temporary_write
                .load(Ordering::SeqCst)
            {
                return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
            }
            fs::rename(&temporary, record_path)?;
            self.sync_root()
        })();
        match record_result {
            Ok(()) => temporary_guard.commit(),
            Err(error) => return Err(temporary_failure(temporary_guard, error)),
        }
        #[cfg(test)]
        if self
            .fail_namespace_migration_after_record_replace
            .load(Ordering::SeqCst)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO).into());
        }
        Ok(migrated)
    }

    fn read_namespace_record_unlocked(
        &self,
        record_path: &Path,
    ) -> Result<DirectoryNamespaceRecord, SftpOpsError> {
        validate_private_registry_file(record_path)?;
        let contents = fs::read_to_string(record_path)?;
        let payload = self.verify_payload(&contents)?;
        let mut lines = payload.lines();
        let version = lines.next();
        if version != Some(DIRECTORY_RESERVATION_REGISTRY_VERSION)
            && version != Some(LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION)
        {
            return Err(SftpOpsError::Operation(format!(
                "Trusted transfer registry version is invalid at {}",
                record_path.display()
            )));
        }
        let mut backend = None;
        let mut device = None;
        let mut namespace_id = None;
        let mut path = None;
        let mut object_id = None;
        let mut generation = None;
        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "backend" => backend = Some(value.to_string()),
                "device" => device = value.parse::<u64>().ok(),
                "namespace" => namespace_id = Some(value.to_string()),
                "path" => path = Some(path_from_registry_bytes(decode_hex(value)?)?),
                "object" => object_id = Some(value.to_string()),
                "generation" => generation = Some(value.to_string()),
                _ => {}
            }
        }
        if backend.as_deref() != Some(&self.backend_key) {
            return Err(SftpOpsError::Operation(
                "Trusted transfer registry belongs to a different backend root".to_string(),
            ));
        }
        Ok(DirectoryNamespaceRecord {
            path: path.ok_or_else(|| {
                SftpOpsError::Operation("Trusted namespace record has no path".to_string())
            })?,
            device: device.ok_or_else(|| {
                SftpOpsError::Operation("Trusted namespace record has no device".to_string())
            })?,
            namespace_id: namespace_id.ok_or_else(|| {
                SftpOpsError::Operation("Trusted namespace record has no ID".to_string())
            })?,
            object_id: object_id.ok_or_else(|| {
                SftpOpsError::Operation("Trusted namespace record has no object ID".to_string())
            })?,
            generation: generation.unwrap_or_else(|| {
                format!("legacy-{}", hex_bytes(&Sha256::digest(contents.as_bytes())))
            }),
            legacy: version == Some(LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION),
        })
    }

    fn records(&self) -> Vec<Result<DirectoryNamespaceRecord, SftpOpsError>> {
        match fs::read_dir(&self.root) {
            Ok(entries) => entries
                .map(|entry| {
                    let entry = entry.map_err(SftpOpsError::from)?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("namespace-") && name.ends_with(".record") {
                        self.read_namespace_record(&entry.path()).map(Some)
                    } else {
                        Ok(None)
                    }
                })
                .filter_map(|record| match record {
                    Ok(Some(record)) => Some(Ok(record)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect(),
            Err(error) => vec![Err(error.into())],
        }
    }

    fn marker_contents(&self, payload: &str) -> String {
        self.signed_payload(payload)
    }

    fn namespace_name(&self) -> String {
        let material = format!("namespace-name-v1\nbackend={}", self.backend_key);
        let digest = hmac_sha256(self.secret.as_ref(), material.as_bytes());
        format!("{DIRECTORY_RESERVATION_NAMESPACE_PREFIX}-{}", &digest[..16])
    }

    fn verify_marker(&self, contents: &str) -> Result<String, SftpOpsError> {
        self.verify_payload(contents)
    }
}

#[cfg(unix)]
struct RegistryLock(fs::File);

#[cfg(unix)]
impl Drop for RegistryLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(not(unix))]
struct RegistryLock;

impl InMemorySftpBackend {
    /// Creates a new in-memory backend using the specified directory as root.
    pub fn new(root: PathBuf) -> Self {
        let registry = DirectoryReservationRegistry::open(&root);
        let mut startup_unresolved_paths = HashSet::new();
        let directory_reservation_registry = match registry {
            Ok(registry) => Some(registry),
            Err(error) => {
                startup_unresolved_paths.insert(Self::unresolved_registry_path(
                    "registry-open",
                    &error.to_string(),
                ));
                None
            }
        };
        let backend = Self {
            root,
            directory_reservation_registry,
            safe_mutation_capabilities: Mutex::new(HashMap::new()),
            cleanup_recovery_identities: Mutex::new(HashMap::new()),
            directory_reservation_namespaces: Mutex::new(HashMap::new()),
            reserved_directory_namespace_paths: Mutex::new(HashMap::new()),
            #[cfg(test)]
            force_in_tree_directory_reservation_namespace: false,
            opaque_recovery_paths: Arc::new(Mutex::new(HashMap::new())),
            opaque_recovery_markers: Mutex::new(HashMap::new()),
            artifact_lifecycle: Mutex::new(()),
            persistent_artifact_records: Mutex::new(HashMap::new()),
            persistent_exchange_records: Mutex::new(HashMap::new()),
            startup_unresolved_paths: Mutex::new(startup_unresolved_paths),
            #[cfg(test)]
            before_rename: None,
            #[cfg(test)]
            after_guarded_rename_check_before_mutation: None,
            #[cfg(test)]
            before_guarded_rename_restore: None,
            #[cfg(test)]
            before_placeholder_isolation: None,
            #[cfg(test)]
            before_placeholder_tombstone_cleanup: None,
            #[cfg(test)]
            after_placeholder_final_check_before_delete: None,
            #[cfg(test)]
            before_private_placeholder_unlink: None,
            #[cfg(test)]
            after_private_namespace_unlink: None,
            #[cfg(test)]
            before_private_placeholder_isolation: None,
            #[cfg(test)]
            after_placeholder_isolation_before_classification: None,
            #[cfg(test)]
            after_guarded_exchange_before_classification: None,
            #[cfg(test)]
            after_guarded_cleanup_verification: None,
            #[cfg(test)]
            after_rename: None,
            #[cfg(test)]
            forced_lstat_error: None,
            #[cfg(test)]
            fail_staged_identity: false,
            #[cfg(test)]
            fail_published_identity: None,
            #[cfg(test)]
            published_identity_calls: AtomicU64::new(0),
            #[cfg(test)]
            fail_delete_after_apply: None,
            #[cfg(test)]
            fail_delete_matching: None,
            #[cfg(test)]
            fail_delete_matching_once: false,
            #[cfg(test)]
            delete_matching_failed: AtomicBool::new(false),
            #[cfg(test)]
            fail_replace_after_apply: None,
            #[cfg(test)]
            before_replace: None,
            #[cfg(test)]
            after_replace: None,
            #[cfg(test)]
            fail_rename_after_apply: None,
            #[cfg(test)]
            fail_writer_on_create: None,
            #[cfg(test)]
            fail_writer_create_after_apply: None,
            #[cfg(test)]
            corrupt_writer_on_create: None,
            #[cfg(test)]
            writer_creates: AtomicU64::new(0),
            #[cfg(test)]
            fail_directory_create_after_apply: None,
            #[cfg(test)]
            directory_creates: AtomicU64::new(0),
            #[cfg(test)]
            after_directory_create_before_anchor: None,
            #[cfg(test)]
            after_directory_anchor_before_publish: None,
            #[cfg(test)]
            after_namespace_create_before_anchor: None,
            #[cfg(test)]
            after_writer_validation_before_open: None,
            #[cfg(test)]
            directory_reservation_failure: None,
            #[cfg(test)]
            ignore_noreplace_probe_semantics: false,
            #[cfg(test)]
            fail_preflight_cleanup: false,
            #[cfg(test)]
            preflight_collision_suffix: None,
            #[cfg(test)]
            preflight_collision: Mutex::new(None),
            #[cfg(test)]
            preflight_rename_copy_unlink: false,
            #[cfg(test)]
            preflight_exchange_content_swap: false,
            #[cfg(test)]
            replace_preflight_source_before_rename: false,
            #[cfg(test)]
            replace_preflight_source_before_exchange: false,
            #[cfg(test)]
            replace_preflight_sources_before_reject: false,
            #[cfg(test)]
            preflight_mutation_replacement: Mutex::new(None),
            #[cfg(test)]
            preflight_reject_replacements: Mutex::new(Vec::new()),
            #[cfg(test)]
            fail_preflight_rename_after_apply: false,
            #[cfg(test)]
            fail_preflight_create_after_apply: None,
            #[cfg(test)]
            preflight_uncertain_create: Mutex::new(None),
            #[cfg(test)]
            replace_preflight_owned_before_cleanup: None,
            #[cfg(test)]
            preflight_cleanup_replacement: Mutex::new(None),
            #[cfg(test)]
            replace_preflight_owned_after_check: None,
            #[cfg(test)]
            preflight_cleanup_replacement_after_check: Mutex::new(None),
            #[cfg(test)]
            observe_preflight_cleanup_anchor: None,
            #[cfg(test)]
            preflight_cleanup_anchor_observed: AtomicBool::new(false),
            #[cfg(test)]
            force_preflight_inode_reuse: None,
            #[cfg(test)]
            preflight_inode_reuse_observation: Mutex::new(None),
            #[cfg(test)]
            fail_recursive_delete_partially: false,
            #[cfg(test)]
            after_stable_identity: None,
            #[cfg(test)]
            before_guarded_delete: None,
            #[cfg(test)]
            fail_isolated_delete_before_apply: false,
            #[cfg(test)]
            fail_directory_marker_cleanup: false,
            #[cfg(test)]
            namespace_scan_failure: Mutex::new(None),
            #[cfg(test)]
            sibling_recovery_failure: Mutex::new(None),
            #[cfg(test)]
            before_owned_candidate_anchor_open: None,
            #[cfg(test)]
            before_sibling_registry_commit: None,
            #[cfg(test)]
            after_sibling_registry_write: None,
            #[cfg(test)]
            before_sibling_rescan_iteration: None,
            #[cfg(test)]
            before_sibling_recovery_anchor_open: None,
            #[cfg(test)]
            after_artifact_retirement_generation_check: None,
            #[cfg(test)]
            at_artifact_association_cutpoint: None,
            #[cfg(test)]
            at_failed_directory_candidate_association_cutpoint: None,
            #[cfg(test)]
            at_opaque_cleanup_sibling_publication_cutpoint: None,
            #[cfg(test)]
            after_opaque_cleanup_source_read_before_lifecycle: None,
        };
        backend.discover_root_directory_reservations();
        backend.discover_persistent_artifacts();
        backend
    }

    fn unresolved_registry_path(kind: &str, detail: &str) -> PathBuf {
        PathBuf::from("/.zaplex-unresolved-transfer-registry").join(format!(
            "{kind}-{}",
            hex_bytes(&Sha256::digest(detail.as_bytes()))
        ))
    }

    /// Gets the root directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub(crate) fn with_before_rename(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_rename = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_guarded_rename_check_before_mutation(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_guarded_rename_check_before_mutation = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_guarded_rename_restore(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_guarded_rename_restore = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_placeholder_isolation(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_placeholder_isolation = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_placeholder_tombstone_cleanup(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_placeholder_tombstone_cleanup = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_placeholder_final_check_before_delete(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_placeholder_final_check_before_delete = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_private_placeholder_unlink(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_private_placeholder_unlink = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_private_placeholder_isolation(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_private_placeholder_isolation = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_placeholder_isolation_before_classification(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_placeholder_isolation_before_classification = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_guarded_exchange_before_classification(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_guarded_exchange_before_classification = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_guarded_cleanup_verification(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_guarded_cleanup_verification = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_owned_candidate_anchor_open(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_owned_candidate_anchor_open = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_sibling_registry_commit(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_sibling_registry_commit = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_sibling_registry_write(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_sibling_registry_write = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_sibling_rescan_iteration(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_sibling_rescan_iteration = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_private_namespace_unlink(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_private_namespace_unlink = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_sibling_recovery_anchor_open(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_sibling_recovery_anchor_open = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_artifact_retirement_generation_check(
        mut self,
        hook: impl Fn(&InMemorySftpBackend, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_artifact_retirement_generation_check = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_artifact_association_cutpoint(
        mut self,
        hook: impl Fn(&InMemorySftpBackend, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.at_artifact_association_cutpoint = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_failed_directory_candidate_association_cutpoint(
        mut self,
        hook: impl Fn(&InMemorySftpBackend, &Path, bool) + Send + Sync + 'static,
    ) -> Self {
        self.at_failed_directory_candidate_association_cutpoint = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_opaque_cleanup_sibling_publication_cutpoint(
        mut self,
        hook: impl Fn(&InMemorySftpBackend, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.at_opaque_cleanup_sibling_publication_cutpoint = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_opaque_cleanup_source_read_before_lifecycle(
        mut self,
        hook: impl Fn(&InMemorySftpBackend, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_opaque_cleanup_source_read_before_lifecycle = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_writes_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_writes
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn with_artifact_registry_write_failure_on(self, write: u64) -> Self {
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .expect("test registry must exist");
        registry.artifact_write_calls.store(0, Ordering::SeqCst);
        registry
            .fail_artifact_write_on
            .store(write, Ordering::SeqCst);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_in_tree_directory_reservation_namespace(mut self) -> Self {
        self.force_in_tree_directory_reservation_namespace = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_transitions_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_transitions
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_removals_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_removals
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_retirement_sync_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_retirement_sync
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_retirement_unlink_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_retirement_unlink
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_artifact_registry_retirement_final_sync_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .fail_artifact_retirement_final_sync
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn clear_artifact_registry_retirement_failures_for_test(&self) {
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .expect("test registry must exist");
        registry
            .fail_artifact_removals
            .store(false, Ordering::SeqCst);
        registry
            .fail_artifact_retirement_sync
            .store(false, Ordering::SeqCst);
        registry
            .fail_artifact_retirement_unlink
            .store(false, Ordering::SeqCst);
        registry
            .fail_artifact_retirement_final_sync
            .store(false, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn replace_artifact_before_retirement_for_test(&self) {
        self.directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .replace_artifact_before_retirement
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn with_namespace_probe_failure(self, failure: NamespaceProbeFailure) -> Self {
        *self
            .directory_reservation_registry
            .as_ref()
            .expect("test registry must exist")
            .namespace_probe_failure
            .lock()
            .expect("namespace probe failure lock poisoned") = Some(failure);
        self.directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .clear();
        self
    }

    #[cfg(test)]
    pub(crate) fn namespace_record_contents_for_test(&self) -> Vec<Vec<u8>> {
        let Some(registry) = &self.directory_reservation_registry else {
            return Vec::new();
        };
        let mut records = fs::read_dir(&registry.root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("namespace-") && name.ends_with(".record")
            })
            .map(|entry| fs::read(entry.path()).unwrap())
            .collect::<Vec<_>>();
        records.sort();
        records
    }

    #[cfg(test)]
    pub(crate) fn with_sibling_recovery_failure(self, failure: SiblingRecoveryFailure) -> Self {
        *self
            .sibling_recovery_failure
            .lock()
            .expect("sibling recovery failure lock poisoned") = Some(failure);
        self
    }

    #[cfg(test)]
    pub(crate) fn clear_sibling_recovery_failure(&self) {
        *self
            .sibling_recovery_failure
            .lock()
            .expect("sibling recovery failure lock poisoned") = None;
    }

    #[cfg(test)]
    pub(crate) fn with_after_rename(
        mut self,
        hook: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_rename = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_directory_create_before_anchor(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_directory_create_before_anchor = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_namespace_create_before_anchor(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_namespace_create_before_anchor = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_writer_validation_before_open(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_writer_validation_before_open = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_directory_anchor_before_publish(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_directory_anchor_before_publish = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_directory_reservation_failure(
        mut self,
        failure: DirectoryReservationFailure,
    ) -> Self {
        self.directory_reservation_failure = Some(failure);
        self
    }

    #[cfg(test)]
    pub(crate) fn directory_reservation_artifact_count(&self) -> usize {
        self.directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .values()
            .filter_map(|namespace| fs::read_dir(&namespace.path).ok())
            .flat_map(|entries| entries.filter_map(Result::ok))
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count()
    }

    #[cfg(test)]
    pub(crate) fn startup_recovery_paths_for_test(&self) -> Vec<PathBuf> {
        self.startup_recovery_paths()
    }

    #[cfg(test)]
    pub(crate) fn rescan_namespace_with_failure_for_test(&self, failure: NamespaceScanFailure) {
        *self
            .namespace_scan_failure
            .lock()
            .expect("namespace scan failure lock poisoned") = Some(failure);
        let namespace = self
            .directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .values()
            .next()
            .cloned()
            .expect("test namespace must exist");
        self.scan_directory_reservations(&namespace);
    }

    fn discovered_recovery_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .keys()
            .cloned()
            .chain(
                self.persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned")
                    .keys()
                    .cloned(),
            )
            .chain(
                self.persistent_exchange_records
                    .lock()
                    .expect("persistent transfer exchange lock poisoned")
                    .keys()
                    .cloned(),
            )
            .chain(
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .iter()
                    .cloned(),
            )
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }

    fn persistent_artifact_role(path: &Path) -> Option<&'static str> {
        let name = path.file_name()?.to_string_lossy();
        [
            (".zaplex-transfer-", "file-stage"),
            (".zaplex-tree-", "directory-stage"),
            (".zaplex-backup-", "backup"),
            (".zaplex-source-", "source-quarantine"),
            (".zaplex-delete-", "cleanup-tombstone"),
        ]
        .into_iter()
        .find_map(|(marker, role)| name.contains(marker).then_some(role))
    }

    fn persistent_artifact_physical_path(&self, path: &Path) -> Option<PathBuf> {
        let opaque_path = {
            self.opaque_recovery_paths
                .lock()
                .expect("opaque recovery path lock poisoned")
                .get(path)
                .cloned()
        };
        opaque_path.or_else(|| self.to_local(path).ok())
    }

    fn persist_artifact_intent(&self, path: &Path) -> Result<bool, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let Some(role) = Self::persistent_artifact_role(path) else {
            return Ok(false);
        };
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        let physical_path = self.persistent_artifact_physical_path(path);
        let current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned();
        if let Some(current) = current {
            if !current.retired && current.physical_path == physical_path && current.role == role {
                return Ok(true);
            }
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact intent changed concurrently at {}",
                path.display()
            )));
        }
        let record = PersistentArtifactRecord::active(
            path.to_path_buf(),
            physical_path,
            role.to_string(),
            None,
        );
        registry.write_artifact_record(&record)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), record);
        Ok(true)
    }

    fn persist_artifact_identity(
        &self,
        path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        self.persist_artifact_identity_locked(path, anchor)
    }

    fn persist_artifact_identity_locked(
        &self,
        path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let Some(role) = Self::persistent_artifact_role(path) else {
            return Ok(());
        };
        if !anchor.matches_path(path)? {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact changed before persistence at {}",
                path.display()
            )));
        }
        let identity = anchor.identity()?;
        let current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned();
        let record = current.as_ref().map_or_else(
            || {
                PersistentArtifactRecord::active(
                    path.to_path_buf(),
                    self.persistent_artifact_physical_path(path),
                    role.to_string(),
                    Some(identity.clone()),
                )
            },
            |current| {
                current.transition(
                    self.persistent_artifact_physical_path(path),
                    role.to_string(),
                    Some(identity.clone()),
                )
            },
        );
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        if let Some(current) = current {
            registry.transition_artifact_record(&current, &record)?;
        } else {
            registry.write_artifact_record(&record)?;
        }
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), record);
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .insert(path.to_path_buf(), (identity, anchor));
        Ok(())
    }

    fn persist_artifact_moving_identity(
        &self,
        path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let Some(role) = Self::persistent_artifact_role(path) else {
            return Ok(());
        };
        let identity = anchor.identity()?;
        let current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned();
        let record = current.as_ref().map_or_else(
            || {
                PersistentArtifactRecord::active(
                    path.to_path_buf(),
                    self.persistent_artifact_physical_path(path),
                    role.to_string(),
                    Some(identity.clone()),
                )
            },
            |current| {
                current.transition(
                    self.persistent_artifact_physical_path(path),
                    role.to_string(),
                    Some(identity.clone()),
                )
            },
        );
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        if let Some(current) = current {
            registry.transition_artifact_record(&current, &record)?;
        } else {
            registry.write_artifact_record(&record)?;
        }
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), record);
        Ok(())
    }

    fn persist_unresolved_diagnostic(&self, kind: &str, detail: &str) -> PathBuf {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let path = Self::unresolved_registry_path(kind, detail);
        let record = PersistentArtifactRecord::active(
            path.clone(),
            None,
            format!("unresolved-{kind}"),
            None,
        );
        let persisted = self
            .directory_reservation_registry
            .as_ref()
            .is_some_and(|registry| registry.write_artifact_record(&record).is_ok());
        if persisted {
            self.persistent_artifact_records
                .lock()
                .expect("persistent transfer artifact lock poisoned")
                .insert(path.clone(), record);
        }
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .insert(path.clone());
        path
    }

    fn persist_unresolved_physical_candidate(
        &self,
        kind: &str,
        physical_path: &Path,
        identity: Option<StableEntryIdentity>,
    ) -> Result<PathBuf, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        self.persist_unresolved_physical_candidate_locked(kind, physical_path, identity, None, None)
    }

    fn persist_unresolved_physical_candidate_with_anchor(
        &self,
        kind: &str,
        physical_path: &Path,
        identity: StableEntryIdentity,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<PathBuf, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        self.persist_unresolved_physical_candidate_locked(
            kind,
            physical_path,
            Some(identity.clone()),
            Some((identity, anchor)),
            None,
        )
    }

    fn persist_unresolved_physical_candidate_locked(
        &self,
        kind: &str,
        physical_path: &Path,
        identity: Option<StableEntryIdentity>,
        association: Option<(StableEntryIdentity, Arc<dyn BackendOwnershipAnchor>)>,
        failed_directory_candidate: Option<bool>,
    ) -> Result<PathBuf, SftpOpsError> {
        if let Some((expected, anchor)) = association.as_ref() {
            let actual = anchor.identity()?;
            if !same_immutable_object(expected, &actual)
                || !anchor.matches_local_path(physical_path)?
            {
                return Err(SftpOpsError::Operation(format!(
                    "Recovery candidate changed before atomic publication at {}",
                    physical_path.display()
                )));
            }
        }
        let identity_detail = identity
            .as_ref()
            .map(|identity| identity.object_id.as_str())
            .unwrap_or("unknown");
        let detail = format!("{}:{identity_detail}", physical_path.display());
        let path = Self::unresolved_registry_path(kind, &detail);
        let physical_path = physical_path.to_path_buf();
        let role = format!("unresolved-{kind}");
        let current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(&path)
            .cloned();
        let record = current
            .filter(|current| {
                !current.retired
                    && current.physical_path.as_ref() == Some(&physical_path)
                    && current.role == role
                    && same_optional_identity(current.identity.as_ref(), identity.as_ref())
            })
            .unwrap_or_else(|| {
                PersistentArtifactRecord::active(
                    path.clone(),
                    Some(physical_path.clone()),
                    role,
                    identity,
                )
            });
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        registry.write_artifact_record(&record)?;
        #[cfg(test)]
        if let Some(owned) = failed_directory_candidate {
            if let Some(hook) = &self.at_failed_directory_candidate_association_cutpoint {
                hook(self, &path, owned);
            }
        }
        #[cfg(test)]
        if association.is_some() {
            if let Some(hook) = &self.at_artifact_association_cutpoint {
                hook(self, &path);
            }
        }
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.clone(), record);
        self.opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .insert(path.clone(), physical_path);
        if let Some((identity, anchor)) = association {
            self.cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .insert(path.clone(), (identity, anchor));
        }
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .insert(path.clone());
        Ok(path)
    }

    fn transition_persistent_artifact_identity(
        &self,
        path: &Path,
        next_role: &str,
        next_anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let Some(current) = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned()
        else {
            return self.persist_artifact_identity_locked(path, next_anchor);
        };
        let next_identity = next_anchor.identity()?;
        let next = current.transition(
            current
                .physical_path
                .clone()
                .or_else(|| self.persistent_artifact_physical_path(path)),
            next_role.to_string(),
            Some(next_identity.clone()),
        );
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        registry.transition_artifact_record(&current, &next)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), next);
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .insert(path.to_path_buf(), (next_identity, next_anchor));
        Ok(())
    }

    fn associate_persistent_artifact_anchor(
        &self,
        path: &Path,
        identity: StableEntryIdentity,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        if !self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .is_some_and(|record| {
                !record.retired
                    && record
                        .identity
                        .as_ref()
                        .is_some_and(|expected| same_immutable_object(expected, &identity))
            })
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact retired before its ownership anchor was associated at {}",
                path.display()
            )));
        }
        #[cfg(test)]
        if let Some(hook) = &self.at_artifact_association_cutpoint {
            hook(self, path);
        }
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .insert(path.to_path_buf(), (identity, anchor));
        Ok(())
    }

    fn exchange_candidate_matches(
        candidate: &PersistentExchangeCandidate,
        physical_path: &Path,
    ) -> Result<bool, SftpOpsError> {
        match &candidate.identity {
            Some(expected) => match fs::symlink_metadata(physical_path) {
                Ok(metadata) => Ok(same_immutable_object(
                    expected,
                    &stable_identity_from_local_metadata(&metadata),
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            },
            None => match fs::symlink_metadata(physical_path) {
                Ok(_) => Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
                Err(error) => Err(error.into()),
            },
        }
    }

    fn exchange_path_is_absent(path: &Path) -> Result<bool, SftpOpsError> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    fn applied_deleted_exchange_contains(&self, physical_path: &Path) -> bool {
        let records = self
            .persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        records.into_iter().any(|record| {
            record.phase == PersistentExchangePhase::Applied
                && (record.first.physical_path == physical_path
                    || record.second.physical_path == physical_path)
                && Self::exchange_path_is_absent(&record.first.physical_path).unwrap_or(false)
                && Self::exchange_path_is_absent(&record.second.physical_path).unwrap_or(false)
        })
    }

    fn resolve_persistent_exchange(
        &self,
        record: &PersistentExchangeRecord,
    ) -> Result<Vec<PathBuf>, SftpOpsError> {
        if record.phase == PersistentExchangePhase::Applied
            && Self::exchange_path_is_absent(&record.first.physical_path)?
            && Self::exchange_path_is_absent(&record.second.physical_path)?
        {
            // Both candidates are absent only after the trusted private
            // deletion commit. Retire records tied to either physical path;
            // no public object is mutated during restart convergence.
            let artifact_paths = self
                .persistent_artifact_records
                .lock()
                .expect("persistent transfer artifact lock poisoned")
                .iter()
                .filter_map(|(path, artifact)| {
                    artifact
                        .physical_path
                        .as_ref()
                        .is_some_and(|physical| {
                            physical == &record.first.physical_path
                                || physical == &record.second.physical_path
                        })
                        .then(|| path.clone())
                })
                .collect::<Vec<_>>();
            for path in artifact_paths {
                self.release_cleanup_recovery_path(&path)?;
            }
            self.release_exchange_record(&record.path)?;
            return Ok(Vec::new());
        }
        let first_at_first =
            Self::exchange_candidate_matches(&record.first, &record.first.physical_path)?;
        let second_at_second =
            Self::exchange_candidate_matches(&record.second, &record.second.physical_path)?;
        let first_at_second =
            Self::exchange_candidate_matches(&record.first, &record.second.physical_path)?;
        let second_at_first =
            Self::exchange_candidate_matches(&record.second, &record.first.physical_path)?;
        if record.phase == PersistentExchangePhase::Applied
            && !first_at_first
            && first_at_second
            && !second_at_first
            && !second_at_second
        {
            let identity = record.first.identity.clone().ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Applied cleanup exchange has no owned identity for {}",
                    record.path.display()
                ))
            })?;
            let file = open_local_cleanup_anchor(&record.second.physical_path)?;
            let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file,
                root: self.root.clone(),
                opaque_paths: Some(self.opaque_recovery_paths.clone()),
            });
            if !anchor.matches_local_path(&record.second.physical_path)?
                || !anchor
                    .identity()
                    .is_ok_and(|actual| same_immutable_object(&identity, &actual))
                || (identity.file_type == FileEntryType::File && anchor.link_count()? != Some(1))
            {
                return Err(SftpOpsError::Operation(format!(
                    "Applied cleanup exchange lost its owned private candidate for {}",
                    record.path.display()
                )));
            }
            let (recovery_path, stale_paths) = {
                let artifacts = self
                    .persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned");
                let recovery_path = artifacts.iter().find_map(|(path, artifact)| {
                    (artifact.physical_path.as_ref() == Some(&record.second.physical_path)
                        && artifact
                            .identity
                            .as_ref()
                            .is_some_and(|actual| same_immutable_object(&identity, actual)))
                    .then(|| path.clone())
                });
                let stale_paths = artifacts
                    .iter()
                    .filter_map(|(path, artifact)| {
                        (artifact.physical_path.as_ref() == Some(&record.first.physical_path))
                            .then(|| path.clone())
                    })
                    .collect::<Vec<_>>();
                (recovery_path, stale_paths)
            };
            let recovery_path = match recovery_path {
                Some(recovery_path) => recovery_path,
                None => self.persist_unresolved_physical_candidate_with_anchor(
                    "exchange-owned-private",
                    &record.second.physical_path,
                    identity.clone(),
                    anchor.clone(),
                )?,
            };
            if !self
                .cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .contains_key(&recovery_path)
            {
                self.associate_persistent_artifact_anchor(&recovery_path, identity, anchor)?;
            }
            for path in stale_paths {
                self.release_cleanup_recovery_path(&path)?;
            }
            self.release_exchange_record(&record.path)?;
            return Ok(vec![recovery_path]);
        }
        let resolved = if first_at_first && second_at_second {
            [
                (&record.first, &record.first.physical_path),
                (&record.second, &record.second.physical_path),
            ]
        } else if first_at_second && second_at_first {
            [
                (&record.first, &record.second.physical_path),
                (&record.second, &record.first.physical_path),
            ]
        } else {
            return Err(SftpOpsError::Operation(format!(
                "Transfer exchange candidates are indeterminate for {}",
                record.path.display()
            )));
        };
        let mut recovery_paths = Vec::new();
        for (candidate, physical_path) in resolved {
            let Some(identity) = candidate.identity.clone() else {
                continue;
            };
            let file = open_local_cleanup_anchor(physical_path)?;
            let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file,
                root: self.root.clone(),
                opaque_paths: Some(self.opaque_recovery_paths.clone()),
            });
            if !anchor.matches_local_path(physical_path)?
                || !anchor
                    .identity()
                    .is_ok_and(|actual| same_immutable_object(&identity, &actual))
            {
                return Err(SftpOpsError::Operation(format!(
                    "Transfer exchange candidate changed while resolving {}",
                    record.path.display()
                )));
            }
            let logical = self.persist_unresolved_physical_candidate_with_anchor(
                &format!("exchange-{}", candidate.role),
                physical_path,
                identity.clone(),
                anchor,
            )?;
            recovery_paths.push(logical);
        }
        self.release_exchange_record(&record.path)?;
        Ok(recovery_paths)
    }

    #[cfg(unix)]
    fn isolate_public_placeholder_into_private(
        &self,
        public_path: &Path,
        public_physical: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
        identity: &StableEntryIdentity,
        namespace: &DirectoryReservationNamespace,
    ) -> Result<(), SftpOpsError> {
        self.validate_directory_reservation_namespace(namespace)?;
        let private_physical = namespace
            .path
            .join(format!(".public-placeholder-{}", uuid::Uuid::new_v4()));
        let exchange_path = Self::unresolved_registry_path(
            "public-placeholder-isolation",
            &format!(
                "{}:{}",
                public_physical.display(),
                private_physical.display()
            ),
        );
        let record = PersistentExchangeRecord {
            path: exchange_path.clone(),
            first: PersistentExchangeCandidate {
                physical_path: public_physical.to_path_buf(),
                role: "public-placeholder".to_string(),
                identity: Some(identity.clone()),
            },
            second: PersistentExchangeCandidate {
                physical_path: private_physical.clone(),
                role: "expected-absence".to_string(),
                identity: None,
            },
            phase: PersistentExchangePhase::Prepared,
            generation: uuid::Uuid::new_v4().to_string(),
            legacy: false,
        };
        self.persist_exchange_record(record)?;
        if !anchor.matches_local_path(public_physical)? {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Public cleanup placeholder changed before private isolation at {}",
                    public_path.display()
                ),
                recovery_id: None,
                paths: vec![exchange_path],
                committed: false,
            });
        }
        let rename_error = rename_noreplace(public_physical, &private_physical).err();
        let isolated = anchor.matches_local_path(&private_physical)?
            && !public_physical.try_exists().unwrap_or(true);
        if !isolated {
            let restored = private_physical.exists()
                && !public_physical.exists()
                && rename_noreplace(&private_physical, public_physical).is_ok();
            let mut paths = self.persist_anchor_sibling_recovery(
                public_path,
                anchor,
                identity,
                "owned-public-placeholder",
            )?;
            paths.push(exchange_path);
            paths.sort();
            paths.dedup();
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Public cleanup placeholder isolation is indeterminate at {}{}{}",
                    public_path.display(),
                    rename_error
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default(),
                    if restored {
                        "; moved entry restored"
                    } else {
                        ""
                    }
                ),
                recovery_id: None,
                paths,
                committed: false,
            });
        }
        self.transition_exchange_phase(&exchange_path, PersistentExchangePhase::Applied)?;
        if identity.file_type == FileEntryType::File && anchor.link_count()? != Some(1) {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Public cleanup placeholder gained another hardlink before private deletion at {}",
                    public_path.display()
                ),
                recovery_id: None,
                paths: vec![exchange_path],
                committed: false,
            });
        }
        let namespace_directory = open_local_cleanup_anchor(&namespace.path)?;
        // The final unlink is permitted only after the object is in the
        // authenticated 0700 namespace. Same-UID tampering inside that
        // namespace is the explicit local-account trust boundary.
        unlink_from_anchored_directory(
            &namespace_directory,
            &namespace.path,
            private_physical.file_name().unwrap_or_default(),
            &anchor,
            identity,
            identity.file_type == FileEntryType::Directory,
        )?;
        #[cfg(test)]
        if let Some(hook) = &self.after_private_namespace_unlink {
            hook(&private_physical);
        }
        self.release_exchange_record(&exchange_path)
    }

    fn cleanup_isolation_placeholder(
        &self,
        path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
        identity: &StableEntryIdentity,
    ) -> Result<(), SftpOpsError> {
        let local = self.to_local(path)?;
        let tombstone = path.with_file_name(format!(
            ".{}.zaplex-delete-placeholder-{}",
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            uuid::Uuid::new_v4()
        ));
        self.map_opaque_cleanup_sibling(path, &tombstone)?;
        let local_tombstone = self.to_local(&tombstone)?;
        self.persist_artifact_intent(&tombstone)?;
        let placeholder_file = match identity.file_type {
            FileEntryType::File => open_confined_new_file(&self.root, &tombstone)?,
            FileEntryType::Directory => create_local_directory_with_anchor(
                &local_tombstone,
                #[cfg(test)]
                None,
            )
            .map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Creating isolation cleanup placeholder failed at {}: {}",
                    tombstone.display(),
                    error.source
                ))
            })?,
            FileEntryType::Symlink | FileEntryType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Unsupported isolation placeholder at {}",
                    path.display()
                )));
            }
        };
        let placeholder_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: placeholder_file,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        });
        let placeholder_identity = placeholder_anchor.identity()?;
        self.persist_artifact_identity(&tombstone, placeholder_anchor.clone())?;
        let public_exchange_path = Self::unresolved_registry_path(
            "public-cleanup-exchange",
            &format!("{}:{}", local.display(), local_tombstone.display()),
        );
        self.persist_exchange_record(PersistentExchangeRecord {
            path: public_exchange_path.clone(),
            first: PersistentExchangeCandidate {
                physical_path: local.clone(),
                role: "owned-cleanup-source".to_string(),
                identity: Some(identity.clone()),
            },
            second: PersistentExchangeCandidate {
                physical_path: local_tombstone.clone(),
                role: "public-cleanup-placeholder".to_string(),
                identity: Some(placeholder_identity.clone()),
            },
            phase: PersistentExchangePhase::Prepared,
            generation: uuid::Uuid::new_v4().to_string(),
            legacy: false,
        })?;
        if !anchor.matches_path(path)? {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation placeholder changed before tombstone isolation at {}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![path.to_path_buf(), tombstone],
                committed: false,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_placeholder_isolation {
            hook(&local);
        }
        let exchange_error = replace_atomic_local(&local, &local_tombstone).err();
        #[cfg(test)]
        if let Some(hook) = &self.after_placeholder_isolation_before_classification {
            hook(&local, &local_tombstone);
        }
        let isolated = anchor.matches_path(&tombstone)? && placeholder_anchor.matches_path(path)?;
        if !isolated {
            let restored = placeholder_anchor.matches_path(path)?
                && replace_atomic_local(&local, &local_tombstone).is_ok()
                && placeholder_anchor.matches_path(&tombstone)?;
            self.persist_unresolved_diagnostic(
                "isolation-placeholder-source",
                &path.display().to_string(),
            );
            self.persist_unresolved_diagnostic(
                "isolation-placeholder-tombstone",
                &tombstone.display().to_string(),
            );
            let mut recovery_paths = self.persist_anchor_sibling_recovery(
                path,
                anchor.clone(),
                identity,
                "owned-isolation-placeholder",
            )?;
            recovery_paths.extend(self.persist_anchor_sibling_recovery(
                &tombstone,
                placeholder_anchor,
                &placeholder_identity,
                "owned-isolation-sentinel",
            )?);
            recovery_paths.extend([path.to_path_buf(), tombstone.clone()]);
            recovery_paths.sort();
            recovery_paths.dedup();
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation placeholder exchange could not be safely resolved at {}{}{}",
                    path.display(),
                    exchange_error
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default(),
                    if restored { "; source restored" } else { "" }
                ),
                recovery_id: None,
                paths: recovery_paths,
                committed: false,
            });
        }
        self.transition_exchange_phase(&public_exchange_path, PersistentExchangePhase::Applied)?;
        self.transition_persistent_artifact_identity(
            &tombstone,
            "cleanup-tombstone",
            anchor.clone(),
        )?;
        #[cfg(unix)]
        let sentinel_cleanup = {
            let namespace = self.directory_reservation_namespace(&local)?;
            self.isolate_public_placeholder_into_private(
                path,
                &local,
                placeholder_anchor.clone(),
                &placeholder_identity,
                &namespace,
            )
        };
        #[cfg(not(unix))]
        let sentinel_cleanup = Err(SftpOpsError::Operation(
            "Anchor-relative isolation sentinel cleanup is unsupported".to_string(),
        ));
        if let Err(error) = sentinel_cleanup {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation sentinel cleanup failed after exchange at {}: {error}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![path.to_path_buf(), tombstone],
                committed: false,
            });
        }
        if !anchor.matches_path(&tombstone)? {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation placeholder changed before tombstone cleanup at {}",
                    tombstone.display()
                ),
                recovery_id: None,
                paths: vec![tombstone],
                committed: false,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_placeholder_tombstone_cleanup {
            hook(&local_tombstone);
        }
        if !anchor.matches_path(&tombstone)? {
            let mut recovery_paths = self.persist_anchor_sibling_recovery(
                &tombstone,
                anchor,
                identity,
                "owned-isolation-placeholder",
            )?;
            self.persist_unresolved_diagnostic(
                "isolation-placeholder-tombstone",
                &tombstone.display().to_string(),
            );
            recovery_paths.push(tombstone.clone());
            recovery_paths.sort();
            recovery_paths.dedup();
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation placeholder was replaced before tombstone cleanup at {}",
                    tombstone.display()
                ),
                recovery_id: None,
                paths: recovery_paths,
                committed: false,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.after_placeholder_final_check_before_delete {
            hook(&local_tombstone);
        }
        if !anchor.matches_path(&tombstone)? {
            let mut recovery_paths = self.persist_anchor_sibling_recovery(
                &tombstone,
                anchor,
                identity,
                "owned-isolation-placeholder",
            )?;
            recovery_paths.push(self.persist_unresolved_physical_candidate(
                "isolation-placeholder-final-swap",
                &local_tombstone,
                None,
            )?);
            recovery_paths.sort();
            recovery_paths.dedup();
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Isolation placeholder changed at the private cleanup boundary for {}",
                    tombstone.display()
                ),
                recovery_id: None,
                paths: recovery_paths,
                committed: false,
            });
        }
        let directory_entry = match identity.file_type {
            FileEntryType::File => false,
            FileEntryType::Directory => {
                if fs::read_dir(&local_tombstone)?.next().is_some() {
                    return Err(SftpOpsError::RecoveryRequired {
                        message: format!(
                            "Isolation placeholder directory is no longer empty at {}",
                            tombstone.display()
                        ),
                        recovery_id: None,
                        paths: vec![tombstone],
                        committed: false,
                    });
                }
                true
            }
            FileEntryType::Symlink | FileEntryType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Unsupported isolation placeholder at {}",
                    tombstone.display()
                )));
            }
        };
        #[cfg(not(unix))]
        return Err(SftpOpsError::RecoveryRequired {
            message: format!(
                "Anchor-relative placeholder cleanup is unsupported for {}",
                tombstone.display()
            ),
            recovery_id: None,
            paths: vec![tombstone],
            committed: false,
        });
        #[cfg(unix)]
        {
            let namespace = self.directory_reservation_namespace(&local_tombstone)?;
            self.validate_directory_reservation_namespace(&namespace)?;
            let namespace_directory = open_local_cleanup_anchor(&namespace.path)?;
            let private = namespace
                .path
                .join(format!(".placeholder-cleanup-{}", uuid::Uuid::new_v4()));
            let private_sentinel_file = match identity.file_type {
                FileEntryType::File => {
                    let mut options = fs::OpenOptions::new();
                    options.read(true).write(true).create_new(true);
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                    options.open(&private)?
                }
                FileEntryType::Directory => create_local_directory_with_anchor(
                    &private,
                    #[cfg(test)]
                    None,
                )
                .map_err(|error| error.source)?,
                FileEntryType::Symlink | FileEntryType::Other => unreachable!(),
            };
            let private_sentinel_anchor: Arc<dyn BackendOwnershipAnchor> =
                Arc::new(LocalOwnershipAnchor {
                    file: private_sentinel_file,
                    root: self.root.clone(),
                    opaque_paths: Some(self.opaque_recovery_paths.clone()),
                });
            let private_sentinel_identity = private_sentinel_anchor.identity()?;
            let recovery_path = self.persist_unresolved_physical_candidate(
                "isolation-placeholder-private",
                &private,
                Some(private_sentinel_identity.clone()),
            )?;
            self.opaque_recovery_paths
                .lock()
                .expect("opaque recovery path lock poisoned")
                .insert(recovery_path.clone(), private.clone());
            let private_exchange_path = Self::unresolved_registry_path(
                "private-cleanup-exchange",
                &format!("{}:{}", local_tombstone.display(), private.display()),
            );
            self.persist_exchange_record(PersistentExchangeRecord {
                path: private_exchange_path.clone(),
                first: PersistentExchangeCandidate {
                    physical_path: local_tombstone.clone(),
                    role: "owned-cleanup-tombstone".to_string(),
                    identity: Some(identity.clone()),
                },
                second: PersistentExchangeCandidate {
                    physical_path: private.clone(),
                    role: "private-cleanup-sentinel".to_string(),
                    identity: Some(private_sentinel_identity.clone()),
                },
                phase: PersistentExchangePhase::Prepared,
                generation: uuid::Uuid::new_v4().to_string(),
                legacy: false,
            })?;
            #[cfg(test)]
            if let Some(hook) = &self.before_private_placeholder_isolation {
                hook(&local_tombstone, &private);
            }
            let exchange_error = replace_atomic_local(&local_tombstone, &private).err();
            let isolated = anchor.matches_local_path(&private)?
                && private_sentinel_anchor.matches_local_path(&local_tombstone)?;
            if !isolated {
                let restored = private_sentinel_anchor.matches_local_path(&local_tombstone)?
                    && replace_atomic_local(&local_tombstone, &private).is_ok()
                    && private_sentinel_anchor.matches_local_path(&private)?;
                let mut paths = self.persist_anchor_sibling_recovery(
                    &tombstone,
                    anchor,
                    identity,
                    "owned-isolation-placeholder",
                )?;
                paths.extend([tombstone.clone(), recovery_path]);
                paths.sort();
                paths.dedup();
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Isolation placeholder private cleanup exchange is indeterminate for {}{}{}",
                        tombstone.display(),
                        exchange_error
                            .map(|error| format!(": {error}"))
                            .unwrap_or_default(),
                        if restored { "; replacement restored" } else { "" }
                    ),
                    recovery_id: None,
                    paths,
                    committed: false,
                });
            }
            self.transition_exchange_phase(
                &private_exchange_path,
                PersistentExchangePhase::Applied,
            )?;
            {
                let _lifecycle = self
                    .artifact_lifecycle
                    .lock()
                    .expect("artifact lifecycle lock poisoned");
                let current_record = self
                    .persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned")
                    .get(&recovery_path)
                    .cloned()
                    .ok_or_else(|| {
                        SftpOpsError::Operation(format!(
                            "Private cleanup recovery record is missing at {}",
                            recovery_path.display()
                        ))
                    })?;
                let next_record = current_record.transition(
                    Some(private.clone()),
                    "cleanup-private-tombstone".to_string(),
                    Some(identity.clone()),
                );
                let registry = self
                    .directory_reservation_registry
                    .as_ref()
                    .ok_or_else(|| {
                        SftpOpsError::Operation(
                            "Trusted transfer artifact registry is unavailable".to_string(),
                        )
                    })?;
                registry.transition_artifact_record(&current_record, &next_record)?;
                self.persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned")
                    .insert(recovery_path.clone(), next_record);
            }
            if !anchor.matches_local_path(&private)?
                || !anchor
                    .identity()
                    .is_ok_and(|actual| same_immutable_object(identity, &actual))
            {
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Isolation placeholder private identity changed for {}",
                        tombstone.display()
                    ),
                    recovery_id: None,
                    paths: vec![recovery_path],
                    committed: false,
                });
            }
            let private_file = open_local_cleanup_anchor(&private)?;
            let private_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file: private_file,
                root: self.root.clone(),
                opaque_paths: Some(self.opaque_recovery_paths.clone()),
            });
            if !private_anchor.matches_path(&recovery_path)?
                || !private_anchor
                    .identity()
                    .is_ok_and(|actual| same_immutable_object(identity, &actual))
            {
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Isolation placeholder private recovery anchor changed for {}",
                        tombstone.display()
                    ),
                    recovery_id: None,
                    paths: vec![recovery_path],
                    committed: false,
                });
            }
            self.cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .insert(
                    recovery_path.clone(),
                    (identity.clone(), private_anchor.clone()),
                );
            self.isolate_public_placeholder_into_private(
                &tombstone,
                &local_tombstone,
                private_sentinel_anchor,
                &private_sentinel_identity,
                &namespace,
            )?;
            #[cfg(test)]
            if let Some(hook) = &self.before_private_placeholder_unlink {
                hook(&namespace.path, &private);
            }
            unlink_from_anchored_directory(
                &namespace_directory,
                &namespace.path,
                private.file_name().unwrap_or_default(),
                &private_anchor,
                identity,
                directory_entry,
            )?;
            #[cfg(test)]
            if let Some(hook) = &self.after_private_namespace_unlink {
                hook(&private);
            }
            self.release_exchange_record(&private_exchange_path)?;
            self.release_cleanup_recovery_path(&recovery_path)?;
            if !namespace.anchor.matches_path(&namespace.path)? {
                let diagnostic = self.persist_unresolved_physical_candidate(
                    "private-cleanup-namespace",
                    &namespace.path,
                    None,
                )?;
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Private cleanup namespace changed while retiring {}",
                        tombstone.display()
                    ),
                    recovery_id: None,
                    paths: vec![diagnostic],
                    committed: false,
                });
            }
        }
        self.release_exchange_record(&public_exchange_path)?;
        self.release_cleanup_recovery_path(&tombstone)?;
        self.release_persistent_artifact(path)
    }

    fn persist_anchor_sibling_recovery(
        &self,
        path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
        identity: &StableEntryIdentity,
        role: &str,
    ) -> Result<Vec<PathBuf>, SftpOpsError> {
        let local = match self.to_local(path) {
            Ok(local) => local,
            Err(error) => {
                return Ok(vec![self.persist_unresolved_diagnostic(
                    "anchor-sibling-path",
                    &format!("{}:{error}", path.display()),
                )]);
            }
        };
        let Some(parent) = local.parent() else {
            return Ok(vec![self.persist_unresolved_diagnostic(
                "anchor-sibling-parent",
                &path.display().to_string(),
            )]);
        };
        for _attempt in 0..3 {
            let candidates = match self.scan_anchor_sibling_paths(parent, &anchor) {
                Ok(candidates) => candidates,
                Err(error) => {
                    return Ok(vec![self.persist_sibling_rescan_record(
                        parent,
                        identity,
                        role,
                        &error.to_string(),
                    )?]);
                }
            };
            match candidates.as_slice() {
                [] => {
                    return Ok(vec![self.persist_sibling_rescan_record(
                        parent,
                        identity,
                        role,
                        "anchor is not currently visible below the expected parent",
                    )?]);
                }
                [(candidate, physical)] => {
                    if identity.file_type == FileEntryType::File
                        && anchor.link_count().ok().flatten() != Some(1)
                    {
                        return Ok(vec![
                            self.persist_unresolved_physical_candidate(
                                "anchor-sibling-hardlink",
                                physical,
                                None,
                            )?,
                            self.persist_unresolved_diagnostic(
                                "anchor-sibling-alias",
                                &candidate.display().to_string(),
                            ),
                        ]);
                    }
                    #[cfg(test)]
                    if _attempt == 0 {
                        if let Some(hook) = &self.before_sibling_registry_commit {
                            hook(candidate, physical);
                        }
                    }
                    if !anchor.matches_path(candidate).unwrap_or(false) {
                        continue;
                    }
                    #[cfg(test)]
                    if self
                        .sibling_recovery_failure
                        .lock()
                        .expect("sibling recovery failure lock poisoned")
                        .as_ref()
                        == Some(&SiblingRecoveryFailure::RegistryWrite)
                    {
                        return Ok(vec![self.persist_sibling_rescan_record(
                            parent,
                            identity,
                            role,
                            "registry write failed",
                        )?]);
                    }
                    let record = PersistentArtifactRecord::active(
                        candidate.clone(),
                        Some(physical.clone()),
                        role.to_string(),
                        Some(identity.clone()),
                    );
                    let Some(registry) = &self.directory_reservation_registry else {
                        return Ok(vec![self.persist_sibling_rescan_record(
                            parent,
                            identity,
                            role,
                            "registry is unavailable",
                        )?]);
                    };
                    let _lifecycle = self
                        .artifact_lifecycle
                        .lock()
                        .expect("artifact lifecycle lock poisoned");
                    registry.write_artifact_record(&record)?;
                    #[cfg(test)]
                    if let Some(hook) = &self.after_sibling_registry_write {
                        hook(candidate, physical);
                    }
                    if !anchor.matches_path(candidate).unwrap_or(false) {
                        let rescan = record.transition(
                            Some(parent.to_path_buf()),
                            format!("rescan-anchor-sibling:{role}"),
                            Some(identity.clone()),
                        );
                        registry.transition_artifact_record(&record, &rescan)?;
                        self.persistent_artifact_records
                            .lock()
                            .expect("persistent transfer artifact lock poisoned")
                            .insert(candidate.clone(), rescan);
                        self.startup_unresolved_paths
                            .lock()
                            .expect("startup unresolved path lock poisoned")
                            .insert(candidate.clone());
                        #[cfg(test)]
                        if let Some(hook) = &self.before_sibling_rescan_iteration {
                            hook(candidate);
                        }
                        return Ok(vec![candidate.clone()]);
                    }
                    self.persistent_artifact_records
                        .lock()
                        .expect("persistent transfer artifact lock poisoned")
                        .insert(candidate.clone(), record);
                    self.cleanup_recovery_identities
                        .lock()
                        .expect("cleanup recovery identity lock poisoned")
                        .insert(candidate.clone(), (identity.clone(), anchor.clone()));
                    return Ok(vec![candidate.clone()]);
                }
                many => {
                    let mut paths = Vec::new();
                    for (candidate, physical) in many {
                        self.persist_unresolved_physical_candidate(
                            "anchor-sibling-ambiguous",
                            physical,
                            None,
                        )?;
                        paths.push(self.persist_unresolved_diagnostic(
                            "anchor-sibling-alias",
                            &candidate.display().to_string(),
                        ));
                    }
                    return Ok(paths);
                }
            }
        }
        Ok(vec![self.persist_sibling_rescan_record(
            parent,
            identity,
            role,
            "anchor moved repeatedly while committing recovery",
        )?])
    }

    fn scan_anchor_sibling_paths(
        &self,
        parent: &Path,
        anchor: &Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<Vec<(PathBuf, PathBuf)>, SftpOpsError> {
        #[cfg(test)]
        if self
            .sibling_recovery_failure
            .lock()
            .expect("sibling recovery failure lock poisoned")
            .as_ref()
            == Some(&SiblingRecoveryFailure::ReadDirectory)
        {
            return Err(SftpOpsError::Operation(
                "injected sibling read-directory failure".to_string(),
            ));
        }
        let entries = fs::read_dir(parent)?;
        let mut candidates = Vec::new();
        for entry in entries {
            #[cfg(test)]
            if self
                .sibling_recovery_failure
                .lock()
                .expect("sibling recovery failure lock poisoned")
                .as_ref()
                == Some(&SiblingRecoveryFailure::DirectoryEntry)
            {
                return Err(SftpOpsError::Operation(
                    "injected sibling directory-entry failure".to_string(),
                ));
            }
            let entry = entry?;
            let candidate = self.to_remote(&entry.path());
            #[cfg(test)]
            if self
                .sibling_recovery_failure
                .lock()
                .expect("sibling recovery failure lock poisoned")
                .as_ref()
                == Some(&SiblingRecoveryFailure::AnchorProbe)
            {
                return Err(SftpOpsError::Operation(
                    "injected sibling anchor-probe failure".to_string(),
                ));
            }
            if anchor.matches_path(&candidate)? {
                candidates.push((candidate, entry.path()));
            }
        }
        Ok(candidates)
    }

    fn persist_sibling_rescan_record(
        &self,
        parent: &Path,
        identity: &StableEntryIdentity,
        role: &str,
        detail: &str,
    ) -> Result<PathBuf, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let logical = Self::unresolved_registry_path(
            "anchor-sibling-rescan",
            &format!("{}:{}:{detail}", parent.display(), identity.object_id),
        );
        let record = PersistentArtifactRecord::active(
            logical.clone(),
            Some(parent.to_path_buf()),
            format!("rescan-anchor-sibling:{role}"),
            Some(identity.clone()),
        );
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        registry.write_artifact_record(&record)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(logical.clone(), record);
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .insert(logical.clone());
        Ok(logical)
    }

    fn prepare_exchange_artifact_record(
        &self,
        path: &Path,
        displaced_anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<Option<(PersistentArtifactRecord, PersistentArtifactRecord)>, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let Some(current) = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned()
        else {
            return Ok(None);
        };
        let next = current.transition(
            current.physical_path.clone(),
            "exchange-displaced-target".to_string(),
            Some(displaced_anchor.identity()?),
        );
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        registry.transition_artifact_record(&current, &next)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), next.clone());
        #[cfg(test)]
        if let Some(hook) = &self.at_artifact_association_cutpoint {
            hook(self, path);
        }
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .insert(
                path.to_path_buf(),
                (
                    next.identity
                        .clone()
                        .expect("exchange transition must preserve displaced identity"),
                    displaced_anchor,
                ),
            );
        Ok(Some((current, next)))
    }

    fn restore_exchange_artifact_record(
        &self,
        path: &Path,
        transitioned: &(PersistentArtifactRecord, PersistentArtifactRecord),
        original_anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        registry.transition_artifact_record(&transitioned.1, &transitioned.0)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(path.to_path_buf(), transitioned.0.clone());
        if let Some(identity) = transitioned.0.identity.clone() {
            self.cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .insert(path.to_path_buf(), (identity, original_anchor));
        }
        Ok(())
    }

    fn release_persistent_artifact(&self, path: &Path) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        self.release_persistent_artifact_locked(path, true)
    }

    fn release_persistent_artifact_locked(
        &self,
        path: &Path,
        clear_auxiliary: bool,
    ) -> Result<(), SftpOpsError> {
        let expected = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned();
        let retired_anchor = self
            .cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .get(path)
            .map(|(_, anchor)| anchor.clone());
        let mut clear_auxiliary_state = expected.is_none();
        if let Some(expected) = expected {
            let registry = self
                .directory_reservation_registry
                .as_ref()
                .ok_or_else(|| {
                    SftpOpsError::Operation(
                        "Trusted transfer artifact registry is unavailable".to_string(),
                    )
                })?;
            registry.remove_artifact_record_if_matches(&expected)?;
            let mut records = self
                .persistent_artifact_records
                .lock()
                .expect("persistent transfer artifact lock poisoned");
            if records
                .get(path)
                .is_some_and(|current| same_persistent_artifact_record(current, &expected))
            {
                records.remove(path);
                clear_auxiliary_state = true;
            }
            drop(records);
        }
        if clear_auxiliary && clear_auxiliary_state {
            if let Some(retired_anchor) = retired_anchor {
                let mut identities = self
                    .cleanup_recovery_identities
                    .lock()
                    .expect("cleanup recovery identity lock poisoned");
                if identities
                    .get(path)
                    .is_some_and(|(_, current)| Arc::ptr_eq(current, &retired_anchor))
                {
                    identities.remove(path);
                }
            }
            self.startup_unresolved_paths
                .lock()
                .expect("startup unresolved path lock poisoned")
                .remove(path);
        }
        Ok(())
    }

    fn persistent_artifact_generation(&self, path: &Path) -> Option<String> {
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .map(|record| record.generation.clone())
    }

    fn persist_exchange_record(
        &self,
        record: PersistentExchangeRecord,
    ) -> Result<(), SftpOpsError> {
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer exchange registry is unavailable".to_string(),
                )
            })?;
        registry.write_exchange_record(&record)?;
        self.persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned")
            .insert(record.path.clone(), record);
        Ok(())
    }

    fn transition_exchange_phase(
        &self,
        path: &Path,
        phase: PersistentExchangePhase,
    ) -> Result<(), SftpOpsError> {
        let current = self
            .persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned")
            .get(path)
            .cloned()
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Transfer exchange record is missing for {}",
                    path.display()
                ))
            })?;
        let mut next = current.clone();
        next.phase = phase;
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer exchange registry is unavailable".to_string(),
                )
            })?;
        registry.transition_exchange_record(&current, &next)?;
        self.persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned")
            .insert(path.to_path_buf(), next);
        Ok(())
    }

    fn release_exchange_record(&self, path: &Path) -> Result<(), SftpOpsError> {
        let expected = self
            .persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned")
            .get(path)
            .cloned();
        let Some(expected) = expected else {
            return Ok(());
        };
        self.directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer exchange registry is unavailable".to_string(),
                )
            })?
            .remove_exchange_record_if_matches(&expected)?;
        let mut records = self
            .persistent_exchange_records
            .lock()
            .expect("persistent transfer exchange lock poisoned");
        if records
            .get(path)
            .is_some_and(|current| same_persistent_exchange_record(current, &expected))
        {
            records.remove(path);
        }
        drop(records);
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .remove(path);
        Ok(())
    }

    fn recover_moved_artifact_record(
        &self,
        registry: &DirectoryReservationRegistry,
        record: &PersistentArtifactRecord,
    ) -> Result<bool, SftpOpsError> {
        let (Some(expected), Some(previous_physical)) =
            (record.identity.as_ref(), record.physical_path.as_ref())
        else {
            return Ok(false);
        };
        let Some(parent) = previous_physical.parent() else {
            return Ok(false);
        };
        let replacement = record.transition(
            Some(parent.to_path_buf()),
            format!("rescan-anchor-sibling:{}", record.role),
            Some(expected.clone()),
        );
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        registry.transition_artifact_record(record, &replacement)?;
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(record.path.clone(), replacement);
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .insert(record.path.clone());
        Ok(true)
    }

    fn discover_persistent_artifacts(&self) {
        let Some(registry) = &self.directory_reservation_registry else {
            return;
        };
        match registry.exchange_records() {
            Ok(records) => {
                for (record_path, record) in records {
                    match record {
                        Ok(record) => {
                            let path = record.path.clone();
                            self.persistent_exchange_records
                                .lock()
                                .expect("persistent transfer exchange lock poisoned")
                                .insert(path.clone(), record);
                            self.startup_unresolved_paths
                                .lock()
                                .expect("startup unresolved path lock poisoned")
                                .insert(path);
                        }
                        Err(error) => {
                            self.startup_unresolved_paths
                                .lock()
                                .expect("startup unresolved path lock poisoned")
                                .insert(Self::unresolved_registry_path(
                                    "exchange-record",
                                    &format!("{}:{error}", record_path.display()),
                                ));
                        }
                    }
                }
            }
            Err(error) => {
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(Self::unresolved_registry_path(
                        "exchange-enumeration",
                        &error.to_string(),
                    ));
            }
        }
        let records = match registry.artifact_records() {
            Ok(records) => records,
            Err(error) => {
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(Self::unresolved_registry_path(
                        "artifact-enumeration",
                        &error.to_string(),
                    ));
                return;
            }
        };
        for (record_path, record) in records {
            let record = match record {
                Ok(record) => record,
                Err(error) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "artifact-record",
                            &format!("{}:{error}", record_path.display()),
                        ));
                    continue;
                }
            };
            if record.retired {
                continue;
            }
            let path = record.path.clone();
            if record.role.starts_with("rescan-anchor-sibling:") {
                self.persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned")
                    .insert(path.clone(), record);
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(path);
                continue;
            }
            if record.role.starts_with("unresolved-") {
                if let (Some(physical_path), Some(expected)) =
                    (record.physical_path.as_ref(), record.identity.as_ref())
                {
                    let trusted_physical_path = physical_path.starts_with(&self.root)
                        || self
                            .directory_reservation_namespaces
                            .lock()
                            .expect("directory reservation namespace lock poisoned")
                            .values()
                            .any(|namespace| physical_path.starts_with(&namespace.path));
                    if trusted_physical_path {
                        self.opaque_recovery_paths
                            .lock()
                            .expect("opaque recovery path lock poisoned")
                            .insert(path.clone(), physical_path.clone());
                        if let Ok(file) = open_local_cleanup_anchor(physical_path) {
                            let anchor: Arc<dyn BackendOwnershipAnchor> =
                                Arc::new(LocalOwnershipAnchor {
                                    file,
                                    root: self.root.clone(),
                                    opaque_paths: Some(self.opaque_recovery_paths.clone()),
                                });
                            if anchor.matches_path(&path).unwrap_or(false)
                                && anchor
                                    .identity()
                                    .is_ok_and(|actual| same_immutable_object(expected, &actual))
                            {
                                self.cleanup_recovery_identities
                                    .lock()
                                    .expect("cleanup recovery identity lock poisoned")
                                    .insert(path.clone(), (expected.clone(), anchor));
                            }
                        }
                    }
                }
                self.persistent_artifact_records
                    .lock()
                    .expect("persistent transfer artifact lock poisoned")
                    .insert(path.clone(), record);
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(path);
                continue;
            }
            let local = if let Some(physical) = record.physical_path.clone() {
                let trusted = physical.starts_with(&self.root)
                    || self
                        .directory_reservation_namespaces
                        .lock()
                        .expect("directory reservation namespace lock poisoned")
                        .values()
                        .any(|namespace| physical.starts_with(&namespace.path));
                if !trusted {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "artifact-physical-path",
                            &physical.display().to_string(),
                        ));
                    continue;
                }
                self.opaque_recovery_paths
                    .lock()
                    .expect("opaque recovery path lock poisoned")
                    .insert(path.clone(), physical.clone());
                physical
            } else {
                match self.to_local(&path) {
                    Ok(local) => local,
                    Err(error) => {
                        self.startup_unresolved_paths
                            .lock()
                            .expect("startup unresolved path lock poisoned")
                            .insert(Self::unresolved_registry_path(
                                "artifact-path",
                                &format!("{}:{error}", path.display()),
                            ));
                        continue;
                    }
                }
            };
            match fs::symlink_metadata(&local) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if self.applied_deleted_exchange_contains(&local) {
                        self.persistent_artifact_records
                            .lock()
                            .expect("persistent transfer artifact lock poisoned")
                            .insert(path.clone(), record);
                        self.startup_unresolved_paths
                            .lock()
                            .expect("startup unresolved path lock poisoned")
                            .insert(path);
                        continue;
                    }
                    match self.recover_moved_artifact_record(registry, &record) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(recovery_error) => {
                            self.startup_unresolved_paths
                                .lock()
                                .expect("startup unresolved path lock poisoned")
                                .insert(Self::unresolved_registry_path(
                                    "artifact-rescan",
                                    &format!("{}:{recovery_error}", path.display()),
                                ));
                        }
                    }
                    self.persistent_artifact_records
                        .lock()
                        .expect("persistent transfer artifact lock poisoned")
                        .insert(path.clone(), record);
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(path);
                    continue;
                }
                Err(error) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "artifact-probe",
                            &format!("{}:{error}", path.display()),
                        ));
                    continue;
                }
            }
            self.persistent_artifact_records
                .lock()
                .expect("persistent transfer artifact lock poisoned")
                .insert(path.clone(), record.clone());
            let Some(ref expected) = record.identity else {
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(path);
                continue;
            };
            let file = match open_local_cleanup_anchor(&local) {
                Ok(file) => file,
                Err(_) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(path);
                    continue;
                }
            };
            let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file,
                root: self.root.clone(),
                opaque_paths: Some(self.opaque_recovery_paths.clone()),
            });
            if anchor.matches_path(&path).unwrap_or(false)
                && anchor
                    .identity()
                    .is_ok_and(|actual| same_immutable_object(&expected, &actual))
            {
                self.cleanup_recovery_identities
                    .lock()
                    .expect("cleanup recovery identity lock poisoned")
                    .insert(path, (expected.clone(), anchor));
            } else {
                match self.recover_moved_artifact_record(registry, &record) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(recovery_error) => {
                        self.startup_unresolved_paths
                            .lock()
                            .expect("startup unresolved path lock poisoned")
                            .insert(Self::unresolved_registry_path(
                                "artifact-rescan",
                                &format!("{}:{recovery_error}", path.display()),
                            ));
                    }
                }
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(path);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn directory_reservation_namespace_path_for_test(&self) -> Option<PathBuf> {
        self.directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .values()
            .next()
            .map(|namespace| namespace.path.clone())
    }

    #[cfg(test)]
    pub(crate) fn with_lstat_error(mut self, path: PathBuf) -> Self {
        self.forced_lstat_error = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_staged_identity_failure(mut self) -> Self {
        self.fail_staged_identity = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_published_identity_failure(mut self, path: PathBuf) -> Self {
        self.fail_published_identity = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_delete_after_apply_failure(mut self, path: PathBuf) -> Self {
        self.fail_delete_after_apply = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_delete_failure_matching(mut self, marker: impl Into<String>) -> Self {
        self.fail_delete_matching = Some(marker.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn with_delete_failure_matching_once(mut self, marker: impl Into<String>) -> Self {
        self.fail_delete_matching = Some(marker.into());
        self.fail_delete_matching_once = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_replace_after_apply_failure(mut self, path: PathBuf) -> Self {
        self.fail_replace_after_apply = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_replace(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_replace = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_replace(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_replace = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_rename_after_apply_failure(mut self, path: PathBuf) -> Self {
        self.fail_rename_after_apply = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_writer_failure_on_create(mut self, create_number: u64) -> Self {
        self.fail_writer_on_create = Some(create_number);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_writer_create_after_apply_failure(mut self, create_number: u64) -> Self {
        self.fail_writer_create_after_apply = Some(create_number);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_directory_create_after_apply_failure(mut self, create_number: u64) -> Self {
        self.fail_directory_create_after_apply = Some(create_number);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_corrupt_writer_on_create(mut self, create_number: u64) -> Self {
        self.corrupt_writer_on_create = Some(create_number);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_ignored_noreplace_probe_semantics(mut self) -> Self {
        self.ignore_noreplace_probe_semantics = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_cleanup_failure(mut self) -> Self {
        self.fail_preflight_cleanup = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_collision(mut self, suffix: char) -> Self {
        self.preflight_collision_suffix = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_collision(&self) -> Option<(PathBuf, u64, u64)> {
        self.preflight_collision
            .lock()
            .expect("preflight collision lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_rename_copy_unlink(mut self) -> Self {
        self.preflight_rename_copy_unlink = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_exchange_content_swap(mut self) -> Self {
        self.preflight_exchange_content_swap = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_source_replacement_before_rename(mut self) -> Self {
        self.replace_preflight_source_before_rename = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_source_replacement_before_exchange(mut self) -> Self {
        self.replace_preflight_source_before_exchange = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_source_replacements_before_reject(mut self) -> Self {
        self.replace_preflight_sources_before_reject = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_reject_replacements(&self) -> Vec<PathBuf> {
        self.preflight_reject_replacements
            .lock()
            .expect("preflight reject replacements lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn preflight_mutation_replacement(&self) -> Option<PathBuf> {
        self.preflight_mutation_replacement
            .lock()
            .expect("preflight mutation replacement lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_rename_after_apply_failure(mut self) -> Self {
        self.fail_preflight_rename_after_apply = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_create_after_apply_failure(mut self, suffix: char) -> Self {
        self.fail_preflight_create_after_apply = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_uncertain_create(&self) -> Option<PathBuf> {
        self.preflight_uncertain_create
            .lock()
            .expect("preflight uncertain create lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_cleanup_replacement(mut self, suffix: char) -> Self {
        self.replace_preflight_owned_before_cleanup = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_cleanup_replacement(&self) -> Option<(PathBuf, u64, u64)> {
        self.preflight_cleanup_replacement
            .lock()
            .expect("preflight cleanup replacement lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_cleanup_replacement_after_check(mut self, suffix: char) -> Self {
        self.replace_preflight_owned_after_check = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_cleanup_replacement_after_check(&self) -> Option<(PathBuf, u64, u64)> {
        self.preflight_cleanup_replacement_after_check
            .lock()
            .expect("preflight cleanup replacement lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_cleanup_anchor_observation(mut self, suffix: char) -> Self {
        self.observe_preflight_cleanup_anchor = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_cleanup_anchor_observed(&self) -> bool {
        self.preflight_cleanup_anchor_observed
            .load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn cleanup_recovery_identity_count(&self) -> usize {
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .len()
    }

    #[cfg(test)]
    pub(crate) fn with_preflight_inode_reuse_attempt(mut self, suffix: char) -> Self {
        self.force_preflight_inode_reuse = Some(suffix);
        self
    }

    #[cfg(test)]
    pub(crate) fn preflight_inode_reuse_observation(
        &self,
    ) -> Option<(PathBuf, u64, u64, bool, i64, i64)> {
        self.preflight_inode_reuse_observation
            .lock()
            .expect("preflight inode reuse observation lock poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn writer_create_count(&self) -> u64 {
        self.writer_creates.load(Ordering::SeqCst)
    }

    #[cfg(all(test, unix))]
    fn inject_preflight_collision(&self, path: &Path, suffix: char) -> Result<(), std::io::Error> {
        if self.preflight_collision_suffix != Some(suffix) {
            return Ok(());
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(format!("foreign-{suffix}").as_bytes())?;
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        *self
            .preflight_collision
            .lock()
            .expect("preflight collision lock poisoned") =
            Some((path.to_path_buf(), metadata.dev(), metadata.ino()));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn with_partial_recursive_delete_failure(mut self) -> Self {
        self.fail_recursive_delete_partially = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_stable_identity(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_stable_identity = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_guarded_delete(
        mut self,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.before_guarded_delete = Some(Arc::new(hook));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_isolated_delete_failure(mut self) -> Self {
        self.fail_isolated_delete_before_apply = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_directory_marker_cleanup_failure(mut self) -> Self {
        self.fail_directory_marker_cleanup = true;
        self
    }

    /// Maps a backend path to a local absolute path without permitting lexical
    /// or symlink traversal outside the configured root.
    fn to_local(&self, remote_path: &Path) -> Result<PathBuf, SftpOpsError> {
        if let Some(local) = self
            .opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .get(remote_path)
        {
            return Ok(local.clone());
        }
        let mut relative = PathBuf::new();
        for component in remote_path.components() {
            match component {
                std::path::Component::RootDir | std::path::Component::CurDir => {}
                std::path::Component::Normal(component) => relative.push(component),
                std::path::Component::ParentDir | std::path::Component::Prefix(_) => {
                    return Err(SftpOpsError::Operation(format!(
                        "Path escapes the backend root: {}",
                        remote_path.display()
                    )));
                }
            }
        }
        let local = self.root.join(relative);
        if self
            .reserved_directory_namespace_paths
            .lock()
            .expect("reserved directory namespace lock poisoned")
            .keys()
            .any(|namespace| local.starts_with(namespace))
        {
            return Err(SftpOpsError::Operation(format!(
                "Path enters a reserved transfer namespace: {}",
                remote_path.display()
            )));
        }
        if self
            .directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .values()
            .any(|namespace| local.starts_with(&namespace.path))
        {
            return Err(SftpOpsError::Operation(format!(
                "Path enters a protected transfer namespace: {}",
                remote_path.display()
            )));
        }
        let canonical_root = dunce::canonicalize(&self.root).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Resolving backend root failed at {}: {error}",
                self.root.display()
            ))
        })?;
        let mut existing = if local == self.root {
            local.as_path()
        } else {
            local.parent().unwrap_or(local.as_path())
        };
        while fs::symlink_metadata(existing).is_err() {
            existing = existing.parent().ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Path has no existing backend ancestor: {}",
                    remote_path.display()
                ))
            })?;
        }
        let canonical_existing = dunce::canonicalize(existing).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Resolving backend path failed at {}: {error}",
                remote_path.display()
            ))
        })?;
        if !canonical_existing.starts_with(&canonical_root) {
            return Err(SftpOpsError::Operation(format!(
                "Path resolves outside the backend root: {}",
                remote_path.display()
            )));
        }
        Ok(local)
    }

    fn validate_resolved_local_path(
        &self,
        local: &Path,
        backend_path: &Path,
    ) -> Result<(), SftpOpsError> {
        let opaque_local = self
            .opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .get(backend_path)
            .cloned();
        if opaque_local.as_deref() == Some(local) {
            let namespace = self
                .directory_reservation_namespaces
                .lock()
                .expect("directory reservation namespace lock poisoned")
                .values()
                .find(|namespace| local.starts_with(&namespace.path))
                .cloned()
                .ok_or_else(|| {
                    SftpOpsError::Operation(format!(
                        "Opaque recovery path has no protected namespace: {}",
                        backend_path.display()
                    ))
                })?;
            self.validate_directory_reservation_namespace(&namespace)?;
            let canonical_namespace = dunce::canonicalize(&namespace.path)?;
            let canonical_local = dunce::canonicalize(local).map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Resolving opaque recovery path failed at {}: {error}",
                    backend_path.display()
                ))
            })?;
            if canonical_local.starts_with(canonical_namespace) {
                return Ok(());
            }
            return Err(SftpOpsError::Operation(format!(
                "Opaque recovery path resolves outside its protected namespace: {}",
                backend_path.display()
            )));
        }
        let canonical_root = dunce::canonicalize(&self.root)?;
        let canonical_local = dunce::canonicalize(local).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Resolving backend path failed at {}: {error}",
                backend_path.display()
            ))
        })?;
        if !canonical_local.starts_with(canonical_root) {
            return Err(SftpOpsError::Operation(format!(
                "Path resolves outside the backend root: {}",
                backend_path.display()
            )));
        }
        Ok(())
    }

    fn open_confined_file(&self, path: &Path) -> Result<fs::File, SftpOpsError> {
        let local = self.to_local(path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&local).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Opening confined file failed at {}: {error}",
                path.display()
            ))
        })?;
        self.validate_resolved_local_path(&local, path)?;
        let opened = stable_identity_from_local_metadata(&file.metadata()?);
        let named = stable_identity_from_local_metadata(&fs::symlink_metadata(&local)?);
        if opened.file_type != FileEntryType::File || !same_immutable_object(&opened, &named) {
            return Err(SftpOpsError::Operation(format!(
                "Confined file changed while opening {}",
                path.display()
            )));
        }
        Ok(file)
    }

    /// Converts a local path to a "remote" path.
    fn to_remote(&self, local_path: &Path) -> PathBuf {
        match local_path.strip_prefix(&self.root) {
            Ok(rel) => {
                if rel.as_os_str().is_empty() {
                    PathBuf::from("/")
                } else {
                    PathBuf::from("/").join(rel)
                }
            }
            Err(_) => PathBuf::from("/").join(local_path),
        }
    }

    fn register_directory_reservation_recovery(
        &self,
        private: &Path,
        marker: Option<OwnedReservationMarker>,
        identity: Option<StableEntryIdentity>,
        anchor: Option<Arc<dyn BackendOwnershipAnchor>>,
    ) -> Result<PathBuf, SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let recovery_path = PathBuf::from("/.zaplex-opaque-directory-reservation")
            .join(private.file_name().unwrap_or_default());
        let role = "unresolved-directory-reservation".to_string();
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted transfer artifact registry is unavailable".to_string(),
                )
            })?;
        let mut current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(&recovery_path)
            .cloned();
        if current.is_none() {
            let record_path = registry.artifact_record_path(&recovery_path);
            if DirectoryReservationRegistry::probe_path(&record_path)?.is_some() {
                current = Some(registry.read_artifact_record(&record_path)?);
            }
        }
        let record = match current {
            Some(current)
                if !current.retired
                    && current.physical_path.as_deref() == Some(private)
                    && current.role == role
                    && same_optional_identity(current.identity.as_ref(), identity.as_ref()) =>
            {
                current
            }
            Some(current)
                if !current.retired
                    && current.physical_path.as_deref() == Some(private)
                    && identity.is_none() =>
            {
                let next = current.transition(Some(private.to_path_buf()), role, None);
                registry.transition_artifact_record(&current, &next)?;
                next
            }
            Some(_) | None => {
                let record = PersistentArtifactRecord::active(
                    recovery_path.clone(),
                    Some(private.to_path_buf()),
                    role,
                    identity.clone(),
                );
                registry.write_artifact_record(&record)?;
                record
            }
        };
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(recovery_path.clone(), record);
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .insert(recovery_path.clone());
        #[cfg(test)]
        if let Some(hook) = &self.at_artifact_association_cutpoint {
            hook(self, &recovery_path);
        }
        self.opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .insert(recovery_path.clone(), private.to_path_buf());
        #[cfg(test)]
        if let Some(hook) = &self.at_artifact_association_cutpoint {
            hook(self, &recovery_path);
        }
        if let Some(marker) = marker {
            self.opaque_recovery_markers
                .lock()
                .expect("opaque recovery marker lock poisoned")
                .insert(recovery_path.clone(), marker);
        }
        if let (Some(identity), Some(anchor)) = (identity, anchor) {
            self.cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .insert(recovery_path.clone(), (identity, anchor));
        }
        Ok(recovery_path)
    }

    fn register_failed_directory_reservation_candidates(
        &self,
        private: &Path,
        reserved_identity: Option<&StableEntryIdentity>,
    ) -> Result<Vec<PathBuf>, SftpOpsError> {
        let mut candidates = Vec::<(PathBuf, bool)>::new();
        if let Ok(metadata) = fs::symlink_metadata(private) {
            let current = stable_identity_from_local_metadata(&metadata);
            let owned =
                reserved_identity.is_some_and(|expected| same_immutable_object(expected, &current));
            candidates.push((private.to_path_buf(), owned));
        }
        if let (Some(parent), Some(reserved_identity)) = (private.parent(), reserved_identity) {
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let candidate = entry.path();
                    let matches_reserved = fs::symlink_metadata(&candidate)
                        .map(|metadata| stable_identity_from_local_metadata(&metadata))
                        .is_ok_and(|identity| same_immutable_object(reserved_identity, &identity));
                    if matches_reserved && !candidates.iter().any(|(path, _)| path == &candidate) {
                        candidates.push((candidate, true));
                    }
                }
            }
        }
        if candidates.is_empty() {
            candidates.push((private.to_path_buf(), false));
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let mut recovery_paths = Vec::new();
        for (candidate, owned) in candidates {
            let identity = owned.then(|| reserved_identity.cloned()).flatten();
            let association = if let Some(identity) = identity.as_ref() {
                #[cfg(test)]
                if let Some(hook) = &self.before_owned_candidate_anchor_open {
                    hook(&candidate);
                }
                match open_local_cleanup_anchor(&candidate) {
                    Ok(file) => {
                        let anchor: Arc<dyn BackendOwnershipAnchor> =
                            Arc::new(LocalOwnershipAnchor {
                                file,
                                root: self.root.clone(),
                                opaque_paths: Some(self.opaque_recovery_paths.clone()),
                            });
                        Some((identity.clone(), anchor))
                    }
                    Err(_) => None,
                }
            } else {
                None
            };
            let recovery_path = {
                let _lifecycle = self
                    .artifact_lifecycle
                    .lock()
                    .expect("artifact lifecycle lock poisoned");
                self.persist_unresolved_physical_candidate_locked(
                    if owned {
                        "directory-reservation-owned-candidate"
                    } else {
                        "directory-reservation-ambiguous-candidate"
                    },
                    &candidate,
                    identity,
                    association,
                    Some(owned),
                )?
            };
            recovery_paths.push(recovery_path);
        }
        Ok(recovery_paths)
    }

    fn retain_marker_recovery(
        &self,
        marker: &OwnedReservationMarker,
        local_path: &Path,
    ) -> PathBuf {
        let recovery_path = PathBuf::from("/.zaplex-opaque-directory-marker")
            .join(uuid::Uuid::new_v4().to_string());
        self.opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .insert(recovery_path.clone(), local_path.to_path_buf());
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .insert(
                recovery_path.clone(),
                (marker.identity.clone(), marker.anchor.clone()),
            );
        recovery_path
    }

    fn remove_owned_reservation_marker(
        &self,
        marker: &OwnedReservationMarker,
    ) -> Result<(), SftpOpsError> {
        if !marker.anchor.matches_path(&marker.path)? {
            let recovery_path = self.retain_marker_recovery(marker, &marker.path);
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Directory reservation marker ownership changed at {}",
                    marker.path.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: true,
            });
        }
        let tombstone = marker.path.with_file_name(format!(
            ".{}.zaplex-marker-cleanup-{}",
            marker
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let rename_error = rename_noreplace(&marker.path, &tombstone).err();
        let at_marker = marker.anchor.matches_path(&marker.path)?;
        let at_tombstone = marker.anchor.matches_path(&tombstone)?;
        if !at_tombstone {
            if !at_marker && tombstone.exists() && !marker.path.exists() {
                let _ = rename_noreplace(&tombstone, &marker.path);
            }
            let recovery_path = self.retain_marker_recovery(marker, &marker.path);
            return Err(SftpOpsError::RecoveryRequired {
                message: rename_error
                    .map(|error| {
                        format!(
                            "Isolating directory reservation marker failed at {}: {error}",
                            marker.path.display()
                        )
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "Directory reservation marker isolation was indeterminate at {}",
                            marker.path.display()
                        )
                    }),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: true,
            });
        }
        if at_marker {
            let recovery_path = self.retain_marker_recovery(marker, &tombstone);
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Directory reservation marker resolves to multiple paths at {}",
                    marker.path.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: true,
            });
        }
        #[cfg(test)]
        if self.fail_directory_marker_cleanup {
            let recovery_path = self.retain_marker_recovery(marker, &tombstone);
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Injected directory reservation marker cleanup failure at {}",
                    tombstone.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: true,
            });
        }
        if let Err(error) = fs::remove_file(&tombstone) {
            let recovery_path = self.retain_marker_recovery(marker, &tombstone);
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Removing isolated directory reservation marker failed at {}: {error}",
                    tombstone.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: true,
            });
        }
        Ok(())
    }

    fn validate_directory_reservation_namespace(
        &self,
        namespace: &DirectoryReservationNamespace,
    ) -> Result<(), SftpOpsError> {
        if !namespace.anchor.matches_path(&namespace.path)? {
            return Err(SftpOpsError::Operation(format!(
                "Protected directory reservation namespace changed: {}",
                namespace.path.display()
            )));
        }
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted directory reservation registry is unavailable".to_string(),
                )
            })?;
        let marker = namespace.path.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
        let metadata = validate_private_registry_file(&marker)?;
        let payload = registry.verify_marker(&fs::read_to_string(&marker)?)?;
        let expected = registry.namespace_payload(
            &namespace.path,
            namespace.device,
            &namespace.namespace_id,
            &namespace.anchor.identity()?.object_id,
            &namespace.generation,
        );
        if payload != expected {
            return Err(SftpOpsError::Operation(format!(
                "Protected directory reservation namespace ownership is invalid: {}",
                namespace.path.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(SftpOpsError::Operation(format!(
                    "Protected directory reservation marker permissions are invalid: {}",
                    marker.display()
                )));
            }
            let namespace_metadata = fs::symlink_metadata(&namespace.path)?;
            if namespace_metadata.dev() != namespace.device
                || namespace_metadata.uid() != unsafe { libc::geteuid() }
                || namespace_metadata.mode() & 0o077 != 0
            {
                return Err(SftpOpsError::Operation(format!(
                    "Protected directory reservation namespace identity is invalid: {}",
                    namespace.path.display()
                )));
            }
        }
        Ok(())
    }

    fn open_directory_reservation_namespace(
        &self,
        candidate: &Path,
        record: Option<&DirectoryNamespaceRecord>,
    ) -> Result<DirectoryReservationNamespace, SftpOpsError> {
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted directory reservation registry is unavailable".to_string(),
                )
            })?;
        let created_file = match create_local_directory_with_anchor(
            candidate,
            #[cfg(test)]
            self.after_namespace_create_before_anchor.as_ref(),
        ) {
            Ok(file) => Some(file),
            Err(error) if error.source.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => {
                return Err(SftpOpsError::Operation(format!(
                    "Creating protected directory reservation namespace failed at {}: {}",
                    candidate.display(),
                    error.source
                )));
            }
        };
        let created = created_file.is_some();
        if !created && record.is_none() {
            return Err(SftpOpsError::Operation(format!(
                "Untrusted directory reservation namespace already exists at {}",
                candidate.display()
            )));
        }
        #[cfg(unix)]
        if created {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(candidate, fs::Permissions::from_mode(0o700))?;
        }
        let file = match created_file {
            Some(file) => file,
            None => open_local_cleanup_anchor(candidate)?,
        };
        let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file,
            root: PathBuf::from("/"),
            opaque_paths: None,
        });
        if !anchor.matches_path(candidate)? {
            return Err(SftpOpsError::Operation(format!(
                "Protected directory reservation namespace changed before authentication: {}",
                candidate.display()
            )));
        }
        #[cfg(unix)]
        let device = {
            use std::os::unix::fs::MetadataExt;

            anchor.identity()?;
            fs::symlink_metadata(candidate)?.dev()
        };
        #[cfg(not(unix))]
        let device = 0;
        let object_id = anchor.identity()?.object_id;
        let namespace_id = record
            .map(|record| record.namespace_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let generation = record
            .map(|record| record.generation.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let namespace = DirectoryReservationNamespace {
            path: candidate.to_path_buf(),
            anchor,
            device,
            namespace_id,
            generation,
        };
        if let Some(record) = record.filter(|_| !created) {
            if record.path != candidate
                || record.device != device
                || record.object_id != object_id
                || record.namespace_id != namespace.namespace_id
            {
                return Err(SftpOpsError::Operation(format!(
                    "Trusted directory reservation namespace identity changed at {}",
                    candidate.display()
                )));
            }
        } else {
            let marker = candidate.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                options.mode(0o600);
            }
            let payload = registry.namespace_payload(
                candidate,
                device,
                &namespace.namespace_id,
                &object_id,
                &namespace.generation,
            );
            let mut file = options.open(&marker)?;
            file.write_all(registry.marker_contents(&payload).as_bytes())?;
            file.sync_all()?;
            registry.write_namespace_record(&DirectoryNamespaceRecord {
                path: candidate.to_path_buf(),
                device,
                namespace_id: namespace.namespace_id.clone(),
                object_id,
                generation: namespace.generation.clone(),
                legacy: false,
            })?;
        }
        self.validate_directory_reservation_namespace(&namespace)?;
        Ok(namespace)
    }

    fn reserve_directory_namespace_path(&self, path: PathBuf) -> NamespacePathReservation<'_> {
        let mut reservations = self
            .reserved_directory_namespace_paths
            .lock()
            .expect("reserved directory namespace lock poisoned");
        *reservations.entry(path.clone()).or_insert(0) += 1;
        drop(reservations);
        NamespacePathReservation {
            reservations: &self.reserved_directory_namespace_paths,
            path,
            committed: false,
        }
    }

    fn reserve_directory_namespace_path_permanently(&self, path: PathBuf) {
        self.reserve_directory_namespace_path(path).commit();
    }

    fn directory_reservation_namespace(
        &self,
        visible: &Path,
    ) -> Result<DirectoryReservationNamespace, SftpOpsError> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let mut parent = visible.parent().unwrap_or(visible);
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted directory reservation registry is unavailable".to_string(),
                )
            })?;
        let parent_metadata = loop {
            match registry.probe_namespace_parent(parent)? {
                Some(metadata) if metadata.is_dir() => break metadata,
                Some(_) => {
                    return Err(SftpOpsError::Operation(format!(
                        "Directory reservation parent is not a directory: {}",
                        parent.display()
                    )));
                }
                None => {
                    parent = parent.parent().ok_or_else(|| {
                        SftpOpsError::Operation(format!(
                            "No existing directory for reservation at {}",
                            visible.display()
                        ))
                    })?;
                }
            }
        };
        #[cfg(unix)]
        let target_device = parent_metadata.dev();
        #[cfg(not(unix))]
        let target_device = 0;
        if let Some(namespace) = self
            .directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .values()
            .find(|namespace| namespace.device == target_device)
            .cloned()
        {
            self.validate_directory_reservation_namespace(&namespace)?;
            return Ok(namespace);
        }
        let record_path = registry.record_path(target_device);
        let record = if registry.probe_namespace_record(&record_path)?.is_some() {
            Some(registry.read_namespace_record(&record_path)?)
        } else {
            None
        };
        let name = self.directory_reservation_namespace_name();
        let root_parent = self.root.parent().unwrap_or(&self.root);
        #[cfg(unix)]
        let sibling_is_safe = if root_parent != self.root {
            registry
                .probe_namespace_parent(root_parent)?
                .is_some_and(|metadata| metadata.is_dir() && metadata.dev() == target_device)
        } else {
            false
        };
        #[cfg(not(unix))]
        let sibling_is_safe = false;
        #[cfg(test)]
        let sibling_is_safe =
            sibling_is_safe && !self.force_in_tree_directory_reservation_namespace;
        let generated_candidate = if sibling_is_safe {
            root_parent.join(name)
        } else {
            parent.join(name)
        };
        let recorded_candidate = if let Some(record) = &record {
            let record_parent = record.path.parent().ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Trusted namespace record has no parent: {}",
                    record.path.display()
                ))
            })?;
            let metadata = registry.probe_namespace_parent(record_parent)?;
            #[cfg(unix)]
            if metadata
                .as_ref()
                .is_some_and(|metadata| metadata.dev() != target_device)
            {
                None
            } else {
                metadata
                    .filter(|metadata| metadata.is_dir())
                    .map(|_| record.path.clone())
            }
            #[cfg(not(unix))]
            metadata
                .filter(|metadata| metadata.is_dir())
                .map(|_| record.path.clone())
        } else {
            None
        };
        let candidate = recorded_candidate.unwrap_or(generated_candidate);
        // The scoped reservation closes creation-to-registration exposure.
        // Failure removes only this caller's token; successful authenticated
        // ownership commits it for the backend lifetime.
        let reservation = self.reserve_directory_namespace_path(candidate.clone());
        registry.probe_namespace_path(&candidate)?;
        if let Some(namespace) = self
            .directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .get(&candidate)
            .cloned()
        {
            self.validate_directory_reservation_namespace(&namespace)?;
            reservation.commit();
            return Ok(namespace);
        }
        let namespace = self.open_directory_reservation_namespace(&candidate, record.as_ref())?;
        self.scan_directory_reservations(&namespace);
        self.directory_reservation_namespaces
            .lock()
            .expect("directory reservation namespace lock poisoned")
            .insert(candidate, namespace.clone());
        reservation.commit();
        Ok(namespace)
    }

    fn discover_root_directory_reservations(&self) {
        let Some(registry) = &self.directory_reservation_registry else {
            return;
        };
        for record in registry.records() {
            let record = match record {
                Ok(record) => record,
                Err(error) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "namespace-record",
                            &error.to_string(),
                        ));
                    continue;
                }
            };
            self.reserve_directory_namespace_path_permanently(record.path.clone());
            match registry.probe_namespace_path(&record.path) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "namespace-missing",
                            &record.path.display().to_string(),
                        ));
                    continue;
                }
                Err(error) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "namespace-probe",
                            &format!("{}:{error}", record.path.display()),
                        ));
                    continue;
                }
            }
            let namespace =
                match self.open_directory_reservation_namespace(&record.path, Some(&record)) {
                    Ok(namespace) => namespace,
                    Err(error) => {
                        self.startup_unresolved_paths
                            .lock()
                            .expect("startup unresolved path lock poisoned")
                            .insert(Self::unresolved_registry_path(
                                "namespace-open",
                                &format!("{}:{error}", record.path.display()),
                            ));
                        continue;
                    }
                };
            self.scan_directory_reservations(&namespace);
            self.directory_reservation_namespaces
                .lock()
                .expect("directory reservation namespace lock poisoned")
                .insert(record.path, namespace);
        }
    }

    fn directory_reservation_namespace_name(&self) -> String {
        self.directory_reservation_registry
            .as_ref()
            .map(DirectoryReservationRegistry::namespace_name)
            .unwrap_or_else(|| format!("{DIRECTORY_RESERVATION_NAMESPACE_PREFIX}-unavailable"))
    }

    fn scan_directory_reservations(&self, namespace: &DirectoryReservationNamespace) {
        let Some(registry) = &self.directory_reservation_registry else {
            return;
        };
        #[cfg(test)]
        let injected_failure = self
            .namespace_scan_failure
            .lock()
            .expect("namespace scan failure lock poisoned")
            .take();
        #[cfg(test)]
        if injected_failure == Some(NamespaceScanFailure::ReadDirectory) {
            self.startup_unresolved_paths
                .lock()
                .expect("startup unresolved path lock poisoned")
                .insert(Self::unresolved_registry_path(
                    "namespace-enumeration",
                    &namespace.path.display().to_string(),
                ));
            return;
        }
        let entries = match fs::read_dir(&namespace.path) {
            Ok(entries) => entries,
            Err(error) => {
                self.startup_unresolved_paths
                    .lock()
                    .expect("startup unresolved path lock poisoned")
                    .insert(Self::unresolved_registry_path(
                        "namespace-enumeration",
                        &format!("{}:{error}", namespace.path.display()),
                    ));
                return;
            }
        };
        let mut collected_entries = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => collected_entries.push(entry),
                Err(error) => {
                    self.startup_unresolved_paths
                        .lock()
                        .expect("startup unresolved path lock poisoned")
                        .insert(Self::unresolved_registry_path(
                            "namespace-entry",
                            &format!("{}:{error}", namespace.path.display()),
                        ));
                }
            }
        }
        #[cfg(test)]
        if injected_failure == Some(NamespaceScanFailure::DirectoryEntry) {
            self.startup_unresolved_paths
                .lock()
                .expect("startup unresolved path lock poisoned")
                .insert(Self::unresolved_registry_path(
                    "namespace-entry",
                    &namespace.path.display().to_string(),
                ));
        }
        let entries = collected_entries;
        let mut authenticated_private_names = std::collections::HashSet::new();
        let mut authenticated_marker_names = std::collections::HashSet::new();
        let mut claimed_private_names = std::collections::HashSet::new();
        for entry in &entries {
            let marker = entry.path();
            let marker_name = entry.file_name().to_string_lossy().into_owned();
            let Some(private_name) = marker_name.strip_suffix(DIRECTORY_RESERVATION_MARKER_SUFFIX)
            else {
                continue;
            };
            claimed_private_names.insert(private_name.to_string());
            #[cfg(test)]
            if injected_failure == Some(NamespaceScanFailure::MarkerFileType) {
                self.persist_unresolved_diagnostic(
                    "namespace-marker-file-type",
                    &marker.display().to_string(),
                );
                continue;
            }
            let marker_type = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-marker-file-type",
                        &format!("{}:{error}", marker.display()),
                    );
                    continue;
                }
            };
            if !marker_type.is_file() {
                continue;
            }
            if validate_private_registry_file(&marker).is_err() {
                continue;
            }
            let contents = match fs::read_to_string(&marker) {
                Ok(contents) => contents,
                Err(_) => continue,
            };
            let payload = match registry.verify_marker(&contents) {
                Ok(payload) => payload,
                Err(_) => continue,
            };
            let private = namespace.path.join(private_name);
            let file = match open_local_cleanup_anchor(&private) {
                Ok(file) => file,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-private-open",
                        &format!("{}:{error}", private.display()),
                    );
                    continue;
                }
            };
            let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file,
                root: PathBuf::from("/"),
                opaque_paths: Some(self.opaque_recovery_paths.clone()),
            });
            let identity = match anchor.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-private-identity",
                        &format!("{}:{error}", private.display()),
                    );
                    continue;
                }
            };
            if identity.file_type != FileEntryType::Directory {
                continue;
            }
            let expected_payload = format!(
                "zaplex-owned-directory-v1\nnamespace={}\nreservation={private_name}\nobject={}",
                namespace.namespace_id, identity.object_id
            );
            let matches_private = match anchor.matches_path(&private) {
                Ok(matches) => matches,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-private-match",
                        &format!("{}:{error}", private.display()),
                    );
                    continue;
                }
            };
            if payload != expected_payload || !matches_private {
                continue;
            }
            let marker_file = match open_local_cleanup_anchor(&marker) {
                Ok(file) => file,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-marker-anchor",
                        &format!("{}:{error}", marker.display()),
                    );
                    continue;
                }
            };
            let marker_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file: marker_file,
                root: PathBuf::from("/"),
                opaque_paths: None,
            });
            let marker_identity = match marker_anchor.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-marker-identity",
                        &format!("{}:{error}", marker.display()),
                    );
                    continue;
                }
            };
            authenticated_private_names.insert(private_name.to_string());
            authenticated_marker_names.insert(marker_name);
            if let Err(error) = self.register_directory_reservation_recovery(
                &private,
                Some(OwnedReservationMarker {
                    path: marker,
                    identity: marker_identity,
                    anchor: marker_anchor,
                }),
                Some(identity),
                Some(anchor),
            ) {
                self.persist_unresolved_diagnostic(
                    "directory-reservation-registration",
                    &format!("{}:{error}", private.display()),
                );
            }
        }
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == DIRECTORY_RESERVATION_NAMESPACE_MARKER
                || authenticated_marker_names.contains(&name)
                || authenticated_private_names.contains(&name)
            {
                continue;
            }
            #[cfg(test)]
            if injected_failure == Some(NamespaceScanFailure::UnclaimedFileType) {
                self.persist_unresolved_diagnostic(
                    "namespace-unclaimed-file-type",
                    &entry.path().display().to_string(),
                );
                continue;
            }
            let entry_path = entry.path();
            let entry_type = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    self.persist_unresolved_diagnostic(
                        "namespace-unclaimed-file-type",
                        &format!("{}:{error}", entry_path.display()),
                    );
                    continue;
                }
            };
            if entry_type.is_file() {
                let claims_marker = name.ends_with(DIRECTORY_RESERVATION_MARKER_SUFFIX);
                let authenticated_marker = (|| -> Result<bool, SftpOpsError> {
                    validate_private_registry_file(&entry_path)?;
                    let contents = fs::read_to_string(&entry_path)?;
                    Ok(registry
                        .verify_marker(&contents)?
                        .starts_with("zaplex-owned-directory-v1\n"))
                })();
                match authenticated_marker {
                    Ok(true) => match open_local_cleanup_anchor(&entry_path) {
                        Ok(file) => {
                            let anchor: Arc<dyn BackendOwnershipAnchor> =
                                Arc::new(LocalOwnershipAnchor {
                                    file,
                                    root: PathBuf::from("/"),
                                    opaque_paths: Some(self.opaque_recovery_paths.clone()),
                                });
                            match anchor.identity() {
                                Ok(identity) => {
                                    if let Err(error) = self
                                        .register_directory_reservation_recovery(
                                            &entry_path,
                                            None,
                                            Some(identity),
                                            Some(anchor),
                                        )
                                    {
                                        self.persist_unresolved_diagnostic(
                                            "directory-reservation-registration",
                                            &format!("{}:{error}", entry_path.display()),
                                        );
                                    }
                                    continue;
                                }
                                Err(error) => {
                                    self.persist_unresolved_diagnostic(
                                        "namespace-unclaimed-identity",
                                        &format!("{}:{error}", entry_path.display()),
                                    );
                                    continue;
                                }
                            }
                        }
                        Err(error) => {
                            self.persist_unresolved_diagnostic(
                                "namespace-unclaimed-open",
                                &format!("{}:{error}", entry_path.display()),
                            );
                            continue;
                        }
                    },
                    Ok(false) => {}
                    Err(_) => {
                        if claims_marker {
                            let _ = self.register_directory_reservation_recovery(
                                &entry_path,
                                None,
                                None,
                                None,
                            );
                        }
                        continue;
                    }
                }
            }
            let unresolved_claim = (entry_type.is_dir() && claimed_private_names.contains(&name))
                || (entry_type.is_file() && name.ends_with(DIRECTORY_RESERVATION_MARKER_SUFFIX));
            if unresolved_claim {
                let _ = self.register_directory_reservation_recovery(&entry_path, None, None, None);
            }
        }
    }

    fn map_opaque_cleanup_sibling(&self, path: &Path, sibling: &Path) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let local = self
            .opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .get(path)
            .cloned();
        let Some(local) = local else {
            return Ok(());
        };
        let source_record = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned()
            .filter(|record| !record.retired && record.physical_path.as_ref() == Some(&local))
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Opaque cleanup source has no matching active generation at {}",
                    path.display()
                ))
            })?;
        #[cfg(test)]
        if let Some(hook) = &self.after_opaque_cleanup_source_read_before_lifecycle {
            hook(self, path);
        }
        let source_is_current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .is_some_and(|record| same_persistent_artifact_record(record, &source_record))
            && self
                .opaque_recovery_paths
                .lock()
                .expect("opaque recovery path lock poisoned")
                .get(path)
                == Some(&local);
        if !source_is_current {
            return Err(SftpOpsError::Operation(format!(
                "Opaque cleanup source changed before sibling publication at {}",
                path.display()
            )));
        }
        let sibling_local = local.with_file_name(
            sibling
                .file_name()
                .expect("cleanup sibling always has a file name"),
        );
        let role = Self::persistent_artifact_role(sibling).ok_or_else(|| {
            SftpOpsError::Operation(format!(
                "Opaque cleanup sibling is not a persistent artifact: {}",
                sibling.display()
            ))
        })?;
        let current = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(sibling)
            .cloned();
        let record = match current {
            Some(current)
                if !current.retired
                    && current.physical_path.as_ref() == Some(&sibling_local)
                    && current.role == role =>
            {
                current
            }
            Some(_) => {
                return Err(SftpOpsError::Operation(format!(
                    "Opaque cleanup sibling changed concurrently at {}",
                    sibling.display()
                )));
            }
            None => {
                let record = PersistentArtifactRecord::active(
                    sibling.to_path_buf(),
                    Some(sibling_local.clone()),
                    role.to_string(),
                    None,
                );
                self.directory_reservation_registry
                    .as_ref()
                    .ok_or_else(|| {
                        SftpOpsError::Operation(
                            "Trusted transfer artifact registry is unavailable".to_string(),
                        )
                    })?
                    .write_artifact_record(&record)?;
                record
            }
        };
        #[cfg(test)]
        if let Some(hook) = &self.at_opaque_cleanup_sibling_publication_cutpoint {
            hook(self, sibling);
        }
        self.persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .insert(sibling.to_path_buf(), record);
        self.opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .insert(sibling.to_path_buf(), sibling_local);
        Ok(())
    }

    fn preflight_local_filesystem(
        &self,
        path: &Path,
        require_exchange: bool,
    ) -> Result<(), SftpOpsError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::MetadataExt;

            let local = self.to_local(path)?;
            let mut parent = local.parent().unwrap_or(&self.root);
            let parent_metadata = loop {
                match DirectoryReservationRegistry::probe_path(parent)? {
                    Some(metadata) if metadata.is_dir() => break metadata,
                    Some(_) => {
                        return Err(SftpOpsError::Operation(format!(
                            "Filesystem ancestor is not a directory: {}",
                            parent.display()
                        )));
                    }
                    None => {
                        parent = parent.parent().ok_or_else(|| {
                            SftpOpsError::Operation(format!(
                                "No existing filesystem ancestor for {}",
                                path.display()
                            ))
                        })?;
                    }
                }
            };
            let filesystem = parent_metadata.dev();
            let key = (filesystem, require_exchange);
            if let Some(cached) = self
                .safe_mutation_capabilities
                .lock()
                .expect("safe mutation capability cache poisoned")
                .get(&key)
                .cloned()
            {
                return cached.map_err(SftpOpsError::Operation);
            }

            let token = uuid::Uuid::new_v4();
            let first = parent.join(format!(".zaplex-rename-probe-{token}-a"));
            let second = parent.join(format!(".zaplex-rename-probe-{token}-b"));
            let third = parent.join(format!(".zaplex-rename-probe-{token}-c"));
            let fourth = parent.join(format!(".zaplex-rename-probe-{token}-d"));
            let identity = |path: &Path| -> Result<_, std::io::Error> {
                let metadata = fs::symlink_metadata(path)?;
                Ok((
                    metadata.dev(),
                    metadata.ino(),
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                ))
            };
            let same_object =
                |before: &(u64, u64, u64, i64, i64, i64, i64),
                 after: &(u64, u64, u64, i64, i64, i64, i64)| {
                    before.0 == after.0
                        && before.1 == after.1
                        && before.2 == after.2
                        && before.3 == after.3
                        && before.4 == after.4
                };
            let same_reserved_inode =
                |before: &(u64, u64, u64, i64, i64, i64, i64),
                 after: &(u64, u64, u64, i64, i64, i64, i64)| {
                    before.0 == after.0 && before.1 == after.1
                };
            let stable_probe_identity =
                |identity: &(u64, u64, u64, i64, i64, i64, i64)| StableEntryIdentity {
                    file_type: FileEntryType::File,
                    size: identity.2,
                    object_id: format!("{}:{}", identity.0, identity.1),
                    revision: format!(
                        "{}:{}:{}:{}:{}:{}",
                        identity.0, identity.1, identity.3, identity.4, identity.5, identity.6
                    ),
                };
            let mut owned_first = None;
            let mut owned_second = None;
            let mut owned_third = None;
            let mut owned_fourth = None;
            let mut anchors: [Option<fs::File>; 4] = [None, None, None, None];
            let mut uncertain_paths = Vec::new();
            let probe = (|| -> Result<(), std::io::Error> {
                let mut first_file = match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&first)
                {
                    Ok(file) => file,
                    Err(error) => {
                        if identity(&first).is_ok() {
                            uncertain_paths.push(first.clone());
                        }
                        return Err(error);
                    }
                };
                let first_reserved_identity = identity(&first)?;
                #[cfg(test)]
                if self.fail_preflight_create_after_apply == Some('a') {
                    *self
                        .preflight_uncertain_create
                        .lock()
                        .expect("preflight uncertain create lock poisoned") = Some(first.clone());
                    uncertain_paths.push(first.clone());
                    return Err(std::io::Error::other(
                        "injected first-create acknowledgement failure",
                    ));
                }
                first_file.write_all(b"first")?;
                let first_identity = identity(&first)?;
                if !same_reserved_inode(&first_reserved_identity, &first_identity) {
                    uncertain_paths.push(first.clone());
                    return Err(std::io::Error::other(
                        "first probe reservation changed while initializing",
                    ));
                }
                owned_first = Some(first_identity);
                anchors[0] = Some(first_file);
                #[cfg(test)]
                self.inject_preflight_collision(&second, 'b')?;
                let mut second_file = match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&second)
                {
                    Ok(file) => file,
                    Err(error) => {
                        if identity(&second).is_ok() {
                            uncertain_paths.push(second.clone());
                        }
                        return Err(error);
                    }
                };
                let second_reserved_identity = identity(&second)?;
                #[cfg(test)]
                if self.fail_preflight_create_after_apply == Some('b') {
                    *self
                        .preflight_uncertain_create
                        .lock()
                        .expect("preflight uncertain create lock poisoned") = Some(second.clone());
                    uncertain_paths.push(second.clone());
                    return Err(std::io::Error::other(
                        "injected second-create acknowledgement failure",
                    ));
                }
                second_file.write_all(b"sentinel")?;
                let second_identity = identity(&second)?;
                if !same_reserved_inode(&second_reserved_identity, &second_identity) {
                    uncertain_paths.push(second.clone());
                    return Err(std::io::Error::other(
                        "second probe reservation changed while initializing",
                    ));
                }
                owned_second = Some(second_identity);
                anchors[1] = Some(second_file);
                #[cfg(test)]
                if self.replace_preflight_sources_before_reject {
                    let retained_first = first.with_file_name(".review11-original-reject-first");
                    let retained_second = second.with_file_name(".review11-original-reject-second");
                    fs::rename(&first, retained_first)?;
                    fs::rename(&second, retained_second)?;
                    fs::write(&first, b"foreign-first")?;
                    fs::write(&second, b"foreign-second")?;
                    *self
                        .preflight_reject_replacements
                        .lock()
                        .expect("preflight reject replacements lock poisoned") =
                        vec![first.clone(), second.clone()];
                }
                let anchor_matches_path = |anchor: Option<&fs::File>,
                                           expected: &(u64, u64, u64, i64, i64, i64, i64),
                                           path: &Path| {
                    anchor
                        .and_then(|anchor| anchor.metadata().ok())
                        .zip(identity(path).ok())
                        .is_some_and(|(anchor, current)| {
                            anchor.dev() == current.0
                                && anchor.ino() == current.1
                                && same_object(expected, &current)
                        })
                };
                if !anchor_matches_path(anchors[0].as_ref(), &first_identity, &first)
                    || !anchor_matches_path(anchors[1].as_ref(), &second_identity, &second)
                {
                    if identity(&first).is_ok() {
                        uncertain_paths.push(first.clone());
                    }
                    if identity(&second).is_ok() {
                        uncertain_paths.push(second.clone());
                    }
                    return Err(std::io::Error::other(
                        "NOREPLACE negative probe operands changed immediately before mutation",
                    ));
                }
                #[cfg(test)]
                let rejected = if self.ignore_noreplace_probe_semantics {
                    fs::rename(&first, &second)
                } else {
                    rename_noreplace(&first, &second)
                };
                #[cfg(not(test))]
                let rejected = rename_noreplace(&first, &second);
                let first_after_reject = identity(&first);
                let second_after_reject = identity(&second);
                let operands_unchanged = first_after_reject
                    .as_ref()
                    .is_ok_and(|actual| same_object(&first_identity, actual))
                    && second_after_reject
                        .as_ref()
                        .is_ok_and(|actual| same_object(&second_identity, actual));
                if !operands_unchanged {
                    if first_after_reject.is_ok() {
                        uncertain_paths.push(first.clone());
                    }
                    if second_after_reject.is_ok() {
                        uncertain_paths.push(second.clone());
                    }
                    return Err(std::io::Error::other(format!(
                        "NOREPLACE negative probe changed an operand: {}",
                        rejected
                            .as_ref()
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "operation unexpectedly succeeded".to_string())
                    )));
                }
                match rejected {
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(std::io::Error::other(format!(
                            "NOREPLACE probe returned the wrong error for an existing target: {error}"
                        )));
                    }
                    Ok(()) => {
                        return Err(std::io::Error::other(
                            "NOREPLACE probe replaced an existing target",
                        ));
                    }
                }
                if fs::read(&first)? != b"first" || fs::read(&second)? != b"sentinel" {
                    return Err(std::io::Error::other(
                        "NOREPLACE probe changed its source or sentinel target",
                    ));
                }

                #[cfg(test)]
                self.inject_preflight_collision(&third, 'c')?;
                #[cfg(test)]
                if self.replace_preflight_source_before_rename {
                    let retained = first.with_file_name(".review10-original-rename-probe");
                    fs::rename(&first, retained)?;
                    fs::write(&first, b"foreign-rename")?;
                    *self
                        .preflight_mutation_replacement
                        .lock()
                        .expect("preflight mutation replacement lock poisoned") =
                        Some(first.clone());
                }
                let first_before_move = identity(&first);
                let first_anchor_matches = anchors[0]
                    .as_ref()
                    .and_then(|anchor| anchor.metadata().ok())
                    .zip(first_before_move.as_ref().ok())
                    .is_some_and(|(anchor, path)| {
                        anchor.dev() == path.0
                            && anchor.ino() == path.1
                            && same_object(&first_identity, path)
                    });
                if !first_anchor_matches {
                    if first_before_move.is_ok() {
                        uncertain_paths.push(first.clone());
                    }
                    if identity(&third).is_ok() {
                        uncertain_paths.push(third.clone());
                    }
                    return Err(std::io::Error::other(
                        "NOREPLACE probe source changed immediately before mutation",
                    ));
                }
                #[cfg(test)]
                let moved = if self.preflight_rename_copy_unlink {
                    fs::copy(&first, &third)?;
                    fs::remove_file(&first)
                } else {
                    rename_noreplace(&first, &third)
                };
                #[cfg(not(test))]
                let moved = rename_noreplace(&first, &third);
                #[cfg(test)]
                let moved = if moved.is_ok() && self.fail_preflight_rename_after_apply {
                    Err(std::io::Error::other(
                        "injected rename acknowledgement failure",
                    ))
                } else {
                    moved
                };
                match moved {
                    Ok(()) => {}
                    Err(error) => {
                        let first_after = identity(&first);
                        let third_after = identity(&third);
                        if first_after
                            .as_ref()
                            .is_err_and(|probe| probe.kind() == std::io::ErrorKind::NotFound)
                            && third_after
                                .as_ref()
                                .is_ok_and(|actual| same_object(&first_identity, actual))
                        {
                            owned_third = third_after.ok();
                            owned_first = None;
                            anchors[2] = anchors[0].take();
                        } else {
                            if first_after.is_ok() {
                                uncertain_paths.push(first.clone());
                            }
                            if third_after.is_ok() {
                                uncertain_paths.push(third.clone());
                            }
                        }
                        return Err(error);
                    }
                }
                let third_identity = identity(&third)?;
                if first.exists()
                    || fs::read(&third)? != b"first"
                    || !same_object(&first_identity, &third_identity)
                {
                    if identity(&first).is_ok() {
                        uncertain_paths.push(first.clone());
                    }
                    if identity(&third).is_ok() {
                        uncertain_paths.push(third.clone());
                    }
                    return Err(std::io::Error::other(
                        "NOREPLACE success probe did not atomically move the source object intact",
                    ));
                }
                owned_third = Some(third_identity);
                owned_first = None;
                anchors[2] = anchors[0].take();
                if require_exchange {
                    #[cfg(test)]
                    self.inject_preflight_collision(&fourth, 'd')?;
                    let mut fourth_file = match fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&fourth)
                    {
                        Ok(file) => file,
                        Err(error) => {
                            if identity(&fourth).is_ok() {
                                uncertain_paths.push(fourth.clone());
                            }
                            return Err(error);
                        }
                    };
                    let fourth_reserved_identity = identity(&fourth)?;
                    #[cfg(test)]
                    if self.fail_preflight_create_after_apply == Some('d') {
                        *self
                            .preflight_uncertain_create
                            .lock()
                            .expect("preflight uncertain create lock poisoned") =
                            Some(fourth.clone());
                        uncertain_paths.push(fourth.clone());
                        return Err(std::io::Error::other(
                            "injected fourth-create acknowledgement failure",
                        ));
                    }
                    fourth_file.write_all(b"fourth")?;
                    let fourth_identity = identity(&fourth)?;
                    if !same_reserved_inode(&fourth_reserved_identity, &fourth_identity) {
                        uncertain_paths.push(fourth.clone());
                        return Err(std::io::Error::other(
                            "fourth probe reservation changed while initializing",
                        ));
                    }
                    owned_fourth = Some(fourth_identity);
                    anchors[3] = Some(fourth_file);
                    #[cfg(test)]
                    if self.replace_preflight_source_before_exchange {
                        let retained = third.with_file_name(".review10-original-exchange-probe");
                        fs::rename(&third, retained)?;
                        fs::write(&third, b"foreign-exchange")?;
                        *self
                            .preflight_mutation_replacement
                            .lock()
                            .expect("preflight mutation replacement lock poisoned") =
                            Some(third.clone());
                    }
                    let third_before_exchange = identity(&third);
                    let fourth_before_exchange = identity(&fourth);
                    let third_anchor_matches = anchors[2]
                        .as_ref()
                        .and_then(|anchor| anchor.metadata().ok())
                        .zip(third_before_exchange.as_ref().ok())
                        .is_some_and(|(anchor, path)| {
                            anchor.dev() == path.0
                                && anchor.ino() == path.1
                                && same_object(&third_identity, path)
                        });
                    let fourth_anchor_matches = anchors[3]
                        .as_ref()
                        .and_then(|anchor| anchor.metadata().ok())
                        .zip(fourth_before_exchange.as_ref().ok())
                        .is_some_and(|(anchor, path)| {
                            anchor.dev() == path.0
                                && anchor.ino() == path.1
                                && same_object(&fourth_identity, path)
                        });
                    if !third_anchor_matches || !fourth_anchor_matches {
                        if third_before_exchange.is_ok() {
                            uncertain_paths.push(third.clone());
                        }
                        if fourth_before_exchange.is_ok() {
                            uncertain_paths.push(fourth.clone());
                        }
                        return Err(std::io::Error::other(
                            "atomic exchange probe source changed immediately before mutation",
                        ));
                    }
                    #[cfg(test)]
                    let exchanged = if self.preflight_exchange_content_swap {
                        let third_contents = fs::read(&third)?;
                        let fourth_contents = fs::read(&fourth)?;
                        fs::write(&third, fourth_contents)?;
                        fs::write(&fourth, third_contents)
                    } else {
                        replace_atomic_local(&third, &fourth)
                    };
                    #[cfg(not(test))]
                    let exchanged = replace_atomic_local(&third, &fourth);
                    if let Err(error) = exchanged {
                        if identity(&third).is_ok() {
                            uncertain_paths.push(third.clone());
                        }
                        if identity(&fourth).is_ok() {
                            uncertain_paths.push(fourth.clone());
                        }
                        return Err(error);
                    }
                    let third_after_exchange = identity(&third)?;
                    let fourth_after_exchange = identity(&fourth)?;
                    if fs::read(&third)? != b"fourth"
                        || fs::read(&fourth)? != b"first"
                        || !same_object(&fourth_identity, &third_after_exchange)
                        || !same_object(&third_identity, &fourth_after_exchange)
                    {
                        uncertain_paths.push(third.clone());
                        uncertain_paths.push(fourth.clone());
                        return Err(std::io::Error::other(
                            "atomic exchange probe did not swap the original filesystem objects",
                        ));
                    }
                    owned_third = Some(third_after_exchange);
                    owned_fourth = Some(fourth_after_exchange);
                    anchors.swap(2, 3);
                }
                Ok(())
            })();
            let mut cleanup_error = None;
            #[cfg(test)]
            let mut inject_cleanup_failure = self.fail_preflight_cleanup;
            #[cfg(not(test))]
            let mut inject_cleanup_failure = false;
            for (index, (probe_path, suffix, owned)) in [
                (&first, 'a', &mut owned_first),
                (&second, 'b', &mut owned_second),
                (&third, 'c', &mut owned_third),
                (&fourth, 'd', &mut owned_fourth),
            ]
            .into_iter()
            .enumerate()
            {
                #[cfg(not(test))]
                let _ = suffix;
                let Some(expected) = *owned else {
                    continue;
                };
                let anchor_matches = anchors[index]
                    .as_ref()
                    .and_then(|anchor| anchor.metadata().ok())
                    .is_some_and(|metadata| {
                        metadata.dev() == expected.0 && metadata.ino() == expected.1
                    });
                if !anchor_matches {
                    uncertain_paths.push(probe_path.to_path_buf());
                    if cleanup_error.is_none() {
                        cleanup_error = Some(std::io::Error::other(format!(
                            "preflight reservation anchor changed before cleanup: {}",
                            probe_path.display()
                        )));
                    }
                    continue;
                }
                #[cfg(all(test, target_os = "linux"))]
                if self.observe_preflight_cleanup_anchor == Some(suffix) {
                    let observed = fs::read_dir("/proc/self/fd")
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .filter_map(|entry| fs::metadata(entry.path()).ok())
                        .any(|metadata| {
                            metadata.dev() == expected.0 && metadata.ino() == expected.1
                        });
                    self.preflight_cleanup_anchor_observed
                        .store(observed, Ordering::SeqCst);
                }
                #[cfg(test)]
                {
                    if self.force_preflight_inode_reuse == Some(suffix) {
                        use std::ffi::CString;
                        use std::os::unix::ffi::OsStrExt;

                        let preserve_mtime = || -> Result<(), std::io::Error> {
                            let path = CString::new(probe_path.as_os_str().as_bytes())
                                .map_err(std::io::Error::other)?;
                            let times = [
                                libc::timespec {
                                    tv_sec: expected.3,
                                    tv_nsec: expected.4,
                                },
                                libc::timespec {
                                    tv_sec: expected.3,
                                    tv_nsec: expected.4,
                                },
                            ];
                            if unsafe {
                                libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), 0)
                            } == 0
                            {
                                Ok(())
                            } else {
                                Err(std::io::Error::last_os_error())
                            }
                        };
                        fs::remove_file(probe_path)?;
                        let mut replacement_inode = 0;
                        let mut reused = false;
                        for _ in 0..4096 {
                            fs::write(probe_path, vec![b'x'; expected.2 as usize])?;
                            preserve_mtime()?;
                            let metadata = fs::symlink_metadata(probe_path)?;
                            replacement_inode = metadata.ino();
                            if metadata.dev() == expected.0 && metadata.ino() == expected.1 {
                                reused = true;
                                break;
                            }
                            fs::remove_file(probe_path)?;
                        }
                        if !probe_path.exists() {
                            fs::write(probe_path, vec![b'x'; expected.2 as usize])?;
                            preserve_mtime()?;
                            replacement_inode = fs::symlink_metadata(probe_path)?.ino();
                        }
                        *self
                            .preflight_inode_reuse_observation
                            .lock()
                            .expect("preflight inode reuse observation lock poisoned") = Some((
                            probe_path.to_path_buf(),
                            expected.1,
                            replacement_inode,
                            reused,
                            expected.3,
                            expected.4,
                        ));
                    }
                    if self.replace_preflight_owned_before_cleanup == Some(suffix) {
                        fs::remove_file(probe_path)?;
                        fs::write(probe_path, format!("foreign-{suffix}"))?;
                        let metadata = fs::symlink_metadata(probe_path)?;
                        *self
                            .preflight_cleanup_replacement
                            .lock()
                            .expect("preflight cleanup replacement lock poisoned") =
                            Some((probe_path.to_path_buf(), metadata.dev(), metadata.ino()));
                    }
                }
                match identity(probe_path) {
                    Ok(actual) if actual == expected => {}
                    Ok(_) => {
                        uncertain_paths.push(probe_path.to_path_buf());
                        if cleanup_error.is_none() {
                            cleanup_error = Some(std::io::Error::other(format!(
                                "preflight owned path was replaced before cleanup: {}",
                                probe_path.display()
                            )));
                        }
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        *owned = None;
                        anchors[index] = None;
                        continue;
                    }
                    Err(error) => {
                        uncertain_paths.push(probe_path.to_path_buf());
                        if cleanup_error.is_none() {
                            cleanup_error = Some(error);
                        }
                        continue;
                    }
                }
                #[cfg(test)]
                {
                    if self.replace_preflight_owned_after_check == Some(suffix) {
                        fs::remove_file(probe_path)?;
                        fs::write(probe_path, format!("foreign-{suffix}"))?;
                        let metadata = fs::symlink_metadata(probe_path)?;
                        *self
                            .preflight_cleanup_replacement_after_check
                            .lock()
                            .expect("preflight cleanup replacement lock poisoned") =
                            Some((probe_path.to_path_buf(), metadata.dev(), metadata.ino()));
                    }
                }
                let tombstone = probe_path.with_file_name(format!(
                    ".{}.zaplex-probe-cleanup-{}",
                    probe_path
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_default(),
                    uuid::Uuid::new_v4()
                ));
                if let Err(error) = rename_noreplace(probe_path, &tombstone) {
                    uncertain_paths.push(probe_path.to_path_buf());
                    if tombstone.exists() {
                        uncertain_paths.push(tombstone);
                    }
                    if cleanup_error.is_none() {
                        cleanup_error = Some(std::io::Error::other(format!(
                            "preflight cleanup isolation failed for {}: {error}",
                            probe_path.display()
                        )));
                    }
                    continue;
                }
                let isolated_identity = identity(&tombstone);
                if !isolated_identity.as_ref().is_ok_and(|actual| {
                    same_object(&expected, actual)
                        && anchors[index]
                            .as_ref()
                            .and_then(|anchor| anchor.metadata().ok())
                            .is_some_and(|metadata| {
                                metadata.dev() == actual.0 && metadata.ino() == actual.1
                            })
                }) {
                    let restore = rename_noreplace(&tombstone, probe_path);
                    uncertain_paths.push(probe_path.to_path_buf());
                    if restore.is_err() || tombstone.exists() {
                        uncertain_paths.push(tombstone);
                    }
                    *owned = None;
                    if cleanup_error.is_none() {
                        cleanup_error = Some(std::io::Error::other(format!(
                            "preflight cleanup isolated a replaced entry at {}",
                            probe_path.display()
                        )));
                    }
                    continue;
                }
                let cleanup = if inject_cleanup_failure {
                    inject_cleanup_failure = false;
                    Err(std::io::Error::other("injected preflight cleanup failure"))
                } else {
                    fs::remove_file(&tombstone)
                };
                match cleanup {
                    Ok(()) => {
                        *owned = None;
                        anchors[index] = None;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        *owned = None;
                        anchors[index] = None;
                    }
                    Err(error) => {
                        if let (Ok(actual), Some(anchor)) =
                            (isolated_identity, anchors[index].as_ref())
                        {
                            if let Ok(anchor) = local_ownership_anchor(anchor, &self.root) {
                                self.cleanup_recovery_identities
                                    .lock()
                                    .expect("cleanup recovery identity lock poisoned")
                                    .insert(
                                        self.to_remote(&tombstone),
                                        (stable_probe_identity(&actual), anchor),
                                    );
                            }
                        }
                        uncertain_paths.push(tombstone);
                        *owned = None;
                        anchors[index] = None;
                        if cleanup_error.is_none() {
                            cleanup_error = Some(error);
                        }
                    }
                }
            }
            for (probe_path, owned) in [
                (&first, &owned_first),
                (&second, &owned_second),
                (&third, &owned_third),
                (&fourth, &owned_fourth),
            ] {
                let Some(expected) = owned else {
                    continue;
                };
                match identity(probe_path) {
                    Ok(actual) if &actual == expected => {
                        cleanup_error = Some(std::io::Error::other(format!(
                            "preflight cleanup left probe artifact {}",
                            probe_path.display()
                        )));
                    }
                    Ok(_) => {
                        uncertain_paths.push(probe_path.to_path_buf());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) if cleanup_error.is_none() => cleanup_error = Some(error),
                    Err(_) => {}
                }
            }
            let probe = match (probe, cleanup_error) {
                (_, Some(cleanup_error)) => Err(std::io::Error::other(format!(
                    "rename capability probe cleanup failed: {cleanup_error}"
                ))),
                (probe, None) => probe,
            };
            uncertain_paths.sort();
            uncertain_paths.dedup();
            if !uncertain_paths.is_empty() {
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Required atomic rename primitive probe retained uncertain paths for {}",
                        path.display()
                    ),
                    recovery_id: None,
                    paths: uncertain_paths
                        .iter()
                        .map(|path| self.to_remote(path))
                        .collect(),
                    committed: false,
                });
            }
            let cached = probe
                .map_err(|error| {
                    format!(
                        "Required atomic rename primitives are unavailable for {}: {error}",
                        path.display()
                    )
                })
                .map(|_| ());
            self.safe_mutation_capabilities
                .lock()
                .expect("safe mutation capability cache poisoned")
                .insert(key, cached.clone());
            cached.map_err(SftpOpsError::Operation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = require_exchange;
            Err(SftpOpsError::Operation(format!(
                "Safe local rename primitives are unavailable for {}",
                path.display()
            )))
        }
    }

    fn restore_cleanup_isolation_noreplace(
        &self,
        path: &Path,
        tombstone: &Path,
        primary: SftpOpsError,
    ) -> SftpOpsError {
        let local_path = match self.to_local(path) {
            Ok(path) => path,
            Err(error) => return error,
        };
        let local_tombstone = match self.to_local(tombstone) {
            Ok(path) => path,
            Err(error) => return error,
        };
        match rename_noreplace(&local_tombstone, &local_path) {
            Ok(()) => SftpOpsError::Operation(primary.to_string()),
            Err(restore_error) => SftpOpsError::RecoveryRequired {
                message: format!(
                    "{primary}; atomically restoring cleanup isolation failed: {restore_error}"
                ),
                recovery_id: None,
                paths: vec![path.to_path_buf(), tombstone.to_path_buf()],
                committed: false,
            },
        }
    }

    /// Builds a FileEntry from std::fs::Metadata.
    fn metadata_to_entry(
        &self,
        name: String,
        local_path: &Path,
        meta: &std::fs::Metadata,
    ) -> FileEntry {
        let file_type = if meta.is_symlink() {
            FileEntryType::Symlink
        } else if meta.is_dir() {
            FileEntryType::Directory
        } else if meta.is_file() {
            FileEntryType::File
        } else {
            FileEntryType::Other
        };
        let modified = meta.modified().ok().map(|t| {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.format("%Y-%m-%d %H:%M").to_string()
        });
        FileEntry {
            name,
            path: self.to_remote(local_path),
            file_type,
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
            permissions: None,
            identity: stable_identity_from_local_metadata(meta),
        }
    }
}

impl SftpBackend for InMemorySftpBackend {
    fn supports_atomic_exchange(&self) -> bool {
        local_safe_rename_primitives_available()
    }

    fn supports_identity_bound_cleanup(&self) -> bool {
        local_safe_rename_primitives_available()
    }

    fn cleanup_recovery_identity(&self, path: &Path) -> Option<StableEntryIdentity> {
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .get(path)
            .map(|(identity, _)| identity.clone())
    }

    fn cleanup_recovery_anchor(&self, path: &Path) -> Option<Arc<dyn BackendOwnershipAnchor>> {
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .get(path)
            .map(|(_, anchor)| anchor.clone())
    }

    fn startup_recovery_paths(&self) -> Vec<PathBuf> {
        self.discovered_recovery_paths()
    }

    fn retry_unresolved_recovery(&self, path: &Path) -> Result<Option<Vec<PathBuf>>, SftpOpsError> {
        let exchange_record = {
            self.persistent_exchange_records
                .lock()
                .expect("persistent transfer exchange lock poisoned")
                .get(path)
                .cloned()
        };
        if let Some(record) = exchange_record {
            return self.resolve_persistent_exchange(&record).map(Some);
        }
        let Some(record) = self
            .persistent_artifact_records
            .lock()
            .expect("persistent transfer artifact lock poisoned")
            .get(path)
            .cloned()
        else {
            return Ok(None);
        };
        let Some(role) = record.role.strip_prefix("rescan-anchor-sibling:") else {
            return Ok(None);
        };
        let parent = record.physical_path.ok_or_else(|| {
            SftpOpsError::Operation(format!(
                "Sibling rescan has no physical parent for {}",
                path.display()
            ))
        })?;
        let expected = record.identity.ok_or_else(|| {
            SftpOpsError::Operation(format!(
                "Sibling rescan has no immutable identity for {}",
                path.display()
            ))
        })?;
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&parent)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let actual = stable_identity_from_local_metadata(&metadata);
            if same_immutable_object(&expected, &actual) {
                candidates.push(entry.path());
            }
        }
        let physical = match candidates.as_slice() {
            [physical] => physical.clone(),
            [] => {
                return Err(SftpOpsError::Operation(format!(
                    "Sibling rescan found no identity match below {}",
                    parent.display()
                )));
            }
            _ => {
                return Err(SftpOpsError::Operation(format!(
                    "Sibling rescan found multiple identity matches below {}",
                    parent.display()
                )));
            }
        };
        #[cfg(test)]
        if let Some(hook) = &self.before_sibling_recovery_anchor_open {
            hook(&physical);
        }
        let file = open_local_cleanup_anchor(&physical)?;
        let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        });
        let anchored_identity = anchor.identity()?;
        if !same_immutable_object(&expected, &anchored_identity)
            || !anchor.matches_local_path(&physical)?
        {
            return Err(SftpOpsError::Operation(format!(
                "Sibling rescan candidate changed before ownership could be anchored at {}",
                physical.display()
            )));
        }
        if expected.file_type == FileEntryType::File && anchor.link_count()? != Some(1) {
            return Err(SftpOpsError::Operation(format!(
                "Sibling rescan found a multiply linked file at {}",
                physical.display()
            )));
        }
        let candidate = if physical.starts_with(&self.root) {
            self.to_remote(&physical)
        } else {
            let logical = self.persist_unresolved_physical_candidate_with_anchor(
                "anchor-sibling-resolved",
                &physical,
                expected.clone(),
                anchor.clone(),
            )?;
            logical
        };
        if physical.starts_with(&self.root) {
            let _lifecycle = self
                .artifact_lifecycle
                .lock()
                .expect("artifact lifecycle lock poisoned");
            let replacement = PersistentArtifactRecord::active(
                candidate.clone(),
                Some(physical),
                role.to_string(),
                Some(expected.clone()),
            );
            let registry = self
                .directory_reservation_registry
                .as_ref()
                .ok_or_else(|| {
                    SftpOpsError::Operation(
                        "Trusted transfer artifact registry is unavailable".to_string(),
                    )
                })?;
            registry.write_artifact_record(&replacement)?;
            self.persistent_artifact_records
                .lock()
                .expect("persistent transfer artifact lock poisoned")
                .insert(candidate.clone(), replacement);
            self.cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned")
                .insert(candidate.clone(), (expected, anchor));
        }
        self.release_persistent_artifact(path)?;
        Ok(Some(vec![candidate]))
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, SftpOpsError> {
        if self
            .startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .contains(path)
        {
            return Ok(true);
        }
        match self.lstat(path) {
            Ok(_) => Ok(true),
            Err(SftpOpsError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn existing_entry_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        let file = open_local_cleanup_anchor(&self.to_local(path)?)?;
        Ok(Some(Arc::new(LocalOwnershipAnchor {
            file,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        })))
    }

    fn forget_cleanup_recovery_identity(&self, path: &Path) {
        self.cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .remove(path);
    }

    fn release_cleanup_recovery_path(&self, path: &Path) -> Result<(), SftpOpsError> {
        let _lifecycle = self
            .artifact_lifecycle
            .lock()
            .expect("artifact lifecycle lock poisoned");
        let retiring_generation = self.persistent_artifact_generation(path);
        let marker = self
            .opaque_recovery_markers
            .lock()
            .expect("opaque recovery marker lock poisoned")
            .get(path)
            .cloned();
        let retiring_anchor = self
            .cleanup_recovery_identities
            .lock()
            .expect("cleanup recovery identity lock poisoned")
            .get(path)
            .map(|(_, anchor)| anchor.clone());
        let retiring_physical_path = self
            .opaque_recovery_paths
            .lock()
            .expect("opaque recovery path lock poisoned")
            .get(path)
            .cloned();
        self.release_persistent_artifact_locked(path, false)?;
        if self
            .persistent_artifact_generation(path)
            .is_some_and(|current| Some(current) != retiring_generation)
        {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact generation changed during cleanup retirement at {}",
                path.display()
            )));
        }
        #[cfg(test)]
        if let Some(hook) = &self.after_artifact_retirement_generation_check {
            hook(self, path);
        }
        if self.persistent_artifact_generation(path).is_some() {
            return Err(SftpOpsError::Operation(format!(
                "Transfer artifact generation changed before auxiliary cleanup at {}",
                path.display()
            )));
        }
        marker
            .as_ref()
            .map(|marker| self.remove_owned_reservation_marker(marker))
            .unwrap_or(Ok(()))?;
        if let Some(marker) = marker {
            let mut markers = self
                .opaque_recovery_markers
                .lock()
                .expect("opaque recovery marker lock poisoned");
            if markers.get(path).is_some_and(|current| {
                current.path == marker.path
                    && same_immutable_object(&current.identity, &marker.identity)
                    && Arc::ptr_eq(&current.anchor, &marker.anchor)
            }) {
                markers.remove(path);
            }
        }
        if let Some(retiring_anchor) = retiring_anchor {
            let mut identities = self
                .cleanup_recovery_identities
                .lock()
                .expect("cleanup recovery identity lock poisoned");
            if identities
                .get(path)
                .is_some_and(|(_, current)| Arc::ptr_eq(current, &retiring_anchor))
            {
                identities.remove(path);
            }
        }
        if let Some(retiring_physical_path) = retiring_physical_path {
            let mut paths = self
                .opaque_recovery_paths
                .lock()
                .expect("opaque recovery path lock poisoned");
            if paths.get(path) == Some(&retiring_physical_path) {
                paths.remove(path);
            }
        }
        self.startup_unresolved_paths
            .lock()
            .expect("startup unresolved path lock poisoned")
            .remove(path);
        Ok(())
    }

    fn preflight_safe_mutation(
        &self,
        path: &Path,
        require_exchange: bool,
    ) -> Result<(), SftpOpsError> {
        self.preflight_local_filesystem(path, require_exchange)
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
        let local = self.to_local(path)?;
        self.validate_resolved_local_path(&local, path)?;
        let p = path.display();
        let entries = fs::read_dir(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to list directory {p}: {e}")))?;
        let reserved_namespaces = self
            .reserved_directory_namespace_paths
            .lock()
            .expect("reserved directory namespace lock poisoned")
            .clone();

        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| {
                SftpOpsError::Operation(format!("Failed to read directory entry: {e}"))
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            // Filter out . and ..
            if name == "." || name == ".." {
                continue;
            }
            if reserved_namespaces.contains_key(&entry.path()) {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path())
                .map_err(|e| SftpOpsError::Operation(format!("Failed to read metadata: {e}")))?;
            result.push(self.metadata_to_entry(name, &entry.path(), &meta));
        }

        Ok(result)
    }

    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        if let Some(hook) = &self.before_guarded_delete {
            hook(path);
        }
        #[cfg(test)]
        if self
            .fail_delete_matching
            .as_ref()
            .is_some_and(|marker| path.to_string_lossy().contains(marker))
            && (!self.fail_delete_matching_once
                || !self.delete_matching_failed.swap(true, Ordering::SeqCst))
        {
            return Err(SftpOpsError::Operation(format!(
                "injected cleanup failure for {}",
                path.display()
            )));
        }
        let local = self.to_local(path)?;
        let p = path.display();
        fs::remove_file(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to delete file {p}: {e}")))?;
        #[cfg(test)]
        if self
            .fail_delete_after_apply
            .as_ref()
            .is_some_and(|failure_path| failure_path == path)
        {
            return Err(SftpOpsError::Operation(
                "injected delete acknowledgement failure".to_string(),
            ));
        }
        self.release_persistent_artifact(path)
    }

    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError> {
        let local = self.to_local(path)?;
        let p = path.display();
        #[cfg(test)]
        if let Some(hook) = &self.before_guarded_delete {
            hook(path);
        }
        #[cfg(test)]
        if self.fail_recursive_delete_partially && path.to_string_lossy().contains("zaplex-source")
        {
            if let Some(entry) = fs::read_dir(&local)
                .map_err(|error| SftpOpsError::Operation(error.to_string()))?
                .next()
            {
                let entry = entry.map_err(|error| SftpOpsError::Operation(error.to_string()))?;
                let metadata = fs::symlink_metadata(entry.path())
                    .map_err(|error| SftpOpsError::Operation(error.to_string()))?;
                if metadata.is_dir() {
                    fs::remove_dir_all(entry.path())
                        .map_err(|error| SftpOpsError::Operation(error.to_string()))?;
                } else {
                    fs::remove_file(entry.path())
                        .map_err(|error| SftpOpsError::Operation(error.to_string()))?;
                }
            }
            return Err(SftpOpsError::Operation(format!(
                "injected partial recursive cleanup failure for {p}"
            )));
        }
        fs::remove_dir_all(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to recursively delete directory {p}: {e}"))
        })?;
        self.release_persistent_artifact(path)
    }

    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError> {
        let local = self.to_local(path)?;
        let p = path.display();
        fs::create_dir(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to create directory {p}: {e}")))?;
        #[cfg(test)]
        {
            let create_number = self.directory_creates.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_directory_create_after_apply == Some(create_number) {
                return Err(SftpOpsError::Operation(
                    "injected directory create acknowledgement failure".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn create_dir_with_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        let visible = self.to_local(path)?;
        self.persist_artifact_intent(path)?;
        let reservation_namespace = self.directory_reservation_namespace(&visible)?;
        self.validate_directory_reservation_namespace(&reservation_namespace)?;
        let reservation_name = uuid::Uuid::new_v4().to_string();
        let private = reservation_namespace.path.join(&reservation_name);
        let marker = reservation_namespace.path.join(format!(
            "{reservation_name}{DIRECTORY_RESERVATION_MARKER_SUFFIX}"
        ));
        self.validate_directory_reservation_namespace(&reservation_namespace)?;
        let file = match create_local_directory_with_anchor(
            &private,
            #[cfg(test)]
            self.after_directory_create_before_anchor.as_ref(),
        ) {
            Ok(file) => file,
            Err(error) => {
                let recovery_paths = self.register_failed_directory_reservation_candidates(
                    &private,
                    error.reserved_identity.as_ref(),
                )?;
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Opening protected directory reservation failed for {}: {}",
                        path.display(),
                        error.source
                    ),
                    recovery_id: None,
                    paths: recovery_paths,
                    committed: false,
                });
            }
        };
        let anchored_file = file.try_clone()?;
        let physical_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file,
            root: PathBuf::from("/"),
            opaque_paths: None,
        });
        let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: anchored_file,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        });
        let private_identity = match physical_anchor.identity() {
            Ok(identity) => identity,
            Err(error) => {
                let recovery_path = self.register_directory_reservation_recovery(
                    &private,
                    None,
                    None,
                    Some(anchor),
                )?;
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Identifying protected directory reservation failed for {}: {error}",
                        path.display()
                    ),
                    recovery_id: None,
                    paths: vec![recovery_path],
                    committed: false,
                });
            }
        };
        #[cfg(test)]
        if let Some(hook) = &self.after_directory_anchor_before_publish {
            hook(&private);
        }
        if !physical_anchor.matches_path(&private)? {
            let recovery_path =
                self.register_directory_reservation_recovery(&private, None, None, None)?;
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Directory reservation changed before ownership could be anchored for {}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: false,
            });
        }
        #[cfg(test)]
        if self.directory_reservation_failure == Some(DirectoryReservationFailure::Open) {
            let recovery_path = self.register_directory_reservation_recovery(
                &private,
                None,
                Some(private_identity),
                Some(anchor),
            )?;
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected directory reservation open failure".to_string(),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: false,
            });
        }
        let registry = self
            .directory_reservation_registry
            .as_ref()
            .ok_or_else(|| {
                SftpOpsError::Operation(
                    "Trusted directory reservation registry is unavailable".to_string(),
                )
            })?;
        let mut marker_options = fs::OpenOptions::new();
        marker_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            marker_options.mode(0o600);
        }
        let mut marker_file = match marker_options.open(&marker) {
            Ok(file) => file,
            Err(error) => {
                let recovery_path = self.register_directory_reservation_recovery(
                    &private,
                    None,
                    Some(private_identity),
                    Some(anchor),
                )?;
                return Err(SftpOpsError::RecoveryRequired {
                    message: format!(
                        "Failed to reserve directory ownership marker for {}: {error}",
                        path.display()
                    ),
                    recovery_id: None,
                    paths: vec![recovery_path],
                    committed: false,
                });
            }
        };
        let marker_payload = format!(
            "zaplex-owned-directory-v1\nnamespace={}\nreservation={reservation_name}\nobject={}",
            reservation_namespace.namespace_id, private_identity.object_id
        );
        if let Err(error) = marker_file
            .write_all(registry.marker_contents(&marker_payload).as_bytes())
            .and_then(|()| marker_file.sync_all())
        {
            let recovery_path = self.register_directory_reservation_recovery(
                &private,
                None,
                Some(private_identity),
                Some(anchor),
            )?;
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Writing directory ownership marker failed for {}: {error}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![recovery_path],
                committed: false,
            });
        }
        let marker_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: marker_file,
            root: PathBuf::from("/"),
            opaque_paths: None,
        });
        let marker_identity = marker_anchor.identity()?;
        let private_recovery_path = self.register_directory_reservation_recovery(
            &private,
            Some(OwnedReservationMarker {
                path: marker,
                identity: marker_identity,
                anchor: marker_anchor,
            }),
            Some(private_identity.clone()),
            Some(anchor.clone()),
        )?;
        #[cfg(test)]
        if self.directory_reservation_failure == Some(DirectoryReservationFailure::Identity) {
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected directory reservation identity failure".to_string(),
                recovery_id: None,
                paths: vec![private_recovery_path],
                committed: false,
            });
        }
        #[cfg(test)]
        if self.directory_reservation_failure == Some(DirectoryReservationFailure::Match) {
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected directory reservation match failure".to_string(),
                recovery_id: None,
                paths: vec![private_recovery_path],
                committed: false,
            });
        }
        if !anchor.matches_path(&private_recovery_path)? {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Directory reservation changed before publish for {}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![private_recovery_path],
                committed: false,
            });
        }
        self.validate_directory_reservation_namespace(&reservation_namespace)?;
        #[cfg(test)]
        {
            let create_number = self.directory_creates.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_directory_create_after_apply == Some(create_number) {
                return Err(SftpOpsError::RecoveryRequired {
                    message: "injected directory create acknowledgement failure".to_string(),
                    recovery_id: None,
                    paths: vec![private_recovery_path],
                    committed: false,
                });
            }
        }
        #[cfg(test)]
        let publish_result =
            if self.directory_reservation_failure == Some(DirectoryReservationFailure::Publish) {
                Err(std::io::Error::other(
                    "injected directory reservation publish failure",
                ))
            } else {
                rename_noreplace(&private, &visible)
            };
        #[cfg(not(test))]
        let publish_result = rename_noreplace(&private, &visible);
        if let Err(error) = publish_result {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Publishing reserved directory failed for {}: {error}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![private_recovery_path, path.to_path_buf()],
                committed: false,
            });
        }
        if !anchor.matches_path(path)? {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Published directory ownership is indeterminate for {}",
                    path.display()
                ),
                recovery_id: None,
                paths: vec![private_recovery_path, path.to_path_buf()],
                committed: false,
            });
        }
        self.persist_artifact_identity(path, anchor.clone())?;
        self.release_cleanup_recovery_path(&private_recovery_path)?;
        Ok(Some(anchor))
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let old_local = self.to_local(old_path)?;
        let new_local = self.to_local(new_path)?;
        let new_is_artifact = self.persist_artifact_intent(new_path)?;
        let moved_anchor = if new_is_artifact {
            self.existing_entry_ownership_anchor(old_path)?
        } else {
            None
        };
        if let Some(anchor) = moved_anchor.as_ref() {
            self.persist_artifact_moving_identity(new_path, anchor.clone())?;
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_rename {
            hook(&new_local);
        }
        rename_noreplace(&old_local, &new_local).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                return SftpOpsError::Operation(format!("{} already exists", new_path.display()));
            }
            SftpOpsError::Operation(format!(
                "Failed to rename {} -> {}: {e}",
                old_path.display(),
                new_path.display()
            ))
        })?;
        #[cfg(test)]
        if let Some(hook) = &self.after_rename {
            hook(&old_local, &new_local);
        }
        #[cfg(test)]
        if self.fail_rename_after_apply.as_deref() == Some(old_path) {
            return Err(SftpOpsError::Operation(
                "injected rename acknowledgement failure".to_string(),
            ));
        }
        if let Some(anchor) = moved_anchor {
            self.persist_artifact_identity(new_path, anchor)?;
        }
        self.release_persistent_artifact(old_path)?;
        Ok(())
    }

    fn rename_if_matches(
        &self,
        old_path: &Path,
        new_path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let old_local = self.to_local(old_path)?;
        let new_local = self.to_local(new_path)?;
        let new_is_artifact = self.persist_artifact_intent(new_path)?;
        let source_identity = anchor.identity()?;
        let placeholder_file = match source_identity.file_type {
            FileEntryType::File => open_confined_new_file(&self.root, new_path)?,
            FileEntryType::Directory => create_local_directory_with_anchor(
                &new_local,
                #[cfg(test)]
                None,
            )
            .map_err(|error| {
                SftpOpsError::Operation(format!(
                    "Failed to create guarded isolation placeholder at {}: {}",
                    new_path.display(),
                    error.source
                ))
            })?,
            FileEntryType::Symlink | FileEntryType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Identity-bound isolation is unsupported for {:?} at {}",
                    source_identity.file_type,
                    old_path.display()
                )));
            }
        };
        let placeholder_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: placeholder_file,
            root: self.root.clone(),
            opaque_paths: None,
        });
        let placeholder_identity = placeholder_anchor.identity()?;
        if new_is_artifact {
            self.persist_artifact_identity(new_path, placeholder_anchor.clone())?;
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_rename {
            hook(&new_local);
        }
        if !anchor.matches_path(old_path)? {
            self.cleanup_isolation_placeholder(
                new_path,
                placeholder_anchor,
                &placeholder_identity,
            )?;
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Source ownership changed at the identity-bound rename boundary for {}",
                    old_path.display()
                ),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: false,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.after_guarded_rename_check_before_mutation {
            hook(&old_local, &new_local);
        }
        let exchange_error = replace_atomic_local(&old_local, &new_local).err();
        #[cfg(test)]
        if let Some(hook) = &self.after_guarded_exchange_before_classification {
            hook(&old_local, &new_local);
        }
        let isolated_at_mutation =
            anchor.matches_path(new_path)? && placeholder_anchor.matches_path(old_path)?;
        if !isolated_at_mutation {
            #[cfg(test)]
            if let Some(hook) = &self.after_rename {
                hook(&old_local, &new_local);
            }
            let placeholder_at_source = placeholder_anchor.matches_path(old_path)?;
            #[cfg(test)]
            if let Some(hook) = &self.before_guarded_rename_restore {
                hook(&old_local, &new_local);
            }
            let restored = if placeholder_at_source
                && placeholder_anchor.matches_path(old_path)?
                && replace_atomic_local(&old_local, &new_local).is_ok()
            {
                placeholder_anchor.matches_path(new_path)?
            } else {
                false
            };
            self.persist_unresolved_diagnostic(
                "guarded-isolation-source",
                &old_path.display().to_string(),
            );
            self.persist_unresolved_diagnostic(
                "guarded-isolation-quarantine",
                &new_path.display().to_string(),
            );
            let mut recovery_paths = vec![old_path.to_path_buf(), new_path.to_path_buf()];
            if restored {
                match placeholder_identity.file_type {
                    FileEntryType::File => {
                        let empty_digest = format!("{:x}", Sha256::digest([]));
                        let _ = self.delete_file_if_matches(
                            new_path,
                            &placeholder_identity,
                            &empty_digest,
                        );
                    }
                    FileEntryType::Directory => {
                        let _ = self.delete_empty_dir_if_matches(new_path, &placeholder_identity);
                    }
                    FileEntryType::Symlink | FileEntryType::Other => {}
                }
            } else {
                recovery_paths.extend(self.persist_anchor_sibling_recovery(
                    new_path,
                    anchor.clone(),
                    &source_identity,
                    "owned-isolation-source",
                )?);
                recovery_paths.extend(self.persist_anchor_sibling_recovery(
                    old_path,
                    placeholder_anchor.clone(),
                    &placeholder_identity,
                    "owned-isolation-placeholder",
                )?);
            }
            recovery_paths.sort();
            recovery_paths.dedup();
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Identity-bound exchange isolation is indeterminate for {} -> {}",
                    old_path.display(),
                    new_path.display()
                ),
                recovery_id: None,
                paths: recovery_paths,
                committed: false,
            });
        }
        if new_is_artifact {
            let role = Self::persistent_artifact_role(new_path).ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Isolation target is not a persistent artifact: {}",
                    new_path.display()
                ))
            })?;
            self.transition_persistent_artifact_identity(new_path, role, anchor.clone())?;
        }
        self.cleanup_isolation_placeholder(old_path, placeholder_anchor, &placeholder_identity)?;
        #[cfg(test)]
        if let Some(hook) = &self.after_rename {
            hook(&old_local, &new_local);
        }
        if !anchor.matches_path(new_path)? {
            self.persist_unresolved_diagnostic(
                "guarded-isolation-source",
                &old_path.display().to_string(),
            );
            self.persist_unresolved_diagnostic(
                "guarded-isolation-quarantine",
                &new_path.display().to_string(),
            );
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Identity-bound isolation changed after commit for {} -> {}",
                    old_path.display(),
                    new_path.display()
                ),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: false,
            });
        }
        #[cfg(test)]
        if self.fail_rename_after_apply.as_deref() == Some(old_path) {
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected identity-bound rename acknowledgement failure".to_string(),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: false,
            });
        }
        if let Some(error) = exchange_error {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Identity-bound exchange applied but acknowledgement failed for {} -> {}: {error}",
                    old_path.display(),
                    new_path.display()
                ),
                recovery_id: None,
                paths: vec![new_path.to_path_buf()],
                committed: false,
            });
        }
        self.release_persistent_artifact(old_path)?;
        Ok(())
    }

    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let old_local = self.to_local(old_path)?;
        let new_local = self.to_local(new_path)?;
        let original_anchor = self
            .existing_entry_ownership_anchor(old_path)?
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Atomic exchange source cannot be anchored at {}",
                    old_path.display()
                ))
            })?;
        let displaced_anchor =
            self.existing_entry_ownership_anchor(new_path)?
                .ok_or_else(|| {
                    SftpOpsError::Operation(format!(
                        "Atomic exchange destination cannot be anchored at {}",
                        new_path.display()
                    ))
                })?;
        let transitioned =
            self.prepare_exchange_artifact_record(old_path, displaced_anchor.clone())?;
        #[cfg(test)]
        if let Some(hook) = &self.before_replace {
            hook(new_path);
        }
        let original_matches = original_anchor.matches_path(old_path)?;
        let displaced_matches = displaced_anchor.matches_path(new_path)?;
        if !original_matches || !displaced_matches {
            if original_matches {
                if let Some(transitioned) = &transitioned {
                    self.restore_exchange_artifact_record(old_path, transitioned, original_anchor)?;
                }
            } else {
                self.persist_unresolved_diagnostic(
                    "exchange-source-state",
                    &old_path.display().to_string(),
                );
                self.persist_unresolved_diagnostic(
                    "exchange-target-state",
                    &new_path.display().to_string(),
                );
            }
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Atomic exchange ownership changed immediately before mutation at {}",
                    new_path.display()
                ),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: false,
            });
        }
        let exchange_error = replace_atomic_local(&old_local, &new_local).err();
        #[cfg(test)]
        if let Some(hook) = &self.after_replace {
            hook(new_path);
        }
        let applied =
            original_anchor.matches_path(new_path)? && displaced_anchor.matches_path(old_path)?;
        let not_applied =
            original_anchor.matches_path(old_path)? && displaced_anchor.matches_path(new_path)?;
        if applied {
            // The displaced anchor was published with the durable transition
            // while the artifact lifecycle lock was still held.
        } else if not_applied {
            if let Some(transitioned) = &transitioned {
                self.restore_exchange_artifact_record(old_path, transitioned, original_anchor)?;
            }
            return Err(match exchange_error {
                Some(error) => SftpOpsError::Operation(format!(
                    "Atomic exchange failed for {} -> {}: {error}",
                    old_path.display(),
                    new_path.display()
                )),
                None => SftpOpsError::Operation(format!(
                    "Atomic exchange was not committed for {} -> {}",
                    old_path.display(),
                    new_path.display()
                )),
            });
        } else {
            return Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Atomic exchange outcome is indeterminate for {} -> {}",
                    old_path.display(),
                    new_path.display()
                ),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: false,
            });
        }
        #[cfg(test)]
        if self.fail_replace_after_apply.as_deref() == Some(new_path) {
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected replace acknowledgement failure".to_string(),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: true,
            });
        }
        match exchange_error {
            Some(error) => Err(SftpOpsError::RecoveryRequired {
                message: format!(
                    "Atomic exchange applied but acknowledgement failed for {} -> {}: {error}",
                    old_path.display(),
                    new_path.display()
                ),
                recovery_id: None,
                paths: vec![old_path.to_path_buf(), new_path.to_path_buf()],
                committed: true,
            }),
            None => Ok(()),
        }
    }

    fn delete_file_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
        expected_sha256: &str,
    ) -> Result<(), SftpOpsError> {
        #[cfg(unix)]
        let anchor = open_local_cleanup_anchor(&self.to_local(path)?)?;
        #[cfg(unix)]
        if anchored_object_id(&anchor)? != expected.object_id {
            return Err(SftpOpsError::Operation(format!(
                "Cleanup file identity changed before isolation at {}",
                path.display()
            )));
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_guarded_delete {
            hook(path);
        }
        let tombstone = path.with_file_name(format!(
            ".{}.zaplex-delete-{}",
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            uuid::Uuid::new_v4()
        ));
        self.map_opaque_cleanup_sibling(path, &tombstone)?;
        let local_tombstone = self.to_local(&tombstone)?;
        let rename_error = self.rename(path, &tombstone).err();
        if let Err(error) = isolate_cleanup_entry(self, path, &tombstone, expected, rename_error) {
            return Err(self.restore_cleanup_isolation_noreplace(path, &tombstone, error));
        }

        let actual = self.stable_identity(&tombstone);
        #[cfg(unix)]
        let anchored_matches = anchored_object_id(&anchor)
            .ok()
            .zip(
                actual
                    .as_ref()
                    .ok()
                    .map(|identity| identity.object_id.as_str()),
            )
            .is_some_and(|(anchor, actual)| anchor == actual);
        #[cfg(not(unix))]
        let anchored_matches = true;
        let actual_digest = (|| {
            let mut file = fs::File::open(&local_tombstone)?;
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            Ok::<String, SftpOpsError>(format!("{:x}", digest.finalize()))
        })();
        let matches = anchored_matches
            && actual.as_ref().is_ok_and(|actual| {
                actual.file_type == FileEntryType::File
                    && actual.size == expected.size
                    && actual.object_id == expected.object_id
            })
            && actual_digest
                .as_ref()
                .is_ok_and(|digest| digest == expected_sha256);
        if !matches {
            let primary = SftpOpsError::Operation(format!(
                "Cleanup file changed before deletion at {}",
                path.display()
            ));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }

        #[cfg(test)]
        if let Some(hook) = &self.after_guarded_cleanup_verification {
            hook(&local_tombstone);
        }
        #[cfg(unix)]
        if anchored_link_count(&anchor)? != 1 {
            let primary = SftpOpsError::Operation(format!(
                "Cleanup file gained another hardlink before private isolation at {}",
                path.display()
            ));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }

        #[cfg(test)]
        if self
            .fail_delete_matching
            .as_ref()
            .is_some_and(|marker| path.to_string_lossy().contains(marker))
            && (!self.fail_delete_matching_once
                || !self.delete_matching_failed.swap(true, Ordering::SeqCst))
        {
            let primary =
                SftpOpsError::Operation(format!("injected cleanup failure for {}", path.display()));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }

        #[cfg(test)]
        if self.fail_isolated_delete_before_apply {
            let primary = SftpOpsError::Operation(format!(
                "Injected isolated file delete failure for {}",
                tombstone.display()
            ));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }
        #[cfg(unix)]
        let cleanup_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: anchor.try_clone()?,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        });
        #[cfg(not(unix))]
        return Err(SftpOpsError::Operation(format!(
            "Private cleanup isolation is unsupported for {}",
            tombstone.display()
        )));
        #[cfg(unix)]
        self.cleanup_isolation_placeholder(&tombstone, cleanup_anchor, expected)?;
        self.release_cleanup_recovery_path(&tombstone)?;
        self.release_persistent_artifact(path)?;
        #[cfg(test)]
        if self.fail_recursive_delete_partially && path.to_string_lossy().contains("zaplex-source")
        {
            return Err(SftpOpsError::Operation(format!(
                "injected partial guarded cleanup failure for {}",
                path.display()
            )));
        }
        #[cfg(test)]
        if self
            .fail_delete_after_apply
            .as_deref()
            .is_some_and(|failure| {
                failure == path
                    || (failure.file_name().is_some_and(|expected_name| {
                        path.file_name().is_some_and(|name| {
                            let name = name.to_string_lossy();
                            name.contains(&expected_name.to_string_lossy().to_string())
                                && name.contains("zaplex-source")
                        })
                    }))
            })
        {
            return Err(SftpOpsError::Committed(
                "injected delete acknowledgement failure".to_string(),
            ));
        }
        Ok(())
    }

    fn delete_empty_dir_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
    ) -> Result<(), SftpOpsError> {
        #[cfg(unix)]
        let anchor = open_local_cleanup_anchor(&self.to_local(path)?)?;
        #[cfg(unix)]
        if anchored_object_id(&anchor)? != expected.object_id {
            return Err(SftpOpsError::Operation(format!(
                "Cleanup directory identity changed before isolation at {}",
                path.display()
            )));
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_guarded_delete {
            hook(path);
        }
        let tombstone = path.with_file_name(format!(
            ".{}.zaplex-delete-{}",
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            uuid::Uuid::new_v4()
        ));
        self.map_opaque_cleanup_sibling(path, &tombstone)?;
        let local_tombstone = self.to_local(&tombstone)?;
        let rename_error = self.rename(path, &tombstone).err();
        if let Err(error) = isolate_cleanup_entry(self, path, &tombstone, expected, rename_error) {
            return Err(self.restore_cleanup_isolation_noreplace(path, &tombstone, error));
        }
        let empty = fs::read_dir(&local_tombstone)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        let actual = self.stable_identity(&tombstone);
        #[cfg(unix)]
        let anchored_matches = anchored_object_id(&anchor)
            .ok()
            .zip(
                actual
                    .as_ref()
                    .ok()
                    .map(|identity| identity.object_id.as_str()),
            )
            .is_some_and(|(anchor, actual)| anchor == actual);
        #[cfg(not(unix))]
        let anchored_matches = true;
        let matches = anchored_matches
            && empty
            && actual.as_ref().is_ok_and(|actual| {
                actual.file_type == FileEntryType::Directory
                    && actual.object_id == expected.object_id
            });
        if !matches {
            let primary = SftpOpsError::Operation(format!(
                "Cleanup directory changed before deletion at {}",
                path.display()
            ));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }
        #[cfg(test)]
        if let Some(hook) = &self.after_guarded_cleanup_verification {
            hook(&local_tombstone);
        }
        #[cfg(test)]
        if self.fail_isolated_delete_before_apply {
            let primary = SftpOpsError::Operation(format!(
                "Injected isolated directory delete failure for {}",
                tombstone.display()
            ));
            return Err(restore_isolated_cleanup_entry(
                self, path, &tombstone, &primary,
            ));
        }
        #[cfg(unix)]
        let cleanup_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
            file: anchor.try_clone()?,
            root: self.root.clone(),
            opaque_paths: Some(self.opaque_recovery_paths.clone()),
        });
        #[cfg(not(unix))]
        return Err(SftpOpsError::Operation(format!(
            "Private cleanup isolation is unsupported for {}",
            tombstone.display()
        )));
        #[cfg(unix)]
        self.cleanup_isolation_placeholder(&tombstone, cleanup_anchor, expected)?;
        self.release_cleanup_recovery_path(&tombstone)?;
        self.release_persistent_artifact(path)?;
        Ok(())
    }

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        let local = self.to_local(path)?;
        let p = path.display();
        let canonical = dunce::canonicalize(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to resolve path {p}: {e}")))?;
        self.validate_resolved_local_path(&canonical, path)?;
        Ok(self.to_remote(&canonical))
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let local = self.to_local(path)?;
        let p = path.display();
        // Match the live SFTP backend: `stat` follows links, while `list_dir`
        // deliberately uses `symlink_metadata` (lstat semantics). Keeping
        // those operations distinct lets activation discover the target type
        // without ever making delete/copy recurse through a directory link.
        let meta = fs::metadata(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to get file info {p}: {e}")))?;
        self.validate_resolved_local_path(&local, path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(self.metadata_to_entry(name, &local, &meta))
    }

    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        #[cfg(test)]
        if self.forced_lstat_error.as_deref() == Some(path) {
            return Err(SftpOpsError::Operation(format!(
                "injected lstat failure for {}",
                path.display()
            )));
        }
        let local = self.to_local(path)?;
        let p = path.display();
        let meta = fs::symlink_metadata(&local).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SftpOpsError::NotFound(p.to_string())
            } else {
                SftpOpsError::Operation(format!("Failed to get file info {p}: {error}"))
            }
        })?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(self.metadata_to_entry(name, &local, &meta))
    }

    fn modification_time(
        &self,
        path: &Path,
    ) -> Result<Option<std::time::SystemTime>, SftpOpsError> {
        let local = self.to_local(path)?;
        let metadata = fs::symlink_metadata(local).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SftpOpsError::NotFound(path.display().to_string())
            } else {
                SftpOpsError::Operation(format!(
                    "Failed to get file info {}: {error}",
                    path.display()
                ))
            }
        })?;
        Ok(metadata.modified().ok())
    }

    fn stable_identity(&self, path: &Path) -> Result<StableEntryIdentity, SftpOpsError> {
        #[cfg(test)]
        if self.fail_staged_identity
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".zaplex-transfer-"))
        {
            return Err(SftpOpsError::Operation(
                "injected staged-target verification failure".to_string(),
            ));
        }
        #[cfg(test)]
        if self
            .fail_published_identity
            .as_ref()
            .is_some_and(|failure_path| failure_path == path)
            && self.published_identity_calls.fetch_add(1, Ordering::SeqCst) == 2
        {
            return Err(SftpOpsError::Operation(
                "injected published-target verification failure".to_string(),
            ));
        }
        let local = self.to_local(path)?;
        let metadata = fs::symlink_metadata(&local).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Failed to get stable identity for {}: {error}",
                path.display()
            ))
        })?;
        let identity = stable_identity_from_local_metadata(&metadata);
        #[cfg(test)]
        if let Some(hook) = &self.after_stable_identity {
            hook(path);
        }
        Ok(identity)
    }

    fn open_file_reader(&self, path: &Path) -> Result<Box<dyn BackendFileReader>, SftpOpsError> {
        Ok(Box::new(LocalFileReader(self.open_confined_file(path)?)))
    }

    fn create_file_writer(&self, path: &Path) -> Result<Box<dyn BackendFileWriter>, SftpOpsError> {
        self.persist_artifact_intent(path)?;
        #[cfg(test)]
        if let Some(hook) = &self.after_writer_validation_before_open {
            let local = self.to_local(path)?;
            hook(&local);
        }
        let file = open_confined_new_file(&self.root, path)?;
        let anchor = local_ownership_anchor(&file, &self.root)?;
        self.persist_artifact_identity(path, anchor)?;
        #[cfg(test)]
        {
            let create_number = self.writer_creates.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_writer_create_after_apply == Some(create_number) {
                drop(file);
                return Err(SftpOpsError::Operation(
                    "injected file create acknowledgement failure".to_string(),
                ));
            }
            if self.fail_writer_on_create == Some(create_number) {
                return Ok(Box::new(FailingFileWriter {
                    file,
                    root: self.root.clone(),
                }));
            }
            if self.corrupt_writer_on_create == Some(create_number) {
                return Ok(Box::new(CorruptingFileWriter {
                    file,
                    root: self.root.clone(),
                    corrupted: false,
                }));
            }
        }
        Ok(Box::new(LocalFileWriter {
            file,
            root: self.root.clone(),
        }))
    }

    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let dest = self.to_local(remote_path)?;
        copy_into_place(local_path, &dest, progress_cb, cancel_flag)
    }

    fn download_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let total = self.lstat(remote_path)?.size;
        let mut reader = self.open_file_reader(remote_path)?;
        copy_reader_into_place(
            &mut *reader,
            total,
            local_path,
            progress_cb,
            cancel_flag,
            true,
        )
    }

    fn upload_file_no_replace(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let dest = self.to_local(remote_path)?;
        copy_into_place_no_replace(local_path, &dest, progress_cb, cancel_flag)
    }

    fn download_file_no_replace(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let total = self.lstat(remote_path)?.size;
        let mut reader = self.open_file_reader(remote_path)?;
        copy_reader_into_place(
            &mut *reader,
            total,
            local_path,
            progress_cb,
            cancel_flag,
            false,
        )
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        validate_copy_destination(src, dst, false)?;
        let total = self.lstat(src)?.size;
        let mut reader = self.open_file_reader(src)?;
        let dst_local = self.to_local(dst)?;
        copy_reader_into_place(&mut *reader, total, &dst_local, None, None, true)
    }
}

/// Unique sibling temp path for an in-progress copy next to
/// the destination, so the finalizing rename stays on the same filesystem (and
/// is therefore atomic) and a leftover partial is obviously ours.
static COPY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_copy_temp_sequence(counter: &AtomicU64) -> Result<u64, SftpOpsError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| SftpOpsError::Operation("Copy temporary path ID exhausted".to_string()))
}

fn temp_sibling(dest: &Path) -> Result<PathBuf, SftpOpsError> {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let sequence = next_copy_temp_sequence(&COPY_TEMP_COUNTER)?;
    Ok(dest.with_file_name(format!(
        ".{name}.zaplex_partial-{}-{sequence}",
        std::process::id()
    )))
}

fn create_copy_temp(dest: &Path) -> Result<(PathBuf, fs::File), SftpOpsError> {
    const MAX_ATTEMPTS: usize = 128;

    for _ in 0..MAX_ATTEMPTS {
        let temp = temp_sibling(dest)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(SftpOpsError::LocalIo(format!(
                    "Failed to create temp file: {error}"
                )));
            }
        }
    }

    Err(SftpOpsError::LocalIo(format!(
        "Failed to reserve a temporary file for {}",
        dest.display()
    )))
}

/// Copy `src` onto `dest` the way the REMOTE backend already does: stream into
/// a sibling temp file, then rename it into place.
///
/// Writing straight to `dest` — what the local backend did until the RC audit —
/// destroys the existing file the moment the copy starts. An I/O error, a full
/// disk, a cancel, or copying a file onto itself left the user with a truncated
/// or half-written file and no way back. The closing rename is atomic on every
/// platform we ship, so `dest` holds either its old content or the complete new
/// one, never something in between.
fn copy_into_place(
    src: &Path,
    dest: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), SftpOpsError> {
    copy_into_place_with_mode(src, dest, progress_cb, cancel_flag, true)
}

fn copy_into_place_no_replace(
    src: &Path,
    dest: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), SftpOpsError> {
    copy_into_place_with_mode(src, dest, progress_cb, cancel_flag, false)
}

fn copy_reader_into_place(
    reader: &mut dyn BackendFileReader,
    total: u64,
    dest: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: Option<&AtomicBool>,
    overwrite_destination: bool,
) -> Result<(), SftpOpsError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            SftpOpsError::LocalIo(format!("Failed to create directory: {error}"))
        })?;
    }
    let (temp, mut temp_file) = create_copy_temp(dest)?;
    let result = (|| -> Result<(), SftpOpsError> {
        let mut buffer = [0_u8; 64 * 1024];
        let mut copied = 0_u64;
        loop {
            if cancel_flag.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                return Err(SftpOpsError::Cancelled);
            }
            let read = reader.read_chunk(&mut buffer)?;
            if read == 0 {
                break;
            }
            temp_file.write_all(&buffer[..read]).map_err(|error| {
                SftpOpsError::LocalIo(format!("Writing transfer temp file failed: {error}"))
            })?;
            copied += read as u64;
            if let Some(callback) = progress_cb {
                callback(copied, total);
            }
        }
        temp_file.flush()?;
        temp_file.sync_all()?;
        drop(temp_file);
        if overwrite_destination {
            fs::rename(&temp, dest)
        } else {
            publish_copy_without_replacement(&temp, dest)
        }
        .map_err(|error| {
            SftpOpsError::LocalIo(format!("Failed to finalize streamed copy: {error}"))
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn copy_into_place_with_mode(
    src: &Path,
    dest: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: Option<&AtomicBool>,
    overwrite_destination: bool,
) -> Result<(), SftpOpsError> {
    validate_copy_destination(src, dest, false)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to create directory: {e}")))?;
    }

    let total = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let (temp, mut temp_file) = create_copy_temp(dest)?;

    let result = (|| -> Result<(), SftpOpsError> {
        let mut src_file = fs::File::open(src)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to open source: {e}")))?;

        const CHUNK_SIZE: usize = 32 * 1024;
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut copied: u64 = 0;
        loop {
            if cancel_flag.is_some_and(|f| f.load(Ordering::SeqCst)) {
                return Err(SftpOpsError::Cancelled);
            }
            let n = src_file
                .read(&mut buf)
                .map_err(|e| SftpOpsError::LocalIo(format!("Read failed: {e}")))?;
            if n == 0 {
                break;
            }
            temp_file
                .write_all(&buf[..n])
                .map_err(|e| SftpOpsError::LocalIo(format!("Write failed: {e}")))?;
            copied += n as u64;
            if let Some(cb) = progress_cb {
                cb(copied, total);
            }
        }
        temp_file
            .flush()
            .map_err(|e| SftpOpsError::LocalIo(format!("Flush failed: {e}")))?;
        // Durable before the rename: a crash in between must not publish an
        // empty file over a good one.
        temp_file
            .sync_all()
            .map_err(|e| SftpOpsError::LocalIo(format!("Sync failed: {e}")))?;
        drop(temp_file);

        if overwrite_destination {
            fs::rename(&temp, dest)
                .map_err(|e| SftpOpsError::LocalIo(format!("Failed to finalize copy: {e}")))
        } else {
            publish_copy_without_replacement(&temp, dest).map_err(|e| {
                SftpOpsError::LocalIo(format!("Failed to finalize copy without replacement: {e}"))
            })
        }
    })();

    if result.is_err() {
        // `dest` still holds whatever it held before — only the partial goes.
        let _ = fs::remove_file(&temp);
    }
    result
}

fn publish_copy_without_replacement(temp: &Path, dest: &Path) -> std::io::Result<()> {
    publish_copy_without_replacement_with_cleanup(temp, dest, |path| fs::remove_file(path))
}

fn publish_copy_without_replacement_with_cleanup(
    temp: &Path,
    dest: &Path,
    cleanup_temp: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    fs::hard_link(temp, dest)?;
    if let Err(error) = cleanup_temp(temp) {
        log::warn!(
            "copy published at {} but temporary link {} could not be removed: {error}",
            dest.display(),
            temp.display()
        );
    }
    Ok(())
}

/// Convenience method for creating Arc<dyn SftpBackend>.
impl InMemorySftpBackend {
    /// Creates and wraps as Arc<dyn SftpBackend>.
    pub fn into_backend(self) -> Arc<dyn SftpBackend> {
        Arc::new(self)
    }
}

#[cfg(test)]
#[path = "sftp_backend_data_safety_tests.rs"]
mod data_safety_tests;

#[cfg(test)]
#[path = "sftp_backend_tests.rs"]
mod additional_tests;
