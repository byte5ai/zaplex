use std::collections::HashMap;

use super::*;

/// In-memory `/proc` with stat lines built from (session, tpgid).
#[derive(Default)]
struct FakeProcfs {
    files: HashMap<PathBuf, Vec<u8>>,
    links: HashMap<PathBuf, PathBuf>,
}

impl FakeProcfs {
    fn process(mut self, pid: u32, session: i32, tpgid: i32, cwd: &str, cmdline: &[&str]) -> Self {
        // Field 2 deliberately contains a space and a parenthesis.
        self.files.insert(
            PathBuf::from(format!("/proc/{pid}/stat")),
            format!("{pid} (odd) name) S 1 {pid} {session} 34816 {tpgid} 4194560 0 0 0 0")
                .into_bytes(),
        );
        self.files.insert(
            PathBuf::from(format!("/proc/{pid}/cmdline")),
            cmdline.join("\0").into_bytes(),
        );
        self.links.insert(
            PathBuf::from(format!("/proc/{pid}/cwd")),
            PathBuf::from(cwd),
        );
        self
    }

    fn without_cwd(mut self, pid: u32) -> Self {
        self.links
            .remove(&PathBuf::from(format!("/proc/{pid}/cwd")));
        self
    }
}

impl PtyProcfs for FakeProcfs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        self.links
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))
    }
}

#[test]
fn foreground_job_reports_its_directory_and_executable_name() {
    let procfs = FakeProcfs::default()
        .process(100, 100, 200, "/home/dev", &["-bash"])
        .process(
            200,
            100,
            200,
            "/srv/projects/api",
            &["/home/dev/.local/bin/claude", "--resume", "secret-token"],
        );

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/srv/projects/api");
    assert_eq!(live.foreground_command, "claude");
}

#[test]
fn idle_prompt_reports_the_shell_directory_without_a_command() {
    let procfs = FakeProcfs::default().process(100, 100, 100, "/srv/projects/api", &["-zsh"]);

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/srv/projects/api");
    assert_eq!(live.foreground_command, "");
}

#[test]
fn interpreted_cli_reports_its_script_name_without_arguments() {
    let procfs = FakeProcfs::default()
        .process(100, 100, 300, "/srv/projects/api", &["bash"])
        .process(
            300,
            100,
            300,
            "/srv/projects/api",
            &[
                "node",
                "--no-warnings",
                "/usr/lib/node_modules/@openai/codex/bin/codex.js",
                "exec",
                "private prompt",
            ],
        );

    assert_eq!(
        read_session_live_metadata(&procfs, 100).foreground_command,
        "codex"
    );
}

#[test]
fn unreadable_foreground_directory_falls_back_to_the_shell() {
    let procfs = FakeProcfs::default()
        .process(100, 100, 400, "/srv/projects/api", &["bash"])
        .process(400, 100, 400, "/root", &["sudo", "-i"])
        .without_cwd(400);

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/srv/projects/api");
    assert_eq!(live.foreground_command, "sudo");
}

#[test]
fn foreground_group_from_another_session_is_ignored() {
    // The recorded foreground group id now belongs to an unrelated process.
    let procfs = FakeProcfs::default()
        .process(100, 100, 500, "/srv/projects/api", &["bash"])
        .process(500, 900, 500, "/var/lib/other", &["unrelated-daemon"]);

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/srv/projects/api");
    assert_eq!(live.foreground_command, "");
}

#[test]
fn a_vanished_shell_reports_nothing() {
    assert_eq!(
        read_session_live_metadata(&FakeProcfs::default(), 100),
        SessionLiveMetadata::default()
    );
}
