//! Descriptor-bound remote file operations for the file-manager transfer engine.
//!
//! Plain SFTP cannot expose an immutable inode identity or a no-follow open.
//! This service runs inside the trusted remote daemon and keeps the actual file
//! descriptors alive across chunk calls. Every path mutation is journaled under
//! the remote user's home directory and keyed by a caller-supplied operation id.

use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::proto::{
    safe_file_request, safe_file_response, FileOperationError, SafeFileBeginUploadBatch,
    SafeFileCreateExclusive, SafeFileDelete, SafeFileEntryKind, SafeFileFlushHandle,
    SafeFileIdentity, SafeFileIdentityBatchEntryResult, SafeFileIdentityBatchResult,
    SafeFileIdentityBatchStatus, SafeFileInspectHandle, SafeFileInspectResult,
    SafeFileListIdentities, SafeFileMutationResult, SafeFileMutationState, SafeFileOpenExisting,
    SafeFileOpened, SafeFileReadHandle, SafeFileReadResult, SafeFileRecovery, SafeFileRecoveryList,
    SafeFileRename, SafeFileRenameMode, SafeFileRequest, SafeFileResponse, SafeFileSetModeHandle,
    SafeFileUploadBatchOpened, SafeFileUploadEntry, SafeFileUploadEntryOpened, SafeFileWriteHandle,
};
#[cfg(test)]
use super::proto::{SafeFileCleanupUploadBatch, SafeFileDeleteV2, SafeFileRetryRecovery};
use super::server_model::ConnectionId;

const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const MAX_TERMINAL_JOURNAL_RECORDS: usize = 1024;
const JOURNAL_DIRECTORY: &str = "safe-file-transactions-v1";
const LOCK_RETRY_ATTEMPTS: usize = 20;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct JournalIdentity {
    kind: i32,
    size: u64,
    object_id: String,
    revision: String,
}

impl From<&SafeFileIdentity> for JournalIdentity {
    fn from(identity: &SafeFileIdentity) -> Self {
        Self {
            kind: identity.kind,
            size: identity.size,
            object_id: identity.object_id.clone(),
            revision: identity.revision.clone(),
        }
    }
}

