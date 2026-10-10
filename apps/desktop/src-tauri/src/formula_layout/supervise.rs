// SPDX-License-Identifier: MPL-2.0
use super::{fonts, protocol::*, render, *};
use crate::owned_process::ManagedProcess;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, LazyLock, Mutex, Weak,
    },
    time::{Duration, Instant},
};

struct Gate {
    accepting: bool,
    requests: Vec<Weak<CancellationState>>,
    active_cancel: Option<Weak<CancellationState>>,
}
struct Admission {
    gate: Mutex<Gate>,
    actual: AtomicBool,
}
impl Admission {
    fn new() -> Self {
        Self {
            gate: Mutex::new(Gate {
                accepting: true,
                requests: Vec::new(),
                active_cancel: None,
            }),
            actual: AtomicBool::new(false),
        }
    }
    fn register(&self, cancel: &Cancellation) -> Result<(), String> {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if !gate.accepting {
            return Err("formula_exiting".into());
        }
        gate.requests.retain(|entry| entry.strong_count() != 0);
        let weak = Arc::downgrade(&cancel.0);
        if !gate.requests.iter().any(|entry| Weak::ptr_eq(entry, &weak)) {
            if gate.requests.len() >= 128 {
                return Err("formula_request_budget".into());
            }
            gate.requests.push(weak);
        }
        Ok(())
    }
    fn cancel_all(&self) -> usize {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        gate.accepting = false;
        gate.requests.retain(|entry| entry.strong_count() != 0);
        let mut count = 0;
        for request in gate.requests.iter().filter_map(Weak::upgrade) {
            request.cancelled.store(true, Ordering::Release);
            count += 1;
        }
        if let Some(current) = gate.active_cancel.as_ref().and_then(Weak::upgrade) {
            current.cancelled.store(true, Ordering::Release);
        }
        count
    }
    fn resume_after_failed_exit(&self) {
        // Host must already be RUNNING. Keep every old token cancelled and its
        // real Permit occupied; only fresh requests can eventually acquire.
        self.gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accepting = true;
    }
    fn wait_until_idle(&self, timeout: Duration) -> bool {
        let clock = Instant::now();
        while self.actual.load(Ordering::Acquire) {
            if clock.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
    fn acquire(self: &Arc<Self>, cancel: Cancellation) -> Result<Permit, String> {
        // The same mutex serializes gate closure, real slot acquisition, abort
        // reopening and release. A closed-gate zero-duration idle check cannot
        // miss an acquisition allowed by an earlier, stale open observation.
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if !gate.accepting {
            return Err("formula_exiting".into());
        }
        if cancel.is_cancelled() {
            return Err("formula_cancelled".into());
        }
        self.actual
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "formula_worker_busy")?;
        gate.active_cancel = Some(Arc::downgrade(&cancel.0));
        cancel.0.worker_started.store(false, Ordering::Release);
        cancel.0.active.store(true, Ordering::Release);
        Ok(Permit {
            _held: None,
            cancel,
            admission: self.clone(),
        })
    }
}
static ADMISSION: LazyLock<Arc<Admission>> = LazyLock::new(|| Arc::new(Admission::new()));
fn register(cancel: &Cancellation) -> Result<(), String> {
    ADMISSION.register(cancel)
}
pub(super) fn cancel_all() -> usize {
    ADMISSION.cancel_all()
}
pub(super) fn resume_after_failed_exit() {
    ADMISSION.resume_after_failed_exit();
}
pub(super) fn wait_until_idle(timeout: Duration) -> bool {
    ADMISSION.wait_until_idle(timeout)
}
pub(super) fn active() -> bool {
    ADMISSION.actual.load(Ordering::Acquire)
}
pub(super) fn cancel_active() -> bool {
    let current = ADMISSION
        .gate
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active_cancel
        .as_ref()
        .and_then(Weak::upgrade);
    if let Some(current) = current {
        current.cancelled.store(true, Ordering::Release);
        return true;
    }
    false
}
struct JobDirectory {
    path: PathBuf,
    _parent: File,
    pin: Option<File>,
}
impl JobDirectory {
    fn create(root: &Path) -> Result<Self, String> {
        let parent = fonts::pin_directory(root)?;
        let root = root.canonicalize().map_err(|_| "formula_work_root")?;
        let path = root.join(format!("formula-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|_| "formula_job_directory")?;
        let mut job = Self {
            path,
            _parent: parent,
            pin: None,
        };
        job.pin = Some(fonts::pin_directory(&job.path)?);
        if job
            .path
            .canonicalize()
            .map_err(|_| "formula_job_directory")?
            .parent()
            != Some(root.as_path())
        {
            return Err("formula_job_directory".into());
        }
        Ok(job)
    }
}
impl Drop for JobDirectory {
    fn drop(&mut self) {
        // Only the one output name produced by this job, no recursive deletion,
        // no stale user/cache scan. A reaper owns this until real process exit.
        let _ = fs::remove_file(self.path.join("layout.png"));
        drop(self.pin.take());
        let _ = fs::remove_dir(&self.path); // Unknown files make this fail safely.
    }
}
struct HeldResources {
    fonts: fonts::PinnedFonts,
    directory: JobDirectory,
}
struct Permit {
    _held: Option<Arc<HeldResources>>,
    cancel: Cancellation,
    admission: Arc<Admission>,
}
impl Permit {
    fn acquire(cancel: Cancellation) -> Result<Self, String> {
        ADMISSION.acquire(cancel)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut gate = self
            .admission
            .gate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.cancel.0.active.store(false, Ordering::Release);
        gate.active_cancel = None;
        self.admission.actual.store(false, Ordering::Release);
    }
}

enum ReadEvent {
    Frame(Event),
    Eof,
    Error,
}
fn reader(output: impl Read, sender: mpsc::SyncSender<ReadEvent>) {
    let mut reader = BufReader::new(output);
    // Exactly Ready, Result, Done, EOF. Keep reading after Done so upstream
    // trailing stdout cannot be accepted as a successful strict protocol.
    for _ in 0..4 {
        let mut bytes = Vec::new();
        let event = match reader
            .by_ref()
            .take(MAX_FRAME as u64 + 1)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) => {
                let _ = sender.send(ReadEvent::Eof);
                return;
            }
            Ok(_) => decode_frame(&bytes)
                .map(ReadEvent::Frame)
                .unwrap_or(ReadEvent::Error),
            Err(_) => ReadEvent::Error,
        };
        let end = matches!(event, ReadEvent::Error);
        if sender.send(event).is_err() || end {
            return;
        }
    }
    let _ = sender.send(ReadEvent::Error);
}

fn identity_valid(actual: &Identity, config: &Config) -> bool {
    let fonts = &actual.fonts;
    let faces = [&fonts.primary, &fonts.secondary, &fonts.emoji];
    actual.renderer_id == RENDERER_ID
        && actual.style == StyleIdentity::for_input(&config.input)
        && fonts.katex_inventory_sha256 == fonts::INVENTORY_SHA256
        && faces.iter().all(|f| {
            f.as_ref().is_none_or(|f| {
                f.bytes > 0 && f.bytes <= 64 * 1024 * 1024 && valid_sha256(&f.sha256)
            })
        })
        && faces
            .iter()
            .filter_map(|f| f.as_ref())
            .map(|f| f.bytes)
            .sum::<u64>()
            <= 128 * 1024 * 1024
        && fonts
            .primary
            .as_ref()
            .is_some_and(|f| f.sha256 == config.primary_sha256)
}

pub(super) fn run(
    input: FormulaInput,
    resources: Resources,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    register(&cancel)?;
    input.validate()?;
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    if !cfg!(windows) {
        return Err("formula_worker_platform_unsupported".into());
    }
    let permit = Permit::acquire(cancel.clone())?;
    run_owned(input, resources, cancel, permit)
}
pub(super) fn run_embedded(
    input: FormulaInput,
    root: PathBuf,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    register(&cancel)?;
    input.validate()?;
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    if !cfg!(windows) {
        return Err("formula_worker_platform_unsupported".into());
    }
    // Include lazy resource extraction in the native work/exit-observation slot.
    let permit = Permit::acquire(cancel.clone())?;
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    let resources = Resources::from_embedded(root)?;
    run_owned(input, resources, cancel, permit)
}
fn worker_executable() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("MEWU_FORMULA_TEST_EXE") {
        let path = PathBuf::from(path);
        let expected = std::env::var("MEWU_FORMULA_TEST_EXE_SHA256")
            .map_err(|_| "formula_test_worker_hash_missing")?;
        if !path.is_absolute()
            || expected.len() != 64
            || !expected.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("formula_test_worker_identity_invalid".into());
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| "formula_test_worker_missing")?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > 128 * 1024 * 1024
        {
            return Err("formula_test_worker_identity_invalid".into());
        }
        let bytes = fs::read(&path).map_err(|_| "formula_test_worker_unreadable")?;
        if !render::digest(&bytes).eq_ignore_ascii_case(&expected) {
            return Err("formula_test_worker_hash_mismatch".into());
        }
        return Ok(path);
    }
    std::env::current_exe().map_err(|_| "formula_current_exe".into())
}

