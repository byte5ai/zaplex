use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use tempfile::tempdir;

use super::*;
use crate::sftp_manager::sftp_backend::{
    local_safe_rename_primitives_available, BackendFileReader, BackendFileWriter,
    BackendOwnershipAnchor, DirectoryReservationFailure, InMemorySftpBackend, StableEntryIdentity,
};
use crate::sftp_manager::sftp_ops::ProgressCallback;
use crate::sftp_manager::types::FileEntry;

struct InstrumentedBackend {
    inner: InMemorySftpBackend,
    supports_exchange: bool,
    fail_exchange_preflight: bool,
    writer_creates: Arc<AtomicU64>,
    read_bytes: Arc<AtomicU64>,
    cancel_on_read: Option<Arc<TransferControl>>,
    isolate_cleanup_failure: bool,
    cleanup_failure_applied: AtomicBool,
    collide_writer_create: Option<u64>,
    collide_directory_stage: bool,
    collide_directory_child: bool,
    replace_stage_on_writer_failure: bool,
    fail_writer_after_chunks_on_create: Option<(u64, u64)>,
    cancel_on_writer_collision: Option<Arc<TransferControl>>,
    symlink_writer_collision: bool,
    empty_list_calls: AtomicU64,
    after_empty_list: Option<(u64, Arc<dyn Fn(&Path) + Send + Sync>)>,
    tokenless_identities: bool,
    unsafe_tokenless_cleanup: bool,
    isolated_cleanup_operation_failure: bool,
    replace_file_stage_before_snapshot: bool,
    replace_directory_stage_before_snapshot: bool,
    replace_backup_before_snapshot: bool,
    artifact_identity_calls: Mutex<HashMap<PathBuf, u64>>,
    reuse_replacement_tokens: bool,
    artifact_reserved_identities: Arc<Mutex<HashMap<PathBuf, StableEntryIdentity>>>,
    replaced_artifacts: Arc<Mutex<HashSet<PathBuf>>>,
    fail_reused_directory_identity_call: Option<u64>,
    fixed_modification_time: Option<SystemTime>,
    hide_modification_time: bool,
    occupy_after_missing_probe: Option<(PathBuf, Vec<u8>, Arc<AtomicBool>)>,
    fail_identity_after_rename: Option<PathBuf>,
    rename_completed: AtomicBool,
}

impl InstrumentedBackend {
    fn new(root: &Path) -> Self {
        Self {
            inner: InMemorySftpBackend::new(root.to_path_buf()),
            supports_exchange: true,
            fail_exchange_preflight: false,
            writer_creates: Arc::new(AtomicU64::new(0)),
            read_bytes: Arc::new(AtomicU64::new(0)),
            cancel_on_read: None,
            isolate_cleanup_failure: false,
            cleanup_failure_applied: AtomicBool::new(false),
            collide_writer_create: None,
            collide_directory_stage: false,
            collide_directory_child: false,
            replace_stage_on_writer_failure: false,
            fail_writer_after_chunks_on_create: None,
            cancel_on_writer_collision: None,
            symlink_writer_collision: false,
            empty_list_calls: AtomicU64::new(0),
            after_empty_list: None,
            tokenless_identities: false,
            unsafe_tokenless_cleanup: false,
            isolated_cleanup_operation_failure: false,
            replace_file_stage_before_snapshot: false,
            replace_directory_stage_before_snapshot: false,
            replace_backup_before_snapshot: false,
            artifact_identity_calls: Mutex::new(HashMap::new()),
            reuse_replacement_tokens: false,
            artifact_reserved_identities: Arc::new(Mutex::new(HashMap::new())),
            replaced_artifacts: Arc::new(Mutex::new(HashSet::new())),
            fail_reused_directory_identity_call: None,
            fixed_modification_time: None,
            hide_modification_time: false,
            occupy_after_missing_probe: None,
            fail_identity_after_rename: None,
            rename_completed: AtomicBool::new(false),
        }
    }

    fn without_exchange(mut self) -> Self {
        self.supports_exchange = false;
        self
    }

    fn with_modification_time(mut self, modified: SystemTime) -> Self {
        self.fixed_modification_time = Some(modified);
        self
    }

    fn without_modification_time(mut self) -> Self {
        self.hide_modification_time = true;
        self
    }

    fn occupy_after_missing_probe(mut self, path: PathBuf, contents: Vec<u8>) -> Self {
        self.occupy_after_missing_probe = Some((path, contents, Arc::new(AtomicBool::new(false))));
        self
    }

    fn with_late_unsupported_exchange(mut self) -> Self {
        self.fail_exchange_preflight = true;
        self
    }

    fn failing_identity_after_rename(mut self, path: PathBuf) -> Self {
        self.fail_identity_after_rename = Some(path);
        self
    }

    fn cancelling_reads(mut self, control: Arc<TransferControl>) -> Self {
        self.cancel_on_read = Some(control);
        self
    }

    fn with_isolated_cleanup_failure(mut self) -> Self {
        self.isolate_cleanup_failure = true;
        self
    }

    fn with_unsafe_tokenless_cleanup(mut self) -> Self {
        self.tokenless_identities = true;
        self.unsafe_tokenless_cleanup = true;
        self
    }

    fn with_isolated_cleanup_operation_failure(mut self) -> Self {
        self.isolated_cleanup_operation_failure = true;
        self
    }

    fn replacing_file_stage_before_snapshot(mut self) -> Self {
        self.replace_file_stage_before_snapshot = true;
        self
    }

    fn replacing_directory_stage_before_snapshot(mut self) -> Self {
        self.replace_directory_stage_before_snapshot = true;
        self
    }

    fn replacing_backup_before_snapshot(mut self) -> Self {
        self.replace_backup_before_snapshot = true;
        self
    }

    fn replacing_file_stage_with_reused_token(mut self) -> Self {
        self.replace_file_stage_before_snapshot = true;
        self.reuse_replacement_tokens = true;
        self
    }

    fn replacing_directory_stage_with_reused_token(mut self) -> Self {
        self.replace_directory_stage_before_snapshot = true;
        self.reuse_replacement_tokens = true;
        self
    }

    fn replacing_backup_with_reused_token(mut self) -> Self {
        self.replace_backup_before_snapshot = true;
        self.reuse_replacement_tokens = true;
        self
    }

    fn colliding_writer_create(mut self, create_number: u64) -> Self {
        self.collide_writer_create = Some(create_number);
        self
    }

    fn colliding_directory_stage(mut self) -> Self {
        self.collide_directory_stage = true;
        self
    }

    fn colliding_directory_child(mut self) -> Self {
        self.collide_directory_child = true;
        self
    }

    fn replacing_stage_on_writer_failure(mut self) -> Self {
        self.replace_stage_on_writer_failure = true;
        self
    }

    fn failing_writer_after_chunks(mut self, create_number: u64, successful_chunks: u64) -> Self {
        self.fail_writer_after_chunks_on_create = Some((create_number, successful_chunks));
        self
    }

    fn cancelling_writer_collision(mut self, control: Arc<TransferControl>) -> Self {
        self.collide_writer_create = Some(1);
        self.cancel_on_writer_collision = Some(control);
        self
    }

    fn symlink_writer_collision(mut self) -> Self {
        self.collide_writer_create = Some(1);
        self.symlink_writer_collision = true;
        self
    }

    fn after_empty_list(
        mut self,
        trigger_call: u64,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Self {
        self.after_empty_list = Some((trigger_call, Arc::new(hook)));
        self
    }

    fn local_path(&self, path: &Path) -> PathBuf {
        self.inner
            .root()
            .join(path.strip_prefix("/").unwrap_or(path))
    }

    fn replace_artifact_before_snapshot(&self, path: &Path) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let (enabled, trigger_call, retained_name) = if name.contains("zaplex-tree") {
            (
                self.replace_directory_stage_before_snapshot,
                if self.reuse_replacement_tokens { 2 } else { 3 },
                "review10-original-directory-stage",
            )
        } else {
            return;
        };
        if !enabled {
            return;
        }
        if self.reuse_replacement_tokens
            && self
                .replaced_artifacts
                .lock()
                .expect("replaced artifacts lock poisoned")
                .contains(path)
        {
            return;
        }
        let call = {
            let mut calls = self
                .artifact_identity_calls
                .lock()
                .expect("artifact identity call lock poisoned");
            let call = calls.entry(path.to_path_buf()).or_default();
            *call += 1;
            *call
        };
        if call != trigger_call {
            return;
        }
        let local = self.local_path(path);
        let retained = local.with_file_name(retained_name);
        fs::rename(&local, retained).unwrap();
        fs::create_dir(&local).unwrap();
        if !self.reuse_replacement_tokens {
            fs::write(local.join("foreign.bin"), b"foreign").unwrap();
        }
        self.replaced_artifacts
            .lock()
            .expect("replaced artifacts lock poisoned")
            .insert(path.to_path_buf());
    }
}

struct InstrumentedReader {
    inner: Box<dyn BackendFileReader>,
    read_bytes: Arc<AtomicU64>,
    cancel_on_read: Option<Arc<TransferControl>>,
}

struct ReplacingFailingWriter {
    inner: Option<Box<dyn BackendFileWriter>>,
    path: PathBuf,
}

struct ReplacingOnDropWriter {
    inner: Option<Box<dyn BackendFileWriter>>,
    path: PathBuf,
    remote_path: PathBuf,
    retained_name: &'static str,
    foreign_bytes: &'static [u8],
    reserved_identities: Arc<Mutex<HashMap<PathBuf, StableEntryIdentity>>>,
    replaced_artifacts: Arc<Mutex<HashSet<PathBuf>>>,
}

struct FailingAfterChunksWriter {
    inner: Box<dyn BackendFileWriter>,
    successful_chunks_remaining: u64,
}

impl BackendFileWriter for FailingAfterChunksWriter {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError> {
        if self.successful_chunks_remaining == 0 {
            return Err(SftpOpsError::Operation(
                "injected writer failure after successful chunks".to_string(),
            ));
        }
        self.inner.write_chunk(buffer)?;
        self.successful_chunks_remaining -= 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        self.inner.flush()
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        self.inner.ownership_anchor()
    }
}

impl BackendFileWriter for ReplacingFailingWriter {
    fn write_chunk(&mut self, _buffer: &[u8]) -> Result<(), SftpOpsError> {
        drop(self.inner.take());
        fs::remove_file(&self.path).unwrap();
        fs::write(&self.path, b"foreign").unwrap();
        Err(SftpOpsError::Operation(
            "injected writer failure after stage replacement".to_string(),
        ))
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        Ok(())
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        match self.inner.as_mut() {
            Some(writer) => writer.ownership_anchor(),
            None => Ok(None),
        }
    }
}

impl BackendFileWriter for ReplacingOnDropWriter {
    fn write_chunk(&mut self, buffer: &[u8]) -> Result<(), SftpOpsError> {
        self.inner
            .as_mut()
            .expect("replacement writer is still open")
            .write_chunk(buffer)
    }

    fn flush(&mut self) -> Result<(), SftpOpsError> {
        self.inner
            .as_mut()
            .expect("replacement writer is still open")
            .flush()
    }

    fn ownership_anchor(
        &mut self,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        let anchor = self
            .inner
            .as_mut()
            .expect("replacement writer is still open")
            .ownership_anchor()?;
        if let Some(anchor) = &anchor {
            let identity = anchor.identity()?;
            self.reserved_identities
                .lock()
                .expect("artifact reserved identities lock poisoned")
                .insert(self.remote_path.clone(), identity);
        }
        Ok(anchor)
    }
}

impl Drop for ReplacingOnDropWriter {
    fn drop(&mut self) {
        drop(self.inner.take());
        if !self.path.exists() {
            return;
        }
        let retained = self.path.with_file_name(self.retained_name);
        fs::rename(&self.path, retained).unwrap();
        fs::write(&self.path, self.foreign_bytes).unwrap();
        self.replaced_artifacts
            .lock()
            .expect("replaced artifacts lock poisoned")
            .insert(self.remote_path.clone());
    }
}

impl BackendFileReader for InstrumentedReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, SftpOpsError> {
        let read = self.inner.read_chunk(buffer)?;
        self.read_bytes.fetch_add(read as u64, Ordering::SeqCst);
        if read > 0 {
            if let Some(control) = self.cancel_on_read.take() {
                control.cancel();
            }
        }
        Ok(read)
    }
}

impl SftpBackend for InstrumentedBackend {
    fn supports_atomic_exchange(&self) -> bool {
        self.supports_exchange
    }

    fn supports_identity_bound_cleanup(&self) -> bool {
        true
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, SftpOpsError> {
        let exists = self.inner.entry_exists(path)?;
        if !exists {
            if let Some((probe_path, contents, occupied)) = &self.occupy_after_missing_probe {
                if path == probe_path && !occupied.swap(true, Ordering::SeqCst) {
                    fs::write(self.local_path(path), contents)?;
                }
            }
        }
        Ok(exists)
    }

    fn existing_entry_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        self.inner.existing_entry_ownership_anchor(path)
    }

