// SPDX-License-Identifier: MPL-2.0
//! One supervised local worker for both capability enumeration and dictation.
//! The real child lifetime, not an invoke future, owns the sole process permit.
use crate::owned_process::ManagedProcess;
use crate::speech_backend::{self, SpeechError, VoicePhase, WorkerConfig, WorkerOutput};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    process::{ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 64 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(10);
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_GRACE: Duration = Duration::from_secs(2);
const KILL_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerEvent {
    Ready { version: u32 },
    Progress { phase: VoicePhase },
    Completed { output: WorkerOutput },
    Failed { error: SpeechError },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Cancel {},
}

type Slot = Arc<Mutex<Option<Arc<AtomicBool>>>>;
fn slot() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None)))
}
pub fn busy() -> bool {
    slot().lock().map_or(true, |v| v.is_some())
}
pub fn cancel_all() {
    if let Ok(current) = slot().lock() {
        if let Some(token) = current.as_ref() {
            token.store(true, Ordering::Release);
        }
    }
}
pub async fn wait_until_idle(timeout: Duration) -> bool {
    let started = Instant::now();
    while busy() {
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

struct Permit {
    slot: Slot,
    token: Arc<AtomicBool>,
}
impl Permit {
    fn acquire(slot: Slot, token: Arc<AtomicBool>) -> Result<Self, SpeechError> {
        speech_backend::check_cancel(&token)?;
        {
            let mut guard = slot.lock().map_err(|_| SpeechError::Busy)?;
            if guard.is_some() {
                return Err(SpeechError::Busy);
            }
            *guard = Some(token.clone());
        }
        Ok(Self { slot, token })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.lock() {
            if guard.as_ref().is_some_and(|v| Arc::ptr_eq(v, &self.token)) {
                *guard = None;
            }
        }
    }
}
struct CancelOnDrop(Option<Arc<AtomicBool>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.store(true, Ordering::Release);
        }
    }
}

pub async fn run(
    config: WorkerConfig,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(VoicePhase) + Send + 'static,
) -> Result<WorkerOutput, SpeechError> {
    speech_backend::check_cancel(&cancel)?;
    #[cfg(not(windows))]
    {
        let _ = (config, on_progress);
        return Err(SpeechError::Unsupported);
    }
    #[cfg(windows)]
    {
        let permit = Permit::acquire(slot().clone(), cancel.clone())?;
        let mut cancel_guard = CancelOnDrop(Some(cancel.clone()));
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("mewu-speech-supervisor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    supervise(config, cancel, on_progress, permit)
                }))
                .unwrap_or(Err(SpeechError::WorkerUnavailable));
                let _ = sender.send(result);
            })
            .map_err(|_| SpeechError::WorkerUnavailable)?;
        let result = receiver.await.map_err(|_| SpeechError::WorkerUnavailable)?;
        cancel_guard.0 = None;
        result
    }
}

/// A strict newline frame. EOF in a partial frame is an error, never a result.
fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, SpeechError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| SpeechError::Protocol)?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(SpeechError::Protocol)
            };
        }
        let line_end = available.iter().position(|b| *b == b'\n');
        let consumed = line_end.map_or(available.len(), |v| v + 1);
        if bytes.len().saturating_add(consumed) > MAX_FRAME {
            return Err(SpeechError::Protocol);
        }
        bytes.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if line_end.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return if bytes.is_empty() {
                Err(SpeechError::Protocol)
            } else {
                Ok(Some(bytes))
            };
        }
    }
}
fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<(), SpeechError> {
    let mut frame = serde_json::to_vec(value).map_err(|_| SpeechError::Protocol)?;
    if frame.len() >= MAX_FRAME {
        return Err(SpeechError::Protocol);
    }
    frame.push(b'\n');
    writer
        .write_all(&frame)
        .and_then(|_| writer.flush())
        .map_err(|_| SpeechError::Protocol)
}

