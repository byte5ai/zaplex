use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::process::CommandExt as _;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use lazy_static::lazy_static;

#[derive(Debug, thiserror::Error)]
pub enum JobObjectError {
    #[error("Failed to create job: {0}")]
    CreateFailed(std::io::Error),

    #[error("Failed to assign process to job: {0}")]
    AssignFailed(std::io::Error),

    #[error("Failed to set info for job: {0}")]
    SetInfoFailed(std::io::Error),

    #[error("Failed to get info for job: {0}")]
    GetInfoFailed(std::io::Error),

    #[error("Failed to terminate job: {0}")]
    TerminateFailed(std::io::Error),

    #[error(transparent)]
    Other(anyhow::Error),
}

impl From<win32job::JobError> for JobObjectError {
    fn from(error: win32job::JobError) -> Self {
        match error {
            win32job::JobError::CreateFailed(e) => JobObjectError::CreateFailed(e),
            win32job::JobError::AssignFailed(e) => JobObjectError::AssignFailed(e),
            win32job::JobError::SetInfoFailed(e) => JobObjectError::SetInfoFailed(e),
            win32job::JobError::GetInfoFailed(e) => JobObjectError::GetInfoFailed(e),
            _ => JobObjectError::Other(error.into()),
        }
    }
}

/// We use Job Objects to handle killing child processes when the program is
/// closed. This builder struct is used to configure a Job Object and associate it
/// with processes. Processes associated with a job will be killed when the handle
/// to the job is dropped at the end of the program's lifecycle.
///
/// NOTE: We've encountered issues with assigning some processes to jobs that
/// already contain other processes (i.e. `pwsh.exe`), so we only want to
/// assign a single process to a job.
///
/// For more information on Job Objects, see:
/// https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
#[derive(Debug, Default)]
pub struct JobObject {
    assign_current_process: bool,
    assign_process: Option<isize>,
    kill_children_on_close: bool,
}

impl JobObject {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns the current process to the Job Object. This can be used to ensure
    /// that children of the current process are associated with the job.
    pub fn assign_current_process(mut self) -> Self {
        self.assign_current_process = true;
        self
    }

    /// Assigns a process to the Job Object. This process will be killed when the
    /// current process is closed.
    pub fn assign_process(mut self, process: isize) -> Self {
        self.assign_process = Some(process);
        self
    }

    /// Configures the Job Object so children of the assigned processes are
    /// automatically associated with the job, thus killing them along with
    /// their parents on close.
    pub fn kill_children_on_close(mut self) -> Self {
        self.kill_children_on_close = true;
        self
    }

    fn build(self) -> Result<win32job::Job, win32job::JobError> {
        let job = win32job::Job::create()?;

        let mut info = job.query_extended_limit_info()?;
        // Mark the job as "kill on job close", so all processes associated with
        // the job are killed when the handle to the job is closed.
        info.limit_kill_on_job_close();
        info.limit_breakaway_ok();
        if !self.kill_children_on_close {
            info.limit_silent_breakaway_ok();
        }
        job.set_extended_limit_info(&info)?;

        if self.assign_current_process {
            job.assign_current_process()?;
        }
        if let Some(process) = self.assign_process {
            job.assign_process(process)?;
        }

        Ok(job)
    }

    fn create_internal(self) -> Result<(), win32job::JobError> {
        Box::leak(Box::new(self.build()?));
        Ok(())
    }

    /// Creates a new Job Object and assigns any specified processes to it. The
    /// handle to the job is leaked to ensure that the job lives for the lifetime
    /// of the program.
    pub fn create(self) -> Result<(), JobObjectError> {
        self.create_internal().map_err(Into::into)
    }
}

/// An owned Job Object for one command tree. Keeping this handle alive keeps the
/// root process and every inheriting descendant in one controllable unit.
#[derive(Debug)]
struct ProcessGroup {
    job: win32job::Job,
}

impl ProcessGroup {
    fn for_process(process: isize) -> Result<Self, JobObjectError> {
        let job = JobObject::new()
            .assign_process(process)
            .kill_children_on_close()
            .build()?;
        Ok(Self { job })
    }

    fn terminate(&self) -> Result<(), JobObjectError> {
        // win32job and the workspace currently use adjacent `windows` crate
        // versions, so call the stable Win32 ABI with the raw handle instead of
        // attempting to convert between their distinct HANDLE wrapper types.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            #[link_name = "TerminateJobObject"]
            fn terminate_job_object(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
        }

        // SAFETY: `self.job` owns a valid Job Object handle for this call and
        // remains alive for its entire duration.
        if unsafe { terminate_job_object(self.job.handle() as *mut std::ffi::c_void, 1) } == 0 {
            return Err(JobObjectError::TerminateFailed(
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn is_empty(&self) -> Result<bool, JobObjectError> {
        Ok(self.job.query_process_id_list()?.is_empty())
    }
}

lazy_static! {
    static ref PROCESS_GROUPS: Mutex<HashMap<u32, Arc<ProcessGroup>>> = Mutex::new(HashMap::new());
}

pub(super) fn register_process_group(pid: u32, process: isize) -> Result<(), JobObjectError> {
    let group = Arc::new(ProcessGroup::for_process(process)?);
    let replaced = PROCESS_GROUPS.lock().unwrap().insert(pid, group);
    if replaced.is_some() {
        log::warn!("Replaced a stale Windows process group for reused pid {pid}");
    }
    Ok(())
}

/// Terminates the root process and all descendants assigned to its Job Object.
/// Returns `false` when the group was already released after confirmed exit.
pub fn terminate_process_group(pid: u32) -> Result<bool, JobObjectError> {
    let group = PROCESS_GROUPS.lock().unwrap().get(&pid).cloned();
    let Some(group) = group else {
        return Ok(false);
    };
    group.terminate()?;
    Ok(true)
}

/// Whether the Job Object no longer contains any live processes.
pub fn process_group_is_empty(pid: u32) -> Result<bool, JobObjectError> {
    let group = PROCESS_GROUPS.lock().unwrap().get(&pid).cloned();
    match group {
        Some(group) => group.is_empty(),
        None => Ok(true),
    }
}

/// Releases the owned Job Object after the caller has observed process-tree exit.
pub fn release_process_group(pid: u32) {
    PROCESS_GROUPS.lock().unwrap().remove(&pid);
}

pub fn init() {
    if let Err(e) = JobObject::new()
        .kill_children_on_close()
        .assign_current_process()
        .create()
    {
        log::error!("Failed to create job object for the program: {e:#}");
    }
}

pub trait CommandExt {
    /// Append literal text to the command line without any quoting or escaping.
    ///
    /// This is useful for passing arguments to `cmd.exe /c`, which doesn't follow
    /// `CommandLineToArgvW` escaping rules.
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self;
}

use async_process::windows::CommandExt as _;

impl CommandExt for crate::blocking::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}

impl CommandExt for crate::r#async::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}
