use std::time::Duration;

use super::{wait_for_oauth_callback, CallbackResult, OAuthCallbackWaitError};

fn success() -> CallbackResult {
    CallbackResult::Success {
        code: "code".to_string(),
        csrf_token: "state".to_string(),
    }
}

#[tokio::test(start_paused = true)]
async fn oauth_callback_timeout_drops_the_flow_receiver() {
    let (tx, rx) = async_channel::unbounded();

    let result = wait_for_oauth_callback(rx, Duration::from_secs(10)).await;

    assert_eq!(result, Err(OAuthCallbackWaitError::TimedOut));
    assert_eq!(tx.is_closed(), true);
}

#[tokio::test(start_paused = true)]
async fn callback_immediately_before_timeout_wins() {
    let (tx, rx) = async_channel::unbounded();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(9)).await;
        tx.send(success()).await.expect("receiver should be open");
    });

    let result = wait_for_oauth_callback(rx, Duration::from_secs(10)).await;

    assert!(matches!(result, Ok(CallbackResult::Success { .. })));
}

#[tokio::test(start_paused = true)]
async fn callback_after_timeout_is_rejected_and_new_flow_can_complete() {
    let (old_tx, old_rx) = async_channel::unbounded();
    let old_result = wait_for_oauth_callback(old_rx, Duration::from_secs(10)).await;
    assert_eq!(old_result, Err(OAuthCallbackWaitError::TimedOut));
    assert!(old_tx.send(success()).await.is_err());

    let (new_tx, new_rx) = async_channel::unbounded();
    new_tx
        .send(success())
        .await
        .expect("new flow receiver should be open");
    let new_result = wait_for_oauth_callback(new_rx, Duration::from_secs(10)).await;
    assert!(matches!(new_result, Ok(CallbackResult::Success { .. })));
}
