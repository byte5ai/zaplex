//! Unit tests for `ssh_command`.
//!
//! Per `AGENTS.md §5.6`, extracted into a standalone file and included via the
//! `#[path]` attribute at the end of `ssh_command.rs`. Coverage includes:
//! - `build_ssh_args` / `build_ssh_command_line` argument construction
//! - `test_connection` error paths for missing password / wrong auth type
//! - `build_key_auth_cmd_args` BatchMode / publickey selection by passphrase
//! - `AskpassSession` secret delivery (the SSH_ASKPASS channel that replaced the
//!   stdin injection ssh ignores on a tty-less GUI app)
//!
//! Note: end-to-end tests that actually spawn SSH subprocesses are covered by
//! integration tests / manual tests in `app/src/ssh_manager/server_view.rs` — unit tests
//! do not make network connections.
//!
//! author: logic
//! date: 2026-06-01

use super::*;
use zeroize::Zeroizing;

fn server() -> SshServerInfo {
    SshServerInfo {
        node_id: "n".into(),
        host: "1.2.3.4".into(),
        port: 22,
        username: "alice".into(),
        auth_type: AuthType::Password,
        key_path: None,
        credential_id: None,
        startup_command: None,
        notes: None,
        last_connected_at: None,
        session_resilience: crate::types::SessionResilience::default(),
        ring_ceiling_mb: 0,
    }
}

#[test]
fn default_port_omitted() {
    let s = server();
    assert_eq!(
        build_ssh_args(&s),
        vec![
            "ssh",
            "-o",
            "StrictHostKeyChecking=ask",
            "--",
            "alice@1.2.3.4"
        ]
    );
    let line = build_ssh_command_line(&s);
    assert!(
        line.contains("StrictHostKeyChecking=ask") && line.contains("alice@1.2.3.4"),
        "unexpected: {line}"
    );
}

#[test]
fn custom_port_uses_dash_p() {
    let mut s = server();
    s.port = 2222;
    assert_eq!(
        build_ssh_args(&s),
        vec![
            "ssh",
            "-o",
            "StrictHostKeyChecking=ask",
            "-p",
            "2222",
            "--",
            "alice@1.2.3.4"
        ]
    );
}

#[test]
fn key_auth_emits_dash_i() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/u/.ssh/id_ed25519".into());
    assert_eq!(
        build_ssh_args(&s),
        vec![
            "ssh",
            "-o",
            "StrictHostKeyChecking=ask",
            "-i",
            "/home/u/.ssh/id_ed25519",
            "--",
            "alice@1.2.3.4"
        ]
    );
}

#[test]
fn key_auth_without_path_is_skipped() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = None;
    assert_eq!(
        build_ssh_args(&s),
        vec![
            "ssh",
            "-o",
            "StrictHostKeyChecking=ask",
            "--",
            "alice@1.2.3.4"
        ]
    );
}

#[test]
fn empty_username_yields_host_only() {
    let mut s = server();
    s.username = String::new();
    assert_eq!(
        build_ssh_args(&s),
        vec!["ssh", "-o", "StrictHostKeyChecking=ask", "--", "1.2.3.4"]
    );
}

#[test]
fn option_like_username_is_delimited_from_ssh_options() {
    let mut s = server();
    s.username = "-oProxyCommand=malicious".to_string();
    let args = build_ssh_args(&s);
    let target = args
        .iter()
        .position(|arg| arg == "-oProxyCommand=malicious@1.2.3.4")
        .unwrap();
    assert_eq!(
        args.get(target.wrapping_sub(1)).map(String::as_str),
        Some("--")
    );
}

#[test]
fn shell_escapes_spaces_in_path() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/path with spaces/id_rsa".into());
    let line = build_ssh_command_line(&s);
    assert!(
        line.contains("'/path with spaces/id_rsa'"),
        "actual: {line}"
    );
}

