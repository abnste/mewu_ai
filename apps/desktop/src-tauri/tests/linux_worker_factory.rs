// SPDX-License-Identifier: MPL-2.0
//! Native-only fixture for the actual sealed factory and actual OwnedParts.
//! Cross-compilation is preparation only, never a runtime PASS.
#[cfg(target_os = "linux")]
#[path = "../src/owned_process.rs"]
mod owned_process;
// Faults belong to this separately compiled fixture executable, never to the
// production bootstrap or its environment. The real factory/Parts run unchanged.
#[cfg(target_os = "linux")]
fn faulty_bootstrap() -> bool {
    use rustix::{
        io::{fcntl_setfd, FdFlags},
        net::{sendmsg, SendAncillaryBuffer, SendAncillaryMessage, SendFlags},
        process::{getpid, pidfd_open, PidfdFlags},
    };
    use std::{
        io::{IoSlice, Read},
        mem::MaybeUninit,
        os::unix::process::CommandExt,
        os::{
            fd::{AsFd, FromRawFd},
            unix::net::UnixStream,
        },
        process::Command,
        time::Duration,
    };
    let args: Vec<_> = std::env::args_os().collect();
    if !args
        .get(1)
        .is_some_and(|v| v == "--mewu-linux-worker-bootstrap")
    {
        return false;
    }
    let Ok(fault) = std::env::var("MEWU_FACTORY_FIXTURE_FAULT") else {
        return false;
    };
    assert_eq!(
        std::env::var("MEWU_LINUX_WORKER_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    assert_eq!(args.len(), 6);
    assert_eq!(getpid().as_raw_nonzero().get(), 1);
    let bridge: i32 = args[3].to_str().unwrap().parse().unwrap();
    let source: i32 = args[4].to_str().unwrap().parse().unwrap();
    assert!(bridge >= 3 && source >= 3 && bridge != source);
    assert!(unsafe { libc::fcntl(bridge, libc::F_GETFD) } >= 0);
    assert!(unsafe { libc::fcntl(source, libc::F_GETFD) } >= 0);
    let mut socket = unsafe { UnixStream::from_raw_fd(bridge) };
    let source_file = unsafe { std::fs::File::from_raw_fd(source) };
    fcntl_setfd(&socket, FdFlags::CLOEXEC).unwrap();
    fcntl_setfd(&source_file, FdFlags::CLOEXEC).unwrap();
    if fault == "bridge-eof" {
        return true;
    }
    socket
        .set_read_timeout(Some(Duration::from_millis(400)))
        .unwrap();
    if fault == "missing-receipt" || fault == "cancel-during-receipt" {
        if fault == "cancel-during-receipt" {
            let path = std::env::var_os("MEWU_FACTORY_FIXTURE_READY").unwrap();
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
        }
        let mut byte = [0];
        let _ = socket.read(&mut byte);
        return true;
    }
    let owner = if fault == "wrong-cmsg" {
        rustix::io::fcntl_dupfd_cloexec(std::fs::File::open("/dev/null").unwrap(), 3).unwrap()
    } else {
        pidfd_open(getpid(), PidfdFlags::empty()).unwrap()
    };
    let fds = [owner.as_fd()];
    let mut storage = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut storage);
    assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
    let tag = if fault == "wrong-tag" { b'X' } else { b'O' };
    assert_eq!(
        sendmsg(
            &socket,
            &[IoSlice::new(&[tag])],
            &mut ancillary,
            SendFlags::NOSIGNAL
        )
        .unwrap(),
        1
    );
    if matches!(fault.as_str(), "owner-early-exit" | "owner-dead-before-ack") {
        return true;
    }
    let mut byte = [0];
    let result = socket.read_exact(&mut byte);
    if fault == "post-ack-exec-error" {
        result.unwrap();
        assert_eq!(byte, [b'A']);
        // Inject failure at the same final FD exec boundary, after the parent
        // admitted the real PID1. This is fixture-only, not a production hook.
        drop((owner, socket, source_file));
        let error = Command::new(format!("/proc/self/fd/{source}"))
            .arg("--code-worker")
            .exec();
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
        eprintln!("factory fixture actual final executable FD unavailable after ACK");
        return true;
    }
    assert!(
        result.is_err() || byte == [b'N'],
        "malformed receipt must not receive ACK"
    );
    true
}

#[cfg(target_os = "linux")]
fn failure_controller(case: &str) {
    use crate::owned_process::linux_worker::{TrustedCommand, Worker};
    use std::{
        process::Stdio,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    struct Permit(Arc<AtomicUsize>);
    impl Drop for Permit {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let released = Arc::new(AtomicUsize::new(0));
    let mut command = TrustedCommand::new(Worker::Code).unwrap();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .env("MEWU_FACTORY_FIXTURE_FAULT", case);
    let cancel = Arc::new(AtomicBool::new(case == "cancel-before-exec"));
    let now = Instant::now();
    let deadline = if case == "expiry-before-exec" {
        now
    } else {
        now + if case == "missing-receipt" {
            Duration::from_millis(150)
        } else {
            Duration::from_secs(3)
        }
    };
    let timer = if case == "cancel-during-receipt" {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("mewu-factory-cancel-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let marker = directory.join("bootstrap-entered-no-receipt");
        command.env("MEWU_FACTORY_FIXTURE_READY", &marker);
        let cancel = cancel.clone();
        Some(std::thread::spawn(move || {
            while !marker.exists() {
                assert!(
                    Instant::now() < deadline,
                    "bootstrap did not reach admission fixture"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            cancel.store(true, Ordering::Release);
            std::fs::remove_file(marker).unwrap();
            std::fs::remove_dir(directory).unwrap();
        }))
    } else {
        None
    };
    let result = if case == "owner-dead-before-ack" {
        let mut owned = owned_process::ManagedProcess::prepare_unacknowledged_for_fixture(
            command,
            Permit(released.clone()),
            deadline,
            &cancel,
        )
        .unwrap();
        while owned.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "original wrapper did not exit before admission"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "unadmitted Parts must retain P"
        );
        let error = owned.admit_for_fixture(deadline, &cancel).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        assert!(
            owned.tree_empty(),
            "received dead PID1 kernel witness must prove completion"
        );
        assert!(
            owned.fully_exited(),
            "actual original wait and IO completion required"
        );
        drop(owned);
        Err(error)
    } else {
        owned_process::ManagedProcess::spawn_trusted(
            command,
            Permit(released.clone()),
            deadline,
            &cancel,
        )
    };
    if let Some(timer) = timer {
        timer.join().unwrap();
    }
    match result {
        Ok(owned) => {
            assert!(matches!(case, "owner-early-exit" | "post-ack-exec-error"));
            drop(owned);
        }
        Err(error) => match case {
            "cancel-before-exec" | "cancel-during-receipt" => {
                assert_eq!(error.kind(), std::io::ErrorKind::Interrupted)
            }
            "expiry-before-exec" | "missing-receipt" => {
                assert_eq!(error.kind(), std::io::ErrorKind::TimedOut)
            }
            "bridge-eof"
            | "wrong-cmsg"
            | "wrong-tag"
            | "owner-early-exit"
            | "owner-dead-before-ack" => (),
            _ => panic!("unexpected admission error for {case}: {error}"),
        },
    }
    let known = matches!(
        case,
        "cancel-before-exec"
            | "expiry-before-exec"
            | "owner-early-exit"
            | "owner-dead-before-ack"
            | "post-ack-exec-error"
    );
    if known {
        let limit = Instant::now() + Duration::from_secs(1);
        while released.load(Ordering::SeqCst) != 1 {
            assert!(
                Instant::now() < limit,
                "verified owner cleanup did not release P"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        eprintln!("factory native failure {case}: confirmed no spawn or actual known-owner completion; P released PASS");
    } else {
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "unproven receipt must retain P"
        );
        eprintln!("factory native failure {case}: UNPROVEN DRAIN, actual P retained; conservative failure assertion PASS (not exit/update acceptance)");
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("native Linux factory fixture cannot run on this platform");
}
#[cfg(target_os = "linux")]
fn resource_controller(case: &str) {
    use crate::owned_process::linux_worker::{TrustedCommand, Worker};
    use std::{
        io::{Read, Write},
        process::Stdio,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let root = std::path::PathBuf::from(std::env::var_os("APPDIR").unwrap());
    let captured = TrustedCommand::new(Worker::Code);
    if case != "valid-env" && case != "replace-after-capture" {
        assert!(captured.is_err(), "unsafe AppDir was accepted: {case}");
        eprintln!("factory resource {case}: rejected before any child exec PASS");
        return;
    }
    let mut command = captured.unwrap();
    if case == "replace-after-capture" {
        let binary = root.join("usr/bin/MewuAI");
        std::fs::rename(&binary, root.join("original-loaded-inode")).unwrap();
        std::fs::write(&binary, b"INERT REPLACEMENT").unwrap();
    }
    command
        .env_clear()
        .env("MEWU_LINUX_WORKER_TEST_ISOLATED", "1")
        .env("MEWU_FACTORY_RESOURCE_ROOT", &root)
        .env("LD_LIBRARY_PATH", "/nonexistent/untrusted-loader")
        .env("LD_PRELOAD", "/nonexistent/untrusted-preload")
        .env("LD_AUDIT", "/nonexistent/untrusted-audit")
        .env("GLIBC_TUNABLES", "untrusted")
        .env("APPDIR", "/nonexistent/overwritten-root")
        .env("GIO_MODULE_DIR", "/nonexistent/overwritten-modules")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    struct Permit(Arc<AtomicUsize>);
    impl Drop for Permit {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let released = Arc::new(AtomicUsize::new(0));
    let deadline = Instant::now() + Duration::from_secs(3);
    let result = owned_process::ManagedProcess::spawn_trusted(
        command,
        Permit(released.clone()),
        deadline,
        &AtomicBool::new(false),
    );
    if case == "replace-after-capture" {
        assert!(
            result.is_err(),
            "replaced AppDir binary must fail before exec"
        );
        assert_eq!(released.load(Ordering::SeqCst), 1);
        eprintln!("factory resource {case}: launch rechecked pinned identity before exec; P released PASS");
        return;
    }
    let mut owned = result.unwrap();
    let mut stdout = owned.child_mut().stdout.take().unwrap();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    owned.track_io(std::thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .by_ref()
            .take(1024)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = output_tx.send(result);
    }));
    owned
        .child_mut()
        .stdin
        .take()
        .unwrap()
        .write_all(b"P")
        .unwrap();
    while !owned.fully_exited() {
        assert!(
            Instant::now() < deadline,
            "real resource worker/IO did not finish"
        );
        let _ = owned.try_wait().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(owned.try_wait().unwrap().unwrap().success());
    assert_eq!(
        output_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
            .unwrap(),
        b"trusted-domain\n"
    );
    drop(owned);
    assert_eq!(released.load(Ordering::SeqCst), 1);
    eprintln!("factory resource valid-env: real worker received sealed resources after env_clear/tainted overrides; actual PID1 wait + IO join PASS");
}

#[cfg(target_os = "linux")]
fn resource_fixtures() {
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        process::Command,
    };
    let directories = [
        "usr",
        "usr/bin",
        "usr/lib",
        "usr/lib/gtk-3.0",
        "usr/lib/gtk-3.0/3.0.0",
        "usr/lib/gdk-pixbuf-2.0",
        "usr/lib/gdk-pixbuf-2.0/2.10.0",
        "usr/lib/girepository-1.0",
        "usr/lib/gio",
        "usr/lib/gio/modules",
        "usr/lib/gstreamer-1.0",
        "usr/lib/gstreamer1.0",
        "usr/lib/gstreamer1.0/gstreamer-1.0",
        "usr/share",
        "usr/share/glib-2.0",
        "usr/share/glib-2.0/schemas",
    ];
    let files = [
        "usr/lib/gtk-3.0/3.0.0/immodules.cache",
        "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache",
        "usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner",
        "usr/lib/gstreamer1.0/gstreamer-1.0/gst-ptp-helper",
        "usr/lib/libfixture.so.1",
    ];
    for case in [
        "valid-env",
        "root-other-writable",
        "resource-other-writable",
        "symlink-escape",
        "missing-resource",
        "wrong-loaded-inode",
        "replace-after-capture",
    ] {
        let root = std::env::temp_dir().join(format!(
            "mewu-resource-fixture-{}-{case}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for relative in directories {
            let path = root.join(relative);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        for relative in files {
            let path = root.join(relative);
            std::fs::write(&path, b"fixture resource, never executed").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        symlink("libfixture.so.1", root.join("usr/lib/libfixture.so")).unwrap();
        let executable = root.join("usr/bin/MewuAI");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        match case {
            "root-other-writable" => {
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap()
            }
            "resource-other-writable" => std::fs::set_permissions(
                root.join(files[0]),
                std::fs::Permissions::from_mode(0o666),
            )
            .unwrap(),
            "symlink-escape" => symlink("/usr/lib", root.join("usr/lib/escape")).unwrap(),
            "missing-resource" => std::fs::remove_file(root.join(files[0])).unwrap(),
            "wrong-loaded-inode" => {
                std::fs::copy(&executable, root.join("other-executable")).unwrap();
            }
            _ => (),
        }
        let launch = if case == "wrong-loaded-inode" {
            root.join("other-executable")
        } else {
            executable.clone()
        };
        let status = Command::new(launch)
            .args(["--factory-resource-controller", case])
            .env("APPDIR", &root)
            .status()
            .unwrap();
        assert!(status.success(), "native resource fixture failed: {case}");
        // Remove only this fixture's explicit paths; never follow test symlinks.
        for relative in [
            "usr/lib/libfixture.so",
            "usr/lib/escape",
            "other-executable",
            "original-loaded-inode",
        ] {
            let path = root.join(relative);
            if std::fs::symlink_metadata(&path).is_ok() {
                std::fs::remove_file(path).unwrap();
            }
        }
        std::fs::remove_file(executable).unwrap();
        for relative in files {
            let path = root.join(relative);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        for relative in directories.into_iter().rev() {
            std::fs::remove_dir(root.join(relative)).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
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
    if faulty_bootstrap() {
        std::process::exit(2);
    }
    if let Some(code) = factory::bootstrap_entry() {
        std::process::exit(code);
    }
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_some_and(|v| v == "--factory-resource-controller")
    {
        resource_controller(args[2].to_str().unwrap());
        return;
    }
    if args
        .get(1)
        .is_some_and(|v| v == "--factory-failure-controller")
    {
        failure_controller(args[2].to_str().unwrap());
        return;
    }
    if args.get(1).is_some_and(|v| v == "--code-worker") {
        if let Some(root) = std::env::var_os("MEWU_FACTORY_RESOURCE_ROOT") {
            let root = std::path::PathBuf::from(root);
            assert_eq!(
                std::env::var_os("APPDIR"),
                Some(root.clone().into_os_string())
            );
            assert_eq!(
                std::env::var_os("GIO_MODULE_DIR"),
                Some(root.join("usr/lib/gio/modules").into_os_string())
            );
            assert_eq!(
                std::env::var_os("GST_PLUGIN_PATH_1_0"),
                Some(root.join("usr/lib/gstreamer-1.0").into_os_string())
            );
            for name in [
                "LD_LIBRARY_PATH",
                "LD_PRELOAD",
                "LD_AUDIT",
                "GLIBC_TUNABLES",
            ] {
                assert!(
                    std::env::var_os(name).is_none(),
                    "tainted loader variable survived: {name}"
                );
            }
        }
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
    for case in [
        "cancel-before-exec",
        "expiry-before-exec",
        "cancel-during-receipt",
        "missing-receipt",
        "wrong-cmsg",
        "wrong-tag",
        "owner-early-exit",
        "owner-dead-before-ack",
        "post-ack-exec-error",
        "bridge-eof",
    ] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--factory-failure-controller", case])
            .status()
            .unwrap();
        assert!(
            status.success(),
            "native factory failure fixture {case} failed"
        );
    }
    resource_fixtures();
}
