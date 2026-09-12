use super::{is_provider_credential_environment_variable, IdleTimeoutSender};
use std::{sync::Arc, time::Duration};
use warpui::r#async::{executor::Background, Timer};

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
        let (tx, mut rx) = futures::channel::oneshot::channel();
        let idle_timeout = IdleTimeoutSender::new(tx, executor);
        idle_timeout.end_run_after(Duration::from_millis(5), "stale");
        idle_timeout.end_run_after(Duration::from_millis(25), "current");

        Timer::after(Duration::from_millis(10)).await;
        assert!(matches!(rx.try_recv(), Ok(None)));
        assert_eq!(
            rx.await.expect("current idle timer should complete"),
            "current"
        );
    });
}
