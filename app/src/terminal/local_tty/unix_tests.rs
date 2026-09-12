use super::{get_pw_entry, get_pw_entry_once, spawn_command_in_pty};
use crate::terminal::SizeInfo;
use command::blocking::Command;

#[test]
fn get_pw_entry_once_reports_erange_without_initializing_output() {
    let mut buffer = [0; 1];
    let error = get_pw_entry_once(unsafe { libc::getuid() }, &mut buffer).unwrap_err();

    assert_eq!(error.raw_os_error(), Some(libc::ERANGE));
}

#[test]
fn get_pw_entry_returns_current_user_without_panicking() {
    let passwd = get_pw_entry().unwrap().unwrap();

    assert!(!passwd.name.is_empty());
    assert!(!passwd.dir.as_os_str().is_empty());
}

#[test]
fn missing_executable_is_reported_when_closing_inherited_fds() {
    let command = Command::new("/zaplex-test/missing-shell-executable");
    let result = spawn_command_in_pty(command, &SizeInfo::new_without_font_metrics(24, 80), true);

    let error = match result {
        Ok(_) => panic!("a missing executable must not appear to spawn successfully"),
        Err(error) => error,
    };
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::NotFound)
    );
}

#[test]
fn successful_spawn_still_exits_cleanly_when_closing_inherited_fds() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    let mut spawned =
        match spawn_command_in_pty(command, &SizeInfo::new_without_font_metrics(24, 80), true) {
            Ok(spawned) => spawned,
            Err(error) => panic!("expected successful PTY spawn: {error:#}"),
        };

    let status = spawned.child.wait().expect("child should be waitable");
    unsafe {
        libc::close(spawned.result.leader_fd);
    }
    assert!(status.success());
}
