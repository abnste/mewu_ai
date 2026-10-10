// SPDX-License-Identifier: MPL-2.0
//! One real child/Job/IO slot for lazy inspection and precise local video export.
use crate::{
    owned_process::ManagedProcess,
    video_clip_backend,
    video_clip_contract::{
        self, ClipError, ClipJob, ClipJobResult, Control, WorkerConfig, WorkerEvent, VERSION,
    },
};
use serde::Serialize;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const CONFIG_BYTES: usize = 64 * 1024;
const FRAME_BYTES: usize = 4 * 1024;
const OUTPUT_BYTES: usize = 128 * 1024;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(15);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);
const RENDER_TIMEOUT: Duration = Duration::from_secs(600);
const STOP_GRACE: Duration = Duration::from_secs(2);
const CLEANUP_GRACE: Duration = Duration::from_secs(2);
type Slot = Arc<Mutex<Option<Arc<AtomicBool>>>>;
fn slot() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None)))
}
pub fn busy() -> bool {
    slot().lock().map_or(true, |v| v.is_some())
}
pub fn cancel_all() {
    if let Ok(slot) = slot().lock() {
        if let Some(cancel) = slot.as_ref() {
            cancel.store(true, Ordering::Release);
        }
    }
}
pub async fn wait_until_idle(timeout: Duration) -> bool {
    let start = Instant::now();
    while busy() {
        if start.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}
struct Permit {
    slot: Slot,
    cancel: Arc<AtomicBool>,
}
impl Permit {
    fn acquire(cancel: Arc<AtomicBool>) -> Result<Self, ClipError> {
        video_clip_backend::check_cancel(&cancel)?;
        let slot = slot().clone();
        {
            let mut state = slot.lock().map_err(|_| ClipError::Busy)?;
            if state.is_some() {
                return Err(ClipError::Busy);
            }
            *state = Some(cancel.clone());
        }
        Ok(Self { slot, cancel })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.slot.lock() {
            if state.as_ref().is_some_and(|v| Arc::ptr_eq(v, &self.cancel)) {
                *state = None;
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
    job: ClipJob,
    cancel: Arc<AtomicBool>,
    on_progress: Arc<dyn Fn(u8) + Send + Sync>,
) -> Result<ClipJobResult, ClipError> {
    video_clip_backend::check_cancel(&cancel)?;
    job.validate()?;
    #[cfg(not(windows))]
    {
        let _ = on_progress;
        Err(ClipError::Unsupported)
    }
    #[cfg(windows)]
    {
        let permit = Permit::acquire(cancel.clone())?;
        let mut guard = CancelOnDrop(Some(cancel.clone()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("mewu-video-clip-supervisor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    supervise(job, cancel, on_progress, permit)
                }))
                .unwrap_or(Err(ClipError::WorkerUnavailable));
                let _ = tx.send(result);
            })
            .map_err(|_| ClipError::WorkerUnavailable)?;
        let result = rx.await.map_err(|_| ClipError::WorkerUnavailable)?;
        guard.0 = None;
        result
    }
}
fn worker_executable() -> Result<std::path::PathBuf, ClipError> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("MEWU_VIDEO_CLIP_TEST_EXE") {
        use sha2::{Digest, Sha256};
        let path = std::path::PathBuf::from(path);
        let expected = std::env::var("MEWU_VIDEO_CLIP_TEST_EXE_SHA256")
            .map_err(|_| ClipError::WorkerUnavailable)?;
        let meta = std::fs::symlink_metadata(&path).map_err(|_| ClipError::WorkerUnavailable)?;
        if !path.is_absolute()
            || !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() > 128 * 1024 * 1024
            || expected.len() != 64
            || !expected.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(ClipError::WorkerUnavailable);
        }
        let bytes = std::fs::read(&path).map_err(|_| ClipError::WorkerUnavailable)?;
        if !format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(&expected) {
            return Err(ClipError::WorkerUnavailable);
        }
        return Ok(path);
    }
    std::env::current_exe().map_err(|_| ClipError::WorkerUnavailable)
}
fn read_frame(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, ClipError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| ClipError::Protocol)?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(ClipError::Protocol)
            };
        }
        let ending = available.iter().position(|v| *v == b'\n');
        let used = ending.map_or(available.len(), |v| v + 1);
        if bytes.len().saturating_add(used) > limit {
            return Err(ClipError::Protocol);
        }
        bytes.extend_from_slice(&available[..used]);
        reader.consume(used);
        if ending.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return if bytes.is_empty() {
                Err(ClipError::Protocol)
            } else {
                Ok(Some(bytes))
            };
        }
    }
}
fn write_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    limit: usize,
) -> Result<(), ClipError> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| ClipError::Protocol)?;
    if bytes.len() >= limit {
        return Err(ClipError::Protocol);
    }
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .and_then(|_| writer.flush())
        .map_err(|_| ClipError::Protocol)
}

