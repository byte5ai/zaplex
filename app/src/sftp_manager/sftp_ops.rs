//! SFTP operations wrapper layer
//!
//! Wraps zap_sftp protocol-level API into high-level operations directly usable by the UI layer.
//! author: logic
//! date: 2026-05-26

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use warp_ssh_manager::secrets::SshSecretStore;
use warp_ssh_manager::types::{AuthType, ResolvedSshAuth, SshServerInfo};
use warp_ssh_manager::SshRepository;
use zap_sftp::session::{AuthMethod, SftpSession};
use zap_sftp::types::OpenOptions;
use zap_sftp::Sftp;

use super::types::{FileEntry, FileEntryType, StableEntryIdentity};

/// Whether `name` is one safe child component in a remote directory.
pub(super) fn is_valid_remote_child_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
}

/// SFTP operation error
#[derive(Clone, Debug)]
pub enum SftpOpsError {
    /// Connection error
    Connection(String),
    /// Operation error
    Operation(String),
    /// The server connection has not negotiated descriptor-bound transfers.
    CapabilityRequired(String),
    /// Local I/O error
    LocalIo(String),
    /// Credentials not found
    NoCredentials(String),
    /// Transfer cancelled
    Cancelled,
    /// The destination is committed even though the final acknowledgement failed.
    Committed(String),
    /// The requested path does not exist.
    NotFound(String),
    /// The visible transfer result is safe, but retained paths require recovery.
    RecoveryRequired {
        /// Human-readable failure context.
        message: String,
        /// Process-wide cleanup action, when automatic retry is safe.
        recovery_id: Option<u64>,
        /// Paths retained instead of guessing after an indeterminate operation.
        paths: Vec<PathBuf>,
        /// Whether the new destination has already been committed.
        committed: bool,
    },
}

impl std::fmt::Display for SftpOpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SftpOpsError::Connection(msg) => write!(f, "Connection error: {msg}"),
            SftpOpsError::Operation(msg) => write!(f, "Operation error: {msg}"),
            SftpOpsError::CapabilityRequired(msg) => {
                write!(f, "Secure transfer capability required: {msg}")
            }
            SftpOpsError::LocalIo(msg) => write!(f, "Local I/O error: {msg}"),
            SftpOpsError::NoCredentials(msg) => write!(f, "Credentials not found: {msg}"),
            SftpOpsError::Cancelled => write!(f, "Transfer cancelled"),
            SftpOpsError::Committed(msg) => write!(f, "{msg}"),
            SftpOpsError::NotFound(path) => write!(f, "Path not found: {path}"),
            SftpOpsError::RecoveryRequired { message, paths, .. } => {
                write!(f, "{message}; recovery paths: ")?;
                for (index, path) in paths.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", path.display())?;
                }
                Ok(())
            }
        }
    }
}

impl SftpOpsError {
    /// Localized, non-sensitive summary for UI surfaces.
    ///
    /// The detailed diagnostic remains available through [`Display`] for
    /// logging, but must not be rendered into a toast or transfer row because
    /// backend errors may contain host-local paths or server details.
    pub fn user_message(&self) -> String {
        match self {
            Self::Connection(_) => crate::t!("fm-error-connection"),
            Self::Operation(_) => crate::t!("fm-error-operation"),
            Self::CapabilityRequired(_) => crate::t!("fm-error-secure-transfer-required"),
            Self::LocalIo(_) => crate::t!("fm-error-local-io"),
            Self::NoCredentials(_) => crate::t!("fm-error-credentials"),
            Self::Cancelled => crate::t!("fm-error-cancelled"),
            Self::Committed(_) => crate::t!("fm-error-committed"),
            Self::NotFound(_) => crate::t!("fm-error-not-found"),
            Self::RecoveryRequired { .. } => crate::t!("fm-error-recovery-required"),
        }
    }

    pub fn recovery_id(&self) -> Option<u64> {
        match self {
            Self::RecoveryRequired { recovery_id, .. } => *recovery_id,
            Self::Connection(_)
            | Self::Operation(_)
            | Self::CapabilityRequired(_)
            | Self::LocalIo(_)
            | Self::NoCredentials(_)
            | Self::Cancelled
            | Self::Committed(_)
            | Self::NotFound(_) => None,
        }
    }