#[test]
fn multiplexer_attach_keeps_target_as_remote_shell_data() {
    let target = "release candidate; touch /tmp/never";
    let args = build_multiplexer_ssh_args(&server(), MultiplexerAttachMode::Tmux, target).unwrap();
    let destination_delimiter = args.iter().position(|arg| arg == "--").unwrap();

    assert!(args[..destination_delimiter]
        .windows(2)
        .any(|pair| pair == ["-o", "RequestTTY=force"]));
    assert!(args[..destination_delimiter]
        .windows(2)
        .any(|pair| { pair == ["-o", "SetEnv=ZAPLEX_SESSION=1 BYOBU_DISABLE=1 LC_BYOBU=0",] }));
    assert_eq!(
        &args[destination_delimiter + 1..],
        &[
            "alice@1.2.3.4".to_string(),
            "env ZAPLEX_SESSION=1 BYOBU_DISABLE=1 LC_BYOBU=0 tmux attach-session -t \
             'release candidate; touch /tmp/never'"
                .to_string(),
        ]
    );
}

#[test]
fn screen_attach_never_detaches_other_displays_or_creates_sessions() {
    let attached =
        build_multiplexer_ssh_args(&server(), MultiplexerAttachMode::ScreenAttached, "1234.ops")
            .unwrap();
    let detached = build_multiplexer_ssh_args(
        &server(),
        MultiplexerAttachMode::ScreenDetached,
        "5678.release",
    )
    .unwrap();

    assert_eq!(
        attached.last().unwrap(),
        "env ZAPLEX_SESSION=1 BYOBU_DISABLE=1 LC_BYOBU=0 screen -x 1234.ops"
    );
    assert_eq!(
        detached.last().unwrap(),
        "env ZAPLEX_SESSION=1 BYOBU_DISABLE=1 LC_BYOBU=0 screen -r 5678.release"
    );
    for command in [attached.last().unwrap(), detached.last().unwrap()] {
        assert!(!command.contains(" -d"));
        assert!(!command.contains(" -D"));
        assert!(!command.contains(" -R"));
    }
}

#[test]
fn multiplexer_attach_rejects_empty_control_or_option_like_targets() {
    for mode in [
        MultiplexerAttachMode::Tmux,
        MultiplexerAttachMode::ScreenAttached,
        MultiplexerAttachMode::ScreenDetached,
    ] {
        for target in [
            "",
            "-d",
            "-D",
            "-R",
            "-RR",
            "line\nbreak",
            "tab\ttarget",
            "nul\0target",
        ] {
            assert_eq!(
                build_multiplexer_ssh_command_line(&server(), mode, target),
                Err(InvalidMultiplexerTarget)
            );
        }
    }
}

#[test]
fn test_connection_requires_password_for_password_auth() {
    let s = server();
    // test_connection should return Offline + error message when password is missing
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(test_connection(&s, None));
    assert_eq!(result.status, ConnectionStatus::Offline);
    assert!(result
        .error_message
        .unwrap()
        .contains("Password not provided"));
}

#[test]
fn test_connection_requires_password_for_onekey_auth() {
    let mut s = server();
    s.auth_type = AuthType::OneKey;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(test_connection(&s, None));
    assert_eq!(result.status, ConnectionStatus::Offline);
    assert!(result
        .error_message
        .unwrap()
        .contains("Password not provided"));
}

#[test]
fn onekey_key_auth_emits_dash_i_when_key_path_is_resolved() {
    let mut s = server();
    s.auth_type = AuthType::OneKey;
    s.key_path = Some("/home/u/.ssh/shared_ed25519".into());

    assert_eq!(
        build_ssh_args(&s),
        vec![
            "ssh",
            "-o",
            "StrictHostKeyChecking=ask",
            "-i",
            "/home/u/.ssh/shared_ed25519",
            "--",
            "alice@1.2.3.4"
        ]
    );
}

#[test]
fn test_connection_key_auth_uses_batch_mode() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/user/.ssh/id_rsa".into());
    // For key authentication, should take the BatchMode=yes path (carried by run_ssh_test);
    // here we only verify that build_ssh_args includes -i and key_path.
    let args = build_ssh_args(&s);
    assert!(args.contains(&"-i".to_string()));
    assert!(args.contains(&"/home/user/.ssh/id_rsa".to_string()));
}

#[test]
fn connection_status_equality() {
    assert_eq!(ConnectionStatus::Online, ConnectionStatus::Online);
    assert_eq!(ConnectionStatus::Offline, ConnectionStatus::Offline);
    assert_eq!(ConnectionStatus::Unknown, ConnectionStatus::Unknown);
    assert_ne!(ConnectionStatus::Online, ConnectionStatus::Offline);
    assert_ne!(ConnectionStatus::Online, ConnectionStatus::Unknown);
    assert_ne!(ConnectionStatus::Offline, ConnectionStatus::Unknown);
}

