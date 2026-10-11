// SPDX-License-Identifier: MPL-2.0
//! UIA and WGC live in one supervised owned process. No UIA call can block the
//! application's UI thread. The protocol has one bounded outstanding frame.
use crate::{
    application_snapshot::{ScrollPosition, Session, Viewport, WindowFrame},
    owned_process::ManagedProcess,
    window_snap::SnapshotWindow,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
const HEADER_LIMIT: usize = 2048;
const PIXEL_LIMIT: usize = 4 * 1024 * 1024 * 4;
const START_TIMEOUT: Duration = Duration::from_secs(12);
const STEP_TIMEOUT: Duration = Duration::from_secs(8);
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Start {
        window: SnapshotWindow,
        selection: Viewport,
    },
    Step {
        position: f64,
    },
    Restore,
    Quit,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Ready {
        state: ScrollPosition,
    },
    Frame {
        state: ScrollPosition,
        length: usize,
    },
    Restored {
        state: ScrollPosition,
    },
    Failed,
}
fn write_header(value: &impl Serialize, out: &mut impl Write) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "应用快照消息无效")?;
    if bytes.is_empty() || bytes.len() > HEADER_LIMIT {
        return Err("应用快照消息过大".into());
    }
    out.write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| out.write_all(&bytes))
        .and_then(|_| out.flush())
        .map_err(|_| "应用快照连接已断开".into())
}
fn read_header<T: serde::de::DeserializeOwned>(input: &mut impl Read) -> Result<Option<T>, String> {
    let mut length = [0; 4];
    let first = input
        .read(&mut length[..1])
        .map_err(|_| "应用快照连接不可用")?;
    if first == 0 {
        return Ok(None);
    }
    input
        .read_exact(&mut length[1..])
        .map_err(|_| "应用快照消息未完成")?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > HEADER_LIMIT {
        return Err("应用快照消息长度无效".into());
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .map_err(|_| "应用快照消息未完成")?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "应用快照消息格式无效".into())
}
fn frame_length(state: &ScrollPosition, length: usize) -> Result<(), String> {
    state.validate()?;
    let expected = state.viewport.width as u64 * state.viewport.height as u64 * 4;
    if expected as usize != length || length > PIXEL_LIMIT {
        return Err("应用快照图像长度无效".into());
    }
    Ok(())
}
type Slot = Mutex<Option<Arc<AtomicBool>>>;
fn slot() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}
pub(crate) fn busy() -> bool {
    slot().lock().map_or(true, |s| s.is_some())
}
struct Permit(Arc<AtomicBool>);
impl Permit {
    fn acquire(cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let mut current = slot().lock().map_err(|_| "应用快照状态不可用")?;
        if current.is_some() {
            return Err("应用快照正在结束".into());
        }
        *current = Some(cancel.clone());
        Ok(Self(cancel))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut current) = slot().lock() {
            if current.as_ref().is_some_and(|t| Arc::ptr_eq(t, &self.0)) {
                *current = None;
            }
        }
    }
}
#[derive(Debug)]
enum Output {
    Value(Response, Option<image::RgbaImage>),
    Closed,
}
pub(crate) struct WorkerSession {
    process: ManagedProcess<Permit>,
    output: mpsc::Receiver<Output>,
    original: ScrollPosition,
    restored: bool,
    restore_attempted: bool,
    closed: bool,
    step_pending: bool,
}
impl WorkerSession {
    pub fn start(
        window: SnapshotWindow,
        selection: Viewport,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        Self::start_executable(
            window,
            selection,
            cancel,
            &std::env::current_exe().map_err(|_| "应用快照程序不可用")?,
        )
    }
    fn start_executable(
        window: SnapshotWindow,
        selection: Viewport,
        cancel: Arc<AtomicBool>,
        executable: &std::path::Path,
    ) -> Result<Self, String> {
        window.validate()?;
        if !window.is_current() || cancel.load(Ordering::Acquire) {
            return Err("应用窗口已改变，请重新截图".into());
        }
        let permit = Permit::acquire(cancel.clone())?;
        let mut command = Command::new(executable);
        command
            .arg("--application-snapshot-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut process =
            ManagedProcess::spawn(&mut command, permit).map_err(|_| "无法启动应用快照")?;
        let mut stdout = process
            .child_mut()
            .stdout
            .take()
            .ok_or("应用快照输出不可用")?;
        let (send, output) = mpsc::sync_channel(1);
        process.track_io(
            std::thread::Builder::new()
                .name("mewu-snapshot-output".into())
                .spawn(move || {
                    loop {
                        let Some(response) = read_header::<Response>(&mut stdout).ok().flatten()
                        else {
                            break;
                        };
                        let image = if let Response::Frame { state, length } = &response {
                            if frame_length(state, *length).is_err() {
                                break;
                            }
                            let mut bytes = vec![0; *length];
                            if stdout.read_exact(&mut bytes).is_err() {
                                break;
                            }
                            let Some(image) = image::RgbaImage::from_raw(
                                state.viewport.width,
                                state.viewport.height,
                                bytes,
                            ) else {
                                break;
                            };
                            Some(image)
                        } else {
                            None
                        };
                        if send.send(Output::Value(response, image)).is_err() {
                            return;
                        }
                    }
                    let _ = send.send(Output::Closed);
                })
                .map_err(|_| "应用快照读取线程不可用")?,
        );
        write_header(
            &Request::Start { window, selection },
            process
                .child_mut()
                .stdin
                .as_mut()
                .ok_or("应用快照输入不可用")?,
        )?;
        let response = receive(&output, START_TIMEOUT, Some(&cancel))?;
        let Output::Value(Response::Ready { state }, None) = response else {
            return Err("此应用不支持自动快照，请使用手动长截图".into());
        };
        state.validate()?;
        let viewport = state.viewport;
        if viewport.x < selection.x
            || viewport.y < selection.y
            || i64::from(viewport.x) + i64::from(viewport.width)
                > i64::from(selection.x) + i64::from(selection.width)
            || i64::from(viewport.y) + i64::from(viewport.height)
                > i64::from(selection.y) + i64::from(selection.height)
        {
            return Err("应用快照滚动区域越界".into());
        }
        Ok(Self {
            process,
            output,
            original: state,
            restored: false,
            restore_attempted: false,
            closed: false,
            step_pending: false,
        })
    }
    #[cfg(test)]
    pub(crate) fn start_probe(
        window: SnapshotWindow,
        selection: Viewport,
        cancel: Arc<AtomicBool>,
        executable: &std::path::Path,
    ) -> Result<Self, String> {
        Self::start_executable(window, selection, cancel, executable)
    }
    fn request(&mut self, request: &Request) -> Result<(), String> {
        write_header(
            request,
            self.process
                .child_mut()
                .stdin
                .as_mut()
                .ok_or("应用快照输入已关闭")?,
        )
    }
    pub fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        if !self.restored {
            return Err("应用快照未恢复滚动位置".into());
        }
        self.request(&Request::Quit)?;
        self.process.child_mut().stdin.take();
        let started = Instant::now();
        while !self.process.fully_exited() {
            if started.elapsed() > Duration::from_secs(2) {
                return Err("应用快照采集仍在结束，未保存画面".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.closed = true;
        Ok(())
    }
}
fn receive(
    output: &mpsc::Receiver<Output>,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Output, String> {
    let started = Instant::now();
    loop {
        if cancel.is_some_and(|t| t.load(Ordering::Acquire)) {
            return Err("应用快照已取消".into());
        }
        if started.elapsed() >= timeout {
            return Err("应用快照响应超时".into());
        }
        match output.recv_timeout(Duration::from_millis(30)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("应用快照连接已断开".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
impl Session for WorkerSession {
    fn original(&self) -> &ScrollPosition {
        &self.original
    }
    fn frame_at(&mut self, position: f64, cancel: &AtomicBool) -> Result<WindowFrame, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("应用快照已取消".into());
        }
        if !position.is_finite() || !(0.0..=100.0).contains(&position) {
            return Err("应用滚动位置无效".into());
        }
        self.request(&Request::Step { position })?;
        self.step_pending = true;
        let response = receive(&self.output, STEP_TIMEOUT, Some(cancel))?;
        self.step_pending = false;
        match response {
            Output::Value(Response::Frame { state, .. }, Some(image)) => {
                Ok(WindowFrame { state, image })
            }
            _ => Err("无法获取连续的应用画面".into()),
        }
    }
    fn restore(&mut self) -> Result<ScrollPosition, String> {
        self.restore_attempted = true;
        self.request(&Request::Restore)?;
        // A cancelled Step may still have one outstanding Frame/Failed reply.
        // Drain it without accepting pixels, then require the Restore ack.
        let started = Instant::now();
        let mut drained = !self.step_pending;
        loop {
            let remaining = STEP_TIMEOUT
                .checked_sub(started.elapsed())
                .ok_or("应用滚动恢复超时")?;
            match receive(&self.output, remaining, None)? {
                Output::Value(Response::Restored { state }, None)
                    if self.original.restored(&state) =>
                {
                    self.restored = true;
                    return Ok(state);
                }
                Output::Value(Response::Frame { .. }, _)
                | Output::Value(Response::Failed, None)
                    if !drained =>
                {
                    drained = true;
                }
                _ => return Err("未能确认应用滚动恢复".into()),
            }
        }
    }
}
impl Drop for WorkerSession {
    fn drop(&mut self) {
        if !self.restore_attempted {
            let _ = self.restore();
        }
        if self.restored && !self.closed {
            let _ = self.close();
        }
    }
}

pub fn worker_main() -> i32 {
    #[cfg(not(windows))]
    {
        2
    }
    #[cfg(windows)]
    {
        let mut input = std::io::stdin().lock();
        let Ok(Some(Request::Start { window, selection })) = read_header::<Request>(&mut input)
        else {
            return 2;
        };
        drop(input);
        let (tx, rx) = mpsc::sync_channel(1);
        if std::thread::Builder::new()
            .name("mewu-snapshot-input".into())
            .spawn(move || {
                let mut stdin = std::io::stdin().lock();
                while let Ok(Some(request)) = read_header::<Request>(&mut stdin) {
                    if tx.send(request).is_err() {
                        break;
                    }
                }
            })
            .is_err()
        {
            return 2;
        }
        let mut output = std::io::stdout().lock();
        let mut session =
            match crate::application_snapshot_windows::NativeSession::new(window, selection) {
                Ok(session) => session,
                Err(_) => {
                    let _ = write_header(&Response::Failed, &mut output);
                    return 2;
                }
            };
        if write_header(
            &Response::Ready {
                state: session.original().clone(),
            },
            &mut output,
        )
        .is_err()
        {
            return 2;
        }
        let started = Instant::now();
        let cancel = AtomicBool::new(false);
        let mut restored = false;
        let mut failed = false;
        while started.elapsed() < Duration::from_secs(300) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Request::Step { position }) if !restored && !failed => {
                    match session.frame_at(position, &cancel) {
                        Ok(frame) => {
                            let bytes = frame.image.into_raw();
                            if write_header(
                                &Response::Frame {
                                    state: frame.state,
                                    length: bytes.len(),
                                },
                                &mut output,
                            )
                            .and_then(|_| {
                                output
                                    .write_all(&bytes)
                                    .and_then(|_| output.flush())
                                    .map_err(|_| "closed".into())
                            })
                            .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => {
                            failed = true;
                            if write_header(&Response::Failed, &mut output).is_err() {
                                break;
                            }
                        }
                    }
                }
                Ok(Request::Restore) if !restored => match session.restore() {
                    Ok(state) => {
                        restored = true;
                        if write_header(&Response::Restored { state }, &mut output).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = write_header(&Response::Failed, &mut output);
                        break;
                    }
                },
                Ok(Request::Quit) if restored => return 0,
                Ok(_) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        if !restored {
            let _ = session.restore();
        }
        2
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_protocol_rejects_oversize_truncation_unknown_fields_and_invalid_pixels_before_allocation(
    ) {
        for bytes in [
            vec![0, 0, 0, 0],
            (HEADER_LIMIT as u32 + 1).to_le_bytes().to_vec(),
            vec![5, 0, 0, 0, b'{'],
        ] {
            assert!(read_header::<Request>(&mut bytes.as_slice()).is_err());
        }
        let mut encoded = Vec::new();
        write_header(
            &serde_json::json!({"type":"step","position":0,"hwnd":42}),
            &mut encoded,
        )
        .unwrap();
        assert!(read_header::<Request>(&mut encoded.as_slice()).is_err());
        let state = ScrollPosition {
            viewport: Viewport {
                x: -500,
                y: 20,
                width: 96,
                height: 96,
            },
            horizontal: -1.0,
            vertical: 0.0,
            view_size: 25.0,
        };
        assert!(frame_length(&state, 96 * 96 * 4).is_ok());
        assert!(frame_length(&state, PIXEL_LIMIT + 1).is_err());
        assert!(frame_length(&state, 96 * 96 * 4 - 1).is_err());
    }
    #[test]
    fn restore_receiver_uses_a_real_deadline_and_cancellation_does_not_become_eof() {
        let (_tx, rx) = mpsc::sync_channel(1);
        assert!(receive(&rx, Duration::from_millis(40), None).is_err());
        assert!(
            receive(&rx, Duration::from_secs(1), Some(&AtomicBool::new(true)))
                .unwrap_err()
                .contains("取消")
        );
    }
}
