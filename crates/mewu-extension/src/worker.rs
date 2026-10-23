// SPDX-License-Identifier: MPL-2.0
use crate::{contract::*, ExtensionPackage, ExtensionRuntime, RuntimeLimits};
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::Duration,
};

#[allow(clippy::large_enum_variant)] // Same bounded two-entry protocol queue.
enum Input {
    Request(WorkerRequest),
    Failed,
}
type Output = Arc<dyn Fn(WorkerResponse) -> Result<(), ExtensionError> + Send + Sync>;

fn invocation_limits(limits: &RuntimeLimits, permissions: &[String]) -> RuntimeLimits {
    let mut selected = limits.clone();
    // A model-backed service includes the user's Portal decision and several
    // model/feedback round trips. Bound the whole invocation, rather than
    // giving it the short default intended for local handlers. This grants
    // no capability and never extends an explicit embedding/test deadline.
    let process = permissions.iter().any(|p| p == "process.rpc");
    let model = permissions.iter().any(|p| p == "model.invoke");
    if (process || model)
        && selected.request_timeout == Duration::from_secs(90)
        && selected.max_host_calls == 512
    {
        selected.request_timeout = Duration::from_secs(300);
        if process {
            selected.max_host_calls = 4096;
        }
    }
    selected
}

#[cfg(test)]
mod invocation_budget_tests {
    use super::*;

    #[test]
    fn model_round_trips_get_a_finite_budget_without_more_calls_or_memory() {
        let defaults = RuntimeLimits::default();
        let selected = invocation_limits(&defaults, &["model.invoke".into()]);
        assert_eq!(selected.request_timeout, Duration::from_secs(300));
        assert_eq!(selected.max_host_calls, defaults.max_host_calls);
        assert_eq!(selected.initialization_timeout, defaults.initialization_timeout);
        assert_eq!(selected.memory_bytes, defaults.memory_bytes);
        assert_eq!(selected.stack_bytes, defaults.stack_bytes);
        assert_eq!(invocation_limits(&defaults, &[]).request_timeout, defaults.request_timeout);
        assert_eq!(invocation_limits(&defaults, &["region.input".into()]).request_timeout, defaults.request_timeout);
        let process = invocation_limits(&defaults, &["process.rpc".into(), "model.invoke".into()]);
        assert_eq!(process.request_timeout, Duration::from_secs(300));
        assert_eq!(process.max_host_calls, 4096);
    }

    #[test]
    fn explicit_deadlines_and_call_budgets_are_never_overridden() {
        for timeout in [Duration::from_millis(30), Duration::from_secs(120), Duration::from_secs(300)] {
            let limits = RuntimeLimits { request_timeout: timeout, ..RuntimeLimits::default() };
            let selected = invocation_limits(&limits, &["model.invoke".into(), "process.rpc".into()]);
            assert_eq!(selected.request_timeout, timeout);
            assert_eq!(selected.max_host_calls, limits.max_host_calls);
        }
        let limits = RuntimeLimits { max_host_calls: 20, ..RuntimeLimits::default() };
        let selected = invocation_limits(&limits, &["model.invoke".into()]);
        assert_eq!(selected.max_host_calls, 20);
        assert_eq!(selected.request_timeout, limits.request_timeout);
    }
}

fn read_bounded_line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, ExtensionError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| ExtensionError::Transport)?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(ExtensionError::Protocol("unterminated protocol line"))
            };
        }
        let end = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(available.len());
        if bytes.len() + end > MAX_PROTOCOL_BYTES {
            return Err(ExtensionError::Protocol("protocol line exceeds limit"));
        }
        bytes.extend_from_slice(&available[..end]);
        reader.consume(end);
        if bytes.last() == Some(&b'\n') {
            return Ok(Some(bytes));
        }
    }
}