    fn preflight_safe_mutation(
        &self,
        path: &Path,
        require_exchange: bool,
    ) -> Result<(), SftpOpsError> {
        if require_exchange && self.fail_exchange_preflight {
            return Err(SftpOpsError::Operation(
                "injected filesystem capability failure".to_string(),
            ));
        }
        if require_exchange && !self.supports_exchange {
            return Err(SftpOpsError::Operation(format!(
                "injected unsupported atomic exchange for {}",
                path.display()
            )));
        }
        Ok(())
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
        let entries = self.inner.list_dir(path)?;
        if entries.is_empty() {
            let call = self.empty_list_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((trigger_call, hook)) = &self.after_empty_list {
                if call == *trigger_call {
                    hook(path);
                }
            }
        }
        Ok(entries)
    }

    fn delete_file(&self, path: &Path) -> Result<(), SftpOpsError> {
        self.inner.delete_file(path)
    }

    fn delete_dir_recursive(&self, path: &Path) -> Result<(), SftpOpsError> {
        self.inner.delete_dir_recursive(path)
    }

    fn create_dir(&self, path: &Path) -> Result<(), SftpOpsError> {
        if self.collide_directory_child
            && path.to_string_lossy().contains("zaplex-tree")
            && path.file_name().is_some_and(|name| name == "child")
        {
            let local = self.local_path(path);
            fs::create_dir(&local).unwrap();
            fs::write(local.join("foreign.bin"), b"foreign").unwrap();
            return Err(SftpOpsError::Operation(
                "injected foreign child collision".to_string(),
            ));
        }
        if self.collide_directory_stage && path.to_string_lossy().contains("zaplex-tree") {
            let local = self.local_path(path);
            fs::create_dir(&local).unwrap();
            fs::write(local.join("foreign.bin"), b"foreign").unwrap();
            return Err(SftpOpsError::Operation(
                "injected foreign directory stage collision".to_string(),
            ));
        }
        self.inner.create_dir(path)?;
        if self.reuse_replacement_tokens
            && self.replace_directory_stage_before_snapshot
            && path.to_string_lossy().contains("zaplex-tree")
        {
            let local = self.local_path(path);
            let retained = local.with_file_name("review11-original-directory-stage");
            fs::rename(&local, retained).unwrap();
            fs::create_dir(&local).unwrap();
            self.replaced_artifacts
                .lock()
                .expect("replaced artifacts lock poisoned")
                .insert(path.to_path_buf());
        }
        Ok(())
    }

    fn create_dir_with_ownership_anchor(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
        if self.collide_directory_child || self.collide_directory_stage {
            self.create_dir(path)?;
            return Ok(None);
        }
        let anchor = self.inner.create_dir_with_ownership_anchor(path)?;
        if self.reuse_replacement_tokens
            && self.replace_directory_stage_before_snapshot
            && path.to_string_lossy().contains("zaplex-tree")
        {
            let local = self.local_path(path);
            let retained = local.with_file_name("review11-original-directory-stage");
            fs::rename(&local, retained).unwrap();
            fs::create_dir(&local).unwrap();
            self.replaced_artifacts
                .lock()
                .expect("replaced artifacts lock poisoned")
                .insert(path.to_path_buf());
        }
        Ok(anchor)
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        self.inner.rename(old_path, new_path)
    }

    fn rename_if_matches(
        &self,
        old_path: &Path,
        new_path: &Path,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Result<(), SftpOpsError> {
        let result = self.inner.rename_if_matches(old_path, new_path, anchor);
        if result.is_ok() {
            self.rename_completed.store(true, Ordering::SeqCst);
        }
        result
    }

    fn replace(&self, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
        if self.supports_exchange && !self.fail_exchange_preflight {
            self.inner.replace(old_path, new_path)
        } else {
            Err(SftpOpsError::Operation(
                "injected unsupported atomic exchange".to_string(),
            ))
        }
    }

    fn delete_file_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
        expected_sha256: &str,
    ) -> Result<(), SftpOpsError> {
        if self.reuse_replacement_tokens
            && self
                .replaced_artifacts
                .lock()
                .expect("replaced artifacts lock poisoned")
                .contains(path)
        {
            return self.inner.delete_file(path);
        }
        if self.unsafe_tokenless_cleanup {
            return self.inner.delete_file(path);
        }
        if self.isolated_cleanup_operation_failure
            && !self.cleanup_failure_applied.swap(true, Ordering::SeqCst)
        {
            let tombstone = path.with_file_name(format!(
                ".{}.zaplex-delete-before-apply",
                path.file_name().unwrap().to_string_lossy()
            ));
            self.inner.rename(path, &tombstone)?;
            return Err(SftpOpsError::Operation(
                "injected isolated cleanup delete failure".to_string(),
            ));
        }
        if self.isolate_cleanup_failure
            && !self.cleanup_failure_applied.swap(true, Ordering::SeqCst)
        {
            let tombstone = path.with_file_name(format!(
                ".{}.zaplex-live-recovery",
                path.file_name().unwrap().to_string_lossy()
            ));
            self.inner.rename(path, &tombstone)?;
            return Err(SftpOpsError::RecoveryRequired {
                message: "injected isolated cleanup acknowledgement failure".to_string(),
                recovery_id: None,
                paths: vec![tombstone],
                committed: false,
            });
        }
        self.inner
            .delete_file_if_matches(path, expected, expected_sha256)
    }

    fn delete_empty_dir_if_matches(
        &self,
        path: &Path,
        expected: &StableEntryIdentity,
    ) -> Result<(), SftpOpsError> {
        if self.reuse_replacement_tokens
            && self
                .replaced_artifacts
                .lock()
                .expect("replaced artifacts lock poisoned")
                .contains(path)
        {
            return self.inner.delete_dir_recursive(path);
        }
        if self.isolated_cleanup_operation_failure
            && !self.cleanup_failure_applied.swap(true, Ordering::SeqCst)
        {
            let tombstone = path.with_file_name(format!(
                ".{}.zaplex-delete-before-apply",
                path.file_name().unwrap().to_string_lossy()
            ));
            self.inner.rename(path, &tombstone)?;
            return Err(SftpOpsError::Operation(
                "injected isolated directory cleanup delete failure".to_string(),
            ));
        }
        self.inner.delete_empty_dir_if_matches(path, expected)
    }

    fn realpath(&self, path: &Path) -> Result<PathBuf, SftpOpsError> {
        self.inner.realpath(path)
    }

    fn stat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        self.inner.stat(path)
    }

    fn lstat(&self, path: &Path) -> Result<FileEntry, SftpOpsError> {
        self.inner.lstat(path)
    }

    fn modification_time(&self, path: &Path) -> Result<Option<SystemTime>, SftpOpsError> {
        if self.hide_modification_time {
            return Ok(None);
        }
        Ok(self
            .fixed_modification_time
            .or(self.inner.modification_time(path)?))
    }

    fn stable_identity(&self, path: &Path) -> Result<StableEntryIdentity, SftpOpsError> {
        if self.rename_completed.load(Ordering::SeqCst)
            && self.fail_identity_after_rename.as_deref() == Some(path)
        {
            return Err(SftpOpsError::Operation(
                "injected post-rename identity failure".to_string(),
            ));
        }
        self.replace_artifact_before_snapshot(path);
        let mut identity = self.inner.stable_identity(path)?;
        if self.reuse_replacement_tokens {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            if name.contains("zaplex-") {
                let replaced = self
                    .replaced_artifacts
                    .lock()
                    .expect("replaced artifacts lock poisoned")
                    .contains(path);
                let mut reserved = self
                    .artifact_reserved_identities
                    .lock()
                    .expect("artifact reserved identities lock poisoned");
                if replaced {
                    if let Some(original) = reserved.get(path) {
                        identity.object_id = original.object_id.clone();
                    }
                } else {
                    reserved
                        .entry(path.to_path_buf())
                        .or_insert_with(|| identity.clone());
                }
                let call = self
                    .artifact_identity_calls
                    .lock()
                    .expect("artifact identity call lock poisoned")
                    .get(path)
                    .copied();
                if replaced
                    && self
                        .fail_reused_directory_identity_call
                        .is_some_and(|expected| call == Some(expected))
                {
                    return Err(SftpOpsError::Operation(
                        "injected verification failure after reused directory token".to_string(),
                    ));
                }
            }
        }
        if self.tokenless_identities {
            identity.object_id.clear();
        }
        Ok(identity)
    }

    fn open_file_reader(&self, path: &Path) -> Result<Box<dyn BackendFileReader>, SftpOpsError> {
        Ok(Box::new(InstrumentedReader {
            inner: self.inner.open_file_reader(path)?,
            read_bytes: self.read_bytes.clone(),
            cancel_on_read: self.cancel_on_read.clone(),
        }))
    }

    fn create_file_writer(&self, path: &Path) -> Result<Box<dyn BackendFileWriter>, SftpOpsError> {
        let create_number = self.writer_creates.fetch_add(1, Ordering::SeqCst) + 1;
        if self.collide_writer_create == Some(create_number) {
            #[cfg(unix)]
            if self.symlink_writer_collision {
                std::os::unix::fs::symlink("foreign-target", self.local_path(path)).unwrap();
            } else {
                fs::write(self.local_path(path), b"foreign").unwrap();
            }
            #[cfg(not(unix))]
            fs::write(self.local_path(path), b"foreign").unwrap();
            if let Some(control) = &self.cancel_on_writer_collision {
                control.cancel();
            }
            return Err(SftpOpsError::Operation(
                "injected foreign file reservation collision".to_string(),
            ));
        }
        let writer = self.inner.create_file_writer(path)?;
        if self.replace_stage_on_writer_failure && create_number == 1 {
            return Ok(Box::new(ReplacingFailingWriter {
                inner: Some(writer),
                path: self.local_path(path),
            }));
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        if (self.replace_file_stage_before_snapshot && name.contains("zaplex-transfer"))
            || (self.replace_backup_before_snapshot && name.contains("zaplex-backup"))
        {
            let retained_name = if name.contains("zaplex-transfer") {
                "review10-original-file-stage"
            } else {
                "review10-original-backup"
            };
            return Ok(Box::new(ReplacingOnDropWriter {
                inner: Some(writer),
                path: self.local_path(path),
                remote_path: path.to_path_buf(),
                retained_name,
                foreign_bytes: if !self.reuse_replacement_tokens {
                    b"foreign"
                } else if name.contains("zaplex-transfer") {
                    b"forged"
                } else {
                    b"bad"
                },
                reserved_identities: self.artifact_reserved_identities.clone(),
                replaced_artifacts: self.replaced_artifacts.clone(),
            }));
        }
        if let Some((failure_create, successful_chunks)) = self.fail_writer_after_chunks_on_create {
            if create_number == failure_create {
                return Ok(Box::new(FailingAfterChunksWriter {
                    inner: writer,
                    successful_chunks_remaining: successful_chunks,
                }));
            }
        }
        Ok(writer)
    }

    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        self.inner
            .upload_file(local_path, remote_path, progress_cb, cancel_flag)
    }

    fn download_file(
        &self,
        remote_path: &Path,
        local_path: &Path,
        progress_cb: Option<&ProgressCallback>,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), SftpOpsError> {
        self.inner
            .download_file(remote_path, local_path, progress_cb, cancel_flag)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), SftpOpsError> {
        self.inner.copy_file(src, dst)
    }
}

fn backend(root: &std::path::Path) -> Arc<dyn SftpBackend> {
    Arc::new(InMemorySftpBackend::new(root.to_path_buf()))
}

fn job(
    source_root: &std::path::Path,
    target_root: &std::path::Path,
    operation: TransferOperation,
    conflict: ConflictDecision,
) -> TransferJob {
    TransferJob {
        source_backend: backend(source_root),
        target_backend: backend(target_root),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation,
        conflict,
    }
}

fn directory_job(
    source_backend: Arc<dyn SftpBackend>,
    target_backend: Arc<dyn SftpBackend>,
    operation: TransferOperation,
    conflict: ConflictDecision,
) -> TransferJob {
    TransferJob {
        source_backend,
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source"),
        target_path: PathBuf::from("/target"),
        operation,
        conflict,
    }
}

#[test]
fn conflict_rename_keeps_destination_and_uses_deterministic_available_name() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"destination").unwrap();
    fs::write(target.path().join("target (copy).bin"), b"occupied").unwrap();

    let outcome = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Copy,
            ConflictDecision::Rename,
        ),
        &TransferControl::default(),
        None,
    )
    .unwrap();

    assert_eq!(outcome, TransferOutcome::Completed);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"destination"
    );
    assert_eq!(
        fs::read(target.path().join("target (copy).bin")).unwrap(),
        b"occupied"
    );
    assert_eq!(
        fs::read(target.path().join("target (copy 2).bin")).unwrap(),
        b"source"
    );
}