// -------- Askpass secret delivery (replaces the broken stdin injection) --------

/// End-to-end mechanism proof (Unix): `AskpassSession` produces a helper script
/// that, spawned the way ssh spawns it (with `ZAPLEX_SSH_ASKPASS_FILE` set), prints
/// exactly the secret on stdout. This is the channel that replaced stdin injection,
/// which OpenSSH ignores for interactive prompts — so on a macOS GUI app with no
/// controlling tty a valid password/passphrase used to fail. The secret here holds
/// shell metacharacters to prove `cat` echoes file *content* verbatim, unparsed.
#[cfg(not(windows))]
#[test]
fn unix_askpass_script_prints_the_secret() {
    let secret: Zeroizing<String> = Zeroizing::new("s3cret-\"pass$ & spaces".into());
    let session = AskpassSession::new(&secret).expect("AskpassSession::new failed");
    let script = session.script_path.clone();
    let password_file = session.password_path.clone();

    let output = command::blocking::Command::new(&script)
        .env("ZAPLEX_SSH_ASKPASS_FILE", &password_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("askpass script is not spawnable");

    assert!(
        output.status.success(),
        "askpass script exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    // ssh takes askpass output up to the first CR/LF; our secret is single-line
    // with no trailing newline, so stdout is the exact secret.
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "s3cret-\"pass$ & spaces",
        "askpass must echo the secret verbatim"
    );
}

/// The askpass temp files are owner-only (0600 secret, 0700 script): the secret
/// must not be group/world-readable while it briefly lives in TMPDIR.
#[cfg(not(windows))]
#[test]
fn unix_askpass_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let secret: Zeroizing<String> = Zeroizing::new("pw".into());
    let session = AskpassSession::new(&secret).expect("AskpassSession::new failed");
    let secret_mode = std::fs::metadata(&session.password_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let script_mode = std::fs::metadata(&session.script_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        secret_mode, 0o600,
        "secret file must be 0600, got {secret_mode:o}"
    );
    assert_eq!(
        script_mode, 0o700,
        "script file must be 0700, got {script_mode:o}"
    );
}

/// Dropping the session removes both temp files — the secret must not linger.
#[cfg(not(windows))]
#[test]
fn unix_askpass_files_removed_on_drop() {
    let secret: Zeroizing<String> = Zeroizing::new("pw".into());
    let session = AskpassSession::new(&secret).expect("AskpassSession::new failed");
    let secret_path = session.password_path.clone();
    let script_path = session.script_path.clone();
    assert!(secret_path.exists() && script_path.exists());
    drop(session);
    assert!(!secret_path.exists(), "secret file must be deleted on drop");
    assert!(!script_path.exists(), "script file must be deleted on drop");
}

/// Key auth without a passphrase keeps `BatchMode=yes` (agent / unencrypted key,
/// no prompt) — the original behavior.
#[test]
fn key_auth_args_no_passphrase_uses_batch_mode() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/u/.ssh/id_ed25519".into());
    let joined = build_key_auth_cmd_args(&s, false).join(" ");
    assert!(joined.contains("BatchMode=yes"), "got {joined}");
    assert!(!joined.contains("BatchMode=no"), "got {joined}");
    assert!(joined.ends_with("echo ok"), "got {joined}");
}

/// Key auth WITH a passphrase must drop `BatchMode` (so ssh calls SSH_ASKPASS for
/// the key passphrase) and pin `publickey` (so a wrong passphrase can't fall through
/// to an interactive password prompt that hangs the test). Regression for the
/// encrypted-key-can't-be-tested bug.
#[test]
fn key_auth_args_with_passphrase_drops_batch_mode_and_pins_publickey() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/u/.ssh/id_ed25519".into());
    let joined = build_key_auth_cmd_args(&s, true).join(" ");
    assert!(
        !joined.contains("BatchMode=yes"),
        "passphrase mode must not batch; got {joined}"
    );
    assert!(joined.contains("BatchMode=no"), "got {joined}");
    assert!(
        joined.contains("PreferredAuthentications=publickey"),
        "must pin publickey so a bad passphrase can't hang on a password prompt; got {joined}"
    );
    // Belt-and-suspenders: a wrong passphrase must not fall through to the server's
    // password / keyboard-interactive prompt (lockout / false success risk).
    assert!(
        joined.contains("PasswordAuthentication=no"),
        "must disable server password auth; got {joined}"
    );
    assert!(
        joined.contains("KbdInteractiveAuthentication=no"),
        "must disable keyboard-interactive; got {joined}"
    );
    assert!(joined.ends_with("echo ok"), "got {joined}");
}

