#[cfg(windows)]
mod windows {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use futures_lite::future;
    use sysinfo::{Pid, ProcessesToUpdate, System};
    use warpui::r#async::FutureExt as _;

    use super::super::{CommandExecutor as _, ExecuteCommandOptions, LocalCommandExecutor};
    use crate::terminal::shell::ShellType;

    async fn wait_for_recorded_pids(path: &Path) -> Vec<u32> {
        loop {
            if let Ok(contents) = std::fs::read_to_string(path) {
                let pids: Vec<_> = contents
                    .lines()
                    .filter_map(|line| line.trim().parse().ok())
                    .collect();
                if pids.len() == 2 {
                    return pids;
                }
            }
            async_io::Timer::after(Duration::from_millis(10)).await;
        }
    }

    fn process_is_running(pid: u32) -> bool {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), false);
        system.process(Pid::from_u32(pid)).is_some()
    }

    #[test]
    fn cancel_active_commands_terminates_child_and_grandchild() {
        futures_lite::future::block_on(async {
            let tempdir = tempfile::tempdir().unwrap();
            let pid_file = tempdir.path().join("process-tree-pids.txt");
            let escaped_pid_file = pid_file.display().to_string().replace('\'', "''");
            let command = format!(
                "powershell.exe -NoProfile -NonInteractive -Command \"$grandchild = \
                 Start-Process powershell.exe -ArgumentList @('-NoProfile','-NonInteractive',\
                 '-Command','Start-Sleep -Seconds 300') -PassThru; \
                 Set-Content -LiteralPath '{escaped_pid_file}' -Value @($PID, $grandchild.Id); \
                 Wait-Process -Id $grandchild.Id\""
            );
            let executor = Arc::new(LocalCommandExecutor::new(None, ShellType::PowerShell));
            let execute = executor.execute_local_command(
                &command,
                None,
                None,
                ExecuteCommandOptions::default(),
            );
            let cancel = {
                let executor = executor.clone();
                async move {
                    let pids = wait_for_recorded_pids(&pid_file).await;
                    assert!(pids.iter().copied().all(process_is_running));
                    executor.cancel_active_commands();
                    pids
                }
            };

            let (_, pids) = future::zip(execute, cancel)
                .with_timeout(Duration::from_secs(10))
                .await
                .expect("cancelled process tree should exit within ten seconds");

            assert!(pids.iter().copied().all(|pid| !process_is_running(pid)));
            assert!(executor.spawned_children_pids.lock().is_empty());
        });
    }
}