fn run_owned(
    input: FormulaInput,
    resources: Resources,
    cancel: Cancellation,
    mut permit: Permit,
) -> Result<FormulaLayout, String> {
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    let held = Arc::new(HeldResources {
        fonts: fonts::pin_fonts(&resources.font_directory, &resources.unicode_font)?,
        directory: JobDirectory::create(&resources.work_root)?,
    });
    permit._held = Some(held.clone());
    let config = Config {
        request_id: uuid::Uuid::new_v4().to_string(),
        input: input.clone(),
        font_directory: held.fonts.directory.clone(),
        unicode_font: held.fonts.unicode_font.clone(),
        primary_sha256: held.fonts.primary_sha256.clone(),
        output_directory: held.directory.path.clone(),
    };
    let encoded = serde_json::to_vec(&config).map_err(|_| "formula_config_encode")?;
    if encoded.len() > MAX_CONFIG {
        return Err("formula_config_budget".into());
    }
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    let mut command = Command::new(worker_executable()?);
    command
        .arg("--formula-layout-worker")
        .env_remove("RATEX_UNICODE_FONT")
        .env("RATEX_UNICODE_FONT", &held.fonts.unicode_font)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child =
        ManagedProcess::spawn(&mut command, permit).map_err(|_| "formula_spawn_or_job_failed")?;
    cancel.0.worker_started.store(true, Ordering::Release);
    // The existing ManagedProcess attaches the kill-on-close Job first. No
    // renderer input is written before that attachment succeeds.
    let mut stdin = child
        .child_mut()
        .stdin
        .take()
        .ok_or("formula_stdin_missing")?;
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or("formula_stdout_missing")?;
    child.track_io(
        std::thread::Builder::new()
            .name("mewu-formula-input".into())
            .spawn(move || {
                let _ = stdin.write_all(&encoded);
                let _ = stdin.flush();
            })
            .map_err(|_| "formula_input_thread")?,
    );
    // At most five messages, including failure/EOF, so cancellation cannot
    // leave a pipe reader blocked on a full queue while cleanup waits for IO.
    let (sender, receiver) = mpsc::sync_channel(5);
    child.track_io(
        std::thread::Builder::new()
            .name("mewu-formula-output".into())
            .spawn(move || reader(stdout, sender))
            .map_err(|_| "formula_output_thread")?,
    );
    let mut deadline = Instant::now() + Duration::from_secs(5);
    let mut identity = None;
    let mut result = None;
    let mut done = false;
    let outcome = loop {
        if cancel.is_cancelled() {
            break Err("formula_cancelled".to_string());
        }
        if Instant::now() >= deadline {
            break Err("formula_deadline".to_string());
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(ReadEvent::Frame(Event::Ready { identity: actual }))
                if identity.is_none() && result.is_none() && !done =>
            {
                if !identity_valid(&actual, &config) {
                    break Err("formula_identity_mismatch".into());
                }
                identity = Some(actual);
                deadline = Instant::now() + Duration::from_secs(2);
            }
            Ok(ReadEvent::Frame(Event::Result { result: actual }))
                if identity.is_some() && result.is_none() && !done =>
            {
                if actual.request_id != config.request_id
                    || actual.accepted_input_chars > MAX_INPUT_CHARS
                {
                    break Err("formula_result_identity".into());
                }
                result = Some(actual);
            }
            Ok(ReadEvent::Frame(Event::Done {}))
                if identity.is_some() && result.is_some() && !done =>
            {
                done = true;
            }
            Ok(ReadEvent::Eof) if done => break Ok(()),
            Ok(_) => break Err("formula_protocol".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("formula_protocol_incomplete".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    if outcome.is_err() {
        child.kill();
    }
    let cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while !child.fully_exited() && Instant::now() < cleanup_deadline {
        if outcome.is_err() || cancel.is_cancelled() {
            child.kill();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !child.fully_exited() {
        child.kill();
        // Drop transfers real Job/IO/permit AND the exact directory/font handles
        // to the shared reaper. Neither slot nor directory is released early.
        return Err("formula_cleanup_pending".into());
    }
    outcome?;
    if !matches!(child.try_wait(),Ok(Some(exit)) if exit.success()) {
        return Err("formula_worker_exit".into());
    }
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    let result = result.ok_or("formula_protocol_incomplete")?;
    if result.status != "rendered" {
        // Fixed vocabulary only. Do not return renderer/parser text or TeX.
        return Err(match result.status.as_str() {
            "missing_glyph" => "formula_missing_glyph",
            "parse_error" => "formula_parse_error",
            "renderer_panic" => "formula_renderer_panic",
            "input_budget" | "layout_budget" | "svg_budget" | "pixel_budget" | "png_budget" => {
                "formula_render_budget"
            }
            _ => "formula_render_rejected",
        }
        .into());
    }
    if result.accepted_input_chars != input.tex.chars().count() || !result.missing_glyphs.is_empty()
    {
        return Err("formula_result_identity".into());
    }
    let metrics = result.metrics.ok_or("formula_metrics_missing")?;
    let png = verify_png(&held.directory.path, &metrics)?;
    if cancel.is_cancelled() {
        return Err("formula_cancelled".into());
    }
    let identity = identity.ok_or("formula_identity_missing")?;
    Ok(FormulaLayout {
        input,
        fonts: identity.fonts,
        style: identity.style,
        width: metrics.width,
        height: metrics.height,
        png,
    })
}

fn verify_png(directory: &Path, metrics: &Metrics) -> Result<Vec<u8>, String> {
    if metrics.width == 0
        || metrics.height == 0
        || metrics.width > MAX_SIDE
        || metrics.height > MAX_SIDE
        || u64::from(metrics.width) * u64::from(metrics.height) > MAX_PIXELS
        || metrics.png_bytes == 0
        || metrics.png_bytes > MAX_PNG
        || metrics.svg_bytes > MAX_SVG
        || metrics.glyph_count > 8192
        || metrics.nonspace_glyph_count > metrics.glyph_count
        || metrics.path_count > 20_000
        || !metrics.missing_glyphs.is_empty()
        || metrics.touches_canvas_edge
        || !valid_sha256(&metrics.png_sha256)
        || !valid_sha256(&metrics.rgba_sha256)
        || !valid_sha256(&metrics.svg_sha256)
        || metrics.view_box.iter().any(|v| !v.is_finite())
        || metrics.view_box[0] != 0.0
        || metrics.view_box[1] != 0.0
        || metrics.view_box[2] <= 0.0
        || metrics.view_box[3] <= 0.0
        || metrics.view_box[2].ceil() != f64::from(metrics.width)
        || metrics.view_box[3].ceil() != f64::from(metrics.height)
    {
        return Err("formula_metrics_invalid".into());
    }
    let file = fonts::pin_file(&directory.join("layout.png"), MAX_PNG as u64)?;
    let png = fonts::read_bounded(&file, MAX_PNG as u64)?;
    if png.len() != metrics.png_bytes || render::digest(&png) != metrics.png_sha256 {
        return Err("formula_png_hash".into());
    }
    // Check dimensions with the real PNG decoder before allowing tiny-skia to
    // allocate. No trusting MIME, extension, renderer-reported dimensions or
    // hand-parsing a supposed IHDR byte offset.
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    let decoder = image::codecs::png::PngDecoder::with_limits(std::io::Cursor::new(&png), limits)
        .map_err(|_| "formula_png_decode")?;
    if decoder.dimensions() != (metrics.width, metrics.height) {
        return Err("formula_png_dimensions".into());
    }
    drop(decoder);
    let decoded = resvg::tiny_skia::Pixmap::decode_png(&png).map_err(|_| "formula_png_decode")?;
    if decoded.width() != metrics.width
        || decoded.height() != metrics.height
        || render::digest(decoded.data()) != metrics.rgba_sha256
    {
        return Err("formula_png_roundtrip".into());
    }
    let mut ink = [metrics.width, metrics.height, 0, 0];
    let mut count = 0u64;
    for (index, pixel) in decoded.data().chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        count += 1;
        let x = index as u32 % metrics.width;
        let y = index as u32 / metrics.width;
        ink[0] = ink[0].min(x);
        ink[1] = ink[1].min(y);
        ink[2] = ink[2].max(x + 1);
        ink[3] = ink[3].max(y + 1);
    }
    if count == 0
        || ink != metrics.ink_bounds
        || ink[0] == 0
        || ink[1] == 0
        || ink[2] == metrics.width
        || ink[3] == metrics.height
    {
        return Err("formula_png_ink".into());
    }
    Ok(png)
}

pub(super) fn worker() -> Result<(), String> {
    if !cfg!(windows) {
        return Err("formula_worker_platform_unsupported".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_CONFIG as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "formula_config_read")?;
    if bytes.len() > MAX_CONFIG {
        return Err("formula_config_budget".into());
    }
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "formula_config_json")?;
    config.input.validate()?;
    if uuid::Uuid::parse_str(&config.request_id).is_err()
        || !config.output_directory.is_absolute()
        || !config.font_directory.is_absolute()
        || !config.unicode_font.is_absolute()
        || !valid_sha256(&config.primary_sha256)
    {
        return Err("formula_config_invalid".into());
    }
    if std::env::var_os("RATEX_UNICODE_FONT").as_deref() != Some(config.unicode_font.as_os_str()) {
        return Err("formula_font_environment".into());
    }
    let _output_pin = fonts::pin_directory(&config.output_directory)?;
    if fs::read_dir(&config.output_directory)
        .map_err(|_| "formula_output_directory")?
        .next()
        .is_some()
    {
        return Err("formula_output_not_empty".into());
    }
    let pinned = fonts::pin_fonts(&config.font_directory, &config.unicode_font)?;
    if pinned.primary_sha256 != config.primary_sha256 {
        return Err("formula_primary_font_mismatch".into());
    }
    let identity = Identity {
        renderer_id: RENDERER_ID.into(),
        fonts: fonts::snapshot(&config.primary_sha256)?,
        style: StyleIdentity::for_input(&config.input),
    };
    emit(&Event::Ready { identity })?;
    emit(&Event::Result {
        result: render::execute(&config),
    })?;
    emit(&Event::Done {})
}
fn emit(event: &Event) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(event).map_err(|_| "formula_event_encode")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_FRAME {
        return Err("formula_event_budget".into());
    }
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|_| stdout.flush())
        .map_err(|_| "formula_event_write".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aborted_exit_reopens_only_fresh_requests_without_freeing_old_permit() {
        let admission = Arc::new(Admission::new());
        let old = Cancellation::default();
        admission.register(&old).unwrap();
        let old_permit = admission.acquire(old.clone()).unwrap();
        assert!(old.is_active());
        assert_eq!(admission.cancel_all(), 1);
        assert!(old.is_cancelled());
        assert!(!admission.wait_until_idle(Duration::ZERO));
        let fresh = Cancellation::default();
        assert_eq!(
            admission.register(&fresh).err().as_deref(),
            Some("formula_exiting")
        );
        admission.resume_after_failed_exit();
        assert!(old.is_cancelled() && old.is_active());
        admission.register(&fresh).unwrap();
        assert_eq!(
            admission.acquire(fresh.clone()).err().as_deref(),
            Some("formula_worker_busy")
        );
        assert_eq!(
            admission.acquire(old.clone()).err().as_deref(),
            Some("formula_cancelled")
        );
        drop(old_permit);
        assert!(!old.is_active() && old.is_cancelled());
        assert!(admission.wait_until_idle(Duration::ZERO));
        let fresh_permit = admission.acquire(fresh.clone()).unwrap();
        assert!(fresh.is_active() && !fresh.is_cancelled());
        drop(fresh_permit);
        assert!(admission.wait_until_idle(Duration::ZERO));
    }
    #[test]
    fn closed_gate_blocks_previously_registered_queued_acquisition() {
        let admission = Arc::new(Admission::new());
        let queued = Cancellation::default();
        admission.register(&queued).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (child_admission, child_queued, child_barrier) =
            (admission.clone(), queued.clone(), barrier.clone());
        let thread = std::thread::spawn(move || {
            child_barrier.wait();
            child_admission.acquire(child_queued).err()
        });
        admission.cancel_all();
        assert!(admission.wait_until_idle(Duration::ZERO));
        barrier.wait();
        assert_eq!(thread.join().unwrap().as_deref(), Some("formula_exiting"));
        admission.resume_after_failed_exit();
        assert!(queued.is_cancelled());
        assert_eq!(
            admission.acquire(queued).err().as_deref(),
            Some("formula_cancelled")
        );
        assert!(admission.wait_until_idle(Duration::ZERO));
    }
    #[test]
    fn reader_requires_eof_and_rejects_trailing_stdout() {
        let (tx, rx) = mpsc::sync_channel(5);
        reader(
            std::io::Cursor::new(b"{\"type\":\"done\"}\nupstream log\n"),
            tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            ReadEvent::Frame(Event::Done {})
        ));
        assert!(matches!(rx.recv().unwrap(), ReadEvent::Error));
    }
    #[test]
    fn malformed_metrics_fail_before_any_file_is_read() {
        let metrics = Metrics {
            svg_bytes: 1,
            svg_sha256: "a".repeat(64),
            view_box: [0.0, 0.0, 1.0, 1.0],
            width: 0,
            height: 1,
            glyph_count: 1,
            nonspace_glyph_count: 1,
            path_count: 1,
            missing_glyphs: vec![],
            png_bytes: 1,
            png_sha256: "a".repeat(64),
            rgba_sha256: "a".repeat(64),
            ink_bounds: [0, 0, 1, 1],
            touches_canvas_edge: false,
        };
        assert_eq!(
            verify_png(Path::new("does-not-exist"), &metrics)
                .err()
                .as_deref(),
            Some("formula_metrics_invalid")
        );
    }
}
