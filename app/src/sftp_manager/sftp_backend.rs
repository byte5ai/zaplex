//! SFTP backend operation abstraction layer.
//!
//! Defines the SftpBackend trait to decouple the UI layer from the protocol layer.
//! LiveSftpBackend delegates to a real SFTP connection, and InMemorySftpBackend uses the local filesystem for testing.
//! author: logic
//! date: 2026-05-30

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dunce;

use super::sftp_ops::{self, ProgressCallback, SftpOpsError};
use super::types::{FileEntry, FileEntryType};

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

#[cfg(not(windows))]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    fs::rename(old_path, new_path)
}

#[cfg(windows)]
fn replace_atomic_local(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let old_path: Vec<u16> = old_path.as_os_str().encode_wide().chain(iter::once(0)).collect();
    let new_path: Vec<u16> = new_path.as_os_str().encode_wide().chain(iter::once(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(old_path.as_ptr()),
            PCWSTR(new_path.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(std::io::Error::other)
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

/// SFTP backend operation abstraction to decouple the UI layer from the protocol layer.
pub trait SftpBackend: Send + Sync {
    /// Lists directory contents and returns a list of file entries.
    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError>;

    /// Deletes a remote file.
    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Recursively deletes a remote directory.
    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Creates a remote directory.
    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError>;

    /// Renames a remote file or directory.
    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError>;

    /// Atomically installs `old_path` over an existing `new_path`.
    ///
    /// The safe default refuses the operation. Backends must opt in only when
    /// they can preserve the old destination on every reported failure.
    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        Err(SftpOpsError::Operation(format!(
            "Atomic replacement is unsupported for {} -> {}",
            old_path.display(),
            new_path.display()
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

// ============================================================
// LiveSftpBackend — Delegates to real SFTP connection
// ============================================================

/// Real SFTP backend that wraps zap_sftp::Sftp.
pub struct LiveSftpBackend {
    sftp: zap_sftp::Sftp,
}

impl LiveSftpBackend {
    /// Creates a backend from an Sftp instance.
    pub fn new(sftp: zap_sftp::Sftp) -> Self {
        Self { sftp }
    }

    /// Gets a reference to the internal Sftp instance (used for realpath calls in connect_to_server).
    pub fn inner(&self) -> &zap_sftp::Sftp {
        &self.sftp
    }

    fn metadata_to_entry(path: &Path, metadata: zap_sftp::types::Metadata) -> FileEntry {
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
        }
    }
}

impl SftpBackend for LiveSftpBackend {
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

    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        sftp_ops::rename(&self.sftp, old_path, new_path)
    }

    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        sftp_ops::replace_atomic(&self.sftp, old_path, new_path)
    }

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        self.sftp.realpath(path).map_err(|e| SftpOpsError::Operation(e.to_string()))
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let metadata = self.sftp.stat(path)?;
        Ok(Self::metadata_to_entry(path, metadata))
    }

    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let metadata = self.sftp.lstat(path)?;
        Ok(Self::metadata_to_entry(path, metadata))
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
        // SFTP has no server-side copy, so round-trip the bytes through a local
        // temp file using the proven streaming primitives. Same-host only (the
        // caller guarantees source and destination share this session). Not the
        // most efficient path, but copy is a rare, non-hot operation and this
        // reuses fully-tested transfer code rather than a new protocol path.
        let tmp = unique_temp_path("zaplex-fmcopy");
        self.download_file(src, &tmp, None, None)?;
        let result = self.upload_file(&tmp, dst, None, None);
        let _ = fs::remove_file(&tmp);
        result
    }
}

/// A collision-free temp path (process id + monotonic counter; no wall clock so
/// it stays deterministic-friendly and needs no rng).
fn unique_temp_path(prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()))
}


// ============================================================
// InMemorySftpBackend — Local filesystem-based test implementation
// ============================================================

/// SFTP backend based on memory (local temp directory) for testing.
pub struct InMemorySftpBackend {
    /// Root directory that simulates the remote filesystem root.
    root: PathBuf,
    #[cfg(test)]
    before_rename: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
}

impl InMemorySftpBackend {
    /// Creates a new in-memory backend using the specified directory as root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            #[cfg(test)]
            before_rename: None,
        }
    }

    /// Gets the root directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    fn with_before_rename(mut self, hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        self.before_rename = Some(Arc::new(hook));
        self
    }

    /// Maps a "remote" path to a local absolute path.
    ///
    /// Remote paths start with /, and are mapped to relative paths under root.
    fn to_local(&self, remote_path: &Path) -> PathBuf {
        let relative = remote_path.strip_prefix("/").unwrap_or(remote_path);
        self.root.join(relative)
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
        }
    }
}

