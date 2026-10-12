// SPDX-License-Identifier: MPL-2.0
//! Shared real child/Job/process-group/IO lifetime, extracted from the speech worker.
//! Protocols, deadlines, permits and callbacks remain owned by each feature.
use std::{
    io,
    process::{Child, Command, ExitStatus},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

#[cfg(all(target_os = "linux", test, feature = "linux-worker-tests"))]
#[path = "owned_process_linux_tests.rs"]
mod linux_tests;

#[cfg(target_os = "linux")]
#[path = "linux_worker.rs"]
pub(crate) mod linux_worker;

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
#[cfg(target_os = "macos")]
#[path = "owned_process_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
use macos::ProcessGroup as ProcessJob;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
struct ProcessJob;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl ProcessJob {
    fn attach(_: &Child) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
    fn kill(&self) {}
    fn empty(&self) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
struct ProcessJob(Mutex<self::linux_worker::LifetimeOwner>);
#[cfg(target_os = "linux")]
impl ProcessJob {
    fn provisional(bridge: self::linux_worker::Bridge) -> Self {
        Self(Mutex::new(self::linux_worker::LifetimeOwner::provisional(
            bridge,
        )))
    }
    fn admit(
        &self,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admit(deadline, cancel)
    }
    fn kill(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }
    fn empty(&self) -> bool {
        let mut owner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        owner.collect_late_owner(std::time::Instant::now() + Duration::from_millis(1));
        owner.empty()
    }
    fn observe_launcher_exit(&self,status:&ExitStatus){
        self.0.lock().unwrap_or_else(|e|e.into_inner()).observe_launcher_exit(status);
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
        // macOS retains the unreaped group leader as the PGID ownership anchor.
        // The group adapter handles both leader and descendants; do not signal
        // an already-reaped/reused numeric PID through Child here.
        #[cfg(not(target_os = "macos"))]
        let _ = self.child.kill();
        #[cfg(target_os = "macos")]
        {
            self.child.stdin.take();
            if self.job.is_none() {
                let _ = self.child.kill();
            }
        }
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
        matches!(self.try_wait(), Ok(Some(_))) && self.tree_empty() && self.io_finished()
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(target_os = "macos")]
        if let Some(job) = &self.job {
            return job.try_wait(&mut self.child);
        }
        let status=self.child.try_wait()?;
        #[cfg(target_os="linux")]
        if let Some(status)=&status {if let Some(job)=&self.job{job.observe_launcher_exit(status);}}
        Ok(status)
    }
}
pub struct ManagedProcess<P: Send + 'static>(Option<Parts<P>>);
impl<P: Send + 'static> ManagedProcess<P> {
    /// The child must wait for stdin config before performing domain work. This
    /// call returns only after supervision is established; callers send no input
    /// before it. macOS supports trusted same-executable helpers that retain
    /// their inherited process group, not arbitrary/daemonizing plugin commands.
    #[cfg(not(target_os = "linux"))]
    pub fn spawn(command: &mut Command, permit: P) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        macos::prepare(command)?;
        let child = command.spawn()?;
        let mut owned = Self(Some(Parts {
            child,
            job: None,
            io: Vec::new(),
            _permit: permit,
        }));
        let process = owned.0.as_mut().unwrap();
        #[cfg(not(target_os = "macos"))]
        {
            process.job = Some(ProcessJob::attach(&process.child)?);
        }
        #[cfg(target_os = "macos")]
        {
            // Install the provisional owner before querying the kernel. On a
            // query/verification failure Drop must still retain the leader and
            // permit while conservatively attempting cleanup, without stdin.
            process.job = Some(ProcessJob::new(&process.child)?);
            process.job.as_ref().unwrap().verify()?;
        }
        Ok(owned)
    }
    #[cfg(target_os = "linux")]
    pub fn spawn(command: &mut Command, permit: P) -> io::Result<Self> {
        // A plain Command has no established namespace/owner receipt. Reject
        // before spawn: killing an unsupervised parent afterwards cannot prove
        // that it did not create an independently surviving descendant.
        let _ = (command, permit);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux command has no trusted worker lifetime owner",
        ))
    }
    /// Provisional ownership is installed before receiving
    /// the actual PID1 or acknowledging it. All failure returns run existing
    /// Drop, retaining the original Child, witness, real IO and operation permit.
    #[cfg(target_os = "linux")]
    pub fn spawn_trusted(
        command: self::linux_worker::TrustedCommand,
        permit: P,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<Self> {
        let owned = Self::prepare_trusted(command, permit, deadline, cancel)?;
        owned.admit_trusted(deadline, cancel)?;
        Ok(owned)
    }
    /// Drain requested diagnostic stderr before admitting the PID1 bridge too.
    /// Only fixed classifications and byte counts are logged, never worker text,
    /// media, paths, environment values or credentials. The real reader remains
    /// part of Parts/permit ownership even if bootstrap admission fails.
    #[cfg(target_os = "linux")]
    pub fn spawn_trusted_with_diagnostics(
        command: self::linux_worker::TrustedCommand, permit:P,
        deadline:std::time::Instant, cancel:&std::sync::atomic::AtomicBool,
    )->io::Result<Self>{
        let mut owned=Self::prepare_trusted(command,permit,deadline,cancel)?;
        if let Some(mut stderr)=owned.child_mut().stderr.take(){
            owned.track_io(std::thread::Builder::new().name("mewu-native-stderr".into()).spawn(move||{
                use std::io::Read;
                let mut prefix=Vec::with_capacity(8192);let mut block=[0u8;1024];let mut total=0usize;
                loop {match stderr.read(&mut block){
                    Ok(0)=>break,Ok(count)=>{total=total.saturating_add(count);
                        let retain=count.min(8192-prefix.len());prefix.extend_from_slice(&block[..retain]);},
                    Err(error) if error.kind()==io::ErrorKind::Interrupted=>{},Err(_)=>break,
                }}
                if total!=0{
                    let text=String::from_utf8_lossy(&prefix).to_ascii_lowercase();
                    let reason=if text.contains("uid map")&&text.contains("permission denied"){"namespace-denied"}
                        else if text.contains("bootstrap refused"){"bootstrap-refused"}
                        else if text.contains("error while loading shared libraries"){"runtime-missing"}
                        else if text.contains("client.conf")||text.contains("spa plugin"){"pipewire-runtime"}
                        else{"native-stderr"};
                    eprintln!("mewu-worker-diagnostic reason={reason} bytes={total} retained={} truncated={}",prefix.len(),total>prefix.len());
                }
            })?);
        }
        owned.admit_trusted(deadline,cancel)?;
        Ok(owned)
    }
    #[cfg(target_os = "linux")]
    fn prepare_trusted(
        command: self::linux_worker::TrustedCommand,
        permit: P,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<Self> {
        self::linux_worker::check(deadline, cancel)?;
        let (child, bridge) = command.launch(deadline, cancel)?;
        Ok(Self(Some(Parts {
            child,
            job: Some(ProcessJob::provisional(bridge)),
            io: Vec::new(),
            _permit: permit,
        })))
    }
    #[cfg(target_os = "linux")]
    fn admit_trusted(
        &self,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.0
            .as_ref()
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .admit(deadline, cancel)
    }
    // Rust-only fixture entrypoints. No main/IPC flag exposes these, and the
    // default production build does not compile the test feature. The fixture
    // keeps the exact Child/job/P in real Parts while controlling admission.
    #[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
    pub(crate) fn prepare_unacknowledged_for_fixture(
        command: self::linux_worker::TrustedCommand,
        permit: P,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<Self> {
        Self::prepare_trusted(command, permit, deadline, cancel)
    }
    #[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
    pub(crate) fn admit_for_fixture(
        &self,
        deadline: std::time::Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.admit_trusted(deadline, cancel)
    }
    pub fn child_mut(&mut self) -> &mut Child {
        // Use only for stdin/stdout/stderr/id. Reaping through this escape hatch
        // bypasses macOS's PGID anchor; call ManagedProcess::try_wait instead.
        &mut self.0.as_mut().unwrap().child
    }
    pub fn track_io(&mut self, thread: JoinHandle<()>) {
        self.0.as_mut().unwrap().io.push(thread);
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.as_mut().unwrap().try_wait()
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
    pub(crate) fn contains_process_for_test(&self, pid: u32) -> Option<bool> {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::{
                JobObjects::IsProcessInJob,
                Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            },
        };
        if pid == 0 || self.0 .0.is_null() {
            return None;
        }
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut contained = 0;
            // Query this duplicated Job handle, not NULL ("any Job").
            let queried = IsProcessInJob(process, self.0 .0, &mut contained);
            let closed = CloseHandle(process);
            if queried == 0 || closed == 0 {
                None
            } else {
                Some(contained != 0)
            }
        }
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
            // Retain permit/Job/IO ownership on OS thread exhaustion. Windows
            // closing the Job is the final cleanup boundary. macOS process
            // groups have no kill-on-host-exit guarantee.
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
