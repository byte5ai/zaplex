use super::*;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::tempdir;

use crate::sftp_manager::sftp_backend::{InMemorySftpBackend, SftpBackend};
use crate::sftp_manager::transfer_job::{
    run_transfer, ConflictDecision, TransferJob, TransferOperation,
};

struct CapturingRecoverySpawner {
    workers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl CapturingRecoverySpawner {
    fn new() -> Self {
        Self {
            workers: Mutex::new(Vec::new()),
        }
    }
}

impl RecoveryWorkerSpawner for CapturingRecoverySpawner {
    fn spawn(&self, _name: String, worker: Box<dyn FnOnce() + Send>) -> Result<(), std::io::Error> {
        self.workers.lock().unwrap().push(worker);
        Ok(())
    }
}

struct FailingRecoverySpawner;

impl RecoveryWorkerSpawner for FailingRecoverySpawner {
    fn spawn(
        &self,
        _name: String,
        _worker: Box<dyn FnOnce() + Send>,
    ) -> Result<(), std::io::Error> {
        Err(std::io::Error::other("injected spawn failure"))
    }
}

fn queue_with_retryable_recovery() -> (
    TransferQueue,
    TransferId,
    Arc<TransferControl>,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let job = TransferJob {
        source_backend: Arc::new(InMemorySftpBackend::new(source.path().to_path_buf())),
        target_backend: Arc::new(
            InMemorySftpBackend::new(target.path().to_path_buf())
                .with_delete_failure_matching_once("zaplex-backup"),
        ),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut queue = TransferQueue::new();
    let id = queue
        .enqueue_job(
            "workspace",
            job.source_path.clone(),
            job.target_path.clone(),
            TransferDirection::Upload,
            3,
        )
        .unwrap();
    let activity = queue.activity_handle(id).unwrap();
    let original_control = activity.control();
    let error = run_transfer(&job, &original_control, None).unwrap_err();
    activity.set_error(&error);
    (queue, id, original_control, source, target)
}

#[test]
fn global_and_workspace_activity_survive_view_changes() {
    let mut queue = TransferQueue::new();
    queue.enqueue("7", 100).unwrap();
    queue.enqueue("8", 300).unwrap();

    assert_eq!(
        queue.summary(),
        ActivitySummary {
            active: 2,
            transferred: 0,
            total: 400,
        }
    );
    assert_eq!(
        queue.workspace_summary("7"),
        ActivitySummary {
            active: 1,
            transferred: 0,
            total: 100,
        }
    );
}

#[test]
fn clearing_history_removes_only_terminal_activity() {
    let mut queue = TransferQueue::new();
    let running = queue.enqueue("workspace", 100).unwrap();
    let completed = queue.enqueue("workspace", 200).unwrap();
    queue.set_state(completed, QueuedTransferState::Completed);

    queue.clear_terminal();

    assert!(queue.activity(running).is_some());
    assert!(queue.activity(completed).is_none());
}

#[test]
fn cancelling_job_cannot_be_cleared_before_worker_reports_recovery() {
    let mut queue = TransferQueue::new();
    let id = queue.enqueue("workspace", 100).unwrap();
    let worker = queue.activity_handle(id).unwrap();

    queue.cancel_and_clear(id);

    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Cancelling
    );
    worker.set_error(&SftpOpsError::RecoveryRequired {
        message: "cleanup requires recovery".to_string(),
        recovery_id: Some(41),
        paths: vec![PathBuf::from("/retained")],
        committed: true,
    });

    let activity = queue
        .activity(id)
        .expect("the worker's recovery activity must not be orphaned");
    assert!(matches!(
        activity.state,
        QueuedTransferState::CompletedWithWarning(_)
    ));
    assert_eq!(activity.recovery_paths, vec![PathBuf::from("/retained")]);
    assert!(queue.retry_handle(id).is_some());
}

#[test]
fn cancel_and_clear_removes_job_only_after_worker_finishes() {
    let mut queue = TransferQueue::new();
    let id = queue.enqueue("workspace", 100).unwrap();
    let worker = queue.activity_handle(id).unwrap();

    queue.cancel_and_clear(id);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Cancelling
    );

