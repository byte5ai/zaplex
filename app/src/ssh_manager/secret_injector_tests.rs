use std::sync::Arc;

use futures_lite::future;
use warp_ssh_manager::AuthType;

use super::watch_for_prompt;

#[test]
fn shell_prompt_disarms_secret_injector_before_sudo_prompt() {
    future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(4);
        let rx = rx.deactivate();
        let watch = watch_for_prompt(rx, AuthType::Password);
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
        let watch = watch_for_prompt(rx, AuthType::Password);
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

        assert!(!watch_for_prompt(rx, AuthType::Key).await);
    });
}
