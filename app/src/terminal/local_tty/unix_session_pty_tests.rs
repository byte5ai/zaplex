use super::{session_spawn_command, spawn_session_pty};
use crate::terminal::shell::ShellType;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
fn open_pty_master_fds() -> std::collections::HashSet<i32> {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let target = fs::read_link(entry.path()).ok()?;
            (target == std::path::Path::new("/dev/ptmx")
                || target == std::path::Path::new("/dev/pts/ptmx"))
            .then(|| entry.file_name().to_string_lossy().parse().ok())
            .flatten()
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn daemon_fish_and_pwsh_launch_contract_is_one_shot() {
    for (shell_type, shell_name) in [(ShellType::Fish, "fish"), (ShellType::PowerShell, "pwsh")] {
        let dir = tempfile::tempdir().expect("temporary executable directory");
        let shell_path = dir.path().join(shell_name);
        fs::write(&shell_path, b"#!/bin/sh\n").expect("fake shell executable");
        let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell_path, permissions).unwrap();

        let mut env = HashMap::new();
        env.insert(
            "ZAPLEX_DAEMON_BOOTSTRAP_FILE".to_string(),
            "/tmp/client-controlled-bootstrap".to_string(),
        );
        let prepared =
            session_spawn_command(shell_path.to_str().unwrap(), &env).expect("spawn contract");
        assert_eq!(
            prepared.bootstrap_delivery,
            crate::terminal::bootstrap::DaemonBootstrapDelivery::GuardedFile,
            "fish/PowerShell must use the same guarded-file contract as the server"
        );

        let bootstrap_file = prepared
            .bootstrap_file
            .as_ref()
            .expect("fish/PowerShell require a session-owned body file");
        let expected_route =
            OsString::from_vec(bootstrap_file.path_as_bytes().expect("bootstrap path"));
        let actual_route = prepared
            .command
            .get_envs()
            .find_map(|(key, value)| (key == "ZAPLEX_DAEMON_BOOTSTRAP_FILE").then_some(value))
            .flatten();
        assert_eq!(
            actual_route,
            Some(expected_route.as_os_str()),
            "the daemon-owned route must override a colliding client environment value"
        );
        let body = String::from_utf8(fs::read(&expected_route).expect("bootstrap body file"))
            .expect("bootstrap body is UTF-8");
        assert!(
            body.len() > 1_000,
            "the body must be the real bundled script"
        );
        match shell_type {
            ShellType::Fish => {
                assert!(body.starts_with("if test \"$ZAPLEX_BOOTSTRAPPED\" != 1\n"));
                assert!(body.contains("set -g ZAPLEX_BOOTSTRAPPED 1"));
            }
            ShellType::PowerShell => {
                assert!(body.starts_with("param()\nif ($global:ZAPLEX_BOOTSTRAPPED -ne 1) {"));
                assert!(body.contains("$global:ZAPLEX_BOOTSTRAPPED = 1"));
            }
            ShellType::Bash | ShellType::Zsh => unreachable!(),
        }
        assert_eq!(
            std::path::Path::new(&expected_route)
                .extension()
                .and_then(|s| s.to_str()),
            (shell_type == ShellType::PowerShell).then_some("ps1")
        );

        let args = prepared
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        match shell_type {
            ShellType::Fish => {
                assert_eq!(args.first().map(String::as_str), Some("--no-config"));
                assert_eq!(args.get(1).map(String::as_str), Some("-c"));
                assert!(args.get(2).is_some_and(|arg| {
                    arg.contains("--login --init-command")
                        && arg.contains("ZAPLEX_DAEMON_BOOTSTRAP_FILE")
                }));
            }
            ShellType::PowerShell => {
                assert_eq!(&args[..4], ["-NoLogo", "-NoProfile", "-NoExit", "-Command"]);
                assert!(args
                    .get(4)
                    .is_some_and(|arg| arg.contains("ZAPLEX_DAEMON_BOOTSTRAP_FILE")));
            }
            ShellType::Bash | ShellType::Zsh => unreachable!(),
        }
    }
}

/// Spawns a real shell on a daemon-owned PTY, applies a resize via
/// TIOCSWINSZ (the same ioctl the session host's resize handler uses), and
/// confirms that (a) a command's output streams back through the master and
/// (b) the resize reached the slave tty (`stty size` → `30 100`). Exercises
/// the OS-level behaviour the daemon reader/writer tasks rely on
/// (Stage 1 #9/#10).
///
/// Robustness: the read is non-blocking with a deadline and we tear the
/// shell down ourselves — the test never depends on the shell self-exiting,
/// so it cannot hang.
#[test]
fn spawn_session_pty_streams_and_resizes() {
    let env = HashMap::new();
    let (leader, mut child, _bootstrap_file) =
        spawn_session_pty(None, "/bin/sh", &env, 24, 80).expect("spawn_session_pty");
    let mut master = std::fs::File::from(leader);

    // Resize to 30x100 via the same ioctl the daemon's ResizeSession handler
    // uses, before issuing the command.
    let win = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `master` wraps a live PTY leader fd.
    let rc = unsafe {
        libc::ioctl(
            master.as_raw_fd(),
            libc::TIOCSWINSZ,
            &win as *const libc::winsize,
        )
    };
    assert_eq!(rc, 0, "TIOCSWINSZ should succeed");

    // The marker is computed by the shell (M42K), so it only appears in
    // *executed* output — not in the tty's echo of the command itself.
    master
        .write_all(b"stty size; echo M$((6*7))K\n")
        .expect("write command to pty");

    // Non-blocking master so the read loop can never hang.
    let fd = master.as_raw_fd();
    // SAFETY: toggling O_NONBLOCK on our own fd.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline && !contains(&out, b"M42K") {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }

    // Always tear the shell down ourselves; never rely on it self-exiting.
    let _ = child.kill();
    let _ = child.wait();

    let text = String::from_utf8_lossy(&out);
    assert!(
        contains(&out, b"M42K"),
        "command output should stream back through the PTY; got:\n{text}"
    );
    assert!(
        text.contains("30 100"),
        "stty size should reflect the TIOCSWINSZ resize; got:\n{text}"
    );
}