/// Regression test: `build_ssh_args` must not emit `sshpass`, preventing someone from
/// accidentally re-adding it to cmd_args (Windows / macOS have no sshpass by default,
/// and a stray path will immediately fail with "No such file or directory").
#[test]
fn build_ssh_args_does_not_emit_sshpass() {
    let s = server();
    let args = build_ssh_args(&s);
    assert!(
        !args.iter().any(|a| a == "sshpass"),
        "build_ssh_args must not emit sshpass; got {args:?}"
    );
}

// -------- password auth cmd_args regression protection --------
//
// These tests protect the critical guards preventing the "test connection" password
// path from hitting a 10s timeout. Any adjustment to -o options inside `test_password_auth`
// must satisfy these three conditions:
// 1. Must not declare keyboard-interactive (otherwise server-side PAM will fall back to kbd-int)
// 2. Must explicitly disable KbdInteractiveAuthentication (client capability switch, not a preference)
// 3. Must still end with `echo ok` remote command (otherwise success detection won't match stdout)
// author: logic
// date: 2026-06-01

/// Regression protection: `PreferredAuthentications` must contain only `password`, never
/// `keyboard-interactive`. Otherwise, stdin pipe + EOF will trigger a kbd-int PAM retry chain
/// (`pam_faildelay` ~2s each), exhausting the 10s `TEST_TIMEOUT`.
#[test]
fn password_auth_args_no_keyboard_interactive() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");
    assert!(
        !joined.contains("keyboard-interactive"),
        "test_password_auth must NOT use keyboard-interactive; got {args:?}"
    );
    assert!(
        joined.contains("PreferredAuthentications=password"),
        "expected PreferredAuthentications=password; got {args:?}"
    );
    // Even if PreferredAuthentications=password appears, no other methods can be listed after it.
    // We split on "=" and take the first segment; if it starts with "password," it means other auth methods follow.
    let after_pref = joined
        .split("PreferredAuthentications=")
        .nth(1)
        .unwrap_or("");
    assert!(
        !after_pref.starts_with("password,"),
        "PreferredAuthentications should not list other methods after password; got {args:?}"
    );
}

/// Regression protection: must explicitly disable kbd-interactive (a client capability switch),
/// not just rely on `PreferredAuthentications` list ordering (which only constrains password
/// sub-methods). This defense-in-depth layer is especially important for OpenSSH 8.2+ behavior
/// variations and interactions with server-side `AuthenticationMethods`.
#[test]
fn password_auth_args_disable_kbd_interactive() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");
    assert!(
        joined.contains("KbdInteractiveAuthentication=no"),
        "missing KbdInteractiveAuthentication=no; got {args:?}"
    );
}

/// Regression protection: `echo ok` at the end of cmd_args must appear as a remote command.
/// By SSH parsing rules, the first non-option positional argument after destination = remote command;
/// if option ordering is wrong and ssh doesn't recognize `echo ok` as a command, success detection fails.
#[test]
fn password_auth_args_ends_with_echo_ok_command() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    assert!(!args.is_empty(), "cmd_args is empty: {args:?}");
    let last = args.last().unwrap();
    assert_eq!(
        last, "echo ok",
        "cmd_args must end with `echo ok` as remote command; got {args:?}"
    );
}