#[test]
fn conflict_rename_never_overwrites_a_name_claimed_after_probe() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"destination").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InstrumentedBackend::new(target.path()).occupy_after_missing_probe(
            PathBuf::from("/target (copy).bin"),
            b"racing writer".to_vec(),
        ),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Rename,
    };

    assert_eq!(
        run_transfer(&transfer, &TransferControl::default(), None).unwrap(),
        TransferOutcome::Completed
    );
    assert_eq!(
        fs::read(target.path().join("target (copy).bin")).unwrap(),
        b"racing writer"
    );
    assert_eq!(
        fs::read(target.path().join("target (copy 2).bin")).unwrap(),
        b"source"
    );
}

#[cfg(unix)]
#[test]
fn recursive_move_preserves_symlinks_and_empty_directories() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    fs::create_dir_all(root.path().join("source/empty/nested")).unwrap();
    fs::write(root.path().join("source/file.txt"), b"source").unwrap();
    symlink("file.txt", root.path().join("source/link.txt")).unwrap();
    let shared_backend = backend(root.path());
    let transfer = TransferJob {
        source_backend: shared_backend.clone(),
        target_backend: shared_backend,
        source_path: PathBuf::from("/source"),
        target_path: PathBuf::from("/target"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_directory_transfer(&transfer, &TransferControl::default(), None).unwrap(),
        TransferOutcome::Completed
    );
    assert!(!root.path().join("source").exists());
    assert!(root.path().join("target/empty/nested").is_dir());
    assert!(fs::symlink_metadata(root.path().join("target/link.txt"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(root.path().join("target/link.txt")).unwrap(),
        PathBuf::from("file.txt")
    );
}

#[test]
fn newer_only_overwrites_only_when_source_timestamp_is_strictly_newer() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new source").unwrap();
    fs::write(target.path().join("target.bin"), b"old destination").unwrap();
    let source_backend = Arc::new(
        InstrumentedBackend::new(source.path())
            .with_modification_time(SystemTime::UNIX_EPOCH + Duration::from_secs(20)),
    ) as Arc<dyn SftpBackend>;
    let target_backend = Arc::new(
        InstrumentedBackend::new(target.path())
            .with_modification_time(SystemTime::UNIX_EPOCH + Duration::from_secs(10)),
    ) as Arc<dyn SftpBackend>;
    let transfer = TransferJob {
        source_backend,
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::NewerOnly,
    };

    let outcome = run_transfer(&transfer, &TransferControl::default(), None).unwrap();

    assert_eq!(outcome, TransferOutcome::Completed);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"new source"
    );
}

#[test]
fn newer_only_skips_when_source_is_not_strictly_newer() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"old source").unwrap();
    fs::write(target.path().join("target.bin"), b"new destination").unwrap();
    let source_backend = Arc::new(
        InstrumentedBackend::new(source.path())
            .with_modification_time(SystemTime::UNIX_EPOCH + Duration::from_secs(10)),
    ) as Arc<dyn SftpBackend>;
    let target_backend = Arc::new(
        InstrumentedBackend::new(target.path())
            .with_modification_time(SystemTime::UNIX_EPOCH + Duration::from_secs(20)),
    ) as Arc<dyn SftpBackend>;
    let transfer = TransferJob {
        source_backend,
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::NewerOnly,
    };

    let outcome = run_transfer(&transfer, &TransferControl::default(), None).unwrap();

    assert_eq!(outcome, TransferOutcome::Skipped);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"new destination"
    );
}

#[test]
fn newer_only_skips_when_recency_cannot_be_proven() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"destination").unwrap();
    let transfer = TransferJob {
        source_backend: Arc::new(
            InstrumentedBackend::new(source.path()).without_modification_time(),
        ),
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::NewerOnly,
    };

    assert_eq!(
        run_transfer(&transfer, &TransferControl::default(), None).unwrap(),
        TransferOutcome::Skipped
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"destination"
    );
}

#[test]
fn newer_only_never_replaces_an_existing_directory_tree() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(source.path().join("source/source-only.txt"), b"source").unwrap();
    fs::write(target.path().join("target/target-only.txt"), b"target").unwrap();
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source"),
        target_path: PathBuf::from("/target"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::NewerOnly,
    };

    assert_eq!(
        run_directory_transfer(&transfer, &TransferControl::default(), None).unwrap(),
        TransferOutcome::Skipped
    );
    assert_eq!(
        fs::read(target.path().join("target/target-only.txt")).unwrap(),
        b"target"
    );
    assert!(!target.path().join("target/source-only.txt").exists());
}

#[test]
fn large_copy_streams_without_buffering_entire_file() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let bytes = vec![0x5a; STREAM_CHUNK_SIZE * 3 + 17];
    fs::write(source.path().join("source.bin"), &bytes).unwrap();
    let mut samples = Vec::new();

    let outcome = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        Some(&mut |progress| samples.push(progress.transferred)),
    )
    .unwrap();

    assert_eq!(outcome, TransferOutcome::Completed);
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), bytes);
    assert!(samples
        .windows(2)
        .all(|pair| { pair[1].saturating_sub(pair[0]) <= STREAM_CHUNK_SIZE as u64 }));
}

#[test]
fn cancel_preserves_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let bytes = vec![0x6b; STREAM_CHUNK_SIZE * 2];
    fs::write(source.path().join("source.bin"), &bytes).unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let control = TransferControl::default();

    let result = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &control,
        Some(&mut |_| {
            control.cancel();
        }),
    );

    assert!(matches!(result, Err(SftpOpsError::Cancelled)));
    assert_eq!(fs::read(source.path().join("source.bin")).unwrap(), bytes);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
}

#[test]
fn cancelled_file_transfer_removes_owned_stage_and_returns_cancelled() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let bytes = vec![0x5c; STREAM_CHUNK_SIZE * 3 + 1];
    fs::write(source.path().join("source.bin"), &bytes).unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let control = TransferControl::default();

    let result = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &control,
        Some(&mut |progress| {
            if progress.phase == TransferPhase::Transferring && progress.transferred > 0 {
                assert!(control.cancel());
            }
        }),
    );

    assert!(matches!(result, Err(SftpOpsError::Cancelled)));
    assert_eq!(fs::read(source.path().join("source.bin")).unwrap(), bytes);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
    assert!(
        transfer_artifacts(target.path(), "zaplex-transfer").is_empty(),
        "cancelling after a staged write must remove the owned stage"
    );
    assert!(control.is_cancelled());
}

#[test]
fn failed_remote_to_remote_move_never_deletes_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"relay").unwrap();
    let mut relay = job(
        source.path(),
        target.path(),
        TransferOperation::Move,
        ConflictDecision::Overwrite,
    );
    relay.target_path = PathBuf::from("/missing/target.bin");

    assert!(run_transfer(&relay, &TransferControl::default(), None).is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"relay"
    );
    assert!(!target.path().join("missing/target.bin").exists());
}

#[test]
fn tokenless_move_source_is_rejected_before_streaming_and_preserved() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_backend =
        Arc::new(InstrumentedBackend::new(source.path()).with_unsafe_tokenless_cleanup());
    let target_backend = Arc::new(InstrumentedBackend::new(target.path()));
    let transfer = TransferJob {
        source_backend: source_backend.clone(),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a move without an immutable source token must fail before streaming");

    assert_eq!(source_backend.read_bytes.load(Ordering::SeqCst), 0);
    assert_eq!(target_backend.writer_creates.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert!(!target.path().join("target.bin").exists());
}

#[test]
fn isolated_delete_failure_never_turns_missing_move_source_into_success() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_backend =
        Arc::new(InstrumentedBackend::new(source.path()).with_isolated_cleanup_operation_failure());
    let transfer = TransferJob {
        source_backend,
        target_backend: Arc::new(InstrumentedBackend::new(target.path())),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an unconfirmed tombstone delete must not complete the move");

    assert!(
        source.path().join("source.bin").exists() || error.recovery_id().is_some(),
        "the source must be restored or represented by retryable recovery"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn isolated_directory_delete_failure_retains_retryable_move_recovery() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    let source_backend =
        Arc::new(InstrumentedBackend::new(source.path()).with_isolated_cleanup_operation_failure());
    let transfer = directory_job(
        source_backend,
        Arc::new(InstrumentedBackend::new(target.path())),
        TransferOperation::Move,
        ConflictDecision::Overwrite,
    );

    let error = run_directory_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an unconfirmed directory tombstone delete must not complete the move");

    assert!(
        source.path().join("source").exists() || error.recovery_id().is_some(),
        "the directory source must be restored or represented by retryable recovery"
    );
    assert!(target.path().join("target").is_dir());
}

#[test]
fn unsupported_exchange_is_rejected_before_streaming() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_backend = Arc::new(InstrumentedBackend::new(target.path()).without_exchange());
    let writer_creates = target_backend.writer_creates.clone();
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an unsafe overwrite backend must fail before streaming");

    assert_eq!(writer_creates.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(source.path().join("source.bin")).unwrap(), b"new");
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"old");
}

#[test]
fn skip_preserves_existing_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();

    let outcome = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Move,
            ConflictDecision::Skip,
        ),
        &TransferControl::default(),
        None,
    )
    .unwrap();

    assert_eq!(outcome, TransferOutcome::Skipped);
    assert_eq!(fs::read(source.path().join("source.bin")).unwrap(), b"new");
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
}

#[test]
fn stable_identity_is_revalidated_before_move_delete() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![0x31; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let source_path = source.path().join("source.bin");
    let mut changed = false;

    let result = run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        Some(&mut |_| {
            if !changed {
                changed = true;
                fs::write(&source_path, b"changed while transferring").unwrap();
            }
        }),
    );

    assert!(result.is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"changed while transferring"
    );
    assert!(!target.path().join("target.bin").exists());
}

#[test]
fn lstat_error_is_not_treated_as_missing_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_lstat_error(PathBuf::from("/target.bin")),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert!(run_transfer(&transfer, &TransferControl::default(), None).is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
}

#[test]
fn publish_failure_preserves_source_and_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::create_dir(target.path().join("target.bin")).unwrap();
    fs::write(target.path().join("target.bin/existing"), b"existing").unwrap();

    assert!(run_transfer(
        &job(
            source.path(),
            target.path(),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin/existing")).unwrap(),
        b"existing"
    );
}

#[test]
fn move_deletes_source_only_after_verified_destination_commit() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_staged_identity_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert!(run_transfer(&transfer, &TransferControl::default(), None).is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
}

#[test]
fn published_verify_failure_restores_existing_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_published_identity_failure(PathBuf::from("/target.bin")),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert!(run_transfer(&transfer, &TransferControl::default(), None).is_err());
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"existing"
    );
}

#[test]
fn delete_acknowledgement_error_keeps_verified_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"existing").unwrap();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_delete_after_apply_failure(PathBuf::from("/source.bin")),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an indeterminate source-delete acknowledgement must surface");

    assert!(
        error.to_string().contains("remains committed"),
        "the partial-commit status must explain why the destination is retained"
    );
    assert!(!source.path().join("source.bin").exists());
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source",
        "the verified destination is the recovery copy after delete acknowledgement fails"
    );
}

#[test]
fn progress_eta_and_pause_are_observable() {
    let mut tracker = ProgressTracker::new(1_000);
    let progress = tracker.record_at(250, Duration::from_secs(2));
    assert_eq!(progress.transferred, 250);
    assert_eq!(progress.total, 1_000);
    assert_eq!(progress.bytes_per_second, 125);
    assert_eq!(progress.eta, Some(Duration::from_secs(6)));

    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![0x72; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let control = Arc::new(TransferControl::default());
    control.pause();
    let worker_control = control.clone();
    let transfer = job(
        source.path(),
        target.path(),
        TransferOperation::Copy,
        ConflictDecision::Overwrite,
    );
    let worker = thread::spawn(move || run_transfer(&transfer, &worker_control, None));
    thread::sleep(Duration::from_millis(20));
    assert_eq!(control.progress().transferred, 0);
    control.resume();
    assert_eq!(worker.join().unwrap().unwrap(), TransferOutcome::Completed);
}

#[test]
fn verification_hash_honors_cancel_and_reports_progress() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![0x75; STREAM_CHUNK_SIZE * 3],
    )
    .unwrap();
    let control = Arc::new(TransferControl::default());
    let source_backend =
        Arc::new(InstrumentedBackend::new(source.path()).cancelling_reads(control.clone()));
    let read_bytes = source_backend.read_bytes.clone();
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut samples = Vec::new();

    run_transfer(
        &transfer,
        &control,
        Some(&mut |progress| samples.push(progress)),
    )
    .expect_err("cancelling during source verification must stop the hash");

    assert!(
        read_bytes.load(Ordering::SeqCst) <= STREAM_CHUNK_SIZE as u64,
        "verification must check cancellation between bounded reads"
    );
    assert!(
        !samples.is_empty(),
        "verification reads must publish visible progress"
    );
    assert!(samples
        .iter()
        .all(|progress| progress.phase == crate::sftp_manager::types::TransferPhase::Verifying));
}

#[test]
fn isolated_backend_cleanup_failure_registers_safe_retry() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(source.path()).with_isolated_cleanup_failure());
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("isolated cleanup failure must remain retryable");
    let recovery_id = error
        .recovery_id()
        .expect("isolated backend path must be registered for safe retry");
    assert!(error.destination_committed());
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
    assert!(!source.path().join("source.bin").exists());

    retry_recovery(recovery_id).expect("identity-bound retry must remove the isolated source");
    assert!(fs::read_dir(source.path()).unwrap().next().is_none());
}

