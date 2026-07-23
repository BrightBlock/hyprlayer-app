//! Windows job objects — tie a spawned process *tree* to a handle we hold.
//!
//! Needed because on Windows the pid we get back from a spawn is often not the
//! process we care about. A resolved CLI is frequently an npm `.cmd` shim
//! (OpenCode installs `opencode.cmd`, no `.exe` on PATH), and
//! `std::process::Command` runs `.cmd`/`.bat` through `cmd.exe` — so our child
//! is the wrapper and the real program is *its* child. Killing our child, by
//! `kill_on_drop` or otherwise, reaps the wrapper and leaves the program running.
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` fixes that at the kernel level: every
//! process in the job dies when the last handle to it closes. The kernel closes
//! our handle however we go away — clean exit, `process::exit`, panic,
//! `TerminateProcess` — so teardown does not depend on a destructor running.

use std::io;

use tokio::process::Child;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub struct JobObject(HANDLE);

// A job object is a plain kernel handle with no thread affinity.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    /// A new job that kills everything in it once this handle drops.
    pub fn kill_on_close() -> io::Result<Self> {
        // SAFETY: null attributes/name are the documented "unnamed, default
        // security" form; we own the returned handle and close it in Drop.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` matches the information class and outlives the call.
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

    /// Put `child` in the job. Anything it spawns afterwards joins automatically,
    /// which is the point: the `cmd.exe` wrapper is what we can name, the program
    /// underneath is what we actually need to kill.
    ///
    /// Assignment happens right after the spawn returns, so a wrapper that
    /// launched its own child in that window would escape. Windows offers no way
    /// to spawn directly into a job, and the window here is one syscall wide
    /// against a `cmd.exe` that still has to start and parse a batch file.
    pub fn adopt(&self, child: &Child) -> io::Result<()> {
        let handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("child already reaped"))?;
        // SAFETY: `handle` is the live child's process handle, owned by `child`,
        // which outlives this call.
        let ok = unsafe { AssignProcessToJobObject(self.0, handle as HANDLE) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // Closing the last handle is what kills the tree.
        // SAFETY: we own this handle and only close it once.
        unsafe { CloseHandle(self.0) };
    }
}
