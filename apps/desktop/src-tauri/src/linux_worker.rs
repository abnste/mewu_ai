// SPDX-License-Identifier: MPL-2.0
//! Sealed Linux worker preparation. Native tests are required before enabling constructors.
//! This module prepares a sealed command and bootstrap bridge. The actual
//! process/permit/IO owner must be installed before receipt or ACK operations.
#![cfg(target_os = "linux")]
use rustix::{
    event::{poll, PollFd, PollFlags, Timespec},
    io::{fcntl_dupfd_cloexec, fcntl_setfd, FdFlags},
    net::{
        recvmsg, sendmsg, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
        SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
    },
    process::{getpid, pidfd_open, PidfdFlags},
};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const BOOTSTRAP: &str = "--mewu-linux-worker-bootstrap";
const RECEIPT: u8 = b'O';
const ACK: u8 = b'A';
#[path = "linux_worker_env.rs"]
mod resource_env;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Worker {
    Code,
    Formula,
    VideoClip,
    Snapshot,
    Speech,
    Recording,
}
impl Worker {
    fn admitted(self)->bool {
        matches!(self, Self::Recording) || cfg!(feature="linux-worker-tests")
    }
    pub fn flag(self) -> &'static str {
        match self {
            Self::Code => "--code-worker",
            Self::Formula => "--formula-layout-worker",
            Self::VideoClip => "--video-clip-worker",
            Self::Snapshot => "--application-snapshot-worker",
            Self::Speech => "--speech-worker",
            Self::Recording => "--recording-worker",
        }
    }
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "--code-worker" => Ok(Self::Code),
            "--formula-layout-worker" => Ok(Self::Formula),
            "--video-clip-worker" => Ok(Self::VideoClip),
            "--application-snapshot-worker" => Ok(Self::Snapshot),
            "--speech-worker" => Ok(Self::Speech),
            "--recording-worker" => Ok(Self::Recording),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }
}

/// No arbitrary executable, argument, pre_exec callback or raw Command access.
pub(crate) struct TrustedCommand {
    command: Option<Command>,
    parent: Option<UnixStream>,
    child: Option<UnixStream>,
    source: Option<File>,
    resources: resource_env::ResourceEnvironment,
}

