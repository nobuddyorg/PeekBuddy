//! Job objects for the perf harness (#27).
//!
//! Every process the harness launches runs inside a job created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so nothing survives the harness even
//! when it fails. The job also totals CPU time and I/O of the whole process
//! tree (the viewer the engine spawns stays in the job) and reports the peak
//! commit.

use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject,
};

/// Totals of every process that ever ran in a job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Accounting {
    /// User CPU time, 100 ns ticks.
    pub user_ticks: u64,
    /// Kernel CPU time, 100 ns ticks.
    pub kernel_ticks: u64,
    /// Soft and hard page faults.
    pub page_faults: u32,
    /// Processes ever started in the job.
    pub total_processes: u32,
    /// Processes still running.
    pub active_processes: u32,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Read operations.
    pub read_ops: u64,
    /// Write operations.
    pub write_ops: u64,
}

/// A job whose processes die with it.
#[derive(Debug)]
pub struct Job {
    handle: OwnedHandle,
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

impl Job {
    /// Creates a job with `KILL_ON_JOB_CLOSE`.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the job cannot be created or configured.
    pub fn new() -> std::io::Result<Self> {
        // SAFETY: no attributes and no name; a null result is handled below.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(last_error());
        }
        // SAFETY: CreateJobObjectW succeeded, so `raw` is an owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is a valid extended-limit structure of the stated
        // size and outlives the call.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(Self { handle })
    }

    /// Puts a running process into the job. Children it starts afterwards
    /// join the job too.
    ///
    /// # Errors
    ///
    /// Returns the OS error, for example when the process already belongs to
    /// a job that forbids nesting.
    pub fn assign(&self, process: BorrowedHandle<'_>) -> std::io::Result<()> {
        // SAFETY: both handles are live for the call.
        if unsafe { AssignProcessToJobObject(self.handle.as_raw_handle(), process.as_raw_handle()) }
            == 0
        {
            return Err(last_error());
        }
        Ok(())
    }

    /// CPU time, I/O and process counts of everything that ran in the job.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the query fails.
    pub fn accounting(&self) -> std::io::Result<Accounting> {
        let mut info = JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION::default();
        // SAFETY: `info` is a valid, writable structure of the stated size.
        let ok = unsafe {
            QueryInformationJobObject(
                self.handle.as_raw_handle(),
                JobObjectBasicAndIoAccountingInformation,
                (&raw mut info).cast(),
                u32::try_from(size_of::<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION>())
                    .unwrap_or(0),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        let basic = info.BasicInfo;
        let io = info.IoInfo;
        Ok(Accounting {
            user_ticks: u64::try_from(basic.TotalUserTime).unwrap_or(0),
            kernel_ticks: u64::try_from(basic.TotalKernelTime).unwrap_or(0),
            page_faults: basic.TotalPageFaultCount,
            total_processes: basic.TotalProcesses,
            active_processes: basic.ActiveProcesses,
            read_bytes: io.ReadTransferCount,
            write_bytes: io.WriteTransferCount,
            read_ops: io.ReadOperationCount,
            write_ops: io.WriteOperationCount,
        })
    }

    /// Peak commit of any single process that ran in the job, in bytes.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the query fails.
    pub fn peak_process_commit(&self) -> std::io::Result<u64> {
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: `limits` is a valid, writable structure of the stated size.
        let ok = unsafe {
            QueryInformationJobObject(
                self.handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw mut limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(limits.PeakProcessMemoryUsed as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::io::AsHandle as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::Job;

    fn sleeper() -> std::process::Child {
        Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn closing_the_job_kills_its_processes() {
        let job = Job::new().unwrap();
        let mut child = sleeper();
        job.assign(child.as_handle()).unwrap();
        assert!(job.accounting().unwrap().active_processes >= 1);
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the child outlived its job");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn accounting_counts_cpu_and_io_of_the_whole_tree() {
        let job = Job::new().unwrap();
        let mut child = Command::new("cmd")
            .args(["/c", "dir C:\\Windows\\System32 >nul"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        job.assign(child.as_handle()).unwrap();
        child.wait().unwrap();
        // The job's count of live processes can lag a moment behind the exit of
        // the child (a console host it started, on a loaded machine).
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut accounting = job.accounting().unwrap();
        while accounting.active_processes != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            accounting = job.accounting().unwrap();
        }
        assert_eq!(accounting.active_processes, 0);
        assert!(accounting.total_processes >= 1);
        assert!(accounting.user_ticks + accounting.kernel_ticks > 0);
        assert!(job.peak_process_commit().unwrap() > 0);
    }
}
