//! Kill an A3S Use registry command and every descendant it started.
//!
//! `kill_on_drop` ends only the direct child. Unix puts that child in its own
//! process group. Windows suspends it, assigns a Job Object, then resumes it
//! so a descendant cannot start outside the job.

use tokio::process::Command;

pub(super) fn configure(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(windows)]
    {
        // Suspend until the Job Object owns the process. CREATE_NO_WINDOW
        // keeps the captured registry CLI from opening a console.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = command;
}

pub(super) struct RegistryProcessGroup {
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
}

impl RegistryProcessGroup {
    pub(super) fn attach(child: &tokio::process::Child) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                process_group: child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()),
            })
        }
        #[cfg(windows)]
        {
            windows::attach(child)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            Ok(Self {})
        }
    }

    pub(super) fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(process_group) = self.process_group.take() {
            // SAFETY: the registry CLI was spawned as the leader of this
            // process group. A negative pid targets it and all descendants.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            windows::terminate(&job);
        }
    }
}

impl Drop for RegistryProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use tokio::process::Child;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    use super::RegistryProcessGroup;

    pub(super) fn attach(child: &Child) -> io::Result<RegistryProcessGroup> {
        let raw_process = child.raw_handle().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "A3S Use registry command exited before Job Object assignment",
            )
        })?;
        let process_id = child.id().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "A3S Use registry command has no process id for Job Object assignment",
            )
        })?;
        let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw_job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };
        let mut limits = unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|_| io::Error::other("Job Object limit structure size overflowed"))?;
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), raw_process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if let Err(error) = resume_process(process_id) {
            terminate(&job);
            return Err(error);
        }
        Ok(RegistryProcessGroup { job: Some(job) })
    }

    pub(super) fn terminate(job: &OwnedHandle) {
        unsafe {
            TerminateJobObject(job.as_raw_handle(), 1);
        }
    }

    fn resume_process(process_id: u32) -> io::Result<()> {
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = THREADENTRY32 {
            dwSize: u32::try_from(size_of::<THREADENTRY32>())
                .map_err(|_| io::Error::other("Windows thread entry is too large"))?,
            ..THREADENTRY32::default()
        };
        let mut found = false;
        let mut available = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while available {
            if entry.th32OwnerProcessID == process_id {
                let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                found = true;
            }
            available = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        if !found {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "suspended A3S Use registry command thread was not found",
            ));
        }
        Ok(())
    }
}