#[test]
fn file_target_is_reverified_after_source_quarantine() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_root = target.path().to_path_buf();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename(move |path| {
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
            {
                fs::write(target_root.join("target.bin"), b"evil!!").unwrap();
            }
        }),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("post-quarantine target mutation must block source cleanup");

    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"evil!!"
    );
    assert!(error
        .recovery_paths()
        .iter()
        .any(|path| path == Path::new("/target.bin")));
}

#[test]
fn directory_target_is_reverified_after_source_quarantine() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let target_root = target.path().to_path_buf();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename(move |path| {
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
            {
                fs::write(target_root.join("target/a.bin"), b"evil!!").unwrap();
            }
        }),
    );

    let error = run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("post-quarantine target tree mutation must block source cleanup");

    assert_eq!(
        fs::read(source.path().join("source/a.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target/a.bin")).unwrap(),
        b"evil!!"
    );
    assert!(error
        .recovery_paths()
        .iter()
        .any(|path| path == Path::new("/target")));
}

#[test]
fn review12_file_move_keeps_a_foreign_quarantine_replacement() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_rename(
            move |_, quarantine| {
                if quarantine
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !hook_replaced.swap(true, Ordering::SeqCst)
                {
                    fs::rename(quarantine, source_root.join("review12-original-source.bin"))
                        .unwrap();
                    fs::write(quarantine, b"source").unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a foreign quarantine replacement must stop move cleanup");

    assert!(replaced.load(Ordering::SeqCst));
    assert!(fs::read_dir(source.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
            && fs::read(path).is_ok_and(|contents| contents == b"source")
    }));
    assert!(error.recovery_id().is_some());
}

#[test]
fn review12_directory_move_keeps_a_foreign_quarantine_replacement() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_rename(
            move |_, quarantine| {
                if quarantine
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !hook_replaced.swap(true, Ordering::SeqCst)
                {
                    fs::rename(quarantine, source_root.join("review12-original-source")).unwrap();
                    fs::create_dir(quarantine).unwrap();
                    fs::write(quarantine.join("a.bin"), b"source").unwrap();
                }
            },
        ),
    );

    let error = run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("a foreign directory quarantine replacement must stop move cleanup");

    assert!(replaced.load(Ordering::SeqCst));
    assert!(fs::read_dir(source.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
            && fs::read(path.join("a.bin")).is_ok_and(|contents| contents == b"source")
    }));
    assert!(error.recovery_id().is_some());
}

#[test]
fn review13_file_move_revalidates_source_after_finalizing_before_quarantine() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let original = source.path().join("review13-original-source.bin");
    let source_path = source.path().join("source.bin");
    let control = TransferControl::default();
    control.set_after_finalizing_hook(2, {
        let original = original.clone();
        let source_path = source_path.clone();
        move || {
            fs::rename(&source_path, &original).unwrap();
            fs::write(&source_path, b"source").unwrap();
        }
    });
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &control, None)
        .expect_err("the source replacement must stop before quarantine mutation");

    assert_eq!(fs::read(&source_path).unwrap(), b"source");
    assert_eq!(fs::read(&original).unwrap(), b"source");
}

#[test]
fn review13_directory_move_revalidates_source_after_finalizing_before_quarantine() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let original = source.path().join("review13-original-source");
    let source_path = source.path().join("source");
    let control = TransferControl::default();
    control.set_after_finalizing_hook(2, {
        let original = original.clone();
        let source_path = source_path.clone();
        move || {
            fs::rename(&source_path, &original).unwrap();
            fs::create_dir(&source_path).unwrap();
            fs::write(source_path.join("a.bin"), b"source").unwrap();
        }
    });

    run_directory_transfer(
        &directory_job(
            backend(source.path()),
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &control,
        None,
    )
    .expect_err("the directory replacement must stop before quarantine mutation");

    assert_eq!(fs::read(source_path.join("a.bin")).unwrap(), b"source");
    assert_eq!(fs::read(original.join("a.bin")).unwrap(), b"source");
}

#[test]
fn review13_file_quarantine_recovery_restores_the_anchored_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_rename(
            move |old_path, new_path| {
                if new_path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                {
                    fs::write(old_path, b"foreign").unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the occupied restore path must create recovery");
    let recovery_id = error.recovery_id().expect("recovery must be retryable");
    fs::remove_file(source_root.join("source.bin")).unwrap();

    retry_recovery(recovery_id).expect("retry must restore the anchored source");

    assert_eq!(fs::read(source_root.join("source.bin")).unwrap(), b"source");
    assert!(!fs::read_dir(&source_root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-source")
    }));
}

#[test]
fn review13_directory_quarantine_recovery_restores_the_anchored_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_rename(
            move |old_path, new_path| {
                if new_path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                {
                    fs::create_dir(old_path).unwrap();
                    fs::write(old_path.join("foreign.bin"), b"foreign").unwrap();
                }
            },
        ),
    );

    let error = run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the occupied directory restore path must create recovery");
    let recovery_id = error.recovery_id().expect("recovery must be retryable");
    fs::remove_dir_all(source_root.join("source")).unwrap();

    retry_recovery(recovery_id).expect("retry must restore the anchored directory source");

    assert_eq!(
        fs::read(source_root.join("source/a.bin")).unwrap(),
        b"source"
    );
    assert!(!fs::read_dir(&source_root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-source")
    }));
}

#[test]
fn review13_directory_post_create_recovery_removes_the_owned_private_reservation() {
    let target = tempdir().unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_directory_reservation_failure(DirectoryReservationFailure::Publish),
    );

    let error = match target_backend.create_dir_with_ownership_anchor(Path::new("/stage")) {
        Ok(_) => panic!("the injected post-create step must fail"),
        Err(error) => error,
    };
    let error = retryable_backend_recovery(error, target_backend, Path::new("/stage"));
    let recovery_id = error.recovery_id().expect("recovery must be retryable");

    retry_recovery(recovery_id).expect("retry must clean the owned private reservation");

    assert!(fs::read_dir(target.path()).unwrap().next().is_none());
}

#[test]
fn replace_applied_then_error_is_resolved_as_committed() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_replace_after_apply_failure(PathBuf::from("/target.bin")),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_transfer(&transfer, &TransferControl::default(), None).unwrap(),
        TransferOutcome::Completed
    );
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"new");
}

#[test]
fn target_change_after_backup_is_never_overwritten() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_root = target.path().to_path_buf();
    let changed = Arc::new(AtomicBool::new(false));
    let hook_changed = changed.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_after_stable_identity(
            move |path| {
                if path.to_string_lossy().contains(".zaplex-backup-")
                    && !hook_changed.swap(true, Ordering::SeqCst)
                {
                    fs::write(target_root.join("target.bin"), b"late").unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    assert!(run_transfer(&transfer, &TransferControl::default(), None).is_err());
    assert!(changed.load(Ordering::SeqCst), "the race hook must run");
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"late",
        "a destination changed after backup verification must remain untouched"
    );
    assert!(source.path().join("source.bin").exists());
}

#[test]
fn directory_file_failure_leaves_no_partial_new_target() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.txt"), b"a").unwrap();
    fs::write(source.path().join("source/b.txt"), b"b").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_writer_failure_on_create(2),
    );

    assert!(run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .is_err());
    assert!(
        !target.path().join("target").exists(),
        "a failed staged directory must not expose a partial destination"
    );
}

#[test]
fn own_file_stage_with_completed_chunk_is_cleaned_after_writer_failure() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![0x51; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).failing_writer_after_chunks(1, 1));
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the injected second-chunk writer failure must abort");

    assert!(
        transfer_artifacts(target.path(), "zaplex-transfer").is_empty(),
        "a stage that still has its reserved filesystem object must be cleaned"
    );
}

#[test]
fn own_directory_stage_with_completed_child_is_cleaned_after_later_failure() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.txt"), b"a").unwrap();
    fs::write(source.path().join("source/b.txt"), b"b").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_writer_failure_on_create(2),
    );

    let error = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the injected second-child writer failure must abort");

    let artifacts = transfer_artifacts(target.path(), "zaplex-tree");
    assert!(
        artifacts.is_empty(),
        "a directory stage and completed owned children must be cleaned: {error:?}; {artifacts:?}"
    );
}

#[test]
fn cancelled_transfer_preserves_source_and_removes_partial_output() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(
        source.path().join("source/a.txt"),
        vec![0x41; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let control = TransferControl::default();

    let result = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            backend(target.path()),
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &control,
        Some(&mut |progress| {
            if progress.phase == crate::sftp_manager::types::TransferPhase::Transferring
                && progress.transferred > 0
            {
                control.cancel();
            }
        }),
    );

    assert!(result.is_err());
    assert!(source.path().join("source/a.txt").exists());
    assert!(
        !target.path().join("target").exists(),
        "cancelling a directory stage must leave no visible partial destination"
    );
}

#[test]
fn directory_failure_preserves_existing_target_tree_exactly() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.txt"), b"a").unwrap();
    fs::write(source.path().join("source/b.txt"), b"b").unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(target.path().join("target/old.txt"), b"old").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_writer_failure_on_create(2),
    );

    assert!(run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .is_err());
    assert_eq!(
        fs::read_dir(target.path().join("target"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("old.txt")]
    );
    assert_eq!(
        fs::read(target.path().join("target/old.txt")).unwrap(),
        b"old"
    );
}

#[test]
fn review9_directory_merge_skip_commits_non_conflicts_and_preserves_move_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(
        source.path().join("source/conflict.txt"),
        b"source-conflict",
    )
    .unwrap();
    fs::write(source.path().join("source/new.txt"), b"new").unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(
        target.path().join("target/conflict.txt"),
        b"destination-conflict",
    )
    .unwrap();
    let transfer = directory_job(
        backend(source.path()),
        backend(target.path()),
        TransferOperation::Move,
        ConflictDecision::MergeSkip,
    );

    let outcome = run_directory_transfer(&transfer, &TransferControl::default(), None).unwrap();

    assert_eq!(
        outcome,
        TransferOutcome::PartiallyCompleted {
            transferred: 1,
            published: 1,
            skipped: 1,
            source_kept: true,
        }
    );
    assert_eq!(
        fs::read(target.path().join("target/conflict.txt")).unwrap(),
        b"destination-conflict"
    );
    assert_eq!(
        fs::read(target.path().join("target/new.txt")).unwrap(),
        b"new"
    );
    assert_eq!(
        fs::read(source.path().join("source/conflict.txt")).unwrap(),
        b"source-conflict"
    );
    assert_eq!(
        fs::read(source.path().join("source/new.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn review9_directory_merge_skip_reports_full_skip_when_no_file_is_published() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/conflict.txt"), b"source").unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(target.path().join("target/conflict.txt"), b"destination").unwrap();
    let transfer = directory_job(
        backend(source.path()),
        backend(target.path()),
        TransferOperation::Copy,
        ConflictDecision::MergeSkip,
    );

    let outcome = run_directory_transfer(&transfer, &TransferControl::default(), None).unwrap();

    assert_eq!(outcome, TransferOutcome::Skipped);
    assert_eq!(
        fs::read(target.path().join("target/conflict.txt")).unwrap(),
        b"destination"
    );
}

#[test]
fn review10_file_stage_replacement_before_snapshot_is_never_rebound_or_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).replacing_file_stage_before_snapshot());
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a replaced stage must never be rebound to the transfer");

    let stages = transfer_artifacts(target.path(), "zaplex-transfer");
    assert_eq!(stages.len(), 1, "the foreign replacement must remain");
    assert_eq!(fs::read(&stages[0]).unwrap(), b"foreign");
    assert!(error.recovery_id().is_some());
}

#[test]
fn review10_directory_stage_replacement_before_snapshot_is_never_rebound_or_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InstrumentedBackend::new(target.path()).replacing_directory_stage_before_snapshot(),
    );

    let error = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("a replaced directory stage must never be rebound to the transfer");

    let stages = transfer_artifacts(target.path(), "zaplex-tree");
    assert_eq!(stages.len(), 1, "the foreign replacement must remain");
    assert_eq!(fs::read(stages[0].join("foreign.bin")).unwrap(), b"foreign");
    assert!(error.recovery_id().is_some());
}

#[test]
fn review10_backup_replacement_before_snapshot_is_never_rebound_or_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).replacing_backup_before_snapshot());
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a replaced backup must never be rebound to the transfer");

    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"old");
    let backups = transfer_artifacts(target.path(), "zaplex-backup");
    assert_eq!(backups.len(), 1, "the foreign replacement must remain");
    assert_eq!(fs::read(&backups[0]).unwrap(), b"foreign");
    assert!(error.recovery_id().is_some());
}