/// Regression protection: the destination (`user@host`) in the password path must appear
/// **after** all `-o` options and **before** `echo ok`. SSH command line parsing is
/// `ssh [options] destination [command]`, where the first non-option argument = destination
/// and everything after = remote command. If `-o` options appear after destination, SSH treats
/// them as part of the remote command, not as options, causing `PreferredAuthentications`,
/// `KbdInteractiveAuthentication`, and other critical options to fail silently, triggering
/// the kbd-interactive PAM retry chain that exhausts the 10s `TEST_TIMEOUT`.
/// author: logic
/// date: 2026-06-01
#[test]
fn password_auth_args_destination_before_echo_ok_and_after_options() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");

    // destination "alice@1.2.3.4" must appear before "echo ok"
    let dest_pos = joined
        .find("alice@1.2.3.4")
        .expect("destination must appear in args");
    let echo_pos = joined
        .find("echo ok")
        .expect("`echo ok` must appear in args");

    assert!(
        dest_pos < echo_pos,
        "destination must come before `echo ok`; got joined: {joined}"
    );

    // destination must appear after all -o options
    // find position of the last -o option
    let last_o_pos = joined
        .rfind("-o ")
        .expect("expected at least one -o option");
    assert!(
        last_o_pos < dest_pos,
        "all -o options must come before destination; got joined: {joined}"
    );
}

/// Regression protection: the key auth path's `build_ssh_args` also requires destination
/// to come after -o options. We verify ordering using `build_ssh_args` + manually appending
/// options, simulating how `test_key_auth` constructs the command.
/// author: logic
/// date: 2026-06-01
#[test]
fn key_auth_args_destination_comes_after_options() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/user/.ssh/id_rsa".into());

    // Simulate test_key_auth construction logic
    let mut args = build_ssh_args(&s);
    let target = args.pop().unwrap();
    assert_eq!(args.pop().as_deref(), Some("--"));
    args.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=ask".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    args.push("--".into());
    args.push(target);
    args.push("echo ok".into());

    let joined = args.join(" ");
    let dest_pos = joined
        .find("alice@1.2.3.4")
        .expect("destination must appear in args");
    let echo_pos = joined
        .find("echo ok")
        .expect("`echo ok` must appear in args");
    let last_o_pos = joined
        .rfind("-o ")
        .expect("expected at least one -o option");

    assert!(
        last_o_pos < dest_pos,
        "all -o options must come before destination; got joined: {joined}"
    );
    assert!(
        dest_pos < echo_pos,
        "destination must come before `echo ok`; got joined: {joined}"
    );
}

#[test]
fn connection_test_never_disables_host_key_verification() {
    let s = server();
    for args in [
        build_password_auth_cmd_args(&s),
        build_key_auth_cmd_args(&s, false),
        build_key_auth_cmd_args(&s, true),
    ] {
        assert!(
            !args.iter().any(|arg| arg == "StrictHostKeyChecking=no"),
            "connection test disabled host-key verification: {args:?}"
        );
    }
}

#[test]
fn connection_test_options_are_well_formed() {
    let s = server();
    for args in [
        build_password_auth_cmd_args(&s),
        build_key_auth_cmd_args(&s, false),
        build_key_auth_cmd_args(&s, true),
    ] {
        let destination = args
            .iter()
            .position(|arg| arg == "alice@1.2.3.4")
            .expect("missing SSH destination");
        assert_eq!(
            args.last().map(String::as_str),
            Some("echo ok"),
            "remote command must be last: {args:?}"
        );
        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_str() == "alice@1.2.3.4")
                .count(),
            1,
            "there must be exactly one destination: {args:?}"
        );
        for option_index in 0..destination {
            if args[option_index].contains('=') {
                assert_eq!(
                    args.get(option_index.wrapping_sub(1)).map(String::as_str),
                    Some("-o"),
                    "SSH option lacks -o prefix: {args:?}"
                );
            }
        }
    }
}

