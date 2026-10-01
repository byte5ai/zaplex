use super::{is_provider_credential_environment_variable, IdleTimeoutSender};
use std::{sync::Arc, time::Duration};
use warpui::r#async::executor::Background;

#[test]
fn identifies_provider_credentials_that_must_not_reach_cli_harnesses() {
    for name in crate::ai::subscription_agent::CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES
        .into_iter()
        .chain(["OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"])
    {
        assert!(is_provider_credential_environment_variable(name));
    }

    assert!(!is_provider_credential_environment_variable("GITHUB_TOKEN"));
}

#[test]
fn resetting_idle_timeout_cancels_the_previous_async_timer() {
    let executor = Arc::new(Background::new(1, |_| "idle-timeout-test".to_string()));

    warpui::r#async::block_on(async move {
        let (tx, rx) = futures::channel::oneshot::channel();
        let idle_timeout = IdleTimeoutSender::new(tx, executor);
        idle_timeout.end_run_after(Duration::from_secs(60), "stale");
        let previous_timer = idle_timeout
            .timer_abort_handle
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        idle_timeout.end_run_after(Duration::ZERO, "current");

        // Cancellation and delivery do not depend on waking inside a 15 ms window.
        assert!(previous_timer.is_aborted());
        assert_eq!(
            rx.await.expect("current idle timer should complete"),
            "current"
        );
    });
}
