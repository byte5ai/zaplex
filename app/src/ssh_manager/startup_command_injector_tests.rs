use std::sync::Arc;
use std::time::Duration;

use futures_lite::future;

use super::{wait_for_startup_command, StartupCommandWaitOutcome};

#[test]
fn startup_command_runs_after_percent_prompt() {
    futures_lite::future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(4);
        let (started, start) = tokio::sync::oneshot::channel();
        started.send(rx).unwrap();
        let wait =
            wait_for_startup_command(start, "printf ready".to_string(), Duration::from_secs(1));
        let send = async move {
            tx.broadcast(Arc::new(b"user@host:~% ".to_vec()))
                .await
                .unwrap();
        };

        let (outcome, ()) = future::zip(wait, send).await;

        assert_eq!(
            outcome,
            StartupCommandWaitOutcome::Ready("printf ready".to_string())
        );
    });
}

#[test]
fn startup_ignores_local_prompt_and_waits_for_the_launched_ssh_prompt() {
    future::block_on(async {
        let (tx, keeper) = async_broadcast::broadcast(4);
        let inactive = keeper.clone().deactivate();
        tx.try_broadcast(Arc::new(b"local@host:~$ ".to_vec()))
            .unwrap();
        let (started, start) = tokio::sync::oneshot::channel();
        let mut wait = Box::pin(wait_for_startup_command(
            start,
            "printf remote-only".to_string(),
            Duration::from_secs(1),
        ));
        assert!(
            future::poll_once(&mut wait).await.is_none(),
            "no launch yet"
        );
        // The real event observer activates at the expected ExecuteCommand event.
        started.send(inactive.activate()).unwrap();
        assert!(
            future::poll_once(&mut wait).await.is_none(),
            "queued local prompt is excluded"
        );
        tx.try_broadcast(Arc::new(b"remote@host:~% ".to_vec()))
            .unwrap();
        assert_eq!(
            wait.await,
            StartupCommandWaitOutcome::Ready("printf remote-only".to_string())
        );
        drop(keeper);
    });
}

#[test]
fn cancelled_launch_never_arms_startup_prompt_detection() {
    future::block_on(async {
        let (started, start) = tokio::sync::oneshot::channel();
        drop(started);
        assert_eq!(
            wait_for_startup_command(start, "must not run".to_string(), Duration::from_secs(1))
                .await,
            StartupCommandWaitOutcome::EndOfStream,
        );
    });
}
