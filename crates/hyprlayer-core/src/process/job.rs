use std::io;

use tokio::process::Child;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub struct JobObject(HANDLE);

unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    pub fn kill_on_close() -> io::Result<Self> {
        Self::with_limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    }

    /// For a terminal: a process started with `CREATE_BREAKAWAY_FROM_JOB` is meant to outlive it.
    pub fn kill_on_close_allowing_breakaway() -> io::Result<Self> {
        Self::with_limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK)
    }

    fn with_limits(flags: u32) -> io::Result<Self> {
        // SAFETY: Null parameters create an unnamed job; Drop owns the handle.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: `info` matches the requested class and outlives the call.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub fn adopt(&self, child: &Child) -> io::Result<()> {
        let handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("child already reaped"))?;
        self.adopt_raw(handle as HANDLE)
    }

    /// The caller keeps `process` open for the duration of the call.
    pub fn adopt_raw(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: the caller guarantees `process` is a live process handle.
        let ok = unsafe { AssignProcessToJobObject(self.0, process) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn terminate(&self) {
        // SAFETY: we own this job handle.
        unsafe { TerminateJobObject(self.0, 1) };
    }

    /// True once every process in the job has exited, and with it closed its handles.
    pub fn is_empty(&self) -> bool {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` matches the requested class and outlives the call.
        let ok = unsafe {
            QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        ok != 0 && info.ActiveProcesses == 0
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: we own this handle and only close it once.
        unsafe { CloseHandle(self.0) };
    }
}
