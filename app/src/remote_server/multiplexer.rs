//! Typed discovery of existing tmux/byobu sessions on a daemon host.
//!
//! Discovery executes each backend directly with fixed argv. Session names are
//! parsed as opaque data and cross the protocol boundary in typed fields; they
//! are never interpolated into a scan command.

use std::io;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use remote_server::proto::{MultiplexerKind, MultiplexerSessionInfo, MultiplexerSessionList};

const TMUX_FORMAT: &str = "#{session_name}\t#{session_windows}\t#{session_attached}";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

async fn command_output(program: &str, args: &[&str]) -> io::Result<Output> {
    tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        command::r#async::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{program} session discovery timed out"),
        )
    })?
}

fn byobu_config_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BYOBU_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let legacy = home.join(".byobu");
    if legacy.is_dir() {
        return Some(legacy);
    }
    Some(
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("byobu"),
    )
}

fn parse_byobu_backend(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("BYOBU_BACKEND=")
            .map(str::trim)
            .filter(|backend| matches!(*backend, "tmux" | "screen"))
            .map(ToOwned::to_owned)
    })
}

fn configured_byobu_backend() -> Option<String> {
    let contents = std::fs::read_to_string(byobu_config_dir()?.join("backend")).ok()?;
    parse_byobu_backend(&contents)
}

fn output_text(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.stdout.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    text.trim().to_owned()
}

fn parse_tmux_sessions(
    bytes: &[u8],
    kind: MultiplexerKind,
) -> Result<Vec<MultiplexerSessionInfo>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "tmux returned non-UTF-8 session data".to_string())?;
    let mut sessions = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.split('\t');
        let (Some(name), Some(windows), Some(attached_clients), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("tmux returned malformed session data".to_string());
        };
        if name.is_empty() {
            return Err("tmux returned an empty session name".to_string());
        }
        let windows = windows
            .parse::<u32>()
            .map_err(|_| "tmux returned an invalid window count".to_string())?;
        let attached_clients = attached_clients
            .parse::<u32>()
            .map_err(|_| "tmux returned an invalid client count".to_string())?;
        sessions.push(MultiplexerSessionInfo {
            kind: kind.into(),
            target: name.to_owned(),
            name: name.to_owned(),
            windows,
            attached_clients,
        });
    }
    Ok(sessions)
}

fn parse_byobu_screen_sessions(bytes: &[u8]) -> Result<Vec<MultiplexerSessionInfo>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "byobu-screen returned non-UTF-8 session data".to_string())?;
    let mut sessions = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(target) = line.split_whitespace().next() else {
            continue;
        };
        let Some((pid, name)) = target.split_once('.') else {
            continue;
        };
        if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) || name.is_empty() {
            continue;
        }
        let attached_clients = if line.contains("(Attached)") { 1 } else { 0 };
        sessions.push(MultiplexerSessionInfo {
            kind: MultiplexerKind::ByobuScreen.into(),
            target: target.to_owned(),
            name: name.to_owned(),
            windows: 0,
            attached_clients,
        });
    }
    if sessions.is_empty() {
        return Err("byobu-screen returned no recognizable session data".to_string());
    }
    Ok(sessions)
}

fn tmux_server_is_byobu(output: &Output) -> bool {
    output.status.success()
        && std::str::from_utf8(&output.stdout)
            .is_ok_and(|text| text.lines().any(|line| line == "BYOBU_BACKEND=tmux"))
}

/// Enumerate all attachable tmux sessions plus byobu-screen sessions when the
/// user's configured byobu backend is GNU screen.
pub async fn discover_multiplexer_sessions() -> MultiplexerSessionList {
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();

    let byobu_tmux =
        match command_output("tmux", &["show-environment", "-g", "BYOBU_BACKEND"]).await {
            Ok(output) => tmux_server_is_byobu(&output),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                warnings.push(format!("Could not inspect tmux environment: {error}"));
                false
            }
        };

    match command_output("tmux", &["list-sessions", "-F", TMUX_FORMAT]).await {
        Ok(output) if output.status.success() => {
            let kind = if byobu_tmux {
                MultiplexerKind::ByobuTmux
            } else {
                MultiplexerKind::Tmux
            };
            match parse_tmux_sessions(&output.stdout, kind) {
                Ok(found) => sessions.extend(found),
                Err(error) => warnings.push(error),
            }
        }
        Ok(output) => {
            let detail = output_text(&output);
            if !detail.contains("no server running on") {
                warnings.push(if detail.is_empty() {
                    "tmux session scan failed".to_string()
                } else {
                    format!("tmux session scan failed: {detail}")
                });
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => warnings.push(format!("Could not run tmux session scan: {error}")),
    }

    if configured_byobu_backend().as_deref() == Some("screen") {
        match command_output("screen", &["-ls"]).await {
            Ok(output) if output.status.success() => {
                match parse_byobu_screen_sessions(&output.stdout) {
                    Ok(found) => sessions.extend(found),
                    Err(error) => warnings.push(error),
                }
            }
            Ok(output) => {
                let detail = output_text(&output);
                if !detail.contains("No Sockets found in")
                    && !detail.contains("No screen session found.")
                {
                    warnings.push(if detail.is_empty() {
                        "byobu-screen session scan failed".to_string()
                    } else {
                        format!("byobu-screen session scan failed: {detail}")
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                warnings.push(format!("Could not run byobu-screen session scan: {error}"))
            }
        }
    }

    sessions.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.target.cmp(&right.target))
    });
    MultiplexerSessionList { sessions, warnings }
}

#[cfg(test)]
#[path = "multiplexer_tests.rs"]
mod tests;