#[test]
fn review11_file_stage_reused_token_remains_foreign_after_writer_drop() {
    let target = tempdir().unwrap();
    let backend =
        Arc::new(InstrumentedBackend::new(target.path()).replacing_file_stage_with_reused_token());
    let backend_trait: Arc<dyn SftpBackend> = backend.clone();
    let path = PathBuf::from("/.target.zaplex-transfer-review11");
    let (mut writer, mut ownership) =
        create_owned_writer(&*backend, &path, PathOwnership::empty(&path))
            .unwrap_or_else(|failure| panic!("file reservation must succeed: {}", failure.error));
    writer.write_chunk(b"source").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let snapshot = capture_snapshot(&*backend, &path).unwrap();
    let bind_error = bind_snapshot_to_reserved_ownership(&mut ownership, &snapshot)
        .expect_err("the live file anchor must reject the replacement");

    let error = cleanup_failed_stage(
        bind_error,
        backend_trait,
        &path,
        ownership,
        false,
        &TransferControl::default(),
        &mut None,
    )
    .expect_err("cleanup must not delete a replacement with a reused visible token");

    assert_eq!(
        fs::read(target.path().join(".target.zaplex-transfer-review11")).unwrap(),
        b"forged"
    );
    assert!(error.recovery_id().is_some());
}

#[test]
fn review11_directory_stage_reused_token_remains_foreign_after_create() {
    let target = tempdir().unwrap();
    let backend = Arc::new(
        InstrumentedBackend::new(target.path()).replacing_directory_stage_with_reused_token(),
    );
    let backend_trait: Arc<dyn SftpBackend> = backend.clone();
    let path = PathBuf::from("/.target.zaplex-tree-review11");
    let mut ownership = create_owned_directory(&*backend, &path, PathOwnership::empty(&path))
        .unwrap_or_else(|failure| panic!("directory reservation must succeed: {}", failure.error));
    let snapshot = capture_snapshot(&*backend, &path).unwrap();
    bind_snapshot_to_reserved_ownership(&mut ownership, &snapshot).unwrap();

    let error = cleanup_failed_stage(
        SftpOpsError::Operation("injected post-bind failure".to_string()),
        backend_trait,
        &path,
        ownership,
        false,
        &TransferControl::default(),
        &mut None,
    )
    .expect_err("cleanup must not delete a replacement directory with a reused visible token");

    assert!(target.path().join(".target.zaplex-tree-review11").is_dir());
    assert!(error.recovery_id().is_some());
}

#[test]
fn review11_backup_reused_token_remains_foreign_after_writer_drop() {
    let target = tempdir().unwrap();
    let backend =
        Arc::new(InstrumentedBackend::new(target.path()).replacing_backup_with_reused_token());
    let backend_trait: Arc<dyn SftpBackend> = backend.clone();
    let path = PathBuf::from("/.target.zaplex-backup-review11");
    let (mut writer, mut ownership) =
        create_owned_writer(&*backend, &path, PathOwnership::empty(&path))
            .unwrap_or_else(|failure| panic!("backup reservation must succeed: {}", failure.error));
    writer.write_chunk(b"old").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let snapshot = capture_snapshot(&*backend, &path).unwrap();
    let bind_error = bind_snapshot_to_reserved_ownership(&mut ownership, &snapshot)
        .expect_err("the live backup anchor must reject the replacement");

    let error = cleanup_failed_stage(
        bind_error,
        backend_trait,
        &path,
        ownership,
        false,
        &TransferControl::default(),
        &mut None,
    )
    .expect_err("cleanup must not delete a replacement backup with a reused visible token");

    assert_eq!(
        fs::read(target.path().join(".target.zaplex-backup-review11")).unwrap(),
        b"bad"
    );
    assert!(error.recovery_id().is_some());
}

#[test]
fn review10_merge_skip_with_new_empty_directory_is_partial_for_copy() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir_all(source.path().join("source/new-empty")).unwrap();
    fs::write(source.path().join("source/conflict.txt"), b"source").unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(target.path().join("target/conflict.txt"), b"target").unwrap();

    let outcome = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            backend(target.path()),
            TransferOperation::Copy,
            ConflictDecision::MergeSkip,
        ),
        &TransferControl::default(),
        None,
    )
    .unwrap();

    assert!(matches!(
        outcome,
        TransferOutcome::PartiallyCompleted {
            transferred: 0,
            published: 1,
            skipped: 1,
            source_kept: false,
        }
    ));
    assert!(target.path().join("target/new-empty").is_dir());
}

#[test]
fn review10_merge_skip_with_new_empty_directory_is_partial_and_keeps_move_source() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir_all(source.path().join("source/new-empty")).unwrap();
    fs::write(source.path().join("source/conflict.txt"), b"source").unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(target.path().join("target/conflict.txt"), b"target").unwrap();

    let outcome = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::MergeSkip,
        ),
        &TransferControl::default(),
        None,
    )
    .unwrap();

    assert!(matches!(
        outcome,
        TransferOutcome::PartiallyCompleted {
            transferred: 0,
            published: 1,
            skipped: 1,
            source_kept: true,
        }
    ));
    assert!(target.path().join("target/new-empty").is_dir());
    assert!(source.path().join("source/new-empty").is_dir());
    assert_eq!(
        fs::read(source.path().join("source/conflict.txt")).unwrap(),
        b"source"
    );
}

#[test]
fn directory_move_uses_quarantine_even_when_rename_acknowledgement_fails() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.txt"), b"a").unwrap();
    let rename_destinations = Arc::new(Mutex::new(Vec::new()));
    let observed_destinations = rename_destinations.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_before_rename(move |destination| {
                observed_destinations
                    .lock()
                    .unwrap()
                    .push(destination.to_path_buf());
            })
            .with_rename_after_apply_failure(PathBuf::from("/source")),
    );

    assert_eq!(
        run_directory_transfer(
            &directory_job(
                source_backend,
                backend(target.path()),
                TransferOperation::Move,
                ConflictDecision::Overwrite,
            ),
            &TransferControl::default(),
            None,
        )
        .unwrap(),
        TransferOutcome::Completed
    );
    let first_destination = rename_destinations
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap();
    assert!(
        first_destination
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(".zaplex-source-"),
        "source cleanup must begin with an atomic quarantine rename"
    );
    assert!(!source.path().join("source").exists());
    assert_eq!(fs::read(target.path().join("target/a.txt")).unwrap(), b"a");
}

#[test]
fn partial_quarantine_delete_keeps_recovery_tree_and_complete_target() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.txt"), b"a").unwrap();
    fs::write(source.path().join("source/b.txt"), b"b").unwrap();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_partial_recursive_delete_failure(),
    );

    assert!(run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .is_err());
    assert!(
        !source.path().join("source").exists(),
        "new data at the original path must be isolated from cleanup"
    );
    let recovery = fs::read_dir(source.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("zaplex-source")
        })
        .expect("remaining quarantine must be retained as a recovery path");
    assert!(recovery.is_dir());
    assert_eq!(fs::read(target.path().join("target/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(target.path().join("target/b.txt")).unwrap(), b"b");
}

#[cfg(unix)]
#[test]
fn directory_replaced_by_symlink_during_traversal_is_rejected_before_target_mutation() {
    use std::os::unix::fs::symlink;

    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::create_dir_all(source.path().join("source/sub")).unwrap();
    fs::write(source.path().join("source/sub/original.txt"), b"original").unwrap();
    fs::write(outside.path().join("foreign.txt"), b"foreign").unwrap();
    let source_root = source.path().to_path_buf();
    let outside_root = outside.path().to_path_buf();
    let mutated = Arc::new(AtomicBool::new(false));
    let mutation = mutated.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_stable_identity(
            move |path| {
                if path == Path::new("/source/sub") && !mutation.swap(true, Ordering::SeqCst) {
                    fs::rename(
                        source_root.join("source/sub"),
                        source_root.join("source/original-sub"),
                    )
                    .unwrap();
                    symlink(&outside_root, source_root.join("source/sub")).unwrap();
                }
            },
        ),
    );

    assert!(run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .is_err());
    assert!(!target.path().join("target").exists());
}

#[test]
fn atomic_directory_move_reports_committed_when_post_rename_verification_fails() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    fs::write(root.path().join("source/payload.bin"), b"payload").unwrap();
    let backend = Arc::new(
        InstrumentedBackend::new(root.path())
            .failing_identity_after_rename(PathBuf::from("/target")),
    ) as Arc<dyn SftpBackend>;
    let transfer = TransferJob {
        source_backend: backend.clone(),
        target_backend: backend,
        source_path: PathBuf::from("/source"),
        target_path: PathBuf::from("/target"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_directory_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("post-rename verification failure must require recovery");

    assert!(error.destination_committed());
    assert!(!root.path().join("source").exists());
    assert_eq!(
        fs::read(root.path().join("target/payload.bin")).unwrap(),
        b"payload"
    );
}

#[test]
fn same_size_foreign_publish_never_allows_source_delete() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"oldold").unwrap();
    let target_root = target.path().to_path_buf();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_after_replace(move |path| {
            if path == Path::new("/target.bin") {
                fs::write(target_root.join("target.bin"), b"evil!!").unwrap();
            }
        }),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a foreign post-publish replacement must block source deletion");
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"evil!!"
    );
    assert!(error.recovery_paths().iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-backup"))
    }));
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-backup"))
            && fs::read(path).is_ok_and(|contents| contents == b"oldold")
    }));
}

#[test]
fn cancel_after_publish_commit_is_rejected_and_move_completes() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"oldold").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_after_replace(move |path| {
            if path == Path::new("/target.bin") {
                hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
            }
        }),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_transfer(&transfer, &control, None).unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!source.path().join("source.bin").exists());
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn backup_cleanup_failure_is_reported_with_recovery_path() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_delete_failure_matching("zaplex-backup"),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("cleanup failure must not be logged and discarded");
    assert!(error.to_string().contains("cleanup"));
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"new");
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-backup")
    }));
}

#[test]
fn target_mutation_in_final_exchange_window_is_restored_unchanged() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_root = target.path().to_path_buf();
    let mutated = Arc::new(AtomicBool::new(false));
    let hook_mutated = mutated.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_replace(move |path| {
            if path == Path::new("/target.bin") && !hook_mutated.swap(true, Ordering::SeqCst) {
                fs::write(target_root.join("target.bin"), b"late").unwrap();
            }
        }),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a last-window destination mutation must abort publish");

    assert!(mutated.load(Ordering::SeqCst));
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"late");
    assert_eq!(fs::read(source.path().join("source.bin")).unwrap(), b"new");
}

#[test]
fn rollback_race_never_discards_the_late_target() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_root = target.path().to_path_buf();
    let target_root_after_publish = target_root.clone();
    let exchanges = Arc::new(AtomicU64::new(0));
    let hook_exchanges = exchanges.clone();
    let published = Arc::new(AtomicBool::new(false));
    let hook_published = published.clone();
    let progress_published = published.clone();
    let verification_events = Arc::new(AtomicU64::new(0));
    let progress_verification_events = verification_events.clone();
    let corrupted = Arc::new(AtomicBool::new(false));
    let progress_corrupted = corrupted.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_before_replace(move |path| {
                if path == Path::new("/target.bin")
                    && hook_exchanges.fetch_add(1, Ordering::SeqCst) == 1
                {
                    fs::write(target_root.join("target.bin"), b"late").unwrap();
                }
            })
            .with_after_replace(move |path| {
                if path == Path::new("/target.bin") {
                    hook_published.store(true, Ordering::SeqCst);
                }
            }),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let mut mutate_after_publish_read = |progress: TransferProgress| {
        if progress.phase == TransferPhase::Verifying
            && progress_published.load(Ordering::SeqCst)
            && progress_verification_events.fetch_add(1, Ordering::SeqCst) == 5
            && !progress_corrupted.swap(true, Ordering::SeqCst)
        {
            fs::write(target_root_after_publish.join("target.bin"), b"evil").unwrap();
        }
    };
    run_transfer(
        &transfer,
        &TransferControl::default(),
        Some(&mut mutate_after_publish_read),
    )
    .expect_err("published verification failure must attempt safe rollback");

    let exchange_count = exchanges.load(Ordering::SeqCst);
    let verification_count = verification_events.load(Ordering::SeqCst);
    let was_corrupted = corrupted.load(Ordering::SeqCst);
    assert!(
        was_corrupted,
        "corruption hook was not reached; exchange_count={exchange_count}, verification_count={verification_count}"
    );
    assert!(
        exchange_count >= 2,
        "rollback exchange was not reached; exchange_count={exchange_count}, verification_count={verification_count}"
    );
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"late");
}

#[test]
fn same_size_corruption_in_file_stage_never_publishes() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_corrupt_writer_on_create(1),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("same-size file corruption must fail source-to-stage verification");
    assert!(!target.path().join("target.bin").exists());
}

#[test]
fn same_size_corruption_in_directory_stage_never_publishes() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_corrupt_writer_on_create(1),
    );

    run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("same-size directory corruption must fail the content manifest");
    assert!(!target.path().join("target").exists());
}