#[cfg(target_os = "linux")]
#[test]
#[serial_test::serial]
fn failed_spawn_session_pty_does_not_leak_master_fds() {
    let env = HashMap::new();
    let before = open_pty_master_fds();

    for _ in 0..32 {
        let result = spawn_session_pty(
            Some(std::path::Path::new("/definitely-missing-zaplex-cwd")),
            "/bin/sh",
            &env,
            24,
            80,
        );
        assert!(result.is_err());
    }

    assert_eq!(open_pty_master_fds(), before);
}

/// Starting a bash session must not dump the bootstrap into the terminal.
///
/// This is the acceptance defect of 2026-07-17..19, reproduced end to end
/// through the real spawn contract and the real bootstrap script: the
/// daemon writes the shell *body* as the session's first input, and the
/// user saw ~1500 of its lines come straight back — the shell echoing its
/// own input, each continuation line prefixed by bash's `> ` PS2 prompt.
///
/// Two independent causes, both fixed at the source and both asserted here:
///   * `stty raw` does NOT clear ECHO (neither GNU/uutils nor BSD stty
///     include it in `raw`), so `bash_init_shell.sh` now says
///     `stty raw -echo`;
///   * `bash.sh` never blanked PS2 while the heredoc was read — the zsh
///     path always did — so every heredoc line drew a `> `.
#[test]
fn spawn_session_pty_does_not_echo_the_bootstrap() {
    let env = HashMap::new();
    let (leader, mut child, _bootstrap_file) =
        match spawn_session_pty(None, "/bin/bash", &env, 24, 80) {
            Ok(pair) => pair,
            // No bash on this machine: nothing to assert about bash's
            // bootstrap. (The CI images all have it.)
            Err(_) => return,
        };
    let mut master = std::fs::File::from(leader);

    // Non-blocking master so no read below can hang.
    let fd = master.as_raw_fd();
    // SAFETY: toggling O_NONBLOCK on our own fd.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    // Recreate the remote condition IMMEDIATELY, before the shell has
    // finished exec'ing: on the classic-SSH path the PTY is allocated by
    // `ssh` on the remote host with ECHO on, and `spawn_session_pty`'s
    // local clearing has no reach there. The rcfile is what must turn it
    // off — that is the fix under test — so this has to happen before the
    // rcfile runs, never after it (doing it after would simply undo the
    // fix and make the test fail for the wrong reason).
    //
    // Ordering: fork+exec+rcfile takes milliseconds, this ioctl
    // microseconds, so the parent wins in practice. Should it ever lose,
    // the failure is a visible red test, not a silent green one.
    // SAFETY: `fd` is our live PTY leader; termios is zero-initialised and
    // then filled by tcgetattr before use.
    unsafe {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tio) == 0 {
            tio.c_lflag |= libc::ECHO;
            libc::tcsetattr(fd, libc::TCSANOW, &tio);
        }
    }

    // Then wait for the rcfile to have run: it ends by emitting the
    // InitShell DCS (`ESC P $ d …`). Writing the body only after that is
    // what makes the assertions below deterministic — with the fix in
    // place the rcfile has cleared ECHO by now, without it ECHO is still
    // on and the body comes straight back.
    let init_deadline = Instant::now() + Duration::from_secs(10);
    let mut preamble = Vec::new();
    let mut probe = [0u8; 4096];
    while Instant::now() < init_deadline && !contains(&preamble, b"\x1bP$d") {
        match master.read(&mut probe) {
            Ok(0) => break,
            Ok(n) => preamble.extend_from_slice(&probe[..n]),
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    assert!(
        contains(&preamble, b"\x1bP$d"),
        "the rcfile should emit the InitShell DCS before the body is written"
    );

    // Exactly what `handle_open_session` enqueues for a bash session.
    let body = crate::terminal::bootstrap::script_for_shell(ShellType::Bash, &crate::ASSETS);
    // Write from a thread: the body is ~250 KB and the PTY buffer is a few
    // KB, so a blocking write would deadlock against our own read loop.
    let writer = {
        let mut w = master.try_clone().expect("clone pty leader");
        let body = body.to_vec();
        std::thread::spawn(move || {
            let _ = w.write_all(&body);
        })
    };

    // Read for a bounded window; we are looking for the ABSENCE of echo,
    // so there is no marker to wait for — drain what the shell produces.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = writer.join();

    let text = String::from_utf8_lossy(&out);
    // A line that exists only inside the body. Coming back means the tty
    // echoed our input — the dump the user reported.
    let echoed = text.matches("warp_send_json_message").count();
    assert_eq!(
        echoed,
        0,
        "the bootstrap body must not be echoed back ({echoed} occurrences); \
         first 800 bytes:\n{}",
        &text.chars().take(800).collect::<String>()
    );
    // The other half of the dump: bash's `> ` continuation prompt, drawn
    // once per line of every multi-line construct in the script. Measured
    // against the pre-fix scripts: ~1766. A couple can still appear around
    // the very first line (PS2 is only blanked once that line executes),
    // so this asserts the order of magnitude rather than zero.
    let ps2 = text.matches("> ").count();
    assert!(
        ps2 < 20,
        "the bootstrap must not draw a PS2 prompt per line ({ps2} seen); \
         first 800 bytes:\n{}",
        &text.chars().take(800).collect::<String>()
    );
}