impl SftpBackend for InMemorySftpBackend {
    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        let entries = fs::read_dir(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to list directory {p}: {e}"))
        })?;

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
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| {
                SftpOpsError::Operation(format!("Failed to read metadata: {e}"))
            })?;
            result.push(self.metadata_to_entry(name, &entry.path(), &meta));
        }

        Ok(result)
    }

    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        fs::remove_file(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to delete file {p}: {e}"))
        })
    }

    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        fs::remove_dir_all(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to recursively delete directory {p}: {e}"))
        })
    }

    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        fs::create_dir(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to create directory {p}: {e}"))
        })
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let old_local = self.to_local(old_path);
        let new_local = self.to_local(new_path);
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
        })
    }

    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        let old_local = self.to_local(old_path);
        let new_local = self.to_local(new_path);
        replace_atomic_local(&old_local, &new_local).map_err(|error| {
            SftpOpsError::Operation(format!(
                "Failed to atomically replace {} -> {}: {error}",
                old_path.display(),
                new_path.display()
            ))
        })
    }

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        let canonical = dunce::canonicalize(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to resolve path {p}: {e}"))
        })?;
        Ok(self.to_remote(&canonical))
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        // Match the live SFTP backend: `stat` follows links, while `list_dir`
        // deliberately uses `symlink_metadata` (lstat semantics). Keeping
        // those operations distinct lets activation discover the target type
        // without ever making delete/copy recurse through a directory link.
        let meta = fs::metadata(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to get file info {p}: {e}")))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(self.metadata_to_entry(name, &local, &meta))
    }

    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let local = self.to_local(path);
        let p = path.display();
        let meta = fs::symlink_metadata(&local)
            .map_err(|e| SftpOpsError::Operation(format!("Failed to get file info {p}: {e}")))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(self.metadata_to_entry(name, &local, &meta))
    }

    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let dest = self.to_local(remote_path);
        copy_into_place(local_path, &dest, progress_cb, cancel_flag)
    }

    fn download_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let src = self.to_local(remote_path);
        copy_into_place(&src, local_path, progress_cb, cancel_flag)
    }

    fn upload_file_no_replace(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let dest = self.to_local(remote_path);
        copy_into_place_no_replace(local_path, &dest, progress_cb, cancel_flag)
    }

    fn download_file_no_replace(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        let src = self.to_local(remote_path);
        copy_into_place_no_replace(&src, local_path, progress_cb, cancel_flag)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        let src_local = self.to_local(src);
        let dst_local = self.to_local(dst);
        copy_into_place(&src_local, &dst_local, None, None)
    }
}

/// Unique sibling temp path for an in-progress copy next to
/// the destination, so the finalizing rename stays on the same filesystem (and
/// is therefore atomic) and a leftover partial is obviously ours.
static COPY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_sibling(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let sequence = COPY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dest.with_file_name(format!(
        ".{name}.zaplex_partial-{}-{sequence}",
        std::process::id()
    ))
}

fn create_copy_temp(dest: &Path) -> Result<(PathBuf, fs::File), SftpOpsError> {
    const MAX_ATTEMPTS: usize = 128;

    for _ in 0..MAX_ATTEMPTS {
        let temp = temp_sibling(dest);
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
                )))
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

