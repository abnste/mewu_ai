// SPDX-License-Identifier: MPL-2.0
use crate::{
    endpoint::before_close, ControlError, Endpoint, ErrorCode, ExecutionOutcome, Request, Response,
    MAX_FRAME_BYTES,
};
use interprocess::{
    local_socket::{ConnectOptions, Stream},
    ConnectWaitMode,
};
use serde::Serialize;
use std::{
    io::{self, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub(crate) const TICK: Duration = Duration::from_millis(5);
static NEVER_STOP: AtomicBool = AtomicBool::new(false);

pub(crate) struct Wire(pub(crate) Stream);
impl Wire {
    pub(crate) fn new(stream: Stream) -> io::Result<Self> {
        let wire = Self(stream);
        #[cfg(windows)]
        {
            let Stream::NamedPipe(pipe) = &wire.0;
            // Accept stays nonblocking on the listener's next instance. Each
            // connected handle instead uses real overlapped I/O: Microsoft's
            // legacy PIPE_NOWAIT mode must not be mixed with that mechanism.
            // Reads below still Peek first and request only available bytes.
            pipe.inner().set_nonblocking(false)?;
        }
        Ok(wire)
    }
    pub(crate) fn write_available(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        stop: &AtomicBool,
        sent: &mut bool,
    ) -> Result<usize, FrameError> {
        #[cfg(windows)]
        {
            write_windows(&self.0, bytes, deadline, stop, sent)
        }
        #[cfg(unix)]
        {
            let _ = (deadline, stop, sent);
            self.0.write(bytes).map_err(FrameError::Io)
        }
    }
    fn read_available(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsHandle, AsRawHandle};
            use windows_sys::Win32::System::Pipes::PeekNamedPipe;
            let Stream::NamedPipe(stream) = &self.0;
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    stream.as_handle().as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if available == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let count = bytes.len().min(available as usize);
            self.0.read(&mut bytes[..count])
        }
        #[cfg(unix)]
        {
            self.0.read(bytes)
        }
    }
}

#[cfg(windows)]
fn write_windows(
    stream: &Stream,
    bytes: &[u8],
    deadline: Instant,
    stop: &AtomicBool,
    sent: &mut bool,
) -> Result<usize, FrameError> {
    use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_IO_INCOMPLETE, ERROR_IO_PENDING},
        Storage::FileSystem::WriteFile,
        System::{
            Threading::CreateEventW,
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        },
    };
    let Stream::NamedPipe(pipe) = stream;
    let handle = pipe.as_handle().as_raw_handle();
    let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if event.is_null() {
        return Err(FrameError::Io(io::Error::last_os_error()));
    }
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.as_raw_handle();
    // Interprocess 2.4.6 waits for WriteFileEx's APC with SleepEx(INFINITE),
    // outside our deadline. Its legacy PIPE_NOWAIT setup also completed 16 KiB
    // writes with zero bytes even on an empty real pipe. Wire::new uses PIPE_WAIT
    // on connected handles; this live OVERLAPPED/event lets our own deadline
    // control the pending write without enlarging the kernel buffer.
    let started = unsafe {
        WriteFile(
            handle,
            bytes.as_ptr(),
            bytes.len() as u32,
            std::ptr::null_mut(),
            &mut overlapped,
        )
    };
    if started == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(FrameError::Io(error));
        }
        // A pending native write may already have an external effect. A later
        // timeout cannot safely be reported as NotSent, even before its count.
        *sent = true;
    }
    loop {
        let mut written = 0;
        if unsafe { GetOverlappedResult(handle, &overlapped, &mut written, 0) } != 0 {
            return Ok(written as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_INCOMPLETE as i32) {
            *sent = true;
            return Err(FrameError::Io(error));
        }
        *sent = true;
        if let Err(stopped) = check(deadline, stop).and_then(|_| wait(deadline, stop)) {
            // Cancel only this operation. A canceled/finished OVERLAPPED must
            // be observed before its stack and borrowed buffer can be freed.
            // This is native I/O cancellation, never a peer-dependent flush.
            unsafe {
                let _ = CancelIoEx(handle, &overlapped);
                let mut ignored = 0;
                let _ = GetOverlappedResult(handle, &overlapped, &mut ignored, 1);
            }
            return Err(stopped);
        }
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        before_close(&self.0);
    }
}

