//! SFTP channel operations module
//!
//! Wraps ssh2::Sftp to provide a thread-safe remote filesystem operations interface,
//! including file open, directory read/write, rename, delete, etc.
//! author: logic
//! date: 2026-05-31

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
}

impl Sftp {
    /// Create Sftp instance from ssh2::Sftp
    pub(crate) fn new(sftp: ssh2::Sftp) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sftp)),
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
