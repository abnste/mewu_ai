// SPDX-License-Identifier: MPL-2.0
//! A conservative Darwin process-group owner for trusted stdin-gated helpers.
//!
//! This is not a Windows Job or a security sandbox. Helpers/descendants must not
//! call setsid/setpgid or delegate work outside their inherited group. Arbitrary
//! executables are rejected. A crashed host does not kill the group; unobserved
//! daemonizing descendants cannot be accounted for by this mechanism.
//!
//! The leader remains waitable (WNOWAIT) until the kernel lists only that exited
//! leader, including zombies. This pins its PID/PGID against numeric reuse. A
//! failed, truncated, or inconsistent query never means the group is empty.
//!
//! Contracts: std::os::unix::process::CommandExt::process_group; Apple XNU
//! bsd/kern/proc_info.c::proc_listpids and kern_exit.c::waitid; libproc's wrapper
//! returns zero both for an empty list and for an error. killpg filters SZOMB, so
//! kill(-pgid, 0) is insufficient evidence that all group processes are gone.
use std::{
    io,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command, ExitStatus},
    sync::{Mutex, OnceLock},
};

// sys/proc_info.h. A bounded list is sufficient to prove "only the leader";
// reaching the bound means unknown, not an invitation to truncate ownership.
const PROC_PGRP_ONLY: u32 = 2;
const MAX_GROUP_PROCESSES: usize = 1024;

pub(super) fn prepare(command: &mut Command) -> io::Result<()> {
    if std::fs::canonicalize(command.get_program())?
        != std::fs::canonicalize(std::env::current_exe()?)?
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "macOS supervision requires a trusted same-executable helper",
        ));
    }
    let first = command.get_args().next();
    let gated_helper = first.is_some_and(|arg| {
        [
            "--code-worker",
            "--formula-layout-worker",
            "--speech-worker",
            "--recording-worker",
            "--video-clip-worker",
            "--application-snapshot-worker",
        ]
        .iter()
        .any(|flag| arg == std::ffi::OsStr::new(flag))
    });
    #[cfg(test)]
    let gated_helper = gated_helper
        || command.get_args().take(2).eq([
            std::ffi::OsStr::new("--exact"),
            std::ffi::OsStr::new("owned_process::macos::tests::synthetic_child"),
        ]);
    if !gated_helper {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "macOS supervision requires a stdin-gated helper entry point",
        ));
    }
    // std performs setpgid(0, 0) before exec and propagates failure from spawn.
    // Never attach a running child to a group after it has begun domain work.
    command.process_group(0);
    Ok(())
}

#[derive(Default)]
struct State {
    observed: Option<ExitStatus>,
    reaped: bool,
    ownership_lost: bool,
}

pub(super) struct ProcessGroup {
    leader: libc::pid_t,
    identity: OnceLock<(u64, u64)>,
    state: Mutex<State>,
}