    pub fn recovery_paths(&self) -> &[PathBuf] {
        match self {
            Self::RecoveryRequired { paths, .. } => paths,
            Self::Connection(_)
            | Self::Operation(_)
            | Self::CapabilityRequired(_)
            | Self::LocalIo(_)
            | Self::NoCredentials(_)
            | Self::Cancelled
            | Self::Committed(_)
            | Self::NotFound(_) => &[],
        }
    }

    pub fn destination_committed(&self) -> bool {
        matches!(
            self,
            Self::Committed(_)
                | Self::RecoveryRequired {
                    committed: true,
                    ..
                }
        )
    }
}

impl From<zap_sftp::SftpError> for SftpOpsError {
    fn from(e: zap_sftp::SftpError) -> Self {
        if e.is_not_found() {
            SftpOpsError::NotFound(e.to_string())
        } else {
            SftpOpsError::Operation(e.to_string())
        }
    }
}

impl From<std::io::Error> for SftpOpsError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            SftpOpsError::NotFound(e.to_string())
        } else {
            SftpOpsError::LocalIo(e.to_string())
        }
    }
}

/// Progress callback type
pub type ProgressCallback = Box<dyn Fn(u64, u64) + Send>;

/// Connection timeout duration
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn unique_transfer_sibling(path: &Path, marker: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    path.with_file_name(format!(".{name}.{marker}-{}", uuid::Uuid::new_v4()))
}