/// stdio ownership is exclusive. Call this before creating any application/UI
/// state, logging subscriber, single-instance lease, or credential store.
pub fn run_stdio_worker() -> Result<(), ExtensionError> {
    run_worker(
        std::io::stdin(),
        std::io::stdout(),
        RuntimeLimits::default(),
    )
}

/// The reader thread receives cancellation even while QuickJS executes a loop.
/// Responses and host calls share one serialized output sink.
pub fn run_worker<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    limits: RuntimeLimits,
) -> Result<(), ExtensionError> {
    let writer = Arc::new(Mutex::new(writer));
    let output: Output = Arc::new(move |message| {
        let bytes = serde_json::to_vec(&message).map_err(|_| ExtensionError::Transport)?;
        if bytes.len() > MAX_PROTOCOL_BYTES {
            return Err(ExtensionError::Protocol(
                "outbound protocol line exceeds limit",
            ));
        }
        let mut writer = writer.lock().map_err(|_| ExtensionError::Transport)?;
        writer
            .write_all(&bytes)
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|_| ExtensionError::Transport)
    });
    let canceled = Arc::new(AtomicBool::new(false));
    let active_id = Arc::new(Mutex::new(None::<String>));
    let reader_cancel = canceled.clone();
    let reader_active = active_id.clone();
    let (tx, rx) = mpsc::sync_channel(2);
    std::thread::Builder::new()
        .name("mewu-extension-input".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let input = match read_bounded_line(&mut reader) {
                    Ok(Some(bytes)) => match serde_json::from_slice::<WorkerRequest>(&bytes) {
                        Ok(request) => request,
                        Err(_) => {
                            reader_cancel.store(true, Ordering::Release);
                            let _ = tx.send(Input::Failed);
                            break;
                        }
                    },
                    Ok(None) | Err(_) => {
                        reader_cancel.store(true, Ordering::Release);
                        let _ = tx.send(Input::Failed);
                        break;
                    }
                };
                let cancel = match &input {
                    WorkerRequest::Shutdown { .. } => true,
                    WorkerRequest::Cancel { id } => {
                        id.is_none()
                            || reader_active
                                .lock()
                                .map_or(true, |active| active.as_ref() == id.as_ref())
                    }
                    _ => false,
                };
                if cancel {
                    reader_cancel.store(true, Ordering::Release);
                }
                if tx.send(Input::Request(input)).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| ExtensionError::Transport)?;
    let mut runtime = None;
    while let Ok(input) = rx.recv() {
        match input {
            Input::Failed => return Err(ExtensionError::Transport),
            Input::Request(WorkerRequest::Initialize {
                id,
                manifest,
                modules,
            }) => {
                validate_id(&id)?;
                if runtime.is_some() {
                    respond_error(
                        &output,
                        id,
                        ExtensionError::Protocol("worker already initialized"),
                    )?;
                    continue;
                }
                canceled.store(false, Ordering::Release);
                *active_id.lock().map_err(|_| ExtensionError::Transport)? = Some(id.clone());
                let invocation_limits = invocation_limits(&limits, &manifest.permissions);
                let result = ExtensionRuntime::new(
                    ExtensionPackage { manifest, modules },
                    invocation_limits,
                    canceled.clone(),
                    output.clone(),
                );
                let mut created = match result {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        respond_error(&output, id, error)?;
                        return Ok(());
                    }
                };
                let result = drive_initialize(&rx, &mut created, &canceled, &output);
                match result {
                    Ok(registrations) => {
                        output(WorkerResponse::Initialized { id, registrations })?;
                        runtime = Some(created);
                    }
                    Err(error) => {
                        respond_error(&output, id, error)?;
                        return Ok(());
                    }
                }
                *active_id.lock().map_err(|_| ExtensionError::Transport)? = None;
            }
            Input::Request(WorkerRequest::Invoke {
                id,
                service_id,
                input,
                context,
            }) => {
                validate_id(&id)?;
                let Some(runtime) = runtime.as_ref() else {
                    respond_error(
                        &output,
                        id,
                        ExtensionError::Protocol("worker is not initialized"),
                    )?;
                    continue;
                };
                canceled.store(false, Ordering::Release);
                *active_id.lock().map_err(|_| ExtensionError::Transport)? = Some(id.clone());
                let result = runtime
                    .begin_invoke(&service_id, &input, &context)
                    .and_then(|_| {
                        drive(&rx, runtime, &canceled, &output, || runtime.poll_result())
                    });
                match result {
                    Ok(result) => output(WorkerResponse::Result { id, result })?,
                    Err(error) => {
                        respond_error(&output, id, error)?;
                        return Ok(());
                    }
                }
                *active_id.lock().map_err(|_| ExtensionError::Transport)? = None;
            }
            Input::Request(WorkerRequest::Shutdown { id }) => {
                validate_id(&id)?;
                canceled.store(false, Ordering::Release);
                if let Some(runtime) = runtime {
                    let _ = runtime.deactivate();
                }
                output(WorkerResponse::Stopped { id })?;
                return Ok(());
            }
            Input::Request(WorkerRequest::Cancel { id }) => {
                canceled.store(false, Ordering::Release);
                if let Some(id) = id {
                    output(WorkerResponse::Stopped { id })?;
                }
            }
            Input::Request(WorkerRequest::HostResult { .. }) => {
                return Err(ExtensionError::Protocol(
                    "host response without an active request",
                ))
            }
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), ExtensionError> {
    if identifier(id) {
        Ok(())
    } else {
        Err(ExtensionError::Protocol("invalid request ID"))
    }
}
fn respond_error(output: &Output, id: String, error: ExtensionError) -> Result<(), ExtensionError> {
    output(WorkerResponse::Error {
        id,
        error: error.protocol_error(),
    })
}