impl ProcessGroup {
    pub(super) fn new(child: &Child) -> io::Result<Self> {
        let leader = libc::pid_t::try_from(child.id()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid process group leader")
        })?;
        if leader <= 1 || leader == unsafe { libc::getpgrp() } {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsafe process group leader",
            ));
        }
        Ok(Self {
            leader,
            identity: OnceLock::new(),
            state: Mutex::new(State::default()),
        })
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        self.check_leader_group()?;
        let members = self.members()?;
        if !members.contains(&self.leader) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "process group leader absent from kernel list",
            ));
        }
        Ok(())
    }

    fn check_leader_group(&self) -> io::Result<()> {
        if self.leader <= 1 || self.leader == unsafe { libc::getpgrp() } {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "refusing host process group",
            ));
        }
        // getpgid uses proc_find and cannot inspect an exited Darwin leader.
        // PROC_PIDTBSDINFO with arg=1 explicitly includes the zombie list.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let bytes = unsafe {
            libc::proc_pidinfo(
                self.leader,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                std::mem::size_of_val(&info) as libc::c_int,
            )
        };
        if bytes as usize != std::mem::size_of_val(&info) {
            return Err(io::Error::last_os_error());
        }
        let identity = (info.pbi_start_tvsec, info.pbi_start_tvusec);
        let expected = self.identity.get_or_init(|| identity);
        if info.pbi_pid != self.leader as u32
            || info.pbi_ppid != unsafe { libc::getpid() } as u32
            || info.pbi_pgid != self.leader as u32
            || *expected != identity
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "worker left its owned process group",
            ));
        }
        Ok(())
    }

    fn members(&self) -> io::Result<Vec<libc::pid_t>> {
        let mut members = vec![0 as libc::pid_t; MAX_GROUP_PROCESSES];
        let bytes = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                self.leader as u32,
                members.as_mut_ptr().cast(),
                std::mem::size_of_val(members.as_slice()) as libc::c_int,
            )
        };
        let capacity_bytes = std::mem::size_of_val(members.as_slice());
        // There must always be our unreaped leader, including its zombie. Zero
        // is ambiguous in libproc; a full buffer may have omitted more members.
        if bytes <= 0
            || bytes as usize >= capacity_bytes
            || bytes as usize % std::mem::size_of::<libc::pid_t>() != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "process group enumeration incomplete",
            ));
        }
        members.truncate(bytes as usize / std::mem::size_of::<libc::pid_t>());
        if members.iter().any(|pid| *pid <= 1) || !members.contains(&self.leader) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "process group enumeration inconsistent",
            ));
        }
        Ok(members)
    }

    pub(super) fn kill(&self) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.reaped
            || state.ownership_lost
            || self.leader <= 1
            || self.leader == unsafe { libc::getpgrp() }
        {
            return;
        }
        // The leader is not reaped while descendants remain, so its PGID
        // cannot have been reassigned to an unrelated process group.
        if self.check_leader_group().is_ok() {
            unsafe { libc::kill(-self.leader, libc::SIGKILL) };
        }
        // A failed identity query or a group mismatch is unknown. In
        // particular, waitid success alone must not authorize killing a PID
        // that an external reaper may have recycled into a different child.
    }

    pub(super) fn empty(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).reaped
    }

    pub(super) fn try_wait(&self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.reaped {
            return Ok(state.observed);
        }
        if state.ownership_lost || child.id() != self.leader as u32 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "process group ownership lost",
            ));
        }
        if state.observed.is_none() {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.leader as libc::id_t,
                    &mut info,
                    libc::WNOHANG | libc::WNOWAIT | libc::WEXITED,
                )
            } != 0
            {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) {
                    // Someone reaped through Child or changed SIGCHLD policy.
                    // Never reuse the numeric group/PID or release the permit.
                    state.ownership_lost = true;
                }
                return Err(error);
            }
            if info.si_pid == 0 {
                return Ok(None);
            }
            if info.si_pid != self.leader {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "wrong child exit",
                ));
            }
            // Darwin sys/signal.h CLD_EXITED/KILLED/DUMPED. siginfo si_status is
            // the exit code or signal, not the raw waitpid status encoding.
            let raw = match info.si_code {
                1 => (info.si_status & 0xff) << 8,
                2 => info.si_status & 0x7f,
                3 => (info.si_status & 0x7f) | 0x80,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "wrong exit event",
                    ))
                }
            };
            state.observed = Some(ExitStatus::from_raw(raw));
        }
        if self.check_leader_group().is_ok()
            && self.members().is_ok_and(|members| members == [self.leader])
        {
            // An exited leader cannot fork. With no descendants (even zombies)
            // left, this last reap ends ownership; future signals are disabled.
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.observed = Some(status);
                    state.reaped = true;
                }
                Ok(None) => {}
                Err(error) => {
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        state.ownership_lost = true;
                    }
                    return Err(error);
                }
            }
        }
        Ok(state.observed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_process::ManagedProcess;
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant},
    };

    const CHILD_TEST: &str = "owned_process::macos::tests::synthetic_child";
    const CHILD_MODE: &str = "MEWU_DARWIN_PROCESS_TEST_CHILD";
    struct TestPermit(Arc<AtomicBool>);
    impl Drop for TestPermit {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    fn command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env(CHILD_MODE, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }
    fn spawn(
        mode: &str,
    ) -> (
        ManagedProcess<TestPermit>,
        Arc<AtomicBool>,
        mpsc::Receiver<String>,
    ) {
        let released = Arc::new(AtomicBool::new(false));
        let mut process =
            ManagedProcess::spawn(&mut command(mode), TestPermit(released.clone())).unwrap();
        let stdout = process.child_mut().stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        process.track_io(std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if let Ok(line) = line {
                    if let Some(marker) = line.strip_prefix("MEWU_TEST:") {
                        let _ = tx.send(marker.into());
                    }
                } else {
                    break;
                }
            }
        }));
        (process, released, rx)
    }
    fn go(process: &mut ManagedProcess<TestPermit>) {
        process
            .child_mut()
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
    }
    fn until(mut predicate: impl FnMut() -> bool) {
        let started = Instant::now();
        while !predicate() {
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "process lifetime deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn marker(message: &str) {
        // libtest may have written a partial "test name ... " line before
        // entering the fixture. Start our plain-text marker on its own line.
        println!("\nMEWU_TEST:{message}");
        std::io::stdout().flush().unwrap();
    }
    fn gate() {
        let mut line = String::new();
        assert_eq!(std::io::stdin().read_line(&mut line).unwrap(), 3);
        assert_eq!(line, "go\n");
    }

    #[test]
    fn synthetic_child() {
        let Ok(mode) = std::env::var(CHILD_MODE) else {
            return;
        };
        if mode == "descendant" {
            marker("descendant_ready");
            // A finite failsafe avoids a stray fixture even if assertions fail.
            std::thread::sleep(Duration::from_secs(20));
            std::process::exit(0);
        }
        if mode == "exit_descendant" {
            std::process::exit(0);
        }
        gate();
        match mode.as_str() {
            "exit" => std::process::exit(7),
            "hang" => {
                marker("domain_started");
                std::thread::sleep(Duration::from_secs(20));
                std::process::exit(0);
            }
            "parent_first" => {
                let mut descendant = command("descendant");
                descendant.stdin(Stdio::null()).stdout(Stdio::inherit());
                let child = descendant.spawn().unwrap();
                marker(&format!("descendant_pid:{}", child.id()));
                // Intentionally leave a real descendant holding stdout after
                // the direct child exits. No process_group override is applied.
                std::process::exit(0);
            }
            "zombie" => {
                let mut descendant = command("exit_descendant");
                descendant.stdin(Stdio::null()).stdout(Stdio::null());
                let mut child = descendant.spawn().unwrap();
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                assert_eq!(
                    unsafe {
                        libc::waitid(
                            libc::P_PID,
                            child.id() as libc::id_t,
                            &mut info,
                            libc::WEXITED | libc::WNOWAIT,
                        )
                    },
                    0
                );
                marker(&format!("zombie_pid:{}", child.id()));
                gate();
                child.wait().unwrap();
                std::process::exit(0);
            }
            _ => panic!("unexpected synthetic process mode"),
        }
    }

    #[test]
    fn spawn_establishes_distinct_group_before_stdin_domain_work() {
        let (mut process, released, events) = spawn("hang");
        let pid = process.child_mut().id() as libc::pid_t;
        assert_eq!(unsafe { libc::getpgid(pid) }, pid);
        assert_ne!(pid, unsafe { libc::getpgrp() });
        assert!(events.recv_timeout(Duration::from_millis(80)).is_err());
        assert!(process.try_wait().unwrap().is_none());
        go(&mut process);
        assert_eq!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            "domain_started"
        );
        process.kill();
        until(|| process.fully_exited());
        assert!(process.try_wait().unwrap().unwrap().signal().is_some());
        assert!(!released.load(Ordering::Acquire));
        drop(process);
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn normal_exit_reaps_only_after_group_drains() {
        let (mut process, released, _) = spawn("exit");
        go(&mut process);
        until(|| process.fully_exited());
        assert_eq!(process.try_wait().unwrap().unwrap().code(), Some(7));
        assert!(process.tree_empty());
        process.kill(); // Repeated calls after reap must not signal a reused PID.
        drop(process);
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn failed_spawn_releases_permit_without_starting_worker() {
        let released = Arc::new(AtomicBool::new(false));
        let mut command = command("hang");
        command.current_dir(
            std::env::temp_dir().join(format!("mewu-absent-{}", uuid::Uuid::new_v4())),
        );
        assert!(ManagedProcess::spawn(&mut command, TestPermit(released.clone())).is_err());
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn arbitrary_executable_or_non_worker_entry_point_is_rejected() {
        for mut command in [
            Command::new("/bin/sh"),
            Command::new(std::env::current_exe().unwrap()),
        ] {
            let released = Arc::new(AtomicBool::new(false));
            assert_eq!(
                ManagedProcess::spawn(&mut command, TestPermit(released.clone()))
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::Unsupported
            );
            assert!(released.load(Ordering::Acquire));
        }
    }

    #[test]
    fn tracked_io_holds_permit_after_parent_and_group_exit() {
        let (mut process, released, _) = spawn("exit");
        let (tx, rx) = mpsc::channel::<()>();
        process.track_io(std::thread::spawn(move || {
            let _ = rx.recv();
        }));
        go(&mut process);
        until(|| process.try_wait().unwrap().is_some() && process.tree_empty());
        assert!(!process.fully_exited());
        drop(process);
        assert!(!released.load(Ordering::Acquire));
        tx.send(()).unwrap();
        until(|| released.load(Ordering::Acquire));
    }

    #[test]
    fn parent_first_exit_retains_descendant_pipe_and_permit_until_drop_cleanup() {
        let (mut process, released, events) = spawn("parent_first");
        go(&mut process);
        let mut descendant_pid = None;
        let mut descendant_ready = false;
        for _ in 0..2 {
            let event = events.recv_timeout(Duration::from_secs(3)).unwrap();
            if let Some(pid) = event.strip_prefix("descendant_pid:") {
                descendant_pid = Some(pid.parse::<libc::pid_t>().unwrap());
            }
            descendant_ready |= event == "descendant_ready";
        }
        assert!(descendant_ready);
        let descendant = descendant_pid.unwrap();
        until(|| process.try_wait().unwrap().is_some());
        assert!(!process.tree_empty());
        assert!(!process.fully_exited());
        assert!(!released.load(Ordering::Acquire));
        assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
        drop(process);
        until(|| released.load(Ordering::Acquire));
        assert_eq!(unsafe { libc::kill(descendant, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn elapsed_deadline_drop_kills_live_group_without_releasing_io_early() {
        let (mut process, released, events) = spawn("hang");
        go(&mut process);
        assert_eq!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            "domain_started"
        );
        let (tx, rx) = mpsc::channel::<()>();
        process.track_io(std::thread::spawn(move || {
            let _ = rx.recv();
        }));
        let deadline = Instant::now() + Duration::from_millis(30);
        while Instant::now() < deadline {
            assert!(process.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(process);
        assert!(!released.load(Ordering::Acquire));
        tx.send(()).unwrap();
        until(|| released.load(Ordering::Acquire));
    }

    #[test]
    fn darwin_group_list_includes_unreaped_descendant_zombie() {
        let (mut process, released, events) = spawn("zombie");
        go(&mut process);
        let event = events.recv_timeout(Duration::from_secs(3)).unwrap();
        let zombie = event
            .strip_prefix("zombie_pid:")
            .unwrap()
            .parse::<libc::pid_t>()
            .unwrap();
        let job = process.0.as_ref().unwrap().job.as_ref().unwrap();
        let members = job.members().unwrap();
        assert!(members.contains(&zombie));
        assert_eq!(members.len(), 2);
        assert!(!job.empty());
        go(&mut process);
        until(|| process.fully_exited());
        drop(process);
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn external_reap_is_unknown_and_host_group_is_never_owned() {
        let mut command = command("exit");
        prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let group = ProcessGroup::new(&child).unwrap();
        group.verify().unwrap();
        child.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
        child.wait().unwrap();
        assert!(group.try_wait(&mut child).is_err());
        assert!(!group.empty());
        group.kill(); // Ownership lost: deliberately sends no signal.
        let host_group = ProcessGroup {
            leader: unsafe { libc::getpgrp() },
            identity: OnceLock::new(),
            state: Mutex::new(State::default()),
        };
        assert!(host_group.verify().is_err());
        host_group.kill();
        assert!(!host_group.empty());
    }
}