impl TrustedCommand {
    pub fn new(worker: Worker) -> io::Result<Self> {
        if !worker.admitted(){return Err(io::ErrorKind::PermissionDenied.into());}
        // Freeze the actual loaded executable inode, not its replaceable path.
        // /proc/self/exe is the kernel-owned identity of this running process.
        let loaded = File::open("/proc/self/exe")?;
        let source = File::from(fcntl_dupfd_cloexec(&loaded, 3)?);
        drop(loaded);
        let resources = resource_env::ResourceEnvironment::capture(&source)?;
        let source_raw = source.as_raw_fd();
        let executable = format!("/proc/self/fd/{source_raw}");
        let (a, b) = UnixStream::pair()?;
        // Do not assume 0..2 were open or hard-code fd3. Keep both endpoints
        // outside stdio before fork, with atomic close-on-exec duplication.
        let parent = UnixStream::from(fcntl_dupfd_cloexec(&a, 3)?);
        let child = UnixStream::from(fcntl_dupfd_cloexec(&b, 3)?);
        drop((a, b));
        parent.set_nonblocking(true)?;
        let raw = child.as_raw_fd();
        let mut command = Command::new("/usr/bin/bwrap");
        command
            .args([
                "--unshare-user",
                "--unshare-pid",
                "--as-pid-1",
                "--die-with-parent",
                "--bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--",
            ])
            .arg(executable)
            .args([BOOTSTRAP, worker.flag()])
            .arg(raw.to_string())
            .arg(source_raw.to_string());
        // SAFETY: the inherited endpoint remains owned until spawn returns.
        // This closure only performs the specified FD syscalls and creates
        // inline OS errors. No allocation, mutex, logging, env or directory scan.
        unsafe {
            command.pre_exec(move || {
                if libc::syscall(
                    libc::SYS_close_range,
                    3u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::fcntl(source_raw, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(Self {
            command: Some(command),
            parent: Some(parent),
            child: Some(child),
            source: Some(source),
            resources,
        })
    }
    fn builder(&mut self) -> &mut Command {
        self.command
            .as_mut()
            .expect("trusted command already consumed")
    }
    pub fn stdin(&mut self, value: Stdio) -> &mut Self {
        self.builder().stdin(value);
        self
    }
    pub fn stdout(&mut self, value: Stdio) -> &mut Self {
        self.builder().stdout(value);
        self
    }
    pub fn stderr(&mut self, value: Stdio) -> &mut Self {
        self.builder().stderr(value);
        self
    }
    pub fn env_clear(&mut self) -> &mut Self {
        self.builder().env_clear();
        self
    }
    pub fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.builder().env(name, value);
        self
    }
    pub fn env_remove(&mut self, name: impl AsRef<OsStr>) -> &mut Self {
        self.builder().env_remove(name);
        self
    }
    pub fn current_dir(&mut self, value: impl AsRef<Path>) -> &mut Self {
        self.builder().current_dir(value);
        self
    }
    /// Caller must immediately retain this Child and bridge with the operation
    /// permit before any fallible receipt or cancellation operation. No domain
    /// configuration can be sent before the owner is admitted and ACK succeeds.
    pub fn launch(mut self, deadline: Instant, cancel: &AtomicBool) -> io::Result<(Child, Bridge)> {
        check(deadline, cancel)?;
        // Sample the kernel clock BEFORE Instant so conversion can only shorten
        // the original deadline. Child reverses sampling order for the same
        // conservative conversion; no fresh startup allowance is introduced.
        let sampled = monotonic_nanos()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let absolute = sampled
            .checked_add(
                remaining
                    .as_nanos()
                    .try_into()
                    .map_err(|_| io::ErrorKind::InvalidInput)?,
            )
            .ok_or(io::ErrorKind::InvalidInput)?;
        check(deadline, cancel)?;
        self.resources.apply(
            self.command.as_mut().unwrap(),
            self.source.as_ref().unwrap(),
        )?;
        check(deadline, cancel)?;
        let child = self
            .command
            .take()
            .unwrap()
            .arg(absolute.to_string())
            .spawn()?;
        drop(self.child.take());
        drop(self.source.take());
        Ok((child, Bridge{socket:self.parent.take().unwrap(),candidates:Vec::new(),clean_eof:false,invalid:false}))
    }
}

// Own received descriptors before validating the packet. A malformed packet
// must not silently discard a possible kernel owner while its permit is held.
pub(crate) struct Bridge { socket:UnixStream, candidates:Vec<OwnedFd>, clean_eof:bool, invalid:bool }
#[cfg(test)]
impl Bridge{
    pub(super) fn for_race_fixture(socket:UnixStream)->Self{Self{socket,candidates:Vec::new(),clean_eof:false,invalid:false}}
}


/// Conservative ownership sketch. Retained permits/IO belong to the caller's
/// actual Parts. A failed, never-admitted bootstrap may finish only with a
/// normally reaped fixed bwrap launcher AND a clean bridge EOF with no candidate
/// receipt. Signaled launchers, bad packets and unknown descriptors stay held.
pub(crate) struct LifetimeOwner {
    bridge: Option<Bridge>,
    owner: Option<OwnedFd>,
    launcher_normal_exit:bool,
}
impl LifetimeOwner {
    pub fn provisional(bridge: Bridge) -> Self {
        Self {
            bridge: Some(bridge),
            owner: None,
            launcher_normal_exit:false,
        }
    }
    /// Call only after this owner, the original Child and permit are installed
    /// into actual Parts. No config writer exists until this returns success.
    pub fn admit(&mut self, deadline: Instant, cancel: &AtomicBool) -> io::Result<()> {
        let bridge = self.bridge.as_mut().ok_or(io::ErrorKind::InvalidInput)?;
        self.owner = Some(bridge.receive_owner(deadline, cancel)?);
        if self.empty() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.bridge
            .as_ref()
            .unwrap()
            .acknowledge(deadline, cancel)?;
        drop(self.bridge.take());
        Ok(())
    }
    /// Kernel PID1 completion is meaningful only with an actually retained
    /// namespace-init pidfd. Wait/reap and real IO joining remain separate.
    pub fn empty(&self) -> bool {
        let Some(owner) = self.owner.as_ref() else {
            return self.launcher_normal_exit&&self.bridge.as_ref().is_some_and(|bridge|
                bridge.clean_eof&&!bridge.invalid&&bridge.candidates.is_empty());
        };
        let mut fds = [PollFd::new(owner, PollFlags::IN)];
        matches!(
            poll(
                &mut fds,
                Some(&Timespec {
                    tv_sec: 0,
                    tv_nsec: 0
                })
            ),
            Ok(_)
        ) && fds[0].revents().contains(PollFlags::IN)
    }
    pub fn kill(&self) {
        if let Some(owner) = self.owner.as_ref() {
            let _ = rustix::process::pidfd_send_signal(owner, rustix::process::Signal::KILL);
        } else if let Some(bridge) = self.bridge.as_ref() {
            // NACK prevents domain work if the owner receipt arrives later.
            // Keep the reading endpoint to collect that immutable witness;
            // do not discard it and fabricate an absent-owner drain receipt.
            let mut storage = [];
            let mut ancillary = SendAncillaryBuffer::new(&mut storage);
            let _ = sendmsg(
                &bridge.socket,
                &[IoSlice::new(b"N")],
                &mut ancillary,
                SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
            );
        }
    }
    /// Failure cleanup can collect a late self-owner without ever ACKing it.
    /// Use the existing cleanup budget, not a new startup/Ready allowance.
    pub fn collect_late_owner(&mut self, cleanup_deadline: Instant) {
        if self.owner.is_some() {
            return;
        }
        if let Some(bridge) = self.bridge.as_mut() {
            if let Ok(owner) = bridge.receive_owner(cleanup_deadline, &AtomicBool::new(false)) {
                self.owner = Some(owner);
                self.kill();
            }
        }
    }
    /// Only the owner of the actual Child may provide its wait result. Fixed
    /// bwrap's normal monitor return waits for namespace init to exit (upstream
    /// 0.9.0 monitor_child). A signal delivered to the wrapper is not that proof.
    pub(super) fn observe_launcher_exit(&mut self,status:&std::process::ExitStatus){
        self.launcher_normal_exit=status.code().is_some();
    }
}
pub(super) fn check(deadline: Instant, cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(())
}
fn ready(
    fd: impl AsFd,
    events: PollFlags,
    deadline: Instant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    loop {
        check(deadline, cancel)?;
        // Bounded polling checks the same original deadline and cancellation,
        // rather than introducing an independent startup timeout.
        let remaining = deadline.saturating_duration_since(Instant::now());
        let delay = remaining.min(std::time::Duration::from_millis(10));
        let mut fds = [PollFd::new(&fd, events)];
        match poll(
            &mut fds,
            Some(&Timespec {
                tv_sec: delay.as_secs() as i64,
                tv_nsec: delay.subsec_nanos() as i64,
            }),
        ) {
            Ok(_)
                if fds[0]
                    .revents()
                    .intersects(events | PollFlags::HUP | PollFlags::ERR) =>
            {
                return Ok(())
            }
            Ok(_) | Err(rustix::io::Errno::INTR) => (),
            Err(e) => return Err(e.into()),
        }
    }
}

impl Bridge {
    pub fn receive_owner(&mut self, deadline: Instant, cancel: &AtomicBool) -> io::Result<OwnedFd> {
        // This sealed bootstrap sends one receipt. After a malformed receipt,
        // retain its descriptors and remain unadmitted; do not read an unbounded
        // sequence of new descriptors or acknowledge a subsequent packet.
        if self.invalid||!self.candidates.is_empty() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        loop {
            check(deadline, cancel)?;
            let mut bytes = [0u8; 1];
            let mut storage = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
            let result = match recvmsg(
                &self.socket,
                &mut [IoSliceMut::new(&mut bytes)],
                &mut ancillary,
                RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
            ) {
                Ok(v) => v,
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => {
                    ready(&self.socket, PollFlags::IN, deadline, cancel)?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let mut unknown_control = false;
            for message in ancillary.drain() {
                match message {
                    RecvAncillaryMessage::ScmRights(fds) => {
                        self.candidates.extend(fds);
                    }
                    _ => unknown_control = true,
                }
            }
            if result.bytes==0&&self.candidates.is_empty()&&!unknown_control
                &&result.flags.difference(ReturnFlags::CMSG_CLOEXEC).is_empty(){
                self.clean_eof=true;return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if unknown_control
                || self.candidates.len() != 1
                || result.bytes != 1
                || bytes[0] != RECEIPT
                || result
                    .flags
                    .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC)
            {
                self.invalid=true;
                return Err(io::ErrorKind::InvalidData.into());
            }
            let owner = &self.candidates[0];
            // SIG0 validates that this is actually a kernel pidfd. It changes
            // no target state. A regular-file/closed/malformed receipt is refused.
            let validity = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    owner.as_raw_fd(),
                    0i32,
                    std::ptr::null::<libc::siginfo_t>(),
                    0u32,
                )
            };
            if validity != 0 {
                let error = io::Error::last_os_error();
                // ESRCH still establishes a pidfd for an exited process.
                // Retain it so the actual parent wait/IO cleanup can finish;
                // no ACK will be sent for a signaled owner.
                if error.raw_os_error() != Some(libc::ESRCH) {
                    self.invalid=true;
                    return Err(error);
                }
            }
            return Ok(self.candidates.pop().unwrap());
        }
    }
    pub fn acknowledge(&self, deadline: Instant, cancel: &AtomicBool) -> io::Result<()> {
        loop {
            check(deadline, cancel)?;
            let mut storage = [];
            let mut ancillary = SendAncillaryBuffer::new(&mut storage);
            match sendmsg(
                &self.socket,
                &[IoSlice::new(&[ACK])],
                &mut ancillary,
                SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
            ) {
                Ok(1) => return Ok(()),
                Ok(_) => return Err(io::ErrorKind::WriteZero.into()),
                Err(rustix::io::Errno::INTR) => (),
                Err(rustix::io::Errno::AGAIN) => ready(&self.socket, PollFlags::OUT, deadline, cancel)?,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

fn monotonic_nanos() -> io::Result<u64> {
    let mut value = MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, value.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    let seconds: u64 = value
        .tv_sec
        .try_into()
        .map_err(|_| io::ErrorKind::InvalidData)?;
    let nanos: u64 = value
        .tv_nsec
        .try_into()
        .map_err(|_| io::ErrorKind::InvalidData)?;
    if nanos >= 1_000_000_000 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|v| v.checked_add(nanos))
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}

/// Fixed bootstrap routing. Any malformed bootstrap request is a terminal
/// error, never a fallthrough to GUI or normal domain dispatch. Main must exit
/// with this return code when Some is returned, before app initialization.
pub fn bootstrap_entry() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).map(|v| v != BOOTSTRAP).unwrap_or(true) {
        return None;
    }
    let result = (|| {
        if args.len() != 6 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let value = |index: usize| args[index].to_str().ok_or(io::ErrorKind::InvalidInput);
        let worker = Worker::parse(value(2)?)?;
        let bridge: i32 = value(3)?.parse().map_err(|_| io::ErrorKind::InvalidInput)?;
        let source: i32 = value(4)?.parse().map_err(|_| io::ErrorKind::InvalidInput)?;
        let absolute: u64 = value(5)?.parse().map_err(|_| io::ErrorKind::InvalidInput)?;
        let sampled = Instant::now();
        let remaining = absolute
            .checked_sub(monotonic_nanos()?)
            .ok_or(io::ErrorKind::TimedOut)?;
        let deadline = sampled
            .checked_add(Duration::from_nanos(remaining))
            .ok_or(io::ErrorKind::InvalidInput)?;
        bootstrap(worker, bridge, source, deadline)
    })();
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("Linux worker bootstrap refused: {error}");
            Some(2)
        }
    }
}

/// Called only at the fixed bootstrap entry before the original domain worker.
/// This is ordinary code after exec, not a pre_exec hook. The caller exits PID1
/// on any error; it must never fall through to normal GUI or worker dispatch.
pub fn bootstrap(worker: Worker, raw: i32, source_raw: i32, deadline: Instant) -> io::Result<()> {
    if !worker.admitted(){return Err(io::ErrorKind::PermissionDenied.into());}
    if raw < 3 || source_raw < 3 || raw == source_raw || getpid().as_raw_nonzero().get() != 1 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    // Check validity before creating an owned Rust descriptor; do not borrow a
    // potentially invalid command-line FD. Only the fixed standalone bootstrap
    // consumes it, once, before application code starts.
    if unsafe { libc::fcntl(raw, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(source_raw, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let source = unsafe { File::from_raw_fd(source_raw) };
    fcntl_setfd(&source, FdFlags::CLOEXEC)?;
    let pinned = source.metadata()?;
    let loaded = std::fs::metadata("/proc/self/exe")?;
    if !pinned.is_file() || pinned.dev() != loaded.dev() || pinned.ino() != loaded.ino() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let socket = unsafe { UnixStream::from_raw_fd(raw) };
    fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
    socket.set_nonblocking(true)?;
    let owner = pidfd_open(getpid(), PidfdFlags::empty())?;
    let no_cancel = AtomicBool::new(false);
    loop {
        check(deadline, &no_cancel)?;
        let fds = [owner.as_fd()];
        let mut storage = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut storage);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        match sendmsg(
            &socket,
            &[IoSlice::new(&[RECEIPT])],
            &mut ancillary,
            SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
        ) {
            Ok(1) => break,
            Ok(_) => return Err(io::ErrorKind::WriteZero.into()),
            Err(rustix::io::Errno::INTR) => (),
            Err(rustix::io::Errno::AGAIN) => ready(&socket, PollFlags::OUT, deadline, &no_cancel)?,
            Err(e) => return Err(e.into()),
        }
    }
    loop {
        check(deadline, &no_cancel)?;
        let mut bytes = [0u8; 1];
        let mut storage = [];
        let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
        match recvmsg(
            &socket,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
        ) {
            // Linux echoes the requested MSG_CMSG_CLOEXEC in msg_flags,
            // including ACKs with no ancillary data. It is not a bad packet.
            // Every other flag, including truncation, remains forbidden.
            Ok(result)
                if result.bytes == 1
                    && bytes[0] == ACK
                    && result
                        .flags
                        .difference(ReturnFlags::CMSG_CLOEXEC)
                        .is_empty() =>
            {
                break
            }
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(rustix::io::Errno::INTR) => (),
            Err(rustix::io::Errno::AGAIN) => ready(&socket, PollFlags::IN, deadline, &no_cancel)?,
            Err(e) => return Err(e.into()),
        }
    }
    check(deadline, &no_cancel)?;
    drop((owner, socket));
    // The PID1 identity survives exec. Domain worker inherits only stdio and
    // the fixed constructor's existing environment, never the owner bridge.
    // The same immutable source descriptor remains owned until exec returns.
    // CLOEXEC closes it on successful ELF exec; the original worker gets no
    // bridge/source handles. Never reopen current_exe's mutable pathname.
    Err(Command::new(format!("/proc/self/fd/{source_raw}"))
        .arg(worker.flag())
        .exec())
}

#[cfg(all(test,not(feature="linux-worker-tests")))]
mod production_tests {
    use super::*;
    #[test]
    fn production_bootstrap_admission_keeps_non_recording_domains_closed(){
        assert!(Worker::Recording.admitted());
        for worker in [Worker::Code,Worker::Formula,Worker::VideoClip,Worker::Snapshot,Worker::Speech]{
            assert!(!worker.admitted());
            assert_eq!(TrustedCommand::new(worker).err().unwrap().kind(),io::ErrorKind::PermissionDenied);
            assert_eq!(bootstrap(worker,-1,-1,Instant::now()).unwrap_err().kind(),io::ErrorKind::PermissionDenied);
        }
        // Production admission never grants an invalid/missing PID1 bridge.
        assert_eq!(bootstrap(Worker::Recording,-1,-1,Instant::now()).unwrap_err().kind(),io::ErrorKind::PermissionDenied);
    }
    fn closed_bridge()->Bridge{
        let (socket,peer)=UnixStream::pair().unwrap();socket.set_nonblocking(true).unwrap();drop(peer);
        Bridge{socket,candidates:Vec::new(),clean_eof:false,invalid:false}
    }
    #[test]
    fn production_pre_admission_eof_requires_real_normal_launcher_wait(){
        let mut owner=LifetimeOwner::provisional(closed_bridge());
        assert!(owner.admit(Instant::now()+Duration::from_secs(1),&AtomicBool::new(false)).is_err());
        assert!(!owner.empty(),"EOF alone never releases a namespace permit");
        let status=Command::new("/usr/bin/true").status().unwrap();
        owner.observe_launcher_exit(&status);
        assert!(owner.empty(),"clean unadmitted EOF plus actual normal launcher wait");

        let mut signaled=LifetimeOwner::provisional(closed_bridge());
        assert!(signaled.admit(Instant::now()+Duration::from_secs(1),&AtomicBool::new(false)).is_err());
        let mut child=Command::new("/usr/bin/sleep").arg("10").spawn().unwrap();
        child.kill().unwrap();let status=child.wait().unwrap();assert!(status.code().is_none());
        signaled.observe_launcher_exit(&status);
        assert!(!signaled.empty(),"signaled wrapper is not kernel namespace-init exit proof");
    }
    #[test]
    fn production_bad_owner_receipt_stays_held_after_launcher_exit(){
        let (socket,peer)=UnixStream::pair().unwrap();socket.set_nonblocking(true).unwrap();
        let pidfd=pidfd_open(getpid(),PidfdFlags::empty()).unwrap();
        let fds=[pidfd.as_fd()];let mut storage=[MaybeUninit::uninit();rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary=SendAncillaryBuffer::new(&mut storage);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
        assert_eq!(sendmsg(&peer,&[IoSlice::new(b"X")],&mut ancillary,SendFlags::NOSIGNAL).unwrap(),1);drop(peer);
        let mut owner=LifetimeOwner::provisional(Bridge{socket,candidates:Vec::new(),clean_eof:false,invalid:false});
        assert!(owner.admit(Instant::now()+Duration::from_secs(1),&AtomicBool::new(false)).is_err());
        owner.observe_launcher_exit(&Command::new("/usr/bin/true").status().unwrap());
        owner.collect_late_owner(Instant::now()+Duration::from_millis(10));
        assert!(!owner.empty(),"bad packet retains its real live pidfd; no absent-owner shortcut");
        assert_eq!(owner.bridge.as_ref().unwrap().candidates.len(),1);
    }
    #[test]
    fn production_failed_sealed_startup_reaps_real_launcher_and_io_before_release(){
        use std::sync::Arc;
        struct Held(Arc<AtomicBool>);impl Drop for Held{fn drop(&mut self){self.0.store(true,Ordering::Release);}}
        let released=Arc::new(AtomicBool::new(false));
        let mut command=TrustedCommand::new(Worker::Recording).unwrap();
        command.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Actual sealed bwrap launch. This test ELF deliberately has no normal
        // application's bootstrap entry, so no PID1 receipt/ACK/domain config
        // is possible. Docker may instead deny namespace creation even earlier.
        // Both exercise real pre-admission launcher failure, not a fake Ready.
        assert!(crate::owned_process::ManagedProcess::spawn_trusted_with_diagnostics(
            command,Held(released.clone()),Instant::now()+Duration::from_secs(3),&AtomicBool::new(false)).is_err());
        let until=Instant::now()+Duration::from_secs(5);
        while !released.load(Ordering::Acquire){assert!(Instant::now()<until,"actual failed launcher/IO permit leaked");
            std::thread::sleep(Duration::from_millis(10));}
    }
}