    worker.set_state(QueuedTransferState::Cancelled);
    assert!(queue.activity(id).is_none());
}

#[test]
fn terminal_transition_prunes_all_queue_resources_after_active_burst() {
    let mut queue = TransferQueue::new();
    let ids = (0..150)
        .map(|_| queue.enqueue("workspace", 1).unwrap())
        .collect::<Vec<_>>();

    for id in ids {
        queue.set_state(id, QueuedTransferState::Completed);
    }

    assert_eq!(queue.activities().count(), 100);
    assert!(queue.activity(0).is_none());
    assert!(queue.control(0).is_none());
    assert!(queue.cancel_handle(0).is_none());
    assert!(queue.pause_handle(0).is_none());
}

#[test]
fn retained_cleanup_is_global_and_identity_checked_retry_is_safe() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let source_backend: Arc<dyn SftpBackend> =
        Arc::new(InMemorySftpBackend::new(source.path().to_path_buf()));
    let target_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(target.path().to_path_buf())
            .with_delete_failure_matching_once("zaplex-backup"),
    );
    let job = TransferJob {
        source_backend,
        target_backend,
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut queue = TransferQueue::new();
    let id = queue
        .enqueue_job(
            "workspace",
            job.source_path.clone(),
            job.target_path.clone(),
            TransferDirection::Upload,
            3,
        )
        .unwrap();
    let activity = queue.activity_handle(id).unwrap();

    let error = run_transfer(&job, &activity.control(), None).unwrap_err();
    activity.set_error(&error);
    queue.clear_terminal();

    let retained = queue.activity(id).unwrap();
    assert!(retained.destination_committed);
    assert!(retained.recovery_retryable);
    assert_eq!(retained.recovery_paths.len(), 1);
    assert!(queue.retry_handle(id).is_some());
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"new");

    queue.retry_recovery(id).unwrap();

    let recovered = queue.activity(id).unwrap();
    assert_eq!(recovered.state, QueuedTransferState::Completed);
    assert!(recovered.recovery_paths.is_empty());
    assert!(!recovered.recovery_retryable);
    assert!(queue.retry_handle(id).is_none());
}

#[test]
fn recovery_retry_runs_as_a_global_background_job() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    fs::write(target.path().join("target.bin"), b"old").unwrap();
    let job = TransferJob {
        source_backend: Arc::new(InMemorySftpBackend::new(source.path().to_path_buf())),
        target_backend: Arc::new(
            InMemorySftpBackend::new(target.path().to_path_buf())
                .with_delete_failure_matching_once("zaplex-backup"),
        ),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Copy,
        conflict: ConflictDecision::Overwrite,
    };
    let mut queue = TransferQueue::new();
    let id = queue
        .enqueue_job(
            "workspace",
            job.source_path.clone(),
            job.target_path.clone(),
            TransferDirection::Upload,
            3,
        )
        .unwrap();
    let activity = queue.activity_handle(id).unwrap();
    let error = run_transfer(&job, &activity.control(), None).unwrap_err();
    activity.set_error(&error);

    queue.retry_recovery_in_background(id).unwrap();
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Running
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if queue
            .activity(id)
            .is_some_and(|activity| matches!(activity.state, QueuedTransferState::Completed))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let completed = queue.activity(id).unwrap();
    assert_eq!(completed.state, QueuedTransferState::Completed);
    assert!(completed.recovery_paths.is_empty());
}