fn open_new_local_transfer_file(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

fn create_unique_local_transfer_file(
    path: &Path,
    marker: &str,
) -> Result<(PathBuf, fs::File), SftpOpsError> {
    for _ in 0..128 {
        let candidate = unique_transfer_sibling(path, marker);
        match open_new_local_transfer_file(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(SftpOpsError::LocalIo(error.to_string())),
        }
    }
    Err(SftpOpsError::LocalIo(format!(
        "Could not create a unique temporary sibling for {}",
        path.display()
    )))
}

fn create_unique_remote_transfer_file(
    sftp: &Sftp,
    path: &Path,
    marker: &str,
) -> Result<(PathBuf, zap_sftp::File), SftpOpsError> {
    for _ in 0..128 {
        let candidate = unique_transfer_sibling(path, marker);
        match sftp.open(&candidate, OpenOptions::create_new()) {
            Ok(file) => return Ok((candidate, file)),
            Err(_) if sftp.lstat(&candidate).is_ok() => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(SftpOpsError::Operation(format!(
        "Could not create a unique temporary sibling for {}",
        path.display()
    )))
}

/// Establish SFTP connection using server configuration
pub fn connect_from_server(
    server: &SshServerInfo,
    secret_store: &dyn SshSecretStore,
) -> Result<SftpSession, SftpOpsError> {
    let resolved_auth = resolve_sftp_auth(server)?;
    let auth = build_auth_method(server, &resolved_auth, secret_store)?;
    SftpSession::connect(
        &server.host,
        server.port,
        &resolved_auth.username,
        auth,
        Some(CONNECT_TIMEOUT),
    )
    .map_err(|e| SftpOpsError::Connection(e.to_string()))
}

fn resolve_sftp_auth(server: &SshServerInfo) -> Result<ResolvedSshAuth, SftpOpsError> {
    warp_ssh_manager::with_conn(|conn| Ok(SshRepository::resolve_server_auth(conn, server)?))
        .map_err(|e| SftpOpsError::NoCredentials(format!("Authentication resolution failed: {e}")))
}

/// List remote directory contents and convert to UI-layer FileEntry
pub fn list_dir(sftp: &Sftp, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
    let entries = sftp.read_dir(path)?;
    let result = entries
        .into_iter()
        .map(|entry| {
            let file_type = match entry.metadata.file_type {
                zap_sftp::types::FileType::Dir => FileEntryType::Directory,
                zap_sftp::types::FileType::File => FileEntryType::File,
                zap_sftp::types::FileType::Symlink => FileEntryType::Symlink,
                zap_sftp::types::FileType::Other => FileEntryType::Other,
            };
            let modified = entry.metadata.modified.map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            });
            let perms = &entry.metadata.permissions;
            let owner = bool_to_rwx(perms.owner_read, perms.owner_write, perms.owner_exec);
            let group = bool_to_rwx(perms.group_read, perms.group_write, perms.group_exec);
            let other = bool_to_rwx(perms.other_read, perms.other_write, perms.other_exec);
            let permissions = Some(format!("{owner}{group}{other}"));
            let modified_revision = entry
                .metadata
                .modified
                .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            FileEntry {
                name: entry.name,
                path: entry.path,
                file_type,
                size: entry.metadata.size,
                modified,
                permissions,
                identity: StableEntryIdentity {
                    file_type,
                    size: entry.metadata.size,
                    object_id: String::new(),
                    revision: format!(
                        "{}:{}:{modified_revision}",
                        entry.metadata.uid, entry.metadata.gid
                    ),
                },
            }
        })
        .collect();
    Ok(result)
}

/// Delete remote file
pub fn delete_file(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    sftp.remove_file(path)?;
    Ok(())
}

/// Recursively delete remote directory
pub fn delete_dir_recursive(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    let entries = sftp.read_dir(path)?;
    for entry in entries {
        match entry.metadata.file_type {
            zap_sftp::types::FileType::Dir => {
                delete_dir_recursive(sftp, &entry.path)?;
            }
            zap_sftp::types::FileType::File
            | zap_sftp::types::FileType::Symlink
            | zap_sftp::types::FileType::Other => {
                sftp.remove_file(&entry.path)?;
            }
        }
    }
    sftp.remove_dir(path)?;
    Ok(())
}

/// Create remote directory
pub fn create_dir(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    sftp.create_dir(path)?;
    Ok(())
}

/// Rename remote file or directory
pub fn rename(sftp: &Sftp, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
    let opts = zap_sftp::types::RenameOptions {
        overwrite: false,
        atomic: false,
        native: false,
    };
    sftp.rename(old_path, new_path, opts)?;
    Ok(())
}

/// Atomically replaces an existing remote entry.
///
/// Servers that do not support atomic replacement must reject the operation;
/// falling back to a remove or backup dance would leave the destination absent
/// across a crash boundary.
pub fn replace_atomic(sftp: &Sftp, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
    let opts = zap_sftp::types::RenameOptions {
        overwrite: true,
        atomic: true,
        native: false,
    };
    sftp.rename(old_path, new_path, opts)?;
    Ok(())
}

/// Stream-upload local file to remote
///
/// Uses temporary file pattern: first uploads to a temporary path with .sftp_partial suffix,
/// then renames to target path on completion. Cleans up temporary file on cancellation or failure
/// to avoid truncating existing remote files and causing data loss.
pub fn upload_file_streaming(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    upload_file_streaming_with_mode(
        sftp,
        local_path,
        remote_path,
        progress_cb,
        cancel_flag,
        true,
    )
}

pub fn upload_file_streaming_no_replace(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    upload_file_streaming_with_mode(
        sftp,
        local_path,
        remote_path,
        progress_cb,
        cancel_flag,
        false,
    )
}

fn upload_file_streaming_with_mode(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
    overwrite_destination: bool,
) -> Result<(), SftpOpsError> {
    let mut local_file =
        fs::File::open(local_path).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
    let total_size = local_file.metadata().map(|m| m.len()).unwrap_or(0);

    // Each in-flight transfer owns its temporary path. Two writes to the same
    // destination must never truncate, finalize, or clean up each other's data.
    let (temp_remote_path, mut remote_file) =
        create_unique_remote_transfer_file(sftp, remote_path, "sftp_partial")?;

    const CHUNK_SIZE: usize = 32 * 1024;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;

    let result = (|| -> Result<(), SftpOpsError> {
        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                return Err(SftpOpsError::Cancelled);
            }
            let n = std::io::Read::read(&mut local_file, &mut buf)
                .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
            if n == 0 {
                break;
            }
            remote_file.write_all(&buf[..n])?;
            transferred += n as u64;
            if let Some(cb) = progress_cb {
                cb(transferred, total_size);
            }
        }
        remote_file.flush()?;
        Ok(())
    })();
    drop(remote_file);

    match &result {
        Ok(()) => {
            if !overwrite_destination {
                if let Err(error) = sftp.rename(
                    &temp_remote_path,
                    remote_path,
                    zap_sftp::types::RenameOptions {
                        overwrite: false,
                        atomic: false,
                        native: false,
                    },
                ) {
                    let _ = sftp.remove_file(&temp_remote_path);
                    return Err(SftpOpsError::Operation(format!(
                        "Failed to commit remote file without replacement: {error}"
                    )));
                }
                return Ok(());
            }
            // Publish in one atomic replacement. If the server cannot provide
            // that guarantee, fail safely and keep the existing destination.
            if let Err(error) = sftp.rename(
                &temp_remote_path,
                remote_path,
                zap_sftp::types::RenameOptions {
                    overwrite: true,
                    atomic: true,
                    native: false,
                },
            ) {
                let _ = sftp.remove_file(&temp_remote_path);
                return Err(SftpOpsError::Operation(format!(
                    "Failed to atomically replace remote file: {error}"
                )));
            }
        }
        Err(_) => {
            // Cancel or failure: clean up temporary file
            let _ = sftp.remove_file(&temp_remote_path);
        }
    }

    result
}

