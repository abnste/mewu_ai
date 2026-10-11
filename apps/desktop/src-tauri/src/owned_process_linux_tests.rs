// SPDX-License-Identifier: MPL-2.0
//! Explicit native prerequisite tests, not a production process adapter.
//! Every helper is this exact test executable. Receipts are kernel pidfds sent
//! by their owners, never a process scan or a numeric-PID liveness assertion.
use rustix::{
    event::{poll, PollFd, PollFlags, Timespec},
    net::{
        recvmsg, sendmsg, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
        SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
    },
    process::{getpid, pidfd_open, pidfd_send_signal, setsid, Pid, PidfdFlags, Signal},
};
use std::{
    collections::HashMap,
    io::{IoSlice, IoSliceMut, Read, Write},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            fs::DirBuilderExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const ENTRY: &str = "owned_process::linux_tests::native_linux_namespace_fixture";
const MARKER: &str = "MEWU_LINUX_WORKER_TEST_ISOLATED";
const MODE: &str = "MEWU_LINUX_WORKER_FIXTURE_MODE";
const SOCKET: &str = "MEWU_LINUX_WORKER_FIXTURE_SOCKET";
const LIMIT: Duration = Duration::from_secs(5);

fn send_owner(socket: &UnixStream, tag: u8) {
    let owner = pidfd_open(getpid(), PidfdFlags::empty()).expect("self pidfd");
    send_fd(socket, tag, &owner);
}

fn send_fd(socket: &UnixStream, tag: u8, owner: &OwnedFd) {
    let fds = [owner.as_fd()];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    assert!(control.push(SendAncillaryMessage::ScmRights(&fds)));
    assert_eq!(
        sendmsg(
            socket,
            &[IoSlice::new(&[tag])],
            &mut control,
            SendFlags::NOSIGNAL
        )
        .expect("send actual owned pidfd"),
        1
    );
}

fn receive_owner(socket: &UnixStream) -> (u8, OwnedFd) {
    socket.set_read_timeout(Some(LIMIT)).unwrap();
    let mut tag = [0u8];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let result = recvmsg(
        socket,
        &mut [IoSliceMut::new(&mut tag)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC,
    )
    .expect("receive actual owned pidfd");
    assert_eq!(result.bytes, 1);
    assert!(!result
        .flags
        .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC));
    let mut owners = Vec::new();
    for message in control.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(fds) => owners.extend(fds),
            _ => panic!("unexpected owner receipt"),
        }
    }
    assert_eq!(owners.len(), 1);
    (tag[0], owners.pop().unwrap())
}

