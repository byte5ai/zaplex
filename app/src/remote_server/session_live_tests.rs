use std::collections::HashMap;

use super::*;

/// In-memory `/proc` with stat lines built from (group, session, tpgid).
#[derive(Default)]
struct FakeProcfs {
    files: HashMap<PathBuf, Vec<u8>>,
    links: HashMap<PathBuf, PathBuf>,
}

impl FakeProcfs {
    /// A process leading its own process group.
    fn process(self, pid: u32, session: i32, tpgid: i32, cwd: &str, cmdline: &[&str]) -> Self {
        self.member(pid, pid, session, tpgid, cwd, cmdline)
    }

    fn member(
        mut self,
        pid: u32,
        group: u32,
        session: i32,
        tpgid: i32,
        cwd: &str,
        cmdline: &[&str],
    ) -> Self {
        // Field 2 deliberately contains a space and a parenthesis.
        self.files.insert(
            PathBuf::from(format!("/proc/{pid}/stat")),
            format!("{pid} (odd) name) S 1 {group} {session} 34816 {tpgid} 4194560 0 0 0 0")
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

    /// Turns `pid` into a zombie: its stat stays, everything else is gone.
    fn exited(mut self, pid: u32) -> Self {
        let stat = PathBuf::from(format!("/proc/{pid}/stat"));
        let line = String::from_utf8(self.files[&stat].clone()).unwrap();
        self.files
            .insert(stat, line.replacen(") S ", ") Z ", 1).into_bytes());
        self.files
            .insert(PathBuf::from(format!("/proc/{pid}/cmdline")), Vec::new());
        self.without_cwd(pid)
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

    fn pids(&self) -> io::Result<Vec<u32>> {
        Ok(self
            .files
            .keys()
            .filter(|path| path.ends_with("stat"))
            .filter_map(|path| path.parent()?.file_name()?.to_str()?.parse::<u32>().ok())
            .collect())
    }
}

fn command_of(cmdline: &[&str]) -> String {
    let procfs = FakeProcfs::default()
        .process(100, 100, 700, "/srv/projects/api", &["bash"])
        .process(700, 100, 700, "/srv/projects/api", cmdline);
    read_session_live_metadata(&procfs, 100).foreground_command
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

#[test]
fn inline_interpreter_code_never_leaves_the_host() {
    for cmdline in [
        &[
            "python3",
            "-c",
            "import time; token='SECRET'; time.sleep(600)",
        ][..],
        &["python3", "-Ic", "open('/etc/secret.py').read()"],
        &["node", "-e", "require('/srv/app/secret.js')"],
        &["node", "--eval=console.log(1)", "/srv/app/server.js"],
        &["node", "-p", "process.env.TOKEN"],
        &["perl", "-ne", "print if /secret/", "/var/log/app.log"],
        &["ruby", "-e", "puts ENV['TOKEN']"],
    ] {
        let interpreter = cmdline[0];
        assert_eq!(command_of(cmdline), interpreter, "{cmdline:?}");
    }
}

#[test]
fn interpreter_operands_that_are_not_script_files_stay_on_the_host() {
    // Module names, option values and subcommands are arguments too.
    assert_eq!(
        command_of(&["python3", "-m", "http.server", "8000"]),
        "python3"
    );
    assert_eq!(
        command_of(&["python3", "-W", "ignore", "secret"]),
        "python3"
    );
    assert_eq!(
        command_of(&["deno", "eval", "Deno.env.get('TOKEN')"]),
        "deno"
    );
    assert_eq!(
        command_of(&["python3", "-u", "./build.py", "--token", "x"]),
        "build"
    );
    assert_eq!(
        command_of(&["node", "/home/dev/.local/bin/claude", "--resume"]),
        "claude"
    );
}

#[test]
fn rewritten_process_title_reports_only_the_program_name() {
    assert_eq!(command_of(&["npm run dev --token=SECRET"]), "npm");
    assert_eq!(command_of(&["sshd: dev@pts/3"]), "sshd");
}

#[test]
fn pipeline_whose_leader_exited_reports_a_remaining_stage() {
    // `cat app.log | less`: cat (the group leader) is done, less still runs.
    let procfs = FakeProcfs::default()
        .process(100, 100, 600, "/srv/projects/api", &["bash"])
        .process(600, 100, 600, "/srv/projects/api", &["cat", "app.log"])
        .exited(600)
        .member(601, 600, 100, 600, "/var/log/app", &["less", "-R"])
        // Same group id, but another kernel session: never this job.
        .member(9000, 600, 900, 600, "/var/lib/other", &["unrelated-daemon"]);

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/var/log/app");
    assert_eq!(live.foreground_command, "less");
}

#[test]
fn reaped_pipeline_leader_without_survivors_reports_the_shell() {
    let procfs = FakeProcfs::default().process(100, 100, 600, "/srv/projects/api", &["bash"]);

    let live = read_session_live_metadata(&procfs, 100);

    assert_eq!(live.cwd, "/srv/projects/api");
    assert_eq!(live.foreground_command, "");
}