fn queue_with_source_restore_recovery() -> (
    TransferQueue,
    TransferId,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"source").unwrap();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf()).with_after_rename(
            |old_path, new_path| {
                if new_path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("zaplex-source"))
                {
                    fs::write(old_path, b"foreign").unwrap();
                }
            },
        ),
    );
    let job = TransferJob {
        source_backend,
        target_backend: Arc::new(InMemorySftpBackend::new(target.path().to_path_buf())),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };
    let mut queue = TransferQueue::new();
    let id = queue
        .enqueue_job(
            "workspace",
            job.source_path.clone(),
            job.target_path.clone(),
            TransferDirection::Upload,
            6,
        )
        .unwrap();
    let activity = queue.activity_handle(id).unwrap();
    let error = run_transfer(&job, &activity.control(), None)
        .expect_err("the occupied source path must retain a restore recovery");
    activity.set_error(&error);
    fs::remove_file(source.path().join("source.bin")).unwrap();
    (queue, id, source, target)
}

#[test]
fn review14_sync_source_restore_is_not_reported_as_completed_move() {
    let (mut queue, id, source, target) = queue_with_source_restore_recovery();

    queue.retry_recovery(id).unwrap();

    let activity = queue.activity(id).unwrap();
    assert_eq!(
        activity.state,
        QueuedTransferState::CompletedWithWarning(SOURCE_RESTORED_WARNING.to_string()),
        "a restored source means the move completed as a copy with its source kept"
    );
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn review14_background_source_restore_is_not_reported_as_completed_move() {
    let (mut queue, id, source, target) = queue_with_source_restore_recovery();
    let spawner = Arc::new(CapturingRecoverySpawner::new());
    queue.set_recovery_worker_spawner(spawner.clone());

    queue.retry_recovery_in_background(id).unwrap();
    spawner
        .workers
        .lock()
        .unwrap()
        .pop()
        .expect("the recovery worker must be captured")();

    let activity = queue.activity(id).unwrap();
    assert_eq!(
        activity.state,
        QueuedTransferState::CompletedWithWarning(SOURCE_RESTORED_WARNING.to_string()),
        "background restore must preserve the source-kept outcome"
    );
    assert_eq!(
        fs::read(source.path().join("source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(target.path().join("target.bin")).unwrap(),
        b"source"
    );
}

#[test]
fn applied_source_delete_error_remains_committed_in_global_activity() {
    let source = tempdir().unwrap();
    let target = tempdir().unwrap();
    fs::write(source.path().join("source.bin"), b"new").unwrap();
    let source_backend: Arc<dyn SftpBackend> = Arc::new(
        InMemorySftpBackend::new(source.path().to_path_buf())
            .with_delete_after_apply_failure(PathBuf::from("/source.bin")),
    );
    let job = TransferJob {
        source_backend,
        target_backend: Arc::new(InMemorySftpBackend::new(target.path().to_path_buf())),
        source_path: PathBuf::from("/source.bin"),
        target_path: PathBuf::from("/target.bin"),
        operation: TransferOperation::Move,
        conflict: ConflictDecision::Overwrite,
    };
    let mut queue = TransferQueue::new();
    let id = queue
        .enqueue_job(
            "workspace",
            job.source_path.clone(),
            job.target_path.clone(),
            TransferDirection::Upload,
            3,
        )
        .unwrap();
    let activity = queue.activity_handle(id).unwrap();

    let error = run_transfer(&job, &activity.control(), None)
        .expect_err("delete acknowledgement failure must remain visible");
    activity.set_error(&error);

    assert!(
        error.destination_committed(),
        "the transfer error must retain its committed outcome"
    );
    let retained = queue.activity(id).unwrap();
    assert!(retained.destination_committed);
    assert!(
        !matches!(retained.state, QueuedTransferState::Failed(_)),
        "a committed destination must not be presented as a failed transfer"
    );
    assert!(retained.recovery_paths.is_empty());
    assert_eq!(fs::read(target.path().join("target.bin")).unwrap(), b"new");
}

#[test]
fn recovery_retry_is_single_flight() {
    let (mut queue, id, _, _source, _target) = queue_with_retryable_recovery();
    let spawner = Arc::new(CapturingRecoverySpawner::new());
    queue.set_recovery_worker_spawner(spawner.clone());

    queue.retry_recovery_in_background(id).unwrap();
    queue
        .retry_recovery_in_background(id)
        .expect_err("a second retry must not start while the first owns the recovery action");

    assert_eq!(spawner.workers.lock().unwrap().len(), 1);
}

#[test]
fn accepted_recovery_cancel_retains_the_retryable_artifact() {
    let (mut queue, id, _, _source, _target) = queue_with_retryable_recovery();
    let spawner = Arc::new(CapturingRecoverySpawner::new());
    queue.set_recovery_worker_spawner(spawner.clone());

    queue.retry_recovery_in_background(id).unwrap();
    let target = queue.activity(id).unwrap().action_target();
    queue.cancel(target);
    let worker = spawner
        .workers
        .lock()
        .unwrap()
        .pop()
        .expect("the recovery worker must have been captured");
    worker();

    let activity = queue.activity(id).unwrap();
    assert!(matches!(
        activity.state,
        QueuedTransferState::CompletedWithWarning(_)
    ));
    assert!(activity.recovery_retryable);
    assert!(!activity.recovery_paths.is_empty());
    assert!(queue.retry_handle(id).is_some());
}

#[test]
fn recovery_spawn_failure_restores_retryable_terminal_state_and_control() {
    let (mut queue, id, original_control, _source, _target) = queue_with_retryable_recovery();
    queue.set_recovery_worker_spawner(Arc::new(FailingRecoverySpawner));

    queue
        .retry_recovery_in_background(id)
        .expect_err("the injected spawn failure must remain visible");

    let activity = queue.activity(id).unwrap();
    assert!(matches!(
        activity.state,
        QueuedTransferState::CompletedWithWarning(_)
    ));
    assert!(activity.recovery_retryable);
    assert!(queue.retry_handle(id).is_some());
    assert!(Arc::ptr_eq(
        &queue
            .control(id)
            .expect("the original control must be restored"),
        &original_control
    ));
}

#[test]
fn stale_pause_and_resume_actions_never_reactivate_terminal_history() {
    let terminal_states = [
        QueuedTransferState::Skipped,
        QueuedTransferState::Failed("failed".to_string()),
        QueuedTransferState::Completed,
        QueuedTransferState::CompletedWithWarning("warning".to_string()),
        QueuedTransferState::PartiallyCompleted {
            transferred: 2,
            published: 2,
            skipped: 1,
            source_kept: true,
        },
        QueuedTransferState::Cancelled,
    ];

    for terminal_state in terminal_states {
        let mut pause_queue = TransferQueue::new();
        let pause_id = pause_queue.enqueue("workspace", 1).unwrap();
        let pause_target = pause_queue.activity(pause_id).unwrap().action_target();
        pause_queue.set_state(pause_id, terminal_state.clone());
        pause_queue.pause(pause_target);
        assert_eq!(
            pause_queue.activity(pause_id).unwrap().state,
            terminal_state,
            "a stale pause action must not reactivate terminal history"
        );

        let mut resume_queue = TransferQueue::new();
        let resume_id = resume_queue.enqueue("workspace", 1).unwrap();
        let resume_target = resume_queue.activity(resume_id).unwrap().action_target();
        resume_queue.set_state(resume_id, terminal_state.clone());
        resume_queue.resume(resume_target);
        assert_eq!(
            resume_queue.activity(resume_id).unwrap().state,
            terminal_state,
            "a stale resume action must not reactivate terminal history"
        );
    }
}

#[test]
fn cancel_cannot_overwrite_a_worker_terminal_transition() {
    let mut queue = TransferQueue::new();
    let id = queue.enqueue("workspace", 1).unwrap();
    let worker = queue.activity_handle(id).unwrap();
    let target = queue.activity(id).unwrap().action_target();
    queue.set_cancel_after_state_check(move || {
        worker.set_state(QueuedTransferState::Failed("worker failed".to_string()));
    });

    queue.cancel(target);

    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Failed("worker failed".to_string())
    );
    queue.clear_terminal();
    assert!(queue.activity(id).is_none());
}

#[test]
fn stale_actions_from_the_previous_worker_do_not_control_recovery() {
    let (mut queue, id, _, _source, _target) = queue_with_retryable_recovery();
    let stale_target = queue.activity(id).unwrap().action_target();
    let spawner = Arc::new(CapturingRecoverySpawner::new());
    queue.set_recovery_worker_spawner(spawner);
    queue.retry_recovery_in_background(id).unwrap();
    let recovery_target = queue.activity(id).unwrap().action_target();
    assert_ne!(stale_target.control_epoch, recovery_target.control_epoch);

    queue.pause(stale_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Running,
        "a delayed pause from the completed worker must not pause its recovery"
    );
    queue.pause(recovery_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Paused
    );

    queue.resume(stale_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Paused,
        "a delayed resume from the completed worker must not resume its recovery"
    );
    queue.resume(recovery_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Running
    );

    queue.cancel(stale_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Running,
        "a delayed cancel from the completed worker must not cancel its recovery"
    );
    queue.cancel(recovery_target);
    assert_eq!(
        queue.activity(id).unwrap().state,
        QueuedTransferState::Cancelling
    );
}

#[test]
fn transfer_id_exhaustion_does_not_panic_or_poison_queue() {
    let mut queue = TransferQueue::new();
    queue
        .data
        .lock()
        .expect("transfer queue lock poisoned")
        .next_id = u64::MAX;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        queue.enqueue("workspace", 1)
    }));
    assert!(
        outcome.is_ok(),
        "transfer ID exhaustion must be returned as an error"
    );
    assert!(outcome.unwrap().is_err());
    assert!(
        queue.data.lock().is_ok(),
        "ID exhaustion must not poison the queue lock"
    );
    {
        let data = queue.data.lock().unwrap();
        assert!(data.activities.is_empty());
        assert!(data.controls.is_empty());
        assert!(data.cancel_handles.is_empty());
        assert!(data.pause_handles.is_empty());
    }
    queue.data.lock().unwrap().next_id = 7;
    assert_eq!(queue.enqueue("workspace", 1).unwrap(), 7);
}

