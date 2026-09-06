use std::sync::Arc;
use std::time::Duration;

use futures_lite::future;

use super::{wait_for_startup_command, StartupCommandWaitOutcome};

#[test]
fn startup_command_runs_after_percent_prompt() {
    futures_lite::future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(4);
        let rx = rx.deactivate();
        let wait = wait_for_startup_command(rx, "printf ready".to_string(), Duration::from_secs(1));
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