struct Protocol {
    config: WorkerConfig,
    ready: bool,
    listening: bool,
    stopping: bool,
    count: usize,
    terminal: Option<Result<WorkerOutput, SpeechError>>,
}
impl Protocol {
    fn new(config: WorkerConfig) -> Self {
        Self {
            config,
            ready: false,
            listening: false,
            stopping: false,
            count: 0,
            terminal: None,
        }
    }
    fn accept(&mut self, event: WorkerEvent) -> Result<Option<VoicePhase>, SpeechError> {
        self.count += 1;
        if self.count > 256 || self.terminal.is_some() {
            return Err(SpeechError::Protocol);
        }
        if let WorkerEvent::Ready { version } = event {
            if self.ready || version != VERSION {
                return Err(SpeechError::Protocol);
            }
            self.ready = true;
            return Ok(None);
        }
        if !self.ready {
            return Err(SpeechError::Protocol);
        }
        match event {
            WorkerEvent::Progress { phase } => {
                if matches!(self.config, WorkerConfig::Capabilities) {
                    return Err(SpeechError::Protocol);
                }
                match phase {
                    VoicePhase::Listening if !self.listening && !self.stopping => {
                        self.listening = true
                    }
                    VoicePhase::Stopping if !self.stopping => self.stopping = true,
                    _ => return Err(SpeechError::Protocol),
                }
                Ok(Some(phase))
            }
            WorkerEvent::Completed { output } => {
                if matches!(self.config, WorkerConfig::Dictation { .. }) && !self.listening {
                    return Err(SpeechError::Protocol);
                }
                speech_backend::validate_output(self.config, &output)?;
                self.terminal = Some(Ok(output));
                Ok(None)
            }
            WorkerEvent::Failed { error } => {
                self.terminal = Some(Err(error));
                Ok(None)
            }
            WorkerEvent::Ready { .. } => unreachable!(),
        }
    }
}
enum ReadEvent {
    Frame(WorkerEvent),
    Eof,
    Error,
}
fn start_reader(
    stdout: std::process::ChildStdout,
) -> Result<(mpsc::Receiver<ReadEvent>, std::thread::JoinHandle<()>), SpeechError> {
    let (tx, rx) = mpsc::sync_channel(8);
    let thread = std::thread::Builder::new()
        .name("mewu-speech-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let event = match read_frame(&mut reader) {
                    Ok(Some(bytes)) => {
                        total = total.saturating_add(bytes.len() + 1);
                        if total > MAX_OUTPUT {
                            ReadEvent::Error
                        } else {
                            match serde_json::from_slice(&bytes) {
                                Ok(event) => ReadEvent::Frame(event),
                                Err(_) => ReadEvent::Error,
                            }
                        }
                    }
                    Ok(None) => ReadEvent::Eof,
                    Err(_) => ReadEvent::Error,
                };
                let done = matches!(event, ReadEvent::Eof | ReadEvent::Error);
                if tx.send(event).is_err() || done {
                    break;
                }
            }
        })
        .map_err(|_| SpeechError::WorkerUnavailable)?;
    Ok((rx, thread))
}

type OwnedProcess = ManagedProcess<Permit>;

