// SPDX-License-Identifier: MPL-2.0
//! Supervised local decoder, with no Host/scene/URL/clipboard dependency.
use crate::{
    code_decode::{self, CodeError, CodeImage, DecodedCode},
    owned_process::ManagedProcess,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
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
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputHeader {
    version: u32,
    width: u32,
    height: u32,
    length: usize,
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
    image.validate()?;
    if cancel.load(Ordering::Acquire) {
        return Err(CodeError::Cancelled);
    }
    #[cfg(not(windows))]
    {
        let _ = image;
        return Err(CodeError::Unsupported);
    }
    #[cfg(windows)]
    {
        let permit = Permit::acquire(cancel.clone())?;
        let mut guard = CancelOnDrop(Some(cancel.clone()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("mewu-code-supervisor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    supervise(image, cancel, permit)
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
    let (out_tx, out_rx) = mpsc::sync_channel(4);
    let reader = std::thread::Builder::new()
        .name("mewu-code-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
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
    cancel: Arc<AtomicBool>,
    permit: Permit,
) -> Result<Vec<DecodedCode>, CodeError> {
    let started = Instant::now();
    if cancel.load(Ordering::Acquire) {
        return Err(CodeError::Cancelled);
    }
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| CodeError::WorkerUnavailable)?);
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
    let mut process =
        ManagedProcess::spawn(&mut command, permit).map_err(|_| CodeError::WorkerUnavailable)?;
    let (output, writer) = attach_io(&mut process, image)?;
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
        if failure.is_none()
            && terminal.is_none()
            && ((!ready && started.elapsed() >= START_TIMEOUT)
                || started.elapsed() >= TOTAL_TIMEOUT)
        {
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
            if !process.tree_empty() {
                failure.get_or_insert(CodeError::Protocol);
            }
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
                return terminal.ok_or(CodeError::Protocol)?;
            }
        }
        if killed_at.is_some_and(|at| at.elapsed() >= CLEANUP_TIMEOUT) {
            return Err(CodeError::CleanupTimedOut);
        }
        if failure.is_none() && exited_at.is_some_and(|at| at.elapsed() >= CLEANUP_TIMEOUT) {
            failure = Some(CodeError::Protocol);
        }
        // A malformed child can emit a terminal event and remain alive. Its
        // post-result lifetime is bounded too; receiving JSON alone is not done.
        if terminal.is_some() && started.elapsed() >= TOTAL_TIMEOUT && failure.is_none() {
            failure = Some(CodeError::TimedOut);
        }
        match output.recv_timeout(Duration::from_millis(20)) {
            Ok(Output::Frame(event)) => {
                let accepted = if terminal.is_some() {
                    Err(CodeError::Protocol)
                } else {
                    match event {
                        Event::Ready { version } if !ready && version == VERSION => {
                            ready = true;
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
                    failure.get_or_insert(error);
                }
            }
            Ok(Output::Eof) => {
                eof = true;
                if terminal.is_none() {
                    failure.get_or_insert(CodeError::Protocol);
                }
            }
            Ok(Output::Error) => {
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

fn read_image(reader: &mut impl BufRead) -> Result<CodeImage, CodeError> {
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
    Ok(CodeImage {
        width: header.width,
        height: header.height,
        luma,
    })
}
/// main first dispatches exact --code-worker before opening DB/server/Tauri.
pub fn worker_main() -> i32 {
    let image = match read_image(&mut BufReader::new(std::io::stdin())) {
        Ok(image) => image,
        Err(_) => return 2,
    };
    if write_event(&Event::Ready { version: VERSION }).is_err() {
        return 2;
    }
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| code_decode::decode(image)))
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

#[cfg(test)]
mod tests {
    use super::*;
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