impl From<&JournalIdentity> for SafeFileIdentity {
    fn from(identity: &JournalIdentity) -> Self {
        Self {
            kind: identity.kind,
            size: identity.size,
            object_id: identity.object_id.clone(),
            revision: identity.revision.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct JournalRenameBoundary {
    old: Option<JournalIdentity>,
    new: Option<JournalIdentity>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JournalOperation {
    Create {
        path: String,
        identity: Option<JournalIdentity>,
    },
    Rename {
        old_path: String,
        new_path: String,
        mode: i32,
        source: JournalIdentity,
        target: Option<JournalIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boundary: Option<JournalRenameBoundary>,
    },
    Delete {
        path: String,
        tombstone: String,
        expected: JournalIdentity,
        expected_sha256: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JournalState {
    Started,
    Applied,
    Consumed,
    Rejected,
    Recovery,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct JournalRecord {
    operation_id: String,
    state: JournalState,
    operation: JournalOperation,
    recovery_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

struct Journal {
    directory: PathBuf,
}

struct OperationLock {
    _global: File,
    _operation: File,
}

impl Journal {
    fn new() -> std::io::Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| std::io::Error::other("HOME is unavailable"))?;
        let directory = home
            .join(".zaplex")
            .join("remote-server")
            .join(JOURNAL_DIRECTORY);
        Self::new_at(directory)
    }

    fn new_at(directory: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        Ok(Self { directory })
    }

    fn validate_operation_id(operation_id: &str) -> std::io::Result<()> {
        let valid = !operation_id.is_empty()
            && operation_id.len() <= 128
            && operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if valid {
            Ok(())
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
        }
    }

    fn record_path(&self, operation_id: &str) -> std::io::Result<PathBuf> {
        Self::validate_operation_id(operation_id)?;
        Ok(self.directory.join(format!("{operation_id}.json")))
    }

    fn lock_path(&self, operation_id: &str) -> std::io::Result<PathBuf> {
        Self::validate_operation_id(operation_id)?;
        Ok(self.directory.join(format!("{operation_id}.lock")))
    }

    fn global_lock_path(&self) -> PathBuf {
        self.directory.join(".journal.lock")
    }

    fn open_lock(path: &Path) -> std::io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
    }

    fn try_lock(&self, operation_id: &str) -> std::io::Result<OperationLock> {
        self.prune_terminal_records()?;
        let global = Self::open_lock(&self.global_lock_path())?;
        try_flock(&global, libc::LOCK_SH | libc::LOCK_NB)?;
        let operation = Self::open_lock(&self.lock_path(operation_id)?)?;
        try_flock(&operation, libc::LOCK_EX | libc::LOCK_NB)?;
        Ok(OperationLock {
            _global: global,
            _operation: operation,
        })
    }

    fn load(&self, operation_id: &str) -> std::io::Result<Option<JournalRecord>> {
        let path = self.record_path(operation_id)?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn save(&self, record: &JournalRecord) -> std::io::Result<()> {
        let path = self.record_path(&record.operation_id)?;
        let temporary = self.directory.join(format!(
            "{}.{}.tmp",
            record.operation_id,
            uuid::Uuid::new_v4()
        ));
        let bytes = serde_json::to_vec(record).map_err(std::io::Error::other)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        if let Ok(directory) = File::open(&self.directory) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    fn records(&self) -> std::io::Result<Vec<JournalRecord>> {
        let mut records: Vec<JournalRecord> = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let bytes = fs::read(path)?;
                records.push(serde_json::from_slice(&bytes).map_err(std::io::Error::other)?);
            }
        }
        records.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        Ok(records)
    }

    fn prune_terminal_records(&self) -> std::io::Result<()> {
        let global = Self::open_lock(&self.global_lock_path())?;
        let result = unsafe { libc::flock(global.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            return Ok(());
        }
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let record: JournalRecord =
                serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            if record.is_prunable() {
                let modified = entry
                    .metadata()?
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((modified, record.operation_id, path));
            }
        }
        if candidates.len() <= MAX_TERMINAL_JOURNAL_RECORDS {
            return Ok(());
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        let remove_count = candidates.len() - MAX_TERMINAL_JOURNAL_RECORDS;
        for (_, operation_id, path) in candidates.into_iter().take(remove_count) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            match fs::remove_file(self.lock_path(&operation_id)?) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn try_flock(file: &File, flags: libc::c_int) -> std::io::Result<()> {
    for attempt in 0..LOCK_RETRY_ATTEMPTS {
        if unsafe { libc::flock(file.as_raw_fd(), flags) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        match error.kind() {
            std::io::ErrorKind::Interrupted => {}
            std::io::ErrorKind::WouldBlock if attempt + 1 < LOCK_RETRY_ATTEMPTS => {
                std::thread::sleep(LOCK_RETRY_DELAY);
            }
            std::io::ErrorKind::WouldBlock => return Err(error),
            _ => return Err(error),
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::Interrupted))
}

impl JournalRecord {
    fn is_prunable(&self) -> bool {
        matches!(self.state, JournalState::Consumed | JournalState::Rejected)
    }
}

struct SafeHandle {
    owner: ConnectionId,
    file: File,
    kind: SafeFileEntryKind,
    path: PathBuf,
    artifact_operation_id: Option<String>,
    _artifact_lock: Option<OperationLock>,
}

struct SafeUploadBatch {
    owner: ConnectionId,
    parent: File,
    root: File,
    root_name: CString,
    root_path: PathBuf,
}

pub struct SafeFileServer {
    journal: Option<Journal>,
    initialization_error: Option<String>,
    handles: HashMap<String, SafeHandle>,
    upload_batches: HashMap<String, SafeUploadBatch>,
    #[cfg(test)]
    before_rename_mutation: Option<Box<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    after_rename_mutation: Option<Box<dyn Fn(&Path, &Path) + Send + Sync>>,
    #[cfg(test)]
    before_delete_isolation: Option<Box<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_private_delete_unlink: Option<Box<dyn Fn(&Path) + Send + Sync>>,
    #[cfg(test)]
    before_handle: Option<Box<dyn Fn(&SafeFileRequest) + Send + Sync>>,
}

impl SafeFileServer {
    pub fn new() -> Self {
        match Journal::new() {
            Ok(journal) => {
                let mut server = Self {
                    journal: Some(journal),
                    initialization_error: None,
                    handles: HashMap::new(),
                    upload_batches: HashMap::new(),
                    #[cfg(test)]
                    before_rename_mutation: None,
                    #[cfg(test)]
                    after_rename_mutation: None,
                    #[cfg(test)]
                    before_delete_isolation: None,
                    #[cfg(test)]
                    before_private_delete_unlink: None,
                    #[cfg(test)]
                    before_handle: None,
                };
                server.recover_abandoned_records();
                server
            }
            Err(error) => Self {
                journal: None,
                initialization_error: Some(error.to_string()),
                handles: HashMap::new(),
                upload_batches: HashMap::new(),
                #[cfg(test)]
                before_rename_mutation: None,
                #[cfg(test)]
                after_rename_mutation: None,
                #[cfg(test)]
                before_delete_isolation: None,
                #[cfg(test)]
                before_private_delete_unlink: None,
                #[cfg(test)]
                before_handle: None,
            },
        }
    }

    pub fn is_available(&self) -> bool {
        self.journal.is_some()
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(directory: PathBuf) -> Self {
        let journal = Journal::new_at(directory).expect("test safe-file journal should initialize");
        let mut server = Self {
            journal: Some(journal),
            initialization_error: None,
            handles: HashMap::new(),
            upload_batches: HashMap::new(),
            before_rename_mutation: None,
            after_rename_mutation: None,
            before_delete_isolation: None,
            before_private_delete_unlink: None,
            before_handle: None,
        };
        server.recover_abandoned_records();
        server
    }

    #[cfg(test)]
    pub(crate) fn set_before_handle_for_test(
        &mut self,
        hook: impl Fn(&SafeFileRequest) + Send + Sync + 'static,
    ) {
        self.before_handle = Some(Box::new(hook));
    }

    pub fn close_connection(&mut self, connection_id: ConnectionId) {
        let handles = self
            .handles
            .iter()
            .filter_map(|(handle_id, handle)| {
                (handle.owner == connection_id).then_some(handle_id.clone())
            })
            .collect::<Vec<_>>();
        for handle_id in handles {
            if let Some(handle) = self.handles.remove(&handle_id) {
                self.cleanup_owned_artifact(&handle);
            }
        }
        let upload_batches = self
            .upload_batches
            .iter()
            .filter_map(|(handle_id, batch)| {
                (batch.owner == connection_id).then_some(handle_id.clone())
            })
            .collect::<Vec<_>>();
        for handle_id in upload_batches {
            if let Err(error) = self.cleanup_upload_batch(connection_id, &handle_id) {
                log::warn!("Failed to clean up disconnected upload batch: {error}");
            }
        }
    }

    pub fn handle(
        &mut self,
        connection_id: ConnectionId,
        request: SafeFileRequest,
    ) -> SafeFileResponse {
        #[cfg(test)]
        if let Some(hook) = &self.before_handle {
            hook(&request);
        }
        let result = match request.operation {
            Some(safe_file_request::Operation::OpenExisting(open)) => self
                .open_existing(connection_id, open)
                .map(safe_file_response::Result::Opened),
            Some(safe_file_request::Operation::CreateExclusive(create)) => self
                .create_exclusive(connection_id, &request.operation_id, create)
                .map(safe_file_response::Result::Opened),
            Some(safe_file_request::Operation::ReadHandle(read)) => self
                .read_handle(connection_id, read)
                .map(safe_file_response::Result::Read),
            Some(safe_file_request::Operation::WriteHandle(write)) => self
                .write_handle(connection_id, write)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::FlushHandle(flush)) => self
                .flush_handle(connection_id, flush)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::SetModeHandle(set_mode)) => self
                .set_mode_handle(connection_id, set_mode)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::InspectHandle(inspect)) => self
                .inspect_handle(connection_id, inspect)
                .map(safe_file_response::Result::Inspected),
            Some(safe_file_request::Operation::CloseHandle(close)) => self
                .close_handle(connection_id, &close.handle_id)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::Rename(rename)) => self
                .rename(connection_id, &request.operation_id, rename)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::Delete(delete)) => self
                .delete(&request.operation_id, delete, true)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::DeleteV2(delete)) => self
                .delete(
                    &request.operation_id,
                    SafeFileDelete {
                        path: delete.path,
                        expected: delete.expected,
                        expected_sha256: None,
                    },
                    false,
                )
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::ListRecoveries(_)) => self
                .list_recoveries()
                .map(safe_file_response::Result::Recoveries),
            Some(safe_file_request::Operation::RetryRecovery(_)) => self
                .retry_recovery(&request.operation_id)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::BeginUploadBatch(begin)) => self
                .begin_upload_batch(connection_id, begin)
                .map(safe_file_response::Result::UploadBatchOpened),
            Some(safe_file_request::Operation::CleanupUploadBatch(cleanup)) => self
                .cleanup_upload_batch(connection_id, &cleanup.batch_handle_id)
                .map(safe_file_response::Result::Mutation),
            Some(safe_file_request::Operation::ListIdentities(list)) => self
                .list_identities(list)
                .map(safe_file_response::Result::Identities),
            None => Err("Safe-file request has no operation".to_string()),
        };
        SafeFileResponse {
            result: Some(result.unwrap_or_else(|message| {
                safe_file_response::Result::Error(FileOperationError { message })
            })),
        }
    }

    fn journal(&self) -> Result<&Journal, String> {
        self.journal.as_ref().ok_or_else(|| {
            format!(
                "Safe-file journal is unavailable: {}",
                self.initialization_error
                    .as_deref()
                    .unwrap_or("unknown error")
            )
        })
    }

    fn begin_upload_batch(
        &mut self,
        owner: ConnectionId,
        request: SafeFileBeginUploadBatch,
    ) -> Result<SafeFileUploadBatchOpened, String> {
        Journal::validate_operation_id(&request.batch_id).map_err(|error| error.to_string())?;
        let destination_path = validated_path(&request.destination_directory)?;
        let destination = open_nofollow(&destination_path, SafeFileEntryKind::Directory, false)
            .map_err(|error| error.to_string())?;
        let staging_name = CString::new(".zap-upload-staging").expect("static name has no NUL");
        let staging_parent = ensure_private_directory_at(&destination, &staging_name, true)?;
        let batch_name = CString::new(request.batch_id.as_bytes())
            .map_err(|_| "Upload batch id contains NUL".to_string())?;
        let root = ensure_private_directory_at(&staging_parent, &batch_name, false)?;
        let root_path = destination_path
            .join(OsStr::from_bytes(staging_name.as_bytes()))
            .join(OsStr::from_bytes(batch_name.as_bytes()));

        let entries_result = create_upload_entries(&root, &root_path, request.entries);
        let entries = match entries_result {
            Ok(entries) => entries,
            Err(error) => {
                let _ = remove_directory_contents(&root);
                let _ = unlink_directory_at(&staging_parent, &batch_name);
                return Err(error);
            }
        };

        let mut opened_entries = Vec::with_capacity(entries.len());
        for (relative_path, path, kind, file, identity) in entries {
            let handle_id = uuid::Uuid::new_v4().to_string();
            self.handles.insert(
                handle_id.clone(),
                SafeHandle {
                    owner,
                    file,
                    kind,
                    path,
                    artifact_operation_id: None,
                    _artifact_lock: None,
                },
            );
            opened_entries.push(SafeFileUploadEntryOpened {
                relative_path,
                handle_id,
                identity: Some(identity),
            });
        }

        let batch_handle_id = uuid::Uuid::new_v4().to_string();
        let root_path_string = path_to_string(&root_path)?;
        self.upload_batches.insert(
            batch_handle_id.clone(),
            SafeUploadBatch {
                owner,
                parent: staging_parent,
                root,
                root_name: batch_name,
                root_path,
            },
        );
        Ok(SafeFileUploadBatchOpened {
            batch_handle_id,
            root_path: root_path_string,
            entries: opened_entries,
        })
    }

    fn cleanup_upload_batch(
        &mut self,
        owner: ConnectionId,
        batch_handle_id: &str,
    ) -> Result<SafeFileMutationResult, String> {
        if self
            .upload_batches
            .get(batch_handle_id)
            .is_none_or(|batch| batch.owner != owner)
        {
            return Err("Upload batch is unknown or belongs to another connection".to_string());
        }
        let batch = self
            .upload_batches
            .remove(batch_handle_id)
            .expect("owned upload batch disappeared");
        let result = authenticate_upload_batch(&batch)
            .and_then(|()| remove_directory_contents(&batch.root))
            .and_then(|()| unlink_directory_at(&batch.parent, &batch.root_name));
        match result {
            Ok(()) => Ok(applied_mutation()),
            Err(error) => {
                self.upload_batches
                    .insert(batch_handle_id.to_string(), batch);
                Err(error)
            }
        }
    }

    fn open_existing(
        &mut self,
        owner: ConnectionId,
        request: SafeFileOpenExisting,
    ) -> Result<SafeFileOpened, String> {
        let kind = SafeFileEntryKind::try_from(request.expected_kind)
            .map_err(|_| "Invalid expected safe-file kind".to_string())?;
        let path = validated_path(&request.path)?;
        let file = open_nofollow(&path, kind, false).map_err(|error| error.to_string())?;
        self.insert_handle(owner, file, kind, path, None, None)
    }

    fn list_identities(
        &self,
        request: SafeFileListIdentities,
    ) -> Result<SafeFileIdentityBatchResult, String> {
        let mut entries = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let expected_kind = SafeFileEntryKind::try_from(entry.expected_kind)
                .map_err(|_| "Invalid expected safe-file kind".to_string())?;
            if expected_kind == SafeFileEntryKind::Unspecified {
                return Err("Safe-file identity batch requires a concrete kind".to_string());
            }
            let path = validated_path(&entry.path)?;
            let result = match fs::symlink_metadata(&path) {
                Ok(metadata) => match identity_from_metadata(&metadata, expected_kind) {
                    Ok(identity) => SafeFileIdentityBatchEntryResult {
                        path: entry.path,
                        status: SafeFileIdentityBatchStatus::Found as i32,
                        identity: Some(identity),
                    },
                    Err(error) => {
                        log::debug!(
                            "Safe-file listing identity changed for {}: {error}",
                            path.display()
                        );
                        SafeFileIdentityBatchEntryResult {
                            path: entry.path,
                            status: SafeFileIdentityBatchStatus::KindChanged as i32,
                            identity: None,
                        }
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    SafeFileIdentityBatchEntryResult {
                        path: entry.path,
                        status: SafeFileIdentityBatchStatus::NotFound as i32,
                        identity: None,
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    SafeFileIdentityBatchEntryResult {
                        path: entry.path,
                        status: SafeFileIdentityBatchStatus::PermissionDenied as i32,
                        identity: None,
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "Failed to inspect safe-file identity for {}: {error}",
                        path.display()
                    ));
                }
            };
            entries.push(result);
        }
        Ok(SafeFileIdentityBatchResult { entries })
    }

    fn create_exclusive(
        &mut self,
        owner: ConnectionId,
        operation_id: &str,
        request: SafeFileCreateExclusive,
    ) -> Result<SafeFileOpened, String> {
        let kind = SafeFileEntryKind::try_from(request.kind)
            .map_err(|_| "Invalid created safe-file kind".to_string())?;
        let path = validated_path(&request.path)?;
        if let Some((handle_id, handle)) = self.handles.iter().find(|(_, handle)| {
            handle.owner == owner
                && handle.artifact_operation_id.as_deref() == Some(operation_id)
                && handle.path == path
                && handle.kind == kind
        }) {
            return Ok(SafeFileOpened {
                handle_id: handle_id.clone(),
                identity: Some(identity_for_file(&handle.file, handle.kind)?),
            });
        }
        let journal = self.journal()?;
        let artifact_lock = journal
            .try_lock(operation_id)
            .map_err(|error| format!("Safe-file operation is already active: {error}"))?;
        if let Some(record) = journal
            .load(operation_id)
            .map_err(|error| error.to_string())?
        {
            if let (
                JournalState::Applied,
                JournalOperation::Create {
                    path: recorded_path,
                    identity: Some(identity),
                },
            ) = (&record.state, &record.operation)
            {
                if recorded_path == &request.path {
                    let file =
                        open_nofollow(&path, kind, false).map_err(|error| error.to_string())?;
                    let actual = identity_for_file(&file, kind)?;
                    if same_object(&actual, &SafeFileIdentity::from(identity)) {
                        return self.insert_handle(
                            owner,
                            file,
                            kind,
                            path,
                            Some(operation_id.to_string()),
                            Some(artifact_lock),
                        );
                    }
                }
            }
            return Err(format!(
                "Safe-file operation id {operation_id} is already bound to another result"
            ));
        }

        let cleanup_tombstone = owned_cleanup_tombstone(&path, operation_id);
        let mut record = JournalRecord {
            operation_id: operation_id.to_string(),
            state: JournalState::Started,
            operation: JournalOperation::Create {
                path: request.path.clone(),
                identity: None,
            },
            recovery_paths: vec![
                path.to_string_lossy().into_owned(),
                cleanup_tombstone.to_string_lossy().into_owned(),
            ],
            failure: None,
        };
        journal.save(&record).map_err(|error| error.to_string())?;
        let file = match open_nofollow(&path, kind, true) {
            Ok(file) => file,
            Err(error) => {
                record.state = JournalState::Rejected;
                record.failure = Some(error.to_string());
                journal.save(&record).map_err(|save_error| {
                    format!("{error}; persisting the rejected create failed: {save_error}")
                })?;
                return Err(error.to_string());
            }
        };
        let identity = identity_for_file(&file, kind)?;
        record.state = JournalState::Applied;
        record.operation = JournalOperation::Create {
            path: request.path,
            identity: Some(JournalIdentity::from(&identity)),
        };
        if let Err(error) = journal.save(&record) {
            let cleanup = cleanup_owned_object(&path, &cleanup_tombstone, &identity);
            return Err(match cleanup {
                Ok(()) => error.to_string(),
                Err(cleanup_error) => {
                    format!("{error}; removing the unjournaled artifact failed: {cleanup_error}")
                }
            });
        }
        self.insert_handle(
            owner,
            file,
            kind,
            path,
            Some(operation_id.to_string()),
            Some(artifact_lock),
        )
    }

    fn insert_handle(
        &mut self,
        owner: ConnectionId,
        file: File,
        kind: SafeFileEntryKind,
        path: PathBuf,
        artifact_operation_id: Option<String>,
        artifact_lock: Option<OperationLock>,
    ) -> Result<SafeFileOpened, String> {
        let identity = identity_for_file(&file, kind)?;
        let handle_id = uuid::Uuid::new_v4().to_string();
        self.handles.insert(
            handle_id.clone(),
            SafeHandle {
                owner,
                file,
                kind,
                path,
                artifact_operation_id,
                _artifact_lock: artifact_lock,
            },
        );
        Ok(SafeFileOpened {
            handle_id,
            identity: Some(identity),
        })
    }

    fn owned_handle_mut(
        &mut self,
        owner: ConnectionId,
        handle_id: &str,
    ) -> Result<&mut SafeHandle, String> {
        self.handles
            .get_mut(handle_id)
            .filter(|handle| handle.owner == owner)
            .ok_or_else(|| {
                "Safe-file handle is unknown or belongs to another connection".to_string()
            })
    }

    fn read_handle(
        &mut self,
        owner: ConnectionId,
        request: SafeFileReadHandle,
    ) -> Result<SafeFileReadResult, String> {
        let handle = self.owned_handle_mut(owner, &request.handle_id)?;
        if handle.kind != SafeFileEntryKind::Regular {
            return Err("Cannot read bytes from a directory handle".to_string());
        }
        let max_bytes = usize::try_from(request.max_bytes)
            .unwrap_or(usize::MAX)
            .min(MAX_CHUNK_BYTES);
        if max_bytes == 0 {
            return Err("Safe-file reads require a positive byte limit".to_string());
        }
        let mut bytes = vec![0; max_bytes];
        let read = handle
            .file
            .read(&mut bytes)
            .map_err(|error| error.to_string())?;
        bytes.truncate(read);
        Ok(SafeFileReadResult {
            bytes,
            eof: read == 0,
        })
    }

    fn write_handle(
        &mut self,
        owner: ConnectionId,
        request: SafeFileWriteHandle,
    ) -> Result<SafeFileMutationResult, String> {
        let handle = self.owned_handle_mut(owner, &request.handle_id)?;
        if handle.kind != SafeFileEntryKind::Regular {
            return Err("Cannot write bytes to a directory handle".to_string());
        }
        if request.bytes.len() > MAX_CHUNK_BYTES {
            return Err(format!(
                "Safe-file write exceeds the {MAX_CHUNK_BYTES} byte chunk limit"
            ));
        }
        handle
            .file
            .write_all(&request.bytes)
            .map_err(|error| error.to_string())?;
        Ok(applied_mutation())
    }

    fn flush_handle(
        &mut self,
        owner: ConnectionId,
        request: SafeFileFlushHandle,
    ) -> Result<SafeFileMutationResult, String> {
        let handle = self.owned_handle_mut(owner, &request.handle_id)?;
        handle.file.sync_all().map_err(|error| error.to_string())?;
        Ok(applied_mutation())
    }

    fn set_mode_handle(
        &mut self,
        owner: ConnectionId,
        request: SafeFileSetModeHandle,
    ) -> Result<SafeFileMutationResult, String> {
        let handle = self.owned_handle_mut(owner, &request.handle_id)?;
        if handle.kind != SafeFileEntryKind::Regular {
            return Err("Cannot set a file mode on a directory handle".to_string());
        }
        handle
            .file
            .set_permissions(fs::Permissions::from_mode(request.mode & 0o777))
            .map_err(|error| error.to_string())?;
        Ok(applied_mutation())
    }

    fn inspect_handle(
        &mut self,
        owner: ConnectionId,
        request: SafeFileInspectHandle,
    ) -> Result<SafeFileInspectResult, String> {
        let handle = self.owned_handle_mut(owner, &request.handle_id)?;
        let identity = identity_for_file(&handle.file, handle.kind)?;
        let matches_path = if request.path.is_empty() {
            false
        } else {
            let path = validated_path(&request.path)?;
            identity_for_path(&path)
                .as_ref()
                .is_ok_and(|actual| same_object(&identity, actual))
        };
        Ok(SafeFileInspectResult {
            identity: Some(identity),
            matches_path,
            link_count: Some(
                handle
                    .file
                    .metadata()
                    .map_err(|error| error.to_string())?
                    .nlink(),
            ),
        })
    }

    fn close_handle(
        &mut self,
        owner: ConnectionId,
        handle_id: &str,
    ) -> Result<SafeFileMutationResult, String> {
        if self
            .upload_batches
            .get(handle_id)
            .is_some_and(|batch| batch.owner == owner)
        {
            return self.cleanup_upload_batch(owner, handle_id);
        }
        if self
            .handles
            .get(handle_id)
            .is_none_or(|handle| handle.owner != owner)
        {
            return Err("Safe-file handle is unknown or belongs to another connection".to_string());
        }
        let handle = self
            .handles
            .remove(handle_id)
            .expect("owned safe-file handle disappeared");
        self.cleanup_owned_artifact(&handle);
        Ok(applied_mutation())
    }

    fn rename(
        &mut self,
        owner: ConnectionId,
        operation_id: &str,
        request: SafeFileRename,
    ) -> Result<SafeFileMutationResult, String> {
        let mode = SafeFileRenameMode::try_from(request.mode)
            .map_err(|_| "Invalid safe-file rename mode".to_string())?;
        let old_path = validated_path(&request.old_path)?;
        let new_path = validated_path(&request.new_path)?;
        let _operation_lock = self
            .journal()?
            .try_lock(operation_id)
            .map_err(|error| format!("Safe-file operation is already active: {error}"))?;
        let existing_record = self
            .journal()?
            .load(operation_id)
            .map_err(|error| error.to_string())?;
        if let Some(record) = existing_record {
            return self.resolve_existing_mutation(record);
        }
        let source = {
            let handle = self.owned_handle_mut(owner, &request.handle_id)?;
            identity_for_file(&handle.file, handle.kind)?
        };
        if !identity_for_path(&old_path)
            .as_ref()
            .is_ok_and(|actual| same_identity(&source, actual))
        {
            return Err("Safe-file rename source no longer matches its handle".to_string());
        }
        let target = request.expected_target.clone();
        match mode {
            SafeFileRenameMode::NoReplace => {
                if target.is_some() {
                    return Err("No-replace rename cannot carry a target identity".to_string());
                }
            }
            SafeFileRenameMode::Exchange => {
                let expected = target
                    .as_ref()
                    .ok_or_else(|| "Exchange requires a target identity".to_string())?;
                if !identity_for_path(&new_path)
                    .as_ref()
                    .is_ok_and(|actual| same_identity(expected, actual))
                {
                    return Err("Safe-file exchange target identity changed".to_string());
                }
            }
            SafeFileRenameMode::Unspecified => {
                return Err("Safe-file rename mode is unspecified".to_string());
            }
        }
        let mut record = JournalRecord {
            operation_id: operation_id.to_string(),
            state: JournalState::Started,
            operation: JournalOperation::Rename {
                old_path: request.old_path,
                new_path: request.new_path,
                mode: request.mode,
                source: JournalIdentity::from(&source),
                target: target.as_ref().map(JournalIdentity::from),
                boundary: None,
            },
            recovery_paths: vec![
                old_path.to_string_lossy().into_owned(),
                new_path.to_string_lossy().into_owned(),
            ],
            failure: None,
        };
        self.journal()?
            .save(&record)
            .map_err(|error| error.to_string())?;
        #[cfg(test)]
        if let Some(hook) = &self.before_rename_mutation {
            hook(&old_path, &new_path);
        }
        let pre_old = identity_for_path(&old_path).ok();
        let pre_new = identity_for_path(&new_path).ok();
        if let JournalOperation::Rename { boundary, .. } = &mut record.operation {
            *boundary = Some(JournalRenameBoundary {
                old: pre_old.as_ref().map(JournalIdentity::from),
                new: pre_new.as_ref().map(JournalIdentity::from),
            });
        }
        self.journal()?
            .save(&record)
            .map_err(|error| error.to_string())?;
        let mutation = match mode {
            SafeFileRenameMode::NoReplace => rename_noreplace(&old_path, &new_path),
            SafeFileRenameMode::Exchange => rename_exchange(&old_path, &new_path),
            SafeFileRenameMode::Unspecified => unreachable!(),
        };
        #[cfg(test)]
        if let Some(hook) = &self.after_rename_mutation {
            hook(&old_path, &new_path);
        }
        self.reconcile_rename(
            &mut record,
            mutation.as_ref().err().map(ToString::to_string),
        )?;
        let consumed_artifact = if let Some(handle) = self.handles.get_mut(&request.handle_id) {
            handle.path = new_path;
            let artifact_operation_id = handle.artifact_operation_id.take();
            if artifact_operation_id.is_some() {
                handle._artifact_lock = None;
            }
            artifact_operation_id
        } else {
            None
        };
        if let Some(artifact_operation_id) = consumed_artifact {
            self.consume_artifact(&artifact_operation_id);
        }
        Ok(applied_mutation())
    }

    fn delete(
        &mut self,
        operation_id: &str,
        request: SafeFileDelete,
        require_file_digest: bool,
    ) -> Result<SafeFileMutationResult, String> {
        let expected = request
            .expected
            .ok_or_else(|| "Safe-file delete requires an identity".to_string())?;
        let kind = SafeFileEntryKind::try_from(expected.kind)
            .map_err(|_| "Invalid safe-file delete kind".to_string())?;
        let path = validated_path(&request.path)?;
        let journal = self.journal()?;
        let _operation_lock = journal
            .try_lock(operation_id)
            .map_err(|error| format!("Safe-file operation is already active: {error}"))?;
        if let Some(record) = journal
            .load(operation_id)
            .map_err(|error| error.to_string())?
        {
            return self.resolve_existing_mutation(record);
        }
        let tombstone = delete_tombstone(&path, operation_id);
        let tombstone_string = path_to_string(&tombstone)?;
        let private_tombstone_string = path_to_string(&private_delete_entry(&tombstone))?;
        let file = open_nofollow(&path, kind, false).map_err(|error| error.to_string())?;
        let actual = identity_for_file(&file, kind)?;
        if !matches_delete_identity(&expected, &actual) {
            return Err("Safe-file delete identity changed".to_string());
        }
        match kind {
            SafeFileEntryKind::Regular => {
                if require_file_digest {
                    let expected_digest = request.expected_sha256.as_deref().ok_or_else(|| {
                        "Safe-file v1 delete requires a content digest".to_string()
                    })?;
                    let digest = sha256_file(&file)?;
                    if expected_digest != digest {
                        return Err("Safe-file delete content digest changed".to_string());
                    }
                }
            }
            SafeFileEntryKind::Directory => {
                if fs::read_dir(&path)
                    .map_err(|error| error.to_string())?
                    .next()
                    .is_some()
                {
                    return Err("Safe-file directory is not empty".to_string());
                }
            }
            SafeFileEntryKind::Symlink => {
                if request.expected_sha256.is_some() {
                    return Err("Safe-file symlink delete cannot carry a digest".to_string());
                }
            }
            SafeFileEntryKind::Unspecified => {
                return Err("Safe-file delete kind is unspecified".to_string());
            }
        }
        let mut record = JournalRecord {
            operation_id: operation_id.to_string(),
            state: JournalState::Started,
            operation: JournalOperation::Delete {
                path: request.path,
                tombstone: tombstone_string.clone(),
                expected: JournalIdentity::from(&expected),
                expected_sha256: request.expected_sha256,
            },
            recovery_paths: vec![
                path.to_string_lossy().into_owned(),
                tombstone_string,
                private_tombstone_string,
            ],
            failure: None,
        };
        journal.save(&record).map_err(|error| error.to_string())?;
        self.reconcile_delete(&mut record, None)?;
        Ok(applied_mutation())
    }

    fn resolve_existing_mutation(
        &self,
        mut record: JournalRecord,
    ) -> Result<SafeFileMutationResult, String> {
        match record.state {
            JournalState::Applied | JournalState::Consumed => Ok(SafeFileMutationResult {
                state: SafeFileMutationState::AlreadyApplied as i32,
            }),
            JournalState::Started => match record.operation {
                JournalOperation::Rename { .. } => self.reconcile_rename(&mut record, None),
                JournalOperation::Delete { .. } => self.reconcile_delete(&mut record, None),
                JournalOperation::Create { .. } => {
                    Err("Incomplete safe-file create requires recovery".to_string())
                }
            },
            JournalState::Rejected => Err(record
                .failure
                .unwrap_or_else(|| "Safe-file operation was rejected".to_string())),
            JournalState::Recovery => Err(format!(
                "Safe-file operation requires recovery: {}",
                record.recovery_paths.join(", ")
            )),
        }
    }

    fn reconcile_rename(
        &self,
        record: &mut JournalRecord,
        primary_error: Option<String>,
    ) -> Result<SafeFileMutationResult, String> {
        let JournalOperation::Rename {
            old_path,
            new_path,
            mode,
            source,
            target,
            boundary,
        } = record.operation.clone()
        else {
            return Err("Safe-file journal kind mismatch".to_string());
        };
        let old_path = validated_path(&old_path)?;
        let new_path = validated_path(&new_path)?;
        let source = SafeFileIdentity::from(&source);
        let target = target.as_ref().map(SafeFileIdentity::from);
        let mode = SafeFileRenameMode::try_from(mode)
            .map_err(|_| "Invalid journaled rename mode".to_string())?;
        let old = identity_for_path(&old_path).ok();
        let new = identity_for_path(&new_path).ok();
        if rename_was_applied(
            mode,
            &source,
            target.as_ref(),
            path_is_absent(&old_path),
            old.as_ref(),
            new.as_ref(),
        ) {
            record.state = JournalState::Applied;
            record.failure = None;
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Ok(SafeFileMutationResult {
                state: SafeFileMutationState::AlreadyApplied as i32,
            });
        }
        let Some(boundary) = boundary else {
            record.state = JournalState::Recovery;
            record.failure = Some(
                "Safe-file rename journal predates boundary snapshots and requires recovery"
                    .to_string(),
            );
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Err("Safe-file rename boundary state is unavailable".to_string());
        };
        let boundary_old = boundary.old.as_ref().map(SafeFileIdentity::from);
        let boundary_new = boundary.new.as_ref().map(SafeFileIdentity::from);
        let boundary_applied = match mode {
            SafeFileRenameMode::NoReplace => {
                boundary_new.is_none()
                    && path_is_absent(&old_path)
                    && boundary_old.as_ref().is_some_and(|expected| {
                        new.as_ref()
                            .is_some_and(|actual| same_renamed_identity(expected, actual))
                    })
            }
            SafeFileRenameMode::Exchange => {
                boundary_old.as_ref().is_some_and(|expected| {
                    new.as_ref()
                        .is_some_and(|actual| same_renamed_identity(expected, actual))
                }) && boundary_new.as_ref().is_some_and(|expected| {
                    old.as_ref()
                        .is_some_and(|actual| same_renamed_identity(expected, actual))
                })
            }
            SafeFileRenameMode::Unspecified => false,
        };
        if boundary_applied {
            let restored = match mode {
                SafeFileRenameMode::NoReplace => {
                    rename_noreplace(&new_path, &old_path).is_ok()
                        && boundary_state_matches(
                            &old_path,
                            &new_path,
                            boundary_old.as_ref(),
                            boundary_new.as_ref(),
                        )
                }
                SafeFileRenameMode::Exchange => {
                    rename_exchange(&old_path, &new_path).is_ok()
                        && boundary_state_matches(
                            &old_path,
                            &new_path,
                            boundary_old.as_ref(),
                            boundary_new.as_ref(),
                        )
                }
                SafeFileRenameMode::Unspecified => false,
            };
            record.state = if restored {
                JournalState::Rejected
            } else {
                JournalState::Recovery
            };
            record.failure = Some(if restored {
                "Safe-file rename input changed at the mutation boundary; namespace restored"
                    .to_string()
            } else {
                "Safe-file rename boundary mutation requires recovery".to_string()
            });
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Err(record.failure.clone().unwrap());
        }
        if boundary_state_matches(
            &old_path,
            &new_path,
            boundary_old.as_ref(),
            boundary_new.as_ref(),
        ) {
            let boundary_matches_request = boundary_old
                .as_ref()
                .is_some_and(|actual| same_identity(&source, actual))
                && match mode {
                    SafeFileRenameMode::NoReplace => boundary_new.is_none(),
                    SafeFileRenameMode::Exchange => target.as_ref().is_some_and(|expected| {
                        boundary_new
                            .as_ref()
                            .is_some_and(|actual| same_identity(expected, actual))
                    }),
                    SafeFileRenameMode::Unspecified => false,
                };
            if !boundary_matches_request || primary_error.is_some() {
                let error = primary_error.unwrap_or_else(|| {
                    "Safe-file rename inputs changed before the journaled mutation".to_string()
                });
                record.state = JournalState::Rejected;
                record.failure = Some(error.clone());
                self.journal()?
                    .save(record)
                    .map_err(|save_error| save_error.to_string())?;
                return Err(error);
            }
            let retry = match mode {
                SafeFileRenameMode::NoReplace => rename_noreplace(&old_path, &new_path),
                SafeFileRenameMode::Exchange => rename_exchange(&old_path, &new_path),
                SafeFileRenameMode::Unspecified => unreachable!(),
            };
            return self.reconcile_rename(record, retry.as_ref().err().map(ToString::to_string));
        }
        record.state = JournalState::Recovery;
        record.failure = primary_error;
        self.journal()?
            .save(record)
            .map_err(|error| error.to_string())?;
        Err("Safe-file rename state is inconsistent and requires recovery".to_string())
    }

    fn reconcile_delete(
        &self,
        record: &mut JournalRecord,
        primary_error: Option<String>,
    ) -> Result<SafeFileMutationResult, String> {
        let JournalOperation::Delete {
            path,
            tombstone,
            expected,
            expected_sha256,
        } = record.operation.clone()
        else {
            return Err("Safe-file journal kind mismatch".to_string());
        };
        let path = validated_path(&path)?;
        let tombstone = validated_path(&tombstone)?;
        let expected = SafeFileIdentity::from(&expected);
        let current = identity_for_path(&path).ok();
        let isolated = identity_for_path(&tombstone).ok();
        let private = identity_for_path(&private_delete_entry(&tombstone)).ok();
        if private
            .as_ref()
            .is_some_and(|actual| matches_isolated_delete_identity(&expected, actual))
        {
            let outcome = delete_exact_path(
                &tombstone,
                &expected,
                expected_sha256.as_deref(),
                None,
                None,
            )?;
            if let DeleteExactOutcome::Unattributed(error) = outcome {
                record.state = JournalState::Recovery;
                record.failure = Some(error.clone());
                self.journal()?
                    .save(record)
                    .map_err(|save_error| save_error.to_string())?;
                return Err(error);
            }
            record.state = JournalState::Applied;
            record.failure = None;
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Ok(SafeFileMutationResult {
                state: SafeFileMutationState::AlreadyApplied as i32,
            });
        }
        if private.is_some() {
            record.state = JournalState::Recovery;
            record.failure = Some("Private safe-file delete entry changed".to_string());
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Err("Private safe-file delete entry requires recovery".to_string());
        }
        if path_is_absent(&path) && path_is_absent(&tombstone) {
            let error = "Safe-file delete objects are absent without a verified committed unlink"
                .to_string();
            record.state = JournalState::Recovery;
            record.failure = Some(error.clone());
            self.journal()?
                .save(record)
                .map_err(|save_error| save_error.to_string())?;
            return Err(error);
        }
        if isolated
            .as_ref()
            .is_some_and(|actual| matches_isolated_delete_identity(&expected, actual))
        {
            match delete_exact_path(
                &tombstone,
                &expected,
                expected_sha256.as_deref(),
                None,
                #[cfg(test)]
                self.before_private_delete_unlink.as_deref(),
                #[cfg(not(test))]
                None,
            ) {
                Ok(DeleteExactOutcome::Committed) => {
                    record.state = JournalState::Applied;
                    record.failure = None;
                    self.journal()?
                        .save(record)
                        .map_err(|error| error.to_string())?;
                    return Ok(SafeFileMutationResult {
                        state: SafeFileMutationState::AlreadyApplied as i32,
                    });
                }
                Ok(DeleteExactOutcome::Unattributed(error)) => {
                    record.state = JournalState::Recovery;
                    record.failure = Some(error.clone());
                    self.journal()?
                        .save(record)
                        .map_err(|save_error| save_error.to_string())?;
                    return Err(error);
                }
                Err(error) => {
                    if path_is_absent(&tombstone)
                        && path_is_absent(&private_delete_entry(&tombstone))
                    {
                        record.state = JournalState::Recovery;
                        record.failure = Some(error.clone());
                        self.journal()?
                            .save(record)
                            .map_err(|save_error| save_error.to_string())?;
                        return Err(error);
                    }
                    if identity_for_path(&tombstone)
                        .as_ref()
                        .is_ok_and(|actual| matches_isolated_delete_identity(&expected, actual))
                    {
                        record.state = JournalState::Recovery;
                        record.failure = Some(error.clone());
                        self.journal()?
                            .save(record)
                            .map_err(|save_error| save_error.to_string())?;
                        return Err(format!(
                            "{error}; the isolated safe-file object requires recovery"
                        ));
                    }
                }
            }
        }
        if isolated.is_some() {
            record.state = JournalState::Recovery;
            record.failure = Some("Safe-file delete tombstone names another object".to_string());
            self.journal()?
                .save(record)
                .map_err(|error| error.to_string())?;
            return Err(
                "Safe-file delete tombstone names another object and requires recovery".to_string(),
            );
        }
        if current
            .as_ref()
            .is_some_and(|actual| same_object(&expected, actual))
        {
            if !current
                .as_ref()
                .is_some_and(|actual| matches_delete_identity(&expected, actual))
            {
                let error = "Safe-file delete target content changed before deletion".to_string();
                record.state = JournalState::Rejected;
                record.failure = Some(error.clone());
                self.journal()?
                    .save(record)
                    .map_err(|save_error| save_error.to_string())?;
                return Err(error);
            }
            if let Some(error) = primary_error.as_ref() {
                record.state = JournalState::Rejected;
                record.failure = Some(error.clone());
                self.journal()?
                    .save(record)
                    .map_err(|save_error| save_error.to_string())?;
                return Err(error.clone());
            }
            #[cfg(test)]
            if let Some(hook) = &self.before_delete_isolation {
                hook(&path);
            }
            let Some(pre_isolation) = identity_for_path(&path)
                .ok()
                .filter(|actual| matches_delete_identity(&expected, actual))
            else {
                let error = "Safe-file delete target content changed before isolation".to_string();
                record.state = JournalState::Rejected;
                record.failure = Some(error.clone());
                self.journal()?
                    .save(record)
                    .map_err(|save_error| save_error.to_string())?;
                return Err(error);
            };
            match rename_noreplace(&path, &tombstone) {
                Ok(()) => {
                    let isolated_matches =
                        identity_for_path(&tombstone).as_ref().is_ok_and(|actual| {
                            matches_isolated_delete_identity(&pre_isolation, actual)
                        });
                    if !isolated_matches {
                        let isolated = identity_for_path(&tombstone).ok();
                        let restored = path_is_absent(&path)
                            && rename_noreplace(&tombstone, &path).is_ok()
                            && isolated.as_ref().is_some_and(|isolated| {
                                identity_for_path(&path)
                                    .as_ref()
                                    .is_ok_and(|actual| same_renamed_identity(isolated, actual))
                            });
                        record.state = if restored {
                            JournalState::Rejected
                        } else {
                            JournalState::Recovery
                        };
                        record.failure = Some(if restored {
                            "Safe-file delete replacement was restored before mutation".to_string()
                        } else {
                            "Safe-file delete isolated an unexpected object and requires recovery"
                                .to_string()
                        });
                        self.journal()?
                            .save(record)
                            .map_err(|error| error.to_string())?;
                        return Err(record.failure.clone().unwrap());
                    }
                    return self.reconcile_delete(record, None);
                }
                Err(error) => return self.reconcile_delete(record, Some(error.to_string())),
            }
        }
        record.state = JournalState::Recovery;
        record.failure = primary_error;
        self.journal()?
            .save(record)
            .map_err(|error| error.to_string())?;
        Err("Safe-file delete path now names another object and requires recovery".to_string())
    }

    fn consume_artifact(&self, operation_id: &str) {
        let Ok(Some(mut record)) = self.journal().and_then(|journal| {
            journal
                .load(operation_id)
                .map_err(|error| error.to_string())
        }) else {
            return;
        };
        if matches!(record.operation, JournalOperation::Create { .. }) {
            record.state = JournalState::Consumed;
            record.failure = None;
            let _ = self
                .journal()
                .and_then(|journal| journal.save(&record).map_err(|error| error.to_string()));
        }
    }

    fn cleanup_owned_artifact(&self, handle: &SafeHandle) {
        let Some(operation_id) = handle.artifact_operation_id.as_deref() else {
            return;
        };
        let Ok(identity) = identity_for_file(&handle.file, handle.kind) else {
            return;
        };
        let cleanup_tombstone = owned_cleanup_tombstone(&handle.path, operation_id);
        let (state, failure) =
            match cleanup_owned_object(&handle.path, &cleanup_tombstone, &identity) {
                Ok(()) => (JournalState::Consumed, None),
                Err(error) => (JournalState::Recovery, Some(error)),
            };
        let Ok(Some(mut record)) = self.journal().and_then(|journal| {
            journal
                .load(operation_id)
                .map_err(|error| error.to_string())
        }) else {
            return;
        };
        record.state = state;
        record.failure = failure;
        let _ = self
            .journal()
            .and_then(|journal| journal.save(&record).map_err(|error| error.to_string()));
    }

    fn list_recoveries(&self) -> Result<SafeFileRecoveryList, String> {
        let recoveries = self
            .journal()?
            .records()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|record| {
                record.state == JournalState::Recovery
                    || matches!(
                        (&record.state, &record.operation),
                        (
                            JournalState::Applied,
                            JournalOperation::Rename { .. } | JournalOperation::Delete { .. }
                        )
                    )
            })
            .map(|record| {
                let source_preserved_after_commit =
                    matches!(record.operation, JournalOperation::Rename { .. });
                let mut paths = record.recovery_paths;
                if source_preserved_after_commit && paths.len() > 1 {
                    paths.rotate_left(1);
                }
                SafeFileRecovery {
                    operation_id: record.operation_id,
                    paths,
                    source_preserved_after_commit,
                }
            })
            .collect();
        Ok(SafeFileRecoveryList { recoveries })
    }

    fn consume_applied_mutation(
        &self,
        mut record: JournalRecord,
    ) -> Result<SafeFileMutationResult, String> {
        if !matches!(
            (&record.state, &record.operation),
            (
                JournalState::Applied,
                JournalOperation::Rename { .. } | JournalOperation::Delete { .. }
            )
        ) {
            return self.resolve_existing_mutation(record);
        }
        record.state = JournalState::Consumed;
        record.failure = None;
        self.journal()?
            .save(&record)
            .map_err(|error| error.to_string())?;
        Ok(SafeFileMutationResult {
            state: SafeFileMutationState::AlreadyApplied as i32,
        })
    }

    fn retry_recovery(&self, operation_id: &str) -> Result<SafeFileMutationResult, String> {
        let journal = self.journal()?;
        let _operation_lock = journal
            .try_lock(operation_id)
            .map_err(|error| format!("Safe-file recovery is already active: {error}"))?;
        let mut record = journal
            .load(operation_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Safe-file recovery {operation_id} does not exist"))?;
        if record.state == JournalState::Applied {
            return self.consume_applied_mutation(record);
        }
        if record.state != JournalState::Recovery {
            return self.resolve_existing_mutation(record);
        }

        let operation = record.operation.clone();
        let result = match operation {
            JournalOperation::Create {
                path,
                identity: Some(identity),
            } => {
                let path = validated_path(&path)?;
                let identity = SafeFileIdentity::from(&identity);
                let tombstone = owned_cleanup_tombstone(&path, operation_id);
                match cleanup_owned_object(&path, &tombstone, &identity) {
                    Ok(()) => {
                        record.state = JournalState::Consumed;
                        record.failure = None;
                        journal.save(&record).map_err(|error| error.to_string())?;
                        Ok(SafeFileMutationResult {
                            state: SafeFileMutationState::AlreadyApplied as i32,
                        })
                    }
                    Err(error) => {
                        record.failure = Some(error.clone());
                        journal.save(&record).map_err(|save_error| {
                            format!("{error}; persisting recovery state failed: {save_error}")
                        })?;
                        Err(error)
                    }
                }
            }
            JournalOperation::Rename { .. } => self.reconcile_rename(&mut record, None),
            JournalOperation::Delete { .. } => self.reconcile_delete(&mut record, None),
            JournalOperation::Create { identity: None, .. } => {
                Err("Safe-file create has no durable ownership identity".to_string())
            }
        };
        if result.is_ok()
            && matches!(
                record.operation,
                JournalOperation::Rename { .. } | JournalOperation::Delete { .. }
            )
        {
            let applied = journal
                .load(operation_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Safe-file recovery {operation_id} disappeared"))?;
            return self.consume_applied_mutation(applied);
        }
        result
    }

    fn recover_abandoned_records(&mut self) {
        let Some(journal) = self.journal.as_ref() else {
            return;
        };
        let Ok(records) = journal.records() else {
            return;
        };
        for mut record in records {
            let Ok(_lock) = journal.try_lock(&record.operation_id) else {
                continue;
            };
            match (&record.state, &record.operation) {
                (
                    JournalState::Applied,
                    JournalOperation::Create {
                        path,
                        identity: Some(identity),
                    },
                ) => {
                    let Ok(path) = validated_path(path) else {
                        record.state = JournalState::Recovery;
                        let _ = journal.save(&record);
                        continue;
                    };
                    let identity = SafeFileIdentity::from(identity);
                    let tombstone = owned_cleanup_tombstone(&path, &record.operation_id);
                    match cleanup_owned_object(&path, &tombstone, &identity) {
                        Ok(()) => {
                            record.state = JournalState::Consumed;
                            record.failure = None;
                        }
                        Err(error) => {
                            record.state = JournalState::Recovery;
                            record.failure = Some(error);
                        }
                    }
                    let _ = journal.save(&record);
                }
                (JournalState::Started, JournalOperation::Rename { .. }) => {
                    let _ = self.reconcile_rename(&mut record, None);
                }
                (JournalState::Started, JournalOperation::Delete { .. }) => {
                    let _ = self.reconcile_delete(&mut record, None);
                }
                (JournalState::Started, JournalOperation::Create { .. }) => {
                    record.state = JournalState::Recovery;
                    record.failure =
                        Some("Safe-file create stopped before ownership was recorded".to_string());
                    let _ = journal.save(&record);
                }
                (JournalState::Applied, JournalOperation::Create { identity: None, .. }) => {
                    record.state = JournalState::Recovery;
                    record.failure =
                        Some("Safe-file create has no durable ownership identity".to_string());
                    let _ = journal.save(&record);
                }
                (
                    JournalState::Applied
                    | JournalState::Consumed
                    | JournalState::Rejected
                    | JournalState::Recovery,
                    JournalOperation::Rename { .. } | JournalOperation::Delete { .. },
                )
                | (
                    JournalState::Consumed | JournalState::Rejected | JournalState::Recovery,
                    JournalOperation::Create { .. },
                ) => {}
            }
        }
    }
}

fn applied_mutation() -> SafeFileMutationResult {
    SafeFileMutationResult {
        state: SafeFileMutationState::Applied as i32,
    }
}

fn validated_path(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err("Safe-file operations require an absolute path".to_string());
    }
    Ok(path)
}

type CreatedUploadEntry = (String, PathBuf, SafeFileEntryKind, File, SafeFileIdentity);

fn create_upload_entries(
    root: &File,
    root_path: &Path,
    entries: Vec<SafeFileUploadEntry>,
) -> Result<Vec<CreatedUploadEntry>, String> {
    let mut seen = HashSet::new();
    let mut created = Vec::with_capacity(entries.len());
    for entry in entries {
        if !seen.insert(entry.relative_path.clone()) {
            return Err(format!(
                "Upload batch contains duplicate entry {:?}",
                entry.relative_path
            ));
        }
        let kind = SafeFileEntryKind::try_from(entry.kind)
            .map_err(|_| "Invalid upload entry kind".to_string())?;
        if !matches!(
            kind,
            SafeFileEntryKind::Regular | SafeFileEntryKind::Directory
        ) {
            return Err("Upload entries must be regular files or directories".to_string());
        }
        let components = validated_upload_components(&entry.relative_path)?;
        let (leaf, parents) = components
            .split_last()
            .ok_or_else(|| "Upload entry path is empty".to_string())?;
        let mut directory = root.try_clone().map_err(|error| error.to_string())?;
        for parent in parents {
            directory = ensure_private_directory_at(&directory, parent, true)?;
        }
        let file = match kind {
            SafeFileEntryKind::Regular => create_private_file_at(&directory, leaf)?,
            SafeFileEntryKind::Directory => ensure_private_directory_at(&directory, leaf, true)?,
            SafeFileEntryKind::Symlink | SafeFileEntryKind::Unspecified => unreachable!(),
        };
        let identity = identity_for_file(&file, kind)?;
        let relative_path = entry.relative_path;
        created.push((
            relative_path.clone(),
            root_path.join(relative_path),
            kind,
            file,
            identity,
        ));
    }
    Ok(created)
}

fn validated_upload_components(path: &str) -> Result<Vec<CString>, String> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("Upload entry path must be relative".to_string());
    }
    let components = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(component) => CString::new(component.as_bytes())
                .map_err(|_| "Upload entry path contains NUL".to_string()),
            std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir
            | std::path::Component::Prefix(_) => {
                Err("Upload entry path escapes its batch root".to_string())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err("Upload entry path is empty".to_string());
    }
    Ok(components)
}