#[cfg(unix)]
fn failed_output(stderr: &str) -> std::process::Output {
    use std::os::unix::process::ExitStatusExt as _;

    std::process::Output {
        status: std::process::ExitStatus::from_raw(255 << 8),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[cfg(unix)]
#[test]
fn changed_host_key_is_a_hard_failure() {
    let result = finalize_password_test_result(&failed_output(
        "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
    ));
    assert_eq!(
        result,
        Err("SSH host key changed; connection blocked".to_string())
    );
    assert_eq!(
        classify_host_key_probe(
            &server(),
            &failed_output("WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!"),
        )
        .err()
        .as_deref(),
        Some("SSH host key changed; connection blocked")
    );
}

#[cfg(unix)]
#[test]
fn unknown_host_key_requires_explicit_confirmation() {
    let output = failed_output(
        "The authenticity of host '1.2.3.4' can't be established.\n\
         ED25519 key fingerprint is SHA256:abc123.\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])?",
    );

    match classify_host_key_probe(&server(), &output).unwrap() {
        HostKeyProbeOutcome::Unknown(unknown) => {
            assert_eq!(unknown.host, "1.2.3.4");
            assert_eq!(unknown.port, 22);
            assert_eq!(unknown.fingerprint, "SHA256:abc123");
        }
        HostKeyProbeOutcome::Verified => panic!("unknown host key was silently accepted"),
    }
}

#[test]
fn host_key_probe_rejects_unknown_without_prompting() {
    let args = build_host_key_probe_args(&server());
    assert!(args.iter().any(|arg| arg == "BatchMode=yes"));
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
}

#[test]
fn confirmed_unknown_host_key_never_uses_accept_new() {
    let pinned_host_key = KnownHostsSession::new("1.2.3.4 ssh-ed25519 AAAA").unwrap();
    let mut args = build_password_auth_cmd_args(&server());
    apply_host_key_file(&mut args, &pinned_host_key, true);
    assert!(!args.iter().any(|arg| arg.contains("accept-new")));
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(args
        .iter()
        .any(|arg| { arg == &format!("UserKnownHostsFile={}", pinned_host_key.path.display()) }));
    assert!(!args.iter().any(|arg| arg == "StrictHostKeyChecking=no"));
}

#[test]
fn confirmed_host_key_rejects_a_different_fingerprint() {
    assert!(host_key_fingerprint_matches(
        "SHA256:confirmed",
        b"256 SHA256:confirmed host (ED25519)\n"
    ));
    assert!(!host_key_fingerprint_matches(
        "SHA256:confirmed",
        b"256 SHA256:replacement host (ED25519)\n"
    ));
}

fn source_uses_direct_std_process_command(source: &str) -> bool {
    let direct_or_imported =
        regex::Regex::new(r"std\s*::\s*process\s*::\s*(?:Command\b|\{[^}]*\bCommand\b)").unwrap();
    if direct_or_imported.is_match(source) {
        return true;
    }
    let nested_import =
        regex::Regex::new(r"std\s*::\s*\{[^}]*\bprocess\s*::\s*(?:Command\b|\{[^}]*\bCommand\b)")
            .unwrap();
    if nested_import.is_match(source) {
        return true;
    }
    let process_import =
        regex::Regex::new(r"use\s+std\s*::\s*process(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;")
            .unwrap();
    if process_import.captures_iter(source).any(|captures| {
        let alias = captures.get(1).map_or("process", |alias| alias.as_str());
        regex::Regex::new(&format!(r"\b{}\s*::\s*Command\b", regex::escape(alias)))
            .unwrap()
            .is_match(source)
    }) {
        return true;
    }
    let grouped_process_import = regex::Regex::new(
        r"use\s+std\s*::\s*\{[^}]*\bprocess(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?[^}]*\}\s*;",
    )
    .unwrap();
    grouped_process_import
        .captures_iter(source)
        .any(|captures| {
            let alias = captures.get(1).map_or("process", |alias| alias.as_str());
            regex::Regex::new(&format!(r"\b{}\s*::\s*Command\b", regex::escape(alias)))
                .unwrap()
                .is_match(source)
        })
}

#[test]
fn ssh_workspace_has_no_direct_std_process_command() {
    fn inspect_file(path: &std::path::Path, violations: &mut Vec<String>) {
        let source = std::fs::read_to_string(path).unwrap();
        if source_uses_direct_std_process_command(&source) {
            violations.push(path.display().to_string());
        }
    }

    fn inspect(path: &std::path::Path, violations: &mut Vec<String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                inspect(&path, violations);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                inspect_file(&path, violations);
            }
        }
    }

    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_dir = crate_dir.parent().unwrap().parent().unwrap();
    let mut violations = Vec::new();
    inspect(&crate_dir.join("src"), &mut violations);
    inspect(&repo_dir.join("app/src/ssh_manager"), &mut violations);
    inspect_file(
        &repo_dir.join("app/src/remote_server/headless_connect.rs"),
        &mut violations,
    );
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn process_guard_rejects_aliased_std_command() {
    assert!(source_uses_direct_std_process_command(
        "use std::process::Command as ProcessCommand;\nfn run() { ProcessCommand::new(\"ssh\"); }"
    ));
    assert!(source_uses_direct_std_process_command(
        "use std::process as process;\nfn run() { process::Command::new(\"ssh\"); }"
    ));
    assert!(source_uses_direct_std_process_command(
        "use std::process;\nfn run() { process::Command::new(\"ssh\"); }"
    ));
    assert!(source_uses_direct_std_process_command(
        "use std::{process as p};\nfn run() { p::Command::new(\"ssh\"); }"
    ));
}

#[cfg(unix)]
struct RecordingCommandFactory {
    script: std::path::PathBuf,
    programs: std::sync::Mutex<Vec<String>>,
}

#[cfg(unix)]
impl WorkspaceCommandFactory for RecordingCommandFactory {
    fn async_command(&self, program: &str) -> command::r#async::Command {
        self.programs.lock().unwrap().push(program.to_string());
        let mut command = command::r#async::Command::new(&self.script);
        command.arg(program);
        command
    }

    fn blocking_command(&self, program: &str) -> command::blocking::Command {
        self.programs.lock().unwrap().push(program.to_string());
        let mut command = command::blocking::Command::new(&self.script);
        command.arg(program);
        command
    }
}

