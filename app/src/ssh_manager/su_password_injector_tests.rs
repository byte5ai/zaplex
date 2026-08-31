use super::{
    is_su_to_root, should_spawn_su_password_injector, su_prompt_events, ShellReadyOutcome,
    SuInjectorEvent, PASSWORD_PROMPT_REGEX, SU_ROOT_CMD_REGEX,
};
use futures_lite::StreamExt as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

fn pw_matches(input: &str) -> bool {
    PASSWORD_PROMPT_REGEX.is_match(input.as_bytes())
}

fn su_matches(input: &str) -> bool {
    SU_ROOT_CMD_REGEX.is_match(input.as_bytes())
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
    // sudo su (still matches trailing `su`)
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
}

#[test]
fn is_su_to_root_detects_in_buffer() {
    let buf = b"user@host:~$ su root\r\nPassword: ";
    assert!(is_su_to_root(buf));

    let buf = b"user@host:~$ su lg\r\nPassword: ";
    assert!(!is_su_to_root(buf));
}

#[test]
fn full_pipeline_su_root_with_password_prompt() {
    // Simulate complete PTY sequence: user enters `su -`, echoed back with password prompt
    let buf = b"alice@kylin:~$ su -\r\n\xe5\xaf\x86\xe7\xa0\x81\xef\xbc\x9a";
    assert!(PASSWORD_PROMPT_REGEX.is_match(buf));
    assert!(is_su_to_root(buf));
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
    let mut events = Box::pin(su_prompt_events(rx, Duration::from_millis(30)));
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