fn ensure_private_directory_at(
    parent: &File,
    name: &CString,
    allow_existing: bool,
) -> Result<File, String> {
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
    if !created {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists || !allow_existing {
            return Err(error.to_string());
        }
    }
    let result = (|| {
        let directory = open_directory_at(parent, name)?;
        if created {
            let result = unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        authenticate_private_directory(&directory)?;
        Ok(directory)
    })();
    if created && result.is_err() {
        let _ = unlink_directory_at(parent, name);
    }
    result
}

fn open_directory_at(parent: &File, name: &CString) -> Result<File, String> {
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn authenticate_private_directory(directory: &File) -> Result<(), String> {
    let metadata = directory.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err("Upload staging directory is not owned private storage".to_string());
    }
    Ok(())
}

fn create_private_file_at(parent: &File, name: &CString) -> Result<File, String> {
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let result = unsafe { libc::fchmod(file.as_raw_fd(), 0o600) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(file)
}

fn authenticate_upload_batch(batch: &SafeUploadBatch) -> Result<(), String> {
    authenticate_private_directory(&batch.parent)?;
    authenticate_private_directory(&batch.root)?;
    let current = open_directory_at(&batch.parent, &batch.root_name).map_err(|error| {
        format!(
            "Upload batch root {} is inaccessible: {error}",
            batch.root_path.display()
        )
    })?;
    let expected = identity_for_file(&batch.root, SafeFileEntryKind::Directory)?;
    let actual = identity_for_file(&current, SafeFileEntryKind::Directory)?;
    if !same_object(&expected, &actual) {
        return Err("Upload batch root no longer matches its held handle".to_string());
    }
    Ok(())
}

fn remove_directory_contents(directory: &File) -> Result<(), String> {
    let reader = directory.try_clone().map_err(|error| error.to_string())?;
    let mut reader = nix::dir::Dir::from(reader).map_err(|error| error.to_string())?;
    let names = reader
        .iter()
        .filter_map(|entry| match entry {
            Ok(entry)
                if entry.file_name().to_bytes() != b"."
                    && entry.file_name().to_bytes() != b".." =>
            {
                Some(Ok(entry.file_name().to_owned()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error.to_string())),
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(reader);

    for name in names {
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child_name = name.to_owned();
            let child = open_directory_at(directory, &child_name)?;
            remove_directory_contents(&child)?;
            unlink_directory_at(directory, &child_name)?;
        } else {
            let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
    }
    Ok(())
}

fn unlink_directory_at(parent: &File, name: &CString) -> Result<(), String> {
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn path_to_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Safe-file protocol requires UTF-8 paths".to_string())
}

fn delete_tombstone(path: &Path, operation_id: &str) -> PathBuf {
    path.parent()
        .unwrap_or(Path::new("/"))
        .join(format!(".zaplex-delete-{operation_id}"))
}

fn owned_cleanup_tombstone(path: &Path, operation_id: &str) -> PathBuf {
    path.parent()
        .unwrap_or(Path::new("/"))
        .join(format!(".zaplex-owned-{operation_id}"))
}

fn cleanup_owned_object(
    path: &Path,
    tombstone: &Path,
    expected: &SafeFileIdentity,
) -> Result<(), String> {
    let mut cleaned_isolated_object = false;
    if let Ok(isolated) = identity_for_path(tombstone) {
        if !same_object(expected, &isolated) {
            return Err("Owned safe-file cleanup tombstone names another object".to_string());
        }
        delete_owned_tombstone(tombstone, expected)?;
        cleaned_isolated_object = true;
    } else if !path_is_absent(tombstone) {
        return Err("Owned safe-file cleanup tombstone is inaccessible".to_string());
    }

    if path_is_absent(path) {
        return Ok(());
    }
    let current = identity_for_path(path)
        .map_err(|error| format!("Owned safe-file artifact is inaccessible: {error}"))?;
    if !same_object(expected, &current) {
        if cleaned_isolated_object {
            return Ok(());
        }
        return Err("Owned safe-file artifact path now names another object".to_string());
    }
    rename_noreplace(path, tombstone)
        .map_err(|error| format!("Owned safe-file artifact could not be isolated: {error}"))?;
    let isolated = identity_for_path(tombstone)
        .map_err(|error| format!("Owned safe-file cleanup tombstone is inaccessible: {error}"))?;
    if !same_object(expected, &isolated) {
        return Err("Owned safe-file artifact identity changed during isolation".to_string());
    }
    delete_owned_tombstone(tombstone, expected)
}

fn delete_owned_tombstone(tombstone: &Path, expected: &SafeFileIdentity) -> Result<(), String> {
    let actual = identity_for_path(tombstone)
        .map_err(|error| format!("Owned safe-file cleanup tombstone is inaccessible: {error}"))?;
    if !same_object(expected, &actual) {
        return Err("Owned safe-file cleanup tombstone identity changed".to_string());
    }
    let kind = SafeFileEntryKind::try_from(expected.kind).ok();
    let current = identity_for_path(tombstone)
        .map_err(|error| format!("Owned safe-file cleanup tombstone is inaccessible: {error}"))?;
    if !same_object(expected, &current) {
        return Err(
            "Owned safe-file cleanup tombstone identity changed before removal".to_string(),
        );
    }
    match kind {
        Some(SafeFileEntryKind::Regular | SafeFileEntryKind::Symlink) => fs::remove_file(tombstone),
        Some(SafeFileEntryKind::Directory) => fs::remove_dir(tombstone),
        Some(SafeFileEntryKind::Unspecified) | None => {
            return Err("Owned safe-file artifact kind is invalid".to_string());
        }
    }
    .map_err(|error| format!("Removing the isolated safe-file artifact failed: {error}"))
}

fn path_is_absent(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn open_nofollow(
    path: &Path,
    kind: SafeFileEntryKind,
    create_exclusive: bool,
) -> std::io::Result<File> {
    match kind {
        SafeFileEntryKind::Regular => {
            let mut options = OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
            if create_exclusive {
                options.write(true).create_new(true).mode(0o600);
            }
            options.open(path)
        }
        SafeFileEntryKind::Directory => {
            if create_exclusive {
                use std::os::unix::fs::DirBuilderExt as _;

                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700).create(path)?;
            }
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY)
                .open(path);
            let directory = match directory {
                Ok(directory) => directory,
                Err(error) => {
                    if create_exclusive {
                        let _ = fs::remove_dir(path);
                    }
                    return Err(error);
                }
            };
            if create_exclusive {
                let chmod = unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) };
                if chmod != 0 {
                    let error = std::io::Error::last_os_error();
                    drop(directory);
                    let _ = fs::remove_dir(path);
                    return Err(error);
                }
            }
            Ok(directory)
        }
        SafeFileEntryKind::Symlink => {
            if create_exclusive {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
            #[cfg(target_os = "linux")]
            {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(path)
            }
            #[cfg(target_os = "macos")]
            {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_SYMLINK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(path)
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
            }
        }
        SafeFileEntryKind::Unspecified => {
            Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
        }
    }
}

fn identity_for_file(
    file: &File,
    expected_kind: SafeFileEntryKind,
) -> Result<SafeFileIdentity, String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    identity_from_metadata(&metadata, expected_kind)
}

fn identity_for_path(path: &Path) -> Result<SafeFileIdentity, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let kind = if metadata.file_type().is_symlink() {
        SafeFileEntryKind::Symlink
    } else if metadata.is_file() {
        SafeFileEntryKind::Regular
    } else if metadata.is_dir() {
        SafeFileEntryKind::Directory
    } else {
        return Err(
            "Safe-file path is neither a regular file, directory, nor symbolic link".to_string(),
        );
    };
    identity_from_metadata(&metadata, kind)
}

fn identity_from_metadata(
    metadata: &fs::Metadata,
    expected_kind: SafeFileEntryKind,
) -> Result<SafeFileIdentity, String> {
    let actual_kind = if metadata.file_type().is_symlink() {
        SafeFileEntryKind::Symlink
    } else if metadata.is_file() {
        SafeFileEntryKind::Regular
    } else if metadata.is_dir() {
        SafeFileEntryKind::Directory
    } else {
        return Err(
            "Safe-file object is neither a regular file, directory, nor symbolic link".to_string(),
        );
    };
    if actual_kind != expected_kind {
        return Err("Safe-file object kind changed".to_string());
    }
    Ok(SafeFileIdentity {
        kind: actual_kind as i32,
        size: metadata.len(),
        object_id: format!("{}:{}", metadata.dev(), metadata.ino()),
        revision: format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            metadata.len()
        ),
    })
}

