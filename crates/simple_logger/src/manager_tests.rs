use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
};

use warpui::r#async::executor::Background;

use super::LogManager;

fn temp_path(name: &str) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "simple-logger-tests-{name}-{}-{id}",
        std::process::id()
    ))
}
fn cleanup_log_path(log_path: &Path) {
    let _ = std::fs::remove_file(log_path);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

#[test]
fn register_resolved_path_reuses_stale_entries_after_drop() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("re-register").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");

    drop(logger);

    let logger = manager
        .register_resolved_path(log_path.clone(), executor)
        .expect("stale entry should be reclaimed after the logger is dropped");
    logger.close();
    futures::executor::block_on(logger.wait_closed());
    cleanup_log_path(&log_path);
}

#[test]
fn register_resolved_path_rejects_duplicate_active_loggers() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("collision").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");
    assert!(
        manager
            .register_resolved_path(log_path.clone(), executor)
            .is_err(),
        "live logger should block duplicate registration"
    );

    logger.close();
    futures::executor::block_on(logger.wait_closed());
    cleanup_log_path(&log_path);
}

#[test]
fn register_reclaims_closed_logger() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("close-reclaim").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");

    // Close the channel without dropping the logger — the Arc<LogFileWriter> is still alive.
    logger.close();

    let new_logger = manager
        .register_resolved_path(log_path.clone(), executor)
        .expect("closed logger should be reclaimed even when Arc is still alive");

    new_logger.close();
    futures::executor::block_on(async {
        futures::join!(logger.wait_closed(), new_logger.wait_closed());
    });
    cleanup_log_path(&log_path);
}

#[test]
fn closed_logger_is_drained_before_path_reuse() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("drain-before-reuse").join("server.log");
    let received = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let received_by_worker = received.clone();
    let release_worker = release.clone();

    let old_logger = manager
        .register_resolved_path_with_hook(
            log_path.clone(),
            executor.clone(),
            Some(Arc::new(move |line| {
                if line == "old-final" {
                    received_by_worker.wait();
                    release_worker.wait();
                }
            })),
        )
        .expect("initial registration should succeed");
    old_logger.log("old-final".to_string());
    received.wait();
    old_logger.close();

    let new_logger = manager
        .register_resolved_path(log_path.clone(), executor)
        .expect("a closed generation should be replaced transparently");
    new_logger.log("new-first".to_string());
    new_logger.close();
    release.wait();

    futures::executor::block_on(async {
        futures::join!(old_logger.wait_closed(), new_logger.wait_closed());
    });

    let contents = std::fs::read(&log_path).unwrap();
    assert!(
        !contents.contains(&0),
        "the new generation must not contain NUL gaps"
    );
    let contents = String::from_utf8(contents).unwrap();
    assert!(contents.contains("new-first"));
    assert!(!contents.contains("old-final"));
    cleanup_log_path(&log_path);
}