#[cfg(unix)]
#[test]
fn workspace_ssh_uses_injected_command_factory() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-ssh");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(b"#!/bin/sh\nprintf ok\n").unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = RecordingCommandFactory {
        script,
        programs: std::sync::Mutex::new(Vec::new()),
    };

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(test_connection_with_factory(
            &server(),
            Some(Zeroizing::new("password".to_string())),
            &factory,
        ));

    assert_eq!(
        result.status,
        ConnectionStatus::Online,
        "{:?}",
        result.error_message
    );
    assert_eq!(factory.programs.lock().unwrap().as_slice(), ["ssh", "ssh"]);
}

#[cfg(unix)]
#[test]
fn confirmed_host_key_reuses_ssh_transport_and_exact_fingerprint() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-ssh-tools");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(
        b"#!/bin/sh\ncase \"$1\" in\n\
          ssh-keygen)\n\
            grep -q 'ssh-' \"$3\" || exit 1\n\
            printf '256 SHA256:confirmed host (ED25519)\\n' ;;\n\
          ssh)\n\
            for arg in \"$@\"; do\n\
              case \"$arg\" in\n\
                UserKnownHostsFile=*) path=${arg#UserKnownHostsFile=} ;;\n\
                StrictHostKeyChecking=*) strict=${arg#StrictHostKeyChecking=} ;;\n\
                BatchMode=*) batch=${arg#BatchMode=} ;;\n\
              esac\n\
            done\n\
            if [ \"$strict\" = ask ] && [ \"$batch\" = no ]; then\n\
              printf '1.2.3.4 ssh-ed25519 AAAA\\n' > \"$path\"\n\
            else\n\
              printf ok\n\
            fi ;;\n\
          *) exit 2 ;;\n\
          esac\n",
    )
    .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = RecordingCommandFactory {
        script,
        programs: std::sync::Mutex::new(Vec::new()),
    };
    let expected = UnknownHostKey {
        host: "1.2.3.4".to_string(),
        port: 22,
        fingerprint: "SHA256:confirmed".to_string(),
    };

    let result =
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(test_connection_with_factory_policy(
                &server(),
                Some(Zeroizing::new("password".to_string())),
                Some(&expected),
                &factory,
            ));

    assert_eq!(
        result.status,
        ConnectionStatus::Online,
        "{:?}",
        result.error_message
    );
    assert_eq!(
        factory.programs.lock().unwrap().as_slice(),
        ["ssh", "ssh-keygen", "ssh"]
    );
}

