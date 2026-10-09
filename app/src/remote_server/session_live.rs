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
    /// Ids of all processes currently listed under `/proc`.
    fn pids(&self) -> io::Result<Vec<u32>>;
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

    fn pids(&self) -> io::Result<Vec<u32>> {
        Ok(std::fs::read_dir("/proc")?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .collect())
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
    // An idle prompt is the shell itself in the foreground.
    let foreground = u32::try_from(shell.foreground_group)
        .ok()
        .filter(|group| *group != shell_pid)
        .and_then(|group| foreground_process(procfs, group, shell.session));
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

/// A live process of the terminal's foreground group: its leader, or, once a
/// pipeline's leader has exited (`cat app.log | less`), the newest remaining
/// member, usually the stage the user is looking at. A group outside the
/// shell's session cannot be this terminal's job (PID reuse).
fn foreground_process(procfs: &impl PtyProcfs, group: u32, session: i32) -> Option<u32> {
    let in_foreground_job = |pid: u32| {
        read_stat(procfs, pid).is_some_and(|stat| {
            !stat.exited && stat.session == session && u32::try_from(stat.group).ok() == Some(group)
        })
    };
    if in_foreground_job(group) {
        return Some(group);
    }
    procfs
        .pids()
        .ok()?
        .into_iter()
        .filter(|pid| in_foreground_job(*pid))
        .max()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PtyProcessStat {
    exited: bool,
    group: i32,
    session: i32,
    foreground_group: i32,
}

fn read_stat(procfs: &impl PtyProcfs, pid: u32) -> Option<PtyProcessStat> {
    let contents = procfs
        .read(&PathBuf::from(format!("/proc/{pid}/stat")))
        .ok()?;
    parse_stat(&String::from_utf8_lossy(&contents))
}

/// Parses state (field 3), process group (field 5), session (field 6) and
/// terminal foreground group (field 8). The command name in field 2 may
/// contain spaces and parentheses, so fields are counted after its last
/// closing parenthesis.
fn parse_stat(contents: &str) -> Option<PtyProcessStat> {
    let fields: Vec<&str> = contents[contents.rfind(')')? + 1..]
        .split_whitespace()
        .collect();
    Some(PtyProcessStat {
        exited: matches!(*fields.first()?, "Z" | "X"),
        group: fields.get(2)?.parse().ok()?,
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

/// The executable's base name, or the script's for a known interpreter. Only
/// file names are reported: inline code, module names and option operands
/// are arguments, so the interpreter's own name stands in for them.
fn command_name_from_cmdline(cmdline: &[u8]) -> Option<String> {
    let mut args = cmdline
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(String::from_utf8_lossy);
    // A program that rewrites its process title puts its arguments into
    // argv[0] ("npm run dev"); only the first word names the program.
    let executable = args
        .next()?
        .split_whitespace()
        .next()
        .and_then(base_name)
        .map(|name| name.trim_end_matches(':').to_string())
        .filter(|name| is_plain_file_name(name))?;
    if !INTERPRETERS.contains(&executable.as_str()) {
        return Some(executable);
    }
    let script = args
        .take_while(|arg| !runs_inline_code(arg))
        .find(|arg| !arg.starts_with('-'))
        .and_then(|arg| script_name(&arg));
    Some(script.unwrap_or(executable))
}

/// Whether an interpreter option turns the next operand into program text
/// (`python -c`, `node -e`/`-p`/`--eval`, `perl -e`/`-E`, `ruby -e`).
fn runs_inline_code(arg: &str) -> bool {
    match arg.strip_prefix("--") {
        Some(long) => matches!(long.split('=').next(), Some("eval" | "print")),
        None => arg.strip_prefix('-').is_some_and(|flags| {
            flags
                .chars()
                .any(|flag| matches!(flag, 'c' | 'e' | 'E' | 'p'))
        }),
    }
}

/// A script's name without its extension. Only a path (`/usr/bin/claude`,
/// `./build.py`) or a file with a script extension counts as a script.
fn script_name(arg: &str) -> Option<String> {
    let name = base_name(arg)?;
    let is_script = arg.contains('/') || has_script_extension(&name);
    (is_script && is_plain_file_name(&name)).then(|| strip_script_extension(&name))
}

fn base_name(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '@'))
}

fn has_script_extension(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, extension)| !stem.is_empty() && SCRIPT_EXTENSIONS.contains(&extension))
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
