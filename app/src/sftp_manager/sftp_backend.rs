//! SFTP backend operation abstraction layer.
//!
//! Defines the SftpBackend trait to decouple the UI layer from the protocol layer.
//! LiveSftpBackend delegates to a real SFTP connection, and InMemorySftpBackend uses the local filesystem for testing.
//! author: logic
//! date: 2026-05-30

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dunce;

use super::sftp_ops::{self, ProgressCallback, SftpOpsError};
use super::types::{FileEntry, FileEntryType};

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

    /// Resolves the real path.
    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError>;

    /// Gets file/directory details.
    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError>;

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

    /// Copies a single file *within this backend* (same filesystem namespace),
    /// e.g. between two local file-manager panes or two panes on the same host.
    /// Cross-connection copy (local↔remote) is a separate transfer path.
    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError>;

    /// Recursively copies a directory within this backend. The default walks
    /// with `list_dir` + `create_dir` + `copy_file`; backends may override with
    /// a native recursive copy.
    fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        self.create_dir(dst)?;
        for entry in self.list_dir(src)? {
            let child_dst = dst.join(&entry.name);
            match entry.file_type {
                FileEntryType::Directory => self.copy_dir_recursive(&entry.path, &child_dst)?,
                _ => self.copy_file(&entry.path, &child_dst)?,
            }
        }
        Ok(())
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

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        self.sftp.realpath(path).map_err(|e| SftpOpsError::Operation(e.to_string()))
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        let metadata = self.sftp.stat(path)?;
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
        Ok(FileEntry {
            name,
            path: path.to_path_buf(),
            file_type,
            size: metadata.size,
            modified,
            permissions,
        })
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
}

impl InMemorySftpBackend {
    /// Creates a new in-memory backend using the specified directory as root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Gets the root directory path.
    pub fn root(&self) -> &Path {
        &self.root
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
        } else {
            FileEntryType::File
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
        // `fs::rename` silently replaces an existing target on Unix, so renaming
        // onto a name that already existed destroyed that file without a word.
        // The remote path has always refused this (`overwrite: false`); the
        // local one now does too, and the caller surfaces the conflict.
        if fs::symlink_metadata(&new_local).is_ok() {
            return Err(SftpOpsError::Operation(format!(
                "{} already exists",
                new_path.display()
            )));
        }
        fs::rename(&old_local, &new_local).map_err(|e| {
            SftpOpsError::Operation(format!(
                "Failed to rename {} -> {}: {e}",
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
        let meta = fs::symlink_metadata(&local).map_err(|e| {
            SftpOpsError::Operation(format!("Failed to get file info {p}: {e}"))
        })?;
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

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        let src_local = self.to_local(src);
        let dst_local = self.to_local(dst);
        copy_into_place(&src_local, &dst_local, None, None)
    }
}

/// Sibling temp path for an in-progress copy: `.<name>.zaplex_partial` next to
/// the destination, so the finalizing rename stays on the same filesystem (and
/// is therefore atomic) and a leftover partial is obviously ours.
fn temp_sibling(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    dest.with_file_name(format!(".{name}.zaplex_partial"))
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
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to create directory: {e}")))?;
    }

    let total = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let temp = temp_sibling(dest);

    let result = (|| -> Result<(), SftpOpsError> {
        let mut src_file = fs::File::open(src)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to open source: {e}")))?;
        let mut temp_file = fs::File::create(&temp)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to create temp file: {e}")))?;

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

        fs::rename(&temp, dest)
            .map_err(|e| SftpOpsError::LocalIo(format!("Failed to finalize copy: {e}")))
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
    fn rename_refuses_to_clobber_an_existing_file() {
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

    /// A cancelled copy must leave the destination exactly as it was — the old
    /// code wrote straight into it, so a cancel truncated a good file.
    #[test]
    fn cancelled_copy_leaves_the_destination_intact() {
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
}