fn same_object(expected: &SafeFileIdentity, actual: &SafeFileIdentity) -> bool {
    expected.kind == actual.kind
        && !expected.object_id.is_empty()
        && expected.object_id == actual.object_id
}

fn same_identity(expected: &SafeFileIdentity, actual: &SafeFileIdentity) -> bool {
    same_object(expected, actual)
        && expected.size == actual.size
        && same_revision(&expected.revision, &actual.revision)
}

fn same_renamed_identity(expected: &SafeFileIdentity, actual: &SafeFileIdentity) -> bool {
    same_object(expected, actual)
        && expected.size == actual.size
        && same_content_revision(&expected.revision, &actual.revision)
}

fn same_revision(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    let legacy = expected.split(':').collect::<Vec<_>>();
    let current = actual.split(':').collect::<Vec<_>>();
    legacy.len() == 5
        && current.len() == 7
        && legacy[..4] == current[..4]
        && legacy[4] == current[6]
}

fn same_content_revision(expected: &str, actual: &str) -> bool {
    fn content_fields(revision: &str) -> Option<[&str; 5]> {
        let fields = revision.split(':').collect::<Vec<_>>();
        match fields.as_slice() {
            [device, inode, modified, modified_nanos, size] => {
                Some([device, inode, modified, modified_nanos, size])
            }
            [device, inode, modified, modified_nanos, _, _, size] => {
                Some([device, inode, modified, modified_nanos, size])
            }
            _ => None,
        }
    }

    expected == actual
        || content_fields(expected)
            .zip(content_fields(actual))
            .is_some_and(|(expected, actual)| expected == actual)
}