/// Stream-download remote file to local
///
/// Uses temporary file pattern: first writes to a temporary file with .sftp_partial suffix,
/// then renames to target path on completion. Cleans up temporary file on cancellation or failure
/// to avoid truncating existing local files and causing data loss.
pub fn download_file_streaming(
    sftp: &Sftp,
    remote_path: &Path,
    local_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    download_file_streaming_with_mode(
        sftp,
        remote_path,
        local_path,
        progress_cb,
        cancel_flag,
        true,
    )
}

pub fn download_file_streaming_no_replace(
    sftp: &Sftp,
    remote_path: &Path,
    local_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    download_file_streaming_with_mode(
        sftp,
        remote_path,
        local_path,
        progress_cb,
        cancel_flag,
        false,
    )
}

fn download_file_streaming_with_mode(
    sftp: &Sftp,
    remote_path: &Path,
    local_path: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
    overwrite_destination: bool,
) -> Result<(), SftpOpsError> {
    let mut remote_file = sftp.open(remote_path, OpenOptions::read())?;
    let metadata = remote_file.stat()?;
    let total_size = metadata.size;

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
    }

    // Use a transfer-owned sibling so parallel downloads to the same final
    // path cannot corrupt or remove each other's temporary data.
    let (temp_local_path, mut local_file) =
        create_unique_local_transfer_file(local_path, "sftp_partial")?;

    const CHUNK_SIZE: usize = 32 * 1024;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;

    let result = (|| -> Result<(), SftpOpsError> {
        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                return Err(SftpOpsError::Cancelled);
            }
            let n = remote_file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            local_file
                .write_all(&buf[..n])
                .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
            transferred += n as u64;
            if let Some(cb) = progress_cb {
                cb(transferred, total_size);
            }
        }
        local_file
            .flush()
            .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
        Ok(())
    })();
    drop(local_file);

    match &result {
        Ok(()) => {
            // Download successful: rename temporary file to target path
            let finalize = if overwrite_destination {
                fs::rename(&temp_local_path, local_path)
            } else {
                fs::hard_link(&temp_local_path, local_path)
                    .and_then(|()| fs::remove_file(&temp_local_path))
            };
            if let Err(e) = finalize {
                let _ = fs::remove_file(&temp_local_path);
                return Err(SftpOpsError::LocalIo(format!(
                    "Failed to finalize downloaded temporary file: {e}"
                )));
            }
        }
        Err(_) => {
            // Cancel or failure: clean up temporary file
            let _ = fs::remove_file(&temp_local_path);
        }
    }

    result
}

/// Recursively upload local directory to remote
pub fn upload_dir_recursive(
    sftp: &Sftp,
    local_dir: &Path,
    remote_dir: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(SftpOpsError::Cancelled);
    }

    sftp.create_dir(remote_dir)?;

    let entries = fs::read_dir(local_dir).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;

    for entry in entries {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(SftpOpsError::Cancelled);
        }

        let entry = entry.map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
        let file_name = entry.file_name();
        let remote_path = normalize_remote_path(&remote_dir.join(&file_name));

        let file_type = entry
            .file_type()
            .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;

        if file_type.is_dir() {
            upload_dir_recursive(sftp, &entry.path(), &remote_path, progress_cb, cancel_flag)?;
        } else if file_type.is_file() {
            upload_file_streaming(sftp, &entry.path(), &remote_path, progress_cb, cancel_flag)?;
        } else if file_type.is_symlink() {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to recursively upload symbolic link {}",
                entry.path().display()
            )));
        } else {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to recursively upload special file {}",
                entry.path().display()
            )));
        }
    }

    Ok(())
}

