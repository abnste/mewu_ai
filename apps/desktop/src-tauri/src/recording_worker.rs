// SPDX-License-Identifier: MPL-2.0
//! Recording protocol and ownership of the real encoder process and pipe threads.
use crate::{owned_process::ManagedProcess, recording_backend as backend};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

static BUSY: AtomicBool = AtomicBool::new(false);
const FRAME_BYTES: usize = 16 * 1024;
const TOTAL_OUTPUT: usize = 4 * 1024 * 1024;

pub fn busy() -> bool {
    BUSY.load(Ordering::Acquire)
}
struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "录屏进程尚未结束")?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// At most one pending pause state and one terminal command; repeated UI input
/// cannot grow a queue while a codec or device is blocked.
#[derive(Default)]
pub struct ControlBus(Mutex<ControlState>);
#[derive(Default)]
struct ControlState {
    paused: Option<bool>,
    terminal: Option<backend::Control>,
    terminal_delivered: bool,
}
impl ControlBus {
    pub fn send(&self, control: backend::Control) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "录屏控制不可用")?;
        if state.terminal.is_some() {
            return Ok(());
        }
        match control {
            backend::Control::Pause => state.paused = Some(true),
            backend::Control::Resume => state.paused = Some(false),
            backend::Control::Stop | backend::Control::Cancel => {
                state.paused = None;
                state.terminal = Some(control);
            }
        }
        Ok(())
    }
    pub fn take(&self) -> Result<Option<backend::Control>, String> {
        let mut state = self.0.lock().map_err(|_| "录屏控制不可用")?;
        if let Some(terminal) = state.terminal {
            if state.terminal_delivered {
                return Ok(None);
            }
            state.terminal_delivered = true;
            return Ok(Some(terminal));
        }
        Ok(state.paused.take().map(|v| {
            if v {
                backend::Control::Pause
            } else {
                backend::Control::Resume
            }
        }))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerEvent {
    Progress(backend::Progress),
    Completed(backend::Report),
    StoppedBeforeReady {},
    Failed { error: String },
}
fn write_line(writer: &mut impl Write, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "无效录屏通信")?;
    if bytes.len() >= FRAME_BYTES {
        return Err("无效录屏通信".into());
    }
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .and_then(|_| writer.flush())
        .map_err(|_| "录屏连接已断开".into())
}
fn read_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    let size = reader
        .take((FRAME_BYTES + 1) as u64)
        .read_line(&mut line)
        .map_err(|_| "录屏连接读取失败")?;
    if size == 0 {
        return Ok(None);
    }
    if size > FRAME_BYTES || !line.ends_with('\n') {
        return Err("无效录屏通信".into());
    }
    Ok(Some(line))
}