/// A sequential, persistent client. Requests are never replayed automatically.
/// On any incomplete exchange the connection is discarded, since its framing
/// and execution outcome can no longer be established safely.
pub struct Client {
    wire: Option<Wire>,
}
impl Client {
    pub fn connect(endpoint: &Endpoint, timeout: Duration) -> Result<Self, ControlError> {
        let deadline = deadline(timeout)?;
        loop {
            let name = endpoint.name().map_err(|_| {
                ControlError::not_sent(ErrorCode::Unavailable, "Local control endpoint unavailable")
            })?;
            let result = ConnectOptions::new()
                .name(name)
                .nonblocking_stream(true)
                .wait_mode(ConnectWaitMode::Timeout(
                    deadline.saturating_duration_since(Instant::now()),
                ))
                .connect_sync();
            match result {
                Ok(stream) => {
                    let wire = Wire::new(stream).map_err(|_| {
                        ControlError::not_sent(ErrorCode::Unavailable, "Local control connection setup failed")
                    })?;
                    // Authenticate the server before a single request byte is sent.
                    endpoint.verify_peer(&wire.0, false).map_err(|_| {
                        ControlError::not_sent(
                            ErrorCode::PermissionDenied,
                            "Local control server identity rejected",
                        )
                    })?;
                    return Ok(Self { wire: Some(wire) });
                }
                Err(error) if connect_pending(&error) => {
                    wait(deadline, &NEVER_STOP)
                        .map_err(|error| error.public(ExecutionOutcome::NotSent))?;
                }
                Err(_) => {
                    return Err(ControlError::not_sent(
                        ErrorCode::Unavailable,
                        "Local control host is unavailable",
                    ))
                }
            }
        }
    }
    /// Use this for application commands containing text or attachments. OS user
    /// identity alone does not prove that the server is the installed Mewu host.
    pub fn connect_verified(
        endpoint: &Endpoint,
        expected_server_executable: &std::path::Path,
        timeout: Duration,
    ) -> Result<Self, ControlError> {
        let client = Self::connect(endpoint, timeout)?;
        let wire = client.wire.as_ref().expect("newly connected wire");
        endpoint
            .verify_server_image(&wire.0, expected_server_executable)
            .map_err(|_| {
                ControlError::not_sent(
                    ErrorCode::PermissionDenied,
                    "Local control server executable rejected",
                )
            })?;
        Ok(client)
    }
    /// Only for the AppImage host's own console entry. Different FUSE mounts
    /// must still run exactly this executable's bytes and the same source image.
    /// This is explicit, never a fallback from ordinary executable verification.
    /// Requires Linux 6.5+ SO_PEERPIDFD; unsupported kernels are rejected before
    /// any request body is sent, without changing ordinary exact-path clients.
    #[cfg(target_os = "linux")]
    pub fn connect_appimage_verified(
        endpoint: &Endpoint,
        timeout: Duration,
    ) -> Result<Self, ControlError> {
        let until = deadline(timeout)?;
        let client = Self::connect(endpoint, until.saturating_duration_since(Instant::now()))?;
        let wire = client.wire.as_ref().expect("newly connected wire");
        endpoint.verify_appimage_server(&wire.0, until).map_err(|error| {
            if error.kind() == io::ErrorKind::TimedOut {
                ControlError::not_sent(ErrorCode::Timeout, "Local AppImage identity check timed out")
            } else if error.kind() == io::ErrorKind::Unsupported {
                ControlError::not_sent(ErrorCode::Unsupported, "AppImage console requires Linux 6.5+ SO_PEERPIDFD support")
            } else {
                ControlError::not_sent(ErrorCode::PermissionDenied, "Local AppImage server executable rejected")
            }
        })?;
        Ok(client)
    }
    pub fn call(&mut self, request: &Request, timeout: Duration) -> Result<Response, ControlError> {
        request.validate()?;
        let bytes = encode(request, MAX_FRAME_BYTES).map_err(|_| {
            ControlError::not_sent(
                ErrorCode::FrameTooLarge,
                "Request exceeds the control frame limit",
            )
        })?;
        let deadline = deadline(timeout)?;
        let mut wire = self.wire.take().ok_or_else(|| {
            ControlError::not_sent(ErrorCode::Unavailable, "Control connection is closed")
        })?;
        let mut sent = false;
        write_frame(&mut wire, &bytes, deadline, &NEVER_STOP, &mut sent).map_err(|error| {
            error.public(if sent {
                ExecutionOutcome::Unknown
            } else {
                ExecutionOutcome::NotSent
            })
        })?;
        let bytes = read_frame(&mut wire, MAX_FRAME_BYTES, deadline, &NEVER_STOP)
            .map_err(|error| error.public(ExecutionOutcome::Unknown))?;
        let response: Response = serde_json::from_slice(&bytes).map_err(|_| {
            ControlError::unknown(
                ErrorCode::InvalidResponse,
                "Invalid control response; execution outcome is unknown",
            )
        })?;
        if response.id != request.id {
            return Err(ControlError::unknown(
                ErrorCode::InvalidResponse,
                "Control response id mismatch; execution outcome is unknown",
            ));
        }
        self.wire = Some(wire);
        Ok(response)
    }
}

