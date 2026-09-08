//! Streaming file-transfer job shared by local and remote file-manager panes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::sftp_backend::{BackendOwnershipAnchor, SftpBackend};
use super::sftp_ops::SftpOpsError;
use super::types::{FileEntryType, TransferPhase};

pub const STREAM_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOperation {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictDecision {
    Skip,
    Overwrite,
    MergeSkip,
    Rename,
    NewerOnly,
}

#[derive(Clone)]
pub struct TransferJob {
    pub source_backend: Arc<dyn SftpBackend>,
    pub target_backend: Arc<dyn SftpBackend>,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub operation: TransferOperation,
    pub conflict: ConflictDecision,
}

fn conflict_name(path: &Path, is_directory: bool, sequence: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "entry".to_string());
    let suffix = if sequence == 1 {
        "copy".to_string()
    } else {
        format!("copy {sequence}")
    };
    let renamed = if is_directory {
        format!("{name} ({suffix})")
    } else {
        let name_path = Path::new(&name);
        match (name_path.file_stem(), name_path.extension()) {
            (Some(stem), Some(extension)) => format!(
                "{} ({suffix}).{}",
                stem.to_string_lossy(),
                extension.to_string_lossy()
            ),
            (Some(stem), None) => format!("{} ({suffix})", stem.to_string_lossy()),
            (None, Some(_)) | (None, None) => format!("{name} ({suffix})"),
        }
    };
    path.with_file_name(renamed)
}

