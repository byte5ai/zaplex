use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows::Win32::{
    Foundation::HANDLE,
    System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

use super::release_process_group;
use crate::r#async::Command;

struct ReleaseGroup(u32);

impl Drop for ReleaseGroup {
    fn drop(&mut self) {
        release_process_group(self.0);
    }
}

#[test]
fn exited_root_identity_survives_consumed_child_until_group_release() {
    futures_lite::future::block_on(async {
        let child = Command::new_with_process_group("cmd.exe")
            .args(["/D", "/C", "exit", "/B", "42"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let release = ReleaseGroup(pid);
        // output() consumes Child and closes its process handle after collecting the exit status.
        assert_eq!(child.output().await.unwrap().status.code(), Some(42));

        // The registry must still keep this exact exited process addressable by its PID.
        // SAFETY: OpenProcess returns an independently owned handle on success.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .expect("group cleanup must pin the exited root process identity");
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let mut exit_code = 0;
        // SAFETY: the owned handle remains valid and exit_code points to writable storage.
        unsafe { GetExitCodeProcess(HANDLE(handle.as_raw_handle()), &mut exit_code) }.unwrap();
        assert_eq!(exit_code, 42);
        drop(handle);
        drop(release);
    });
}