fn copy_into_place_with_mode(
    src: &Path,
    dest: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: Option<&AtomicBool>,
    overwrite_destination: bool,
) -> Result<(), SftpOpsError> {
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
            fs::hard_link(&temp, dest)
                .and_then(|()| fs::remove_file(&temp))
                .map_err(|e| {
                    SftpOpsError::LocalIo(format!(
                        "Failed to finalize copy without replacement: {e}"
                    ))
                })
        }
    })();

    if result.is_err() {
        // `dest` still holds whatever it held before — only the partial goes.
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Convenience method for creating Arc<dyn SftpBackend>.
impl InMemorySftpBackend {
    /// Creates and wraps as Arc<dyn SftpBackend>.
    pub fn into_backend(self) -> Arc<dyn SftpBackend> {
        Arc::new(self)
    }
}

#[cfg(test)]
mod data_safety_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn backend() -> (InMemorySftpBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        (InMemorySftpBackend::new(dir.path().to_path_buf()), dir)
    }

    /// Renaming onto an existing name must refuse, not silently destroy it.
    /// `fs::rename` overwrites on Unix; the remote backend has always used
    /// `overwrite: false`, and this is the local path catching up (RC audit).
    #[test]
    fn local_rename_never_replaces_existing_destination() {
        let (be, dir) = backend();
        fs::write(dir.path().join("victim.txt"), b"PRECIOUS").unwrap();
        fs::write(dir.path().join("source.txt"), b"new").unwrap();

        let err = be
            .rename(Path::new("/source.txt"), Path::new("/victim.txt"))
            .expect_err("renaming onto an existing file must fail");
        assert!(
            matches!(err, SftpOpsError::Operation(ref m) if m.contains("already exists")),
            "expected an already-exists conflict, got {err:?}"
        );
        assert_eq!(
            fs::read(dir.path().join("victim.txt")).unwrap(),
            b"PRECIOUS",
            "the existing file must be untouched"
        );
    }

    /// A destination created after the initial lookup is still a conflict.
    /// The commit itself must be no-replace; a check followed by plain rename
    /// loses this race on Unix.
    #[test]
    fn local_rename_never_replaces_concurrently_created_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("source.txt"), b"source").unwrap();
        let backend =
            InMemorySftpBackend::new(dir.path().to_path_buf()).with_before_rename(|destination| {
                fs::write(destination, b"CONCURRENT").unwrap();
            });

        backend
            .rename(Path::new("/source.txt"), Path::new("/destination.txt"))
            .expect_err("an atomically created destination must block rename");

        assert_eq!(
            fs::read(dir.path().join("destination.txt")).unwrap(),
            b"CONCURRENT",
            "rename must not replace the destination created in the race window"
        );
        assert!(
            dir.path().join("source.txt").exists(),
            "failed rename must preserve its source"
        );
    }

    /// Exercise the concrete platform replace primitive on a failure that the
    /// OS itself reports: a regular file cannot replace a non-empty directory.
    #[test]
    fn local_atomic_replace_failure_preserves_source_and_destination() {
        let (be, dir) = backend();
        fs::write(dir.path().join("source.txt"), b"replacement").unwrap();
        fs::create_dir(dir.path().join("destination")).unwrap();
        fs::write(dir.path().join("destination/precious.txt"), b"PRECIOUS").unwrap();

        be.replace(Path::new("/source.txt"), Path::new("/destination"))
            .expect_err("the platform replace primitive must reject a type mismatch");

        assert_eq!(
            fs::read(dir.path().join("source.txt")).unwrap(),
            b"replacement",
            "a failed atomic replace must preserve its source"
        );
        assert_eq!(
            fs::read(dir.path().join("destination/precious.txt")).unwrap(),
            b"PRECIOUS",
            "a failed atomic replace must preserve the existing destination"
        );
    }

    /// A cancelled copy must leave the destination exactly as it was — the old
    /// code wrote straight into it, so a cancel truncated a good file.
    #[test]
    fn local_copy_failure_keeps_existing_destination_intact() {
        let (be, dir) = backend();
        let dest = dir.path().join("dest.bin");
        fs::write(&dest, b"ORIGINAL-CONTENT").unwrap();
        // Source big enough that the cancel is observed inside the copy loop.
        fs::write(dir.path().join("src.bin"), vec![b'x'; 512 * 1024]).unwrap();

        let cancel = AtomicBool::new(true);
        let err = be
            .upload_file(
                &dir.path().join("src.bin"),
                Path::new("/dest.bin"),
                None,
                Some(&cancel),
            )
            .expect_err("a pre-cancelled copy must fail");
        assert!(matches!(err, SftpOpsError::Cancelled), "got {err:?}");
        assert_eq!(
            fs::read(&dest).unwrap(),
            b"ORIGINAL-CONTENT",
            "a cancelled copy must not touch the destination"
        );
        // And it must not litter: no partial left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("zaplex_partial"))
            .collect();
        assert!(leftovers.is_empty(), "partial left behind: {leftovers:?}");
    }

    /// A successful copy still replaces the destination completely.
    #[test]
    fn successful_copy_replaces_the_destination() {
        let (be, dir) = backend();
        fs::write(dir.path().join("dest.bin"), b"OLD").unwrap();
        fs::write(dir.path().join("src.bin"), b"NEW-CONTENT").unwrap();

        be.upload_file(
            &dir.path().join("src.bin"),
            Path::new("/dest.bin"),
            None,
            None,
        )
        .expect("copy should succeed");
        assert_eq!(fs::read(dir.path().join("dest.bin")).unwrap(), b"NEW-CONTENT");
    }

    /// Independent transfers targeting the same final name must never share
    /// their in-progress file. Otherwise either transfer can truncate, rename
    /// or clean up the other transfer's bytes.
    #[test]
    fn concurrent_copies_reserve_distinct_temporary_paths() {
        let destination = Path::new("/tmp/destination.bin");

        let first = temp_sibling(destination);
        let second = temp_sibling(destination);

        assert_ne!(
            first, second,
            "each transfer needs an exclusive temporary sibling"
        );
    }

    #[test]
    fn local_no_replace_copy_preserves_existing_destination() {
        let (backend, dir) = backend();
        let source = dir.path().join("source.bin");
        fs::write(&source, b"NEW").unwrap();
        fs::write(dir.path().join("destination.bin"), b"EXISTING").unwrap();

        backend
            .upload_file_no_replace(
                &source,
                Path::new("/destination.bin"),
                None,
                None,
            )
            .expect_err("an unconfirmed copy must not replace its destination");

        assert_eq!(
            fs::read(dir.path().join("destination.bin")).unwrap(),
            b"EXISTING"
        );
    }

    /// Exclusive temp creation must reject symlink collisions instead of
    /// truncating the symlink target through `File::create`.
    #[cfg(unix)]
    #[test]
    fn local_copy_temp_creation_never_follows_existing_symlink() {
        use std::os::unix::fs::symlink;

        let (backend, dir) = backend();
        let source = dir.path().join("source.bin");
        let destination = dir.path().join("destination.bin");
        let victim = dir.path().join("victim.bin");
        fs::write(&source, b"NEW").unwrap();
        fs::write(&victim, b"PRECIOUS").unwrap();

        let first_sequence = COPY_TEMP_COUNTER.load(Ordering::Relaxed);
        for sequence in first_sequence..first_sequence + 32 {
            let candidate = destination.with_file_name(format!(
                ".destination.bin.zaplex_partial-{}-{sequence}",
                std::process::id()
            ));
            symlink(&victim, candidate).unwrap();
        }

        backend
            .upload_file(&source, Path::new("/destination.bin"), None, None)
            .expect("copy should retry after colliding with symlinks");

        assert_eq!(
            fs::read(&victim).unwrap(),
            b"PRECIOUS",
            "temporary-file creation must never follow a symlink"
        );
        assert_eq!(fs::read(destination).unwrap(), b"NEW");
    }

    #[cfg(unix)]
    #[test]
    fn stat_follows_a_symlink_to_a_file() {
        use std::os::unix::fs::symlink;

        let (be, dir) = backend();
        fs::write(dir.path().join("target.txt"), b"target").unwrap();
        symlink("target.txt", dir.path().join("link.txt")).unwrap();

        let entry = be
            .stat(Path::new("/link.txt"))
            .expect("a valid file symlink should resolve");

        assert_eq!(entry.file_type, FileEntryType::File);
        assert_eq!(entry.size, 6);
    }

    #[cfg(unix)]
    #[test]
    fn stat_follows_a_symlink_to_a_directory() {
        use std::os::unix::fs::symlink;

        let (be, dir) = backend();
        fs::create_dir(dir.path().join("target-dir")).unwrap();
        symlink("target-dir", dir.path().join("link-dir")).unwrap();

        let entry = be
            .stat(Path::new("/link-dir"))
            .expect("a valid directory symlink should resolve");

        assert_eq!(entry.file_type, FileEntryType::Directory);
    }

    #[cfg(unix)]
    #[test]
    fn stat_rejects_a_broken_symlink() {
        use std::os::unix::fs::symlink;

        let (be, dir) = backend();
        symlink("missing-target", dir.path().join("broken-link")).unwrap();

        assert_eq!(
            be.lstat(Path::new("/broken-link")).unwrap().file_type,
            FileEntryType::Symlink,
            "lstat must still see a broken link for overwrite/delete checks"
        );
        be.stat(Path::new("/broken-link"))
            .expect_err("a broken symlink must not masquerade as a usable entry");
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_directory_symlink_never_deletes_its_target() {
        use std::os::unix::fs::symlink;

        let (be, dir) = backend();
        fs::create_dir(dir.path().join("target-dir")).unwrap();
        fs::write(dir.path().join("target-dir/keep.txt"), b"keep").unwrap();
        symlink("target-dir", dir.path().join("link-dir")).unwrap();

        let listed = be.list_dir(Path::new("/")).unwrap();
        let link = listed
            .iter()
            .find(|entry| entry.name == "link-dir")
            .expect("the symlink should be listed");
        assert_eq!(
            link.file_type,
            FileEntryType::Symlink,
            "directory listings must retain lstat semantics for destructive decisions"
        );

        be.delete_file(Path::new("/link-dir"))
            .expect("deleting the link should succeed");

        assert_eq!(dir.path().join("link-dir").exists(), false);
        assert_eq!(
            fs::read(dir.path().join("target-dir/keep.txt")).unwrap(),
            b"keep",
            "deleting the symlink must not recurse into its directory target"
        );
    }
}
