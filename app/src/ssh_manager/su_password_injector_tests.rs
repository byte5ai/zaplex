use super::{
    is_su_to_root, should_spawn_su_password_injector, su_prompt_events, ShellReadyOutcome,
    SuInjectorEvent, SuRootAttemptGuard, PASSWORD_PROMPT_REGEX, SU_ROOT_ATTEMPT_TIMEOUT,
};
use futures_lite::{future, StreamExt as _};
use instant::Instant;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use warp_core::SessionId;
use zeroize::Zeroizing;

fn pw_matches(input: &str) -> bool {
    PASSWORD_PROMPT_REGEX.is_match(input.as_bytes())
}

fn su_matches(input: &str) -> bool {
    is_su_to_root(input)
}

#[test]
fn password_prompt_matches_typical_forms() {
    // Half-width colon
    assert!(pw_matches("Password:"));
    assert!(pw_matches("Password: "));
    assert!(pw_matches("[sudo] password for alice: "));
    assert!(pw_matches("user@host's password: "));
    // Full-width colon (Chinese input method)
    assert!(pw_matches("密码:"));
    assert!(pw_matches("密码："));
    // Kylin Galaxy V10 colon-less special case
    assert!(pw_matches("输入密码"));
    assert!(pw_matches("输入密码 "));
    // passphrase
    assert!(pw_matches(
        "Enter passphrase for key '/home/u/.ssh/id_rsa': "
    ));
}

#[test]
fn password_prompt_rejects_false_positives() {
    // These all contain 'password' (or its localized form) but are not actual prompts; must avoid false positives
    assert!(!pw_matches("Your password has expired"));
    assert!(!pw_matches("Bad password, try again"));
    assert!(!pw_matches("password changed successfully"));
    assert!(!pw_matches("New password for root"));
    assert!(!pw_matches("Welcome! Please change your password soon.\n"));
    assert!(!pw_matches(
        "Last login: Mon Jan 1 password rotated yesterday\n"
    ));
    // Same logic for Chinese
    assert!(!pw_matches("您的密码已过期"));
}

#[test]
fn su_root_matches_common_variants() {
    // Most basic
    assert!(su_matches("su"));
    assert!(su_matches("su\n"));
    // Shortcut form without username (defaults to root)
    assert!(su_matches("su -"));
    assert!(su_matches("su -l"));
    assert!(su_matches("su --login"));
    // Explicit root
    assert!(su_matches("su root"));
    assert!(su_matches("su - root"));
    assert!(su_matches("su -l root"));
    assert!(su_matches("su --login root"));
    // Explicitly supported sudo wrapper
    assert!(su_matches("sudo su"));
}

#[test]
fn su_to_other_user_does_not_match() {
    // Switching to non-root user should not trigger
    assert!(!su_matches("su lg"));
    assert!(!su_matches("su - lg"));
    assert!(!su_matches("su -l lg"));
    assert!(!su_matches("su --login lg"));
    assert!(!su_matches("su admin"));
}

#[test]
fn su_in_middle_of_other_command_does_not_match() {
    // su not at line end should not trigger
    assert!(!su_matches("susan"));
    assert!(!su_matches("issue"));
    // Commands like grep su file; line end is neither su nor su root pattern
    assert!(!su_matches("grep su /etc/passwd"));
    assert!(!su_matches("echo su"));
    assert!(!su_matches("printf done; su"));
}

#[test]
fn absolute_su_path_targets_root() {
    assert!(su_matches("/bin/su - root"));
    assert!(su_matches("sudo /usr/bin/su --login root"));
}

fn shared_guard() -> Arc<Mutex<SuRootAttemptGuard>> {
    Arc::new(Mutex::new(SuRootAttemptGuard::default()))
}

fn collect_prompt_events(
    chunks: Vec<Vec<u8>>,
    attempt_guard: Arc<Mutex<SuRootAttemptGuard>>,
) -> Vec<SuInjectorEvent> {
    future::block_on(async {
        let (tx, rx) = async_broadcast::broadcast(4);
        let mut stream = Box::pin(su_prompt_events(
            rx.deactivate(),
            Duration::from_secs(1),
            attempt_guard,
        ));
        let collect = async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event);
            }
            events
        };
        let send = async move {
            for chunk in chunks {
                tx.broadcast(Arc::new(chunk)).await.unwrap();
            }
            drop(tx);
        };

        let (events, ()) = future::zip(collect, send).await;
        events
    })
}

#[test]
fn remote_su_echo_and_password_prompt_do_not_authorize_confirmation() {
    let events = collect_prompt_events(
        vec![b"user@host:~$ ".to_vec(), b"su\r\nPassword: ".to_vec()],
        shared_guard(),
    );

    assert_eq!(
        events,
        vec![SuInjectorEvent::ShellReadyFinished(
            ShellReadyOutcome::Ready
        )]
    );
}