#[test]
fn control_epoch_exhaustion_does_not_partially_allocate_transfer_id() {
    let mut queue = TransferQueue::new();
    {
        let mut data = queue.data.lock().unwrap();
        data.next_id = 9;
        data.next_control_epoch = u64::MAX;
    }

    assert!(queue.enqueue("workspace", 1).is_err());
    {
        let data = queue.data.lock().unwrap();
        assert_eq!(data.next_id, 9);
        assert!(data.activities.is_empty());
        assert!(data.controls.is_empty());
    }
    queue.data.lock().unwrap().next_control_epoch = 12;
    assert_eq!(queue.enqueue("workspace", 1).unwrap(), 9);
}

#[test]
fn recovery_attempt_exhaustion_leaves_existing_worker_generation_unchanged() {
    let (mut queue, id, _, _source, _target) = queue_with_retryable_recovery();
    let before = queue.activity(id).unwrap();
    let before_control = queue.control(id).unwrap();
    queue.set_recovery_worker_spawner(Arc::new(CapturingRecoverySpawner::new()));
    queue.data.lock().unwrap().next_recovery_attempt = u64::MAX;

    assert!(queue.retry_recovery_in_background(id).is_err());

    let after = queue.activity(id).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.control_epoch, before.control_epoch);
    assert!(Arc::ptr_eq(&queue.control(id).unwrap(), &before_control));
    assert!(!queue
        .data
        .lock()
        .unwrap()
        .recovery_attempts
        .contains_key(&id));
}