fn matches_delete_identity(expected: &SafeFileIdentity, actual: &SafeFileIdentity) -> bool {
    match SafeFileEntryKind::try_from(expected.kind).ok() {
        Some(SafeFileEntryKind::Regular) => same_identity(expected, actual),
        Some(SafeFileEntryKind::Directory | SafeFileEntryKind::Symlink) => {
            same_object(expected, actual)
        }
        Some(SafeFileEntryKind::Unspecified) | None => false,
    }
}

// A successful isolation rename can advance ctime without changing the object
// or its contents. Callers use this only after the journaled delete has crossed
// that namespace boundary; delete_exact_path captures the resulting identity
// and compares it strictly again immediately before unlinking.
fn matches_isolated_delete_identity(
    expected: &SafeFileIdentity,
    actual: &SafeFileIdentity,
) -> bool {
    match SafeFileEntryKind::try_from(expected.kind).ok() {
        Some(SafeFileEntryKind::Regular) => same_renamed_identity(expected, actual),
        Some(SafeFileEntryKind::Directory | SafeFileEntryKind::Symlink) => {
            same_object(expected, actual)
        }
        Some(SafeFileEntryKind::Unspecified) | None => false,
    }
}

fn rename_was_applied(
    mode: SafeFileRenameMode,
    source: &SafeFileIdentity,
    target: Option<&SafeFileIdentity>,
    old_absent: bool,
    old: Option<&SafeFileIdentity>,
    new: Option<&SafeFileIdentity>,
) -> bool {
    match mode {
        SafeFileRenameMode::NoReplace => {
            old_absent && new.is_some_and(|actual| same_renamed_identity(source, actual))
        }
        SafeFileRenameMode::Exchange => target.is_some_and(|target| {
            old.is_some_and(|actual| same_renamed_identity(target, actual))
                && new.is_some_and(|actual| same_renamed_identity(source, actual))
        }),
        SafeFileRenameMode::Unspecified => false,
    }
}