fn exited(owner: &OwnedFd) -> bool {
    let mut fds = [PollFd::new(owner, PollFlags::IN)];
    match poll(
        &mut fds,
        Some(&Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    ) {
        Ok(_) => fds[0].revents().contains(PollFlags::IN),
        Err(rustix::io::Errno::INTR) => false,
        Err(e) => panic!("pidfd poll failed: {e}"),
    }
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < LIMIT, "real owner/IO did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fixed_helper(mode: &str, path: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", ENTRY, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env(MARKER, "1")
        .env(MODE, mode)
        .env(SOCKET, path);
    command
}

fn fixed_namespace(path: &Path) -> Command {
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
            "--",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", ENTRY, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env(MARKER, "1")
        .env(MODE, "owner")
        .env(SOCKET, path);
    command
}

/// The escaped child keeps stdout open; its grandchild deliberately closes all
/// stdio. Both send their own immutable pidfd before the test continues.
fn helper(mode: &str, path: &Path) -> ! {
    let mut socket = UnixStream::connect(path).unwrap();
    match mode {
        "owner" => {
            assert_eq!(getpid().as_raw_nonzero().get(), 1, "must be actual PID 1");
            send_owner(&socket, b'O');
            let mut child = fixed_helper("escaped", path)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            // The original config pipe is kept exclusively by this PID 1.
            let mut request = [0];
            std::io::stdin().read_exact(&mut request).unwrap();
            assert_eq!(request, [b'X']);
            // Deliberately leave the escaped subtree alive for kernel teardown.
            assert!(child.try_wait().unwrap().is_none());
            std::process::exit(0);
        }
        "escaped" => {
            setsid().expect("escape the original process group");
            send_owner(&socket, b'E');
            let mut child = fixed_helper("closed-stdio", path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut request = [0];
            let _ = socket.read(&mut request);
            // On a broken test parent, bound the fixture rather than daemonize.
            let _ = child.kill();
            let _ = child.wait();
            std::process::exit(0);
        }
        "closed-stdio" => {
            send_owner(&socket, b'L');
            let mut request = [0];
            let _ = socket.read(&mut request);
            std::process::exit(0);
        }
        "controller" => {
            send_owner(&socket, b'C');
            let mut child = fixed_namespace(path)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            // This direct child is still unreaped and exclusively owned here;
            // its numeric PID cannot be reused before we capture its pidfd.
            let outer = pidfd_open(
                Pid::from_raw(child.id() as i32).unwrap(),
                PidfdFlags::empty(),
            )
            .expect("unreaped owned wrapper pidfd");
            let outer_receipt = UnixStream::connect(path).unwrap();
            send_fd(&outer_receipt, b'B', &outer);
            let _ = child.wait();
            std::process::exit(0);
        }
        _ => panic!("unknown fixed fixture mode"),
    }
}

struct Fixture {
    child: Child,
    dir: PathBuf,
    owners: HashMap<u8, OwnedFd>,
    sockets: Vec<UnixStream>,
    output: Option<std::thread::JoinHandle<Vec<u8>>>,
}
impl Fixture {
    fn start(controller: bool) -> Self {
        let dir = std::env::temp_dir().join(format!("mewu-linux-owner-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("receipt.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut command = if controller {
            fixed_helper("controller", &path)
        } else {
            fixed_namespace(&path)
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let output = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(1_048_577).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() <= 1_048_576, "fixture output exceeded bound");
            bytes
        });
        let mut fixture = Self {
            child,
            dir,
            owners: HashMap::new(),
            sockets: Vec::new(),
            output: Some(output),
        };
        let expected = if controller { 5 } else { 3 };
        wait_for(|| {
            match listener.accept() {
                Ok((socket, _)) => {
                    let (tag, owner) = receive_owner(&socket);
                    assert!(matches!(tag, b'O' | b'E' | b'L' | b'C' | b'B'));
                    assert!(fixture.owners.insert(tag, owner).is_none());
                    fixture.sockets.push(socket);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(e) => panic!("receipt accept failed: {e}"),
            }
            fixture.owners.len() == expected
        });
        assert!(fixture.owners.values().all(|owner| !exited(owner)));
        assert!(fixture.child.try_wait().unwrap().is_none());
        assert!(
            !fixture.output.as_ref().unwrap().is_finished(),
            "escaped child must retain actual stdout"
        );
        fixture
    }
    fn finish(&mut self) {
        wait_for(|| self.owners.values().all(exited));
        wait_for(|| self.child.try_wait().unwrap().is_some());
        wait_for(|| self.output.as_ref().unwrap().is_finished());
        let _ = self.output.take().unwrap().join().unwrap();
        // All known owners, including the stdio-free grandchild, are signaled;
        // the actual original child is reaped and the actual reader is joined.
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(owner) = self.owners.get(&b'O') {
            let _ = pidfd_send_signal(owner, Signal::KILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.sockets.clear();
        // This exclusively created test directory contains only our socket.
        let _ = std::fs::remove_file(self.dir.join("receipt.sock"));
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[test]
fn native_linux_namespace_fixture() {
    assert_eq!(
        std::env::var(MARKER).as_deref(),
        Ok("1"),
        "explicit isolated test marker required"
    );
    if let Ok(mode) = std::env::var(MODE) {
        let path = PathBuf::from(std::env::var_os(SOCKET).unwrap());
        helper(&mode, &path);
    }
    // Normal PID 1 exit, owner cancellation, wrapper cancellation, and actual
    // application-parent death. Never infer subtree completion from wrapper exit.
    for mode in [
        "normal",
        "owner-kill",
        "timeout",
        "wrapper-kill",
        "parent-death",
    ] {
        let mut fixture = Fixture::start(mode == "parent-death");
        match mode {
            "normal" => fixture
                .child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"X")
                .unwrap(),
            "owner-kill" => pidfd_send_signal(&fixture.owners[&b'O'], Signal::KILL).unwrap(),
            "timeout" => {
                let deadline = Instant::now() + Duration::from_millis(80);
                while Instant::now() < deadline {
                    assert!(fixture.owners.values().all(|owner| !exited(owner)));
                    assert!(!fixture.output.as_ref().unwrap().is_finished());
                    std::thread::sleep(Duration::from_millis(2));
                }
                pidfd_send_signal(&fixture.owners[&b'O'], Signal::KILL).unwrap();
            }
            "wrapper-kill" => fixture.child.kill().unwrap(),
            "parent-death" => pidfd_send_signal(&fixture.owners[&b'C'], Signal::KILL).unwrap(),
            _ => unreachable!(),
        }
        fixture.finish();
        eprintln!("namespace prerequisite {mode}: PID1/setsid child/closed-stdio grandchild exit + original wait + actual IO join PASS");
    }
}