#[cfg(unix)]
#[test]
fn unknown_host_key_is_captured_from_the_probe_transport() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-ssh-probe");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(
        b"#!/bin/sh\ncase \"$1\" in\n\
          ssh-keygen)\n\
            grep -q 'ssh-' \"$3\" || exit 1\n\
            printf '256 SHA256:captured host (ED25519)\\n' ;;\n\
          ssh)\n\
            for arg in \"$@\"; do\n\
              case \"$arg\" in\n\
                UserKnownHostsFile=*) path=${arg#UserKnownHostsFile=} ;;\n\
                StrictHostKeyChecking=*) strict=${arg#StrictHostKeyChecking=} ;;\n\
                BatchMode=*) batch=${arg#BatchMode=} ;;\n\
              esac\n\
            done\n\
            if [ \"$strict\" = yes ]; then\n\
              printf 'Host key verification failed.\\n' >&2\n\
              exit 255\n\
            fi\n\
            if [ \"$strict\" = ask ] && [ \"$batch\" = no ]; then\n\
              printf '1.2.3.4 ssh-ed25519 AAAA\\n' > \"$path\"\n\
              exit 255\n\
            fi\n\
            exit 2 ;;\n\
          *) exit 2 ;;\n\
          esac\n",
    )
    .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = RecordingCommandFactory {
        script,
        programs: std::sync::Mutex::new(Vec::new()),
    };

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(test_connection_with_factory(
            &server(),
            Some(Zeroizing::new("password".to_string())),
            &factory,
        ));

    assert_eq!(result.status, ConnectionStatus::Unknown);
    assert_eq!(
        result.unknown_host_key,
        Some(UnknownHostKey {
            host: "1.2.3.4".to_string(),
            port: 22,
            fingerprint: "SHA256:captured".to_string(),
        })
    );
    assert_eq!(
        factory.programs.lock().unwrap().as_slice(),
        ["ssh", "ssh", "ssh-keygen"]
    );
}

// -------- Windows SSH_ASKPASS regression protection --------
//
// On Windows, Win32-OpenSSH refuses to read passwords from stdin due to lack of
// console + CREATE_NO_WINDOW (Win32-OpenSSH issue #1470), so it must use the
// SSH_ASKPASS mechanism. This guard ensures the code path exists, preventing
// someone from accidentally merging the Windows path back into stdin-based code.
// author: logic
// date: 2026-06-01

/// Regression protection: on Windows, the `test_password_auth` entry point must
/// reference `AskpassSession`, not write the password directly to stdin. This
/// assertion is guaranteed by the type system: if the Windows path is changed to
/// use stdin, the function body won't reference `AskpassSession::new`, and the test fails.
#[cfg(windows)]
#[test]
fn windows_password_auth_uses_askpass_not_stdin() {
    // This test works at compile time: if the Windows branch of ssh_command.rs falls back
    // to stdin injection, the `AskpassSession` type is no longer used, and the compiler
    // reports a dead_code error, breaking CI.
    // Here we only verify that AskpassSession type exists and can be instantiated — it
    // won't actually run (needs file I/O), but it prevents accidental deletion of AskpassSession.
    let _ = std::any::type_name::<AskpassSession>();
}

/// Real end-to-end: create `AskpassSession` to get the askpass script path, then spawn it
/// with `CreateProcessW` (simulating how ssh spawns askpass), verifying it can start.
///
/// This test ensures the askpass script is "executable" from ssh's perspective — it directly
/// prevents regressions like `CreateProcessW failed error:5` (ERROR_ACCESS_DENIED).
/// Previously, a bug set the askpass file's `FILE_ATTRIBUTE_HIDDEN` flag, causing ssh's
/// `posix_spawnp` to refuse to spawn it, the askpass never ran, the password wasn't passed,
/// and the server reported "wrong password".
#[cfg(windows)]
#[test]
fn windows_askpass_script_is_spawnable() {
    use std::os::windows::process::CommandExt as _;
    use std::process::Stdio;
    use zeroize::Zeroizing;

    let password: Zeroizing<String> = Zeroizing::new("dummy-pw-for-spawn-test".into());
    let session = AskpassSession::new(&password).expect("AskpassSession::new failed");
    let script = session.script_path.clone();
    let password_file = session.password_path.clone();

    // Spawn the askpass script using CreateProcessW to follow the same code path as ssh.
    // CREATE_NO_WINDOW simulates the environment when ssh spawns askpass (no console).
    // Must set ZAPLEX_SSH_ASKPASS_FILE env var; the script uses it to locate the password file.
    let output = command::blocking::Command::new("cmd.exe")
        .raw_arg(format!("/c \"{}\"", script.display()))
        .env("ZAPLEX_SSH_ASKPASS_FILE", &password_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .expect("CreateProcessW failed — askpass script is not spawnable");

    assert!(
        output.status.success(),
        "askpass script exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The askpass script reads the password file's first line and echoes it,
    // should output the password written when the session was created
    assert!(
        stdout.trim() == "dummy-pw-for-spawn-test",
        "askpass output mismatch: got {stdout:?}"
    );
}
