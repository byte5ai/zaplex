//! Point-in-time facts about a daemon-hosted PTY that the daemon reads from
//! `/proc` when it answers `ListSessions`: the working directory and the
//! executable name of the terminal's foreground job.
//!
//! The session shell is a session leader with the PTY as its controlling
//! terminal, so field 8 (`tpgid`) of its `/proc/<pid>/stat` names the PTY's
//! foreground process group. Only executable names leave the host, never
//! command-line arguments.

use std::io;
use std::path::{Path, PathBuf};

use remote_server::proto::SessionLiveMetadata;

/// Interpreters whose first script argument names the actual command.
const INTERPRETERS: &[&str] = &[
    "node", "nodejs", "bun", "deno", "python", "python3", "ruby", "perl",
];
const SCRIPT_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "py", "rb", "pl"];

pub(crate) trait PtyProcfs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn read_link(&self, path: &Path) -> io::Result<PathBuf>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RealPtyProcfs;

impl PtyProcfs for RealPtyProcfs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::read_link(path)
    }
}

/// Reads the live metadata of the PTY whose session shell is `shell_pid`.
/// Unreadable facts stay empty; nothing is guessed.
pub(crate) fn read_session_live_metadata(
    procfs: &impl PtyProcfs,
    shell_pid: u32,
) -> SessionLiveMetadata {
    let Some(shell) = read_stat(procfs, shell_pid) else {
        return SessionLiveMetadata::default();
    };
    // An idle prompt is the shell itself in the foreground. A process group
    // outside the shell's session cannot be this terminal's job (PID reuse).
    let foreground = u32::try_from(shell.foreground_group)
        .ok()
        .filter(|pid| *pid != shell_pid)
        .filter(|pid| read_stat(procfs, *pid).is_some_and(|stat| stat.session == shell.session));
    let cwd = foreground
        .and_then(|pid| read_cwd(procfs, pid))
        .or_else(|| read_cwd(procfs, shell_pid))
        .unwrap_or_default();
    let foreground_command = foreground
        .and_then(|pid| foreground_command_name(procfs, pid))
        .unwrap_or_default();
    SessionLiveMetadata {
        cwd,
        foreground_command,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PtyProcessStat {
    session: i32,
    foreground_group: i32,
}

fn read_stat(procfs: &impl PtyProcfs, pid: u32) -> Option<PtyProcessStat> {
    let contents = procfs
        .read(&PathBuf::from(format!("/proc/{pid}/stat")))
        .ok()?;
    parse_stat(&String::from_utf8_lossy(&contents))
}

/// Parses session (field 6) and terminal foreground group (field 8). The
/// command name in field 2 may contain spaces and parentheses, so fields are
/// counted after its last closing parenthesis.
fn parse_stat(contents: &str) -> Option<PtyProcessStat> {
    let fields: Vec<&str> = contents[contents.rfind(')')? + 1..]
        .split_whitespace()
        .collect();
    Some(PtyProcessStat {
        session: fields.get(3)?.parse().ok()?,
        foreground_group: fields.get(5)?.parse().ok()?,
    })
}

fn read_cwd(procfs: &impl PtyProcfs, pid: u32) -> Option<String> {
    let cwd = procfs
        .read_link(&PathBuf::from(format!("/proc/{pid}/cwd")))
        .ok()?;
    let cwd = cwd.to_str()?;
    (!cwd.is_empty()).then(|| cwd.to_string())
}

fn foreground_command_name(procfs: &impl PtyProcfs, pid: u32) -> Option<String> {
    procfs
        .read(&PathBuf::from(format!("/proc/{pid}/cmdline")))
        .ok()
        .and_then(|cmdline| command_name_from_cmdline(&cmdline))
        .or_else(|| {
            let comm = procfs
                .read(&PathBuf::from(format!("/proc/{pid}/comm")))
                .ok()?;
            let comm = String::from_utf8_lossy(&comm).trim().to_string();
            (!comm.is_empty()).then_some(comm)
        })
}

/// The executable's base name, or the script's for a known interpreter.
fn command_name_from_cmdline(cmdline: &[u8]) -> Option<String> {
    let mut args = cmdline
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(String::from_utf8_lossy);
    let executable = base_name(&args.next()?)?;
    if !INTERPRETERS.contains(&executable.as_str()) {
        return Some(executable);
    }
    let script = args
        .find(|arg| !arg.starts_with('-'))
        .and_then(|script| base_name(&script));
    Some(script.map_or(executable, |script| strip_script_extension(&script)))
}

fn base_name(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn strip_script_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && SCRIPT_EXTENSIONS.contains(&extension) => {
            stem.to_string()
        }
        _ => name.to_string(),
    }
}

#[cfg(test)]
#[path = "session_live_tests.rs"]
mod tests;