fn connect_pending(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::TimedOut
    ) || cfg!(windows) && error.raw_os_error() == Some(231) // ERROR_PIPE_BUSY
}

pub(crate) fn deadline(timeout: Duration) -> Result<Instant, ControlError> {
    if timeout.is_zero() || timeout > Duration::from_secs(3600) {
        return Err(ControlError::not_sent(
            ErrorCode::InvalidRequest,
            "Timeout must be greater than zero and at most one hour",
        ));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| ControlError::not_sent(ErrorCode::InvalidRequest, "Invalid timeout"))
}

#[derive(Debug)]
pub(crate) enum FrameError {
    Io(io::Error),
    TooLarge,
    Stopped,
    Deadline,
}
impl FrameError {
    pub(crate) fn public(&self, outcome: ExecutionOutcome) -> ControlError {
        let (code, text) = match self {
            Self::TooLarge => (ErrorCode::FrameTooLarge, "Control frame exceeds the limit"),
            Self::Stopped => (
                ErrorCode::ShuttingDown,
                "Control transport is shutting down",
            ),
            Self::Deadline => (ErrorCode::Timeout, "Control exchange timed out"),
            Self::Io(error) => {
                let _kind = error.kind();
                (
                    ErrorCode::Disconnected,
                    "Control connection was interrupted",
                )
            }
        };
        ControlError::new(code, text, outcome)
    }
}

pub(crate) fn wait(deadline: Instant, stop: &AtomicBool) -> Result<(), FrameError> {
    if stop.load(Ordering::Acquire) {
        return Err(FrameError::Stopped);
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(FrameError::Deadline);
    }
    thread::sleep(TICK.min(remaining));
    Ok(())
}
fn check(deadline: Instant, stop: &AtomicBool) -> Result<(), FrameError> {
    if stop.load(Ordering::Acquire) {
        return Err(FrameError::Stopped);
    }
    if Instant::now() >= deadline {
        return Err(FrameError::Deadline);
    }
    Ok(())
}
fn read_exact(
    wire: &mut Wire,
    mut bytes: &mut [u8],
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<(), FrameError> {
    while !bytes.is_empty() {
        check(deadline, stop)?;
        match wire.read_available(bytes) {
            Ok(0) => return Err(FrameError::Io(io::ErrorKind::UnexpectedEof.into())),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait(deadline, stop)?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(FrameError::Io(error)),
        }
    }
    Ok(())
}
pub(crate) fn read_frame(
    wire: &mut Wire,
    limit: usize,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    read_exact(wire, &mut header, deadline, stop)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > limit {
        return Err(FrameError::TooLarge);
    }
    let mut bytes = vec![0u8; len];
    read_exact(wire, &mut bytes, deadline, stop)?;
    Ok(bytes)
}
pub(crate) fn write_frame(
    wire: &mut Wire,
    bytes: &[u8],
    deadline: Instant,
    stop: &AtomicBool,
    sent: &mut bool,
) -> Result<(), FrameError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    for part in [&(bytes.len() as u32).to_be_bytes()[..], bytes] {
        let mut rest = part;
        while !rest.is_empty() {
            check(deadline, stop)?;
            // Keep native chunks bounded; every partial/pending write shares
            // the frame's absolute deadline and shutdown signal.
            match wire.write_available(&rest[..rest.len().min(16 * 1024)], deadline, stop, sent) {
                Ok(0) => {
                    #[cfg(windows)]
                    wait(deadline, stop)?;
                    #[cfg(unix)]
                    return Err(FrameError::Io(io::ErrorKind::WriteZero.into()));
                }
                Ok(count) => {
                    *sent = true;
                    rest = &rest[count..];
                }
                Err(FrameError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait(deadline, stop)?
                }
                #[cfg(unix)]
                Err(FrameError::Io(error)) if error.kind() == io::ErrorKind::Interrupted => (),
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

pub(crate) fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, ()> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::with_capacity(1024.min(limit)),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ())?;
    Ok(writer.bytes)
}