fn request_stop(input: &mut Option<ChildStdin>) {
    if let Some(mut pipe) = input.take() {
        let _ = write_json(&mut pipe, &Control::Cancel {});
    }
}
fn report(callback: &impl Fn(VoicePhase), phase: VoicePhase) -> Result<(), SpeechError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(phase)))
        .map_err(|_| SpeechError::WorkerUnavailable)
}
fn supervise(
    config: WorkerConfig,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(VoicePhase),
    permit: Permit,
) -> Result<WorkerOutput, SpeechError> {
    let started = Instant::now();
    speech_backend::check_cancel(&cancel)?;
    // No slot mutex is held during callbacks or process/pipe I/O.
    if matches!(config, WorkerConfig::Dictation { .. }) {
        report(&on_progress, VoicePhase::Starting)?;
    }
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| SpeechError::WorkerUnavailable)?);
    command
        .arg("--speech-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.env_clear();
    for name in [
        "SystemRoot",
        "WINDIR",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "TEMP",
        "TMP",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut owned =
        OwnedProcess::spawn(&mut command, permit).map_err(|_| SpeechError::WorkerUnavailable)?;
    let process = &mut owned;
    let mut input = Some(
        process
            .child_mut()
            .stdin
            .take()
            .ok_or(SpeechError::WorkerUnavailable)?,
    );
    let stdout = process
        .child_mut()
        .stdout
        .take()
        .ok_or(SpeechError::WorkerUnavailable)?;
    let (output, reader) = start_reader(stdout)?;
    process.track_io(reader);
    speech_backend::check_cancel(&cancel)?;
    // The child waits for this bounded config before COM; its job is attached now.
    write_json(input.as_mut().unwrap(), &config)?;
    let mut protocol = Protocol::new(config);
    let mut eof = false;
    let mut exit: Option<ExitStatus> = None;
    let mut exit_at: Option<Instant> = None;
    let mut stop_at: Option<Instant> = None;
    let mut killed_at: Option<Instant> = None;
    let mut failure: Option<SpeechError> = None;
    let mut stopping_notified = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            failure = Some(SpeechError::Cancelled);
        }
        if failure.is_none() && protocol.terminal.is_none() {
            if (!protocol.listening && started.elapsed() >= START_TIMEOUT)
                || started.elapsed() >= SESSION_TIMEOUT
            {
                failure = Some(SpeechError::TimedOut);
            }
        }
        if (failure.is_some() || protocol.terminal.is_some()) && stop_at.is_none() {
            stop_at = Some(Instant::now());
            if !stopping_notified && matches!(config, WorkerConfig::Dictation { .. }) {
                if let Err(error) = report(&on_progress, VoicePhase::Stopping) {
                    failure.get_or_insert(error);
                }
                stopping_notified = true;
            }
            request_stop(&mut input);
        }
        if exit.is_none() {
            match process.try_wait() {
                Ok(Some(status)) => {
                    exit = Some(status);
                    exit_at = Some(Instant::now());
                }
                Ok(None) => {}
                Err(_) => {
                    failure.get_or_insert(SpeechError::WorkerUnavailable);
                }
            }
        }
        let tree_exited = exit.is_some() && process.tree_empty();
        if exit.is_some() && !tree_exited {
            process.kill();
            stop_at.get_or_insert_with(Instant::now);
            killed_at.get_or_insert_with(Instant::now);
        }
        if let Some(status) = exit.filter(|_| tree_exited) {
            if let Some(error) = failure {
                return Err(error);
            }
            if eof {
                if !status.success() {
                    return Err(SpeechError::WorkerUnavailable);
                }
                return protocol.terminal.take().ok_or(SpeechError::Protocol)?;
            }
            if exit_at.is_some_and(|at| at.elapsed() >= KILL_GRACE) {
                return Err(SpeechError::Protocol);
            }
        } else if stop_at.is_some_and(|at| at.elapsed() >= STOP_GRACE) {
            if killed_at.is_none() {
                process.kill();
                killed_at = Some(Instant::now());
            }
            if killed_at.is_some_and(|at| at.elapsed() >= KILL_GRACE) {
                return Err(SpeechError::CleanupTimedOut);
            }
        }
        match output.recv_timeout(Duration::from_millis(25)) {
            Ok(ReadEvent::Frame(event)) => match protocol.accept(event) {
                Ok(Some(phase)) if failure.is_none() => {
                    if phase != VoicePhase::Stopping || !stopping_notified {
                        if let Err(error) = report(&on_progress, phase) {
                            failure = Some(error);
                        }
                    }
                    if phase == VoicePhase::Stopping {
                        stopping_notified = true;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            },
            Ok(ReadEvent::Eof) => {
                eof = true;
                if protocol.terminal.is_none() {
                    failure.get_or_insert(SpeechError::Protocol);
                }
            }
            Ok(ReadEvent::Error) => {
                failure.get_or_insert(SpeechError::Protocol);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !eof {
                    failure.get_or_insert(SpeechError::Protocol);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn emit(event: &WorkerEvent) -> Result<(), SpeechError> {
    write_json(&mut std::io::stdout().lock(), event)
}

/// Called by main's first --speech-worker branch, before opening app data or UI.
pub fn worker_main() -> i32 {
    let mut reader = BufReader::new(std::io::stdin());
    let config = match read_frame(&mut reader).and_then(|v| {
        serde_json::from_slice::<WorkerConfig>(&v.ok_or(SpeechError::Protocol)?)
            .map_err(|_| SpeechError::Protocol)
    }) {
        Ok(config) => config,
        Err(_) => return 2,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let control_cancel = cancel.clone();
    if std::thread::Builder::new()
        .name("mewu-speech-control".into())
        .spawn(move || {
            // Preserve the first reader's buffer: a cancel may already follow config.
            let _ = read_frame(&mut reader).and_then(|line| {
                let line = line.ok_or(SpeechError::Protocol)?;
                serde_json::from_slice::<Control>(&line).map_err(|_| SpeechError::Protocol)
            });
            control_cancel.store(true, Ordering::Release);
        })
        .is_err()
    {
        return 2;
    }
    if emit(&WorkerEvent::Ready { version: VERSION }).is_err() {
        return 2;
    }
    let mut stopping = false;
    let result = speech_backend::execute(config, &cancel, &mut |phase| {
        if phase == VoicePhase::Stopping {
            stopping = true;
        }
        if emit(&WorkerEvent::Progress { phase }).is_err() {
            cancel.store(true, Ordering::Release);
        }
    });
    if matches!(config, WorkerConfig::Dictation { .. }) && !stopping {
        if emit(&WorkerEvent::Progress {
            phase: VoicePhase::Stopping,
        })
        .is_err()
        {
            return 2;
        }
    }
    let result = if cancel.load(Ordering::Acquire) {
        Err(SpeechError::Cancelled)
    } else {
        result
    };
    let final_event = match result {
        Ok(output) => WorkerEvent::Completed { output },
        Err(error) => WorkerEvent::Failed { error },
    };
    if emit(&final_event).is_ok() {
        0
    } else {
        2
    }
}

#[cfg(all(test, windows))]
pub(crate) fn run_isolated_platform_test(name: &str) -> Result<(), SpeechError> {
    use std::os::windows::process::CommandExt;
    let permit = Permit::acquire(slot().clone(), Arc::new(AtomicBool::new(false)))?;
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| SpeechError::WorkerUnavailable)?);
    command
        .args(["--exact", name, "--ignored", "--nocapture"])
        .env("MEWU_SPEECH_SAPI_TEST_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .creation_flags(0x08000000);
    let mut owned =
        OwnedProcess::spawn(&mut command, permit).map_err(|_| SpeechError::WorkerUnavailable)?;
    let process = &mut owned;
    process
        .child_mut()
        .stdin
        .take()
        .ok_or(SpeechError::WorkerUnavailable)?
        .write_all(b"go\n")
        .map_err(|_| SpeechError::WorkerUnavailable)?;
    let started = Instant::now();
    loop {
        if let Some(status) = process
            .try_wait()
            .map_err(|_| SpeechError::WorkerUnavailable)?
        {
            if process.tree_empty() {
                return if status.success() {
                    Ok(())
                } else {
                    Err(SpeechError::WorkerUnavailable)
                };
            }
            process.kill();
        }
        if started.elapsed() >= Duration::from_secs(20) {
            return Err(SpeechError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech_backend::{DictationResult, VoiceConfidence, VoiceLanguage};
    fn result() -> WorkerEvent {
        WorkerEvent::Completed {
            output: WorkerOutput::Dictation(DictationResult {
                text: "合成结果".into(),
                language: "zh-CN".into(),
                confidence: VoiceConfidence::Recognized,
            }),
        }
    }
    fn config() -> WorkerConfig {
        WorkerConfig::Dictation {
            language: VoiceLanguage::ZhCn,
        }
    }
    #[test]
    fn protocol_needs_ready_and_listening_before_result() {
        let mut p = Protocol::new(config());
        assert_eq!(p.accept(result()), Err(SpeechError::Protocol));
        let mut p = Protocol::new(config());
        p.accept(WorkerEvent::Ready { version: VERSION }).unwrap();
        assert_eq!(p.accept(result()), Err(SpeechError::Protocol));
        p.accept(WorkerEvent::Progress {
            phase: VoicePhase::Listening,
        })
        .unwrap();
        p.accept(result()).unwrap();
        assert_eq!(p.accept(result()), Err(SpeechError::Protocol));
    }
    #[test]
    fn protocol_rejects_backwards_phase_and_wrong_version() {
        let mut p = Protocol::new(config());
        assert_eq!(
            p.accept(WorkerEvent::Ready { version: 2 }),
            Err(SpeechError::Protocol)
        );
        let mut p = Protocol::new(config());
        p.accept(WorkerEvent::Ready { version: 1 }).unwrap();
        p.accept(WorkerEvent::Progress {
            phase: VoicePhase::Stopping,
        })
        .unwrap();
        assert_eq!(
            p.accept(WorkerEvent::Progress {
                phase: VoicePhase::Listening
            }),
            Err(SpeechError::Protocol)
        );
    }
    #[test]
    fn frame_reader_is_bounded_and_preserves_buffered_cancel() {
        let mut reader =
            BufReader::new(&b"{\"mode\":\"capabilities\"}\n{\"type\":\"cancel\"}\n"[..]);
        assert!(
            serde_json::from_slice::<WorkerConfig>(&read_frame(&mut reader).unwrap().unwrap())
                .is_ok()
        );
        assert!(
            serde_json::from_slice::<Control>(&read_frame(&mut reader).unwrap().unwrap()).is_ok()
        );
        assert!(read_frame(&mut reader).unwrap().is_none());
        assert_eq!(
            read_frame(&mut BufReader::new(&b"partial"[..])),
            Err(SpeechError::Protocol)
        );
        assert_eq!(
            read_frame(&mut BufReader::new(vec![b'x'; MAX_FRAME + 1].as_slice())),
            Err(SpeechError::Protocol)
        );
        assert!(serde_json::from_str::<Control>(r#"{"type":"cancel","text":"ignored"}"#).is_err());
    }
    #[test]
    fn cancel_does_not_release_actual_process_permit() {
        let slot: Slot = Arc::new(Mutex::new(None));
        let token = Arc::new(AtomicBool::new(false));
        let permit = Permit::acquire(slot.clone(), token.clone()).unwrap();
        drop(CancelOnDrop(Some(token.clone())));
        assert!(token.load(Ordering::Acquire));
        assert!(matches!(
            Permit::acquire(slot.clone(), Arc::new(AtomicBool::new(false))),
            Err(SpeechError::Busy)
        ));
        drop(permit);
        assert!(Permit::acquire(slot, Arc::new(AtomicBool::new(false))).is_ok());
    }
    #[tokio::test]
    async fn pre_cancelled_run_cannot_spawn_worker() {
        assert_eq!(
            run(
                WorkerConfig::Capabilities,
                Arc::new(AtomicBool::new(true)),
                |_| panic!("no progress")
            )
            .await,
            Err(SpeechError::Cancelled)
        );
    }

    #[cfg(windows)]
    fn synthetic_command(depth: &str) -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "speech_worker::tests::synthetic_process_tree",
                "--nocapture",
            ])
            .env("MEWU_SPEECH_TEST_DEPTH", depth)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        command
    }

    // The normal test invocation is a no-op. A dedicated child invocation can
    // only wait or spawn another copy of this test; it never enters the backend.
    #[cfg(windows)]
    #[test]
    fn synthetic_process_tree() {
        let Ok(depth) = std::env::var("MEWU_SPEECH_TEST_DEPTH") else {
            return;
        };
        if depth != "1" && depth != "2" {
            return;
        }
        let mut gate = String::new();
        if std::io::stdin().read_line(&mut gate).is_err() || gate != "go\n" {
            return;
        }
        let mut descendant = if depth == "2" {
            let mut child = synthetic_command("1").spawn().unwrap();
            child.stdin.take().unwrap().write_all(b"go\n").unwrap();
            Some(child)
        } else {
            None
        };
        std::thread::sleep(Duration::from_secs(8));
        if let Some(child) = descendant.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[cfg(windows)]
    #[test]
    fn owned_job_reaps_a_real_synthetic_tree_before_releasing_slot() {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        struct ObservedProcess(HANDLE);
        impl Drop for ObservedProcess {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let local_slot: Slot = Arc::new(Mutex::new(None));
        let permit = Permit::acquire(local_slot.clone(), Arc::new(AtomicBool::new(false))).unwrap();
        let mut owned = OwnedProcess::spawn(&mut synthetic_command("2"), permit).unwrap();
        let observer = owned.observe_job_for_test().unwrap();
        let process_handle =
            ObservedProcess(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, owned.child_mut().id()) });
        assert!(!process_handle.0.is_null());
        owned
            .child_mut()
            .stdin
            .take()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
        let started = Instant::now();
        while observer.active_processes().is_some_and(|count| count < 2)
            && started.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            observer.active_processes(),
            Some(2),
            "synthetic descendant entered the same job"
        );
        drop(owned);
        let started = Instant::now();
        while local_slot.lock().unwrap().is_some() && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            local_slot.lock().unwrap().is_none(),
            "reaper released permit after actual tree exit"
        );
        assert_eq!(observer.active_processes(), Some(0));
        assert_eq!(
            unsafe { WaitForSingleObject(process_handle.0, 0) },
            WAIT_OBJECT_0
        );
    }
}
