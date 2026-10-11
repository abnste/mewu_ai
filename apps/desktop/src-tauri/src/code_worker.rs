// SPDX-License-Identifier: MPL-2.0
//! Supervised local decoder, with no Host/scene/URL/clipboard dependency.
use crate::{
    code_decode::{self, CodeError, CodeImage, DecodedCode},
    owned_process::ManagedProcess,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
const VERSION: u32 = 1;
const MAX_HEADER: usize = 256;
const MAX_FRAME: usize = code_decode::MAX_RESULT_BYTES + 1024;
const MAX_OUTPUT: usize = 2 * MAX_FRAME;
const START_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const TEXT_TIMEOUT: Duration = Duration::from_secs(25);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
struct Timeouts {
    startup: Duration,
    total: Duration,
    cleanup: Duration,
}
impl Timeouts {
    fn for_mode(mode: ScanMode) -> Self {
        Self {
            startup: START_TIMEOUT,
            total: if mode.has_text() {
                TEXT_TIMEOUT
            } else {
                TOTAL_TIMEOUT
            },
            cleanup: CLEANUP_TIMEOUT,
        }
    }
    fn expired(self, ready: bool, elapsed: Duration) -> bool {
        (!ready && elapsed >= self.startup) || elapsed >= self.total
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputHeader {
    version: u32,
    width: u32,
    height: u32,
    length: usize,
    #[serde(default)]
    mode: ScanMode,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Ready { version: u32 },
    Completed { codes: Vec<DecodedCode> },
    Failed { error: CodeError },
}

type Slot = Arc<Mutex<Option<Arc<AtomicBool>>>>;
fn slot() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None)))
}
pub fn busy() -> bool {
    slot().lock().map_or(true, |slot| slot.is_some())
}
pub fn cancel_all() {
    if let Ok(slot) = slot().lock() {
        if let Some(token) = slot.as_ref() {
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
    fn acquire(token: Arc<AtomicBool>) -> Result<Self, CodeError> {
        if token.load(Ordering::Acquire) {
            return Err(CodeError::Cancelled);
        }
        let slot = slot().clone();
        {
            let mut guard = slot.lock().map_err(|_| CodeError::Busy)?;
            if guard.is_some() {
                return Err(CodeError::Busy);
            }
            *guard = Some(token.clone());
        }
        Ok(Self { slot, token })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            if slot.as_ref().is_some_and(|t| Arc::ptr_eq(t, &self.token)) {
                *slot = None;
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

pub async fn run(image: CodeImage, cancel: Arc<AtomicBool>) -> Result<Vec<DecodedCode>, CodeError> {
    run_scan(image, ScanMode::Codes, cancel).await
}
pub async fn run_scan(
    image: CodeImage,
    mode: ScanMode,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<DecodedCode>, CodeError> {
    image.validate()?;
    if cancel.load(Ordering::Acquire) {
        return Err(CodeError::Cancelled);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = image;
        return Err(CodeError::Unsupported);
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let permit = Permit::acquire(cancel.clone())?;
        let mut guard = CancelOnDrop(Some(cancel.clone()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("mewu-code-supervisor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    supervise(image, mode, cancel, permit)
                }))
                .unwrap_or(Err(CodeError::WorkerUnavailable));
                let _ = tx.send(result);
            })
            .map_err(|_| CodeError::WorkerUnavailable)?;
        let result = rx.await.map_err(|_| CodeError::WorkerUnavailable)?;
        guard.0 = None;
        result
    }
}

fn read_line(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, CodeError> {
    let mut out = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| CodeError::Protocol)?;
        if available.is_empty() {
            return if out.is_empty() {
                Ok(None)
            } else {
                Err(CodeError::Protocol)
            };
        }
        let ending = available.iter().position(|&b| b == b'\n');
        let count = ending.map_or(available.len(), |position| position + 1);
        if out.len().saturating_add(count) > limit {
            return Err(CodeError::Protocol);
        }
        out.extend_from_slice(&available[..count]);
        reader.consume(count);
        if ending.is_some() {
            out.pop();
            return Ok(Some(out));
        }
    }
}
fn write_event(event: &Event) -> Result<(), CodeError> {
    let bytes = serde_json::to_vec(event).map_err(|_| CodeError::Protocol)?;
    if bytes.len() + 1 > MAX_FRAME {
        return Err(CodeError::ResultLimit);
    }
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|_| CodeError::Protocol)
}
enum Output {
    Frame(Event),
    Eof,
    Error,
}
fn attach_io(
    process: &mut ManagedProcess<Permit>,
    image: CodeImage,
    mode: ScanMode,
) -> Result<
    (
        mpsc::Receiver<Output>,
        mpsc::Receiver<Result<(), CodeError>>,
    ),
    CodeError,
> {
    let stdout = process
        .child_mut()
        .stdout
        .take()
        .ok_or(CodeError::WorkerUnavailable)?;
    attach_io_from(process, image, mode, stdout)
}
fn attach_io_from(
    process: &mut ManagedProcess<Permit>,
    image: CodeImage,
    mode: ScanMode,
    output: impl Read + Send + 'static,
) -> Result<
    (
        mpsc::Receiver<Output>,
        mpsc::Receiver<Result<(), CodeError>>,
    ),
    CodeError,
> {
    let (out_tx, out_rx) = mpsc::sync_channel(4);
    let reader = std::thread::Builder::new()
        .name("mewu-code-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(output);
            let mut total = 0usize;
            loop {
                let output = match read_line(&mut reader, MAX_FRAME) {
                    Ok(Some(bytes)) => {
                        total = total.saturating_add(bytes.len() + 1);
                        if total > MAX_OUTPUT {
                            Output::Error
                        } else {
                            match serde_json::from_slice(&bytes) {
                                Ok(event) => Output::Frame(event),
                                Err(_) => Output::Error,
                            }
                        }
                    }
                    Ok(None) => Output::Eof,
                    Err(_) => Output::Error,
                };
                let end = !matches!(output, Output::Frame(_));
                if out_tx.send(output).is_err() || end {
                    break;
                }
            }
        })
        .map_err(|_| CodeError::WorkerUnavailable)?;
    process.track_io(reader);
    let mut stdin = process
        .child_mut()
        .stdin
        .take()
        .ok_or(CodeError::WorkerUnavailable)?;
    let (write_tx, write_rx) = mpsc::sync_channel(1);
    let writer = std::thread::Builder::new()
        .name("mewu-code-input".into())
        .spawn(move || {
            let result = (|| {
                let header = serde_json::to_vec(&InputHeader {
                    version: VERSION,
                    width: image.width,
                    height: image.height,
                    length: image.luma.len(),
                    mode,
                })
                .map_err(|_| CodeError::Protocol)?;
                if header.len() + 1 > MAX_HEADER {
                    return Err(CodeError::Protocol);
                }
                // Only this thread can block on the <=4MiB write. Supervisor retains
                // its own cancellation/deadline loop and can close the remote pipe.
                stdin
                    .write_all(&header)
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.write_all(&image.luma))
                    .and_then(|_| stdin.flush())
                    .map_err(|_| CodeError::Protocol)
            })();
            drop(stdin);
            drop(image);
            let _ = write_tx.send(result);
        })
        .map_err(|_| CodeError::WorkerUnavailable)?;
    process.track_io(writer);
    Ok((out_rx, write_rx))
}

