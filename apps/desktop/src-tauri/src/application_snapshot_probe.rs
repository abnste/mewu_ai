// SPDX-License-Identifier: MPL-2.0
//! Explicit Windows integration tests use a separate owned synthetic window.
//! No desktop capture, real application control, credentials or user DB access.
use crate::{
    application_snapshot::{self, Session, Viewport},
    application_snapshot_windows::NativeSession,
    owned_process::ManagedProcess,
    window_snap,
};
use serde::Deserialize;
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
struct Source {
    process: ManagedProcess<()>,
    root: std::path::PathBuf,
    close: std::path::PathBuf,
    window: window_snap::SnapshotWindow,
}
impl Source {
    fn new() -> Self {
        use std::os::windows::process::CommandExt;
        let root =
            std::env::temp_dir().join(format!("mewu-owned-snapshot-{}", uuid::Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("source.ps1");
        let ready = root.join("ready.json");
        let close = root.join("close");
        std::fs::write(
            &script,
            include_str!("../tests/fixtures/application-snapshot-window.ps1"),
        )
        .unwrap();
        let mut command =
            Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-STA",
                "-File",
            ])
            .arg(&script)
            .arg("-ReadyPath")
            .arg(&ready)
            .arg("-ClosePath")
            .arg(&close)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000);
        let mut process = ManagedProcess::spawn(&mut command, ()).unwrap();
        process
            .child_mut()
            .stdin
            .take()
            .unwrap()
            .write_all(b"start\n")
            .unwrap();
        let started = Instant::now();
        while !ready.exists() {
            if process.try_wait().unwrap().is_some() {
                panic!("owned source failed to start");
            }
            assert!(
                started.elapsed() < Duration::from_secs(12),
                "owned source readiness timeout"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Ready {
            handle: usize,
            process_id: u32,
        }
        let ready: Ready = serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
        assert_eq!(ready.process_id, process.child_mut().id());
        let window =
            window_snap::snapshot_test_window(ready.handle).expect("owned window identity");
        assert_eq!(window.process, ready.process_id);
        assert!(window.is_current());
        Self {
            process,
            root,
            close,
            window,
        }
    }
    fn selection(&self) -> Viewport {
        Viewport {
            x: self.window.x,
            y: self.window.y,
            width: self.window.width,
            height: self.window.height,
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.close, b"close");
        let started = Instant::now();
        while !self.process.fully_exited() {
            if started.elapsed() > Duration::from_secs(3) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for name in ["source.ps1", "ready.json", "close"] {
            let _ = std::fs::remove_file(self.root.join(name));
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}
#[test]
#[ignore = "real Windows UIA + WGC on a separate owned synthetic WPF window; no desktop or user data"]
fn real_window_capture_stitches_owned_pixels_and_restores_original_position_on_success_and_cancel()
{
    let source = Source::new();
    let mut session = NativeSession::new(source.window.clone(), source.selection()).unwrap();
    let original = session.original().clone();
    assert!(original.vertical > 0.0);
    let mut frames = 0;
    let image = application_snapshot::capture(&mut session, &AtomicBool::new(false), |_, _| {
        frames += 1;
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert!(frames >= 4);
    assert_eq!(image.width(), original.viewport.width);
    assert!(image.height() > original.viewport.height * 3);
    assert!(original.restored(&session.restore().unwrap()));
    drop(session);
    let mut session = NativeSession::new(source.window.clone(), source.selection()).unwrap();
    let before = session.original().clone();
    assert!(original.restored(&before));
    let cancel = Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let result = application_snapshot::capture(&mut session, &cancel, |_, _| {
        token.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    })
    .unwrap();
    assert!(result.is_none());
    assert!(before.restored(&session.restore().unwrap()));
}

#[test]
#[ignore = "requires the exact freshly compiled app EXE; owned UIA/WGC worker IPC, no user storage"]
fn real_supervised_worker_restores_before_publication_and_keeps_its_slot_until_process_and_io_exit()
{
    use crate::application_snapshot_worker::{self, WorkerSession};
    let executable = std::path::PathBuf::from(
        std::env::var_os("MEWU_SNAPSHOT_PROBE_EXECUTABLE")
            .expect("explicit freshly built app path"),
    );
    assert!(executable.is_absolute() && executable.is_file());
    let source = Source::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut session = WorkerSession::start_probe(
        source.window.clone(),
        source.selection(),
        cancel.clone(),
        &executable,
    )
    .unwrap();
    let original = session.original().clone();
    assert!(original.vertical > 0.0);
    assert!(application_snapshot_worker::busy());
    assert!(WorkerSession::start_probe(
        source.window.clone(),
        source.selection(),
        cancel.clone(),
        &executable
    )
    .is_err());
    let image = application_snapshot::capture(&mut session, &cancel, |_, _| Ok(()))
        .unwrap()
        .unwrap();
    assert!(image.height() > original.viewport.height * 3);
    session.close().unwrap();
    drop(session);
    assert!(!application_snapshot_worker::busy());
    let cancel = Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let mut session = WorkerSession::start_probe(
        source.window.clone(),
        source.selection(),
        cancel.clone(),
        &executable,
    )
    .unwrap();
    assert!(original.restored(session.original()));
    let image = application_snapshot::capture(&mut session, &cancel, |_, _| {
        token.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    })
    .unwrap();
    assert!(image.is_none());
    session.close().unwrap();
    drop(session);
    assert!(!application_snapshot_worker::busy());
    let mut session = WorkerSession::start_probe(
        source.window.clone(),
        source.selection(),
        Arc::new(AtomicBool::new(false)),
        &executable,
    )
    .unwrap();
    assert!(original.restored(session.original()));
    session.frame_at(0.0, &AtomicBool::new(false)).unwrap();
    drop(session);
    assert!(!application_snapshot_worker::busy());
    let mut check = WorkerSession::start_probe(
        source.window.clone(),
        source.selection(),
        Arc::new(AtomicBool::new(false)),
        &executable,
    )
    .unwrap();
    assert!(original.restored(check.original()));
    check.restore().unwrap();
    check.close().unwrap();
    drop(check);
    assert!(!application_snapshot_worker::busy());
}
