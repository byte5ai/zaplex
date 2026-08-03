//! Process-wide file-transfer activity shared by all workspaces and panes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use warpui::elements::{ClippedScrollStateHandle, MouseStateHandle};
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::sftp_backend::{InMemorySftpBackend, SftpBackend};
use super::sftp_ops::SftpOpsError;
use super::transfer_job::{
    retry_recovery, retry_recovery_controlled, startup_backend_recovery_error, ConflictDecision,
    RecoveryOutcome, TransferControl, TransferOperation, TransferProgress,
};
use super::types::{TransferDirection, TransferPhase};

pub type TransferId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferActionTarget {
    pub id: TransferId,
    pub control_epoch: u64,
}

const MAX_TERMINAL_HISTORY: usize = 100;
const ACTIVITY_NOTIFICATION_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const SOURCE_RESTORED_WARNING: &str =
    "Move source was restored; the destination remains complete";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferTopology {
    SameFilesystem,
    LocalToRemote,
    RemoteToLocal,
    RemoteRelay,
}

trait RecoveryWorkerSpawner: Send + Sync {
    fn spawn(&self, name: String, worker: Box<dyn FnOnce() + Send>) -> Result<(), std::io::Error>;
}

struct ThreadRecoveryWorkerSpawner;

impl RecoveryWorkerSpawner for ThreadRecoveryWorkerSpawner {
    fn spawn(&self, name: String, worker: Box<dyn FnOnce() + Send>) -> Result<(), std::io::Error> {
        std::thread::Builder::new()
            .name(name)
            .spawn(worker)
            .map(|_| ())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueuedTransferState {
    Running,
    Paused,
    Cancelling,
    Completed,
    PartiallyCompleted {
        transferred: usize,
        published: usize,
        skipped: usize,
        source_kept: bool,
    },
    CompletedWithWarning(String),
    Failed(String),
    Cancelled,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferActivity {
    pub id: TransferId,
    pub control_epoch: u64,
    pub workspace_id: String,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub direction: TransferDirection,
    pub operation: TransferOperation,
    pub conflict: ConflictDecision,
    pub topology: TransferTopology,
    pub progress: TransferProgress,
    pub state: QueuedTransferState,
    pub recovery_paths: Vec<PathBuf>,
    pub recovery_retryable: bool,
    pub destination_committed: bool,
}

impl TransferActivity {
    pub fn action_target(&self) -> TransferActionTarget {
        TransferActionTarget {
            id: self.id,
            control_epoch: self.control_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivitySummary {
    pub active: usize,
    pub transferred: u64,
    pub total: u64,
    pub bytes_per_second: u64,
}

struct TransferQueueData {
    next_id: TransferId,
    next_control_epoch: u64,
    activities: HashMap<TransferId, TransferActivity>,
    controls: HashMap<TransferId, Arc<TransferControl>>,
    cancel_handles: HashMap<TransferId, MouseStateHandle>,
    pause_handles: HashMap<TransferId, MouseStateHandle>,
    retry_handles: HashMap<TransferId, MouseStateHandle>,
    recovery_ids: HashMap<TransferId, u64>,
    recovery_attempts: HashMap<TransferId, u64>,
    next_recovery_attempt: u64,
    clear_when_terminal: HashSet<TransferId>,
    workspace_close_handle: MouseStateHandle,
    workspace_scroll_handle: ClippedScrollStateHandle,
    recovery_worker_spawner: Arc<dyn RecoveryWorkerSpawner>,
    revision: u64,
}

impl Default for TransferQueueData {
    fn default() -> Self {
        Self {
            next_id: 0,
            next_control_epoch: 0,
            activities: HashMap::new(),
            controls: HashMap::new(),
            cancel_handles: HashMap::new(),
            pause_handles: HashMap::new(),
            retry_handles: HashMap::new(),
            recovery_ids: HashMap::new(),
            recovery_attempts: HashMap::new(),
            next_recovery_attempt: 0,
            clear_when_terminal: HashSet::new(),
            workspace_close_handle: MouseStateHandle::default(),
            workspace_scroll_handle: ClippedScrollStateHandle::default(),
            recovery_worker_spawner: Arc::new(ThreadRecoveryWorkerSpawner),
            revision: 0,
        }
    }
}

#[derive(Clone)]
pub struct TransferQueue {
    data: Arc<Mutex<TransferQueueData>>,
    notified_revision: u64,
    #[cfg(test)]
    cancel_after_state_check: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Default for TransferQueue {
    fn default() -> Self {
        Self {
            data: Arc::new(Mutex::new(TransferQueueData::default())),
            notified_revision: 0,
            #[cfg(test)]
            cancel_after_state_check: None,
        }
    }
}

#[derive(Clone)]
pub struct TransferActivityHandle {
    id: TransferId,
    control_epoch: u64,
    data: Arc<Mutex<TransferQueueData>>,
    control: Arc<TransferControl>,
}

impl TransferActivityHandle {
    pub fn control(&self) -> Arc<TransferControl> {
        self.control.clone()
    }

    pub fn update_progress(&self, progress: TransferProgress) {
        self.control.record(progress);
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if let Some(activity) = data
            .activities
            .get_mut(&self.id)
            .filter(|activity| activity.control_epoch == self.control_epoch)
        {
            activity.progress = progress;
            mark_changed(&mut data);
        }
    }

    pub fn set_state(&self, state: QueuedTransferState) {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if let Some(activity) = data
            .activities
            .get_mut(&self.id)
            .filter(|activity| activity.control_epoch == self.control_epoch)
        {
            activity.state = state;
            mark_changed(&mut data);
        }
        if data
            .activities
            .get(&self.id)
            .is_some_and(|activity| activity.control_epoch == self.control_epoch)
        {
            remove_requested_terminal(&mut data, self.id);
            prune_terminal_history(&mut data, MAX_TERMINAL_HISTORY);
        }
    }

    pub fn set_error(&self, error: &SftpOpsError) {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if let Some(activity) = data
            .activities
            .get_mut(&self.id)
            .filter(|activity| activity.control_epoch == self.control_epoch)
        {
            activity.state = if error.destination_committed() {
                QueuedTransferState::CompletedWithWarning(error.user_message())
            } else {
                QueuedTransferState::Failed(error.user_message())
            };
            activity.recovery_paths = error.recovery_paths().to_vec();
            activity.recovery_retryable = error.recovery_id().is_some();
            activity.destination_committed = error.destination_committed();
            mark_changed(&mut data);
        }
        let is_current = data
            .activities
            .get(&self.id)
            .is_some_and(|activity| activity.control_epoch == self.control_epoch);
        if is_current {
            if let Some(recovery_id) = error.recovery_id() {
                data.recovery_ids.insert(self.id, recovery_id);
                data.retry_handles
                    .entry(self.id)
                    .or_insert_with(MouseStateHandle::default);
            }
            remove_requested_terminal(&mut data, self.id);
            prune_terminal_history(&mut data, MAX_TERMINAL_HISTORY);
        }
    }

    pub fn snapshot(&self) -> TransferActivity {
        let mut activity = self
            .data
            .lock()
            .expect("transfer queue lock poisoned")
            .activities
            .get(&self.id)
            .expect("transfer activity removed while its handle is active")
            .clone();
        if matches!(
            activity.state,
            QueuedTransferState::Running
                | QueuedTransferState::Paused
                | QueuedTransferState::Cancelling
        ) {
            activity.progress = self.control.progress();
        }
        activity
    }
}

impl TransferQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_notifications(ctx: &mut ModelContext<Self>) -> Self {
        let mut queue = Self::new();
        queue.register_startup_backend_recoveries(Arc::new(InMemorySftpBackend::new(
            PathBuf::from("/"),
        )));
        queue.schedule_activity_notifications(ctx);
        queue
    }

    fn register_startup_backend_recoveries(&mut self, backend: Arc<dyn SftpBackend>) {
        let paths = backend.startup_recovery_paths();
        self.register_backend_recovery_paths(backend, paths);
    }

    fn register_backend_recovery_paths(
        &mut self,
        backend: Arc<dyn SftpBackend>,
        paths: Vec<PathBuf>,
    ) {
        for path in paths {
            let error = startup_backend_recovery_error(backend.clone(), vec![path.clone()]);
            let Ok(id) = self.enqueue_job("global", path.clone(), path, TransferDirection::Copy, 0)
            else {
                return;
            };
            if let Some(activity) = self.activity_handle(id) {
                activity.set_error(&error);
            }
        }
    }

    /// Scans a newly connected backend for durable recovery records without
    /// blocking the UI or tying the scan to the lifetime of the source pane.
    pub(crate) fn register_backend_recoveries_async(&self, backend: Arc<dyn SftpBackend>) {
        let mut queue = self.clone();
        let spawn = std::thread::Builder::new()
            .name("sftp-recovery-scan".to_string())
            .spawn(move || {
                let paths = backend.startup_recovery_paths();
                queue.register_backend_recovery_paths(backend, paths);
            });
        if let Err(error) = spawn {
            log::warn!("Could not start SFTP recovery scan: {error}");
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_startup_backend_for_test(
        backend: Arc<dyn SftpBackend>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut queue = Self::new();
        queue.register_startup_backend_recoveries(backend);
        queue.schedule_activity_notifications(ctx);
        queue
    }

    fn schedule_activity_notifications(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async {
                Timer::after(ACTIVITY_NOTIFICATION_INTERVAL).await;
            },
            |queue, _, ctx| {
                let revision = queue
                    .data
                    .lock()
                    .expect("transfer queue lock poisoned")
                    .revision;
                if revision != queue.notified_revision {
                    queue.notified_revision = revision;
                    ctx.notify();
                }
                queue.schedule_activity_notifications(ctx);
            },
        );
    }

    pub fn enqueue(
        &mut self,
        workspace_id: impl Into<String>,
        total: u64,
    ) -> Result<TransferId, SftpOpsError> {
        self.enqueue_job(
            workspace_id,
            PathBuf::new(),
            PathBuf::new(),
            TransferDirection::Upload,
            total,
        )
    }

    pub fn enqueue_job(
        &mut self,
        workspace_id: impl Into<String>,
        source_path: PathBuf,
        target_path: PathBuf,
        direction: TransferDirection,
        total: u64,
    ) -> Result<TransferId, SftpOpsError> {
        let topology = match direction {
            TransferDirection::Upload => TransferTopology::LocalToRemote,
            TransferDirection::Download => TransferTopology::RemoteToLocal,
            TransferDirection::Copy => TransferTopology::SameFilesystem,
        };
        self.enqueue_job_with_audit(
            workspace_id,
            source_path,
            target_path,
            direction,
            TransferOperation::Copy,
            ConflictDecision::Skip,
            topology,
            total,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_job_with_audit(
        &mut self,
        workspace_id: impl Into<String>,
        source_path: PathBuf,
        target_path: PathBuf,
        direction: TransferDirection,
        operation: TransferOperation,
        conflict: ConflictDecision,
        topology: TransferTopology,
        total: u64,
    ) -> Result<TransferId, SftpOpsError> {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        let id = data.next_id;
        let next_id = data
            .next_id
            .checked_add(1)
            .ok_or_else(|| SftpOpsError::Operation("Transfer queue ID exhausted".to_string()))?;
        let next_control_epoch = data.next_control_epoch.checked_add(1).ok_or_else(|| {
            SftpOpsError::Operation("Transfer control epoch exhausted".to_string())
        })?;
        data.next_id = next_id;
        data.next_control_epoch = next_control_epoch;
        let control_epoch = next_control_epoch;
        data.activities.insert(
            id,
            TransferActivity {
                id,
                control_epoch,
                workspace_id: workspace_id.into(),
                source_path,
                target_path,
                direction,
                operation,
                conflict,
                topology,
                progress: TransferProgress {
                    transferred: 0,
                    total,
                    bytes_per_second: 0,
                    eta: None,
                    phase: TransferPhase::Transferring,
                },
                state: QueuedTransferState::Running,
                recovery_paths: Vec::new(),
                recovery_retryable: false,
                destination_committed: false,
            },
        );
        data.controls
            .insert(id, Arc::new(TransferControl::new(total)));
        data.cancel_handles.insert(id, MouseStateHandle::default());
        data.pause_handles.insert(id, MouseStateHandle::default());
        mark_changed(&mut data);
        prune_terminal_history(&mut data, MAX_TERMINAL_HISTORY);
        Ok(id)
    }

    pub fn control(&self, id: TransferId) -> Option<Arc<TransferControl>> {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .controls
            .get(&id)
            .cloned()
    }

    pub fn activity_handle(&self, id: TransferId) -> Option<TransferActivityHandle> {
        let data = self.data.lock().expect("transfer queue lock poisoned");
        let activity = data.activities.get(&id)?;
        Some(TransferActivityHandle {
            id,
            control_epoch: activity.control_epoch,
            data: self.data.clone(),
            control: data.controls.get(&id)?.clone(),
        })
    }

    pub fn activity(&self, id: TransferId) -> Option<TransferActivity> {
        let data = self.data.lock().expect("transfer queue lock poisoned");
        let mut activity = data.activities.get(&id)?.clone();
        if matches!(
            activity.state,
            QueuedTransferState::Running
                | QueuedTransferState::Paused
                | QueuedTransferState::Cancelling
        ) {
            activity.progress = data.controls.get(&id)?.progress();
        }
        Some(activity)
    }

    pub fn cancel_handle(&self, id: TransferId) -> Option<MouseStateHandle> {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .cancel_handles
            .get(&id)
            .cloned()
    }

    pub fn pause_handle(&self, id: TransferId) -> Option<MouseStateHandle> {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .pause_handles
            .get(&id)
            .cloned()
    }

    pub fn retry_handle(&self, id: TransferId) -> Option<MouseStateHandle> {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .retry_handles
            .get(&id)
            .cloned()
    }

    pub fn workspace_panel_handles(&self) -> (MouseStateHandle, ClippedScrollStateHandle) {
        let data = self.data.lock().expect("transfer queue lock poisoned");
        (
            data.workspace_close_handle.clone(),
            data.workspace_scroll_handle.clone(),
        )
    }

    pub fn update_progress(&mut self, id: TransferId, progress: TransferProgress) {
        if let Some(handle) = self.activity_handle(id) {
            handle.update_progress(progress);
        }
    }

    pub fn set_state(&mut self, id: TransferId, state: QueuedTransferState) {
        if let Some(handle) = self.activity_handle(id) {
            handle.set_state(state);
        }
    }

    pub fn pause(&mut self, target: TransferActionTarget) {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if !data.activities.get(&target.id).is_some_and(|activity| {
            activity.control_epoch == target.control_epoch
                && matches!(activity.state, QueuedTransferState::Running)
        }) {
            return;
        }
        let Some(control) = data.controls.get(&target.id).cloned() else {
            return;
        };
        if control.pause() {
            if let Some(activity) = data.activities.get_mut(&target.id) {
                activity.state = QueuedTransferState::Paused;
                mark_changed(&mut data);
            }
        }
    }

    pub fn resume(&mut self, target: TransferActionTarget) {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if !data.activities.get(&target.id).is_some_and(|activity| {
            activity.control_epoch == target.control_epoch
                && matches!(activity.state, QueuedTransferState::Paused)
        }) {
            return;
        }
        let Some(control) = data.controls.get(&target.id).cloned() else {
            return;
        };
        if control.resume() {
            if let Some(activity) = data.activities.get_mut(&target.id) {
                activity.state = QueuedTransferState::Running;
                mark_changed(&mut data);
            }
        }
    }

    pub fn cancel(&mut self, target: TransferActionTarget) {
        #[cfg(test)]
        if let Some(hook) = &self.cancel_after_state_check {
            hook();
        }
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        if !data.activities.get(&target.id).is_some_and(|activity| {
            activity.control_epoch == target.control_epoch
                && matches!(
                    activity.state,
                    QueuedTransferState::Running | QueuedTransferState::Paused
                )
        }) {
            return;
        }
        let Some(control) = data.controls.get(&target.id).cloned() else {
            return;
        };
        if control.cancel() {
            if let Some(activity) = data.activities.get_mut(&target.id) {
                activity.state = QueuedTransferState::Cancelling;
                mark_changed(&mut data);
            }
        }
    }

    pub fn cancel_and_clear(&mut self, id: TransferId) {
        let target = {
            let mut data = self.data.lock().expect("transfer queue lock poisoned");
            data.clear_when_terminal.insert(id);
            data.activities
                .get(&id)
                .map(TransferActivity::action_target)
        };
        if let Some(target) = target {
            self.cancel(target);
        }
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        remove_requested_terminal(&mut data, id);
    }

    pub fn retry_recovery(&mut self, id: TransferId) -> Result<(), SftpOpsError> {
        let recovery_id = self
            .data
            .lock()
            .expect("transfer queue lock poisoned")
            .recovery_ids
            .get(&id)
            .copied()
            .ok_or_else(|| {
                SftpOpsError::Operation(format!("Transfer {id} has no retryable recovery action"))
            })?;
        let outcome = retry_recovery(recovery_id)?;
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        data.recovery_ids.remove(&id);
        data.retry_handles.remove(&id);
        if let Some(activity) = data.activities.get_mut(&id) {
            activity.recovery_paths.clear();
            activity.recovery_retryable = false;
            activity.state = match outcome {
                RecoveryOutcome::CleanupCompleted => QueuedTransferState::Completed,
                RecoveryOutcome::SourceRestored if activity.destination_committed => {
                    QueuedTransferState::CompletedWithWarning(SOURCE_RESTORED_WARNING.to_string())
                }
                RecoveryOutcome::SourceRestored => QueuedTransferState::Failed(
                    "Move source was restored before the destination committed".to_string(),
                ),
                RecoveryOutcome::DestinationCommittedSourcePreserved => {
                    QueuedTransferState::CompletedWithWarning(SOURCE_RESTORED_WARNING.to_string())
                }
            };
        }
        mark_changed(&mut data);
        remove_requested_terminal(&mut data, id);
        prune_terminal_history(&mut data, MAX_TERMINAL_HISTORY);
        Ok(())
    }

    pub fn retry_recovery_in_background(&mut self, id: TransferId) -> Result<(), SftpOpsError> {
        let (
            recovery_id,
            committed,
            control,
            previous_control,
            previous_control_epoch,
            previous_progress,
            attempt,
            spawner,
        ) = {
            let mut data = self.data.lock().expect("transfer queue lock poisoned");
            if data.recovery_attempts.contains_key(&id) {
                return Err(SftpOpsError::Operation(format!(
                    "Transfer {id} recovery is already running"
                )));
            }
            let recovery_id = data.recovery_ids.get(&id).copied().ok_or_else(|| {
                SftpOpsError::Operation(format!("Transfer {id} has no retryable recovery action"))
            })?;
            let activity = data
                .activities
                .get(&id)
                .ok_or_else(|| SftpOpsError::Operation(format!("Transfer {id} no longer exists")))?
                .clone();
            let next_recovery_attempt =
                data.next_recovery_attempt.checked_add(1).ok_or_else(|| {
                    SftpOpsError::Operation("Transfer recovery attempt ID exhausted".to_string())
                })?;
            let next_control_epoch = data.next_control_epoch.checked_add(1).ok_or_else(|| {
                SftpOpsError::Operation("Transfer control epoch exhausted".to_string())
            })?;
            let control = Arc::new(TransferControl::new(activity.progress.total));
            let previous_control = data.controls.get(&id).cloned();
            let previous_control_epoch = activity.control_epoch;
            data.next_recovery_attempt = next_recovery_attempt;
            let attempt = next_recovery_attempt;
            data.next_control_epoch = next_control_epoch;
            let control_epoch = next_control_epoch;
            data.recovery_attempts.insert(id, attempt);
            data.controls.insert(id, control.clone());
            if let Some(current) = data.activities.get_mut(&id) {
                current.control_epoch = control_epoch;
                current.state = QueuedTransferState::Running;
                current.progress.phase = TransferPhase::Verifying;
            }
            mark_changed(&mut data);
            (
                recovery_id,
                activity.destination_committed,
                control,
                previous_control,
                previous_control_epoch,
                activity.progress,
                attempt,
                data.recovery_worker_spawner.clone(),
            )
        };
        let data = self.data.clone();
        let spawn_result = spawner.spawn(
            format!("transfer-recovery-{id}"),
            Box::new(move || {
                let progress_data = data.clone();
                let progress_control = control.clone();
                let mut on_progress = move |progress| {
                    progress_control.record(progress);
                    let mut data = progress_data.lock().expect("transfer queue lock poisoned");
                    if data.recovery_attempts.get(&id) == Some(&attempt) {
                        if let Some(activity) = data.activities.get_mut(&id) {
                            activity.progress = progress;
                            mark_changed(&mut data);
                        }
                    }
                };
                let result =
                    retry_recovery_controlled(recovery_id, &control, Some(&mut on_progress));
                let mut data = data.lock().expect("transfer queue lock poisoned");
                if data.recovery_attempts.get(&id) != Some(&attempt) {
                    return;
                }
                data.recovery_attempts.remove(&id);
                match result {
                    Ok(outcome) => {
                        data.recovery_ids.remove(&id);
                        data.retry_handles.remove(&id);
                        if let Some(activity) = data.activities.get_mut(&id) {
                            activity.recovery_paths.clear();
                            activity.recovery_retryable = false;
                            activity.state = match outcome {
                                RecoveryOutcome::CleanupCompleted => QueuedTransferState::Completed,
                                RecoveryOutcome::SourceRestored if committed => {
                                    QueuedTransferState::CompletedWithWarning(
                                        SOURCE_RESTORED_WARNING.to_string(),
                                    )
                                }
                                RecoveryOutcome::SourceRestored => QueuedTransferState::Failed(
                                    "Move source was restored before the destination committed"
                                        .to_string(),
                                ),
                                RecoveryOutcome::DestinationCommittedSourcePreserved => {
                                    QueuedTransferState::CompletedWithWarning(
                                        SOURCE_RESTORED_WARNING.to_string(),
                                    )
                                }
                            };
                        }
                    }
                    Err(error) => {
                        if let Some(activity) = data.activities.get_mut(&id) {
                            activity.state = if committed {
                                QueuedTransferState::CompletedWithWarning(error.user_message())
                            } else {
                                QueuedTransferState::Failed(error.user_message())
                            };
                            activity.recovery_retryable = true;
                        }
                    }
                }
                mark_changed(&mut data);
                remove_requested_terminal(&mut data, id);
                prune_terminal_history(&mut data, MAX_TERMINAL_HISTORY);
            }),
        );
        if let Err(error) = spawn_result {
            let error = SftpOpsError::Operation(format!(
                "Failed to start transfer recovery worker: {error}"
            ));
            let mut data = self.data.lock().expect("transfer queue lock poisoned");
            if data.recovery_attempts.get(&id) == Some(&attempt) {
                data.recovery_attempts.remove(&id);
                match previous_control {
                    Some(control) => {
                        data.controls.insert(id, control);
                    }
                    None => {
                        data.controls.remove(&id);
                    }
                }
                if let Some(activity) = data.activities.get_mut(&id) {
                    activity.control_epoch = previous_control_epoch;
                    activity.state = if committed {
                        QueuedTransferState::CompletedWithWarning(error.user_message())
                    } else {
                        QueuedTransferState::Failed(error.user_message())
                    };
                    activity.progress = previous_progress;
                    activity.recovery_retryable = true;
                }
                mark_changed(&mut data);
            }
            return Err(error);
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_recovery_worker_spawner(&mut self, spawner: Arc<dyn RecoveryWorkerSpawner>) {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .recovery_worker_spawner = spawner;
    }

    #[cfg(test)]
    fn set_cancel_after_state_check(&mut self, hook: impl Fn() + Send + Sync + 'static) {
        self.cancel_after_state_check = Some(Arc::new(hook));
    }

    pub fn activities(&self) -> impl Iterator<Item = TransferActivity> {
        let controls = {
            let data = self.data.lock().expect("transfer queue lock poisoned");
            data.activities
                .values()
                .cloned()
                .map(|activity| {
                    let control = data.controls.get(&activity.id).cloned();
                    (activity, control)
                })
                .collect::<Vec<_>>()
        };
        let mut activities = controls
            .into_iter()
            .map(|(mut activity, control)| {
                if matches!(
                    activity.state,
                    QueuedTransferState::Running
                        | QueuedTransferState::Paused
                        | QueuedTransferState::Cancelling
                ) {
                    if let Some(control) = control {
                        activity.progress = control.progress();
                    }
                }
                activity
            })
            .collect::<Vec<_>>();
        activities.sort_by_key(|activity| activity.id);
        activities.into_iter()
    }

    pub fn summary(&self) -> ActivitySummary {
        summarize(self.activities())
    }

    pub fn workspace_summary(&self, workspace_id: &str) -> ActivitySummary {
        summarize(
            self.activities()
                .filter(|activity| activity.workspace_id == workspace_id),
        )
    }

    pub fn clear_terminal(&mut self) {
        let mut data = self.data.lock().expect("transfer queue lock poisoned");
        let terminal = removable_terminal_ids(&data);
        for id in terminal {
            remove_activity(&mut data, id);
        }
    }

    #[cfg(test)]
    pub(crate) fn exhaust_recovery_attempt_ids(&mut self) {
        self.data
            .lock()
            .expect("transfer queue lock poisoned")
            .next_recovery_attempt = u64::MAX;
    }
}

fn removable_terminal_ids(data: &TransferQueueData) -> Vec<TransferId> {
    data.activities
        .values()
        .filter(|activity| {
            !matches!(
                activity.state,
                QueuedTransferState::Running
                    | QueuedTransferState::Paused
                    | QueuedTransferState::Cancelling
            ) && activity.recovery_paths.is_empty()
        })
        .map(|activity| activity.id)
        .collect()
}

fn prune_terminal_history(data: &mut TransferQueueData, maximum: usize) {
    let mut terminal = removable_terminal_ids(data);
    terminal.sort_unstable();
    let remove_count = terminal.len().saturating_sub(maximum);
    for id in terminal.into_iter().take(remove_count) {
        remove_activity(data, id);
    }
}

fn remove_activity(data: &mut TransferQueueData, id: TransferId) {
    let removed = data.activities.remove(&id).is_some();
    data.controls.remove(&id);
    data.cancel_handles.remove(&id);
    data.pause_handles.remove(&id);
    data.retry_handles.remove(&id);
    data.recovery_ids.remove(&id);
    data.recovery_attempts.remove(&id);
    data.clear_when_terminal.remove(&id);
    if removed {
        mark_changed(data);
    }
}

fn mark_changed(data: &mut TransferQueueData) {
    data.revision = data.revision.wrapping_add(1);
}

fn remove_requested_terminal(data: &mut TransferQueueData, id: TransferId) {
    let removable = data.clear_when_terminal.contains(&id)
        && data.activities.get(&id).is_some_and(|activity| {
            !matches!(
                activity.state,
                QueuedTransferState::Running
                    | QueuedTransferState::Paused
                    | QueuedTransferState::Cancelling
            ) && activity.recovery_paths.is_empty()
        });
    if removable {
        remove_activity(data, id);
    }
}

fn summarize(activities: impl Iterator<Item = TransferActivity>) -> ActivitySummary {
    activities
        .filter(|activity| {
            matches!(
                activity.state,
                QueuedTransferState::Running
                    | QueuedTransferState::Paused
                    | QueuedTransferState::Cancelling
            )
        })
        .fold(ActivitySummary::default(), |mut summary, activity| {
            summary.active += 1;
            summary.transferred = summary
                .transferred
                .saturating_add(activity.progress.transferred);
            summary.total = summary.total.saturating_add(activity.progress.total);
            summary.bytes_per_second = summary
                .bytes_per_second
                .saturating_add(activity.progress.bytes_per_second);
            summary
        })
}

impl Entity for TransferQueue {
    type Event = ();
}

impl SingletonEntity for TransferQueue {}

#[cfg(test)]
#[path = "transfer_queue_tests.rs"]
mod tests;