fn boundary_state_matches(
    old_path: &Path,
    new_path: &Path,
    expected_old: Option<&SafeFileIdentity>,
    expected_new: Option<&SafeFileIdentity>,
) -> bool {
    let old_matches = match expected_old {
        Some(expected) => identity_for_path(old_path)
            .as_ref()
            .is_ok_and(|actual| same_renamed_identity(expected, actual)),
        None => path_is_absent(old_path),
    };
    let new_matches = match expected_new {
        Some(expected) => identity_for_path(new_path)
            .as_ref()
            .is_ok_and(|actual| same_renamed_identity(expected, actual)),
        None => path_is_absent(new_path),
    };
    old_matches && new_matches
}

fn private_delete_directory(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "entry".to_string());
    path.with_file_name(format!(".{name}.private"))
}

fn private_delete_entry(path: &Path) -> PathBuf {
    private_delete_directory(path).join("entry")
}

enum DeleteExactOutcome {
    Committed,
    Unattributed(String),
}

fn delete_exact_path(
    path: &Path,
    expected: &SafeFileIdentity,
    expected_sha256: Option<&str>,
    before_isolation: Option<&(dyn Fn(&Path) + Send + Sync)>,
    before_unlink: Option<&(dyn Fn(&Path) + Send + Sync)>,
) -> Result<DeleteExactOutcome, String> {
    let kind = SafeFileEntryKind::try_from(expected.kind)
        .map_err(|_| "Invalid journaled safe-file delete kind".to_string())?;
    let private_directory = private_delete_directory(path);
    let private_entry = private_delete_entry(path);
    if path_is_absent(&private_entry) {
        let boundary = identity_for_path(path)?;
        if !matches_isolated_delete_identity(expected, &boundary) {
            return Err("Safe-file delete isolation identity changed".to_string());
        }
        match fs::create_dir(&private_directory) {
            Ok(()) => fs::set_permissions(&private_directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let metadata =
            fs::symlink_metadata(&private_directory).map_err(|error| error.to_string())?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            return Err("Private safe-file delete directory is not authenticated".to_string());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY)
            .open(&private_directory)
            .map_err(|error| error.to_string())?;
        let name =
            std::ffi::CString::new(b"entry".as_slice()).map_err(|error| error.to_string())?;
        if let Some(hook) = before_isolation {
            hook(path);
        }
        let pre_isolation = identity_for_path(path)?;
        if !matches_delete_identity(&boundary, &pre_isolation) {
            return Err("Safe-file delete identity changed before private isolation".to_string());
        }
        rename_noreplace_into_directory(path, &directory, &name)
            .map_err(|error| error.to_string())?;
        let isolated = identity_for_path(&private_entry)?;
        if !matches_isolated_delete_identity(&pre_isolation, &isolated) {
            let restored = path_is_absent(path)
                && rename_noreplace_from_directory(&directory, &name, path).is_ok()
                && identity_for_path(path)
                    .as_ref()
                    .is_ok_and(|actual| same_object(&isolated, actual));
            let _ = fs::remove_dir(&private_directory);
            return Err(if restored {
                "Safe-file delete replacement was restored before mutation".to_string()
            } else {
                "Safe-file delete isolated an unexpected object and requires recovery".to_string()
            });
        }
    }
    let metadata = fs::symlink_metadata(&private_directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err("Private safe-file delete directory is not authenticated".to_string());
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY)
        .open(&private_directory)
        .map_err(|error| error.to_string())?;
    let file = open_nofollow(&private_entry, kind, false).map_err(|error| error.to_string())?;
    let actual = identity_for_file(&file, kind)?;
    if !matches_isolated_delete_identity(expected, &actual) {
        return Err("Safe-file delete identity changed".to_string());
    }
    match kind {
        SafeFileEntryKind::Regular => {
            if let Some(expected_sha256) = expected_sha256 {
                let digest = sha256_file(&file)?;
                if expected_sha256 != digest {
                    return Err("Safe-file delete content digest changed".to_string());
                }
            }
        }
        SafeFileEntryKind::Directory => {
            if fs::read_dir(&private_entry)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                return Err("Safe-file directory is not empty".to_string());
            }
        }
        SafeFileEntryKind::Symlink => {
            if expected_sha256.is_some() {
                return Err("Safe-file symlink delete cannot carry a digest".to_string());
            }
        }
        SafeFileEntryKind::Unspecified => {
            return Err("Safe-file delete kind is unspecified".to_string());
        }
    }
    let current = identity_for_path(&private_entry)?;
    if !matches_delete_identity(&actual, &current) {
        return Err("Private safe-file delete identity changed before removal".to_string());
    }
    let expected_links_before = file.metadata().map_err(|error| error.to_string())?.nlink();
    if let Some(hook) = before_unlink {
        hook(&private_entry);
    }
    let name = std::ffi::CString::new(b"entry".as_slice()).map_err(|error| error.to_string())?;
    let flags = if kind == SafeFileEntryKind::Directory {
        libc::AT_REMOVEDIR
    } else {
        0
    };
    // Linux and macOS do not provide an `unlinkat` equivalent of opening an
    // object handle and unlinking that exact handle (`AT_EMPTY_PATH` is not an
    // unlink flag). The final operation is therefore anchored to the private
    // directory descriptor and entry name, not to `file` itself. A same-UID
    // process can still race that last name lookup. Keep the cutpoint after the
    // final identity check and classify the held object's link-count change and
    // the anchored name afterward; any mismatch remains durable recovery work.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let expected_links_after = file.metadata().map_err(|error| error.to_string())?.nlink();
    if expected_links_after >= expected_links_before
        || !directory_entry_is_absent(&directory, &name)?
    {
        return Ok(DeleteExactOutcome::Unattributed(
            "Safe-file final unlink could not be attributed to the held object; recovery required"
                .to_string(),
        ));
    }
    fs::remove_dir(private_delete_directory(path)).map_err(|error| error.to_string())?;
    Ok(DeleteExactOutcome::Committed)
}