fn supervise(
    image: CodeImage,
    mode: ScanMode,
    cancel: Arc<AtomicBool>,
    permit: Permit,
) -> Result<Vec<DecodedCode>, CodeError> {
    let executable = std::env::current_exe().map_err(|_| CodeError::WorkerUnavailable)?;
    supervise_executable(image, mode, cancel, permit, &executable)
}
fn supervise_executable(
    image: CodeImage,
    mode: ScanMode,
    cancel: Arc<AtomicBool>,
    permit: Permit,
    executable: &std::path::Path,
) -> Result<Vec<DecodedCode>, CodeError> {
    let started = Instant::now();
    if cancel.load(Ordering::Acquire) {
        return Err(CodeError::Cancelled);
    }
    #[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
    let mut process = {
        use crate::owned_process::linux_worker::{TrustedCommand, Worker};
        // The factory pins this actual loaded Mewu ELF. No candidate pathname
        // can replace it, even in the explicit package verification phase.
        let _ = executable;
        let mut command = TrustedCommand::new(Worker::Code)
            .map_err(|_| CodeError::WorkerUnavailable)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        ManagedProcess::spawn_trusted(command, permit, started + START_TIMEOUT, &cancel)
            .map_err(|_| CodeError::WorkerUnavailable)?
    };
    #[cfg(not(all(target_os = "linux", feature = "linux-worker-tests")))]
    let mut process = {
        let mut command = Command::new(executable);
        command
            .arg("--code-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        for name in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        ManagedProcess::spawn(&mut command, permit).map_err(|_| CodeError::WorkerUnavailable)?
    };
    let (output, writer) = attach_io(&mut process, image, mode)?;
    supervise_process(
        process,
        output,
        writer,
        cancel,
        started,
        Timeouts::for_mode(mode),
    )
}
fn supervise_process(
    mut process: ManagedProcess<Permit>,
    output: mpsc::Receiver<Output>,
    writer: mpsc::Receiver<Result<(), CodeError>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    timeouts: Timeouts,
) -> Result<Vec<DecodedCode>, CodeError> {
    let mut ready = false;
    let mut eof = false;
    let mut write_done = false;
    let mut terminal: Option<Result<Vec<DecodedCode>, CodeError>> = None;
    let mut failure = None;
    let mut killed_at = None;
    let mut exited_at = None;
    loop {
        if cancel.load(Ordering::Acquire) {
            failure = Some(CodeError::Cancelled);
        }
        // A terminal frame is not completion: the owned child, descendants
        // and tracked pipes must still finish within this same mode budget.
        if failure.is_none() && timeouts.expired(ready, started.elapsed()) {
            failure = Some(CodeError::TimedOut);
        }
        if !write_done {
            match writer.try_recv() {
                Ok(Ok(())) => write_done = true,
                Ok(Err(error)) => {
                    failure.get_or_insert(error);
                    write_done = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    failure.get_or_insert(CodeError::WorkerUnavailable);
                    write_done = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let exit = process
            .try_wait()
            .map_err(|_| CodeError::WorkerUnavailable)?;
        if let Some(exit) = exit {
            exited_at.get_or_insert_with(Instant::now);
            if !exit.success() {
                failure.get_or_insert(CodeError::WorkerUnavailable);
            }
            // Parent exit and native owner accounting are separate observations.
            // Keep all permits until fully_exited proves the tree and IO are
            // drained, within the existing total and post-exit cleanup budgets.
            // A still-live descendant becomes Protocol at that same cleanup
            // deadline below; it never makes a terminal frame successful.
        }
        if failure.is_some() && killed_at.is_none() {
            process.kill();
            killed_at = Some(Instant::now());
        }
        if process.fully_exited() {
            if let Some(error) = failure {
                return Err(error);
            }
            if eof && ready && write_done {
                #[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
                eprintln!("package code probe: actual Child wait + PID1 owner empty + actual IO join PASS");
                return terminal.ok_or(CodeError::Protocol)?;
            }
        }
        if killed_at.is_some_and(|at| at.elapsed() >= timeouts.cleanup) {
            return Err(CodeError::CleanupTimedOut);
        }
        if failure.is_none() && exited_at.is_some_and(|at| at.elapsed() >= timeouts.cleanup) {
            failure = Some(CodeError::Protocol);
        }
        match output.recv_timeout(Duration::from_millis(20)) {
            Ok(Output::Frame(event)) => {
                let accepted = if terminal.is_some() {
                    Err(CodeError::Protocol)
                } else {
                    match event {
                        Event::Ready { version } if !ready && version == VERSION => {
                            ready = true;
                            #[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
                            eprintln!("package code probe: actual Code Ready version={version} PASS");
                            Ok(())
                        }
                        Event::Completed { codes } if ready => {
                            match code_decode::validate_results(&codes) {
                                Ok(()) => {
                                    terminal = Some(Ok(codes));
                                    Ok(())
                                }
                                Err(error) => Err(error),
                            }
                        }
                        Event::Failed { error } if ready => {
                            terminal = Some(Err(error));
                            Ok(())
                        }
                        _ => Err(CodeError::Protocol),
                    }
                };
                if let Err(error) = accepted {
                    #[cfg(test)]
                    eprintln!("code supervisor frame rejected: error={error:?} ready={ready} terminal={} elapsed={:?}", terminal.is_some(), started.elapsed());
                    failure.get_or_insert(error);
                }
            }
            Ok(Output::Eof) => {
                eof = true;
                if terminal.is_none() {
                    #[cfg(test)]
                    eprintln!("code supervisor protocol: EOF without terminal, ready={ready}, elapsed={:?}", started.elapsed());
                    failure.get_or_insert(CodeError::Protocol);
                }
            }
            Ok(Output::Error) => {
                #[cfg(test)]
                eprintln!("code supervisor protocol: malformed frame, ready={ready} terminal={} elapsed={:?}", terminal.is_some(), started.elapsed());
                failure.get_or_insert(CodeError::Protocol);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !eof {
                    failure.get_or_insert(CodeError::Protocol);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-worker-tests"))]
pub(crate) fn linux_package_probe_main() -> i32 {
    // The CI-owned known QR comes through the normal bounded worker protocol.
    // This feature-only process runs before storage, content server and Tauri.
    if std::env::args_os().count() != 2 || std::env::var_os("APPDIR").is_none() {
        return 2;
    }
    let (image, mode) = match read_scan(&mut BufReader::new(std::io::stdin())) {
        Ok((image, mode @ ScanMode::Codes)) => (image, mode),
        _ => return 2,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let permit = match Permit::acquire(cancel.clone()) {
        Ok(permit) => permit,
        Err(_) => return 2,
    };
    let result = supervise(image, mode, cancel, permit);
    match &result {
        Ok(codes) if codes.len() == 1
            && codes[0].format == code_decode::CodeFormat::QrCode
            && codes[0].text == "MEWU CODE ONLY"
            && !busy() => {
            println!("{{\"schemaVersion\":1,\"kind\":\"feature_only_appdir_code_probe\",\"knownQrVerified\":true,\"actualReadyVerified\":true,\"actualChildTreeIoDrained\":true,\"permitReleased\":true,\"productionAdmitted\":false,\"guiVerified\":false}}");
            0
        }
        _ => {
            eprintln!("package code probe failed: result={result:?} slot_busy={}", busy());
            2
        }
    }
}

fn read_scan(reader: &mut impl BufRead) -> Result<(CodeImage, ScanMode), CodeError> {
    let header: InputHeader =
        serde_json::from_slice(&read_line(reader, MAX_HEADER)?.ok_or(CodeError::Protocol)?)
            .map_err(|_| CodeError::Protocol)?;
    let length = code_decode::pixel_count(header.width, header.height)?;
    if header.version != VERSION || header.length != length {
        return Err(CodeError::Protocol);
    }
    let mut luma = vec![0; length];
    reader
        .read_exact(&mut luma)
        .map_err(|_| CodeError::Protocol)?;
    let mut extra = [0u8; 1];
    if reader.read(&mut extra).map_err(|_| CodeError::Protocol)? != 0 {
        return Err(CodeError::Protocol);
    }
    Ok((
        CodeImage {
            width: header.width,
            height: header.height,
            luma,
        },
        header.mode,
    ))
}
#[cfg(test)]
fn read_image(reader: &mut impl BufRead) -> Result<CodeImage, CodeError> {
    read_scan(reader).map(|(image, _)| image)
}
/// main first dispatches exact --code-worker before opening DB/server/Tauri.
pub fn worker_main() -> i32 {
    let (image, mode) = match read_scan(&mut BufReader::new(std::io::stdin())) {
        Ok(image) => image,
        Err(_) => return 2,
    };
    if write_event(&Event::Ready { version: VERSION }).is_err() {
        return 2;
    }
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_scan(image, mode)))
            .unwrap_or(Err(CodeError::DecodeFailed));
    let event = match result {
        Ok(codes) => Event::Completed { codes },
        Err(error) => Event::Failed { error },
    };
    if write_event(&event).is_ok() {
        0
    } else {
        2
    }
}

fn decode_scan(image: CodeImage, mode: ScanMode) -> Result<Vec<DecodedCode>, CodeError> {
    image.validate()?;
    // OCR is performed in this existing bounded, killable worker. It never
    // reads accessibility text, credentials, files, the desktop or the network.
    let text_pixels = if mode.has_text() {
        let mut pixels = Vec::with_capacity(image.luma.len() * 4);
        for &gray in &image.luma {
            pixels.extend_from_slice(&[gray, gray, gray, 255]);
        }
        Some(
            image::RgbaImage::from_raw(image.width, image.height, pixels)
                .ok_or(CodeError::InputLimit)?,
        )
    } else {
        None
    };
    let mut values = if mode.has_codes() {
        code_decode::decode(image)?
    } else {
        vec![]
    };
    if let Some(pixels) = text_pixels {
        if let Ok(document) = crate::ocr::recognize(pixels, Arc::new(AtomicBool::new(false))) {
            let text = document
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let entities = crate::screen_entities::recognize(&text);
            // Barcode results retain their existing order and budget. An OCR
            // failure (e.g. missing language pack) cannot suppress valid codes.
            values.extend(
                entities
                    .into_iter()
                    .take(code_decode::MAX_CODES.saturating_sub(values.len())),
            );
        }
    }
    code_decode::validate_results(&values)?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_deadlines_include_terminal_lifetime_and_exact_boundaries() {
        assert_eq!(START_TIMEOUT, Duration::from_secs(3));
        assert_eq!(TOTAL_TIMEOUT, Duration::from_secs(10));
        assert_eq!(TEXT_TIMEOUT, Duration::from_secs(25));
        assert_eq!(CLEANUP_TIMEOUT, Duration::from_secs(2));
        for mode in [ScanMode::Codes, ScanMode::Text, ScanMode::CodesAndText] {
            let timeouts = Timeouts::for_mode(mode);
            let total = Duration::from_secs(if mode.has_text() { 25 } else { 10 });
            assert_eq!(timeouts.total, total);
            assert_eq!(timeouts.startup, START_TIMEOUT);
            assert_eq!(timeouts.cleanup, CLEANUP_TIMEOUT);
            assert!(!timeouts.expired(false, START_TIMEOUT - Duration::from_nanos(1)));
            assert!(timeouts.expired(false, START_TIMEOUT));
            assert!(!timeouts.expired(true, total - Duration::from_nanos(1)));
            assert!(timeouts.expired(true, total));
            // This predicate has no terminal exemption or new result deadline.
            assert_eq!(timeouts.expired(true, TOTAL_TIMEOUT), !mode.has_text());
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    mod supervised {
        use super::*;

        pub(super) const FIXTURE: &str = "code_worker::tests::supervised_deadline_fixture";
        pub(super) const FIXTURE_MODE: &str = "MEWU_CODE_DEADLINE_TEST_CHILD";

        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct Evidence {
            pub worker: u32,
            pub descendant: Option<u32>,
        }
        pub(super) fn command(mode: &str) -> Command {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", FIXTURE, "--nocapture"])
                .env(FIXTURE_MODE, mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            command
        }
        pub(super) fn evidence(descendant: Option<u32>) {
            // libtest owns stdout. Its diagnostics never enter the strict
            // protocol pipe, which this fixture writes directly to stderr.
            println!(
                "\n{}",
                serde_json::to_string(&Evidence {
                    worker: std::process::id(),
                    descendant,
                })
                .unwrap()
            );
            std::io::stdout().flush().unwrap();
        }
        pub(super) fn event(event: Event) {
            let mut output = std::io::stderr().lock();
            output
                .write_all(&serde_json::to_vec(&event).unwrap())
                .unwrap();
            output.write_all(b"\n").unwrap();
            output.flush().unwrap();
        }
        struct Fixture {
            process: ManagedProcess<Permit>,
            output: mpsc::Receiver<Output>,
            writer: mpsc::Receiver<Result<(), CodeError>>,
            evidence: mpsc::Receiver<Evidence>,
            slot: Slot,
            cancel: Arc<AtomicBool>,
            started: Instant,
            timeouts: Timeouts,
        }
        impl Fixture {
            fn spawn(scenario: &str, mode: ScanMode) -> Self {
                let cancel = Arc::new(AtomicBool::new(false));
                // A real Permit with an isolated slot lets these native tests
                // run alongside other decoders without changing the global slot.
                let slot = Arc::new(Mutex::new(Some(cancel.clone())));
                let permit = Permit {
                    slot: slot.clone(),
                    token: cancel.clone(),
                };
                let started = Instant::now();
                let mut process = ManagedProcess::spawn(&mut command(scenario), permit).unwrap();
                let stderr = process.child_mut().stderr.take().unwrap();
                let (output, writer) = attach_io_from(
                    &mut process,
                    CodeImage {
                        width: 1,
                        height: 1,
                        luma: vec![255],
                    },
                    mode,
                    stderr,
                )
                .unwrap();
                let stdout = process.child_mut().stdout.take().unwrap();
                let (tx, evidence) = mpsc::channel();
                process.track_io(std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(value) = serde_json::from_str::<Evidence>(&line) {
                            let _ = tx.send(value);
                        }
                    }
                }));
                let mut timeouts = Timeouts::for_mode(mode);
                // Only the total budget is scaled. Production mode selection,
                // the entire supervisor loop, native ownership and IO are used.
                timeouts.total /= 10;
                Self {
                    process,
                    output,
                    writer,
                    evidence,
                    slot,
                    cancel,
                    started,
                    timeouts,
                }
            }
            fn run(self) -> Result<Vec<DecodedCode>, CodeError> {
                supervise_process(
                    self.process,
                    self.output,
                    self.writer,
                    self.cancel,
                    self.started,
                    self.timeouts,
                )
            }
        }
        fn until(mut done: impl FnMut() -> bool) {
            let started = Instant::now();
            while !done() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "native lifecycle deadline"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        #[cfg(target_os = "macos")]
        fn process_gone(pid: u32) -> bool {
            (unsafe { libc::kill(pid as libc::pid_t, 0) }) == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        }
        #[cfg(target_os = "macos")]
        fn assert_process_gone(pid: u32) {
            assert!(process_gone(pid), "fixture process {pid} still exists");
        }

        #[test]
        fn late_text_terminal_can_finish_after_codes_budget() {
            for mode in [ScanMode::Text, ScanMode::CodesAndText, ScanMode::Codes] {
                let mut fixture = Fixture::spawn("late", mode);
                let evidence = fixture
                    .evidence
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
                assert_eq!(evidence.worker, fixture.process.child_mut().id());
                assert!(fixture.process.try_wait().unwrap().is_none());
                assert!(fixture.slot.lock().unwrap().is_some());
                let slot = fixture.slot.clone();
                let started = fixture.started;
                #[cfg(windows)]
                let job = fixture.process.observe_job_for_test().unwrap();
                let result = fixture.run();
                if mode.has_text() {
                    assert_eq!(result, Ok(Vec::new()));
                    assert!(started.elapsed() > TOTAL_TIMEOUT / 10);
                    assert!(started.elapsed() < TEXT_TIMEOUT / 10);
                } else {
                    assert_eq!(result, Err(CodeError::TimedOut));
                }
                assert!(slot.lock().unwrap().is_none());
                #[cfg(windows)]
                assert_eq!(job.active_processes(), Some(0));
                #[cfg(target_os = "macos")]
                assert_process_gone(evidence.worker);
            }
        }

        #[test]
        fn terminal_without_exit_is_killed_at_original_mode_deadline() {
            for (scenario, mode) in [
                ("completed_hang", ScanMode::Codes),
                ("failed_hang", ScanMode::Text),
            ] {
                let mut fixture = Fixture::spawn(scenario, mode);
                let evidence = fixture
                    .evidence
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
                assert!(fixture.process.try_wait().unwrap().is_none());
                let slot = fixture.slot.clone();
                let started = fixture.started;
                let timeouts = fixture.timeouts;
                #[cfg(windows)]
                let job = fixture.process.observe_job_for_test().unwrap();
                assert_eq!(fixture.run(), Err(CodeError::TimedOut));
                assert!(started.elapsed() >= timeouts.total);
                assert!(started.elapsed() < timeouts.total + timeouts.cleanup);
                assert!(slot.lock().unwrap().is_none());
                #[cfg(windows)]
                assert_eq!(job.active_processes(), Some(0));
                #[cfg(target_os = "macos")]
                assert_process_gone(evidence.worker);
            }
        }

        #[test]
        fn terminal_parent_exit_waits_for_actual_owner_and_rejects_live_descendant() {
            let mut fixture = Fixture::spawn("tree_exit", ScanMode::Text);
            let evidence = fixture.evidence.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(evidence.descendant.is_some());
            assert_eq!(evidence.worker, fixture.process.child_mut().id());
            let slot = fixture.slot.clone();
            let started = fixture.started;
            let timeouts = fixture.timeouts;
            #[cfg(windows)]
            let job = fixture.process.observe_job_for_test().unwrap();
            assert_eq!(fixture.run(), Err(CodeError::Protocol));
            assert!(started.elapsed() >= timeouts.cleanup);
            assert!(started.elapsed() < timeouts.total + timeouts.cleanup);
            assert!(slot.lock().unwrap().is_none());
            #[cfg(windows)]
            assert_eq!(job.active_processes(), Some(0));
            #[cfg(target_os = "macos")]
            {
                assert_process_gone(evidence.worker);
                assert_process_gone(evidence.descendant.unwrap());
            }
        }

        #[test]
        fn cancellation_reaps_live_descendant_and_inherited_pipe_before_slot_release() {
            let mut fixture = Fixture::spawn("tree_terminal", ScanMode::Text);
            let evidence = fixture
                .evidence
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            assert!(evidence.descendant.is_some());
            assert_eq!(evidence.worker, fixture.process.child_mut().id());
            assert!(fixture.process.try_wait().unwrap().is_none());
            assert!(!fixture.process.tree_empty());
            assert!(!fixture.process.fully_exited());
            assert!(fixture.slot.lock().unwrap().is_some());
            #[cfg(windows)]
            let job = fixture.process.observe_job_for_test().unwrap();
            #[cfg(windows)]
            {
                assert_eq!(job.contains_process_for_test(evidence.worker), Some(true));
                assert_eq!(
                    job.contains_process_for_test(evidence.descendant.unwrap()),
                    Some(true)
                );
                assert_eq!(job.contains_process_for_test(0), None);
                assert_eq!(
                    job.contains_process_for_test(unsafe {
                        windows_sys::Win32::System::Threading::GetCurrentProcessId()
                    }),
                    Some(false)
                );
            }
            #[cfg(target_os = "macos")]
            assert_eq!(
                unsafe { libc::kill(evidence.descendant.unwrap() as libc::pid_t, 0) },
                0
            );
            let slot = fixture.slot.clone();
            fixture.cancel.store(true, Ordering::Release);
            assert_eq!(fixture.run(), Err(CodeError::Cancelled));
            assert!(slot.lock().unwrap().is_none());
            #[cfg(windows)]
            assert_eq!(job.active_processes(), Some(0));
            #[cfg(target_os = "macos")]
            {
                assert_process_gone(evidence.worker);
                assert_process_gone(evidence.descendant.unwrap());
            }
        }

        #[test]
        fn cleanup_timeout_retains_permit_until_tracked_io_really_finishes() {
            let mut fixture = Fixture::spawn("tree_terminal", ScanMode::Text);
            let evidence = fixture
                .evidence
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            assert!(evidence.descendant.is_some());
            assert_eq!(evidence.worker, fixture.process.child_mut().id());
            #[cfg(windows)]
            let job = fixture.process.observe_job_for_test().unwrap();
            #[cfg(windows)]
            {
                assert_eq!(job.contains_process_for_test(evidence.worker), Some(true));
                assert_eq!(
                    job.contains_process_for_test(evidence.descendant.unwrap()),
                    Some(true)
                );
            }
            let (tx, rx) = mpsc::channel::<()>();
            let io_done = Arc::new(AtomicBool::new(false));
            let done = io_done.clone();
            fixture.process.track_io(std::thread::spawn(move || {
                let _ = rx.recv();
                done.store(true, Ordering::Release);
            }));
            fixture.timeouts.cleanup = Duration::from_millis(100);
            let slot = fixture.slot.clone();
            fixture.cancel.store(true, Ordering::Release);
            assert_eq!(fixture.run(), Err(CodeError::CleanupTimedOut));
            #[cfg(windows)]
            until(|| job.active_processes() == Some(0));
            #[cfg(target_os = "macos")]
            until(|| process_gone(evidence.worker) && process_gone(evidence.descendant.unwrap()));
            assert!(slot.lock().unwrap().is_some());
            assert!(!io_done.load(Ordering::Acquire));
            // Drop moved native ownership and the real Permit to its reaper;
            // neither a timeout nor the finished supervisor released the slot.
            tx.send(()).unwrap();
            until(|| slot.lock().unwrap().is_none());
            assert!(io_done.load(Ordering::Acquire));
            #[cfg(windows)]
            assert_eq!(job.active_processes(), Some(0));
            #[cfg(target_os = "macos")]
            {
                assert_process_gone(evidence.worker);
                assert_process_gone(evidence.descendant.unwrap());
            }
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn supervised_deadline_fixture() {
        use supervised::{command, event, evidence, Evidence, FIXTURE_MODE};
        let Ok(scenario) = std::env::var(FIXTURE_MODE) else {
            return;
        };
        if scenario == "pipe_descendant" {
            let mut gate = String::new();
            assert_eq!(std::io::stdin().read_line(&mut gate).unwrap(), 3);
            assert_eq!(gate, "go\n");
            evidence(None);
            std::thread::sleep(Duration::from_secs(15));
            std::process::exit(0);
        }
        // The parent installs its Job/group before attach_io sends these bytes.
        read_scan(&mut BufReader::new(std::io::stdin().lock())).unwrap();
        event(Event::Ready { version: VERSION });
        match scenario.as_str() {
            "late" => {
                evidence(None);
                std::thread::sleep(Duration::from_millis(1300));
                event(Event::Completed { codes: Vec::new() });
                std::thread::sleep(Duration::from_millis(100));
                std::process::exit(0);
            }
            "completed_hang" | "failed_hang" => {
                event(if scenario == "completed_hang" {
                    Event::Completed { codes: Vec::new() }
                } else {
                    Event::Failed {
                        error: CodeError::DecodeFailed,
                    }
                });
                evidence(None);
            }
            "tree_terminal" | "tree_exit" => {
                let mut child = command("pipe_descendant")
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(b"go\n").unwrap();
                let stdout = child.stdout.take().unwrap();
                let descendant = BufReader::new(stdout)
                    .lines()
                    .map(Result::unwrap)
                    .find_map(|line| serde_json::from_str::<Evidence>(&line).ok())
                    .expect("real descendant consumed its stdin gate");
                assert_eq!(descendant.worker, child.id());
                event(Event::Completed { codes: Vec::new() });
                evidence(Some(child.id()));
                if scenario == "tree_exit" {
                    std::process::exit(0);
                }
                // Keep both workers alive and the inherited protocol pipe open.
                // The supervisor must kill the native tree, not just accept JSON.
            }
            _ => panic!("unknown deadline fixture scenario"),
        }
        std::thread::sleep(Duration::from_secs(15));
        std::process::exit(0);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit exact built app worker verification; synthetic text/QR only, no clipboard, URL opening, desktop or user storage"]
    fn real_supervised_text_and_barcode_scan_use_only_declared_modes() {
        use crate::code_decode::CodeFormat as F;
        use mewu_core::{Drawing, DrawingKind, DrawingPoint, Region};
        let executable = std::path::PathBuf::from(
            std::env::var_os("MEWU_CODE_SCAN_PROBE_EXECUTABLE").expect("exact candidate path"),
        );
        assert!(executable.is_absolute() && executable.is_file());
        let base = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1800,
            700,
            image::Rgba([255, 255, 255, 255]),
        ));
        let drawings = [
            ("https://example.org", 40.),
            ("user@example.org", 180.),
            ("+8613800138000", 320.),
            ("86 139 0013 8000", 460.),
        ]
        .into_iter()
        .map(|(text, y)| Drawing {
            id: uuid::Uuid::new_v4().to_string(),
            kind: DrawingKind::Text,
            color: "#000000".into(),
            stroke_width: 1.,
            points: vec![DrawingPoint { x: 48., y }],
            text: Some(text.into()),
            font_size: Some(64.),
            origin: None,
            rich: None,
        })
        .collect();
        let mut bitmap = crate::drawing_render::crop_region(
            &base,
            &Region {
                id: "synthetic-entities".into(),
                x: 0.,
                y: 0.,
                width: 1800.,
                height: 700.,
                drawings,
                ..Default::default()
            },
        )
        .unwrap()
        .into_rgba8();
        use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};
        let qr = MultiFormatWriter
            .encode_with_hints(
                "MEWU CODE ONLY",
                &BarcodeFormat::QR_CODE,
                240,
                240,
                &EncodeHints::default(),
            )
            .unwrap();
        for y in 0..240 {
            for x in 0..240 {
                if qr.get(x, y) {
                    bitmap.put_pixel(1450 + x, 30 + y, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        let make = || CodeImage {
            width: bitmap.width(),
            height: bitmap.height(),
            luma: bitmap.pixels().map(|p| p[0]).collect(),
        };
        for mode in [ScanMode::CodesAndText, ScanMode::Text, ScanMode::Codes] {
            let token = Arc::new(AtomicBool::new(false));
            let permit = Permit::acquire(token.clone()).unwrap();
            let values = supervise_executable(make(), mode, token, permit, &executable).unwrap();
            assert!(!busy());
            if mode.has_text() {
                for (format, text) in [
                    (F::Url, "https://example.org"),
                    (F::Email, "user@example.org"),
                    (F::Phone, "+8613800138000"),
                    (F::Phone, "+8613900138000"),
                ] {
                    assert!(
                        values.iter().any(|v| v.format == format && v.text == text),
                        "{mode:?}: {values:?}"
                    );
                }
            } else {
                assert!(values
                    .iter()
                    .all(|v| !matches!(v.format, F::Url | F::Email | F::Phone)));
            }
            assert_eq!(
                values
                    .iter()
                    .any(|v| v.format == F::QrCode && v.text == "MEWU CODE ONLY"),
                mode.has_codes()
            );
        }
    }
    #[tokio::test]
    async fn precancelled_input_never_acquires_a_worker() {
        assert_eq!(
            run(
                CodeImage {
                    width: 1,
                    height: 1,
                    luma: vec![255]
                },
                Arc::new(AtomicBool::new(true))
            )
            .await,
            Err(CodeError::Cancelled)
        );
    }
    #[cfg(windows)]
    #[test]
    fn synthetic_silent_input_child() {
        if std::env::var("MEWU_CODE_SILENT_TEST_CHILD").as_deref() != Ok("1") {
            return;
        }
        // Dedicated same-test EXE never opens a source or decoder. It deliberately
        // does not consume stdin, so the parent tests a genuinely full OS pipe.
        std::thread::sleep(Duration::from_secs(8));
    }
    #[cfg(windows)]
    #[test]
    fn blocked_bulk_writer_is_killed_and_reaped_before_slot_release() {
        use std::os::windows::process::CommandExt;
        let token = Arc::new(AtomicBool::new(false));
        let local_slot = Arc::new(Mutex::new(Some(token.clone())));
        let permit = Permit {
            slot: local_slot.clone(),
            token,
        };
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "code_worker::tests::synthetic_silent_input_child",
                "--nocapture",
            ])
            .env("MEWU_CODE_SILENT_TEST_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        let mut process = ManagedProcess::spawn(&mut command, permit).unwrap();
        let (output, writer) = attach_io(
            &mut process,
            CodeImage {
                width: 2048,
                height: 2048,
                luma: vec![255; code_decode::MAX_PIXELS],
            },
            ScanMode::Codes,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            matches!(writer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "4MiB writer is still blocked on an unread pipe"
        );
        assert!(process.try_wait().unwrap().is_none());
        assert!(local_slot.lock().unwrap().is_some());
        // Dropping the protocol receivers first also releases a reader blocked
        // sending to its bounded queue; OwnedProcess owns both actual threads.
        drop(output);
        drop(writer);
        drop(process);
        let started = Instant::now();
        while local_slot.lock().unwrap().is_some() && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            local_slot.lock().unwrap().is_none(),
            "pipe thread and owned process were actually reaped"
        );
    }
    #[test]
    fn binary_input_is_exact_and_bounded_before_allocating() {
        let good = b"{\"version\":1,\"width\":2,\"height\":1,\"length\":2}\n\x00\xff";
        assert_eq!(read_image(&mut &good[..]).unwrap().luma, vec![0, 255]);
        assert!(read_image(&mut &good[..good.len() - 1]).is_err());
        let mut extra = good.to_vec();
        extra.push(0);
        assert!(read_image(&mut &extra[..]).is_err());
        assert!(read_image(
            &mut &b"{\"version\":1,\"width\":8193,\"height\":1,\"length\":8193}\n"[..]
        )
        .is_err());
        let text = b"{\"version\":1,\"width\":1,\"height\":1,\"length\":1,\"mode\":\"text\"}\n\xff";
        assert_eq!(read_scan(&mut &text[..]).unwrap().1, ScanMode::Text);
        assert!(read_scan(&mut &b"{\"version\":1,\"width\":1,\"height\":1,\"length\":1,\"mode\":\"command\"}\n\xff"[..]).is_err());
    }
    #[test]
    fn event_enums_are_strict_and_partial_json_never_succeeds() {
        assert!(
            serde_json::from_str::<Event>(r#"{"type":"ready","version":1,"path":"x"}"#).is_err()
        );
        assert!(read_line(&mut &b"{}"[..], 100).is_err());
        assert!(read_line(&mut &b"012345\n"[..], 6).is_err());
        assert!(serde_json::from_str::<CodeError>(r#"{"busy":null}"#).is_err());
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    #[default]
    Codes,
    Text,
    CodesAndText,
}
impl ScanMode {
    fn has_codes(self) -> bool {
        self != Self::Text
    }
    fn has_text(self) -> bool {
        self != Self::Codes
    }
}
