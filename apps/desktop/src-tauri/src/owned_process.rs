// SPDX-License-Identifier: MPL-2.0
//! Shared real child/Job/IO lifetime, extracted from the speech worker.
//! Protocols, deadlines, permits and callbacks remain owned by each feature.
use std::{
    io,
    process::{Child, Command, ExitStatus},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

#[cfg(windows)]
#[derive(Debug)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        Self::attach_handle(child.as_raw_handle())
    }
    fn attach_handle(parent: std::os::windows::io::RawHandle) -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, parent) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
    fn kill(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
    fn empty(&self) -> bool {
        self.active_processes() == Some(0)
    }
    fn active_processes(&self) -> Option<u32> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            if QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            ) != 0
            {
                Some(info.ActiveProcesses)
            } else {
                None
            }
        }
    }
}

/// An independently owned Windows parent/Job witness. Capture it while the
/// arbitrary child is suspended, before any wrapper resumes execution. A failed
/// Job attachment still retains the parent handle until it is really signaled.
#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct ProcessWitness {
    parent: std::os::windows::io::OwnedHandle,
    job: Option<ProcessJob>,
}
#[cfg(windows)]
impl ProcessWitness {
    pub(crate) fn capture(parent: std::os::windows::io::BorrowedHandle<'_>) -> io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let mut duplicate = std::ptr::null_mut();
        unsafe {
            if DuplicateHandle(
                GetCurrentProcess(),
                parent.as_raw_handle(),
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                parent: std::os::windows::io::OwnedHandle::from_raw_handle(duplicate),
                job: None,
            })
        }
    }
    pub(crate) fn attach_job(&mut self) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        self.job = Some(ProcessJob::attach_handle(self.parent.as_raw_handle())?);
        Ok(())
    }
    pub(crate) fn kill(&self) {
        use std::os::windows::io::AsRawHandle;
        if let Some(job) = &self.job {
            job.kill();
        }
        unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(self.parent.as_raw_handle(), 1);
        }
    }
    /// With no attached Job the caller must have prevented the suspended child
    /// from ever being resumed. An absent/failed query is never an empty Job.
    pub(crate) fn fully_exited(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };
        let parent_done =
            unsafe { WaitForSingleObject(self.parent.as_raw_handle(), 0) == WAIT_OBJECT_0 };
        parent_done && self.job.as_ref().is_none_or(ProcessJob::empty)
    }
    #[cfg(test)]
    pub(crate) fn terminate_parent_for_test(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(self.parent.as_raw_handle(), 1);
        }
    }
    #[cfg(test)]
    pub(crate) fn active_processes_for_test(&self) -> Option<u32> {
        self.job.as_ref().and_then(ProcessJob::active_processes)
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
struct ProcessJob;
#[cfg(not(windows))]
impl ProcessJob {
    fn attach(_: &Child) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
    fn kill(&self) {}
    fn empty(&self) -> bool {
        true
    }
}

struct Parts<P: Send + 'static> {
    child: Child,
    job: Option<ProcessJob>,
    io: Vec<JoinHandle<()>>,
    _permit: P,
}
impl<P: Send + 'static> Parts<P> {
    fn kill(&mut self) {
        if let Some(job) = &self.job {
            job.kill();
        }
        let _ = self.child.kill();
    }
    fn tree_empty(&self) -> bool {
        self.job.as_ref().is_none_or(ProcessJob::empty)
    }
    fn io_finished(&mut self) -> bool {
        // A writer may still own the input buffer after the child exits. Its real
        // thread lifetime is part of the slot, not just a completion notification.
        let mut index = 0;
        while index < self.io.len() {
            if self.io[index].is_finished() {
                let _ = self.io.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        self.io.is_empty()
    }
    fn fully_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_))) && self.tree_empty() && self.io_finished()
    }
}
pub struct ManagedProcess<P: Send + 'static>(Option<Parts<P>>);
impl<P: Send + 'static> ManagedProcess<P> {
    /// The child must wait for stdin config before performing domain work. This
    /// call returns only after Job attachment; callers send no input before it.
    pub fn spawn(command: &mut Command, permit: P) -> io::Result<Self> {
        let child = command.spawn()?;
        let mut owned = Self(Some(Parts {
            child,
            job: None,
            io: Vec::new(),
            _permit: permit,
        }));
        let process = owned.0.as_mut().unwrap();
        process.job = Some(ProcessJob::attach(&process.child)?);
        Ok(owned)
    }
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.0.as_mut().unwrap().child
    }
    pub fn track_io(&mut self, thread: JoinHandle<()>) {
        self.0.as_mut().unwrap().io.push(thread);
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child_mut().try_wait()
    }
    pub fn kill(&mut self) {
        self.0.as_mut().unwrap().kill();
    }
    pub fn tree_empty(&self) -> bool {
        self.0.as_ref().unwrap().tree_empty()
    }
    pub fn fully_exited(&mut self) -> bool {
        self.0.as_mut().unwrap().fully_exited()
    }
    #[cfg(all(windows, test))]
    pub(crate) fn observe_job_for_test(&self) -> io::Result<JobObserver> {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let mut duplicate = std::ptr::null_mut();
        unsafe {
            if DuplicateHandle(
                GetCurrentProcess(),
                self.0.as_ref().unwrap().job.as_ref().unwrap().0,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(JobObserver(ProcessJob(duplicate)))
    }
}
#[cfg(all(windows, test))]
pub(crate) struct JobObserver(ProcessJob);
#[cfg(all(windows, test))]
impl JobObserver {
    pub(crate) fn active_processes(&self) -> Option<u32> {
        self.0.active_processes()
    }
}
impl<P: Send + 'static> Drop for ManagedProcess<P> {
    fn drop(&mut self) {
        let Some(mut parts) = self.0.take() else {
            return;
        };
        if parts.fully_exited() {
            return;
        }
        parts.kill();
        let held = Arc::new(Mutex::new(Some(parts)));
        let worker = held.clone();
        if std::thread::Builder::new()
            .name("mewu-worker-reaper".into())
            .spawn(move || {
                let mut parts = worker
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .expect("owned child");
                while !parts.fully_exited() {
                    parts.kill();
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .is_err()
        {
            // Retain permit/Job/IO ownership on OS thread exhaustion. The host
            // process closing the Job remains the final cleanup boundary.
            std::mem::forget(held);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    #[test]
    fn synthetic_exit_child() {
        // The same-test subprocess has no application initialization. This
        // test is intentionally also a no-op in normal workspace execution.
        if std::env::var("MEWU_OWNED_PROCESS_TEST_CHILD").as_deref() == Ok("1") {
            let mut gate = String::new();
            assert_eq!(std::io::stdin().read_line(&mut gate).unwrap(), 3);
            assert_eq!(gate, "go\n");
        }
    }
    struct TestPermit(Arc<AtomicBool>);
    impl Drop for TestPermit {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    #[test]
    fn completed_child_does_not_release_permit_before_tracked_io_thread() {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        use std::process::Stdio;
        let released = Arc::new(AtomicBool::new(false));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "owned_process::tests::synthetic_exit_child"])
            .env("MEWU_OWNED_PROCESS_TEST_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        let mut process =
            ManagedProcess::spawn(&mut command, TestPermit(released.clone())).unwrap();
        let (tx, rx) = mpsc::channel::<()>();
        process.track_io(std::thread::spawn(move || {
            let _ = rx.recv();
        }));
        process
            .child_mut()
            .stdin
            .take()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
        let started = std::time::Instant::now();
        while process.try_wait().unwrap().is_none() && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(process.try_wait().unwrap().is_some());
        assert!(!process.fully_exited());
        drop(process);
        assert!(!released.load(Ordering::Acquire));
        tx.send(()).unwrap();
        let started = std::time::Instant::now();
        while !released.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(released.load(Ordering::Acquire));
    }
}
