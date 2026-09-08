use super::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn fake_process(script: &str) -> (tempfile::TempDir, JsonLineProcess) {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-subscription-agent");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let launch = ProcessLaunch {
        program: executable,
        args: Vec::new(),
        environment: Vec::new(),
        unset_environment: Vec::new(),
        working_directory: directory.path().to_path_buf(),
        location: ProcessLocation::Local,
    };
    let process = JsonLineProcess::spawn(&launch).unwrap();
    (directory, process)
}

#[cfg(unix)]
#[test]
fn process_exits_gracefully_when_protocol_input_closes() {
    futures_lite::future::block_on(async {
        let (_directory, mut process) = fake_process("cat >/dev/null");

        let termination = process
            .terminate_with_timeouts(Duration::from_secs(1), Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(termination, ProcessTermination::Graceful);
    });
}

#[cfg(unix)]
#[test]
fn process_exit_timeout_escalates_and_reaps_the_child() {
    futures_lite::future::block_on(async {
        let (_directory, mut process) = fake_process("while :; do :; done");
        let pid = process.child.id();

        let termination = process
            .terminate_with_timeouts(Duration::from_millis(20), Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(termination, ProcessTermination::Forced);
        assert!(process.child.try_status().unwrap().is_some());
        assert_eq!(
            std::path::Path::new(&format!("/proc/{pid}")).exists(),
            false
        );
    });
}