#[test]
fn local_root_su_authorizes_exactly_one_confirmation() {
    let attempt_guard = shared_guard();
    assert!(attempt_guard
        .lock()
        .observe_command("su - root", SessionId::from(7), true, Instant::now(),)
        .is_some());

    let events = collect_prompt_events(
        vec![
            b"user@host:~$ ".to_vec(),
            b"Password: ".to_vec(),
            b"Password: ".to_vec(),
        ],
        attempt_guard,
    );

    assert_eq!(
        events,
        vec![
            SuInjectorEvent::ShellReadyFinished(ShellReadyOutcome::Ready),
            SuInjectorEvent::PasswordPrompt,
        ]
    );
}

#[test]
fn non_root_remote_and_non_user_commands_do_not_arm_attempt() {
    let now = Instant::now();
    let mut guard = SuRootAttemptGuard::default();

    assert!(guard
        .observe_command("su admin", SessionId::from(1), true, now)
        .is_none());
    assert!(!guard.consume(now));

    assert!(guard
        .observe_command("su - root", SessionId::from(1), false, now)
        .is_none());
    assert!(!guard.consume(now));
}

#[test]
fn timeout_later_command_and_session_change_clear_attempt() {
    let now = Instant::now();
    let session = SessionId::from(1);
    let mut guard = SuRootAttemptGuard::default();

    assert!(guard.observe_command("su", session, true, now).is_some());
    assert!(!guard.consume(now + SU_ROOT_ATTEMPT_TIMEOUT + Duration::from_millis(1)));

    let expired_generation = guard.observe_command("su", session, true, now).unwrap();
    guard.expire(expired_generation);
    assert!(!guard.consume(now));

    assert!(guard.observe_command("su", session, true, now).is_some());
    assert!(guard
        .observe_command("whoami", session, true, now)
        .is_none());
    assert!(!guard.consume(now));

    assert!(guard.observe_command("su", session, true, now).is_some());
    guard.clear_if_session_changed(Some(SessionId::from(2)));
    assert!(!guard.consume(now));
}

#[test]
fn earlier_timeout_does_not_clear_newer_attempt() {
    let now = Instant::now();
    let session = SessionId::from(1);
    let mut guard = SuRootAttemptGuard::default();

    let first_generation = guard.observe_command("su", session, true, now).unwrap();
    assert!(guard.observe_command("su", session, true, now).is_some());
    guard.expire(first_generation);

    assert!(guard.consume(now));
}

#[test]
fn abort_or_shell_return_clear_attempt() {
    let now = Instant::now();
    let mut guard = SuRootAttemptGuard::default();

    assert!(guard
        .observe_command("su", SessionId::from(1), true, now)
        .is_some());
    guard.clear();
    assert!(!guard.consume(now));
}

#[test]
fn should_spawn_su_password_injector_requires_non_empty_root_password() {
    assert!(!should_spawn_su_password_injector(None));

    let empty_password = Zeroizing::new(String::new());
    assert!(!should_spawn_su_password_injector(Some(&empty_password)));

    let password = Zeroizing::new("root-password".to_string());
    assert!(should_spawn_su_password_injector(Some(&password)));
}

fn timeout_event_under_continuous_output() -> (SuInjectorEvent, Duration) {
    let (tx, rx) = async_broadcast::broadcast(64);
    let rx = rx.deactivate();
    let stop = Arc::new(AtomicBool::new(false));
    let producer_stop = stop.clone();
    let producer = std::thread::spawn(move || {
        while !producer_stop.load(Ordering::Relaxed) {
            let _ = tx.try_broadcast(Arc::new(b"still connecting\r\n".to_vec()));
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let started = Instant::now();
    let mut events = Box::pin(su_prompt_events(
        rx,
        Duration::from_millis(30),
        shared_guard(),
    ));
    let event = futures_lite::future::block_on(events.next()).unwrap();
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    producer.join().unwrap();
    (event, elapsed)
}

#[test]
fn su_shell_ready_timeout_is_absolute_under_continuous_output() {
    let (event, elapsed) = timeout_event_under_continuous_output();

    assert_eq!(
        event,
        SuInjectorEvent::ShellReadyFinished(ShellReadyOutcome::TimedOut)
    );
    assert!(elapsed < Duration::from_millis(250));
}

#[test]
fn su_timeout_releases_onekey_suppression() {
    let (event, _) = timeout_event_under_continuous_output();

    assert!(event.releases_onekey_suppression());
}