#[test]
fn source_quarantine_replacement_at_cleanup_boundary_is_never_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_guarded_delete(
            move |path| {
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !hook_replaced.swap(true, Ordering::SeqCst)
                {
                    let relative = path.strip_prefix("/").unwrap();
                    fs::write(source_root.join(relative), b"foreign").unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("late source replacement must block cleanup");
    assert!(replaced.load(Ordering::SeqCst));
    assert!(fs::read_dir(source.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
            && fs::read(path).is_ok_and(|contents| contents == b"foreign")
    }));
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn late_quarantine_member_is_never_recursively_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let source_root = source.path().to_path_buf();
    let inserted = Arc::new(AtomicBool::new(false));
    let hook_inserted = inserted.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_guarded_delete(
            move |path| {
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !hook_inserted.swap(true, Ordering::SeqCst)
                {
                    let relative = path.strip_prefix("/").unwrap();
                    fs::write(source_root.join(relative).join("late.bin"), b"foreign").unwrap();
                }
            },
        ),
    );

    run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("unexpected quarantine membership must stop cleanup");

    assert!(inserted.load(Ordering::SeqCst));
    assert!(fs::read_dir(source.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.is_dir() && path.join("late.bin").exists()
    }));
}

#[test]
fn late_stage_replacement_is_retained_for_manual_recovery() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![0x42; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let target_root = target.path().to_path_buf();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_corrupt_writer_on_create(1)
            .with_before_guarded_delete(move |path| {
                if path.to_string_lossy().contains("zaplex-transfer")
                    && !hook_replaced.swap(true, Ordering::SeqCst)
                {
                    let relative = path.strip_prefix("/").unwrap();
                    fs::write(target_root.join(relative), b"foreign").unwrap();
                }
            }),
    );
    let control = TransferControl::default();
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &control, None)
        .expect_err("late stage replacement must block cleanup");

    assert!(replaced.load(Ordering::SeqCst));
    assert!(
        error
            .recovery_paths()
            .iter()
            .any(|path| path.to_string_lossy().contains("zaplex-transfer")),
        "the retained stage must be reported for recovery: {error:?}"
    );
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-transfer"))
            && fs::read(path).is_ok_and(|contents| contents == b"foreign")
    }));
}

#[test]
fn late_backup_replacement_is_retained_after_commit() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_root = target.path().to_path_buf();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_guarded_delete(
            move |path| {
                if path.to_string_lossy().contains("zaplex-backup")
                    && !hook_replaced.swap(true, Ordering::SeqCst)
                {
                    let relative = path.strip_prefix("/").unwrap();
                    fs::write(target_root.join(relative), b"foreign").unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("late backup replacement must be retained");

    assert!(replaced.load(Ordering::SeqCst));
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"new");
    assert!(error
        .recovery_paths()
        .iter()
        .any(|path| path.to_string_lossy().contains("zaplex-backup")));
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-backup"))
            && fs::read(path).is_ok_and(|contents| contents == b"foreign")
    }));
}

#[test]
fn failed_file_stage_reservation_never_deletes_foreign_collision() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).colliding_writer_create(1));
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the foreign stage collision must abort the transfer");

    let recovery_id = error
        .recovery_id()
        .expect("ambiguous stage ownership must remain retryable");
    retry_recovery(recovery_id)
        .expect_err("retry must preserve an entry whose ownership is still ambiguous");
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-transfer"))
            && fs::read(path).is_ok_and(|contents| contents == b"foreign")
    }));
}

#[test]
fn failed_directory_stage_reservation_never_deletes_foreign_collision() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).colliding_directory_stage());

    run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the foreign directory stage collision must abort the transfer");

    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-tree"))
            && fs::read(path.join("foreign.bin")).is_ok_and(|contents| contents == b"foreign")
    }));
}

#[test]
fn failed_backup_reservation_never_deletes_foreign_collision() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).colliding_writer_create(2));
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the foreign backup collision must abort the transfer");

    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"old");
    assert!(fs::read_dir(target.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-backup"))
            && fs::read(path).is_ok_and(|contents| contents == b"foreign")
    }));
}

#[test]
fn foreign_child_inside_owned_stage_is_never_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir_all(source.path().join("source/child")).unwrap();
    fs::write(source.path().join("source/child/source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).colliding_directory_child());

    let error = run_directory_transfer(
        &directory_job(
            backend(source.path()),
            target_backend,
            TransferOperation::Copy,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the foreign child collision must abort");

    let stages = transfer_artifacts(target.path(), "zaplex-tree");
    assert_eq!(stages.len(), 1, "the retained stage must remain visible");
    assert_eq!(
        fs::read(stages[0].join("child/foreign.bin")).unwrap(),
        b"foreign"
    );
    assert!(
        error.recovery_id().is_some(),
        "the retained mixed-ownership stage must be retryable"
    );
}

#[cfg(unix)]
#[test]
fn replaced_file_stage_is_never_deleted() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).replacing_stage_on_writer_failure());
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the replaced writer stage must abort");

    let stages = transfer_artifacts(target.path(), "zaplex-transfer");
    assert_eq!(stages.len(), 1, "the replacement must remain visible");
    assert_eq!(fs::read(&stages[0]).unwrap(), b"foreign");
    assert!(
        error.recovery_id().is_some(),
        "the replaced stage must have a retryable recovery handle"
    );
}

#[test]
fn cancel_before_ambiguous_cleanup_keeps_a_retryable_recovery_handle() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InstrumentedBackend::new(target.path()).cancelling_writer_collision(control.clone()),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &control, None)
        .expect_err("cancellation must retain the ambiguous stage");

    assert!(
        error.recovery_id().is_some(),
        "accepted cancellation must not orphan ambiguous cleanup"
    );
    assert_eq!(
        fs::read(transfer_artifacts(target.path(), "zaplex-transfer").remove(0)).unwrap(),
        b"foreign"
    );
}

#[cfg(unix)]
#[test]
fn ambiguous_symlink_stage_remains_retryable_without_snapshot() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(target.path()).symlink_writer_collision());
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the ambiguous symlink must abort");

    assert!(
        error.recovery_id().is_some(),
        "unsupported ambiguous entries must retain unresolved recovery"
    );
    assert!(fs::symlink_metadata(
        transfer_artifacts(target.path(), "zaplex-transfer")
            .first()
            .unwrap()
    )
    .unwrap()
    .file_type()
    .is_symlink());
}

#[test]
fn ambiguous_probe_failure_remains_retryable_without_snapshot() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_writer_create_after_apply_failure(1)
            .with_staged_identity_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the ambiguous identity probe must abort");

    assert!(
        error.recovery_id().is_some(),
        "a failed ambiguous probe must retain unresolved recovery"
    );
    assert_eq!(
        transfer_artifacts(target.path(), "zaplex-transfer").len(),
        1
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn replaced_local_probe_path_is_never_deleted() {
    let target = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(target.path().to_path_buf())
        .with_preflight_cleanup_replacement('c');

    backend
        .preflight_safe_mutation(Path::new("/target.bin"), true)
        .expect_err("a replaced owned probe path must fail safe");

    let (path, device, inode) = backend
        .preflight_cleanup_replacement()
        .expect("the cleanup replacement hook must run");
    let metadata = fs::symlink_metadata(&path).expect("the foreign replacement must remain");
    use std::os::unix::fs::MetadataExt;
    assert_eq!((metadata.dev(), metadata.ino()), (device, inode));
    assert_eq!(fs::read(path).unwrap(), b"foreign-c");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn preflight_create_ack_uncertainty_is_owned_or_retryable() {
    for suffix in ['a', 'b', 'd'] {
        let target = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_create_after_apply_failure(suffix);

        let error = backend
            .preflight_safe_mutation(Path::new("/target.bin"), true)
            .expect_err("an uncertain probe create must fail safe");
        let path = backend
            .preflight_uncertain_create()
            .expect("the applied create must remain observable");

        assert!(
            error
                .recovery_paths()
                .iter()
                .any(|candidate| candidate.file_name() == path.file_name()),
            "uncertain probe create {suffix} must be visible for recovery"
        );
        assert!(
            fs::symlink_metadata(path).is_ok(),
            "uncertain probe create {suffix} must not be silently discarded"
        );
    }
}

#[test]
fn cancel_is_rejected_after_file_finalizing_commit_point() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let cancel = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_guarded_delete(
            move |_| {
                hook_cancel_accepted.store(cancel.cancel(), Ordering::SeqCst);
            },
        ),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_transfer(&transfer, &control, None).unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!source.path().join("source.bin").exists());
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn cancel_is_rejected_after_directory_finalizing_commit_point() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let cancel = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_guarded_delete(
            move |_| {
                hook_cancel_accepted.store(cancel.cancel(), Ordering::SeqCst);
            },
        ),
    );

    assert_eq!(
        run_directory_transfer(
            &directory_job(
                source_backend,
                backend(target.path()),
                TransferOperation::Move,
                ConflictDecision::Overwrite,
            ),
            &control,
            None,
        )
        .unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!source.path().join("source").exists());
    assert_eq!(
        fs::read(target.path().join("target/a.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn guarded_cleanup_retains_tombstone_after_rename_ack_uncertainty() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source.bin"), b"source").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_rename(|old_path, _| fs::write(old_path, b"foreign").unwrap())
        .with_rename_after_apply_failure(PathBuf::from("/source.bin"));
    let identity = backend.stable_identity(Path::new("/source.bin")).unwrap();
    let publication = capture_publication_snapshot(&backend, Path::new("/source.bin")).unwrap();
    let digest = publication
        .entries
        .get(Path::new("/source.bin"))
        .unwrap()
        .revision
        .clone();

    let error = backend
        .delete_file_if_matches(Path::new("/source.bin"), &identity, &digest)
        .expect_err("an uncertain isolation rename must retain every possible cleanup path");

    assert!(matches!(error, SftpOpsError::RecoveryRequired { .. }));
    assert_eq!(
        fs::read(root.path().join("source.bin")).unwrap(),
        b"foreign"
    );
    assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("zaplex-delete"))
            && fs::read(path).is_ok_and(|contents| contents == b"source")
    }));
    assert!(error
        .recovery_paths()
        .iter()
        .any(|path| path == Path::new("/source.bin")));
    assert!(error
        .recovery_paths()
        .iter()
        .any(|path| path.to_string_lossy().contains("zaplex-delete")));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn local_backend_reports_safe_rename_capabilities_on_supported_platforms() {
    assert!(local_safe_rename_primitives_available());
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn local_backend_refuses_safe_rename_capabilities_without_platform_primitives() {
    assert!(!local_safe_rename_primitives_available());
}

#[test]
fn uncontrolled_cleanup_never_deletes_replaced_empty_directory() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("retained")).unwrap();
    fs::create_dir(root.path().join("foreign")).unwrap();
    let snapshot_backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let snapshot = capture_snapshot(&snapshot_backend, Path::new("/retained")).unwrap();
    let publication =
        capture_publication_snapshot(&snapshot_backend, Path::new("/retained")).unwrap();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let local_root = root.path().to_path_buf();
    let backend = InstrumentedBackend::new(root.path()).after_empty_list(3, move |path| {
        if path == Path::new("/retained") && !hook_replaced.swap(true, Ordering::SeqCst) {
            let local = local_root.join("retained");
            fs::remove_dir(&local).unwrap();
            fs::rename(local_root.join("foreign"), &local).unwrap();
        }
    });

    let error = remove_snapshot_root(&backend, &snapshot, &publication)
        .expect_err("a directory replacement must block cleanup");

    assert!(error.recovery_paths().is_empty());
    assert!(root.path().join("retained").is_dir());
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-delete")
    }));
}

#[test]
fn controlled_cleanup_never_deletes_replaced_empty_directory() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("retained")).unwrap();
    fs::create_dir(root.path().join("foreign")).unwrap();
    let snapshot_backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let snapshot = capture_snapshot(&snapshot_backend, Path::new("/retained")).unwrap();
    let publication =
        capture_publication_snapshot(&snapshot_backend, Path::new("/retained")).unwrap();
    let replaced = Arc::new(AtomicBool::new(false));
    let hook_replaced = replaced.clone();
    let local_root = root.path().to_path_buf();
    let backend = InstrumentedBackend::new(root.path()).after_empty_list(3, move |path| {
        if path == Path::new("/retained") && !hook_replaced.swap(true, Ordering::SeqCst) {
            let local = local_root.join("retained");
            fs::remove_dir(&local).unwrap();
            fs::rename(local_root.join("foreign"), &local).unwrap();
        }
    });
    let control = TransferControl::default();
    let mut progress = None;

    let error = remove_snapshot_root_controlled(
        &backend,
        &snapshot,
        &publication,
        &control,
        &mut progress,
        TransferPhase::Finalizing,
    )
    .expect_err("a directory replacement must block controlled cleanup");

    assert!(error.recovery_paths().is_empty());
    assert!(root.path().join("retained").is_dir());
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-delete")
    }));
}

#[test]
fn required_cleanup_never_revokes_an_accepted_cancel() {
    let control = TransferControl::default();
    assert!(control.cancel());
    let mut progress = None;

    begin_required_cleanup(&control, &mut progress, 0)
        .expect_err("an accepted cancellation must block finalizing");

    assert!(control.is_cancelled());
}

