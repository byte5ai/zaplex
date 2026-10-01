//! SFTP channel operations module
//!
//! Wraps ssh2::Sftp to provide a thread-safe remote filesystem operations interface,
//! including file open, directory read/write, rename, delete, etc.
//! author: logic
//! date: 2026-05-31

use std::io::{Read, Write};
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

    /// Replace the exact remote path using OpenSSH's POSIX rename extension.
    /// Servers without the extension fail before any mutation is requested.
    pub fn replace_atomically(&self, src: &Path, dst: &Path) -> Result<(), SftpError> {
        // ssh2 0.9's rename API only exposes standard SFTP rename; its flags do not
        // select posix-rename@openssh.com. Use a separate subsystem channel so the
        // extension cannot interfere with requests on the shared SFTP channel.
        let mut channel = self.session.channel_session()?;
        channel.subsystem("sftp")?;
        let result = posix_replace(&mut channel, src, dst);
        let _ = channel.close();
        result
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

// OpenSSH PROTOCOL, section "posix-rename@openssh.com": the extension invokes
// rename(oldpath, newpath), never mv's directory-target interpretation.
const POSIX_RENAME_EXTENSION: &[u8] = b"posix-rename@openssh.com";
const MAX_RENAME_PACKET: usize = 64 * 1024;

fn posix_replace(
    channel: &mut (impl Read + Write),
    src: &Path,
    dst: &Path,
) -> Result<(), SftpError> {
    let mut request = vec![200]; // SSH_FXP_EXTENDED
    request.extend_from_slice(&1_u32.to_be_bytes());
    append_sftp_string(&mut request, POSIX_RENAME_EXTENSION)?;
    for path in [src, dst] {
        let path = path
            .to_str()
            .filter(|path| !path.is_empty() && !path.contains('\0'))
            .ok_or_else(|| {
                SftpError::General(
                    "Atomic replacement requires a non-empty UTF-8 path without NUL".to_string(),
                )
            })?;
        append_sftp_string(&mut request, path.as_bytes())?;
    }

    write_sftp_packet(channel, &[1, 0, 0, 0, 3])?; // SSH_FXP_INIT, version 3
    let hello = read_sftp_packet(channel)?;
    let mut hello = hello.as_slice();
    if hello.first() != Some(&2) {
        // SSH_FXP_VERSION
        return Err(SftpError::General(
            "Invalid SFTP version response".to_string(),
        ));
    }
    hello = &hello[1..];
    if take_sftp_u32(&mut hello)? != 3 {
        return Err(SftpError::General(
            "Atomic replacement requires SFTP version 3".to_string(),
        ));
    }
    let mut supported = false;
    while !hello.is_empty() {
        let name = take_sftp_string(&mut hello)?;
        let version = take_sftp_string(&mut hello)?;
        if name == POSIX_RENAME_EXTENSION && version == b"1" {
            supported = true;
        }
    }
    if !supported {
        return Err(SftpError::General(
            "Server does not support posix-rename@openssh.com version 1".to_string(),
        ));
    }
    write_sftp_packet(channel, &request)?;
    let response = read_sftp_packet(channel)?;
    let mut response = response.as_slice();
    if response.first() != Some(&101) {
        // SSH_FXP_STATUS
        return Err(SftpError::General(
            "Invalid atomic rename response".to_string(),
        ));
    }
    response = &response[1..];
    if take_sftp_u32(&mut response)? != 1 {
        return Err(SftpError::General(
            "Atomic rename response ID does not match".to_string(),
        ));
    }
    let status = take_sftp_u32(&mut response)?;
    let message = take_sftp_string(&mut response)?;
    let _language = take_sftp_string(&mut response)?;
    if !response.is_empty() {
        return Err(SftpError::General(
            "Unexpected atomic rename response payload".to_string(),
        ));
    }
    if status != 0 {
        return Err(SftpError::General(format!(
            "Atomic remote rename failed with SFTP status {status}: {}",
            String::from_utf8_lossy(message)
        )));
    }
    Ok(())
}

fn append_sftp_string(packet: &mut Vec<u8>, bytes: &[u8]) -> Result<(), SftpError> {
    if bytes.len() > MAX_RENAME_PACKET || packet.len() + 4 + bytes.len() > MAX_RENAME_PACKET {
        return Err(SftpError::General(
            "Atomic rename request is too large".to_string(),
        ));
    }
    packet.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    packet.extend_from_slice(bytes);
    Ok(())
}

fn write_sftp_packet(channel: &mut impl Write, packet: &[u8]) -> Result<(), SftpError> {
    channel.write_all(&(packet.len() as u32).to_be_bytes())?;
    channel.write_all(packet)?;
    channel.flush()?;
    Ok(())
}

fn read_sftp_packet(channel: &mut impl Read) -> Result<Vec<u8>, SftpError> {
    let mut length = [0; 4];
    channel.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_RENAME_PACKET {
        return Err(SftpError::General(
            "Invalid SFTP response length".to_string(),
        ));
    }
    let mut packet = vec![0; length];
    channel.read_exact(&mut packet)?;
    Ok(packet)
}

fn take_sftp_u32(packet: &mut &[u8]) -> Result<u32, SftpError> {
    let mut value = [0; 4];
    packet.read_exact(&mut value)?;
    Ok(u32::from_be_bytes(value))
}

fn take_sftp_string<'a>(packet: &mut &'a [u8]) -> Result<&'a [u8], SftpError> {
    let length = take_sftp_u32(packet)? as usize;
    if length > packet.len() {
        return Err(SftpError::General(
            "Truncated SFTP response string".to_string(),
        ));
    }
    let (value, remainder) = packet.split_at(length);
    *packet = remainder;
    Ok(value)
}

#[cfg(test)]
#[path = "sftp_tests.rs"]
mod tests;
