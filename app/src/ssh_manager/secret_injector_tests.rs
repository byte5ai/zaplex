use std::sync::Arc;

use futures_lite::future;
use warp_ssh_manager::AuthType;

use super::watch_for_prompt;

#[test]
fn shell_prompt_disarms_secret_injector_before_sudo_prompt() {
    future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(4);
        let rx = rx.deactivate();
        let watch = watch_for_prompt(rx.activate(), AuthType::Password);
        let send = async move {
            tx.broadcast(Arc::new(b"user@host:~$ ".to_vec()))
                .await
                .unwrap();
            tx.broadcast(Arc::new(b"sudo -i\r\n[sudo] password for user: ".to_vec()))
                .await
                .unwrap();
        };

        let (should_inject, ()) = future::zip(watch, send).await;

        assert!(!should_inject);
    });
}

#[test]
fn password_prompt_before_shell_requests_one_injection() {
    future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(2);
        let rx = rx.deactivate();
        let watch = watch_for_prompt(rx.activate(), AuthType::Password);
        let send = async move {
            tx.broadcast(Arc::new(b"alice@example.com's password: ".to_vec()))
                .await
                .unwrap();
        };

        let (should_inject, ()) = future::zip(watch, send).await;

        assert!(should_inject);
    });
}

#[test]
fn key_auth_never_activates_pty_secret_watcher() {
    future::block_on(async {
        let (_tx, rx) = async_broadcast::broadcast(1);
        let rx = rx.deactivate();

        assert!(!watch_for_prompt(rx.activate(), AuthType::Key).await);
    });
}

#[test]
fn injection_requires_the_exact_first_launch_and_is_consumed_once() {
    use super::SshInjectionAttempt;
    use warp_core::SessionId;
    let session = SessionId::from(101u64);
    assert!(!SshInjectionAttempt::default().finish());
    let mut wrong = SshInjectionAttempt::default();
    wrong.observe_command("ssh expected", "ssh other", session);
    wrong.observe_command("ssh expected", "ssh expected", session);
    assert!(!wrong.finish());
    let mut current = SshInjectionAttempt::default();
    current.observe_command("ssh expected", "ssh expected", session);
    current.observe_completion(Some(session), false, false);
    current.observe_completion(Some(session), true, true);
    current.observe_completion(Some(SessionId::from(102u64)), true, false);
    assert!(
        current.finish(),
        "bootstrap/background/other-session completion is not this SSH exit"
    );
    assert!(!current.finish());
}

#[test]
fn completed_or_cancelled_ssh_cannot_accept_a_queued_injection() {
    use super::SshInjectionAttempt;
    use warp_core::SessionId;
    let session = SessionId::from(101u64);
    for cancel_byte in [3u8, 4u8] {
        let mut attempt = SshInjectionAttempt::default();
        attempt.observe_command("ssh expected", "ssh expected", session);
        attempt.observe_input(&[cancel_byte]);
        assert!(
            !attempt.finish(),
            "Ctrl-C/EOF invalidates an already detected prompt"
        );
    }
    let mut completed = SshInjectionAttempt::default();
    completed.observe_command("ssh expected", "ssh expected", session);
    completed.observe_completion(Some(session), true, false);
    assert!(!completed.finish());
    let mut next_command = SshInjectionAttempt::default();
    next_command.observe_command("ssh expected", "ssh expected", session);
    next_command.observe_command("ssh expected", "ssh expected", session);
    assert!(
        !next_command.finish(),
        "even an identical later command is a different attempt"
    );
}

#[test]
fn askpass_lifetime_ignores_bootstrap_prompt_and_releases_on_abort() {
    use super::wait_for_askpass_release;
    use std::time::Duration;
    future::block_on(async {
        let (tx, keeper) = async_broadcast::broadcast(4);
        let inactive = keeper.clone().deactivate();
        tx.try_broadcast(Arc::new(b"local@host:~$ ".to_vec()))
            .unwrap();
        let (start_sender, started) = tokio::sync::oneshot::channel();
        let (cancel_sender, cancelled) = tokio::sync::oneshot::channel();
        let mut wait = Box::pin(wait_for_askpass_release(
            started,
            cancelled,
            Duration::from_secs(1),
        ));
        assert!(future::poll_once(&mut wait).await.is_none());
        start_sender.send(inactive.activate()).unwrap();
        assert!(
            future::poll_once(&mut wait).await.is_none(),
            "local bootstrap cannot release askpass files"
        );
        cancel_sender.send(()).unwrap();
        assert!(
            future::poll_once(&mut wait).await.is_some(),
            "abort releases without waiting for PTY output"
        );
        drop(keeper);
    });
}

#[test]
fn askpass_timeout_also_bounds_waiting_for_launch() {
    use super::wait_for_askpass_release;
    use std::time::Duration;
    future::block_on(async {
        let (_start_sender, started) = tokio::sync::oneshot::channel();
        let (_cancel_sender, cancelled) = tokio::sync::oneshot::channel();
        wait_for_askpass_release(started, cancelled, Duration::ZERO).await;
    });
}