#[test]
fn directory_child_tombstone_registers_retry_for_only_the_isolated_child() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/child.bin"), b"child").unwrap();
    let source_backend: Arc<dyn SftpBackend> =
        Arc::new(InstrumentedBackend::new(source.path()).with_isolated_cleanup_failure());
    let control = TransferControl::default();

    let error = run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &control,
        None,
    )
    .expect_err("the isolated child must remain a retryable recovery action");
    let recovery_id = match error {
        SftpOpsError::RecoveryRequired {
            recovery_id: Some(recovery_id),
            ref paths,
            committed: true,
            ..
        } => {
            assert_eq!(paths.len(), 1);
            assert!(paths[0].to_string_lossy().contains("zaplex-live-recovery"));
            recovery_id
        }
        other => panic!("expected retryable committed recovery, got {other:?}"),
    };

    retry_recovery(recovery_id).expect("the isolated child retry must complete safely");
    assert!(!source.path().join("source").exists());
    assert_eq!(
        fs::read(target.path().join("target/child.bin")).unwrap(),
        b"child"
    );
}

#[test]
fn filesystem_capability_failure_stops_before_first_writer() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"target").unwrap();
    let target_backend =
        Arc::new(InstrumentedBackend::new(target.path()).with_late_unsupported_exchange());
    let writer_creates = target_backend.writer_creates.clone();
    let job = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&job, &TransferControl::default(), None)
        .expect_err("an unsupported destination filesystem must fail preflight");

    assert_eq!(
        writer_creates.load(Ordering::SeqCst),
        0,
        "filesystem capability checks must run before creating the stage writer"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"target"
    );
}

#[test]
fn failed_stage_cleanup_reports_visible_finalizing_progress() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(
        source.path().join("source.bin"),
        vec![7_u8; STREAM_CHUNK_SIZE * 2],
    )
    .unwrap();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_writer_failure_on_create(1),
    );
    let job = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut phases = Vec::new();
    let mut on_progress = |progress: TransferProgress| phases.push(progress.phase);

    run_transfer(&job, &TransferControl::default(), Some(&mut on_progress))
        .expect_err("the injected writer failure must enter controlled cleanup");

    assert!(
        phases.contains(&TransferPhase::Finalizing),
        "failure cleanup must be visible as finalizing progress"
    );
}

#[test]
fn file_publish_commit_rejects_cancel_before_first_mutation() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let expected_target = target.path().join("target.bin");
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination == expected_target {
                    hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_transfer(&transfer, &control, None).unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn directory_publish_commit_rejects_cancel_before_first_mutation() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let expected_target = target.path().join("target");
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination == expected_target {
                    hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
                }
            },
        ),
    );

    assert_eq!(
        run_directory_transfer(
            &directory_job(
                backend(source.path()),
                target_backend,
                TransferOperation::Copy,
                ConflictDecision::Overwrite,
            ),
            &control,
            None,
        )
        .unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert_eq!(
        fs::read(target.path().join("target/a.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn file_source_quarantine_rejects_pause_and_cancel_before_rename() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let pause_accepted = Arc::new(AtomicBool::new(true));
    let hook_pause_accepted = pause_accepted.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                {
                    hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
                    hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    assert_eq!(
        run_transfer(&transfer, &control, None).unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!pause_accepted.load(Ordering::SeqCst));
    assert!(!source.path().join("source.bin").exists());
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn directory_source_quarantine_rejects_pause_and_cancel_before_rename() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let pause_accepted = Arc::new(AtomicBool::new(true));
    let hook_pause_accepted = pause_accepted.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                {
                    hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
                    hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
                }
            },
        ),
    );

    assert_eq!(
        run_directory_transfer(
            &directory_job(
                source_backend,
                backend(target.path()),
                TransferOperation::Move,
                ConflictDecision::Overwrite,
            ),
            &control,
            None,
        )
        .unwrap(),
        TransferOutcome::Completed
    );
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!pause_accepted.load(Ordering::SeqCst));
    assert!(!source.path().join("source").exists());
    assert_eq!(
        fs::read(target.path().join("target/a.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn rollback_exchange_rejects_pause_and_cancel_before_mutation() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let exchanges = Arc::new(AtomicU64::new(0));
    let hook_exchanges = exchanges.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let pause_accepted = Arc::new(AtomicBool::new(true));
    let hook_pause_accepted = pause_accepted.clone();
    let published = Arc::new(AtomicBool::new(false));
    let hook_published = published.clone();
    let progress_published = published.clone();
    let verification_events = Arc::new(AtomicU64::new(0));
    let progress_verification_events = verification_events.clone();
    let corrupted = Arc::new(AtomicBool::new(false));
    let progress_corrupted = corrupted.clone();
    let target_root = target.path().to_path_buf();
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_before_replace(move |path| {
                if path == Path::new("/target.bin")
                    && hook_exchanges.fetch_add(1, Ordering::SeqCst) == 1
                {
                    hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
                    hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
                }
            })
            .with_after_replace(move |path| {
                if path == Path::new("/target.bin") {
                    hook_published.store(true, Ordering::SeqCst);
                }
            }),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut corrupt_after_publish_read = |progress: TransferProgress| {
        if progress.phase == TransferPhase::Verifying
            && progress_published.load(Ordering::SeqCst)
            && progress_verification_events.fetch_add(1, Ordering::SeqCst) == 5
            && !progress_corrupted.swap(true, Ordering::SeqCst)
        {
            fs::write(target_root.join("target.bin"), b"evil").unwrap();
        }
    };

    run_transfer(&transfer, &control, Some(&mut corrupt_after_publish_read))
        .expect_err("post-publish corruption must roll back");

    assert!(corrupted.load(Ordering::SeqCst));
    assert!(exchanges.load(Ordering::SeqCst) >= 2);
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!pause_accepted.load(Ordering::SeqCst));
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"evil",
        "rollback must preserve a concurrent foreign target mutation"
    );
}

#[test]
fn directory_rollback_exchange_rejects_pause_and_cancel_before_mutation() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(target.path().join("target")).unwrap();
    fs::write(target.path().join("target/a.bin"), b"new").unwrap();
    fs::create_dir(target.path().join("displaced")).unwrap();
    fs::write(target.path().join("displaced/a.bin"), b"old").unwrap();
    let control = Arc::new(TransferControl::default());
    assert!(control.pause());
    let hook_control = control.clone();
    let cancel_accepted = Arc::new(AtomicBool::new(true));
    let hook_cancel_accepted = cancel_accepted.clone();
    let pause_accepted = Arc::new(AtomicBool::new(true));
    let hook_pause_accepted = pause_accepted.clone();
    let (mutation_tx, mutation_rx) = mpsc::channel();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_replace(move |path| {
            if path == Path::new("/target") {
                mutation_tx.send(()).unwrap();
                hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
                hook_cancel_accepted.store(hook_control.cancel(), Ordering::SeqCst);
            }
        }),
    );
    let published_snapshot = capture_snapshot(&*target_backend, Path::new("/target")).unwrap();
    let published_publication =
        capture_publication_snapshot(&*target_backend, Path::new("/target")).unwrap();
    let displaced = BackupSnapshot {
        path: PathBuf::from("/displaced"),
        snapshot: capture_snapshot(&*target_backend, Path::new("/displaced")).unwrap(),
        publication: capture_publication_snapshot(&*target_backend, Path::new("/displaced"))
            .unwrap(),
        ownership: None,
    };
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source"),
        target_path: PathBuf::from("/target"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let mut progress = None;
            rollback_published_entry(
                &transfer,
                SftpOpsError::Operation("injected post-publish verification failure".to_string()),
                &published_snapshot,
                &published_publication,
                Some(&displaced),
                None,
                &control,
                &mut progress,
            )
        });
        assert!(
            mutation_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "an accepted pause must block the directory rollback exchange"
        );
        assert!(control.resume());
        mutation_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("directory rollback must continue after resume");
        let error = worker.join().unwrap();
        assert!(error
            .to_string()
            .contains("injected post-publish verification failure"));
    });

    assert!(!pause_accepted.load(Ordering::SeqCst));
    assert!(!cancel_accepted.load(Ordering::SeqCst));
    assert!(!control.pause());
    assert!(!control.cancel());
    assert_eq!(
        fs::read(target.path().join("target/a.bin")).unwrap(),
        b"old",
        "rollback must restore the original directory target"
    );
}

#[test]
fn ignored_noreplace_semantics_fail_before_writer_and_preserve_target() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_ignored_noreplace_probe_semantics(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("ignored NOREPLACE semantics must fail capability preflight");

    assert_eq!(target_backend.writer_create_count(), 0);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"foreign"
    );
}

#[test]
fn preflight_cleanup_failure_stops_before_writer_and_is_not_cached() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_preflight_cleanup_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    for _ in 0..2 {
        run_transfer(&transfer, &TransferControl::default(), None)
            .expect_err("probe cleanup failure must remain fail-safe");
    }

    assert_eq!(target_backend.writer_create_count(), 0);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"foreign"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn review9_preflight_keeps_the_reservation_anchor_open_through_cleanup() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_cleanup_anchor_observation('c'),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None).unwrap();

    assert!(
        target_backend.preflight_cleanup_anchor_observed(),
        "the create_new file descriptor must remain open until cleanup isolation completes"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn review9_preflight_anchor_prevents_inode_reuse_from_authorizing_cleanup() {
    use std::os::unix::fs::MetadataExt;

    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_inode_reuse_attempt('c'),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the foreign replacement must make preflight fail safe");

    let (replacement, reserved_inode, replacement_inode, reused, mtime, mtime_nsec) =
        target_backend
            .preflight_inode_reuse_observation()
            .expect("the inode-reuse adversary must run");
    assert!(
        !reused,
        "an open reservation anchor must prevent inode reuse"
    );
    assert_ne!(replacement_inode, reserved_inode);
    assert_eq!(fs::read(&replacement).unwrap(), b"xxxxx");
    let replacement_metadata = fs::symlink_metadata(replacement).unwrap();
    assert_eq!(
        (
            replacement_metadata.mtime(),
            replacement_metadata.mtime_nsec()
        ),
        (mtime, mtime_nsec)
    );
    assert_eq!(target_backend.writer_create_count(), 0);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review9_owned_preflight_tombstone_identity_moves_into_recovery_and_is_released() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_preflight_cleanup_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the first probe cleanup must fail");
    let recovery_id = error
        .recovery_id()
        .expect("the owned tombstone must be retryable");
    assert_eq!(
        target_backend.cleanup_recovery_identity_count(),
        0,
        "the identity must move into the recovery unit instead of remaining in the backend map"
    );
    let retained = error
        .recovery_paths()
        .iter()
        .find(|path| path.to_string_lossy().contains("probe-cleanup"))
        .cloned()
        .expect("the owned tombstone path must be reported");
    assert!(target
        .path()
        .join(retained.strip_prefix("/").unwrap())
        .exists());

    retry_recovery(recovery_id).expect("verified owned cleanup must succeed on retry");
    assert_eq!(target_backend.cleanup_recovery_identity_count(), 0);

    assert!(!target
        .path()
        .join(retained.strip_prefix("/").unwrap())
        .exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn owned_preflight_tombstone_retry_preserves_foreign_replacement() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign-target").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_preflight_cleanup_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the first probe cleanup must fail");
    let recovery_id = error.recovery_id().unwrap();
    let retained = error
        .recovery_paths()
        .iter()
        .find(|path| path.to_string_lossy().contains("probe-cleanup"))
        .cloned()
        .unwrap();
    let retained_local = target.path().join(retained.strip_prefix("/").unwrap());
    fs::remove_file(&retained_local).unwrap();
    fs::write(&retained_local, b"foreign").unwrap();

    retry_recovery(recovery_id).expect_err("a foreign replacement must remain unresolved");

    assert_eq!(fs::read(retained_local).unwrap(), b"foreign");
}

#[test]
fn accepted_pause_blocks_file_publish_until_resume() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let pause_accepted = Arc::new(AtomicBool::new(false));
    let hook_pause_accepted = pause_accepted.clone();
    let paused = Arc::new(Barrier::new(2));
    let hook_paused = paused.clone();
    let release_hook = Arc::new(Barrier::new(2));
    let hook_release = release_hook.clone();
    control.set_before_finalizing_hook(1, move || {
        hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
        hook_paused.wait();
        hook_release.wait();
    });
    let (mutation_tx, mutation_rx) = mpsc::channel();
    let expected_target = target.path().join("target.bin");
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination == expected_target {
                    mutation_tx.send(()).unwrap();
                }
            },
        ),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    thread::scope(|scope| {
        let worker = scope.spawn(|| run_transfer(&transfer, &control, None));
        paused.wait();
        assert!(pause_accepted.load(Ordering::SeqCst));
        release_hook.wait();
        assert!(
            mutation_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "an accepted pause must block the first publish mutation"
        );
        assert!(control.resume());
        mutation_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("publish must continue after resume");
        assert_eq!(worker.join().unwrap().unwrap(), TransferOutcome::Completed);
    });
    assert!(!control.pause(), "pause must be rejected after commit");
}