fn accept(
    input: Input,
    runtime: &ExtensionRuntime,
    canceled: &AtomicBool,
    output: &Output,
) -> Result<(), ExtensionError> {
    match input {
        Input::Request(WorkerRequest::HostResult { id, result, error }) => {
            runtime.host_result(&id, result, error)
        }
        Input::Request(WorkerRequest::Cancel { .. } | WorkerRequest::Shutdown { .. }) => {
            canceled.store(true, Ordering::Release);
            Err(ExtensionError::Canceled)
        }
        Input::Request(WorkerRequest::Invoke { id, .. } | WorkerRequest::Initialize { id, .. }) => {
            respond_error(output, id, ExtensionError::Protocol("worker is busy"))
        }
        Input::Failed => Err(ExtensionError::Transport),
    }
}

fn receive_ready(
    rx: &Receiver<Input>,
    runtime: &ExtensionRuntime,
    canceled: &AtomicBool,
    output: &Output,
) -> Result<(), ExtensionError> {
    loop {
        match rx.try_recv() {
            Ok(input) => accept(input, runtime, canceled, output)?,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Err(ExtensionError::Transport),
        }
    }
}

fn drive<T>(
    rx: &Receiver<Input>,
    runtime: &ExtensionRuntime,
    canceled: &AtomicBool,
    output: &Output,
    mut poll: impl FnMut() -> Result<Option<T>, ExtensionError>,
) -> Result<T, ExtensionError> {
    loop {
        receive_ready(rx, runtime, canceled, output)?;
        if let Some(result) = poll()? {
            return Ok(result);
        }
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(input) => accept(input, runtime, canceled, output)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ExtensionError::Transport),
        }
    }
}

fn drive_initialize(
    rx: &Receiver<Input>,
    runtime: &mut ExtensionRuntime,
    canceled: &AtomicBool,
    output: &Output,
) -> Result<Vec<ServiceRegistration>, ExtensionError> {
    loop {
        receive_ready(rx, runtime, canceled, output)?;
        if let Some(result) = runtime.poll_initialization()? {
            return Ok(result);
        }
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(input) => accept(input, runtime, canceled, output)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ExtensionError::Transport),
        }
    }
}
