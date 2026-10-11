// SPDX-License-Identifier: MPL-2.0
//! Native-only fixture for the actual sealed factory and actual OwnedParts.
//! Cross-compilation is preparation only, never a runtime PASS.
#[cfg(target_os = "linux")]
#[path = "../src/owned_process.rs"]
mod owned_process;
#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("native Linux factory fixture cannot run on this platform");
}
#[cfg(target_os = "linux")]
fn main() {
    use crate::owned_process::linux_worker as factory;
    use std::{
        fs::File,
        io::{Read, Write},
        os::{fd::AsRawFd, unix::fs::PermissionsExt},
        process::{Command, Stdio},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    if let Some(code) = factory::bootstrap_entry() {
        std::process::exit(code);
    }
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|v| v == "--code-worker") {
        assert_eq!(rustix::process::getpid().as_raw_nonzero().get(), 1);
        // Strong whitelist assertion after the original worker exec: neither
        // known source/bridge nor intentionally inheritable parent FD survives.
        for fd in 3..512 {
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_GETFD) },
                -1,
                "unexpected inherited fd {fd}"
            );
        }
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"P");
        std::io::stdout().write_all(b"trusted-domain\n").unwrap();
        return;
    }
    assert_eq!(
        std::env::var("MEWU_LINUX_WORKER_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    if args.get(1).is_some_and(|v| v == "--factory-controller") {
        // This controller runs a private COPY of the compiled fixture. Pin
        // current loaded source first; replace its pathname before any spawn.
        let mut command = factory::TrustedCommand::new(factory::Worker::Code).unwrap();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let old = std::env::current_exe().unwrap();
        let renamed = old.with_file_name("original-loaded-inode");
        std::fs::rename(&old, &renamed).unwrap();
        std::fs::write(&old, b"REPLACED SOURCE PATH MUST NEVER EXECUTE\n").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Expose numerous non-CLOEXEC handles in the real parent. The factory
        // must remove all except the two explicit bootstrap handles and stdio.
        let mut unrelated = Vec::new();
        for _ in 0..32 {
            let f = File::open("/dev/null").unwrap();
            assert_eq!(unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETFD, 0) }, 0);
            unrelated.push(f);
        }
        struct Permit(Arc<AtomicUsize>);
        impl Drop for Permit {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let released = Arc::new(AtomicUsize::new(0));
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut owned = owned_process::ManagedProcess::spawn_trusted(
            command,
            Permit(released.clone()),
            deadline,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut stdout = owned.child_mut().stdout.take().unwrap();
        let (eof_tx, eof_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.by_ref().take(1024).read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"trusted-domain\n");
            eof_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        owned.track_io(reader);
        owned
            .child_mut()
            .stdin
            .take()
            .unwrap()
            .write_all(b"P")
            .unwrap();
        eof_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        while owned.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            owned.tree_empty(),
            "actual PID1 pidfd must prove kernel completion"
        );
        assert!(
            !owned.fully_exited(),
            "unjoined real reader must retain operation permit"
        );
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(owned);
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "Drop cannot release P before IO joins"
        );
        release_tx.send(()).unwrap();
        while released.load(Ordering::SeqCst) != 1 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(unrelated);
        eprintln!("factory native: pinned inode survives pathname replacement; unrelated/source/bridge FD closed; real PID1 wait + IO join + P retained PASS");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mewu-factory-fixture-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let executable = dir.join("current-fixture");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let status = Command::new(&executable)
        .arg("--factory-controller")
        .status()
        .unwrap();
    assert!(status.success(), "native factory controller failed");
    std::fs::remove_file(dir.join("original-loaded-inode")).unwrap();
    std::fs::remove_file(executable).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