fn directory_entry_is_absent(directory: &File, name: &std::ffi::CStr) -> Result<bool, String> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOENT) {
        Ok(true)
    } else {
        Err(error.to_string())
    }
}

fn sha256_file(file: &File) -> Result<String, String> {
    let mut file = file.try_clone().map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(target_os = "linux")]
fn rename_noreplace_into_directory(
    old_path: &Path,
    directory: &File,
    name: &std::ffi::CStr,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old_path.as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace_from_directory(
    directory: &File,
    name: &std::ffi::CStr,
    new_path: &Path,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            directory.as_raw_fd(),
            name.as_ptr(),
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
fn rename_noreplace_into_directory(
    old_path: &Path,
    directory: &File,
    name: &std::ffi::CStr,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            old_path.as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace_from_directory(
    directory: &File,
    name: &std::ffi::CStr,
    new_path: &Path,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace_into_directory(
    _old_path: &Path,
    _directory: &File,
    _name: &std::ffi::CStr,
) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace_from_directory(
    _directory: &File,
    _name: &std::ffi::CStr,
    _new_path: &Path,
) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(target_os = "linux")]
fn rename_noreplace(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
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
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result =
        unsafe { libc::renamex_np(old_path.as_ptr(), new_path.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_exchange(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
#[path = "safe_file_tests.rs"]
mod tests;

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_noreplace(_old_path: &Path, _new_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_exchange(_old_path: &Path, _new_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(target_os = "macos")]
fn rename_exchange(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let old_path = std::ffi::CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let new_path = std::ffi::CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result =
        unsafe { libc::renamex_np(old_path.as_ptr(), new_path.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
