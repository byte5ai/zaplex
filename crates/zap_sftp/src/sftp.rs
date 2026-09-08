//! SFTP channel operations module
//!
//! Wraps ssh2::Sftp to provide a thread-safe remote filesystem operations interface,
//! including file open, directory read/write, rename, delete, etc.
//! author: logic
//! date: 2026-05-31

use std::borrow::Cow;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use crate::dir::Dir;
use crate::error::SftpError;
use crate::file::File;
use crate::types::{DirEntry, Metadata, OpenOptions, RenameOptions};

/// SFTP channel, entry point for all remote filesystem operations
#[derive(Clone)]
pub struct Sftp {
    inner: Arc<Mutex<ssh2::Sftp>>,
    session: Arc<ssh2::Session>,
}

impl Sftp {
    /// Create Sftp instance from ssh2::Sftp
    pub(crate) fn new(sftp: ssh2::Sftp, session: Arc<ssh2::Session>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sftp)),
            session,
        }
    }

    /// Open remote file
    pub fn open(&self, path: &Path, options: OpenOptions) -> Result<File, SftpError> {
        let sftp = self.inner.lock().unwrap();
        File::open(&sftp, path, &options)
    }

    /// Create directory
    pub fn create_dir(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.mkdir(path, 0o755)?;
        Ok(())
    }

    /// Remove directory (must be empty)
    pub fn remove_dir(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.rmdir(path)?;
        Ok(())
    }

    /// Remove file
    pub fn remove_file(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.unlink(path)?;
        Ok(())
    }

    /// Rename / move
    pub fn rename(&self, src: &Path, dst: &Path, opts: RenameOptions) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        let mut flags = ssh2::RenameFlags::empty();
        if opts.overwrite {
            flags |= ssh2::RenameFlags::OVERWRITE;
        }
        if opts.atomic {
            flags |= ssh2::RenameFlags::ATOMIC;
        }
        if opts.native {
            flags |= ssh2::RenameFlags::NATIVE;
        }
        sftp.rename(src, dst, Some(flags))?;
        Ok(())
    }

    /// Atomically replace a remote path through the authenticated SSH session.
    ///
    /// OpenSSH uses SFTP v3, whose standard rename request cannot replace an
    /// existing destination. The staged file is always a sibling of the
    /// destination, so POSIX `mv` publishes it with one same-filesystem rename.
    pub fn replace_atomically(&self, src: &Path, dst: &Path) -> Result<(), SftpError> {
        let command = atomic_replace_command(src, dst)?;
        let mut channel = self.session.channel_session().map_err(|error| {
            SftpError::General(format!(
                "Atomic replacement is unavailable because the SSH command channel could not be opened: {error}"
            ))
        })?;
        channel.exec(&command).map_err(|error| {
            SftpError::General(format!(
                "Atomic replacement is unavailable because /bin/mv could not be started: {error}"
            ))
        })?;

        let mut stdout = Vec::new();
        channel.read_to_end(&mut stdout)?;
        let mut stderr = Vec::new();
        channel.stderr().read_to_end(&mut stderr)?;
        channel.wait_close()?;
        let exit_status = channel.exit_status()?;
        if exit_status == 0 {
            return Ok(());
        }

        let diagnostic = String::from_utf8_lossy(&stderr);
        let diagnostic = diagnostic.trim();
        let detail = if diagnostic.is_empty() {
            String::new()
        } else {
            format!(": {diagnostic}")
        };
        Err(SftpError::General(format!(
            "Atomic replacement through /bin/mv failed with exit status {exit_status}{detail}"
        )))
    }

    /// Get file metadata (follow symlinks)
    pub fn stat(&self, path: &Path) -> Result<Metadata, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let stat = sftp.stat(path)?;
        Ok(Metadata::from_ssh2(stat))
    }

    /// Get file metadata (don't follow symlinks)
    pub fn lstat(&self, path: &Path) -> Result<Metadata, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let stat = sftp.lstat(path)?;
        Ok(Metadata::from_ssh2(stat))
    }

    /// Read directory contents
    pub fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, SftpError> {
        let sftp = self.inner.lock().unwrap();
        Dir::read_dir(&sftp, path)
    }

    /// Create symlink
    pub fn symlink(&self, src: &Path, dst: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.symlink(src, dst)?;
        Ok(())
    }

    /// Read symlink target
    pub fn readlink(&self, path: &Path) -> Result<PathBuf, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let target = sftp.readlink(path)?;
        Ok(target)
    }

    /// Resolve remote path to its real path
    pub fn realpath(&self, path: &Path) -> Result<PathBuf, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let real = sftp.realpath(path)?;
        Ok(real)
    }
}

fn atomic_replace_command(src: &Path, dst: &Path) -> Result<String, SftpError> {
    let src = atomic_replace_operand(src)?;
    let dst = atomic_replace_operand(dst)?;
    let script = shell_escape::unix::escape(Cow::Borrowed(
        r#"if [ -d "$2" ]; then echo "destination is a directory" >&2; exit 73; fi; exec /bin/mv -f "$1" "$2""#,
    ));
    Ok(format!(
        "/bin/sh -c {script} zaplex-atomic-replace {src} {dst}"
    ))
}

fn atomic_replace_operand(path: &Path) -> Result<String, SftpError> {
    let path = path.to_str().ok_or_else(|| {
        SftpError::General("Atomic replacement requires a UTF-8 remote path".to_string())
    })?;
    if path.is_empty() {
        return Err(SftpError::General(
            "Atomic replacement requires a non-empty remote path".to_string(),
        ));
    }

    let operand = if path.starts_with('/') || path.starts_with("./") || path.starts_with("../") {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(format!("./{path}"))
    };
    Ok(shell_escape::unix::escape(operand).into_owned())
}

#[cfg(test)]
#[path = "sftp_tests.rs"]
mod tests;