fn available_conflict_name(
    backend: &dyn SftpBackend,
    path: &Path,
    is_directory: bool,
) -> Result<PathBuf, SftpOpsError> {
    for sequence in 1..=10_000 {
        let candidate = conflict_name(path, is_directory, sequence);
        if !backend.entry_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(SftpOpsError::Operation(format!(
        "Could not find an available conflict name for {}",
        path.display()
    )))
}

fn path_is_strictly_newer(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
) -> Result<bool, SftpOpsError> {
    let source = source_backend.modification_time(source_path)?;
    let target = target_backend.modification_time(target_path)?;
    Ok(matches!((source, target), (Some(source), Some(target)) if source > target))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
    pub bytes_per_second: u64,
    pub eta: Option<Duration>,
    pub phase: TransferPhase,
}

pub struct ProgressTracker {
    total: u64,
}

impl ProgressTracker {
    pub fn new(total: u64) -> Self {
        Self { total }
    }

    pub fn record_at(&mut self, transferred: u64, elapsed: Duration) -> TransferProgress {
        let bytes_per_second = if transferred == 0 || elapsed.is_zero() {
            0
        } else {
            (transferred as u128)
                .saturating_mul(1_000_000_000)
                .checked_div(elapsed.as_nanos())
                .unwrap_or(0)
                .min(u64::MAX as u128) as u64
        };
        let eta = if transferred == 0 || transferred >= self.total {
            None
        } else {
            let remaining = self.total - transferred;
            let nanos = elapsed
                .as_nanos()
                .saturating_mul(remaining as u128)
                .checked_div(transferred as u128)
                .unwrap_or(0)
                .min(u64::MAX as u128) as u64;
            Some(Duration::from_nanos(nanos))
        };
        TransferProgress {
            transferred,
            total: self.total,
            bytes_per_second,
            eta,
            phase: TransferPhase::Transferring,
        }
    }
}

pub struct TransferControl {
    state: Mutex<TransferControlState>,
    resumed: Condvar,
    transferred: AtomicU64,
    total: AtomicU64,
    bytes_per_second: AtomicU64,
    eta_nanos_plus_one: AtomicU64,
    phase: AtomicU8,
    #[cfg(test)]
    finalizing_calls: AtomicU64,
    #[cfg(test)]
    before_finalizing: Mutex<Option<(u64, Arc<dyn Fn() + Send + Sync>)>>,
    #[cfg(test)]
    after_finalizing: Mutex<Option<(u64, Arc<dyn Fn() + Send + Sync>)>>,
}

#[derive(Default)]
struct TransferControlState {
    cancelled: bool,
    paused: bool,
    finalizing: bool,
}

impl Default for TransferControl {
    fn default() -> Self {
        Self {
            state: Mutex::new(TransferControlState::default()),
            resumed: Condvar::new(),
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(0),
            bytes_per_second: AtomicU64::new(0),
            eta_nanos_plus_one: AtomicU64::new(0),
            phase: AtomicU8::new(0),
            #[cfg(test)]
            finalizing_calls: AtomicU64::new(0),
            #[cfg(test)]
            before_finalizing: Mutex::new(None),
            #[cfg(test)]
            after_finalizing: Mutex::new(None),
        }
    }
}

impl TransferControl {
    pub fn new(total: u64) -> Self {
        Self {
            total: AtomicU64::new(total),
            ..Self::default()
        }
    }

    pub fn cancel(&self) -> bool {
        let mut state = self.state.lock().expect("transfer control lock poisoned");
        if state.finalizing {
            return false;
        }
        state.cancelled = true;
        drop(state);
        self.resumed.notify_all();
        true
    }

    pub fn pause(&self) -> bool {
        let mut state = self.state.lock().expect("transfer control lock poisoned");
        if state.finalizing {
            return false;
        }
        state.paused = true;
        true
    }

    pub fn resume(&self) -> bool {
        let mut state = self.state.lock().expect("transfer control lock poisoned");
        if state.finalizing {
            return false;
        }
        state.paused = false;
        drop(state);
        self.resumed.notify_all();
        true
    }

    pub fn progress(&self) -> TransferProgress {
        let eta_nanos_plus_one = self.eta_nanos_plus_one.load(Ordering::SeqCst);
        TransferProgress {
            transferred: self.transferred.load(Ordering::SeqCst),
            total: self.total.load(Ordering::SeqCst),
            bytes_per_second: self.bytes_per_second.load(Ordering::SeqCst),
            eta: eta_nanos_plus_one.checked_sub(1).map(Duration::from_nanos),
            phase: match self.phase.load(Ordering::SeqCst) {
                0 => TransferPhase::Transferring,
                1 => TransferPhase::Verifying,
                2 => TransferPhase::Finalizing,
                _ => unreachable!("invalid transfer phase"),
            },
        }
    }

    fn wait_until_runnable(&self) -> Result<(), SftpOpsError> {
        let mut state = self.state.lock().expect("transfer control lock poisoned");
        while state.paused && !state.cancelled {
            state = self
                .resumed
                .wait(state)
                .expect("transfer control lock poisoned");
        }
        if state.cancelled {
            Err(SftpOpsError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .expect("transfer control lock poisoned")
            .cancelled
    }

    fn begin_finalizing(&self) -> Result<(), SftpOpsError> {
        #[cfg(test)]
        {
            let call = self.finalizing_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let hook = {
                let mut hook = self
                    .before_finalizing
                    .lock()
                    .expect("finalizing hook lock poisoned");
                if hook
                    .as_ref()
                    .is_some_and(|(expected_call, _)| *expected_call == call)
                {
                    hook.take().map(|(_, hook)| hook)
                } else {
                    None
                }
            };
            if let Some(hook) = hook {
                hook();
            }
        }
        let mut state = self.state.lock().expect("transfer control lock poisoned");
        while state.paused && !state.cancelled {
            state = self
                .resumed
                .wait(state)
                .expect("transfer control lock poisoned");
        }
        if state.cancelled {
            return Err(SftpOpsError::Cancelled);
        }
        state.finalizing = true;
        #[cfg(test)]
        {
            let call = self.finalizing_calls.load(Ordering::SeqCst);
            let hook = {
                let mut hook = self
                    .after_finalizing
                    .lock()
                    .expect("finalizing hook lock poisoned");
                if hook
                    .as_ref()
                    .is_some_and(|(expected_call, _)| *expected_call == call)
                {
                    hook.take().map(|(_, hook)| hook)
                } else {
                    None
                }
            };
            drop(state);
            if let Some(hook) = hook {
                hook();
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_before_finalizing_hook(&self, call: u64, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .before_finalizing
            .lock()
            .expect("finalizing hook lock poisoned") = Some((call, Arc::new(hook)));
    }

    #[cfg(test)]
    fn set_after_finalizing_hook(&self, call: u64, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .after_finalizing
            .lock()
            .expect("finalizing hook lock poisoned") = Some((call, Arc::new(hook)));
    }

    pub(crate) fn record(&self, progress: TransferProgress) {
        self.transferred
            .store(progress.transferred, Ordering::SeqCst);
        self.total.store(progress.total, Ordering::SeqCst);
        self.bytes_per_second
            .store(progress.bytes_per_second, Ordering::SeqCst);
        self.eta_nanos_plus_one.store(
            progress
                .eta
                .map(|eta| eta.as_nanos().min((u64::MAX - 1) as u128) as u64 + 1)
                .unwrap_or(0),
            Ordering::SeqCst,
        );
        self.phase.store(
            match progress.phase {
                TransferPhase::Transferring => 0,
                TransferPhase::Verifying => 1,
                TransferPhase::Finalizing => 2,
            },
            Ordering::SeqCst,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Completed,
    PartiallyCompleted {
        transferred: usize,
        published: usize,
        skipped: usize,
        source_kept: bool,
    },
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    CleanupCompleted,
    SourceRestored,
    DestinationCommittedSourcePreserved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntrySnapshot {
    root: PathBuf,
    entries: BTreeMap<PathBuf, super::sftp_backend::StableEntryIdentity>,
    children: BTreeMap<PathBuf, Vec<String>>,
}

struct BackupSnapshot {
    path: PathBuf,
    snapshot: EntrySnapshot,
    publication: EntrySnapshot,
    ownership: Option<PathOwnership>,
}

struct OwnedPathError {
    error: SftpOpsError,
    ownership: PathOwnership,
}

#[derive(Clone)]
struct PathOwnership {
    root: PathBuf,
    owned: BTreeMap<PathBuf, OwnedEntryIdentity>,
    unresolved: BTreeSet<PathBuf>,
    anchored_recovery: Vec<AnchoredRecoveryUnit>,
    retained_anchors: Vec<Arc<dyn BackendOwnershipAnchor>>,
}

#[derive(Clone)]
enum AnchoredRecoveryAction {
    RestoreSource {
        source: PathBuf,
        quarantine: PathBuf,
    },
    CleanupOwned {
        candidates: Vec<PathBuf>,
    },
}

#[derive(Clone)]
struct AnchoredRecoveryUnit {
    anchor: Arc<dyn BackendOwnershipAnchor>,
    action: AnchoredRecoveryAction,
}

impl AnchoredRecoveryUnit {
    fn paths(&self) -> Vec<PathBuf> {
        match &self.action {
            AnchoredRecoveryAction::RestoreSource { source, quarantine } => {
                vec![source.clone(), quarantine.clone()]
            }
            AnchoredRecoveryAction::CleanupOwned { candidates } => candidates.clone(),
        }
    }
}

#[derive(Clone)]
struct OwnedEntryIdentity {
    reserved: super::sftp_backend::StableEntryIdentity,
    guard: super::sftp_backend::StableEntryIdentity,
    anchor: Arc<dyn BackendOwnershipAnchor>,
}

impl OwnedEntryIdentity {
    fn new(
        identity: super::sftp_backend::StableEntryIdentity,
        anchor: Arc<dyn BackendOwnershipAnchor>,
    ) -> Self {
        Self {
            reserved: identity.clone(),
            guard: identity,
            anchor,
        }
    }
}

impl PathOwnership {
    fn empty(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            owned: BTreeMap::new(),
            unresolved: BTreeSet::new(),
            anchored_recovery: Vec::new(),
            retained_anchors: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.owned.is_empty()
            && self.unresolved.is_empty()
            && self.anchored_recovery.is_empty()
            && self.retained_anchors.is_empty()
    }
}

impl EntrySnapshot {
    fn root_identity(&self) -> &super::sftp_backend::StableEntryIdentity {
        self.entries
            .get(&self.root)
            .expect("snapshot always contains its root")
    }

    fn relocated(&self, new_root: &Path) -> Self {
        let relocate = |path: &Path| {
            path.strip_prefix(&self.root)
                .map(|relative| new_root.join(relative))
                .unwrap_or_else(|_| new_root.to_path_buf())
        };
        Self {
            root: new_root.to_path_buf(),
            entries: self
                .entries
                .iter()
                .map(|(path, identity)| (relocate(path), identity.clone()))
                .collect(),
            children: self
                .children
                .iter()
                .map(|(path, names)| (relocate(path), names.clone()))
                .collect(),
        }
    }

    fn total_file_size(&self) -> u64 {
        self.entries
            .values()
            .filter(|identity| identity.file_type == FileEntryType::File)
            .map(|identity| identity.size)
            .sum()
    }

    fn subtree(&self, root: &Path) -> Option<Self> {
        self.entries.get(root)?;
        Some(Self {
            root: root.to_path_buf(),
            entries: self
                .entries
                .iter()
                .filter(|(path, _)| path.starts_with(root))
                .map(|(path, identity)| (path.clone(), identity.clone()))
                .collect(),
            children: self
                .children
                .iter()
                .filter(|(path, _)| path.starts_with(root))
                .map(|(path, names)| (path.clone(), names.clone()))
                .collect(),
        })
    }

    fn is_safe_remainder_of(&self, original: &Self) -> bool {
        self.entries.iter().all(|(path, identity)| {
            original.entries.get(path).is_some_and(|expected| {
                expected == identity
                    || (expected.file_type == identity.file_type
                        && expected.size == identity.size
                        && !expected.object_id.is_empty()
                        && expected.object_id == identity.object_id)
            })
        }) && self.children.iter().all(|(path, names)| {
            let Some(expected_names) = original.children.get(path) else {
                return false;
            };
            names.iter().all(|name| expected_names.contains(name))
        })
    }
}

#[derive(Clone)]
enum CleanupRecoveryUnit {
    Verified {
        backend: Arc<dyn SftpBackend>,
        path: PathBuf,
        snapshot: EntrySnapshot,
        publication: EntrySnapshot,
    },
    Unresolved {
        backend: Arc<dyn SftpBackend>,
        ownership: PathOwnership,
    },
}

#[derive(Clone)]
struct CleanupRecovery {
    units: Vec<CleanupRecoveryUnit>,
    retained_anchors: Vec<Arc<dyn BackendOwnershipAnchor>>,
}

static NEXT_RECOVERY_ID: AtomicU64 = AtomicU64::new(1);
static RECOVERY_ACTIONS: OnceLock<Mutex<HashMap<u64, CleanupRecovery>>> = OnceLock::new();

fn next_monotonic_id(counter: &AtomicU64, kind: &str) -> Result<u64, SftpOpsError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| SftpOpsError::Operation(format!("{kind} exhausted")))
}

fn recovery_actions() -> &'static Mutex<HashMap<u64, CleanupRecovery>> {
    RECOVERY_ACTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn retry_recovery(recovery_id: u64) -> Result<RecoveryOutcome, SftpOpsError> {
    retry_recovery_controlled(recovery_id, &TransferControl::default(), None)
}

pub fn retry_recovery_controlled(
    recovery_id: u64,
    control: &TransferControl,
    mut progress_callback: Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<RecoveryOutcome, SftpOpsError> {
    let mut recovery = recovery_actions()
        .lock()
        .expect("transfer recovery registry lock poisoned")
        .remove(&recovery_id)
        .ok_or_else(|| {
            SftpOpsError::Operation(format!("Recovery action {recovery_id} no longer exists"))
        })?;
    match retry_cleanup(&mut recovery, control, &mut progress_callback) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            recovery_actions()
                .lock()
                .expect("transfer recovery registry lock poisoned")
                .insert(recovery_id, recovery);
            Err(error)
        }
    }
}

fn retry_cleanup(
    recovery: &mut CleanupRecovery,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<RecoveryOutcome, SftpOpsError> {
    control.wait_until_runnable()?;
    let mut outcome = RecoveryOutcome::CleanupCompleted;
    for unit in &mut recovery.units {
        match unit {
            CleanupRecoveryUnit::Verified {
                backend,
                path,
                snapshot,
                publication,
            } => {
                let current = match optional_snapshot_controlled(
                    &**backend,
                    path,
                    snapshot.total_file_size(),
                    control,
                    progress_callback,
                    TransferPhase::Verifying,
                )? {
                    Some(snapshot) => snapshot,
                    None => continue,
                };
                if current != *snapshot && !current.is_safe_remainder_of(snapshot) {
                    return Err(SftpOpsError::Operation(format!(
                        "Recovery path changed since it was retained: {}",
                        path.display()
                    )));
                }
                let current_publication = capture_publication_snapshot_controlled(
                    &**backend,
                    path,
                    current.total_file_size(),
                    control,
                    progress_callback,
                )?;
                if !current_publication.is_safe_remainder_of(publication) {
                    return Err(SftpOpsError::Operation(format!(
                        "Recovery path changed since it was retained: {}",
                        path.display()
                    )));
                }
                begin_required_cleanup(control, progress_callback, current.total_file_size())?;
                if let Err(error) = remove_snapshot_root_controlled(
                    &**backend,
                    &current,
                    &current_publication,
                    control,
                    progress_callback,
                    TransferPhase::Finalizing,
                ) {
                    match optional_snapshot_controlled(
                        &**backend,
                        path,
                        current.total_file_size(),
                        control,
                        progress_callback,
                        TransferPhase::Finalizing,
                    )? {
                        None => continue,
                        Some(remaining) if remaining.is_safe_remainder_of(&current) => {
                            *publication = capture_publication_snapshot_in_phase(
                                &**backend,
                                path,
                                remaining.total_file_size(),
                                control,
                                progress_callback,
                                TransferPhase::Finalizing,
                            )?;
                            *snapshot = remaining;
                            return Err(error);
                        }
                        Some(_) => {
                            return Err(SftpOpsError::Operation(format!(
                                "{error}; recovery path changed during cleanup: {}",
                                path.display()
                            )));
                        }
                    }
                }
            }
            CleanupRecoveryUnit::Unresolved { backend, ownership } => match cleanup_owned_manifest(
                &**backend,
                ownership,
                control,
                progress_callback,
                TransferPhase::Finalizing,
            )? {
                RecoveryOutcome::CleanupCompleted => {}
                RecoveryOutcome::SourceRestored => {
                    if outcome != RecoveryOutcome::DestinationCommittedSourcePreserved {
                        outcome = RecoveryOutcome::SourceRestored;
                    }
                }
                RecoveryOutcome::DestinationCommittedSourcePreserved => {
                    outcome = RecoveryOutcome::DestinationCommittedSourcePreserved;
                }
            },
        }
    }
    recovery.units.clear();
    recovery.retained_anchors.clear();
    Ok(outcome)
}

fn recovery_error(
    message: impl Into<String>,
    paths: Vec<PathBuf>,
    committed: bool,
) -> SftpOpsError {
    SftpOpsError::RecoveryRequired {
        message: message.into(),
        recovery_id: None,
        paths,
        committed,
    }
}

fn cleanup_recovery_error(
    message: impl Into<String>,
    backend: Arc<dyn SftpBackend>,
    path: PathBuf,
    previous_snapshot: &EntrySnapshot,
    previous_publication: &EntrySnapshot,
    committed: bool,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    match optional_snapshot_controlled(
        &*backend,
        &path,
        previous_snapshot.total_file_size(),
        control,
        progress_callback,
        TransferPhase::Finalizing,
    ) {
        Ok(None) => {
            let mut ownership = PathOwnership::empty(&path);
            ownership.unresolved.insert(path);
            ownership_recovery_error(message, backend, ownership, committed)
        }
        Ok(Some(snapshot)) if snapshot.is_safe_remainder_of(previous_snapshot) => {
            let publication = match capture_publication_snapshot_in_phase(
                &*backend,
                &path,
                snapshot.total_file_size(),
                control,
                progress_callback,
                TransferPhase::Finalizing,
            ) {
                Ok(publication) if publication.is_safe_remainder_of(previous_publication) => {
                    publication
                }
                Ok(_) | Err(_) => {
                    return recovery_error(
                        format!(
                            "{}; retained content changed and requires manual inspection",
                            message.into()
                        ),
                        vec![path],
                        committed,
                    );
                }
            };
            let recovery_id = match next_monotonic_id(&NEXT_RECOVERY_ID, "transfer recovery ID") {
                Ok(recovery_id) => recovery_id,
                Err(error) => {
                    return recovery_error(
                        format!("{}; {error}", message.into()),
                        vec![path],
                        committed,
                    );
                }
            };
            recovery_actions()
                .lock()
                .expect("transfer recovery registry lock poisoned")
                .insert(
                    recovery_id,
                    CleanupRecovery {
                        units: vec![CleanupRecoveryUnit::Verified {
                            backend,
                            path: path.clone(),
                            snapshot,
                            publication,
                        }],
                        retained_anchors: Vec::new(),
                    },
                );
            SftpOpsError::RecoveryRequired {
                message: message.into(),
                recovery_id: Some(recovery_id),
                paths: vec![path],
                committed,
            }
        }
        Ok(Some(_)) => recovery_error(
            format!(
                "{}; retained path changed and requires manual inspection",
                message.into()
            ),
            vec![path],
            committed,
        ),
        Err(probe_error) => recovery_error(
            format!(
                "{}; probing retained path failed: {probe_error}",
                message.into()
            ),
            vec![path],
            committed,
        ),
    }
}

fn cleanup_failure_with_backend_recovery(
    message: impl Into<String>,
    error: &SftpOpsError,
    backend: Arc<dyn SftpBackend>,
    path: PathBuf,
    previous_snapshot: &EntrySnapshot,
    previous_publication: &EntrySnapshot,
    committed: bool,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    let message = message.into();
    if error.destination_committed() && matches!(backend.entry_exists(&path), Ok(false)) {
        return SftpOpsError::Committed(message);
    }
    let mut units = Vec::new();
    let mut retained_paths = error.recovery_paths().to_vec();
    for recovery_path in error.recovery_paths() {
        match backend.entry_exists(recovery_path) {
            Ok(true) => {
                let actual = match capture_snapshot_controlled(
                    &*backend,
                    recovery_path,
                    previous_snapshot.total_file_size(),
                    control,
                    progress_callback,
                    TransferPhase::Finalizing,
                ) {
                    Ok(snapshot) => snapshot,
                    Err(probe_error) => {
                        return recovery_error(
                            format!(
                                "{message}; capturing backend recovery path failed: {probe_error}"
                            ),
                            retained_paths,
                            committed,
                        );
                    }
                };
                let publication = match capture_publication_snapshot_in_phase(
                    &*backend,
                    recovery_path,
                    actual.total_file_size(),
                    control,
                    progress_callback,
                    TransferPhase::Finalizing,
                ) {
                    Ok(publication) => publication,
                    Err(probe_error) => {
                        return recovery_error(
                            format!(
                                "{message}; hashing backend recovery path failed: {probe_error}"
                            ),
                            retained_paths,
                            committed,
                        );
                    }
                };
                let mut candidate_roots = previous_snapshot
                    .entries
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                candidate_roots
                    .sort_by_key(|candidate| std::cmp::Reverse(candidate.components().count()));
                let matched = candidate_roots.into_iter().find_map(|candidate| {
                    let expected_snapshot = previous_snapshot
                        .subtree(&candidate)?
                        .relocated(recovery_path);
                    let expected_publication = previous_publication
                        .subtree(&candidate)?
                        .relocated(recovery_path);
                    (actual.is_safe_remainder_of(&expected_snapshot)
                        && publication.is_safe_remainder_of(&expected_publication))
                    .then_some(())
                });
                if matched.is_some() {
                    units.push(CleanupRecoveryUnit::Verified {
                        backend: backend.clone(),
                        path: recovery_path.clone(),
                        snapshot: actual,
                        publication,
                    });
                }
            }
            Ok(false) => {}
            Err(probe_error) => {
                return recovery_error(
                    format!("{message}; probing backend recovery path failed: {probe_error}"),
                    error.recovery_paths().to_vec(),
                    committed,
                );
            }
        }
    }
    if !units.is_empty() {
        let recovery_id = match next_monotonic_id(&NEXT_RECOVERY_ID, "transfer recovery ID") {
            Ok(recovery_id) => recovery_id,
            Err(error) => {
                return recovery_error(format!("{message}; {error}"), retained_paths, committed);
            }
        };
        recovery_actions()
            .lock()
            .expect("transfer recovery registry lock poisoned")
            .insert(
                recovery_id,
                CleanupRecovery {
                    units,
                    retained_anchors: Vec::new(),
                },
            );
        retained_paths.sort();
        retained_paths.dedup();
        return SftpOpsError::RecoveryRequired {
            message,
            recovery_id: Some(recovery_id),
            paths: retained_paths,
            committed,
        };
    }
    if !retained_paths.is_empty() {
        retained_paths.sort();
        retained_paths.dedup();
        return recovery_error(
            format!("{message}; retained backend paths require manual inspection"),
            retained_paths,
            committed,
        );
    }
    cleanup_recovery_error(
        message,
        backend,
        path,
        previous_snapshot,
        previous_publication,
        committed,
        control,
        progress_callback,
    )
}

pub fn run_transfer(
    job: &TransferJob,
    control: &TransferControl,
    mut progress_callback: Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<TransferOutcome, SftpOpsError> {
    if Arc::ptr_eq(&job.source_backend, &job.target_backend) {
        super::sftp_backend::validate_copy_destination(&job.source_path, &job.target_path, false)?;
    }
    let source_identity = stable_identity_now(&*job.source_backend, &job.source_path)?;
    if source_identity.file_type != FileEntryType::File {
        return Err(SftpOpsError::Operation(format!(
            "Refusing to transfer non-regular source {}",
            job.source_path.display()
        )));
    }
    let source_snapshot = capture_snapshot(&*job.source_backend, &job.source_path)?;
    validate_snapshot(&*job.source_backend, &source_snapshot)?;

    let original_target = optional_snapshot_controlled(
        &*job.target_backend,
        &job.target_path,
        source_snapshot.total_file_size(),
        control,
        &mut progress_callback,
        TransferPhase::Verifying,
    )?;
    if original_target.is_some() && job.conflict == ConflictDecision::Rename {
        let mut renamed_job = job.clone();
        renamed_job.target_path =
            available_conflict_name(&*job.target_backend, &job.target_path, false)?;
        return run_transfer(&renamed_job, control, progress_callback);
    }
    if original_target.is_some() && job.conflict == ConflictDecision::Skip {
        return Ok(TransferOutcome::Skipped);
    }
    if original_target.is_some()
        && job.conflict == ConflictDecision::NewerOnly
        && !path_is_strictly_newer(
            &*job.source_backend,
            &job.source_path,
            &*job.target_backend,
            &job.target_path,
        )?
    {
        return Ok(TransferOutcome::Skipped);
    }
    if original_target
        .as_ref()
        .is_some_and(|snapshot| snapshot.root_identity().file_type != FileEntryType::File)
    {
        return Err(SftpOpsError::Operation(format!(
            "Refusing to replace non-regular destination {}",
            job.target_path.display()
        )));
    }
    preflight_transfer_capabilities(job, original_target.is_some())?;
    let source_anchor = capture_move_source_anchor(job, &source_identity)?;
    let source_publication = capture_publication_snapshot_controlled(
        &*job.source_backend,
        &job.source_path,
        source_snapshot.total_file_size(),
        control,
        &mut progress_callback,
    )?;
    let original_target_publication = original_target
        .as_ref()
        .map(|snapshot| {
            let publication = capture_publication_snapshot_controlled(
                &*job.target_backend,
                &job.target_path,
                snapshot.total_file_size(),
                control,
                &mut progress_callback,
            )?;
            validate_snapshot(&*job.target_backend, snapshot)?;
            Ok::<EntrySnapshot, SftpOpsError>(publication)
        })
        .transpose()?;

    let initial = TransferProgress {
        transferred: 0,
        total: source_identity.size,
        bytes_per_second: 0,
        eta: None,
        phase: TransferPhase::Transferring,
    };
    control.record(initial);
    if let Some(callback) = progress_callback.as_mut() {
        callback(initial);
    }

    let staged_path = temporary_target_path(&job.target_path, "transfer")?;
    let mut stage_ownership = match stream_file_to_new_path_owned(
        &*job.source_backend,
        &job.source_path,
        &source_identity,
        &*job.target_backend,
        &staged_path,
        control,
        &mut progress_callback,
        0,
        source_identity.size,
        Instant::now(),
    ) {
        Ok((_, ownership)) => ownership,
        Err(failure) => {
            return cleanup_failed_stage(
                failure.error,
                job.target_backend.clone(),
                &staged_path,
                failure.ownership,
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    let staged_snapshot = match capture_snapshot(&*job.target_backend, &staged_path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership,
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    if let Err(error) = bind_snapshot_to_reserved_ownership(&mut stage_ownership, &staged_snapshot)
    {
        return cleanup_failed_stage(
            error,
            job.target_backend.clone(),
            &staged_path,
            stage_ownership,
            false,
            control,
            &mut progress_callback,
        );
    }
    let published_target_anchor = match owned_root_anchor(&stage_ownership, &staged_path) {
        Ok(anchor) => anchor,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    let staged_publication = match capture_publication_snapshot_controlled(
        &*job.target_backend,
        &staged_path,
        source_identity.size,
        control,
        &mut progress_callback,
    ) {
        Ok(publication) => publication,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    if staged_snapshot.root_identity().file_type != FileEntryType::File
        || staged_publication != source_publication.relocated(&staged_path)
    {
        return cleanup_failed_stage(
            SftpOpsError::Operation(format!(
                "Staged target verification failed for {}",
                job.target_path.display()
            )),
            job.target_backend.clone(),
            &staged_path,
            stage_ownership.clone(),
            false,
            control,
            &mut progress_callback,
        );
    }

    let backup = match (&original_target, &original_target_publication) {
        (Some(target_snapshot), Some(target_publication)) => {
            match create_verified_backup(
                job.target_backend.clone(),
                target_snapshot,
                target_publication,
                &job.target_path,
                control,
                &mut progress_callback,
            ) {
                Ok(backup) => Some(backup),
                Err(error) => {
                    return cleanup_failed_stage(
                        error,
                        job.target_backend.clone(),
                        &staged_path,
                        stage_ownership.clone(),
                        false,
                        control,
                        &mut progress_callback,
                    );
                }
            }
        }
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => {
            return cleanup_failed_stage(
                SftpOpsError::Operation("Incomplete destination backup identity".to_string()),
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };

    control.wait_until_runnable().map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_snapshot(&*job.source_backend, &source_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_snapshot(&*job.target_backend, &staged_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_original_target(
        &*job.target_backend,
        &job.target_path,
        original_target.as_ref(),
        original_target_publication.as_ref(),
        control,
        &mut progress_callback,
    )
    .map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    control.wait_until_runnable().map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    begin_finalizing(control, &mut progress_callback, source_identity.size).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;

    let expected_publication = staged_publication.relocated(&job.target_path);
    let displaced = match (&original_target, &original_target_publication) {
        (Some(original), Some(original_publication)) => {
            match exchange_staged_with_target(
                &*job.target_backend,
                &staged_path,
                &job.target_path,
                &staged_snapshot,
                &staged_publication,
                original,
                original_publication,
                control,
                &mut progress_callback,
            ) {
                Ok(displaced) => Some(displaced),
                Err(SftpOpsError::RecoveryRequired {
                    message,
                    recovery_id,
                    mut paths,
                    committed,
                }) => {
                    if let Some(backup) = &backup {
                        paths.push(backup.path.clone());
                    }
                    return Err(SftpOpsError::RecoveryRequired {
                        message,
                        recovery_id,
                        paths,
                        committed,
                    });
                }
                Err(error) => {
                    return Err(cleanup_before_publish(
                        error,
                        job.target_backend.clone(),
                        &staged_path,
                        &staged_snapshot,
                        &stage_ownership,
                        backup.as_ref(),
                        control,
                    ));
                }
            }
        }
        (None, None) => {
            let publish_error = job
                .target_backend
                .rename(&staged_path, &job.target_path)
                .err();
            match resolve_publish(
                &*job.target_backend,
                &staged_path,
                &job.target_path,
                &staged_snapshot,
                &expected_publication,
                None,
                control,
                &mut progress_callback,
            )
            .unwrap_or(PublishState::Ambiguous)
            {
                PublishState::Committed => None,
                PublishState::NotCommitted => {
                    return Err(cleanup_before_publish(
                        publish_error.unwrap_or_else(|| {
                            SftpOpsError::Operation(format!(
                                "Publishing {} did not install the staged entry",
                                job.target_path.display()
                            ))
                        }),
                        job.target_backend.clone(),
                        &staged_path,
                        &staged_snapshot,
                        &stage_ownership,
                        backup.as_ref(),
                        control,
                    ));
                }
                PublishState::Ambiguous => {
                    return Err(recovery_error(
                        format!(
                            "{}; publish acknowledgement could not be resolved from stable identities",
                            publish_error
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "Publish state is inconsistent".to_string())
                        ),
                        vec![staged_path.clone(), job.target_path.clone()],
                        false,
                    ));
                }
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(recovery_error(
                "Incomplete destination identity during publish",
                vec![staged_path.clone(), job.target_path.clone()],
                false,
            ));
        }
    };
    let published_snapshot = capture_snapshot(&*job.target_backend, &job.target_path)?;
    if let Err(error) = verify_anchor_at_path(
        &published_target_anchor,
        &job.target_path,
        published_snapshot.root_identity(),
    ) {
        return Err(rollback_file_publish(
            job,
            error,
            &published_snapshot,
            &expected_publication,
            displaced.as_ref(),
            backup.as_ref(),
            control,
            &mut progress_callback,
        ));
    }
    let published_publication = capture_publication_snapshot_controlled(
        &*job.target_backend,
        &job.target_path,
        source_identity.size,
        control,
        &mut progress_callback,
    );
    if !published_publication
        .as_ref()
        .is_ok_and(|publication| *publication == expected_publication)
    {
        let error = published_publication.err().unwrap_or_else(|| {
            SftpOpsError::Operation(format!(
                "Published target identity does not match the staged file at {}",
                job.target_path.display()
            ))
        });
        return Err(rollback_file_publish(
            job,
            error,
            &published_snapshot,
            &expected_publication,
            displaced.as_ref(),
            backup.as_ref(),
            control,
            &mut progress_callback,
        ));
    }

    let pre_delete = (|| {
        verify_anchor_at_path(
            &published_target_anchor,
            &job.target_path,
            published_snapshot.root_identity(),
        )?;
        validate_snapshot(&*job.target_backend, &published_snapshot)?;
        control.wait_until_runnable()?;
        let current_source = stable_identity_now(&*job.source_backend, &job.source_path)?;
        if current_source != source_identity {
            return Err(SftpOpsError::Operation(format!(
                "Source identity changed before move completion for {}",
                job.source_path.display()
            )));
        }
        if capture_publication_snapshot_controlled(
            &*job.source_backend,
            &job.source_path,
            source_identity.size,
            control,
            &mut progress_callback,
        )? != source_publication
        {
            return Err(SftpOpsError::Operation(format!(
                "Source content changed before move completion for {}",
                job.source_path.display()
            )));
        }
        if let Some(anchor) = source_anchor.as_ref() {
            if !anchor.matches_path(&job.source_path)? {
                return Err(SftpOpsError::Operation(format!(
                    "Move source ownership changed before quarantine for {}",
                    job.source_path.display()
                )));
            }
        }
        validate_snapshot(&*job.target_backend, &published_snapshot)?;
        control.wait_until_runnable()
    })();
    if let Err(error) = pre_delete {
        return Err(rollback_file_publish(
            job,
            error,
            &published_snapshot,
            &expected_publication,
            displaced.as_ref(),
            backup.as_ref(),
            control,
            &mut progress_callback,
        ));
    }

    if job.operation == TransferOperation::Move {
        let source_anchor = source_anchor
            .as_ref()
            .expect("move preflight always returns a source ownership anchor")
            .clone();
        let quarantine = temporary_target_path(&job.source_path, "source")?;
        control.begin_finalizing()?;
        if !source_anchor.matches_path(&job.source_path)? {
            return Err(SftpOpsError::Operation(format!(
                "Move source ownership changed immediately before quarantine: {}",
                job.source_path.display()
            )));
        }
        let rename_error = job
            .source_backend
            .rename_if_matches(&job.source_path, &quarantine, source_anchor.clone())
            .err();
        if !source_anchor.matches_path(&quarantine).unwrap_or(false) {
            return Err(source_anchor_recovery_error(
                "Source ownership changed during quarantine rename",
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
                true,
            ));
        }
        let expected_quarantine_publication = source_publication.relocated(&quarantine);
        match resolve_publish(
            &*job.source_backend,
            &job.source_path,
            &quarantine,
            &source_snapshot,
            &expected_quarantine_publication,
            None,
            control,
            &mut progress_callback,
        )
        .unwrap_or(PublishState::Ambiguous)
        {
            PublishState::Committed => {}
            PublishState::NotCommitted => {
                return Err(rollback_file_publish(
                    job,
                    rename_error.unwrap_or_else(|| {
                        SftpOpsError::Operation(
                            "Source quarantine rename was not committed".to_string(),
                        )
                    }),
                    &published_snapshot,
                    &expected_publication,
                    displaced.as_ref(),
                    backup.as_ref(),
                    control,
                    &mut progress_callback,
                ));
            }
            PublishState::Ambiguous => {
                return Err(source_anchor_recovery_error(
                    format!(
                        "{}; source quarantine state is indeterminate",
                        rename_error
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "Source quarantine identity mismatch".to_string())
                    ),
                    job.source_backend.clone(),
                    &job.source_path,
                    &quarantine,
                    source_anchor.clone(),
                    true,
                ));
            }
        }
        let quarantined_snapshot = match capture_snapshot(&*job.source_backend, &quarantine) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(source_anchor_recovery_error(
                    format!("Capturing source quarantine failed: {error}"),
                    job.source_backend.clone(),
                    &job.source_path,
                    &quarantine,
                    source_anchor.clone(),
                    true,
                ));
            }
        };
        if capture_publication_snapshot_controlled(
            &*job.source_backend,
            &quarantine,
            source_identity.size,
            control,
            &mut progress_callback,
        )? != expected_quarantine_publication
        {
            return Err(source_anchor_recovery_error(
                format!(
                    "Source quarantine content changed at {}",
                    quarantine.display()
                ),
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor.clone(),
                true,
            ));
        }
        let target_after_quarantine = capture_publication_snapshot_controlled(
            &*job.target_backend,
            &job.target_path,
            source_identity.size,
            control,
            &mut progress_callback,
        );
        if !target_after_quarantine
            .as_ref()
            .is_ok_and(|publication| *publication == expected_publication)
        {
            let primary = target_after_quarantine.err().unwrap_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Published target changed after source quarantine at {}",
                    job.target_path.display()
                ))
            });
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                primary,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = verify_anchor_at_path(
            &published_target_anchor,
            &job.target_path,
            published_snapshot.root_identity(),
        ) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = begin_finalizing(control, &mut progress_callback, source_identity.size)
        {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = verify_anchor_at_path(
            &published_target_anchor,
            &job.target_path,
            published_snapshot.root_identity(),
        ) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if !source_anchor.matches_path(&quarantine).unwrap_or(false) {
            return Err(source_anchor_recovery_error(
                "Source quarantine ownership changed before destructive cleanup",
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
                true,
            ));
        }
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.source_backend,
            &quarantined_snapshot,
            &expected_quarantine_publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            let recovery = cleanup_failure_with_backend_recovery(
                format!(
                    "Target {} remains committed because source quarantine cleanup failed: {error}",
                    job.target_path.display()
                ),
                &error,
                job.source_backend.clone(),
                quarantine.clone(),
                &quarantined_snapshot,
                &expected_quarantine_publication,
                true,
                control,
                &mut progress_callback,
            );
            return Err(retain_source_anchor_for_recovery(
                recovery,
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
            ));
        }
    }

    if displaced.is_some() || backup.is_some() {
        if let Err(error) =
            begin_required_cleanup(control, &mut progress_callback, source_identity.size)
        {
            let mut paths = displaced
                .as_ref()
                .map(|snapshot| snapshot.path.clone())
                .into_iter()
                .collect::<Vec<_>>();
            paths.extend(
                backup
                    .as_ref()
                    .map(|snapshot| snapshot.path.clone())
                    .into_iter(),
            );
            return Err(recovery_error(
                format!("Transfer committed; cleanup was cancelled before finalizing: {error}"),
                paths,
                true,
            ));
        }
    }
    if let Some(displaced) = displaced {
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.target_backend,
            &displaced.snapshot,
            &displaced.publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            return Err(cleanup_failure_with_backend_recovery(
                format!("Transfer committed but displaced target cleanup failed: {error}"),
                &error,
                job.target_backend.clone(),
                displaced.path,
                &displaced.snapshot,
                &displaced.publication,
                true,
                control,
                &mut progress_callback,
            ));
        }
    }
    if let Some(backup) = backup {
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.target_backend,
            &backup.snapshot,
            &backup.publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            return Err(cleanup_failure_with_backend_recovery(
                format!("Transfer committed but destination backup cleanup failed: {error}"),
                &error,
                job.target_backend.clone(),
                backup.path,
                &backup.snapshot,
                &backup.publication,
                true,
                control,
                &mut progress_callback,
            ));
        }
    }
    Ok(TransferOutcome::Completed)
}

fn try_atomic_same_backend_directory_move(
    job: &TransferJob,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<Option<TransferOutcome>, SftpOpsError> {
    if job.operation != TransferOperation::Move
        || !Arc::ptr_eq(&job.source_backend, &job.target_backend)
    {
        return Ok(None);
    }
    let source_identity = stable_identity_now(&*job.source_backend, &job.source_path)?;
    if source_identity.file_type != FileEntryType::Directory
        || source_identity.object_id.is_empty()
        || job.target_backend.entry_exists(&job.target_path)?
    {
        return Ok(None);
    }
    let Some(source_anchor) = job
        .source_backend
        .existing_entry_ownership_anchor(&job.source_path)?
    else {
        return Ok(None);
    };

    job.source_backend
        .preflight_safe_mutation(&job.target_path, false)?;
    control.begin_finalizing()?;
    let progress = TransferProgress {
        transferred: 0,
        total: 0,
        bytes_per_second: 0,
        eta: None,
        phase: TransferPhase::Finalizing,
    };
    control.record(progress);
    if let Some(callback) = progress_callback.as_mut() {
        callback(progress);
    }
    job.source_backend
        .rename_if_matches(&job.source_path, &job.target_path, source_anchor)?;

    let target_identity =
        stable_identity_now(&*job.target_backend, &job.target_path).map_err(|error| {
            SftpOpsError::RecoveryRequired {
                message: format!(
                    "Atomic directory move destination verification failed for {} -> {}: {error}",
                    job.source_path.display(),
                    job.target_path.display()
                ),
                recovery_id: None,
                paths: vec![job.source_path.clone(), job.target_path.clone()],
                committed: true,
            }
        })?;
    let source_absent = !job
        .source_backend
        .entry_exists(&job.source_path)
        .map_err(|error| SftpOpsError::RecoveryRequired {
            message: format!(
                "Atomic directory move source verification failed for {} -> {}: {error}",
                job.source_path.display(),
                job.target_path.display()
            ),
            recovery_id: None,
            paths: vec![job.source_path.clone(), job.target_path.clone()],
            committed: true,
        })?;
    if source_absent
        && target_identity.file_type == FileEntryType::Directory
        && target_identity.object_id == source_identity.object_id
    {
        return Ok(Some(TransferOutcome::Completed));
    }
    Err(SftpOpsError::RecoveryRequired {
        message: format!(
            "Atomic directory move could not be verified: {} -> {}",
            job.source_path.display(),
            job.target_path.display()
        ),
        recovery_id: None,
        paths: vec![job.source_path.clone(), job.target_path.clone()],
        committed: true,
    })
}

pub fn run_directory_transfer(
    job: &TransferJob,
    control: &TransferControl,
    mut progress_callback: Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<TransferOutcome, SftpOpsError> {
    if Arc::ptr_eq(&job.source_backend, &job.target_backend) {
        super::sftp_backend::validate_copy_destination(&job.source_path, &job.target_path, true)?;
    }
    if let Some(outcome) =
        try_atomic_same_backend_directory_move(job, control, &mut progress_callback)?
    {
        return Ok(outcome);
    }
    let source_snapshot = capture_snapshot(&*job.source_backend, &job.source_path)?;
    if source_snapshot.root_identity().file_type != FileEntryType::Directory {
        return Err(SftpOpsError::Operation(format!(
            "Directory transfer source is not a directory: {}",
            job.source_path.display()
        )));
    }
    let original_target = optional_snapshot_controlled(
        &*job.target_backend,
        &job.target_path,
        source_snapshot.total_file_size(),
        control,
        &mut progress_callback,
        TransferPhase::Verifying,
    )?;
    if original_target.is_some() && job.conflict == ConflictDecision::Rename {
        let mut renamed_job = job.clone();
        renamed_job.target_path =
            available_conflict_name(&*job.target_backend, &job.target_path, true)?;
        return run_directory_transfer(&renamed_job, control, progress_callback);
    }
    if original_target.is_some() && job.conflict == ConflictDecision::Skip {
        return Ok(TransferOutcome::Skipped);
    }
    // A directory has no single recency value that can safely decide whether
    // replacing its entire tree is newer. Keep an existing target intact;
    // NewerOnly remains an exact, per-file policy.
    if original_target.is_some() && job.conflict == ConflictDecision::NewerOnly {
        return Ok(TransferOutcome::Skipped);
    }
    if original_target
        .as_ref()
        .is_some_and(|snapshot| snapshot.root_identity().file_type != FileEntryType::Directory)
    {
        return Err(SftpOpsError::Operation(format!(
            "Refusing to replace non-directory destination {}",
            job.target_path.display()
        )));
    }
    preflight_transfer_capabilities(job, original_target.is_some())?;
    let source_anchor = capture_move_source_anchor(job, source_snapshot.root_identity())?;
    let total = source_snapshot.total_file_size();
    let original_target_publication = original_target
        .as_ref()
        .map(|snapshot| {
            let publication = capture_publication_snapshot_controlled(
                &*job.target_backend,
                &job.target_path,
                snapshot.total_file_size(),
                control,
                &mut progress_callback,
            )?;
            validate_snapshot(&*job.target_backend, snapshot)?;
            Ok::<EntrySnapshot, SftpOpsError>(publication)
        })
        .transpose()?;

    let initial = TransferProgress {
        transferred: 0,
        total,
        bytes_per_second: 0,
        eta: None,
        phase: TransferPhase::Transferring,
    };
    control.record(initial);
    if let Some(callback) = progress_callback.as_mut() {
        callback(initial);
    }

    validate_snapshot(&*job.source_backend, &source_snapshot)?;
    control.wait_until_runnable()?;
    let staged_path = temporary_target_path(&job.target_path, "tree")?;
    let started = Instant::now();
    let mut transferred = 0_u64;
    let merge_skip = job.conflict == ConflictDecision::MergeSkip && original_target.is_some();
    let source_publication = capture_publication_snapshot_controlled(
        &*job.source_backend,
        &job.source_path,
        total,
        control,
        &mut progress_callback,
    )?;
    let target_publication = match (merge_skip, original_target.as_ref()) {
        (true, Some(snapshot)) => Some(capture_publication_snapshot_controlled(
            &*job.target_backend,
            &job.target_path,
            snapshot.total_file_size(),
            control,
            &mut progress_callback,
        )?),
        (false, Some(_)) | (false, None) | (true, None) => None,
    };
    let stage_result: Result<PathOwnership, OwnedPathError> =
        if let Some(target_snapshot) = original_target.as_ref().filter(|_| merge_skip) {
            copy_snapshot_to_new_root(
                &*job.target_backend,
                target_snapshot,
                &*job.target_backend,
                &staged_path,
                control,
            )
            .and_then(|ownership| {
                merge_snapshot_into_existing_root(
                    &*job.source_backend,
                    &source_snapshot,
                    target_snapshot,
                    &*job.target_backend,
                    &staged_path,
                    control,
                    &mut transferred,
                    total,
                    started,
                    &mut progress_callback,
                    ownership,
                )
                .map(|(_, ownership)| ownership)
            })
        } else {
            copy_snapshot_to_new_root_with_progress(
                &*job.source_backend,
                &source_snapshot,
                &*job.target_backend,
                &staged_path,
                control,
                &mut transferred,
                total,
                started,
                &mut progress_callback,
            )
        };
    let mut stage_ownership = match stage_result {
        Ok(ownership) => ownership,
        Err(failure) => {
            return cleanup_failed_stage(
                failure.error,
                job.target_backend.clone(),
                &staged_path,
                failure.ownership,
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    let staged_snapshot = match capture_snapshot(&*job.target_backend, &staged_path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership,
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    if let Err(error) = bind_snapshot_to_reserved_ownership(&mut stage_ownership, &staged_snapshot)
    {
        return cleanup_failed_stage(
            error,
            job.target_backend.clone(),
            &staged_path,
            stage_ownership,
            false,
            control,
            &mut progress_callback,
        );
    }
    let published_target_anchor = match owned_root_anchor(&stage_ownership, &staged_path) {
        Ok(anchor) => anchor,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    let staged_publication = match capture_publication_snapshot_controlled(
        &*job.target_backend,
        &staged_path,
        total,
        control,
        &mut progress_callback,
    ) {
        Ok(publication) => publication,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    let merge_result = match (merge_skip, &target_publication) {
        (true, Some(target)) => {
            let (expected, result) = merged_publication_entries(&source_publication, target);
            if publication_entries(&staged_publication) != expected {
                return cleanup_failed_stage(
                    SftpOpsError::Operation(format!(
                        "Staged merged directory verification failed for {}",
                        job.target_path.display()
                    )),
                    job.target_backend.clone(),
                    &staged_path,
                    stage_ownership.clone(),
                    false,
                    control,
                    &mut progress_callback,
                );
            }
            result
        }
        (false, None) => {
            if staged_publication != source_publication.relocated(&staged_path) {
                return cleanup_failed_stage(
                    SftpOpsError::Operation(format!(
                        "Staged directory content verification failed for {}",
                        job.target_path.display()
                    )),
                    job.target_backend.clone(),
                    &staged_path,
                    stage_ownership.clone(),
                    false,
                    control,
                    &mut progress_callback,
                );
            }
            MergeResult::default()
        }
        (true, None) | (false, Some(_)) => {
            return cleanup_failed_stage(
                SftpOpsError::Operation("Incomplete merged-directory identity".to_string()),
                job.target_backend.clone(),
                &staged_path,
                stage_ownership.clone(),
                false,
                control,
                &mut progress_callback,
            );
        }
    };
    validate_snapshot(&*job.source_backend, &source_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            None,
            control,
        )
    })?;
    validate_snapshot(&*job.target_backend, &staged_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            None,
            control,
        )
    })?;

    let backup = match (&original_target, &original_target_publication) {
        (Some(target_snapshot), Some(target_publication)) => {
            match create_verified_backup(
                job.target_backend.clone(),
                target_snapshot,
                target_publication,
                &job.target_path,
                control,
                &mut progress_callback,
            ) {
                Ok(backup) => Some(backup),
                Err(error) => {
                    return Err(cleanup_before_publish(
                        error,
                        job.target_backend.clone(),
                        &staged_path,
                        &staged_snapshot,
                        &stage_ownership,
                        None,
                        control,
                    ));
                }
            }
        }
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => {
            return Err(cleanup_before_publish(
                SftpOpsError::Operation("Incomplete destination backup identity".to_string()),
                job.target_backend.clone(),
                &staged_path,
                &staged_snapshot,
                &stage_ownership,
                None,
                control,
            ));
        }
    };

    control.wait_until_runnable().map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_snapshot(&*job.source_backend, &source_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_snapshot(&*job.target_backend, &staged_snapshot).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    validate_original_target(
        &*job.target_backend,
        &job.target_path,
        original_target.as_ref(),
        original_target_publication.as_ref(),
        control,
        &mut progress_callback,
    )
    .map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    control.wait_until_runnable().map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    begin_finalizing(control, &mut progress_callback, total).map_err(|error| {
        cleanup_before_publish(
            error,
            job.target_backend.clone(),
            &staged_path,
            &staged_snapshot,
            &stage_ownership,
            backup.as_ref(),
            control,
        )
    })?;
    let expected_publication = staged_publication.relocated(&job.target_path);
    let displaced = match (&original_target, &original_target_publication) {
        (Some(original), Some(original_publication)) => {
            match exchange_staged_with_target(
                &*job.target_backend,
                &staged_path,
                &job.target_path,
                &staged_snapshot,
                &staged_publication,
                original,
                original_publication,
                control,
                &mut progress_callback,
            ) {
                Ok(displaced) => Some(displaced),
                Err(SftpOpsError::RecoveryRequired {
                    message,
                    recovery_id,
                    mut paths,
                    committed,
                }) => {
                    if let Some(backup) = &backup {
                        paths.push(backup.path.clone());
                    }
                    return Err(SftpOpsError::RecoveryRequired {
                        message,
                        recovery_id,
                        paths,
                        committed,
                    });
                }
                Err(error) => {
                    return Err(cleanup_before_publish(
                        error,
                        job.target_backend.clone(),
                        &staged_path,
                        &staged_snapshot,
                        &stage_ownership,
                        backup.as_ref(),
                        control,
                    ));
                }
            }
        }
        (None, None) => {
            let publish_error = job
                .target_backend
                .rename(&staged_path, &job.target_path)
                .err();
            match resolve_publish(
                &*job.target_backend,
                &staged_path,
                &job.target_path,
                &staged_snapshot,
                &expected_publication,
                None,
                control,
                &mut progress_callback,
            )
            .unwrap_or(PublishState::Ambiguous)
            {
                PublishState::Committed => None,
                PublishState::NotCommitted => {
                    return Err(cleanup_before_publish(
                        publish_error.unwrap_or_else(|| {
                            SftpOpsError::Operation(
                                "Directory publish was not committed".to_string(),
                            )
                        }),
                        job.target_backend.clone(),
                        &staged_path,
                        &staged_snapshot,
                        &stage_ownership,
                        backup.as_ref(),
                        control,
                    ));
                }
                PublishState::Ambiguous => {
                    return Err(recovery_error(
                        format!(
                            "{}; directory publish state is indeterminate",
                            publish_error
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| {
                                    "Directory publish identity mismatch".to_string()
                                })
                        ),
                        vec![staged_path.clone(), job.target_path.clone()],
                        false,
                    ));
                }
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(recovery_error(
                "Incomplete destination identity during directory publish",
                vec![staged_path.clone(), job.target_path.clone()],
                false,
            ));
        }
    };
    let published_snapshot = capture_snapshot(&*job.target_backend, &job.target_path)?;
    if let Err(error) = verify_anchor_at_path(
        &published_target_anchor,
        &job.target_path,
        published_snapshot.root_identity(),
    ) {
        return Err(rollback_directory_publish(
            job,
            error,
            &published_snapshot,
            &expected_publication,
            displaced.as_ref(),
            backup.as_ref(),
            control,
            &mut progress_callback,
        ));
    }
    let published_publication = capture_publication_snapshot_controlled(
        &*job.target_backend,
        &job.target_path,
        total,
        control,
        &mut progress_callback,
    );
    if !published_publication
        .as_ref()
        .is_ok_and(|publication| *publication == expected_publication)
    {
        let error = published_publication.err().unwrap_or_else(|| {
            SftpOpsError::Operation(format!(
                "Published target tree does not match the staged tree at {}",
                job.target_path.display()
            ))
        });
        return Err(rollback_directory_publish(
            job,
            error,
            &published_snapshot,
            &expected_publication,
            displaced.as_ref(),
            backup.as_ref(),
            control,
            &mut progress_callback,
        ));
    }

    if job.operation == TransferOperation::Move && !merge_result.had_skips {
        let source_anchor = source_anchor
            .as_ref()
            .expect("move preflight always returns a source ownership anchor")
            .clone();
        let before_quarantine = (|| {
            control.wait_until_runnable()?;
            verify_anchor_at_path(
                &published_target_anchor,
                &job.target_path,
                published_snapshot.root_identity(),
            )?;
            validate_snapshot(&*job.source_backend, &source_snapshot)?;
            if capture_publication_snapshot_controlled(
                &*job.source_backend,
                &job.source_path,
                total,
                control,
                &mut progress_callback,
            )? != source_publication
            {
                return Err(SftpOpsError::Operation(format!(
                    "Source tree content changed before move completion for {}",
                    job.source_path.display()
                )));
            }
            if !source_anchor.matches_path(&job.source_path)? {
                return Err(SftpOpsError::Operation(format!(
                    "Move source ownership changed before quarantine for {}",
                    job.source_path.display()
                )));
            }
            validate_snapshot(&*job.target_backend, &published_snapshot)?;
            control.wait_until_runnable()
        })();
        if let Err(error) = before_quarantine {
            return Err(rollback_directory_publish(
                job,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }

        let quarantine = temporary_target_path(&job.source_path, "source")?;
        control.begin_finalizing()?;
        if !source_anchor.matches_path(&job.source_path)? {
            return Err(SftpOpsError::Operation(format!(
                "Directory move source ownership changed immediately before quarantine: {}",
                job.source_path.display()
            )));
        }
        let rename_error = job
            .source_backend
            .rename_if_matches(&job.source_path, &quarantine, source_anchor.clone())
            .err();
        if !source_anchor.matches_path(&quarantine).unwrap_or(false) {
            return Err(source_anchor_recovery_error(
                "Directory source ownership changed during quarantine rename",
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
                true,
            ));
        }
        let expected_quarantine_publication = source_publication.relocated(&quarantine);
        match resolve_publish(
            &*job.source_backend,
            &job.source_path,
            &quarantine,
            &source_snapshot,
            &expected_quarantine_publication,
            None,
            control,
            &mut progress_callback,
        )
        .unwrap_or(PublishState::Ambiguous)
        {
            PublishState::Committed => {}
            PublishState::NotCommitted => {
                return Err(rollback_directory_publish(
                    job,
                    rename_error.unwrap_or_else(|| {
                        SftpOpsError::Operation(
                            "Source quarantine rename was not committed".to_string(),
                        )
                    }),
                    &published_snapshot,
                    &expected_publication,
                    displaced.as_ref(),
                    backup.as_ref(),
                    control,
                    &mut progress_callback,
                ));
            }
            PublishState::Ambiguous => {
                return Err(source_anchor_recovery_error(
                    format!(
                        "{}; source quarantine state is indeterminate",
                        rename_error
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "Source quarantine identity mismatch".to_string())
                    ),
                    job.source_backend.clone(),
                    &job.source_path,
                    &quarantine,
                    source_anchor.clone(),
                    true,
                ));
            }
        }

        let quarantined_snapshot = match capture_snapshot(&*job.source_backend, &quarantine) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(source_anchor_recovery_error(
                    format!("Capturing directory source quarantine failed: {error}"),
                    job.source_backend.clone(),
                    &job.source_path,
                    &quarantine,
                    source_anchor.clone(),
                    true,
                ));
            }
        };
        if capture_publication_snapshot_controlled(
            &*job.source_backend,
            &quarantine,
            total,
            control,
            &mut progress_callback,
        )? != expected_quarantine_publication
        {
            return Err(source_anchor_recovery_error(
                format!(
                    "Source quarantine content does not match the validated source at {}",
                    quarantine.display()
                ),
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor.clone(),
                true,
            ));
        }
        let target_after_quarantine = capture_publication_snapshot_controlled(
            &*job.target_backend,
            &job.target_path,
            total,
            control,
            &mut progress_callback,
        );
        if !target_after_quarantine
            .as_ref()
            .is_ok_and(|publication| *publication == expected_publication)
        {
            let primary = target_after_quarantine.err().unwrap_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Published target tree changed after source quarantine at {}",
                    job.target_path.display()
                ))
            });
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                primary,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = verify_anchor_at_path(
            &published_target_anchor,
            &job.target_path,
            published_snapshot.root_identity(),
        ) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = validate_snapshot(&*job.source_backend, &quarantined_snapshot) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = begin_finalizing(control, &mut progress_callback, total) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if let Err(error) = verify_anchor_at_path(
            &published_target_anchor,
            &job.target_path,
            published_snapshot.root_identity(),
        ) {
            return Err(restore_quarantine_after_validation_failure(
                job,
                &quarantine,
                &source_anchor,
                &quarantined_snapshot,
                &expected_quarantine_publication,
                error,
                &published_snapshot,
                &expected_publication,
                displaced.as_ref(),
                backup.as_ref(),
                control,
                &mut progress_callback,
            ));
        }
        if !source_anchor.matches_path(&quarantine).unwrap_or(false) {
            return Err(source_anchor_recovery_error(
                "Source quarantine ownership changed before destructive cleanup",
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
                true,
            ));
        }
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.source_backend,
            &quarantined_snapshot,
            &expected_quarantine_publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            let recovery = cleanup_failure_with_backend_recovery(
                format!(
                    "Target {} is committed but source quarantine cleanup failed: {error}",
                    job.target_path.display()
                ),
                &error,
                job.source_backend.clone(),
                quarantine.clone(),
                &quarantined_snapshot,
                &expected_quarantine_publication,
                true,
                control,
                &mut progress_callback,
            );
            return Err(retain_source_anchor_for_recovery(
                recovery,
                job.source_backend.clone(),
                &job.source_path,
                &quarantine,
                source_anchor,
            ));
        }
    }

    if displaced.is_some() || backup.is_some() {
        if let Err(error) = begin_required_cleanup(control, &mut progress_callback, total) {
            let mut paths = displaced
                .as_ref()
                .map(|snapshot| snapshot.path.clone())
                .into_iter()
                .collect::<Vec<_>>();
            paths.extend(
                backup
                    .as_ref()
                    .map(|snapshot| snapshot.path.clone())
                    .into_iter(),
            );
            return Err(recovery_error(
                format!(
                    "Directory transfer committed; cleanup was cancelled before finalizing: {error}"
                ),
                paths,
                true,
            ));
        }
    }
    if let Some(displaced) = displaced {
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.target_backend,
            &displaced.snapshot,
            &displaced.publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            return Err(cleanup_failure_with_backend_recovery(
                format!(
                    "Directory transfer committed but displaced target cleanup failed: {error}"
                ),
                &error,
                job.target_backend.clone(),
                displaced.path,
                &displaced.snapshot,
                &displaced.publication,
                true,
                control,
                &mut progress_callback,
            ));
        }
    }
    if let Some(backup) = backup {
        if let Err(error) = remove_snapshot_root_controlled(
            &*job.target_backend,
            &backup.snapshot,
            &backup.publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            return Err(cleanup_failure_with_backend_recovery(
                format!(
                    "Directory transfer committed but destination backup cleanup failed: {error}"
                ),
                &error,
                job.target_backend.clone(),
                backup.path,
                &backup.snapshot,
                &backup.publication,
                true,
                control,
                &mut progress_callback,
            ));
        }
    }
    if merge_result.had_skips && merge_result.published > 0 {
        Ok(TransferOutcome::PartiallyCompleted {
            transferred: merge_result.transferred,
            published: merge_result.published,
            skipped: merge_result.skipped,
            source_kept: job.operation == TransferOperation::Move,
        })
    } else if merge_result.had_skips {
        Ok(TransferOutcome::Skipped)
    } else {
        Ok(TransferOutcome::Completed)
    }
}

fn preflight_transfer_capabilities(
    job: &TransferJob,
    target_exists: bool,
) -> Result<(), SftpOpsError> {
    if job.operation == TransferOperation::Move {
        job.source_backend
            .preflight_safe_mutation(&job.source_path, false)
            .map_err(|error| {
                retryable_backend_recovery(error, job.source_backend.clone(), &job.source_path)
            })?;
        let source_identity = stable_identity_now(&*job.source_backend, &job.source_path)?;
        if source_identity.object_id.is_empty() {
            return Err(SftpOpsError::Operation(format!(
                "Move source has no immutable identity-bound cleanup capability: {}",
                job.source_path.display()
            )));
        }
    }
    job.target_backend
        .preflight_safe_mutation(&job.target_path, target_exists)
        .map_err(|error| {
            retryable_backend_recovery(error, job.target_backend.clone(), &job.target_path)
        })?;
    Ok(())
}

fn capture_move_source_anchor(
    job: &TransferJob,
    expected: &super::sftp_backend::StableEntryIdentity,
) -> Result<Option<Arc<dyn BackendOwnershipAnchor>>, SftpOpsError> {
    if job.operation == TransferOperation::Copy {
        return Ok(None);
    }
    let anchor = job
        .source_backend
        .existing_entry_ownership_anchor(&job.source_path)?
        .ok_or_else(|| {
            SftpOpsError::Operation(format!(
                "Move source cannot be held by an immutable ownership anchor: {}",
                job.source_path.display()
            ))
        })?;
    let anchored = anchor.identity()?;
    if !same_reserved_object(expected, &anchored) || !anchor.matches_path(&job.source_path)? {
        return Err(SftpOpsError::Operation(format!(
            "Move source changed while acquiring its ownership anchor: {}",
            job.source_path.display()
        )));
    }
    Ok(Some(anchor))
}

fn source_anchor_recovery_error(
    message: impl Into<String>,
    backend: Arc<dyn SftpBackend>,
    source_path: &Path,
    quarantine: &Path,
    anchor: Arc<dyn BackendOwnershipAnchor>,
    committed: bool,
) -> SftpOpsError {
    let mut ownership = PathOwnership::empty(source_path);
    ownership.anchored_recovery.push(AnchoredRecoveryUnit {
        anchor,
        action: AnchoredRecoveryAction::RestoreSource {
            source: source_path.to_path_buf(),
            quarantine: quarantine.to_path_buf(),
        },
    });
    ownership_recovery_error(message, backend, ownership, committed)
}

fn retain_source_anchor_for_recovery(
    error: SftpOpsError,
    backend: Arc<dyn SftpBackend>,
    source_path: &Path,
    quarantine: &Path,
    anchor: Arc<dyn BackendOwnershipAnchor>,
) -> SftpOpsError {
    if let Some(recovery_id) = error.recovery_id() {
        if let Some(recovery) = recovery_actions()
            .lock()
            .expect("transfer recovery registry lock poisoned")
            .get_mut(&recovery_id)
        {
            recovery.retained_anchors.push(anchor);
        }
        return error;
    }
    if error.destination_committed() && matches!(error, SftpOpsError::Committed(_)) {
        return error;
    }
    source_anchor_recovery_error(
        error.to_string(),
        backend,
        source_path,
        quarantine,
        anchor,
        true,
    )
}

fn retryable_backend_recovery(
    error: SftpOpsError,
    backend: Arc<dyn SftpBackend>,
    root: &Path,
) -> SftpOpsError {
    if error.recovery_id().is_some() || error.recovery_paths().is_empty() {
        return error;
    }
    let committed = error.destination_committed();
    let mut ownership = PathOwnership::empty(root);
    let mut transferred_identities = Vec::new();
    let recovery_paths = error.recovery_paths().to_vec();
    let mut anchored = false;
    for path in &recovery_paths {
        if backend.cleanup_recovery_identity(path).is_some() {
            if let Some(anchor) = backend.cleanup_recovery_anchor(path) {
                ownership.anchored_recovery.push(AnchoredRecoveryUnit {
                    anchor,
                    action: AnchoredRecoveryAction::CleanupOwned {
                        candidates: recovery_paths.clone(),
                    },
                });
                anchored = true;
            }
            transferred_identities.push(path.clone());
            break;
        }
    }
    if !anchored {
        ownership.unresolved.extend(recovery_paths);
    }
    let recovery =
        ownership_recovery_error(error.to_string(), backend.clone(), ownership, committed);
    if recovery.recovery_id().is_some() {
        for path in transferred_identities {
            backend.forget_cleanup_recovery_identity(&path);
        }
    }
    recovery
}

pub(crate) fn startup_backend_recovery_error(
    backend: Arc<dyn SftpBackend>,
    paths: Vec<PathBuf>,
) -> SftpOpsError {
    retryable_backend_recovery(
        SftpOpsError::RecoveryRequired {
            message: "A retained transfer artifact was found after restart".to_string(),
            recovery_id: None,
            paths,
            committed: false,
        },
        backend,
        Path::new("/"),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublishState {
    Committed,
    NotCommitted,
    Ambiguous,
}

fn same_relocated_objects(actual: &EntrySnapshot, expected: &EntrySnapshot) -> bool {
    let identities = |snapshot: &EntrySnapshot| {
        snapshot
            .entries
            .iter()
            .filter_map(|(path, identity)| {
                path.strip_prefix(&snapshot.root).ok().map(|relative| {
                    (
                        relative.to_path_buf(),
                        (
                            identity.file_type,
                            identity.size,
                            identity.object_id.clone(),
                        ),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>()
    };
    let children = |snapshot: &EntrySnapshot| {
        snapshot
            .children
            .iter()
            .filter_map(|(path, names)| {
                path.strip_prefix(&snapshot.root)
                    .ok()
                    .map(|relative| (relative.to_path_buf(), names.clone()))
            })
            .collect::<BTreeMap<_, _>>()
    };
    identities(actual) == identities(expected) && children(actual) == children(expected)
}

fn capture_publication_resolving_cancel(
    backend: &dyn SftpBackend,
    path: &Path,
    total: u64,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    cancel_requested: &mut bool,
) -> Result<EntrySnapshot, SftpOpsError> {
    match capture_publication_snapshot_controlled(backend, path, total, control, progress_callback)
    {
        Err(SftpOpsError::Cancelled) => {
            *cancel_requested = true;
            Err(SftpOpsError::Cancelled)
        }
        result => result,
    }
}

#[allow(clippy::too_many_arguments)]
fn exchange_staged_with_target(
    backend: &dyn SftpBackend,
    staged_path: &Path,
    target_path: &Path,
    staged_snapshot: &EntrySnapshot,
    staged_publication: &EntrySnapshot,
    original_target: &EntrySnapshot,
    original_publication: &EntrySnapshot,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<BackupSnapshot, SftpOpsError> {
    control.wait_until_runnable()?;
    control.begin_finalizing()?;
    let exchange_error = backend.replace(staged_path, target_path).err();
    if control.is_cancelled() {
        let restore_error = backend.replace(staged_path, target_path).err();
        return Err(recovery_error(
            format!(
                "Cancellation arrived during publish; atomic exchange reversal was attempted and both paths were retained for verification: {}",
                restore_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "exchange restoration acknowledgement pending".to_string())
            ),
            vec![staged_path.to_path_buf(), target_path.to_path_buf()],
            false,
        ));
    }
    let target = optional_snapshot_controlled(
        backend,
        target_path,
        staged_snapshot.total_file_size(),
        control,
        progress_callback,
        TransferPhase::Verifying,
    );
    let mut cancel_requested = false;
    let target_publication = capture_publication_resolving_cancel(
        backend,
        target_path,
        staged_snapshot.total_file_size(),
        control,
        progress_callback,
        &mut cancel_requested,
    );
    let displaced = optional_snapshot_controlled(
        backend,
        staged_path,
        original_target.total_file_size(),
        control,
        progress_callback,
        TransferPhase::Verifying,
    );
    let displaced_publication = capture_publication_resolving_cancel(
        backend,
        staged_path,
        original_target.total_file_size(),
        control,
        progress_callback,
        &mut cancel_requested,
    );
    let expected_target = staged_publication.relocated(target_path);
    let expected_displaced = original_publication.relocated(staged_path);

    match (target, target_publication, displaced, displaced_publication) {
        (
            Ok(Some(_)),
            Ok(target_publication),
            Ok(Some(displaced_entry)),
            Ok(displaced_publication),
        ) if target_publication == expected_target
            && displaced_publication == expected_displaced
            && same_relocated_objects(&displaced_entry, original_target) =>
        {
            if cancel_requested || control.is_cancelled() {
                let restore_error = backend.replace(staged_path, target_path).err();
                return Err(recovery_error(
                    format!(
                        "Cancellation arrived during publish; both exchange paths were retained for verification: {}",
                        restore_error
                            .map(|error| error.to_string())
                            .unwrap_or_else(
                                || "exchange restoration acknowledgement pending".to_string()
                            )
                    ),
                    vec![staged_path.to_path_buf(), target_path.to_path_buf()],
                    false,
                ));
            }
            Ok(BackupSnapshot {
                path: staged_path.to_path_buf(),
                snapshot: displaced_entry,
                publication: displaced_publication,
                ownership: None,
            })
        }
        (Ok(Some(_)), Ok(target_publication), Ok(Some(_)), Ok(displaced_publication))
            if target_publication == expected_target =>
        {
            let restore_error = backend.replace(staged_path, target_path).err();
            let restored_target = capture_publication_resolving_cancel(
                backend,
                target_path,
                original_target.total_file_size(),
                control,
                progress_callback,
                &mut cancel_requested,
            );
            let restored_stage = capture_publication_resolving_cancel(
                backend,
                staged_path,
                staged_snapshot.total_file_size(),
                control,
                progress_callback,
                &mut cancel_requested,
            );
            if restored_target.as_ref().is_ok_and(|publication| {
                *publication == displaced_publication.relocated(target_path)
            }) && restored_stage
                .as_ref()
                .is_ok_and(|publication| *publication == staged_publication.relocated(staged_path))
            {
                Err(SftpOpsError::Operation(format!(
                    "Destination changed in the final publish window at {}",
                    target_path.display()
                )))
            } else {
                Err(recovery_error(
                    format!(
                        "Destination changed during publish and safe restoration is indeterminate: {}",
                        restore_error
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "exchange identity mismatch".to_string())
                    ),
                    vec![staged_path.to_path_buf(), target_path.to_path_buf()],
                    false,
                ))
            }
        }
        (
            Ok(Some(_)),
            Ok(current_target_publication),
            Ok(Some(current_stage)),
            Ok(current_stage_publication),
        ) if current_stage_publication == staged_publication.relocated(staged_path)
            && same_relocated_objects(&current_stage, staged_snapshot)
            && current_target_publication == original_publication.relocated(target_path) =>
        {
            Err(exchange_error.unwrap_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Atomic exchange was not committed at {}",
                    target_path.display()
                ))
            }))
        }
        (Ok(_), Ok(_), Ok(_), Ok(_))
        | (Err(_), _, _, _)
        | (_, Err(_), _, _)
        | (_, _, Err(_), _)
        | (_, _, _, Err(_)) => Err(recovery_error(
            format!(
                "{}; atomic exchange state is indeterminate",
                exchange_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "exchange identity mismatch".to_string())
            ),
            vec![staged_path.to_path_buf(), target_path.to_path_buf()],
            false,
        )),
    }
}

fn resolve_publish(
    backend: &dyn SftpBackend,
    staged_path: &Path,
    target_path: &Path,
    staged_snapshot: &EntrySnapshot,
    expected_publication: &EntrySnapshot,
    original_target: Option<&EntrySnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<PublishState, SftpOpsError> {
    let total = staged_snapshot.total_file_size();
    let staged = optional_snapshot_controlled(
        backend,
        staged_path,
        total,
        control,
        progress_callback,
        TransferPhase::Verifying,
    );
    let target = optional_snapshot_controlled(
        backend,
        target_path,
        total,
        control,
        progress_callback,
        TransferPhase::Verifying,
    );
    Ok(match (staged, target) {
        (Ok(None), Ok(Some(_)))
            if capture_publication_snapshot_controlled(
                backend,
                target_path,
                total,
                control,
                progress_callback,
            )
            .is_ok_and(|target| target == *expected_publication) =>
        {
            PublishState::Committed
        }
        (Ok(Some(staged)), Ok(target)) => {
            let original_unchanged = match (target.as_ref(), original_target) {
                (None, None) => true,
                (Some(current), Some(original)) => current == original,
                (None, Some(_)) | (Some(_), None) => false,
            };
            if staged == *staged_snapshot && original_unchanged {
                PublishState::NotCommitted
            } else {
                PublishState::Ambiguous
            }
        }
        (Ok(_), Ok(_)) | (Err(_), _) | (_, Err(_)) => PublishState::Ambiguous,
    })
}

fn capture_publication_snapshot(
    backend: &dyn SftpBackend,
    root: &Path,
) -> Result<EntrySnapshot, SftpOpsError> {
    let mut snapshot = EntrySnapshot {
        root: root.to_path_buf(),
        entries: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    capture_publication_entry(backend, root, &mut snapshot)?;
    Ok(snapshot)
}

fn capture_publication_snapshot_controlled(
    backend: &dyn SftpBackend,
    root: &Path,
    total: u64,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<EntrySnapshot, SftpOpsError> {
    capture_publication_snapshot_in_phase(
        backend,
        root,
        total,
        control,
        progress_callback,
        TransferPhase::Verifying,
    )
}

fn capture_publication_snapshot_in_phase(
    backend: &dyn SftpBackend,
    root: &Path,
    total: u64,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<EntrySnapshot, SftpOpsError> {
    let mut snapshot = EntrySnapshot {
        root: root.to_path_buf(),
        entries: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    let mut verified = 0_u64;
    let started = Instant::now();
    record_phase_progress(
        control,
        progress_callback,
        verified,
        total,
        started.elapsed(),
        phase,
    );
    capture_publication_entry_controlled(
        backend,
        root,
        &mut snapshot,
        control,
        progress_callback,
        &mut verified,
        total,
        started,
        phase,
    )?;
    Ok(snapshot)
}

#[allow(clippy::too_many_arguments)]
fn capture_publication_entry_controlled(
    backend: &dyn SftpBackend,
    path: &Path,
    snapshot: &mut EntrySnapshot,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    verified: &mut u64,
    total: u64,
    started: Instant,
    phase: TransferPhase,
) -> Result<(), SftpOpsError> {
    control.wait_until_runnable()?;
    let stable = stable_identity_now(backend, path)?;
    let identity = match stable.file_type {
        FileEntryType::File => {
            let before = stable.clone();
            let mut reader = backend.open_file_reader(path)?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; STREAM_CHUNK_SIZE];
            let mut size = 0_u64;
            loop {
                control.wait_until_runnable()?;
                let read = reader.read_chunk(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
                size = size.saturating_add(read as u64);
                *verified = verified.saturating_add(read as u64);
                record_phase_progress(
                    control,
                    progress_callback,
                    *verified,
                    total,
                    started.elapsed(),
                    phase,
                );
            }
            let after = stable_identity_now(backend, path)?;
            if before != after || size != before.size {
                return Err(SftpOpsError::Operation(format!(
                    "File changed while building publication identity at {}",
                    path.display()
                )));
            }
            super::sftp_backend::StableEntryIdentity {
                file_type: before.file_type,
                size,
                object_id: String::new(),
                revision: format!("{:x}", digest.finalize()),
            }
        }
        FileEntryType::Directory => super::sftp_backend::StableEntryIdentity {
            file_type: FileEntryType::Directory,
            size: 0,
            object_id: String::new(),
            revision: "directory".to_string(),
        },
        FileEntryType::Symlink | FileEntryType::Other => {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to identify link or special file {}",
                path.display()
            )));
        }
    };
    match stable.file_type {
        FileEntryType::File => {
            snapshot.entries.insert(path.to_path_buf(), identity);
        }
        FileEntryType::Directory => {
            control.wait_until_runnable()?;
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            let after_list = stable_identity_now(backend, path)?;
            if after_list != stable {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while building publication identity at {}",
                    path.display()
                )));
            }
            let mut names = Vec::with_capacity(entries.len());
            snapshot.entries.insert(path.to_path_buf(), identity);
            for entry in entries {
                control.wait_until_runnable()?;
                validate_child_entry(path, &entry)?;
                names.push(entry.name.clone());
                capture_publication_entry_controlled(
                    backend,
                    &entry.path,
                    snapshot,
                    control,
                    progress_callback,
                    verified,
                    total,
                    started,
                    phase,
                )?;
            }
            snapshot.children.insert(path.to_path_buf(), names);
        }
        FileEntryType::Symlink | FileEntryType::Other => {
            unreachable!("links and special files return before publication traversal")
        }
    }
    Ok(())
}

fn record_phase_progress(
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    transferred: u64,
    total: u64,
    elapsed: Duration,
    phase: TransferPhase,
) {
    let mut progress = ProgressTracker::new(total).record_at(transferred, elapsed);
    progress.phase = phase;
    control.record(progress);
    if let Some(callback) = progress_callback.as_mut() {
        callback(progress);
    }
}

fn capture_publication_entry(
    backend: &dyn SftpBackend,
    path: &Path,
    snapshot: &mut EntrySnapshot,
) -> Result<(), SftpOpsError> {
    let stable = stable_identity_now(backend, path)?;
    let identity = match stable.file_type {
        FileEntryType::File => publication_identity_now(backend, path)?,
        FileEntryType::Directory => super::sftp_backend::StableEntryIdentity {
            file_type: FileEntryType::Directory,
            size: 0,
            object_id: String::new(),
            revision: "directory".to_string(),
        },
        FileEntryType::Symlink | FileEntryType::Other => {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to identify link or special file {}",
                path.display()
            )));
        }
    };
    match stable.file_type {
        FileEntryType::File => {
            snapshot.entries.insert(path.to_path_buf(), identity);
        }
        FileEntryType::Directory => {
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            let after_list = stable_identity_now(backend, path)?;
            if after_list != stable {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while building publication identity at {}",
                    path.display()
                )));
            }
            let mut names = Vec::with_capacity(entries.len());
            snapshot.entries.insert(path.to_path_buf(), identity);
            for entry in entries {
                validate_child_entry(path, &entry)?;
                names.push(entry.name.clone());
                capture_publication_entry(backend, &entry.path, snapshot)?;
            }
            snapshot.children.insert(path.to_path_buf(), names);
        }
        FileEntryType::Symlink | FileEntryType::Other => {
            unreachable!("links and special files return before publication traversal")
        }
    }
    Ok(())
}

fn publication_identity_now(
    backend: &dyn SftpBackend,
    path: &Path,
) -> Result<super::sftp_backend::StableEntryIdentity, SftpOpsError> {
    let before = stable_identity_now(backend, path)?;
    if before.file_type == FileEntryType::Directory {
        return Ok(super::sftp_backend::StableEntryIdentity {
            file_type: FileEntryType::Directory,
            size: 0,
            object_id: String::new(),
            revision: "directory".to_string(),
        });
    }
    let mut reader = backend.open_file_reader(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; STREAM_CHUNK_SIZE];
    let mut size = 0_u64;
    loop {
        let read = reader.read_chunk(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    let after = stable_identity_now(backend, path)?;
    if before != after || size != before.size {
        return Err(SftpOpsError::Operation(format!(
            "File changed while building publication identity at {}",
            path.display()
        )));
    }
    Ok(super::sftp_backend::StableEntryIdentity {
        file_type: before.file_type,
        size,
        object_id: String::new(),
        revision: format!("{:x}", digest.finalize()),
    })
}

fn stable_identity_now(
    backend: &dyn SftpBackend,
    path: &Path,
) -> Result<super::sftp_backend::StableEntryIdentity, SftpOpsError> {
    let first = backend.stable_identity(path)?;
    let second = backend.stable_identity(path)?;
    if first != second {
        return Err(SftpOpsError::Operation(format!(
            "Entry identity changed while inspecting {}",
            path.display()
        )));
    }
    Ok(second)
}

fn optional_snapshot(
    backend: &dyn SftpBackend,
    path: &Path,
) -> Result<Option<EntrySnapshot>, SftpOpsError> {
    if backend.entry_exists(path)? {
        capture_snapshot(backend, path).map(Some)
    } else {
        Ok(None)
    }
}

fn optional_snapshot_controlled(
    backend: &dyn SftpBackend,
    path: &Path,
    total: u64,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<Option<EntrySnapshot>, SftpOpsError> {
    control.wait_until_runnable()?;
    if backend.entry_exists(path)? {
        capture_snapshot_controlled(backend, path, total, control, progress_callback, phase)
            .map(Some)
    } else {
        Ok(None)
    }
}

fn validate_original_target(
    backend: &dyn SftpBackend,
    path: &Path,
    expected_snapshot: Option<&EntrySnapshot>,
    expected_publication: Option<&EntrySnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<(), SftpOpsError> {
    control.wait_until_runnable()?;
    match (expected_snapshot, expected_publication) {
        (None, None) => {
            if backend.entry_exists(path)? {
                Err(SftpOpsError::Operation(format!(
                    "Destination appeared before publish at {}",
                    path.display()
                )))
            } else {
                Ok(())
            }
        }
        (Some(snapshot), Some(publication)) => {
            validate_snapshot(backend, snapshot)?;
            if capture_publication_snapshot_controlled(
                backend,
                path,
                snapshot.total_file_size(),
                control,
                progress_callback,
            )? != *publication
            {
                return Err(SftpOpsError::Operation(format!(
                    "Destination content changed before publish at {}",
                    path.display()
                )));
            }
            Ok(())
        }
        (Some(_), None) | (None, Some(_)) => Err(SftpOpsError::Operation(
            "Incomplete original destination identity".to_string(),
        )),
    }
}

fn capture_snapshot(backend: &dyn SftpBackend, root: &Path) -> Result<EntrySnapshot, SftpOpsError> {
    let mut snapshot = EntrySnapshot {
        root: root.to_path_buf(),
        entries: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    capture_snapshot_entry(backend, root, &mut snapshot)?;
    validate_snapshot(backend, &snapshot)?;
    Ok(snapshot)
}

fn capture_snapshot_controlled(
    backend: &dyn SftpBackend,
    root: &Path,
    total: u64,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<EntrySnapshot, SftpOpsError> {
    record_phase_progress(control, progress_callback, 0, total, Duration::ZERO, phase);
    let mut snapshot = EntrySnapshot {
        root: root.to_path_buf(),
        entries: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    capture_snapshot_entry_controlled(backend, root, &mut snapshot, control)?;
    validate_snapshot_controlled(backend, &snapshot, control)?;
    Ok(snapshot)
}

fn capture_snapshot_entry_controlled(
    backend: &dyn SftpBackend,
    path: &Path,
    snapshot: &mut EntrySnapshot,
    control: &TransferControl,
) -> Result<(), SftpOpsError> {
    control.wait_until_runnable()?;
    let identity = stable_identity_now(backend, path)?;
    match identity.file_type {
        FileEntryType::File => {
            snapshot.entries.insert(path.to_path_buf(), identity);
        }
        FileEntryType::Directory => {
            control.wait_until_runnable()?;
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            if stable_identity_now(backend, path)? != identity {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while traversing {}",
                    path.display()
                )));
            }
            let mut names = Vec::with_capacity(entries.len());
            snapshot.entries.insert(path.to_path_buf(), identity);
            for entry in entries {
                control.wait_until_runnable()?;
                validate_child_entry(path, &entry)?;
                names.push(entry.name.clone());
                capture_snapshot_entry_controlled(backend, &entry.path, snapshot, control)?;
            }
            snapshot.children.insert(path.to_path_buf(), names);
        }
        FileEntryType::Symlink | FileEntryType::Other => {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to snapshot link or special file {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn capture_snapshot_entry(
    backend: &dyn SftpBackend,
    path: &Path,
    snapshot: &mut EntrySnapshot,
) -> Result<(), SftpOpsError> {
    let identity = stable_identity_now(backend, path)?;
    match identity.file_type {
        FileEntryType::File => {
            snapshot.entries.insert(path.to_path_buf(), identity);
        }
        FileEntryType::Directory => {
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            let after_list = stable_identity_now(backend, path)?;
            if after_list != identity {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while traversing {}",
                    path.display()
                )));
            }
            let mut names = Vec::with_capacity(entries.len());
            snapshot.entries.insert(path.to_path_buf(), identity);
            for entry in entries {
                validate_child_entry(path, &entry)?;
                names.push(entry.name.clone());
                capture_snapshot_entry(backend, &entry.path, snapshot)?;
            }
            snapshot.children.insert(path.to_path_buf(), names);
        }
        FileEntryType::Symlink | FileEntryType::Other => {
            return Err(SftpOpsError::Operation(format!(
                "Refusing to snapshot link or special file {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_child_entry(
    parent: &Path,
    entry: &super::types::FileEntry,
) -> Result<(), SftpOpsError> {
    let mut components = Path::new(&entry.name).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
        || entry.path != parent.join(&entry.name)
    {
        return Err(SftpOpsError::Operation(format!(
            "Refusing unsafe directory entry {}",
            entry.path.display()
        )));
    }
    Ok(())
}

fn validate_snapshot(
    backend: &dyn SftpBackend,
    snapshot: &EntrySnapshot,
) -> Result<(), SftpOpsError> {
    for (path, expected) in &snapshot.entries {
        let actual = stable_identity_now(backend, path)?;
        if &actual != expected {
            return Err(SftpOpsError::Operation(format!(
                "Entry identity changed at {}",
                path.display()
            )));
        }
        if expected.file_type == FileEntryType::Directory {
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            let names = entries
                .iter()
                .map(|entry| {
                    validate_child_entry(path, entry)?;
                    Ok(entry.name.clone())
                })
                .collect::<Result<Vec<_>, SftpOpsError>>()?;
            if snapshot.children.get(path) != Some(&names) {
                return Err(SftpOpsError::Operation(format!(
                    "Directory membership changed at {}",
                    path.display()
                )));
            }
            let after_list = stable_identity_now(backend, path)?;
            if &after_list != expected {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while validating {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn validate_snapshot_controlled(
    backend: &dyn SftpBackend,
    snapshot: &EntrySnapshot,
    control: &TransferControl,
) -> Result<(), SftpOpsError> {
    for (path, expected) in &snapshot.entries {
        control.wait_until_runnable()?;
        let actual = stable_identity_now(backend, path)?;
        if &actual != expected {
            return Err(SftpOpsError::Operation(format!(
                "Entry identity changed at {}",
                path.display()
            )));
        }
        if expected.file_type == FileEntryType::Directory {
            control.wait_until_runnable()?;
            let mut entries = backend.list_dir(path)?;
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            let names = entries
                .iter()
                .map(|entry| {
                    validate_child_entry(path, entry)?;
                    Ok(entry.name.clone())
                })
                .collect::<Result<Vec<_>, SftpOpsError>>()?;
            if snapshot.children.get(path) != Some(&names)
                || &stable_identity_now(backend, path)? != expected
            {
                return Err(SftpOpsError::Operation(format!(
                    "Directory changed while validating {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn publication_entries(
    snapshot: &EntrySnapshot,
) -> BTreeMap<PathBuf, super::sftp_backend::StableEntryIdentity> {
    snapshot
        .entries
        .iter()
        .filter_map(|(path, identity)| {
            path.strip_prefix(&snapshot.root)
                .ok()
                .map(|relative| (relative.to_path_buf(), identity.clone()))
        })
        .collect()
}

fn merged_publication_entries(
    source: &EntrySnapshot,
    target: &EntrySnapshot,
) -> (
    BTreeMap<PathBuf, super::sftp_backend::StableEntryIdentity>,
    MergeResult,
) {
    let source_entries = publication_entries(source);
    let mut merged = publication_entries(target);
    let mut result = MergeResult::default();
    for (relative, identity) in source_entries {
        if relative.as_os_str().is_empty() {
            continue;
        }
        let blocked_by_parent = relative.ancestors().skip(1).any(|parent| {
            merged
                .get(parent)
                .is_some_and(|entry| entry.file_type != FileEntryType::Directory)
        });
        if blocked_by_parent {
            result.had_skips = true;
            if identity.file_type == FileEntryType::File {
                result.skipped += 1;
            }
            continue;
        }
        match merged.get(&relative) {
            Some(existing)
                if existing.file_type == FileEntryType::Directory
                    && identity.file_type == FileEntryType::Directory => {}
            Some(_) => {
                result.had_skips = true;
                if identity.file_type == FileEntryType::File {
                    result.skipped += 1;
                }
            }
            None => {
                if identity.file_type == FileEntryType::File {
                    result.transferred += 1;
                }
                result.published += 1;
                merged.insert(relative, identity);
            }
        }
    }
    (merged, result)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MergeResult {
    transferred: usize,
    published: usize,
    skipped: usize,
    had_skips: bool,
}

#[allow(clippy::too_many_arguments)]
fn merge_snapshot_into_existing_root(
    source_backend: &dyn SftpBackend,
    source: &EntrySnapshot,
    existing_target: &EntrySnapshot,
    target_backend: &dyn SftpBackend,
    target_root: &Path,
    control: &TransferControl,
    transferred: &mut u64,
    total: u64,
    started: Instant,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    mut ownership: PathOwnership,
) -> Result<(bool, PathOwnership), OwnedPathError> {
    let existing = existing_target
        .entries
        .iter()
        .filter_map(|(path, identity)| {
            path.strip_prefix(&existing_target.root)
                .ok()
                .map(|relative| (relative.to_path_buf(), identity.file_type))
        })
        .collect::<BTreeMap<_, _>>();
    let is_blocked = |relative: &Path| {
        relative.ancestors().skip(1).any(|parent| {
            existing
                .get(parent)
                .is_some_and(|file_type| *file_type != FileEntryType::Directory)
        })
    };
    let mut skipped = false;
    let mut directories = source
        .entries
        .iter()
        .filter(|(path, identity)| {
            **path != source.root && identity.file_type == FileEntryType::Directory
        })
        .collect::<Vec<_>>();
    directories.sort_by_key(|(path, _)| path.components().count());
    for (source_path, _) in directories {
        control
            .wait_until_runnable()
            .map_err(|error| OwnedPathError {
                error,
                ownership: ownership.clone(),
            })?;
        let relative = source_path
            .strip_prefix(&source.root)
            .expect("snapshot entry is below root");
        if is_blocked(relative) {
            skipped = true;
            continue;
        }
        match existing.get(relative) {
            Some(FileEntryType::Directory) => {}
            Some(FileEntryType::File | FileEntryType::Symlink | FileEntryType::Other) => {
                skipped = true;
            }
            None => {
                ownership =
                    create_owned_directory(target_backend, &target_root.join(relative), ownership)?;
            }
        }
    }
    for (source_path, identity) in &source.entries {
        if identity.file_type != FileEntryType::File {
            continue;
        }
        let relative = source_path
            .strip_prefix(&source.root)
            .expect("snapshot entry is below root");
        if is_blocked(relative) || existing.contains_key(relative) {
            skipped = true;
            continue;
        }
        let (written, next_ownership) = stream_file_to_new_path(
            source_backend,
            source_path,
            identity,
            target_backend,
            &target_root.join(relative),
            control,
            progress_callback,
            *transferred,
            total,
            started,
            ownership,
        )?;
        ownership = next_ownership;
        *transferred = transferred.saturating_add(written);
    }
    Ok((skipped, ownership))
}

fn create_path_failure(
    backend: &dyn SftpBackend,
    path: &Path,
    error: SftpOpsError,
    mut ownership: PathOwnership,
) -> OwnedPathError {
    let recovery_paths = error.recovery_paths().to_vec();
    let mut anchored = false;
    for recovery_path in &recovery_paths {
        if let Some(anchor) = backend.cleanup_recovery_anchor(recovery_path) {
            ownership.anchored_recovery.push(AnchoredRecoveryUnit {
                anchor,
                action: AnchoredRecoveryAction::CleanupOwned {
                    candidates: recovery_paths.clone(),
                },
            });
            backend.forget_cleanup_recovery_identity(recovery_path);
            anchored = true;
            break;
        }
    }
    if !anchored {
        ownership.unresolved.extend(recovery_paths);
    }
    match backend.entry_exists(path) {
        Ok(false) => {}
        Ok(true) | Err(_) => {
            ownership.unresolved.insert(path.to_path_buf());
        }
    }
    OwnedPathError { error, ownership }
}

fn capture_reserved_anchor(
    path: &Path,
    anchor: Option<Arc<dyn BackendOwnershipAnchor>>,
    mut ownership: PathOwnership,
) -> Result<PathOwnership, OwnedPathError> {
    match anchor {
        Some(anchor) => match anchor.identity() {
            Ok(identity)
                if !identity.object_id.is_empty() && anchor.matches_path(path).unwrap_or(false) =>
            {
                ownership.owned.insert(
                    path.to_path_buf(),
                    OwnedEntryIdentity::new(identity, anchor),
                );
                Ok(ownership)
            }
            Ok(_) => {
                ownership.unresolved.insert(path.to_path_buf());
                Ok(ownership)
            }
            Err(error) => {
                ownership.unresolved.insert(path.to_path_buf());
                Err(OwnedPathError { error, ownership })
            }
        },
        None => {
            ownership.unresolved.insert(path.to_path_buf());
            Ok(ownership)
        }
    }
}

fn create_owned_directory(
    backend: &dyn SftpBackend,
    path: &Path,
    ownership: PathOwnership,
) -> Result<PathOwnership, OwnedPathError> {
    let anchor = backend
        .create_dir_with_ownership_anchor(path)
        .map_err(|error| create_path_failure(backend, path, error, ownership.clone()))?;
    let mut ownership = capture_reserved_anchor(path, anchor, ownership)?;
    refresh_owned_ancestors(backend, path, &mut ownership).map_err(|error| OwnedPathError {
        error,
        ownership: ownership.clone(),
    })?;
    Ok(ownership)
}

fn create_owned_writer(
    backend: &dyn SftpBackend,
    path: &Path,
    ownership: PathOwnership,
) -> Result<
    (
        Box<dyn super::sftp_backend::BackendFileWriter>,
        PathOwnership,
    ),
    OwnedPathError,
> {
    let mut writer = backend
        .create_file_writer(path)
        .map_err(|error| create_path_failure(backend, path, error, ownership.clone()))?;
    let mut ownership = ownership;
    match writer.ownership_anchor() {
        Ok(anchor) => {
            ownership = capture_reserved_anchor(path, anchor, ownership)?;
            refresh_owned_ancestors(backend, path, &mut ownership).map_err(|error| {
                OwnedPathError {
                    error,
                    ownership: ownership.clone(),
                }
            })?;
        }
        Err(error) => {
            ownership.unresolved.insert(path.to_path_buf());
            return Err(OwnedPathError { error, ownership });
        }
    }
    Ok((writer, ownership))
}

fn bind_snapshot_to_reserved_ownership(
    ownership: &mut PathOwnership,
    snapshot: &EntrySnapshot,
) -> Result<(), SftpOpsError> {
    let mut changed = false;
    let owned_paths = ownership.owned.keys().cloned().collect::<Vec<_>>();
    for path in owned_paths {
        let matches = ownership
            .owned
            .get(&path)
            .zip(snapshot.entries.get(&path))
            .is_some_and(|(reserved, actual)| {
                same_reserved_object(&reserved.reserved, actual)
                    && reserved.anchor.matches_path(&path).unwrap_or(false)
            });
        if matches {
            if let (Some(entry), Some(actual)) =
                (ownership.owned.get_mut(&path), snapshot.entries.get(&path))
            {
                entry.guard = actual.clone();
            }
        } else {
            ownership.owned.remove(&path);
            ownership.unresolved.insert(path);
            changed = true;
        }
    }
    for path in snapshot.entries.keys() {
        if !ownership.owned.contains_key(path) && !ownership.unresolved.contains(path) {
            ownership.unresolved.insert(path.clone());
            changed = true;
        }
    }
    if changed {
        Err(SftpOpsError::Operation(format!(
            "Reserved transfer ownership changed before snapshot verification at {}",
            snapshot.root.display()
        )))
    } else {
        Ok(())
    }
}

fn same_reserved_object(
    expected: &super::sftp_backend::StableEntryIdentity,
    actual: &super::sftp_backend::StableEntryIdentity,
) -> bool {
    expected.file_type == actual.file_type
        && !expected.object_id.is_empty()
        && expected.object_id == actual.object_id
}

fn owned_root_anchor(
    ownership: &PathOwnership,
    path: &Path,
) -> Result<Arc<dyn BackendOwnershipAnchor>, SftpOpsError> {
    ownership
        .owned
        .get(path)
        .map(|entry| entry.anchor.clone())
        .ok_or_else(|| {
            SftpOpsError::Operation(format!(
                "Transfer root has no immutable ownership anchor at {}",
                path.display()
            ))
        })
}

fn verify_anchor_at_path(
    anchor: &Arc<dyn BackendOwnershipAnchor>,
    path: &Path,
    expected: &super::sftp_backend::StableEntryIdentity,
) -> Result<(), SftpOpsError> {
    let actual = anchor.identity()?;
    if anchor.matches_path(path)? && same_reserved_object(expected, &actual) {
        Ok(())
    } else {
        Err(SftpOpsError::Operation(format!(
            "Published transfer ownership changed at {}",
            path.display()
        )))
    }
}

fn refresh_owned_path(
    backend: &dyn SftpBackend,
    path: &Path,
    ownership: &mut PathOwnership,
) -> Result<(), SftpOpsError> {
    let Some(expected) = ownership.owned.get(path).cloned() else {
        return Ok(());
    };
    let actual = stable_identity_now(backend, path)?;
    if expected.anchor.matches_path(path)? && same_reserved_object(&expected.reserved, &actual) {
        if let Some(entry) = ownership.owned.get_mut(path) {
            entry.guard = actual;
        }
        Ok(())
    } else if expected.reserved.object_id.is_empty() {
        ownership.owned.remove(path);
        ownership.unresolved.insert(path.to_path_buf());
        Ok(())
    } else {
        ownership.owned.remove(path);
        ownership.unresolved.insert(path.to_path_buf());
        Err(SftpOpsError::Operation(format!(
            "Owned transfer path changed while updating its cleanup guard: {}",
            path.display()
        )))
    }
}

fn refresh_owned_ancestors(
    backend: &dyn SftpBackend,
    path: &Path,
    ownership: &mut PathOwnership,
) -> Result<(), SftpOpsError> {
    let ancestors = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| ownership.owned.contains_key(*ancestor))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    for ancestor in ancestors {
        refresh_owned_path(backend, &ancestor, ownership)?;
    }
    Ok(())
}

fn refresh_owned_mutation(
    backend: &dyn SftpBackend,
    path: &Path,
    ownership: &mut PathOwnership,
) -> Result<(), SftpOpsError> {
    refresh_owned_path(backend, path, ownership)?;
    refresh_owned_ancestors(backend, path, ownership)
}

fn copy_snapshot_to_new_root(
    source_backend: &dyn SftpBackend,
    source: &EntrySnapshot,
    target_backend: &dyn SftpBackend,
    target_root: &Path,
    control: &TransferControl,
) -> Result<PathOwnership, OwnedPathError> {
    match source.root_identity().file_type {
        FileEntryType::File => copy_file_without_progress_owned(
            source_backend,
            &source.root,
            source.root_identity(),
            target_backend,
            target_root,
            control,
        ),
        FileEntryType::Directory => {
            let mut ownership = create_owned_directory(
                target_backend,
                target_root,
                PathOwnership::empty(target_root),
            )?;
            let mut entries = source.entries.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(path, _)| path.components().count());
            for (source_path, identity) in entries {
                if source_path == &source.root {
                    continue;
                }
                let relative = source_path
                    .strip_prefix(&source.root)
                    .expect("snapshot entry is below root");
                let target_path = target_root.join(relative);
                match identity.file_type {
                    FileEntryType::Directory => {
                        ownership = create_owned_directory(target_backend, &target_path, ownership)?
                    }
                    FileEntryType::File => {
                        ownership = copy_file_without_progress(
                            source_backend,
                            source_path,
                            identity,
                            target_backend,
                            &target_path,
                            control,
                            ownership,
                        )?
                    }
                    FileEntryType::Symlink | FileEntryType::Other => {
                        return Err(OwnedPathError {
                            error: SftpOpsError::Operation(format!(
                                "Refusing to copy unsupported snapshot {}",
                                source_path.display()
                            )),
                            ownership,
                        });
                    }
                }
            }
            Ok(ownership)
        }
        FileEntryType::Symlink | FileEntryType::Other => Err(OwnedPathError {
            error: SftpOpsError::Operation(format!(
                "Refusing to copy unsupported snapshot {}",
                source.root.display()
            )),
            ownership: PathOwnership::empty(target_root),
        }),
    }
}

fn copy_file_without_progress_owned(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
) -> Result<PathOwnership, OwnedPathError> {
    let empty = PathOwnership::empty(target_path);
    if stable_identity_now(source_backend, source_path).map_err(|error| OwnedPathError {
        error,
        ownership: empty.clone(),
    })? != *source_identity
    {
        return Err(OwnedPathError {
            error: SftpOpsError::Operation(format!(
                "Source changed before copying {}",
                source_path.display()
            )),
            ownership: empty,
        });
    }
    let mut reader = source_backend
        .open_file_reader(source_path)
        .map_err(|error| OwnedPathError {
            error,
            ownership: PathOwnership::empty(target_path),
        })?;
    let (mut writer, mut ownership) = create_owned_writer(
        target_backend,
        target_path,
        PathOwnership::empty(target_path),
    )?;
    copy_open_file_without_progress(
        source_backend,
        source_path,
        source_identity,
        &mut *reader,
        &mut *writer,
        target_backend,
        target_path,
        control,
        &mut ownership,
    )
    .map_err(|error| OwnedPathError {
        error,
        ownership: ownership.clone(),
    })?;
    Ok(ownership)
}

fn copy_file_without_progress(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
    ownership: PathOwnership,
) -> Result<PathOwnership, OwnedPathError> {
    if stable_identity_now(source_backend, source_path).map_err(|error| OwnedPathError {
        error,
        ownership: ownership.clone(),
    })? != *source_identity
    {
        return Err(OwnedPathError {
            error: SftpOpsError::Operation(format!(
                "Source changed before copying {}",
                source_path.display()
            )),
            ownership,
        });
    }
    let mut reader = source_backend
        .open_file_reader(source_path)
        .map_err(|error| OwnedPathError {
            error,
            ownership: ownership.clone(),
        })?;
    let (mut writer, mut ownership) = create_owned_writer(target_backend, target_path, ownership)?;
    copy_open_file_without_progress(
        source_backend,
        source_path,
        source_identity,
        &mut *reader,
        &mut *writer,
        target_backend,
        target_path,
        control,
        &mut ownership,
    )
    .map_err(|error| OwnedPathError {
        error,
        ownership: ownership.clone(),
    })?;
    Ok(ownership)
}

fn copy_open_file_without_progress(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    reader: &mut dyn super::sftp_backend::BackendFileReader,
    writer: &mut dyn super::sftp_backend::BackendFileWriter,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
    ownership: &mut PathOwnership,
) -> Result<(), SftpOpsError> {
    let mut buffer = vec![0_u8; STREAM_CHUNK_SIZE];
    let mut written = 0_u64;
    loop {
        control.wait_until_runnable()?;
        let read = reader.read_chunk(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_chunk(&buffer[..read])?;
        refresh_owned_mutation(target_backend, target_path, ownership)?;
        written = written.saturating_add(read as u64);
    }
    writer.flush()?;
    refresh_owned_mutation(target_backend, target_path, ownership)?;
    if written != source_identity.size
        || stable_identity_now(source_backend, source_path)? != *source_identity
    {
        return Err(SftpOpsError::Operation(format!(
            "Source changed while copying {}",
            source_path.display()
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_snapshot_to_new_root_with_progress(
    source_backend: &dyn SftpBackend,
    source: &EntrySnapshot,
    target_backend: &dyn SftpBackend,
    target_root: &Path,
    control: &TransferControl,
    transferred: &mut u64,
    total: u64,
    started: Instant,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<PathOwnership, OwnedPathError> {
    match source.root_identity().file_type {
        FileEntryType::File => stream_file_to_new_path_owned(
            source_backend,
            &source.root,
            source.root_identity(),
            target_backend,
            target_root,
            control,
            progress_callback,
            *transferred,
            total,
            started,
        )
        .map(|(written, ownership)| {
            *transferred = transferred.saturating_add(written);
            ownership
        }),
        FileEntryType::Directory => {
            let mut ownership = create_owned_directory(
                target_backend,
                target_root,
                PathOwnership::empty(target_root),
            )?;
            let mut directories = source
                .entries
                .iter()
                .filter(|(path, identity)| {
                    **path != source.root && identity.file_type == FileEntryType::Directory
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            directories.sort_by_key(|path| path.components().count());
            for source_path in directories {
                control
                    .wait_until_runnable()
                    .map_err(|error| OwnedPathError {
                        error,
                        ownership: ownership.clone(),
                    })?;
                let relative = source_path
                    .strip_prefix(&source.root)
                    .expect("snapshot entry is below root");
                ownership =
                    create_owned_directory(target_backend, &target_root.join(relative), ownership)?;
            }
            for (source_path, identity) in &source.entries {
                if identity.file_type != FileEntryType::File {
                    continue;
                }
                let relative = source_path
                    .strip_prefix(&source.root)
                    .expect("snapshot entry is below root");
                let (written, next_ownership) = stream_file_to_new_path(
                    source_backend,
                    source_path,
                    identity,
                    target_backend,
                    &target_root.join(relative),
                    control,
                    progress_callback,
                    *transferred,
                    total,
                    started,
                    ownership,
                )?;
                ownership = next_ownership;
                *transferred = transferred.saturating_add(written);
            }
            Ok(ownership)
        }
        FileEntryType::Symlink | FileEntryType::Other => Err(OwnedPathError {
            error: SftpOpsError::Operation(format!(
                "Refusing to copy unsupported snapshot {}",
                source.root.display()
            )),
            ownership: PathOwnership::empty(target_root),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_file_to_new_path(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    transferred_before: u64,
    total: u64,
    started: Instant,
    ownership: PathOwnership,
) -> Result<(u64, PathOwnership), OwnedPathError> {
    stream_file_to_new_path_tracked(
        source_backend,
        source_path,
        source_identity,
        target_backend,
        target_path,
        control,
        progress_callback,
        transferred_before,
        total,
        started,
        ownership,
    )
}

#[allow(clippy::too_many_arguments)]
fn stream_file_to_new_path_owned(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    transferred_before: u64,
    total: u64,
    started: Instant,
) -> Result<(u64, PathOwnership), OwnedPathError> {
    stream_file_to_new_path_tracked(
        source_backend,
        source_path,
        source_identity,
        target_backend,
        target_path,
        control,
        progress_callback,
        transferred_before,
        total,
        started,
        PathOwnership::empty(target_path),
    )
}

#[allow(clippy::too_many_arguments)]
fn stream_file_to_new_path_tracked(
    source_backend: &dyn SftpBackend,
    source_path: &Path,
    source_identity: &super::sftp_backend::StableEntryIdentity,
    target_backend: &dyn SftpBackend,
    target_path: &Path,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    transferred_before: u64,
    total: u64,
    started: Instant,
    ownership: PathOwnership,
) -> Result<(u64, PathOwnership), OwnedPathError> {
    if stable_identity_now(source_backend, source_path).map_err(|error| OwnedPathError {
        error,
        ownership: ownership.clone(),
    })? != *source_identity
    {
        return Err(OwnedPathError {
            error: SftpOpsError::Operation(format!(
                "Source changed before streaming {}",
                source_path.display()
            )),
            ownership,
        });
    }
    control
        .wait_until_runnable()
        .map_err(|error| OwnedPathError {
            error,
            ownership: ownership.clone(),
        })?;
    let mut reader = source_backend
        .open_file_reader(source_path)
        .map_err(|error| OwnedPathError {
            error,
            ownership: ownership.clone(),
        })?;
    let (mut writer, mut ownership) = create_owned_writer(target_backend, target_path, ownership)?;
    let result = (|| {
        let mut written = 0_u64;
        let mut tracker = ProgressTracker::new(total);
        let mut buffer = vec![0_u8; STREAM_CHUNK_SIZE];
        loop {
            control.wait_until_runnable()?;
            let read = reader.read_chunk(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_chunk(&buffer[..read])?;
            refresh_owned_mutation(target_backend, target_path, &mut ownership)?;
            written = written.saturating_add(read as u64);
            let progress = tracker.record_at(
                transferred_before.saturating_add(written),
                started.elapsed(),
            );
            control.record(progress);
            if let Some(callback) = progress_callback.as_mut() {
                callback(progress);
            }
        }
        writer.flush()?;
        refresh_owned_mutation(target_backend, target_path, &mut ownership)?;
        drop(writer);
        if written != source_identity.size
            || stable_identity_now(source_backend, source_path)? != *source_identity
        {
            return Err(SftpOpsError::Operation(format!(
                "Source changed while streaming {}",
                source_path.display()
            )));
        }
        Ok(written)
    })();
    match result {
        Ok(written) => Ok((written, ownership)),
        Err(error) => Err(OwnedPathError { error, ownership }),
    }
}

fn begin_finalizing(
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    total: u64,
) -> Result<(), SftpOpsError> {
    let bytes_per_second = control.progress().bytes_per_second;
    control.begin_finalizing()?;
    let progress = TransferProgress {
        transferred: total,
        total,
        bytes_per_second,
        eta: None,
        phase: TransferPhase::Finalizing,
    };
    control.record(progress);
    if let Some(callback) = progress_callback.as_mut() {
        callback(progress);
    }
    Ok(())
}

fn begin_required_cleanup(
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    total: u64,
) -> Result<(), SftpOpsError> {
    let bytes_per_second = control.progress().bytes_per_second;
    control.begin_finalizing()?;
    let progress = TransferProgress {
        transferred: total,
        total,
        bytes_per_second,
        eta: None,
        phase: TransferPhase::Finalizing,
    };
    control.record(progress);
    if let Some(callback) = progress_callback.as_mut() {
        callback(progress);
    }
    Ok(())
}

fn ownership_recovery_error(
    message: impl Into<String>,
    backend: Arc<dyn SftpBackend>,
    ownership: PathOwnership,
    committed: bool,
) -> SftpOpsError {
    let message = message.into();
    let mut paths = ownership
        .owned
        .keys()
        .chain(ownership.unresolved.iter())
        .cloned()
        .collect::<Vec<_>>();
    paths.extend(
        ownership
            .anchored_recovery
            .iter()
            .flat_map(AnchoredRecoveryUnit::paths),
    );
    paths.push(ownership.root.clone());
    paths.sort();
    paths.dedup();
    let recovery_id = match next_monotonic_id(&NEXT_RECOVERY_ID, "transfer recovery ID") {
        Ok(recovery_id) => recovery_id,
        Err(error) => {
            return recovery_error(format!("{message}; {error}"), paths, committed);
        }
    };
    recovery_actions()
        .lock()
        .expect("transfer recovery registry lock poisoned")
        .insert(
            recovery_id,
            CleanupRecovery {
                units: vec![CleanupRecoveryUnit::Unresolved { backend, ownership }],
                retained_anchors: Vec::new(),
            },
        );
    SftpOpsError::RecoveryRequired {
        message,
        recovery_id: Some(recovery_id),
        paths,
        committed,
    }
}

fn cleanup_owned_manifest(
    backend: &dyn SftpBackend,
    ownership: &mut PathOwnership,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<RecoveryOutcome, SftpOpsError> {
    begin_required_cleanup(control, progress_callback, 0)?;

    let mut outcome = RecoveryOutcome::CleanupCompleted;
    let anchored = ownership.anchored_recovery.clone();
    for unit in &anchored {
        if retry_anchored_recovery(backend, unit, control, progress_callback, phase)?
            == RecoveryOutcome::SourceRestored
        {
            outcome = RecoveryOutcome::SourceRestored;
        }
    }
    ownership.anchored_recovery.clear();

    let unresolved = ownership.unresolved.iter().cloned().collect::<Vec<_>>();
    for path in unresolved {
        if let Some(replacements) = backend.retry_unresolved_recovery(&path)? {
            let source_preserved = backend.take_recovery_source_preserved(&path);
            let source_restored = backend.take_recovery_source_restored(&path);
            ownership.unresolved.remove(&path);
            let mut anchored = None;
            for replacement in &replacements {
                if let Some(anchor) = backend.cleanup_recovery_anchor(replacement) {
                    anchored = Some(AnchoredRecoveryUnit {
                        anchor,
                        action: AnchoredRecoveryAction::CleanupOwned {
                            candidates: replacements.clone(),
                        },
                    });
                    backend.forget_cleanup_recovery_identity(replacement);
                    break;
                }
            }
            if let Some(unit) = anchored {
                if retry_anchored_recovery(backend, &unit, control, progress_callback, phase)?
                    == RecoveryOutcome::SourceRestored
                {
                    outcome = RecoveryOutcome::SourceRestored;
                }
            } else {
                ownership.unresolved.extend(replacements);
            }
            if source_preserved {
                outcome = RecoveryOutcome::DestinationCommittedSourcePreserved;
            } else if source_restored
                && outcome != RecoveryOutcome::DestinationCommittedSourcePreserved
            {
                outcome = RecoveryOutcome::SourceRestored;
            }
            continue;
        }
        match backend.entry_exists(&path) {
            Ok(false) => {
                ownership.unresolved.remove(&path);
            }
            Ok(true) => {}
            Err(error) => {
                return Err(SftpOpsError::Operation(format!(
                    "Probing unresolved transfer path failed at {}: {error}",
                    path.display()
                )));
            }
        }
    }

    let mut entries = ownership
        .owned
        .iter()
        .map(|(path, identity)| (path.clone(), identity.clone()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|(path, identity)| {
        (
            identity.guard.file_type == FileEntryType::Directory,
            std::cmp::Reverse(path.components().count()),
        )
    });
    for (path, _) in entries {
        control.wait_until_runnable()?;
        match backend.entry_exists(&path) {
            Ok(false) => {
                ownership.owned.remove(&path);
                continue;
            }
            Ok(true) => {}
            Err(error) => {
                return Err(SftpOpsError::Operation(format!(
                    "Probing owned transfer path failed at {}: {error}",
                    path.display()
                )));
            }
        }
        let Some(reserved) = ownership.owned.get(&path).cloned() else {
            continue;
        };
        match reserved.anchor.matches_path(&path) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                ownership.owned.remove(&path);
                ownership.unresolved.insert(path);
                continue;
            }
        }
        let current = match stable_identity_now(backend, &path) {
            Ok(identity) if reserved.guard == identity => identity,
            Ok(_) => {
                ownership.owned.remove(&path);
                ownership.unresolved.insert(path);
                continue;
            }
            Err(error) => {
                return Err(SftpOpsError::Operation(format!(
                    "Capturing owned transfer identity failed at {}: {error}",
                    path.display()
                )));
            }
        };
        match current.file_type {
            FileEntryType::File => {
                let publication = capture_publication_snapshot_in_phase(
                    backend,
                    &path,
                    current.size,
                    control,
                    progress_callback,
                    phase,
                )?;
                let digest = publication
                    .entries
                    .get(&path)
                    .map(|entry| entry.revision.as_str())
                    .ok_or_else(|| {
                        SftpOpsError::Operation(format!(
                            "Owned cleanup manifest has no digest for {}",
                            path.display()
                        ))
                    })?;
                match backend.delete_file_if_matches(&path, &current, digest) {
                    Ok(()) => {
                        ownership.owned.remove(&path);
                        refresh_owned_ancestors(backend, &path, ownership)?;
                    }
                    Err(error) => {
                        if !error.recovery_paths().is_empty() {
                            ownership.owned.remove(&path);
                            ownership
                                .unresolved
                                .extend(error.recovery_paths().iter().cloned());
                            return Err(error);
                        }
                        match backend.entry_exists(&path) {
                            Ok(false) => {
                                ownership.owned.remove(&path);
                                refresh_owned_ancestors(backend, &path, ownership)?;
                            }
                            Ok(true) | Err(_) => return Err(error),
                        }
                    }
                }
            }
            FileEntryType::Directory => {
                if backend.list_dir(&path)?.is_empty() {
                    match backend.delete_empty_dir_if_matches(&path, &current) {
                        Ok(()) => {
                            ownership.owned.remove(&path);
                            refresh_owned_ancestors(backend, &path, ownership)?;
                        }
                        Err(error) => {
                            if !error.recovery_paths().is_empty() {
                                ownership.owned.remove(&path);
                                ownership
                                    .unresolved
                                    .extend(error.recovery_paths().iter().cloned());
                                return Err(error);
                            }
                            match backend.entry_exists(&path) {
                                Ok(false) => {
                                    ownership.owned.remove(&path);
                                    refresh_owned_ancestors(backend, &path, ownership)?;
                                }
                                Ok(true) | Err(_) => return Err(error),
                            }
                        }
                    }
                }
            }
            FileEntryType::Symlink | FileEntryType::Other => {
                ownership.owned.remove(&path);
                ownership.unresolved.insert(path);
            }
        }
    }

    if ownership.is_empty() {
        Ok(outcome)
    } else {
        Err(SftpOpsError::Operation(format!(
            "Transfer cleanup retained paths below {} (owned={}, unresolved={}, anchored={}, retained_anchors={})",
            ownership.root.display(),
            ownership.owned.len(),
            ownership.unresolved.len(),
            ownership.anchored_recovery.len(),
            ownership.retained_anchors.len()
        )))
    }
}

fn cleanup_failed_stage(
    primary: SftpOpsError,
    backend: Arc<dyn SftpBackend>,
    path: &Path,
    ownership: PathOwnership,
    committed: bool,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<TransferOutcome, SftpOpsError> {
    if ownership.is_empty() {
        return Err(primary);
    }
    let mut retained = ownership;
    match cleanup_owned_manifest(
        &*backend,
        &mut retained,
        control,
        progress_callback,
        TransferPhase::Finalizing,
    ) {
        Ok(_) => Err(primary),
        Err(cleanup_error) => Err(ownership_recovery_error(
            format!(
                "{primary}; identity-bound staged cleanup retained {}: {cleanup_error}",
                path.display()
            ),
            backend,
            retained,
            committed,
        )),
    }
}

fn create_verified_backup(
    backend: Arc<dyn SftpBackend>,
    target_snapshot: &EntrySnapshot,
    target_publication: &EntrySnapshot,
    target_path: &Path,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> Result<BackupSnapshot, SftpOpsError> {
    let path = temporary_target_path(target_path, "backup")?;
    let mut ownership =
        match copy_snapshot_to_new_root(&*backend, target_snapshot, &*backend, &path, control) {
            Ok(ownership) => ownership,
            Err(failure) => {
                return cleanup_failed_stage(
                    failure.error,
                    backend,
                    &path,
                    failure.ownership,
                    false,
                    control,
                    progress_callback,
                )
                .map(|_| unreachable!("failed backup cleanup never completes a transfer"));
            }
        };
    let snapshot = match capture_snapshot(&*backend, &path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                backend,
                &path,
                ownership,
                false,
                control,
                progress_callback,
            )
            .map(|_| unreachable!("failed backup cleanup never completes a transfer"));
        }
    };
    if let Err(error) = bind_snapshot_to_reserved_ownership(&mut ownership, &snapshot) {
        return cleanup_failed_stage(
            error,
            backend,
            &path,
            ownership,
            false,
            control,
            progress_callback,
        )
        .map(|_| unreachable!("failed backup cleanup never completes a transfer"));
    }
    let publication = match capture_publication_snapshot_controlled(
        &*backend,
        &path,
        target_snapshot.total_file_size(),
        control,
        progress_callback,
    ) {
        Ok(publication) => publication,
        Err(error) => {
            return cleanup_failed_stage(
                error,
                backend,
                &path,
                ownership.clone(),
                false,
                control,
                progress_callback,
            )
            .map(|_| unreachable!("failed backup cleanup never completes a transfer"));
        }
    };
    if publication != target_publication.relocated(&path) {
        return cleanup_failed_stage(
            SftpOpsError::Operation(format!(
                "Destination backup content verification failed for {}",
                target_path.display()
            )),
            backend,
            &path,
            ownership,
            false,
            control,
            progress_callback,
        )
        .map(|_| unreachable!("failed backup cleanup never completes a transfer"));
    }
    Ok(BackupSnapshot {
        path,
        snapshot,
        publication,
        ownership: Some(ownership),
    })
}

fn cleanup_before_publish(
    primary: SftpOpsError,
    backend: Arc<dyn SftpBackend>,
    staged_path: &Path,
    staged_snapshot: &EntrySnapshot,
    staged_ownership: &PathOwnership,
    backup: Option<&BackupSnapshot>,
    control: &TransferControl,
) -> SftpOpsError {
    if let Err(error) = validate_ownership_anchors(staged_ownership) {
        return ownership_recovery_error(
            format!("{primary}; staged ownership anchor changed before cleanup: {error}"),
            backend,
            staged_ownership.clone(),
            false,
        );
    }
    if let Some(backup_ownership) = backup.and_then(|backup| backup.ownership.as_ref()) {
        if let Err(error) = validate_ownership_anchors(backup_ownership) {
            return ownership_recovery_error(
                format!("{primary}; backup ownership anchor changed before cleanup: {error}"),
                backend,
                backup_ownership.clone(),
                false,
            );
        }
    }
    let mut progress_callback = None;
    if let Err(error) = begin_required_cleanup(
        control,
        &mut progress_callback,
        staged_snapshot.total_file_size(),
    ) {
        let mut paths = vec![staged_path.to_path_buf()];
        paths.extend(backup.map(|snapshot| snapshot.path.clone()));
        return recovery_error(
            format!("{primary}; cleanup was cancelled before finalizing: {error}"),
            paths,
            false,
        );
    }
    let staged_publication = match capture_publication_snapshot_in_phase(
        &*backend,
        staged_path,
        staged_snapshot.total_file_size(),
        control,
        &mut progress_callback,
        TransferPhase::Finalizing,
    ) {
        Ok(publication) => publication,
        Err(error) => {
            return recovery_error(
                format!("{primary}; staged cleanup identity failed: {error}"),
                vec![staged_path.to_path_buf()],
                false,
            );
        }
    };
    if let Err(error) = remove_snapshot_root_controlled(
        &*backend,
        staged_snapshot,
        &staged_publication,
        control,
        &mut progress_callback,
        TransferPhase::Finalizing,
    ) {
        return cleanup_failure_with_backend_recovery(
            format!("{primary}; staged cleanup failed: {error}"),
            &error,
            backend,
            staged_path.to_path_buf(),
            staged_snapshot,
            &staged_publication,
            false,
            control,
            &mut progress_callback,
        );
    }
    if let Some(backup) = backup {
        if let Err(error) = remove_snapshot_root_controlled(
            &*backend,
            &backup.snapshot,
            &backup.publication,
            control,
            &mut progress_callback,
            TransferPhase::Finalizing,
        ) {
            return cleanup_failure_with_backend_recovery(
                format!("{primary}; backup cleanup failed: {error}"),
                &error,
                backend,
                backup.path.clone(),
                &backup.snapshot,
                &backup.publication,
                false,
                control,
                &mut progress_callback,
            );
        }
    }
    primary
}

fn validate_ownership_anchors(ownership: &PathOwnership) -> Result<(), SftpOpsError> {
    for (path, identity) in &ownership.owned {
        if !identity.anchor.matches_path(path)? {
            return Err(SftpOpsError::Operation(format!(
                "Reserved ownership anchor no longer names {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn retry_anchored_recovery(
    backend: &dyn SftpBackend,
    unit: &AnchoredRecoveryUnit,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<RecoveryOutcome, SftpOpsError> {
    control.wait_until_runnable()?;
    match &unit.action {
        AnchoredRecoveryAction::RestoreSource { source, quarantine } => {
            let at_source = unit.anchor.matches_path(source)?;
            let at_quarantine = unit.anchor.matches_path(quarantine)?;
            if at_source {
                return Ok(RecoveryOutcome::SourceRestored);
            }
            if !at_quarantine {
                return Err(SftpOpsError::Operation(format!(
                    "Anchored move source is at neither recovery path: {}, {}",
                    source.display(),
                    quarantine.display()
                )));
            }
            if backend.entry_exists(source)? {
                return Err(SftpOpsError::Operation(format!(
                    "Source restore path is occupied by another entry: {}",
                    source.display()
                )));
            }
            begin_required_cleanup(control, progress_callback, 0)?;
            let rename_error = backend.rename(quarantine, source).err();
            let restored = unit.anchor.matches_path(source)?;
            let retained = unit.anchor.matches_path(quarantine)?;
            if restored && !retained {
                return Ok(RecoveryOutcome::SourceRestored);
            }
            Err(rename_error.unwrap_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Anchored source restore is indeterminate between {} and {}",
                    source.display(),
                    quarantine.display()
                ))
            }))
        }
        AnchoredRecoveryAction::CleanupOwned { candidates } => {
            let mut anchored_paths = Vec::new();
            let mut existing_paths = Vec::new();
            for candidate in candidates {
                if unit.anchor.matches_path(candidate)? {
                    anchored_paths.push(candidate.clone());
                }
                if backend.entry_exists(candidate)? {
                    existing_paths.push(candidate.clone());
                }
            }
            let path = match anchored_paths.as_slice() {
                [path] => path.clone(),
                [] if existing_paths.is_empty() => {
                    for candidate in candidates {
                        backend.release_cleanup_recovery_path(candidate)?;
                    }
                    return Ok(RecoveryOutcome::CleanupCompleted);
                }
                [] => {
                    return Err(SftpOpsError::Operation(format!(
                        "Owned recovery object is no longer at any candidate path: {}",
                        candidates
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
                _ => {
                    return Err(SftpOpsError::Operation(
                        "Owned recovery object resolves to multiple candidate paths".to_string(),
                    ));
                }
            };
            let anchored_identity = unit.anchor.identity()?;
            let current = stable_identity_now(backend, &path)?;
            if !same_reserved_object(&anchored_identity, &current) {
                return Err(SftpOpsError::Operation(format!(
                    "Owned recovery identity changed at {}",
                    path.display()
                )));
            }
            if current.file_type == FileEntryType::File && unit.anchor.link_count()? != Some(1) {
                return Err(SftpOpsError::Operation(format!(
                    "Owned recovery file gained another hardlink before cleanup at {}",
                    path.display()
                )));
            }
            let delete_result = match current.file_type {
                FileEntryType::File => {
                    let publication = capture_publication_snapshot_in_phase(
                        backend,
                        &path,
                        current.size,
                        control,
                        progress_callback,
                        phase,
                    )?;
                    let digest = publication
                        .entries
                        .get(&path)
                        .map(|identity| identity.revision.as_str())
                        .ok_or_else(|| {
                            SftpOpsError::Operation(format!(
                                "Owned recovery digest is missing at {}",
                                path.display()
                            ))
                        })?;
                    backend.delete_file_if_matches(&path, &current, digest)
                }
                FileEntryType::Directory => {
                    if !backend.list_dir(&path)?.is_empty() {
                        return Err(SftpOpsError::Operation(format!(
                            "Owned recovery directory is not empty at {}",
                            path.display()
                        )));
                    }
                    backend.delete_empty_dir_if_matches(&path, &current)
                }
                FileEntryType::Symlink | FileEntryType::Other => {
                    return Err(SftpOpsError::Operation(format!(
                        "Owned recovery path has an unsafe type at {}",
                        path.display()
                    )));
                }
            };
            if let Err(error) = delete_result {
                if unit.anchor.matches_path(&path)? || backend.entry_exists(&path)? {
                    return Err(error);
                }
            }
            for candidate in candidates {
                backend.release_cleanup_recovery_path(candidate)?;
            }
            Ok(RecoveryOutcome::CleanupCompleted)
        }
    }
}

fn remove_snapshot_root(
    backend: &dyn SftpBackend,
    snapshot: &EntrySnapshot,
    publication: &EntrySnapshot,
) -> Result<(), SftpOpsError> {
    validate_snapshot(backend, snapshot)?;
    if capture_publication_snapshot(backend, &snapshot.root)? != *publication {
        return Err(SftpOpsError::Operation(format!(
            "Cleanup content changed at {}",
            snapshot.root.display()
        )));
    }

    let mut files = snapshot
        .entries
        .iter()
        .filter(|(_, identity)| identity.file_type == FileEntryType::File)
        .collect::<Vec<_>>();
    files.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, identity) in files {
        let digest = publication
            .entries
            .get(path)
            .filter(|entry| entry.file_type == FileEntryType::File)
            .map(|entry| entry.revision.as_str())
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Cleanup manifest has no file digest for {}",
                    path.display()
                ))
            })?;
        backend.delete_file_if_matches(path, identity, digest)?;
    }

    let mut directories = snapshot
        .entries
        .iter()
        .filter(|(_, identity)| identity.file_type == FileEntryType::Directory)
        .map(|(path, identity)| (path.clone(), identity))
        .collect::<Vec<_>>();
    directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, identity) in directories {
        if !backend.list_dir(&path)?.is_empty() {
            return Err(SftpOpsError::Operation(format!(
                "Unexpected directory membership blocks cleanup at {}",
                path.display()
            )));
        }
        backend.delete_empty_dir_if_matches(&path, identity)?;
    }
    Ok(())
}

fn remove_snapshot_root_controlled(
    backend: &dyn SftpBackend,
    snapshot: &EntrySnapshot,
    publication: &EntrySnapshot,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
    phase: TransferPhase,
) -> Result<(), SftpOpsError> {
    control.wait_until_runnable()?;
    validate_snapshot(backend, snapshot)?;
    if capture_publication_snapshot_in_phase(
        backend,
        &snapshot.root,
        snapshot.total_file_size(),
        control,
        progress_callback,
        phase,
    )? != *publication
    {
        return Err(SftpOpsError::Operation(format!(
            "Cleanup content changed at {}",
            snapshot.root.display()
        )));
    }

    let mut files = snapshot
        .entries
        .iter()
        .filter(|(_, identity)| identity.file_type == FileEntryType::File)
        .collect::<Vec<_>>();
    files.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, identity) in files {
        control.wait_until_runnable()?;
        let digest = publication
            .entries
            .get(path)
            .filter(|entry| entry.file_type == FileEntryType::File)
            .map(|entry| entry.revision.as_str())
            .ok_or_else(|| {
                SftpOpsError::Operation(format!(
                    "Cleanup manifest has no file digest for {}",
                    path.display()
                ))
            })?;
        backend.delete_file_if_matches(path, identity, digest)?;
    }

    let mut directories = snapshot
        .entries
        .iter()
        .filter(|(_, identity)| identity.file_type == FileEntryType::Directory)
        .map(|(path, identity)| (path.clone(), identity))
        .collect::<Vec<_>>();
    directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, identity) in directories {
        control.wait_until_runnable()?;
        if !backend.list_dir(&path)?.is_empty() {
            return Err(SftpOpsError::Operation(format!(
                "Unexpected directory membership blocks cleanup at {}",
                path.display()
            )));
        }
        backend.delete_empty_dir_if_matches(&path, identity)?;
    }
    Ok(())
}

fn rollback_file_publish(
    job: &TransferJob,
    primary: SftpOpsError,
    published_snapshot: &EntrySnapshot,
    published_publication: &EntrySnapshot,
    displaced: Option<&BackupSnapshot>,
    backup: Option<&BackupSnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    rollback_published_entry(
        job,
        primary,
        published_snapshot,
        published_publication,
        displaced,
        backup,
        control,
        progress_callback,
    )
}

fn rollback_directory_publish(
    job: &TransferJob,
    primary: SftpOpsError,
    published_snapshot: &EntrySnapshot,
    published_publication: &EntrySnapshot,
    displaced: Option<&BackupSnapshot>,
    backup: Option<&BackupSnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    rollback_published_entry(
        job,
        primary,
        published_snapshot,
        published_publication,
        displaced,
        backup,
        control,
        progress_callback,
    )
}

fn rollback_published_entry(
    job: &TransferJob,
    primary: SftpOpsError,
    published_snapshot: &EntrySnapshot,
    published_publication: &EntrySnapshot,
    displaced: Option<&BackupSnapshot>,
    backup: Option<&BackupSnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    let total = published_snapshot.total_file_size();
    if let Some(displaced) = displaced {
        if let Err(error) = begin_required_cleanup(control, progress_callback, total) {
            let mut paths = vec![job.target_path.clone(), displaced.path.clone()];
            paths.extend(backup.map(|snapshot| snapshot.path.clone()));
            return recovery_error(
                format!(
                    "{primary}; rollback was cancelled before its atomic exchange began: {error}"
                ),
                paths,
                false,
            );
        }
        let rollback_error = job
            .target_backend
            .replace(&displaced.path, &job.target_path)
            .err();
        let restored_target = capture_publication_snapshot_in_phase(
            &*job.target_backend,
            &job.target_path,
            displaced.snapshot.total_file_size(),
            control,
            progress_callback,
            TransferPhase::Finalizing,
        );
        let displaced_after = capture_publication_snapshot_in_phase(
            &*job.target_backend,
            &displaced.path,
            total,
            control,
            progress_callback,
            TransferPhase::Finalizing,
        );
        let restored_publication = displaced.publication.relocated(&job.target_path);
        let expected_displaced = published_publication.relocated(&displaced.path);
        if restored_target
            .as_ref()
            .is_ok_and(|publication| *publication == restored_publication)
            && displaced_after
                .as_ref()
                .is_ok_and(|publication| *publication == expected_displaced)
        {
            let cleanup_snapshot = match capture_snapshot(&*job.target_backend, &displaced.path) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return recovery_error(
                        format!("{primary}; rolled-back candidate identity failed: {error}"),
                        vec![displaced.path.clone()],
                        false,
                    );
                }
            };
            if let Err(error) = remove_snapshot_root_controlled(
                &*job.target_backend,
                &cleanup_snapshot,
                &expected_displaced,
                control,
                progress_callback,
                TransferPhase::Finalizing,
            ) {
                return recovery_error(
                    format!("{primary}; rolled-back candidate cleanup failed: {error}"),
                    vec![displaced.path.clone()],
                    false,
                );
            }
            if let Some(backup) = backup {
                if let Err(error) = remove_snapshot_root_controlled(
                    &*job.target_backend,
                    &backup.snapshot,
                    &backup.publication,
                    control,
                    progress_callback,
                    TransferPhase::Finalizing,
                ) {
                    return cleanup_failure_with_backend_recovery(
                        format!("{primary}; rollback backup cleanup failed: {error}"),
                        &error,
                        job.target_backend.clone(),
                        backup.path.clone(),
                        &backup.snapshot,
                        &backup.publication,
                        false,
                        control,
                        progress_callback,
                    );
                }
            }
            return primary;
        }

        if restored_target
            .as_ref()
            .is_ok_and(|publication| *publication == restored_publication)
            && displaced_after.is_ok()
        {
            let restore_late_error = job
                .target_backend
                .replace(&displaced.path, &job.target_path)
                .err();
            let late_restored = capture_publication_snapshot_in_phase(
                &*job.target_backend,
                &job.target_path,
                total,
                control,
                progress_callback,
                TransferPhase::Finalizing,
            );
            if let (Ok(displaced_after), Ok(late_restored)) = (displaced_after, late_restored) {
                if late_restored == displaced_after.relocated(&job.target_path) {
                    let mut paths = vec![displaced.path.clone(), job.target_path.clone()];
                    if let Some(backup) = backup {
                        paths.push(backup.path.clone());
                    }
                    return recovery_error(
                        format!(
                            "{primary}; destination changed in the rollback window and was restored"
                        ),
                        paths,
                        false,
                    );
                }
            }
            return recovery_error(
                format!(
                    "{primary}; restoring the concurrent destination is indeterminate: {}",
                    restore_late_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "rollback identity mismatch".to_string())
                ),
                vec![displaced.path.clone(), job.target_path.clone()],
                false,
            );
        }

        return recovery_error(
            format!(
                "{primary}; restoring the previous destination is indeterminate: {}",
                rollback_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "rollback identity mismatch".to_string())
            ),
            vec![displaced.path.clone(), job.target_path.clone()],
            false,
        );
    }

    if let Err(error) = begin_required_cleanup(control, progress_callback, total) {
        let mut paths = vec![job.target_path.clone()];
        paths.extend(backup.map(|snapshot| snapshot.path.clone()));
        return recovery_error(
            format!("{primary}; rollback was cancelled before finalizing: {error}"),
            paths,
            false,
        );
    }
    match remove_snapshot_root_controlled(
        &*job.target_backend,
        published_snapshot,
        published_publication,
        control,
        progress_callback,
        TransferPhase::Finalizing,
    ) {
        Ok(()) => primary,
        Err(error) => match optional_snapshot_controlled(
            &*job.target_backend,
            &job.target_path,
            total,
            control,
            progress_callback,
            TransferPhase::Finalizing,
        ) {
            Ok(None) => primary,
            Ok(Some(_)) | Err(_) => recovery_error(
                format!("{primary}; removing the published destination failed: {error}"),
                vec![job.target_path.clone()],
                false,
            ),
        },
    }
}

fn restore_quarantine_after_validation_failure(
    job: &TransferJob,
    quarantine: &Path,
    source_anchor: &Arc<dyn BackendOwnershipAnchor>,
    quarantined_snapshot: &EntrySnapshot,
    quarantined_publication: &EntrySnapshot,
    primary: SftpOpsError,
    published_snapshot: &EntrySnapshot,
    published_publication: &EntrySnapshot,
    displaced: Option<&BackupSnapshot>,
    backup: Option<&BackupSnapshot>,
    control: &TransferControl,
    progress_callback: &mut Option<&mut dyn FnMut(TransferProgress)>,
) -> SftpOpsError {
    if !source_anchor.matches_path(quarantine).unwrap_or(false) {
        return source_anchor_recovery_error(
            format!("{primary}; source quarantine ownership changed before restore"),
            job.source_backend.clone(),
            &job.source_path,
            quarantine,
            source_anchor.clone(),
            true,
        );
    }
    let restore_error = job
        .source_backend
        .rename(quarantine, &job.source_path)
        .err();
    if !source_anchor
        .matches_path(&job.source_path)
        .unwrap_or(false)
    {
        return source_anchor_recovery_error(
            format!("{primary}; restored source ownership is indeterminate"),
            job.source_backend.clone(),
            &job.source_path,
            quarantine,
            source_anchor.clone(),
            true,
        );
    }
    let restored_publication = quarantined_publication.relocated(&job.source_path);
    match resolve_publish(
        &*job.source_backend,
        quarantine,
        &job.source_path,
        quarantined_snapshot,
        &restored_publication,
        None,
        control,
        progress_callback,
    )
    .unwrap_or(PublishState::Ambiguous)
    {
        PublishState::Committed => rollback_directory_publish(
            job,
            primary,
            published_snapshot,
            published_publication,
            displaced,
            backup,
            control,
            progress_callback,
        ),
        PublishState::NotCommitted | PublishState::Ambiguous => source_anchor_recovery_error(
            format!(
                "{primary}; restoring changed source quarantine is indeterminate: {}",
                restore_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "restore identity mismatch".to_string())
            ),
            job.source_backend.clone(),
            &job.source_path,
            quarantine,
            source_anchor.clone(),
            true,
        ),
    }
}

fn temporary_target_path(target: &std::path::Path, kind: &str) -> Result<PathBuf, SftpOpsError> {
    static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(1);
    let name = target.file_name().ok_or_else(|| {
        SftpOpsError::Operation(format!(
            "Transfer target has no file name: {}",
            target.display()
        ))
    })?;
    let id = next_monotonic_id(&NEXT_STAGE_ID, "transfer stage ID")?;
    Ok(target.with_file_name(format!(
        ".{}.zaplex-{kind}-{}-{id}",
        name.to_string_lossy(),
        std::process::id()
    )))
}

#[cfg(test)]
#[path = "transfer_job_tests.rs"]
mod tests;
