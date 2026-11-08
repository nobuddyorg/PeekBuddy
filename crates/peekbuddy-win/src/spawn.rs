//! Starting a user-configured program for a rule (#81).
//!
//! The program is named by its full path and started directly with
//! `CreateProcessW`: no shell, no `PATH` search, no console window. It runs
//! in a job that kills it with its children when the job closes, so a
//! timeout or the end of PeekBuddy leaves nothing behind. The child is
//! created suspended, put into the job, then resumed, so it cannot start a
//! grandchild outside the job.

use std::os::windows::io::{AsHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    GetExitCodeProcess, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW, WaitForSingleObject,
};

use crate::job::Job;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// A started program and the job that holds it.
#[derive(Debug)]
pub struct Child {
    process: OwnedHandle,
    /// Closing the job kills whatever is left.
    job: Job,
}

/// Starts `program` (a full path, UTF-16) with `command_line` (the quoted
/// arguments, program first).
///
/// # Errors
///
/// The OS error when the job or the process cannot be created. A NUL in
/// either string is `InvalidInput`.
pub fn spawn(program: &[u16], command_line: &[u16]) -> std::io::Result<Child> {
    if program.contains(&0) || command_line.contains(&0) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let job = Job::new()?;
    let mut app: Vec<u16> = program.to_vec();
    app.push(0);
    // CreateProcessW may write into the command line.
    let mut line: Vec<u16> = command_line.to_vec();
    line.push(0);
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(0),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `app` and `line` are NUL-terminated buffers that outlive the
    // call, `line` is writable as the API requires, the structures are
    // initialised, and no attributes or directory are passed.
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &raw const startup,
            &raw mut info,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: CreateProcessW succeeded, so both handles are ours to close.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: as above.
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    // A process that is not in the job must not run: dropping `process`
    // without resuming it leaves it suspended, and the caller sees the error.
    job.assign(process.as_handle())?;
    // SAFETY: `thread` is the live primary thread handle of the new process.
    if unsafe { ResumeThread(std::os::windows::io::AsRawHandle::as_raw_handle(&thread)) }
        == u32::MAX
    {
        return Err(last_error());
    }
    Ok(Child { process, job })
}

impl Child {
    /// Waits up to `timeout_ms` for the program to end.
    ///
    /// Returns its exit code, or `None` when it is still running.
    ///
    /// # Errors
    ///
    /// The OS error when waiting or reading the code fails.
    pub fn wait(&self, timeout_ms: u32) -> std::io::Result<Option<u32>> {
        // SAFETY: the process handle is live for the call.
        let waited = unsafe {
            WaitForSingleObject(
                std::os::windows::io::AsRawHandle::as_raw_handle(&self.process),
                timeout_ms,
            )
        };
        match waited {
            WAIT_OBJECT_0 => {
                let mut code = 0_u32;
                // SAFETY: live handle and a valid out pointer.
                let ok = unsafe {
                    GetExitCodeProcess(
                        std::os::windows::io::AsRawHandle::as_raw_handle(&self.process),
                        &raw mut code,
                    )
                };
                if ok == 0 {
                    return Err(last_error());
                }
                Ok(Some(code))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(last_error()),
        }
    }

    /// Kills the program and everything it started.
    pub fn kill(self) {
        // Dropping the job ends every process in it.
        drop(self.job);
    }
}