/// Recursively download remote directory to local
pub fn download_dir_recursive(
    sftp: &Sftp,
    remote_dir: &Path,
    local_dir: &Path,
    progress_cb: Option<&ProgressCallback>,
    cancel_flag: &AtomicBool,
) -> Result<(), SftpOpsError> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(SftpOpsError::Cancelled);
    }

    fs::create_dir_all(local_dir).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;

    let entries = sftp.read_dir(remote_dir)?;

    for entry in entries {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(SftpOpsError::Cancelled);
        }

        // Path traversal protection: verify safety of filenames returned by remote server
        if !is_valid_remote_child_name(&entry.name) {
            return Err(SftpOpsError::Operation(format!(
                "Refusing unsafe remote directory entry: {}",
                entry.name
            )));
        }

        let safe_remote_path = normalize_remote_path(&remote_dir.join(&entry.name));
        let local_path = local_dir.join(&entry.name);

        match entry.metadata.file_type {
            zap_sftp::types::FileType::Dir => {
                download_dir_recursive(
                    sftp,
                    &safe_remote_path,
                    &local_path,
                    progress_cb,
                    cancel_flag,
                )?;
            }
            zap_sftp::types::FileType::File => {
                download_file_streaming(
                    sftp,
                    &safe_remote_path,
                    &local_path,
                    progress_cb,
                    cancel_flag,
                )?;
            }
            zap_sftp::types::FileType::Symlink => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to recursively download symbolic link {}",
                    safe_remote_path.display()
                )));
            }
            zap_sftp::types::FileType::Other => {
                return Err(SftpOpsError::Operation(format!(
                    "Refusing to recursively download special file {}",
                    safe_remote_path.display()
                )));
            }
        }
    }

    Ok(())
}

/// Build authentication method based on server configuration
fn build_auth_method(
    server: &SshServerInfo,
    resolved_auth: &ResolvedSshAuth,
    secret_store: &dyn SshSecretStore,
) -> Result<AuthMethod, SftpOpsError> {
    match resolved_auth.auth_type {
        AuthType::Password | AuthType::OneKey => {
            let password = secret_store
                .get(&resolved_auth.secret_lookup_id, resolved_auth.secret_kind)
                .map_err(|e| SftpOpsError::NoCredentials(format!("Failed to read password: {e}")))?
                .ok_or_else(|| {
                    SftpOpsError::NoCredentials(format!(
                        "No password stored for server {}",
                        server.host
                    ))
                })?;
            Ok(AuthMethod::Password {
                password: password.to_string(),
            })
        }
        AuthType::Key => {
            let passphrase = secret_store
                .get(&resolved_auth.secret_lookup_id, resolved_auth.secret_kind)
                .ok()
                .flatten()
                .map(|p| p.to_string());
            // A host configured for key auth without an explicit key file is
            // the normal case for anyone relying on `~/.ssh/config` +
            // `ssh-agent`: the terminal path shells out to `ssh` and just
            // works, while the file manager used to refuse to even try
            // ("no key path specified" — the FM never opened on such a host).
            // Fall back to what ssh itself does: agent, then default keys.
            match resolved_auth.key_path.as_ref() {
                Some(key_path) => Ok(AuthMethod::PublicKey {
                    key_path: PathBuf::from(shellexpand_path(key_path)),
                    passphrase,
                }),
                None => Ok(AuthMethod::AgentOrDefaultKeys { passphrase }),
            }
        }
    }
}

/// Expand ~ in path to user home directory
fn shellexpand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            let home_path = home.display();
            let suffix = &path[2..];
            return format!("{home_path}/{suffix}");
        }
    }
    path.to_string()
}

/// Convert read/write/execute booleans to rwx permission string
pub(crate) fn bool_to_rwx(read: bool, write: bool, exec: bool) -> String {
    let mut s = String::with_capacity(3);
    s.push(if read { 'r' } else { '-' });
    s.push(if write { 'w' } else { '-' });
    s.push(if exec { 'x' } else { '-' });
    s
}

/// Normalize remote path by converting Windows backslashes to forward slashes
///
/// Remote servers (Linux) only accept forward slash path separators.
/// On Windows, path joins produce backslashes, which must be converted.
pub(crate) fn normalize_remote_path(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
#[path = "sftp_ops_tests.rs"]
mod tests;