#[test]
fn accepted_pause_blocks_directory_publish_until_resume() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/a.bin"), b"source").unwrap();
    let control = Arc::new(TransferControl::default());
    let hook_control = control.clone();
    let pause_accepted = Arc::new(AtomicBool::new(false));
    let hook_pause_accepted = pause_accepted.clone();
    let paused = Arc::new(Barrier::new(2));
    let hook_paused = paused.clone();
    let release_hook = Arc::new(Barrier::new(2));
    let hook_release = release_hook.clone();
    control.set_before_finalizing_hook(1, move || {
        hook_pause_accepted.store(hook_control.pause(), Ordering::SeqCst);
        hook_paused.wait();
        hook_release.wait();
    });
    let (mutation_tx, mutation_rx) = mpsc::channel();
    let expected_target = target.path().join("target");
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_before_rename(
            move |destination| {
                if destination == expected_target {
                    mutation_tx.send(()).unwrap();
                }
            },
        ),
    );
    let transfer = directory_job(
        backend(source.path()),
        target_backend.clone(),
        TransferOperation::Copy,
        ConflictDecision::Overwrite,
    );

    thread::scope(|scope| {
        let worker = scope.spawn(|| run_directory_transfer(&transfer, &control, None));
        paused.wait();
        assert!(pause_accepted.load(Ordering::SeqCst));
        release_hook.wait();
        assert!(
            mutation_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "an accepted pause must block the first directory publish mutation"
        );
        assert!(control.resume());
        mutation_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("directory publish must continue after resume");
        assert_eq!(worker.join().unwrap().unwrap(), TransferOutcome::Completed);
    });
    assert!(!control.pause(), "pause must be rejected after commit");
}

#[cfg(unix)]
#[test]
fn preflight_collisions_preserve_foreign_probe_entries() {
    use std::os::unix::fs::MetadataExt;

    for suffix in ['b', 'c', 'd'] {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        fs::write(source.path().join("source.bin"), b"source").unwrap();
        fs::write(target.path().join("target.bin"), b"foreign-target").unwrap();
        let target_backend = Arc::new(
            InMemorySftpBackend::new(target.path().to_path_buf()).with_preflight_collision(suffix),
        );
        let transfer = TransferJob {
            source_backend: backend(source.path()),
            target_backend: target_backend.clone(),
            source_path: PathBuf::from("/source.bin"),
            target_path: PathBuf::from("/target.bin"),
            operation: TransferOperation::Copy,
            conflict: ConflictDecision::Overwrite,
        };

        run_transfer(&transfer, &TransferControl::default(), None)
            .expect_err("a foreign probe-path collision must fail preflight");

        let (collision, device, inode) = target_backend
            .preflight_collision()
            .expect("the collision must have been injected");
        let metadata = fs::symlink_metadata(&collision)
            .expect("cleanup must not remove a foreign probe entry");
        assert_eq!((metadata.dev(), metadata.ino()), (device, inode));
        assert_eq!(
            fs::read(&collision).unwrap(),
            format!("foreign-{suffix}").as_bytes()
        );
        assert_eq!(target_backend.writer_create_count(), 0);
        assert_eq!(
            fs::read(target.path().join("target.bin")).unwrap(),
            b"foreign-target"
        );
    }
}

#[cfg(unix)]
#[test]
fn preflight_cleanup_swap_after_identity_check_preserves_foreign_entry() {
    use std::os::unix::fs::MetadataExt;

    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign-target").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_cleanup_replacement_after_check('c'),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("a cleanup swap after identity verification must fail safe");

    let (replacement, device, inode) = target_backend
        .preflight_cleanup_replacement_after_check()
        .expect("the cleanup race must have been injected");
    let metadata = fs::symlink_metadata(&replacement).expect("the foreign replacement must remain");
    assert_eq!((metadata.dev(), metadata.ino()), (device, inode));
    assert_eq!(fs::read(&replacement).unwrap(), b"foreign-c");
    assert!(
        error.recovery_id().is_some(),
        "the isolated cleanup race must remain visible and retryable"
    );
    assert_eq!(target_backend.writer_create_count(), 0);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"foreign-target"
    );
}

#[test]
fn copy_unlink_rename_probe_fails_before_writer_and_is_not_cached() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf()).with_preflight_rename_copy_unlink(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    for _ in 0..2 {
        run_transfer(&transfer, &TransferControl::default(), None)
            .expect_err("copy-and-unlink must not satisfy the atomic rename capability");
    }
    assert_eq!(target_backend.writer_create_count(), 0);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"foreign"
    );
}

#[test]
fn content_swap_exchange_probe_fails_before_writer_and_is_not_cached() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"foreign").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_exchange_content_swap(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    for _ in 0..2 {
        run_transfer(&transfer, &TransferControl::default(), None)
            .expect_err("content swapping must not satisfy atomic exchange capability");
    }
    assert_eq!(target_backend.writer_create_count(), 0);
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"foreign"
    );
}

fn transfer_artifacts(root: &Path, marker: &str) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains(marker))
        .collect()
}

fn assert_artifacts_are_reported(error: &SftpOpsError, artifacts: &[PathBuf]) {
    assert!(
        !artifacts.is_empty(),
        "the injected applied create must exist"
    );
    for artifact in artifacts {
        assert!(
            error.recovery_paths().iter().any(|path| {
                path.file_name()
                    .is_some_and(|name| Some(name) == artifact.file_name())
            }),
            "every possible job artifact must remain visible for recovery: {}",
            artifact.display()
        );
    }
    assert!(
        error.recovery_id().is_some(),
        "ambiguous ownership must remain retryable"
    );
}

#[test]
fn file_stage_create_applied_then_error_is_registered_for_recovery() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_writer_create_after_apply_failure(1),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an ambiguous file stage reservation must abort");

    assert_artifacts_are_reported(
        &error,
        &transfer_artifacts(target.path(), "zaplex-transfer"),
    );
}

#[test]
fn directory_stage_create_applied_then_error_is_registered_for_recovery() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::create_dir(source.path().join("source")).unwrap();
    fs::write(source.path().join("source/file.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_directory_create_after_apply_failure(1),
    );
    let transfer = directory_job(
        backend(source.path()),
        target_backend.clone(),
        TransferOperation::Copy,
        ConflictDecision::Overwrite,
    );

    let error = run_directory_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an ambiguous directory stage reservation must abort");

    if let Some(recovery_id) = error.recovery_id() {
        retry_recovery(recovery_id).expect("the protected reservation retry must complete");
    }
    assert_eq!(
        target_backend.directory_reservation_artifact_count(),
        0,
        "the applied create must be cleaned immediately or by its retryable recovery"
    );
    assert!(
        transfer_artifacts(target.path(), "zaplex-tree").is_empty(),
        "the protected reservation must never leak into the visible target namespace"
    );
}

#[test]
fn backup_create_applied_then_error_is_registered_for_recovery() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    fs::write(target.path().join("target.bin"), b"target").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_writer_create_after_apply_failure(2),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    let error = run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an ambiguous backup reservation must abort");

    assert_artifacts_are_reported(&error, &transfer_artifacts(target.path(), "zaplex-backup"));
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"target"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn preflight_rename_applied_then_error_cleans_the_owned_destination() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let target_backend = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_preflight_rename_after_apply_failure(),
    );
    let transfer = TransferJob {
        source_backend: backend(source.path()),
        target_backend: target_backend.clone(),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("an uncertain preflight rename acknowledgement must fail safe");

    assert!(
        transfer_artifacts(target.path(), "zaplex-rename-probe").is_empty(),
        "the identity-proven moved probe must remain owned and be cleaned"
    );
    assert_eq!(target_backend.writer_create_count(), 0);
}

#[test]
fn monotonic_id_exhaustion_does_not_panic() {
    let counter = AtomicU64::new(u64::MAX);
    let outcome = std::panic::catch_unwind(|| next_monotonic_id(&counter, "test ID"));
    assert!(
        outcome.is_ok(),
        "ID exhaustion must be returned as an error"
    );
    assert!(outcome.unwrap().is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn review17_file_move_never_isolates_a_source_replaced_inside_backend_rename() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let source_path = source.path().join("source.bin");
    let retained = source.path().join("review17-original-source.bin");
    fs::write(&source_path, b"source").unwrap();
    let replaced = Arc::new(AtomicBool::new(false));
    let replace_once = replaced.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename({
            let source_path = source_path.clone();
            let retained = retained.clone();
            move |destination| {
                if destination
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !replace_once.swap(true, Ordering::SeqCst)
                {
                    fs::rename(&source_path, &retained).unwrap();
                    fs::write(&source_path, b"foreign").unwrap();
                }
            }
        }),
    );
    let transfer = TransferJob {
        source_backend,
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the backend-local replacement must abort source isolation");

    assert!(replaced.load(Ordering::SeqCst));
    assert_eq!(fs::read(&source_path).unwrap(), b"foreign");
    assert_eq!(fs::read(&retained).unwrap(), b"source");
    assert!(
        !fs::read_dir(source.path()).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
        }),
        "the foreign replacement must never be moved into quarantine"
    );
}

#[test]
fn review17_directory_move_never_isolates_a_source_replaced_inside_backend_rename() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let source_path = source.path().join("source");
    let retained = source.path().join("review17-original-source");
    fs::create_dir(&source_path).unwrap();
    fs::write(source_path.join("a.bin"), b"source").unwrap();
    let replaced = Arc::new(AtomicBool::new(false));
    let replace_once = replaced.clone();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_before_rename({
            let source_path = source_path.clone();
            let retained = retained.clone();
            move |destination| {
                if destination
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    && !replace_once.swap(true, Ordering::SeqCst)
                {
                    fs::rename(&source_path, &retained).unwrap();
                    fs::create_dir(&source_path).unwrap();
                    fs::write(source_path.join("a.bin"), b"foreign").unwrap();
                }
            }
        }),
    );

    run_directory_transfer(
        &directory_job(
            source_backend,
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the backend-local directory replacement must abort source isolation");

    assert!(replaced.load(Ordering::SeqCst));
    assert_eq!(fs::read(source_path.join("a.bin")).unwrap(), b"foreign");
    assert_eq!(fs::read(retained.join("a.bin")).unwrap(), b"source");
    assert!(
        !fs::read_dir(source.path()).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
        }),
        "the foreign directory replacement must never be moved into quarantine"
    );
}

#[cfg(unix)]
#[test]
fn review18_file_isolation_restores_a_replacement_swapped_after_the_final_guard() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let source_path = source.path().join("source.bin");
    let retained = source.path().join("review18-original-source.bin");
    fs::write(&source_path, b"source").unwrap();
    let source_backend = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_after_guarded_rename_check_before_mutation({
                let source_path = source_path.clone();
                let retained = retained.clone();
                move |old, new| {
                    if new
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    {
                        assert_eq!(old, source_path);
                        fs::rename(&source_path, &retained).unwrap();
                        fs::write(&source_path, b"foreign").unwrap();
                    }
                }
            }),
    );
    let transfer = TransferJob {
        source_backend: source_backend.clone(),
        target_backend: backend(target.path()),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };

    run_transfer(&transfer, &TransferControl::default(), None)
        .expect_err("the post-guard replacement must abort isolation");

    assert_eq!(fs::read(&source_path).unwrap(), b"foreign");
    assert_eq!(fs::read(&retained).unwrap(), b"source");
    assert!(
        fs::read_dir(source.path()).unwrap().all(|entry| {
            let path = entry.unwrap().path();
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                || fs::read(path).is_ok_and(|bytes| bytes != b"foreign")
        }),
        "the foreign replacement must be restored from quarantine"
    );
    assert!(
        !source_backend.startup_recovery_paths_for_test().is_empty(),
        "the ambiguous isolation must remain globally recoverable"
    );
}

#[cfg(unix)]
#[test]
fn review18_directory_isolation_restores_a_replacement_swapped_after_the_final_guard() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    let source_path = source.path().join("source");
    let retained = source.path().join("review18-original-source");
    fs::create_dir(&source_path).unwrap();
    fs::write(source_path.join("a.bin"), b"source").unwrap();
    let source_backend = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_after_guarded_rename_check_before_mutation({
                let source_path = source_path.clone();
                let retained = retained.clone();
                move |old, new| {
                    if new
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                    {
                        assert_eq!(old, source_path);
                        fs::rename(&source_path, &retained).unwrap();
                        fs::create_dir(&source_path).unwrap();
                        fs::write(source_path.join("a.bin"), b"foreign").unwrap();
                    }
                }
            }),
    );

    run_directory_transfer(
        &directory_job(
            source_backend.clone(),
            backend(target.path()),
            TransferOperation::Move,
            ConflictDecision::Overwrite,
        ),
        &TransferControl::default(),
        None,
    )
    .expect_err("the post-guard directory replacement must abort isolation");

    assert_eq!(fs::read(source_path.join("a.bin")).unwrap(), b"foreign");
    assert_eq!(fs::read(retained.join("a.bin")).unwrap(), b"source");
    assert!(
        !source_backend.startup_recovery_paths_for_test().is_empty(),
        "the ambiguous directory isolation must remain globally recoverable"
    );
}