pub fn worker_main() -> i32 {
    let mut input = BufReader::new(std::io::stdin());
    let config = match read_line(&mut input).and_then(|line| {
        serde_json::from_str::<backend::Config>(&line.ok_or("缺少录屏配置")?)
            .map_err(|_| "无效录屏配置".into())
    }) {
        Ok(config) => config,
        Err(_) => return 2,
    };
    let (sender, receiver) = mpsc::sync_channel(8);
    let controls = match std::thread::Builder::new()
        .name("mewu-recording-controls".into())
        .spawn(move || {
            while let Ok(Some(line)) = read_line(&mut input) {
                match serde_json::from_str::<backend::Control>(&line) {
                    Ok(control) => {
                        if sender.send(control).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sender.send(backend::Control::Cancel);
        }) {
        Ok(thread) => thread,
        Err(_) => return 2,
    };
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let progress = output.clone();
    let result = backend::run(config, receiver, move |value| {
        if let Ok(mut out) = progress.lock() {
            let _ = write_line(&mut *out, &WorkerEvent::Progress(value));
        }
    });
    let event = match result {
        Ok(Some(report)) => WorkerEvent::Completed(report),
        Ok(None) => WorkerEvent::StoppedBeforeReady {},
        Err(error) => WorkerEvent::Failed { error },
    };
    let success = matches!(
        event,
        WorkerEvent::Completed(_) | WorkerEvent::StoppedBeforeReady {}
    );
    let sent = output
        .lock()
        .ok()
        .is_some_and(|mut out| write_line(&mut *out, &event).is_ok());
    // A terminal receipt asks the parent to close stdin. Join only after this
    // handshake, so receipt and actual device/COM/pipe teardown are distinct.
    let _ = controls.join();
    if sent && success {
        0
    } else {
        1
    }
}

enum Output {
    Event(WorkerEvent),
    Eof,
    Invalid,
}

pub fn run(
    config: backend::Config,
    poll_control: impl FnMut() -> Result<Option<backend::Control>, String>,
    on_progress: impl FnMut(backend::Progress),
    on_stopping: impl FnMut(),
    on_finalized: impl FnMut(),
) -> Result<Option<backend::Report>, String> {
    let mut command = Command::new(std::env::current_exe().map_err(|_| "无法启动录屏进程")?);
    command.arg("--recording-worker").env_clear();
    for name in [
        "SystemRoot",
        "WINDIR",
        "SystemDrive",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(parent) = config.output.parent() {
        command.current_dir(parent);
    }
    supervise(
        command,
        config,
        poll_control,
        on_progress,
        on_stopping,
        on_finalized,
    )
}

fn supervise(
    mut command: Command,
    config: backend::Config,
    mut poll_control: impl FnMut() -> Result<Option<backend::Control>, String>,
    mut on_progress: impl FnMut(backend::Progress),
    mut on_stopping: impl FnMut(),
    mut on_finalized: impl FnMut(),
) -> Result<Option<backend::Report>, String> {
    let permit = Permit::acquire()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut process =
        ManagedProcess::spawn(&mut command, permit).map_err(|_| "无法管理录屏进程")?;
    let mut input = process.child_mut().stdin.take().ok_or("无法控制录屏进程")?;
    let output = process
        .child_mut()
        .stdout
        .take()
        .ok_or("无法读取录屏状态")?;
    let (writer_tx, writer_rx) = mpsc::sync_channel::<backend::Control>(8);
    let (write_error_tx, write_error_rx) = mpsc::sync_channel(1);
    let wire_config = config.clone();
    process.track_io(
        std::thread::Builder::new()
            .name("mewu-recording-writer".into())
            .spawn(move || {
                let result = (|| {
                    write_line(&mut input, &wire_config)?;
                    while let Ok(control) = writer_rx.recv() {
                        write_line(&mut input, &control)?;
                    }
                    Ok::<_, String>(())
                })();
                if result.is_err() {
                    let _ = write_error_tx.try_send(());
                }
            })
            .map_err(|_| "无法建立录屏连接")?,
    );
    let (sender, events) = mpsc::sync_channel(8);
    process.track_io(
        std::thread::Builder::new()
            .name("mewu-recording-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(output);
                let mut total = 0usize;
                loop {
                    let output = match read_line(&mut reader) {
                        Ok(Some(line)) => {
                            total = total.saturating_add(line.len());
                            if total > TOTAL_OUTPUT {
                                Output::Invalid
                            } else {
                                serde_json::from_str(&line)
                                    .map(Output::Event)
                                    .unwrap_or(Output::Invalid)
                            }
                        }
                        Ok(None) => Output::Eof,
                        Err(_) => Output::Invalid,
                    };
                    let finished = matches!(output, Output::Eof | Output::Invalid);
                    if sender.send(output).is_err() || finished {
                        break;
                    }
                }
            })
            .map_err(|_| "无法读取录屏连接")?,
    );
    let mut writer_tx = Some(writer_tx);
    let started = Instant::now();
    let mut last_progress = started;
    let mut elapsed_ms = 0;
    let mut ready = false;
    let mut stopping: Option<Instant> = None;
    let mut canceled = false;
    let mut terminal: Option<Result<Option<backend::Report>, String>> = None;
    let mut eof = false;
    let result = (|| -> Result<Option<backend::Report>, String> {
        loop {
            if terminal.is_none() && stopping.is_none() {
                if let Some(control) = poll_control()? {
                    if matches!(control, backend::Control::Stop | backend::Control::Cancel) {
                        stopping = Some(Instant::now());
                        canceled = matches!(control, backend::Control::Cancel);
                        on_stopping();
                    }
                    writer_tx
                        .as_ref()
                        .ok_or("录屏连接已断开")?
                        .try_send(control)
                        .map_err(|_| "录屏控制未能送达")?;
                }
            }
            match events.recv_timeout(Duration::from_millis(20)) {
                Ok(Output::Event(event)) => {
                    if terminal.is_some() || eof {
                        return Err("无效录屏状态".into());
                    }
                    match event {
                        WorkerEvent::Progress(progress) => {
                            if progress.elapsed_ms < elapsed_ms || progress.elapsed_ms > 600_100 {
                                return Err("无效录屏进度".into());
                            }
                            ready = true;
                            elapsed_ms = progress.elapsed_ms;
                            last_progress = Instant::now();
                            if stopping.is_none() {
                                on_progress(progress);
                            }
                        }
                        WorkerEvent::Completed(report) => {
                            validate_report(&config, &report)?;
                            on_finalized();
                            terminal = Some(Ok(Some(report)));
                            writer_tx.take();
                            stopping = Some(Instant::now());
                        }
                        WorkerEvent::StoppedBeforeReady {} => {
                            if ready || stopping.is_none() {
                                return Err("无效录屏停止状态".into());
                            }
                            terminal = Some(Ok(None));
                            writer_tx.take();
                            stopping = Some(Instant::now());
                        }
                        WorkerEvent::Failed { error } => {
                            if error.is_empty() || error.len() > 4096 {
                                return Err("无效录屏结果".into());
                            }
                            terminal = Some(Err(error));
                            writer_tx.take();
                            stopping = Some(Instant::now());
                        }
                    }
                }
                Ok(Output::Eof) => {
                    eof = true;
                    if terminal.is_none() {
                        return Err("录屏进程已中断".into());
                    }
                }
                Ok(Output::Invalid) => return Err("无效录屏通信".into()),
                Err(mpsc::RecvTimeoutError::Disconnected) if !eof => {
                    return Err("录屏进程已中断".into())
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => {}
            }
            if eof && process.fully_exited() {
                let status = process
                    .try_wait()
                    .map_err(|_| "无法确认录屏退出")?
                    .ok_or("录屏尚未退出")?;
                let result = terminal.take().ok_or("录屏未完成")?;
                if status.success() != result.is_ok() {
                    return Err("录屏进程异常退出".into());
                }
                return if canceled { Ok(None) } else { result };
            }
            if terminal.is_none() && write_error_rx.try_recv().is_ok() {
                return Err("录屏控制未能送达".into());
            }
            if (!ready && started.elapsed() > Duration::from_secs(15))
                || stopping.is_some_and(|time| time.elapsed() > Duration::from_secs(20))
                || (ready
                    && stopping.is_none()
                    && last_progress.elapsed() > Duration::from_secs(20))
            {
                return Err("录屏响应超时，已停止".into());
            }
            let bytes = std::fs::metadata(&config.output)
                .map(|m| m.len())
                .unwrap_or(0);
            if terminal.is_none()
                && stopping.is_none()
                && (started.elapsed() > Duration::from_secs(1800) || bytes >= 448 * 1024 * 1024)
            {
                stopping = Some(Instant::now());
                on_stopping();
                writer_tx
                    .as_ref()
                    .ok_or("录屏连接已断开")?
                    .try_send(backend::Control::Stop)
                    .map_err(|_| "录屏控制未能送达")?;
            }
            if bytes > 512 * 1024 * 1024 {
                return Err("录屏文件超出安全上限".into());
            }
        }
    })();
    drop(writer_tx);
    drop(events);
    if result.is_err() {
        process.kill();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !process.fully_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // ManagedProcess's reaper retains Permit if a driver/pipe has not exited.
    drop(process);
    result
}

fn validate_report(config: &backend::Config, report: &backend::Report) -> Result<(), String> {
    if report.frames == 0
        || report.frames > 9_002
        || report.width != config.width
        || report.height != config.height
        || report.duration_ms == 0
        || report.duration_ms > 600_100
        || report.audio != config.audio
        || (report.audio == backend::AudioMode::Mute && report.stop_reason.is_some())
    {
        return Err("无效录屏结果".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_coalesce_and_terminal_cannot_be_overwritten() {
        let bus = ControlBus::default();
        for _ in 0..10000 {
            bus.send(backend::Control::Pause).unwrap();
            bus.send(backend::Control::Resume).unwrap();
        }
        assert!(matches!(
            bus.take().unwrap(),
            Some(backend::Control::Resume)
        ));
        assert!(bus.take().unwrap().is_none());
        bus.send(backend::Control::Pause).unwrap();
        bus.send(backend::Control::Stop).unwrap();
        bus.send(backend::Control::Cancel).unwrap();
        bus.send(backend::Control::Resume).unwrap();
        assert!(matches!(bus.take().unwrap(), Some(backend::Control::Stop)));
        assert!(bus.take().unwrap().is_none());
    }
    #[test]
    fn recording_wire_is_bounded_and_rejects_incomplete_lines() {
        assert!(read_line(&mut &b"{}"[..]).is_err());
        assert!(read_line(&mut vec![b'a'; FRAME_BYTES + 1].as_slice()).is_err());
        assert!(serde_json::from_str::<WorkerEvent>(
            r#"{"type":"failed","error":"x","extra":true}"#
        )
        .is_err());
        assert!(serde_json::from_str::<WorkerEvent>(
            r#"{"type":"stopped_before_ready","extra":true}"#
        )
        .is_err());
    }
    #[cfg(windows)]
    #[test]
    fn supervised_terminal_waits_for_real_child_and_io_and_rejects_invalid_report() {
        let config = backend::Config {
            output: std::env::temp_dir()
                .join(format!("mewu-audio-protocol-{}.mp4", uuid::Uuid::new_v4())),
            origin_x: 0,
            origin_y: 0,
            monitor_width: 64,
            monitor_height: 64,
            x: 0,
            y: 0,
            width: 64,
            height: 64,
            audio: backend::AudioMode::Mute,
        };
        // This fixture creates no media, desktop windows or capture devices.
        // PowerShell is a direct child with bounded stdin/stdout, attached to
        // the same real Windows Job and IO owner used by the production worker.
        let script = r#"
$ErrorActionPreference = 'Stop'
$recordingFixture = [Console]::In.ReadLine() | ConvertFrom-Json
$preReady = $env:MEWU_RECORDING_PROTOCOL_CASE -in @('stop-before-ready','unexpected-stop','stop-after-ready')
if ($env:MEWU_RECORDING_PROTOCOL_CASE -ne 'stop-before-ready' -and $env:MEWU_RECORDING_PROTOCOL_CASE -ne 'unexpected-stop') {
    [Console]::Out.WriteLine('{"type":"progress","elapsedMs":0,"paused":false}')
}
if ($preReady) {
    [Console]::Out.WriteLine('{"type":"stopped_before_ready"}')
} elseif ($env:MEWU_RECORDING_PROTOCOL_CASE -eq 'failure') {
    [Console]::Out.WriteLine('{"type":"failed","error":"合成声源失败"}')
} else {
    $recordingWidth = $recordingFixture.width
    if ($env:MEWU_RECORDING_PROTOCOL_CASE -eq 'invalid') { $recordingWidth += 2 }
    [Console]::Out.WriteLine((@{type='completed';width=$recordingWidth;height=$recordingFixture.height;durationMs=1000;frames=15;audio='mute'} | ConvertTo-Json -Compress))
}
while ($null -ne [Console]::In.ReadLine()) {}
Start-Sleep -Milliseconds 180
if ($env:MEWU_RECORDING_PROTOCOL_CASE -eq 'failure') { exit 1 }
exit 0
"#;
        for mode in [
            "success",
            "cancel",
            "invalid",
            "failure",
            "stop-before-ready",
            "unexpected-stop",
            "stop-after-ready",
        ] {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
            command.env("MEWU_RECORDING_PROTOCOL_CASE", mode);
            let began = Instant::now();
            let mut progress = 0;
            let mut cancel_sent = false;
            let result = supervise(
                command,
                config.clone(),
                || {
                    if matches!(mode, "cancel" | "stop-before-ready" | "stop-after-ready")
                        && !cancel_sent
                    {
                        cancel_sent = true;
                        Ok(Some(if mode == "cancel" {
                            backend::Control::Cancel
                        } else {
                            backend::Control::Stop
                        }))
                    } else {
                        Ok(None)
                    }
                },
                |_| {
                    progress += 1;
                },
                || {},
                || {},
            );
            match mode {
                "success" => {
                    assert!(result.unwrap().is_some());
                    assert_eq!(progress, 1);
                    assert!(began.elapsed() >= Duration::from_millis(180));
                }
                "cancel" | "stop-before-ready" => assert!(result.unwrap().is_none()),
                _ => assert!(result.is_err()),
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while busy() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!busy(), "process and IO ownership for {mode}");
            assert!(!config.output.exists());
        }
    }
}