struct Protocol {
    job: ClipJob,
    ready: bool,
    progress: Option<u8>,
    terminal: Option<Result<ClipJobResult, ClipError>>,
    count: usize,
}
impl Protocol {
    fn new(job: ClipJob) -> Self {
        Self {
            job,
            ready: false,
            progress: None,
            terminal: None,
            count: 0,
        }
    }
    fn accept(&mut self, event: WorkerEvent) -> Result<Option<u8>, ClipError> {
        self.count += 1;
        if self.count > 104 || self.terminal.is_some() {
            return Err(ClipError::Protocol);
        }
        if let WorkerEvent::Ready { version } = event {
            if self.ready || version != VERSION {
                return Err(ClipError::Protocol);
            }
            self.ready = true;
            return Ok(None);
        }
        if !self.ready {
            return Err(ClipError::Protocol);
        }
        match event {
            WorkerEvent::Progress { percent } => {
                if !matches!(
                    self.job,
                    ClipJob::Render { .. }
                        | ClipJob::RenderGif { .. }
                        | ClipJob::RenderAnnotated { .. }
                        | ClipJob::RenderGifAnnotated { .. }
                ) || percent > 100
                    || self.progress.is_some_and(|prior| percent <= prior)
                {
                    return Err(ClipError::Protocol);
                }
                self.progress = Some(percent);
                return Ok(Some(percent));
            }
            WorkerEvent::Inspected { metadata } if matches!(self.job, ClipJob::Inspect { .. }) => {
                metadata.validate()?;
                self.terminal = Some(Ok(ClipJobResult::Inspected(metadata)));
            }
            WorkerEvent::Rendered { metadata } => {
                if let ClipJob::RenderAnnotated {
                    expected, range, ..
                } = &self.job
                {
                    let full = range.clone().unwrap_or(video_clip_contract::VideoRange {
                        start_ticks: 0,
                        end_ticks: expected.duration_ticks,
                    });
                    video_clip_contract::validate_annotated_rendered(expected, &full, &metadata)?;
                    self.terminal = Some(Ok(ClipJobResult::Rendered(metadata)));
                    return Ok(None);
                }
                let ClipJob::Render {
                    expected, range, ..
                } = &self.job
                else {
                    return Err(ClipError::Protocol);
                };
                video_clip_contract::validate_rendered(expected, range, &metadata)?;
                self.terminal = Some(Ok(ClipJobResult::Rendered(metadata)));
            }
            WorkerEvent::GifRendered { metadata } => {
                if let ClipJob::RenderGifAnnotated {
                    expected,
                    range,
                    overlays,
                    ..
                } = &self.job
                {
                    let schedule = video_clip_contract::AnnotatedGifSchedule::new(
                        expected,
                        range.as_ref(),
                        overlays,
                    )?;
                    if metadata != schedule.metadata {
                        return Err(ClipError::Protocol);
                    }
                    self.terminal = Some(Ok(ClipJobResult::GifRendered(metadata)));
                    return Ok(None);
                }
                let ClipJob::RenderGif {
                    expected, range, ..
                } = &self.job
                else {
                    return Err(ClipError::Protocol);
                };
                metadata.validate_for(expected, range.as_ref())?;
                self.terminal = Some(Ok(ClipJobResult::GifRendered(metadata)));
            }
            WorkerEvent::FramesExtracted { frames } => {
                let ClipJob::ExtractFrames {
                    expected,
                    window,
                    requested_ticks,
                    ..
                } = &self.job
                else {
                    return Err(ClipError::Protocol);
                };
                video_clip_contract::validate_frame_descriptors(
                    expected,
                    window,
                    requested_ticks,
                    &frames,
                )?;
                self.terminal = Some(Ok(ClipJobResult::FramesExtracted { frames }));
            }
            WorkerEvent::Cancelled {} => self.terminal = Some(Err(ClipError::Cancelled)),
            WorkerEvent::Failed { error } => self.terminal = Some(Err(error)),
            _ => return Err(ClipError::Protocol),
        }
        Ok(None)
    }
}
enum Output {
    Frame(WorkerEvent),
    Eof,
    Error,
}
/// The writer has independent lifetime too: a max-size config can fill a pipe
/// if the child stalls before consuming it. Supervisor never blocks on write.
fn attach_io(
    process: &mut ManagedProcess<Permit>,
    config: WorkerConfig,
) -> Result<(mpsc::Receiver<Output>, mpsc::SyncSender<Control>), ClipError> {
    let stdout = process
        .child_mut()
        .stdout
        .take()
        .ok_or(ClipError::WorkerUnavailable)?;
    let (tx, rx) = mpsc::sync_channel(8);
    let reader = std::thread::Builder::new()
        .name("mewu-video-clip-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let event = match read_frame(&mut reader, FRAME_BYTES) {
                    Ok(Some(bytes)) => {
                        total = total.saturating_add(bytes.len() + 1);
                        if total > OUTPUT_BYTES {
                            Output::Error
                        } else {
                            serde_json::from_slice(&bytes)
                                .map(Output::Frame)
                                .unwrap_or(Output::Error)
                        }
                    }
                    Ok(None) => Output::Eof,
                    Err(_) => Output::Error,
                };
                let end = !matches!(event, Output::Frame(_));
                if tx.send(event).is_err() || end {
                    break;
                }
            }
        })
        .map_err(|_| ClipError::WorkerUnavailable)?;
    process.track_io(reader);
    let mut stdin = process
        .child_mut()
        .stdin
        .take()
        .ok_or(ClipError::WorkerUnavailable)?;
    let (tx, control) = mpsc::sync_channel(1);
    let writer = std::thread::Builder::new()
        .name("mewu-video-clip-input".into())
        .spawn(move || {
            if write_frame(&mut stdin, &config, CONFIG_BYTES).is_ok() {
                // Only one cancel is meaningful. Retain the pipe until the owner
                // closes the channel (terminal or cleanup), so EOF is not premature.
                if let Ok(command) = control.recv() {
                    let _ = write_frame(&mut stdin, &command, 256);
                }
                let _ = control.recv();
            }
        })
        .map_err(|_| ClipError::WorkerUnavailable)?;
    process.track_io(writer);
    Ok((rx, tx))
}
fn supervise(
    job: ClipJob,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(u8) + Send + Sync>,
    permit: Permit,
) -> Result<ClipJobResult, ClipError> {
    let started = Instant::now();
    video_clip_backend::check_cancel(&cancel)?;
    let timeout = match &job {
        ClipJob::Inspect { .. } => INSPECT_TIMEOUT,
        ClipJob::ExtractFrames { .. } => EXTRACT_TIMEOUT,
        _ => RENDER_TIMEOUT,
    };
    let directory = match &job {
        ClipJob::Inspect { source } => source.parent(),
        ClipJob::Render { output, .. }
        | ClipJob::RenderGif { output, .. }
        | ClipJob::RenderAnnotated { output, .. }
        | ClipJob::RenderGifAnnotated { output, .. } => output.parent(),
        ClipJob::ExtractFrames {
            output_directory, ..
        } => Some(output_directory.as_path()),
    }
    .ok_or(ClipError::UnsupportedPath)?;
    let mut command = Command::new(worker_executable()?);
    command
        .arg("--video-clip-worker")
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut process =
        ManagedProcess::spawn(&mut command, permit).map_err(|_| ClipError::WorkerUnavailable)?;
    let (output, sender) = attach_io(
        &mut process,
        WorkerConfig {
            version: VERSION,
            job: job.clone(),
        },
    )?;
    let mut sender = Some(sender);
    let mut protocol = Protocol::new(job);
    let mut failure = None;
    let mut stopping_at = None;
    let mut killed_at = None;
    let mut terminal_at = None;
    let mut exited_at = None;
    let mut eof = false;
    let mut confirmed_exit = false;
    loop {
        // Drain before observing actual EOF: the reader can enqueue its terminal
        // event immediately before its JoinHandle changes to finished.
        while let Ok(event) = output.try_recv() {
            match event {
                Output::Frame(event) => match protocol.accept(event) {
                    Ok(Some(value)) => {
                        if failure.is_none() && !cancel.load(Ordering::Acquire) {
                            progress(value);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                },
                Output::Eof => {
                    eof = true;
                    if protocol.terminal.is_none() {
                        failure.get_or_insert(ClipError::Protocol);
                    }
                }
                Output::Error => {
                    failure.get_or_insert(ClipError::Protocol);
                }
            }
        }
        if cancel.load(Ordering::Acquire) {
            failure = Some(ClipError::Cancelled);
        }
        if failure.is_none()
            && protocol.terminal.is_none()
            && ((!protocol.ready && started.elapsed() >= READY_TIMEOUT)
                || started.elapsed() >= timeout)
        {
            failure = Some(ClipError::TimedOut);
        }
        if protocol.terminal.is_some() {
            terminal_at.get_or_insert_with(Instant::now);
            sender.take();
        }
        if failure.is_some() && stopping_at.is_none() {
            if let Some(sender) = &sender {
                let _ = sender.try_send(Control::Cancel {});
            }
            stopping_at = Some(Instant::now());
        }
        if stopping_at.is_some_and(|at: Instant| at.elapsed() >= STOP_GRACE)
            || terminal_at.is_some_and(|at: Instant| at.elapsed() >= CLEANUP_GRACE)
            || exited_at.is_some_and(|at: Instant| at.elapsed() >= CLEANUP_GRACE)
        {
            if killed_at.is_none() {
                failure.get_or_insert(ClipError::Protocol);
                sender.take();
                process.kill();
                killed_at = Some(Instant::now());
            }
        }
        if confirmed_exit {
            if let Some(error) = failure {
                return Err(error);
            }
            return if eof && protocol.ready {
                let result = protocol.terminal.take().ok_or(ClipError::Protocol)??;
                if let (ClipJob::RenderGif { output, .. }, ClipJobResult::GifRendered(metadata)) =
                    (&protocol.job, &result)
                {
                    // Child exit is required before reading its finished file.
                    // The actual permit remains owned during this bounded decode.
                    crate::video_gif_backend::validate_file(output, metadata, &cancel)?;
                }
                if let (
                    ClipJob::RenderGifAnnotated {
                        output,
                        expected,
                        range,
                        overlays,
                        ..
                    },
                    ClipJobResult::GifRendered(metadata),
                ) = (&protocol.job, &result)
                {
                    let schedule = video_clip_contract::AnnotatedGifSchedule::new(
                        expected,
                        range.as_ref(),
                        overlays,
                    )?;
                    if metadata != &schedule.metadata {
                        return Err(ClipError::Protocol);
                    }
                    crate::video_gif_backend::validate_annotated_file(
                        output,
                        metadata,
                        &schedule.delays,
                        &cancel,
                    )?;
                }
                if let (
                    ClipJob::ExtractFrames {
                        output_directory,
                        expected,
                        window,
                        requested_ticks,
                        ..
                    },
                    ClipJobResult::FramesExtracted { frames },
                ) = (&protocol.job, &result)
                {
                    // Validate independently only after child, Job descendants
                    // and tracked IO have really exited; permit remains held.
                    crate::video_annotation_render::validate_extracted_files(
                        output_directory,
                        expected,
                        window,
                        requested_ticks,
                        frames,
                        &cancel,
                    )?;
                }
                Ok(result)
            } else {
                Err(ClipError::Protocol)
            };
        }
        if let Some(exit) = process
            .try_wait()
            .map_err(|_| ClipError::WorkerUnavailable)?
        {
            exited_at.get_or_insert_with(Instant::now);
            sender.take();
            if !exit.success() {
                failure.get_or_insert(ClipError::WorkerUnavailable);
            }
        }
        if process.fully_exited() {
            confirmed_exit = true;
            continue;
        }
        if killed_at.is_some_and(|at| at.elapsed() >= CLEANUP_GRACE) {
            return Err(ClipError::CleanupTimedOut);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
/// The exact CLI branch must run before any Tauri/DB/asset service setup.
pub fn worker_main() -> i32 {
    let mut input = BufReader::new(std::io::stdin());
    let config: WorkerConfig = match read_frame(&mut input, CONFIG_BYTES)
        .and_then(|v| v.ok_or(ClipError::Protocol))
        .and_then(|v| serde_json::from_slice(&v).map_err(|_| ClipError::Protocol))
    {
        Ok(config) => config,
        Err(_) => return 2,
    };
    if config.version != VERSION || config.job.validate().is_err() {
        return 2;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let invalid_control = Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let bad = invalid_control.clone();
    let controller = match std::thread::Builder::new()
        .name("mewu-video-clip-control".into())
        .spawn(move || {
            let mut received = false;
            loop {
                match read_frame(&mut input, 256) {
                    Ok(Some(bytes))
                        if !received
                            && matches!(
                                serde_json::from_slice::<Control>(&bytes),
                                Ok(Control::Cancel {})
                            ) =>
                    {
                        received = true;
                        token.store(true, Ordering::Release);
                    }
                    Ok(None) => {
                        token.store(true, Ordering::Release);
                        break;
                    }
                    _ => {
                        bad.store(true, Ordering::Release);
                        token.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }) {
        Ok(thread) => thread,
        Err(_) => return 2,
    };
    let emit = |event: &WorkerEvent| write_frame(&mut std::io::stdout().lock(), event, FRAME_BYTES);
    if emit(&WorkerEvent::Ready { version: VERSION }).is_err() {
        return 2;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        video_clip_backend::execute(&config.job, &cancel, &|percent| {
            if emit(&WorkerEvent::Progress { percent }).is_err() {
                cancel.store(true, Ordering::Release);
            }
        })
    }))
    .unwrap_or(Err(ClipError::EncodingFailed));
    let event = if invalid_control.load(Ordering::Acquire) {
        WorkerEvent::Failed {
            error: ClipError::Protocol,
        }
    } else {
        match result {
            Ok(ClipJobResult::Inspected(metadata)) => WorkerEvent::Inspected { metadata },
            Ok(ClipJobResult::Rendered(metadata)) => WorkerEvent::Rendered { metadata },
            Ok(ClipJobResult::GifRendered(metadata)) => WorkerEvent::GifRendered { metadata },
            Ok(ClipJobResult::FramesExtracted { frames }) => {
                WorkerEvent::FramesExtracted { frames }
            }
            Err(ClipError::Cancelled) => WorkerEvent::Cancelled {},
            Err(error) => WorkerEvent::Failed { error },
        }
    };
    if emit(&event).is_err() {
        return 2;
    }
    // Parent closes stdin after terminal; its deadline/Job covers a bad peer.
    if controller.join().is_err() {
        return 2;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "explicit same-HEAD synthetic product executable/Job/pipe/decode/cancel integration; EXE + SHA required"]
    async fn synthetic_product_worker_frames_annotated_mp4_gif_and_cancel() {
        use sha2::{Digest, Sha256};
        use video_clip_contract::{AnnotatedGifSchedule, VideoRange};
        assert!(
            std::env::var_os("MEWU_VIDEO_CLIP_TEST_EXE").is_some(),
            "explicit product exe required"
        );
        let exe = worker_executable().unwrap();
        let exe_hash = format!("{:x}", Sha256::digest(std::fs::read(&exe).unwrap()));
        let (root, source, expected, overlays) =
            crate::video_annotation_render::tests::media_gate::worker_fixture();
        crate::video_host::tests::core_capabilities_native_admission(&root);
        let source_hash = format!("{:x}", Sha256::digest(std::fs::read(&source).unwrap()));
        let noop: Arc<dyn Fn(u8) + Send + Sync> = Arc::new(|_| {});
        let range = VideoRange {
            start_ticks: 2_150_000,
            end_ticks: 11_150_000,
        };
        let requests = vec![
            2_150_000, 3_150_000, 5_500_000, 6_800_000, 8_150_000, 10_150_000,
        ];
        let directory = root.join("frames");
        std::fs::create_dir(&directory).unwrap();
        let ClipJobResult::FramesExtracted { frames } = run(
            ClipJob::ExtractFrames {
                source: source.clone(),
                output_directory: directory.clone(),
                expected: expected.clone(),
                window: range.clone(),
                requested_ticks: requests.clone(),
            },
            Arc::new(AtomicBool::new(false)),
            noop.clone(),
        )
        .await
        .unwrap() else {
            panic!("frames")
        };
        assert!(!busy(), "terminal returns only after real Job and IO exit");
        crate::video_annotation_render::validate_extracted_files(
            &directory,
            &expected,
            &range,
            &requests,
            &frames,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mp4 = root.join("worker-annotated.mp4");
        let ClipJobResult::Rendered(metadata) = run(
            ClipJob::RenderAnnotated {
                source: source.clone(),
                output: mp4.clone(),
                expected: expected.clone(),
                range: Some(range.clone()),
                overlays: overlays.clone(),
            },
            Arc::new(AtomicBool::new(false)),
            noop.clone(),
        )
        .await
        .unwrap() else {
            panic!("mp4")
        };
        assert!(!busy());
        let media_proof = crate::video_annotation_render::tests::media_gate::verify_worker_video(
            &source, &mp4, &range, &overlays,
        );
        let gif = root.join("worker-annotated.gif");
        let ClipJobResult::GifRendered(gif_metadata) = run(
            ClipJob::RenderGifAnnotated {
                source: source.clone(),
                output: gif.clone(),
                expected: expected.clone(),
                range: Some(range.clone()),
                overlays: overlays.clone(),
            },
            Arc::new(AtomicBool::new(false)),
            noop.clone(),
        )
        .await
        .unwrap() else {
            panic!("gif")
        };
        assert!(!busy());
        let schedule = AnnotatedGifSchedule::new(&expected, Some(&range), &overlays).unwrap();
        crate::video_gif_backend::validate_annotated_file(
            &gif,
            &gif_metadata,
            &schedule.delays,
            &AtomicBool::new(false),
        )
        .unwrap();
        let token = Arc::new(AtomicBool::new(false));
        let to_cancel = token.clone();
        let cancelled_output = root.join("worker-cancelled.mp4");
        let result = run(
            ClipJob::RenderAnnotated {
                source: source.clone(),
                output: cancelled_output.clone(),
                expected: expected.clone(),
                range: Some(range),
                overlays,
            },
            token,
            Arc::new(move |percent| {
                if percent >= 5 {
                    to_cancel.store(true, Ordering::Release)
                }
            }),
        )
        .await;
        assert_eq!(result.unwrap_err(), ClipError::Cancelled);
        assert!(wait_until_idle(Duration::from_secs(5)).await);
        // This directory is the host's synthetic staging lease. A late completed
        // child can leave bytes, but cancellation never authorizes publication;
        // remove its exact owned staging output only after the actual Job exit.
        if cancelled_output.exists() {
            std::fs::remove_file(&cancelled_output).unwrap();
        }
        assert!(!cancelled_output.exists());
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&source).unwrap())),
            source_hash
        );
        let proof = serde_json::json!({"syntheticOnly":true,"coreCapabilitiesNativeAdmission":true,"actualProductExecutable":exe,"executableSha256":exe_hash,"sourceSha256":source_hash,"clock":"sourcePlaybackTicks","frames":frames,"mp4Metadata":metadata,"media":media_proof,"gifMetadata":gif_metadata,"gifRequestedTicks":schedule.requested_ticks,"gifDelays":schedule.delays,"cancellation":"actual worker canceled, permit/Job/IO drained before exact host staging cleanup; no publication"});
        std::fs::write(
            root.join("product-video-worker-gate.json"),
            serde_json::to_vec_pretty(&proof).unwrap(),
        )
        .unwrap();
    }
    use crate::video_clip_contract::{VideoMetadata, VideoRange};
    fn metadata() -> VideoMetadata {
        VideoMetadata {
            duration_ticks: 20_000_000,
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            has_audio: true,
        }
    }
    #[test]
    fn strict_frames_reject_partial_oversized_and_unknown_controls() {
        assert!(read_frame(&mut &b"{}"[..], 8).is_err());
        assert!(read_frame(&mut &b"12345678\n"[..], 8).is_err());
        assert_eq!(
            read_frame(&mut &b"{}\r\n"[..], 8).unwrap(),
            Some(b"{}".to_vec())
        );
        assert!(serde_json::from_str::<Control>(r#"{"type":"cancel","extra":true}"#).is_err());
        assert!(
            serde_json::from_str::<WorkerEvent>(r#"{"type":"cancelled","extra":true}"#).is_err()
        );
    }
    #[test]
    fn results_require_ready_and_matching_job_no_duplicate_or_rollback_progress() {
        let inspect = ClipJob::Inspect {
            source: "D:/owned/a.mp4".into(),
        };
        let mut protocol = Protocol::new(inspect.clone());
        assert_eq!(
            protocol.accept(WorkerEvent::Inspected {
                metadata: metadata()
            }),
            Err(ClipError::Protocol)
        );
        let mut protocol = Protocol::new(inspect);
        protocol
            .accept(WorkerEvent::Ready { version: VERSION })
            .unwrap();
        assert!(protocol
            .accept(WorkerEvent::Progress { percent: 0 })
            .is_err());
        protocol
            .accept(WorkerEvent::Inspected {
                metadata: metadata(),
            })
            .unwrap();
        assert!(protocol.accept(WorkerEvent::Cancelled {}).is_err());
        let mut protocol = Protocol::new(ClipJob::Render {
            source: "D:/owned/a.mp4".into(),
            output: "D:/stage/b.mp4".into(),
            expected: metadata(),
            range: VideoRange {
                start_ticks: 1_000_000,
                end_ticks: 11_000_000,
            },
        });
        protocol
            .accept(WorkerEvent::Ready { version: VERSION })
            .unwrap();
        protocol
            .accept(WorkerEvent::Progress { percent: 40 })
            .unwrap();
        assert!(protocol
            .accept(WorkerEvent::Progress { percent: 39 })
            .is_err());
        assert!(protocol
            .accept(WorkerEvent::Rendered {
                metadata: metadata()
            })
            .is_err());
    }
    #[test]
    fn gif_terminal_cannot_spoof_mp4_or_another_interval() {
        let source = metadata();
        let mut protocol = Protocol::new(ClipJob::RenderGif {
            source: std::env::temp_dir().join("owned-source.mp4"),
            output: std::env::temp_dir().join("owned-output.gif"),
            expected: source.clone(),
            range: None,
        });
        protocol
            .accept(WorkerEvent::Ready { version: VERSION })
            .unwrap();
        assert!(protocol
            .accept(WorkerEvent::Rendered {
                metadata: source.clone()
            })
            .is_err());
        let correct = video_clip_contract::GifMetadata::for_source(&source, None).unwrap();
        let mut wrong = correct.clone();
        wrong.frame_count += 1;
        assert!(protocol
            .accept(WorkerEvent::GifRendered { metadata: wrong })
            .is_err());
        protocol
            .accept(WorkerEvent::GifRendered {
                metadata: correct.clone(),
            })
            .unwrap();
        assert!(
            matches!(protocol.terminal,Some(Ok(ClipJobResult::GifRendered(ref value))) if value==&correct)
        );
        assert!(protocol
            .accept(WorkerEvent::GifRendered { metadata: correct })
            .is_err());
    }
    #[test]
    fn extracted_terminal_rejects_wrong_job_time_duplicate_index_and_requires_ready() {
        use crate::video_clip_contract::FrameDescriptor;
        let expected = metadata();
        let requested_ticks = vec![2_150_000, 12_150_000];
        let frames = requested_ticks
            .iter()
            .enumerate()
            .map(|(index, &tick)| FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: tick - 150_000,
                sample_duration_ticks: 333_333,
                encoded_width: 320,
                encoded_height: 180,
                jpeg_sha256: "a".repeat(64),
                bytes_length: 1000,
            })
            .collect::<Vec<_>>();
        let job = ClipJob::ExtractFrames {
            source: "D:/owned/source.mp4".into(),
            output_directory: "D:/stage/frames".into(),
            expected: expected.clone(),
            window: VideoRange {
                start_ticks: 0,
                end_ticks: expected.duration_ticks,
            },
            requested_ticks,
        };
        let mut protocol = Protocol::new(job.clone());
        assert!(protocol
            .accept(WorkerEvent::FramesExtracted {
                frames: frames.clone()
            })
            .is_err());
        let mut protocol = Protocol::new(job.clone());
        protocol
            .accept(WorkerEvent::Ready { version: VERSION })
            .unwrap();
        assert!(protocol
            .accept(WorkerEvent::Inspected {
                metadata: expected.clone()
            })
            .is_err());
        let mut wrong = frames.clone();
        wrong[1].requested_ticks += 1;
        assert!(protocol
            .accept(WorkerEvent::FramesExtracted { frames: wrong })
            .is_err());
        let mut wrong = frames.clone();
        wrong[1].index = 0;
        assert!(protocol
            .accept(WorkerEvent::FramesExtracted { frames: wrong })
            .is_err());
        protocol
            .accept(WorkerEvent::FramesExtracted {
                frames: frames.clone(),
            })
            .unwrap();
        assert!(matches!(
            protocol.terminal,
            Some(Ok(ClipJobResult::FramesExtracted { .. }))
        ));
        assert!(protocol
            .accept(WorkerEvent::FramesExtracted {
                frames: frames.clone()
            })
            .is_err());
        let mut inspect = Protocol::new(ClipJob::Inspect {
            source: "D:/owned/source.mp4".into(),
        });
        inspect
            .accept(WorkerEvent::Ready { version: VERSION })
            .unwrap();
        assert!(inspect
            .accept(WorkerEvent::FramesExtracted { frames })
            .is_err());
    }
    #[tokio::test]
    async fn precancelled_job_never_launches_native_media() {
        assert_eq!(
            run(
                ClipJob::Inspect {
                    source: "missing".into()
                },
                Arc::new(AtomicBool::new(true)),
                Arc::new(|_| panic!("no callback"))
            )
            .await,
            Err(ClipError::Cancelled)
        );
    }
    #[test]
    fn dropping_waiter_only_revokes_token_not_actual_slot() {
        let token = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(Mutex::new(Some(token.clone())));
        let permit = Permit {
            slot: slot.clone(),
            cancel: token.clone(),
        };
        drop(CancelOnDrop(Some(token.clone())));
        assert!(token.load(Ordering::Acquire));
        assert!(slot.lock().unwrap().is_some());
        drop(permit);
        assert!(slot.lock().unwrap().is_none());
    }
}
