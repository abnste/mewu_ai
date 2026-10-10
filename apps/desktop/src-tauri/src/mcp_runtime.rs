// SPDX-License-Identifier: MPL-2.0
//! Local, run-scoped MCP client. Protocol and lifecycle belong to the official
//! SDK; the host bounds bytes, validates schemas, and owns process lifetime.

use jsonschema::{Draft, PatternOptions, Retrieve, Uri, Validator};
use mewu_core::{
    ContentUnavailable, JournalContent, McpCommand, McpTool, NoResponseReason, NotSentReason,
    ObservedToolResult, ToolResponseKind,
};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use rmcp::{
    model::{
        CallToolRequestParams, ClientConfig, ClientRequest, PaginatedRequestParams, ServerResult,
    },
    service::RunningService,
    transport::async_rw::AsyncRwTransport,
    ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceError,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock, Weak,
    },
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    process::Command,
    sync::{OwnedSemaphorePermit, Semaphore},
};

const START_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LINE: usize = 1024 * 1024;
const MAX_STDOUT: usize = 32 * 1024 * 1024;
const MAX_STDERR: usize = 4 * 1024 * 1024;
const MAX_TOOLS: usize = 64;
const MAX_SCHEMA: usize = 16 * 1024;
const MAX_CATALOG: usize = 256 * 1024;
const MAX_RESULT: usize = 128 * 1024;
const MAX_ARGUMENTS: usize = 16 * 1024;
const MAX_PAGES: usize = 8;

pub enum McpCallOutcome {
    NotSent(NotSentReason),
    Responded(ObservedToolResult),
    Unknown(NoResponseReason),
}

fn process_slots() -> &'static Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(4)))
}

#[derive(Default)]
struct Registry {
    epoch: u64,
    quiescing: Option<u64>,
    permanent: bool,
    children: Vec<Weak<ProcessState>>,
}
impl Registry {
    fn admission_epoch(&self) -> Result<u64, String> {
        if self.permanent || self.quiescing.is_some() {
            Err("MCP 已停止".into())
        } else {
            Ok(self.epoch)
        }
    }
    fn admits(&self, epoch: u64) -> bool {
        !self.permanent && self.quiescing.is_none() && self.epoch == epoch
    }
    fn begin(&mut self) -> Result<QuiesceTicket, String> {
        self.admission_epoch()?;
        self.epoch = self.epoch.checked_add(1).ok_or("MCP 状态不可用")?;
        self.quiescing = Some(self.epoch);
        Ok(QuiesceTicket { epoch: self.epoch })
    }
    fn resume(&mut self, ticket: &QuiesceTicket) -> Result<(), String> {
        if self.permanent || self.quiescing != Some(ticket.epoch) {
            return Err("MCP 停止状态已变化".into());
        }
        self.epoch = self.epoch.checked_add(1).ok_or("MCP 状态不可用")?;
        self.quiescing = None;
        Ok(())
    }
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}
/// A nonce for exactly one reversible admission barrier. Cloning it grants no
/// authority to revive an old session or a pre-barrier queued connection.
#[derive(Clone, Debug)]
pub struct QuiesceTicket {
    epoch: u64,
}

pub fn begin_quiesce() -> Result<QuiesceTicket, String> {
    let mut registry = registry().lock().map_err(|_| "MCP 状态不可用")?;
    let ticket = registry.begin()?;
    for process in registry.children.iter().filter_map(Weak::upgrade) {
        process.kill();
    }
    Ok(ticket)
}
pub fn resume_after_failed_quiesce(ticket: &QuiesceTicket) -> Result<(), String> {
    registry()
        .lock()
        .map_err(|_| "MCP 状态不可用")?
        .resume(ticket)
}
fn ticket_current(ticket: &QuiesceTicket) -> bool {
    registry()
        .lock()
        .is_ok_and(|r| !r.permanent && r.quiescing == Some(ticket.epoch))
}
pub async fn drain(ticket: &QuiesceTicket, timeout: Duration) -> Result<(), String> {
    if !ticket_current(ticket) {
        return Err("MCP 停止状态已变化".into());
    }
    if !wait_until_idle(timeout).await {
        return Err("MCP 尚未停止，已取消退出".into());
    }
    if !ticket_current(ticket) {
        return Err("MCP 停止状态已变化".into());
    }
    Ok(())
}
pub fn final_shutdown(ticket: &QuiesceTicket) -> Result<(), String> {
    let mut registry = registry().lock().map_err(|_| "MCP 状态不可用")?;
    if registry.permanent || registry.quiescing != Some(ticket.epoch) {
        return Err("MCP 停止状态已变化".into());
    }
    if process_slots().available_permits() != 4 {
        return Err("MCP 尚未停止，已取消退出".into());
    }
    registry.permanent = true;
    process_slots().close();
    Ok(())
}
/// Legacy irreversible exit hook. New migration/exit paths must use a ticket
/// and a successful real drain before final_shutdown.
pub fn shutdown_all() {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry.permanent = true;
    process_slots().close();
    for process in registry.children.iter().filter_map(Weak::upgrade) {
        process.kill();
    }
}
pub async fn wait_until_idle(timeout: Duration) -> bool {
    let started = tokio::time::Instant::now();
    loop {
        if process_slots().available_permits() == 4 {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

trait PipeStop: Send + Sync {
    fn stop(&self);
}
#[cfg(windows)]
enum Witness {
    Unstarted,
    Unknown,
    Captured(crate::owned_process::ProcessWitness),
}
struct ProcessState {
    child: Mutex<Option<Box<dyn ChildWrapper>>>,
    #[cfg(windows)]
    witness: Mutex<Witness>,
    #[cfg(unix)]
    group: Mutex<Option<i32>>,
    exceeded: AtomicBool,
    stopped: AtomicBool,
    setup_done: AtomicBool,
    reaping: AtomicBool,
    pipes: Mutex<Vec<Arc<dyn PipeStop>>>,
    io: Mutex<Vec<std::thread::JoinHandle<()>>>,
    slot: Mutex<Option<OwnedSemaphorePermit>>,
}
impl ProcessState {
    fn new(slot: OwnedSemaphorePermit) -> Arc<Self> {
        Arc::new(Self {
            child: Mutex::new(None),
            #[cfg(windows)]
            witness: Mutex::new(Witness::Unstarted),
            #[cfg(unix)]
            group: Mutex::new(None),
            exceeded: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            setup_done: AtomicBool::new(false),
            reaping: AtomicBool::new(false),
            pipes: Mutex::new(Vec::new()),
            io: Mutex::new(Vec::new()),
            slot: Mutex::new(Some(slot)),
        })
    }
    fn register_pipe(&self, pipe: Arc<dyn PipeStop>) {
        let mut pipes = self.pipes.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            pipe.stop();
        }
        pipes.push(pipe);
    }
    fn track_io(&self, thread: std::thread::JoinHandle<()>) {
        self.io
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(thread);
    }
    fn signal_kill(&self) {
        self.stopped.store(true, Ordering::Release);
        for pipe in self.pipes.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            pipe.stop();
        }
        #[cfg(windows)]
        if let Witness::Captured(witness) = &*self.witness.lock().unwrap_or_else(|e| e.into_inner())
        {
            witness.kill();
        }
        #[cfg(unix)]
        if let Some(group) = *self.group.lock().unwrap_or_else(|e| e.into_inner()) {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            unsafe {
                kill(-group, 9);
            }
        }
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = child.start_kill();
        }
    }
    fn kill(self: &Arc<Self>) {
        self.signal_kill();
        if self.reaping.swap(true, Ordering::AcqRel) {
            return;
        }
        // Retain the whole owner if thread creation fails; releasing the permit
        // on this failure would falsely certify migration-safe cleanup.
        let held = Arc::new(Mutex::new(Some(self.clone())));
        let worker = held.clone();
        if std::thread::Builder::new()
            .name("mewu-mcp-reaper".into())
            .spawn(move || {
                let state = worker
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap();
                while !state.fully_exited() {
                    state.signal_kill();
                    std::thread::sleep(Duration::from_millis(20));
                }
                // At this point parent/Job and all real OS pipe threads are finished.
                state.child.lock().unwrap_or_else(|e| e.into_inner()).take();
                state.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            })
            .is_err()
        {
            std::mem::forget(held);
        }
    }
    fn fully_exited(&self) -> bool {
        if !self.setup_done.load(Ordering::Acquire) {
            return false;
        }
        #[cfg(windows)]
        let process_done = match &*self.witness.lock().unwrap_or_else(|e| e.into_inner()) {
            Witness::Unstarted => true,
            Witness::Unknown => false,
            Witness::Captured(witness) => witness.fully_exited(),
        };
        #[cfg(unix)]
        let process_done = {
            let group = *self.group.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(group) = group {
                // Parent-only cached try_wait never proves group emptiness.
                unsafe extern "C" {
                    fn kill(pid: i32, signal: i32) -> i32;
                }
                let empty = unsafe { kill(-group, 0) } == -1
                    && io::Error::last_os_error().raw_os_error() == Some(3);
                let parent = self
                    .child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                    .is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))));
                empty && parent
            } else {
                true
            }
        };
        #[cfg(not(any(windows, unix)))]
        let process_done = false;
        if !process_done {
            return false;
        }
        let mut threads = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let mut index = 0;
        while index < threads.len() {
            if threads[index].is_finished() {
                let _ = threads.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        threads.is_empty()
    }
    fn limit_exceeded(self: &Arc<Self>) -> io::Error {
        self.exceeded.store(true, Ordering::Release);
        self.kill();
        io::Error::new(io::ErrorKind::InvalidData, "MCP transport limit exceeded")
    }
    fn check(&self) -> Result<(), String> {
        if self.exceeded.load(Ordering::Acquire) {
            Err("MCP 输出超过上限，进程已停止".into())
        } else if self.stopped.load(Ordering::Acquire) {
            Err("MCP 已停止".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
struct ObserveSuspended(Arc<ProcessState>);
#[cfg(windows)]
impl std::fmt::Debug for ObserveSuspended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObserveSuspended")
    }
}
#[cfg(windows)]
impl process_wrap::tokio::CommandWrapper for ObserveSuspended {
    fn post_spawn(
        &mut self,
        _: &mut Command,
        child: &mut tokio::process::Child,
        _: &CommandWrap,
    ) -> io::Result<()> {
        // process-wrap 10.0.1 executes all post_spawn hooks before JobObject's
        // wrap_child attaches its Job and resumes its temporarily suspended child.
        let mut slot = self.0.witness.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Witness::Unknown;
        let handle = ChildWrapper::process_handle(child)
            .ok_or_else(|| io::Error::other("MCP process handle unavailable"))?;
        *slot = Witness::Captured(crate::owned_process::ProcessWitness::capture(handle)?);
        if let Witness::Captured(witness) = &mut *slot {
            witness.attach_job()?;
        }
        Ok(())
    }
}
#[cfg(unix)]
struct ObserveGroup(Arc<ProcessState>);
#[cfg(unix)]
impl std::fmt::Debug for ObserveGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObserveGroup")
    }
}
#[cfg(unix)]
impl process_wrap::tokio::CommandWrapper for ObserveGroup {
    fn post_spawn(
        &mut self,
        _: &mut Command,
        child: &mut tokio::process::Child,
        _: &CommandWrap,
    ) -> io::Result<()> {
        let group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .ok_or_else(|| io::Error::other("MCP process id unavailable"))?;
        *self.0.group.lock().unwrap_or_else(|e| e.into_inner()) = Some(group);
        Ok(())
    }
}
struct ProcessGuard {
    state: Arc<ProcessState>,
}
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.state.setup_done.store(true, Ordering::Release);
        self.state.kill();
    }
}

// The SDK sees only memory queues. Every OS File is owned by a tracked real
// thread, so neither SDK Drop nor Tokio's hidden Blocking<ArcFile> can detach IO.
const PIPE_CHUNK: usize = 8192;
const PIPE_QUEUE: usize = 4;
struct ReadState {
    chunks: VecDeque<Vec<u8>>,
    offset: usize,
    eof: bool,
    stopped: bool,
    error: Option<io::ErrorKind>,
    waker: Option<Waker>,
}
struct ReadQueue {
    inner: Mutex<ReadState>,
    space: Condvar,
}
impl ReadQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ReadState {
                chunks: VecDeque::new(),
                offset: 0,
                eof: false,
                stopped: false,
                error: None,
                waker: None,
            }),
            space: Condvar::new(),
        })
    }
    fn finish(&self, error: Option<io::ErrorKind>) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.eof = true;
        state.error = error;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}
impl PipeStop for ReadQueue {
    fn stop(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        state.chunks.clear();
        state.offset = 0;
        self.space.notify_all();
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}
struct PipeReader {
    queue: Arc<ReadQueue>,
}
impl AsyncRead for PipeReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut state = self.queue.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if let Some(chunk) = state.chunks.front() {
            let n = (chunk.len() - state.offset).min(buf.remaining());
            buf.put_slice(&chunk[state.offset..state.offset + n]);
            state.offset += n;
            if state.offset == state.chunks.front().unwrap().len() {
                state.chunks.pop_front();
                state.offset = 0;
                self.queue.space.notify_one();
            }
            return Poll::Ready(Ok(()));
        }
        if let Some(error) = state.error.take() {
            return Poll::Ready(Err(error.into()));
        }
        if state.eof {
            return Poll::Ready(Ok(()));
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
impl Drop for PipeReader {
    fn drop(&mut self) {
        self.queue.stop();
    }
}
enum WriteCommand {
    Bytes(Vec<u8>),
    Flush,
}
struct WriteState {
    command: Option<WriteCommand>,
    result: Option<Result<(), io::ErrorKind>>,
    stopped: bool,
    waker: Option<Waker>,
}
struct WriteQueue {
    inner: Mutex<WriteState>,
    work: Condvar,
}
impl WriteQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(WriteState {
                command: None,
                result: None,
                stopped: false,
                waker: None,
            }),
            work: Condvar::new(),
        })
    }
}
impl PipeStop for WriteQueue {
    fn stop(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        state.command.take();
        self.work.notify_all();
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}
struct PipeWriter {
    queue: Arc<WriteQueue>,
    pending: bool,
    pending_flush: bool,
    needs_flush: bool,
}
impl PipeWriter {
    fn complete(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut state = self.queue.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if !self.pending {
            return Poll::Ready(Ok(()));
        }
        if let Some(result) = state.result.take() {
            self.pending = false;
            if self.pending_flush {
                self.pending_flush = false;
                self.needs_flush = false;
            }
            Poll::Ready(result.map_err(Into::into))
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
    fn submit(&mut self, command: WriteCommand, cx: &mut Context<'_>) -> io::Result<()> {
        let mut state = self.queue.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        debug_assert!(state.command.is_none() && state.result.is_none());
        state.command = Some(command);
        state.waker = Some(cx.waker().clone());
        self.pending = true;
        self.queue.work.notify_one();
        Ok(())
    }
}
impl AsyncWrite for PipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.complete(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = bytes.len().min(PIPE_CHUNK);
        match this.submit(WriteCommand::Bytes(bytes[..n].to_vec()), cx) {
            Ok(()) => {
                this.needs_flush = true;
                Poll::Ready(Ok(n))
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.complete(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        if !this.needs_flush {
            return Poll::Ready(Ok(()));
        }
        if let Err(e) = this.submit(WriteCommand::Flush, cx) {
            return Poll::Ready(Err(e));
        }
        this.pending_flush = true;
        Poll::Pending
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                self.queue.stop();
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.queue.stop();
    }
}
fn file_read(
    file: &mut std::fs::File,
    buffer: &mut [u8],
    process: &ProcessState,
) -> io::Result<usize> {
    loop {
        if process.stopped.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match file.read(buffer) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            result => return result,
        }
    }
}
fn file_write(file: &mut std::fs::File, bytes: &[u8], process: &ProcessState) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        if process.stopped.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match file.write(&bytes[written..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => written += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn pipe_reader(mut file: std::fs::File, process: &Arc<ProcessState>) -> io::Result<PipeReader> {
    let queue = ReadQueue::new();
    process.register_pipe(queue.clone());
    let worker = queue.clone();
    let state = process.clone();
    let thread = std::thread::Builder::new()
        .name("mewu-mcp-stdout".into())
        .spawn(move || {
            let mut bytes = [0u8; PIPE_CHUNK];
            let mut total = 0usize;
            loop {
                let n = match file_read(&mut file, &mut bytes, &state) {
                    Ok(0) => {
                        worker.finish(None);
                        break;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        worker.finish(Some(e.kind()));
                        break;
                    }
                };
                total = total.saturating_add(n);
                if total > MAX_STDOUT {
                    state.limit_exceeded();
                    break;
                }
                let mut inner = worker.inner.lock().unwrap_or_else(|e| e.into_inner());
                while !inner.stopped && inner.chunks.len() >= PIPE_QUEUE {
                    inner = worker.space.wait(inner).unwrap_or_else(|e| e.into_inner());
                }
                if inner.stopped {
                    break;
                }
                inner.chunks.push_back(bytes[..n].to_vec());
                if let Some(waker) = inner.waker.take() {
                    waker.wake();
                }
            }
        })?;
    process.track_io(thread);
    Ok(PipeReader { queue })
}
fn pipe_writer(mut file: std::fs::File, process: &Arc<ProcessState>) -> io::Result<PipeWriter> {
    let queue = WriteQueue::new();
    process.register_pipe(queue.clone());
    let worker = queue.clone();
    let owner = process.clone();
    let thread = std::thread::Builder::new()
        .name("mewu-mcp-stdin".into())
        .spawn(move || loop {
            let command = {
                let mut inner = worker.inner.lock().unwrap_or_else(|e| e.into_inner());
                while !inner.stopped && inner.command.is_none() {
                    inner = worker.work.wait(inner).unwrap_or_else(|e| e.into_inner());
                }
                if inner.stopped {
                    break;
                }
                inner.command.take().unwrap()
            };
            let result = match command {
                WriteCommand::Bytes(bytes) => file_write(&mut file, &bytes, &owner),
                WriteCommand::Flush => file.flush(),
            }
            .map_err(|e| e.kind());
            let failed = result.is_err();
            let mut inner = worker.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.result = Some(result);
            if failed {
                inner.stopped = true;
            }
            if let Some(waker) = inner.waker.take() {
                waker.wake();
            }
            if failed {
                break;
            }
        })?;
    process.track_io(thread);
    Ok(PipeWriter {
        queue,
        pending: false,
        pending_flush: false,
        needs_flush: false,
    })
}
fn pipe_stderr(mut file: std::fs::File, process: &Arc<ProcessState>) -> io::Result<()> {
    let state = process.clone();
    let thread = std::thread::Builder::new()
        .name("mewu-mcp-stderr".into())
        .spawn(move || {
            let mut bytes = [0u8; PIPE_CHUNK];
            let mut total = 0usize;
            while let Ok(n) = file_read(&mut file, &mut bytes, &state) {
                if n == 0 {
                    break;
                }
                total = total.saturating_add(n);
                if total > MAX_STDERR {
                    state.limit_exceeded();
                    break;
                }
            }
        })?;
    process.track_io(thread);
    Ok(())
}
macro_rules! native_pipe {
    ($name:ident, $kind:ty) => {
        fn $name(pipe: $kind) -> io::Result<std::fs::File> {
            #[cfg(windows)]
            {
                Ok(std::fs::File::from(pipe.into_owned_handle()?))
            }
            #[cfg(unix)]
            {
                Ok(std::fs::File::from(pipe.into_owned_fd()?))
            }
            #[cfg(not(any(windows, unix)))]
            {
                let _ = pipe;
                Err(io::ErrorKind::Unsupported.into())
            }
        }
    };
}
native_pipe!(native_stdin, tokio::process::ChildStdin);
native_pipe!(native_stdout, tokio::process::ChildStdout);
native_pipe!(native_stderr, tokio::process::ChildStderr);

/// Byte/line quotas precede the SDK's read_until and JSON parsing. No JSON-RPC
/// interpretation is duplicated here. Counters survive receive cancellation.
struct BoundedRead<R> {
    inner: R,
    process: Arc<ProcessState>,
    line: usize,
    total: usize,
    lines: usize,
}
impl<R: AsyncRead + Unpin> AsyncRead for BoundedRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let bytes = &buf.filled()[before..];
                this.total = this.total.saturating_add(bytes.len());
                for byte in bytes {
                    if *byte == b'\n' {
                        this.line = 0;
                        this.lines += 1;
                    } else {
                        this.line += 1;
                    }
                    if this.line > MAX_LINE || this.lines > 16_384 || this.total > MAX_STDOUT {
                        return Poll::Ready(Err(this.process.limit_exceeded()));
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

/// Advertise no roots, sampling, elicitation, subscriptions or task support.
/// The SDK's default handler rejects sampling and declines elicitation; roots
/// are empty, never the host's working directory or user profile.
struct RestrictedClient;
impl ClientHandler for RestrictedClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

pub struct McpSession {
    service: RunningService<RoleClient, RestrictedClient>,
    process: ProcessGuard,
    tools: HashMap<String, Validator>,
}

/// Starts only the exact local executable selected by the host. Acquisition,
/// spawn and SDK discovery/legacy fallback share one 15-second deadline.
pub async fn connect(config: &McpCommand) -> Result<McpSession, String> {
    tokio::time::timeout(START_TIMEOUT, async {
        let (executable, cwd) = validate_command(config)?;
        let epoch = registry()
            .lock()
            .map_err(|_| "MCP 状态不可用")?
            .admission_epoch()?;
        let slot = process_slots()
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "MCP 已停止")?;
        let mut command = Command::new(&executable);
        command.args(&config.args).current_dir(cwd).env_clear();
        for name in [
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "HOME",
            "USERPROFILE",
            "LANG",
            "LC_ALL",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        // Fixed program resolution never uses PATH. Descendants receive only the
        // runtime directory and OS binary directories, not the host's custom PATH.
        let mut paths = vec![executable.parent().ok_or("MCP 程序目录无效")?.to_path_buf()];
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            paths.push(PathBuf::from(root).join("System32"));
        }
        #[cfg(unix)]
        paths.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
        command.env(
            "PATH",
            std::env::join_paths(paths).map_err(|_| "MCP 程序目录无效")?,
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let state = ProcessState::new(slot);
        let process = ProcessGuard {
            state: state.clone(),
        };
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        #[cfg(windows)]
        command
            .wrap(process_wrap::tokio::CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ))
            .wrap(ObserveSuspended(state.clone()))
            .wrap(process_wrap::tokio::JobObject);
        #[cfg(unix)]
        command
            .wrap(ObserveGroup(state.clone()))
            .wrap(process_wrap::tokio::ProcessGroup::leader());
        {
            let mut registry = registry().lock().map_err(|_| "MCP 状态不可用")?;
            if !registry.admits(epoch) {
                return Err("MCP 已停止".into());
            }
            registry.children.retain(|entry| entry.strong_count() > 0);
            registry.children.push(Arc::downgrade(&state));
            let child = command
                .spawn()
                .map_err(|_| "无法启动 MCP 程序，请检查路径和运行权限")?;
            *state.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        }
        let (stdin, stdout, stderr) = {
            let mut lock = state.child.lock().unwrap_or_else(|e| e.into_inner());
            let child = lock.as_mut().ok_or("MCP 进程已结束")?;
            (
                child.stdin().take().ok_or("无法连接 MCP 输入")?,
                child.stdout().take().ok_or("无法连接 MCP 输出")?,
                child.stderr().take().ok_or("无法连接 MCP 错误输出")?,
            )
        };
        // These conversions happen before the first Tokio OS pipe poll. No
        // hidden Blocking<ArcFile> operation has been started or detached.
        let stdin = native_stdin(stdin).map_err(|_| "无法连接 MCP 输入")?;
        let stdout = native_stdout(stdout).map_err(|_| "无法连接 MCP 输出")?;
        let stderr = native_stderr(stderr).map_err(|_| "无法连接 MCP 错误输出")?;
        let stdin = pipe_writer(stdin, &state).map_err(|_| "无法连接 MCP 输入")?;
        let stdout = pipe_reader(stdout, &state).map_err(|_| "无法连接 MCP 输出")?;
        pipe_stderr(stderr, &state).map_err(|_| "无法连接 MCP 错误输出")?;
        state.setup_done.store(true, Ordering::Release);
        state.check()?;
        let bounded = BoundedRead {
            inner: stdout,
            process: state.clone(),
            line: 0,
            total: 0,
            lines: 0,
        };
        let transport = AsyncRwTransport::<RoleClient, _, _>::new(bounded, stdin);
        let service = RestrictedClient
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Auto {
                    preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
                    legacy_version: Some(rmcp::model::ProtocolVersion::V_2025_11_25),
                },
            )
            .await
            .map_err(|_| "MCP 连接失败，请检查程序和协议支持")?;
        state.check()?;
        Ok(McpSession {
            service,
            process,
            tools: HashMap::new(),
        })
    })
    .await
    .map_err(|_| "MCP 启动超时")?
}

impl McpSession {
    pub async fn connect(config: &McpCommand) -> Result<Self, String> {
        connect(config).await
    }

    /// A fresh service is created per run. This method deliberately avoids the
    /// SDK's unbounded list_all_tools pagination helper.
    pub async fn catalog(&mut self) -> Result<Vec<McpTool>, String> {
        self.process.state.check()?;
        self.tools.clear();
        let result = tokio::time::timeout(START_TIMEOUT, async {
            let mut tools = Vec::new();
            let mut validators = HashMap::new();
            let mut cursor = None;
            let mut cursors = HashSet::new();
            for _ in 0..MAX_PAGES {
                // send_request bypasses SDK list cache; every explicit refresh
                // must see the server's current list, including changed schemas.
                let mut params = PaginatedRequestParams::default();
                params.cursor = cursor;
                let request = rmcp::model::ListToolsRequest::with_param(params);
                let response = self
                    .service
                    .peer()
                    .send_request(ClientRequest::ListToolsRequest(request))
                    .await
                    .map_err(|_| "无法读取 MCP 工具列表")?;
                let ServerResult::ListToolsResult(page) = response else {
                    return Err("MCP 工具列表格式无效".into());
                };
                for tool in page.tools {
                    let name = tool.name.to_string();
                    let description = tool.description.map(|v| v.to_string()).unwrap_or_default();
                    let input_schema = Value::Object(tool.input_schema.as_ref().clone());
                    if tools.len() >= MAX_TOOLS
                        || name.is_empty()
                        || name.chars().count() > 128
                        || name.chars().any(char::is_control)
                        || description.chars().count() > 4000
                        || validators.contains_key(&name)
                    {
                        return Err("MCP 工具列表无效或超过上限".into());
                    }
                    let validator = validate_schema(&input_schema)?;
                    validators.insert(name.clone(), validator);
                    tools.push(McpTool {
                        name,
                        description,
                        input_schema,
                    });
                    if serde_json::to_vec(&tools)
                        .map_err(|_| "MCP 工具列表格式无效")?
                        .len()
                        > MAX_CATALOG
                    {
                        return Err("MCP 工具列表超过长度上限".into());
                    }
                }
                cursor = page.next_cursor;
                match &cursor {
                    None => {
                        self.tools = validators;
                        return Ok(tools);
                    }
                    Some(value) if value.len() <= 512 && cursors.insert(value.clone()) => {}
                    _ => return Err("MCP 工具列表分页无效".into()),
                }
            }
            Err("MCP 工具列表分页超过上限".into())
        })
        .await
        .map_err(|_| "读取 MCP 工具列表超时".to_string())
        .and_then(|r| r);
        if result.is_err() {
            self.process.state.kill();
        }
        self.process.state.check()?;
        result
    }

    pub async fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        match self.call_recorded(name, arguments, || Ok(())).await {
            McpCallOutcome::Responded(result) => result
                .model_visible_json
                .ok_or_else(|| "MCP 回复无法用于本次处理".to_string())
                .and_then(|json| {
                    serde_json::from_str(&json).map_err(|_| "MCP 回复格式无效".into())
                }),
            McpCallOutcome::NotSent(_) => Err("MCP 工具参数或调用权限无效".into()),
            McpCallOutcome::Unknown(_) => Err("MCP 执行结果未确认，未自动重试".into()),
        }
    }

    pub async fn call_recorded(
        &mut self,
        name: &str,
        arguments: Value,
        before_dispatch: impl FnOnce() -> Result<(), ()>,
    ) -> McpCallOutcome {
        if self.process.state.check().is_err() {
            return McpCallOutcome::NotSent(NotSentReason::PreparationFailed);
        }
        let Some(validator) = self.tools.get(name) else {
            return McpCallOutcome::NotSent(NotSentReason::InvalidArguments);
        };
        let arguments_size = serde_json::to_vec(&arguments).map_or(usize::MAX, |b| b.len());
        if arguments_size > MAX_ARGUMENTS
            || !arguments.is_object()
            || !validator.is_valid(&arguments)
        {
            return McpCallOutcome::NotSent(NotSentReason::InvalidArguments);
        }
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(
            arguments
                .as_object()
                .cloned()
                .expect("argument shape checked above"),
        );
        // This callback persists the host's exact dispatch gate. All local
        // validation precedes it; no connect/catalog/await may follow before
        // the single SDK request. Failure means tools/call is never sent.
        if before_dispatch().is_err() {
            return McpCallOutcome::NotSent(NotSentReason::AuthorityChanged);
        }
        let result = tokio::time::timeout(CALL_TIMEOUT, self.service.call_tool_once(params)).await;
        let observed = match result {
            Ok(Ok(rmcp::model::CallToolResponse::Complete(result))) => observe_complete(result),
            Ok(Ok(rmcp::model::CallToolResponse::InputRequired(result))) => {
                observe_metadata(ToolResponseKind::InputRequired, result)
            }
            Ok(Ok(rmcp::model::CallToolResponse::Task(result))) => {
                observe_metadata(ToolResponseKind::Task, result)
            }
            Ok(Err(ServiceError::McpError(error))) => {
                observe_metadata(ToolResponseKind::RpcError, error)
            }
            Ok(Ok(_)) | Ok(Err(ServiceError::UnexpectedResponse)) => {
                self.process.state.kill();
                return McpCallOutcome::Unknown(NoResponseReason::ProtocolInvalid);
            }
            Ok(Err(ServiceError::Cancelled { .. })) => {
                self.process.state.kill();
                return McpCallOutcome::Unknown(NoResponseReason::Canceled);
            }
            Ok(Err(ServiceError::Timeout { .. })) | Err(_) => {
                self.process.state.kill();
                return McpCallOutcome::Unknown(NoResponseReason::TimedOut);
            }
            Ok(Err(_)) => {
                self.process.state.kill();
                return McpCallOutcome::Unknown(NoResponseReason::TransportLost);
            }
        };
        match observed {
            Ok(mut observed) => {
                // A quota failure cannot erase an SDK reply already received.
                if self.process.state.check().is_err() {
                    observed.model_visible_json = None;
                }
                if observed.model_visible_json.is_none() {
                    self.process.state.kill();
                }
                McpCallOutcome::Responded(observed)
            }
            Err(_) => {
                self.process.state.kill();
                McpCallOutcome::Unknown(NoResponseReason::ProtocolInvalid)
            }
        }
    }

    pub async fn close(mut self) {
        // Cancellation is a signal. The independent reaper retains the actual
        // parent/Job/stdio ownership even if SDK cleanup times out or this future
        // is dropped. No SDK JoinHandle result is used as an OS exit proof.
        self.service.cancellation_token().cancel();
        self.process.state.kill();
        let _ = self
            .service
            .close_with_timeout(Duration::from_millis(500))
            .await;
        let started = tokio::time::Instant::now();
        while self
            .process
            .state
            .slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            if started.elapsed() >= Duration::from_secs(2) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

const OBSERVED_ENCODING: &str = "rmcp-3.5.1/serde-json-value-v1";
struct HashCount {
    hash: Sha256,
    bytes: u64,
}
impl io::Write for HashCount {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("result size overflow"))?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn observation(
    kind: ToolResponseKind,
    raw: &Value,
    returned_error: Option<bool>,
) -> Result<ObservedToolResult, String> {
    let mut sink = HashCount {
        hash: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut sink, raw).map_err(|_| "MCP 工具结果格式无效")?;
    Ok(ObservedToolResult {
        response_kind: kind,
        returned_error,
        observed_encoding: OBSERVED_ENCODING.into(),
        observed_bytes: sink.bytes,
        observed_sha256: format!("{:x}", sink.hash.finalize()),
        retained_content: JournalContent::Omitted {
            reason: ContentUnavailable::UnsupportedContent,
        },
        references: vec![],
        model_visible_json: None,
    })
}
fn observe_metadata(
    kind: ToolResponseKind,
    value: impl serde::Serialize,
) -> Result<ObservedToolResult, String> {
    let raw = serde_json::to_value(value).map_err(|_| "MCP 工具结果格式无效")?;
    observation(kind, &raw, None)
}
fn observe_complete(result: rmcp::model::CallToolResult) -> Result<ObservedToolResult, String> {
    let raw = serde_json::to_value(result).map_err(|_| "MCP 工具结果格式无效")?;
    let mut observed = observation(
        ToolResponseKind::Complete,
        &raw,
        Some(raw.get("isError").and_then(Value::as_bool).unwrap_or(false)),
    )?;
    if observed.observed_bytes > MAX_RESULT as u64 {
        observed.retained_content = JournalContent::Omitted {
            reason: ContentUnavailable::Oversized,
        };
        return Ok(observed);
    }
    let content = raw
        .get("content")
        .and_then(Value::as_array)
        .ok_or("MCP 工具结果缺少内容")?;
    let mut text = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("text")
            || !block.get("text").is_some_and(Value::is_string)
        {
            return Ok(observed);
        }
        text.push(json!({"type":"text","text":block["text"]}));
    }
    let mut output = json!({"content":text,"isError":raw.get("isError").and_then(Value::as_bool).unwrap_or(false)});
    if let Some(structured) = raw.get("structuredContent") {
        output["structuredContent"] = structured.clone();
    }
    // _meta, links, annotations, server instructions and opaque request state
    // never become model instructions, host capabilities, or persisted state.
    let content = serde_json::to_string(&output).map_err(|_| "MCP 工具结果格式无效")?;
    if content.len() > MAX_RESULT {
        observed.retained_content = JournalContent::Omitted {
            reason: ContentUnavailable::Oversized,
        };
    } else {
        observed.retained_content = JournalContent::Json { value: output };
        observed.model_visible_json = Some(content);
    }
    Ok(observed)
}

fn validate_command(config: &McpCommand) -> Result<(PathBuf, PathBuf), String> {
    if config.executable.trim().is_empty()
        || config.executable.chars().count() > 4096
        || config.executable.chars().any(char::is_control)
        || config.cwd.as_ref().is_some_and(|cwd| {
            cwd.trim().is_empty() || cwd.chars().count() > 4096 || cwd.chars().any(char::is_control)
        })
        || config.args.len() > 64
        || config.args.iter().any(|s| s.chars().any(char::is_control))
        || config
            .executable
            .len()
            .saturating_add(config.cwd.as_ref().map_or(0, String::len))
            .saturating_add(config.args.iter().map(String::len).sum::<usize>())
            > 16 * 1024
    {
        return Err("MCP 启动参数无效或过长".into());
    }
    let requested = Path::new(&config.executable);
    if !requested.is_absolute() || !local_path(requested) {
        return Err("MCP 程序必须使用绝对路径".into());
    }
    let executable = requested.canonicalize().map_err(|_| "MCP 程序不存在")?;
    if !executable.is_file() || !local_path(&executable) {
        return Err("请选择本机 MCP 可执行文件".into());
    }
    let extension = executable
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "bat" | "cmd" | "ps1" | "sh") {
        return Err("请指定已安装的运行程序，并将脚本作为独立参数".into());
    }
    let basename = executable
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        basename.as_str(),
        "cmd"
            | "powershell"
            | "pwsh"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "npx"
            | "npm"
            | "yarn"
            | "pnpm"
            | "uvx"
    ) {
        return Err("MCP 需要固定程序，不支持通用命令行或自动安装器".into());
    }
    #[cfg(windows)]
    if extension != "exe" {
        return Err("请选择本机 exe 运行程序".into());
    }
    let cwd = if let Some(cwd) = &config.cwd {
        let requested = Path::new(cwd);
        if !requested.is_absolute() || !local_path(requested) {
            return Err("MCP 工作目录必须使用绝对路径".into());
        }
        requested.canonicalize().map_err(|_| "MCP 工作目录不存在")?
    } else {
        executable.parent().ok_or("MCP 程序目录无效")?.to_path_buf()
    };
    if !cwd.is_dir() || !local_path(&cwd) {
        return Err("MCP 工作目录必须是本机目录".into());
    }
    Ok((executable, cwd))
}

fn local_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        matches!(path.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    }
    #[cfg(not(windows))]
    {
        path.is_absolute()
    }
}

struct NoExternalSchemas;
impl Retrieve for NoExternalSchemas {
    fn retrieve(&self, _: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("External schema retrieval is disabled".into())
    }
}

fn validate_schema(schema: &Value) -> Result<Validator, String> {
    if serde_json::to_vec(schema)
        .map_err(|_| "MCP 工具定义无效")?
        .len()
        > MAX_SCHEMA
    {
        return Err("MCP 工具参数定义超过长度上限".into());
    }
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("MCP 工具参数定义必须是对象".into());
    }
    let mut budget = 4096;
    inspect_schema(schema, schema, 0, &mut budget, &mut HashSet::new())?;
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .with_pattern_options(PatternOptions::regex())
        .with_retriever(NoExternalSchemas)
        .build(schema)
        .map_err(|_| "MCP 工具参数定义无效或包含不支持的约束".into())
}

fn inspect_schema(
    value: &Value,
    root: &Value,
    depth: usize,
    budget: &mut usize,
    active: &mut HashSet<String>,
) -> Result<(), String> {
    if depth > 32 || *budget == 0 {
        return Err("MCP 工具定义过于复杂".into());
    }
    *budget -= 1;
    match value {
        Value::Object(map) => {
            if map.contains_key("$dynamicRef") || map.contains_key("$recursiveRef") {
                return Err("MCP 工具定义暂不支持动态引用".into());
            }
            if let Some(reference) = map.get("$ref") {
                let reference = reference.as_str().ok_or("MCP 工具定义引用无效")?;
                if !reference.starts_with("#/") || !active.insert(reference.to_string()) {
                    return Err("MCP 工具定义不支持外部或循环引用".into());
                }
                let target = root
                    .pointer(&reference[1..])
                    .ok_or("MCP 工具定义引用不存在")?;
                inspect_schema(target, root, depth + 1, budget, active)?;
                active.remove(reference);
            }
            for child in map.values() {
                inspect_schema(child, root, depth + 1, budget, active)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                inspect_schema(child, root, depth + 1, budget, active)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    static FIXTURE_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct Fixture {
        directory: PathBuf,
        command: McpCommand,
    }
    impl Fixture {
        fn new(mode: &str) -> Self {
            let runtime = std::env::var_os("MEWU_TEST_NODE")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("PATH").and_then(|path| {
                        std::env::split_paths(&path)
                            .map(|directory| {
                                directory.join(if cfg!(windows) { "node.exe" } else { "node" })
                            })
                            .find(|path| path.is_file())
                    })
                })
                .expect("MCP fixtures require the project's installed Node runtime");
            let runtime = runtime.canonicalize().unwrap();
            let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .join(".private/remake-review/mcp-fixture-results")
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&directory).unwrap();
            let script =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_server.cjs");
            let command = McpCommand {
                executable: runtime.to_string_lossy().into_owned(),
                args: vec![
                    script.to_string_lossy().into_owned(),
                    directory.to_string_lossy().into_owned(),
                    mode.into(),
                ],
                cwd: Some(directory.to_string_lossy().into_owned()),
            };
            Self { directory, command }
        }
        async fn started(&self) -> Value {
            for _ in 0..300 {
                if let Ok(bytes) = std::fs::read(self.directory.join("started.json")) {
                    if let Ok(value) = serde_json::from_slice(&bytes) {
                        return value;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("fixture failed to start")
        }
        fn requests(&self) -> Vec<Value> {
            std::fs::read_to_string(self.directory.join("requests.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for filename in ["started.json", "requests.jsonl"] {
                let _ = std::fs::remove_file(self.directory.join(filename));
            }
            let _ = std::fs::remove_dir(&self.directory);
        }
    }

    #[cfg(windows)]
    struct ProcessProof(windows_sys::Win32::Foundation::HANDLE);
    #[cfg(windows)]
    impl ProcessProof {
        fn open(pid: u32) -> Self {
            use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(!handle.is_null(), "fixture must be alive before cleanup");
            Self(handle)
        }
        async fn assert_exited(&self) {
            use windows_sys::Win32::{
                Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
                System::Threading::WaitForSingleObject,
            };
            for _ in 0..200 {
                let status = unsafe { WaitForSingleObject(self.0, 0) };
                if status == WAIT_OBJECT_0 {
                    return;
                }
                assert_eq!(status, WAIT_TIMEOUT);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("owned fixture process survived cleanup");
        }
    }
    #[cfg(windows)]
    impl Drop for ProcessProof {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    #[tokio::test]
    async fn modern_and_legacy_lifecycles_keep_results_bounded_and_environment_private() {
        let _serial = FIXTURE_TESTS.lock().await;
        for mode in ["modern", "legacy", "tool-error"] {
            let fixture = Fixture::new(mode);
            let mut session = McpSession::connect(&fixture.command).await.unwrap();
            assert_eq!(session.catalog().await.unwrap()[0].name, "fixture.echo");
            assert!(session
                .call("fixture.echo", json!({"text":3}))
                .await
                .is_err());
            assert!(session
                .call("fixture.echo", json!({"text":"x","extra":true}))
                .await
                .is_err());
            assert!(session.call("unknown", json!({"text":"x"})).await.is_err());
            let result = session
                .call("fixture.echo", json!({"text":"中文🙂"}))
                .await
                .unwrap();
            assert_eq!(result["content"][0]["text"], "中文🙂");
            assert_eq!(result["isError"], mode == "tool-error");
            assert!(result.get("_meta").is_none());
            let names = result["structuredContent"]["envNames"].as_array().unwrap();
            for forbidden in [
                "RUSTUP_HOME",
                "CARGO_HOME",
                "CODEX_HOME",
                "NODE_OPTIONS",
                "OPENAI_API_KEY",
                "DEEPSEEK_API_KEY",
            ] {
                assert!(!names.iter().any(|name| name
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(forbidden))));
            }
            session.close().await;
            let requests = fixture.requests();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r["method"] == "tools/call")
                    .count(),
                1
            );
            assert_eq!(
                requests.iter().any(|r| r["method"] == "initialize"),
                mode == "legacy"
            );
            if mode != "legacy" {
                assert!(requests
                    .iter()
                    .find(|r| r["method"] == "tools/call")
                    .unwrap()["params"]
                    .get("_meta")
                    .is_some());
            }
        }
    }

    #[tokio::test]
    async fn catalog_refresh_is_fresh_and_pagination_and_size_limits_fail_closed() {
        let _serial = FIXTURE_TESTS.lock().await;
        let fixture = Fixture::new("changed-catalog");
        let mut session = connect(&fixture.command).await.unwrap();
        let first = session.catalog().await.unwrap();
        let second = session.catalog().await.unwrap();
        assert_ne!(first[0].description, second[0].description);
        session.close().await;
        for mode in [
            "loop-pages",
            "too-many-tools",
            "huge-schema",
            "oversized-line",
            "flood-stderr",
        ] {
            let fixture = Fixture::new(mode);
            let mut session = connect(&fixture.command).await.unwrap();
            assert!(session.catalog().await.is_err(), "mode {mode}");
            assert!(session
                .call("fixture.echo", json!({"text":"x"}))
                .await
                .is_err());
            session.close().await;
        }
    }

    #[tokio::test]
    async fn unsupported_interaction_media_and_oversized_result_do_not_retry() {
        let _serial = FIXTURE_TESTS.lock().await;
        for (mode, expected) in [
            ("input-required", ToolResponseKind::InputRequired),
            ("media", ToolResponseKind::Complete),
            ("huge-result", ToolResponseKind::Complete),
        ] {
            let fixture = Fixture::new(mode);
            let mut session = connect(&fixture.command).await.unwrap();
            session.catalog().await.unwrap();
            let result = session
                .call_recorded("fixture.echo", json!({"text":"x"}), || Ok(()))
                .await;
            let McpCallOutcome::Responded(observed) = result else {
                panic!("{mode} was a fully received SDK reply")
            };
            assert_eq!(observed.response_kind, expected);
            assert!(observed.model_visible_json.is_none());
            assert!(observed.observed_bytes > 0);
            assert_eq!(observed.observed_sha256.len(), 64);
            assert_eq!(observed.observed_encoding, OBSERVED_ENCODING);
            if mode == "huge-result" {
                assert!(observed.observed_bytes > MAX_RESULT as u64);
                assert!(matches!(
                    observed.retained_content,
                    JournalContent::Omitted {
                        reason: ContentUnavailable::Oversized
                    }
                ));
            }
            session.close().await;
            assert_eq!(
                fixture
                    .requests()
                    .iter()
                    .filter(|r| r["method"] == "tools/call")
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn dispatch_gate_runs_after_validation_and_prevents_any_call_on_failure() {
        let _serial = FIXTURE_TESTS.lock().await;
        let fixture = Fixture::new("modern");
        let mut session = connect(&fixture.command).await.unwrap();
        session.catalog().await.unwrap();
        let count = std::sync::atomic::AtomicUsize::new(0);
        let result = session
            .call_recorded("fixture.echo", json!({"text":3}), || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert!(matches!(
            result,
            McpCallOutcome::NotSent(NotSentReason::InvalidArguments)
        ));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let result = session
            .call_recorded("fixture.echo", json!({"text":"未发送"}), || {
                count.fetch_add(1, Ordering::SeqCst);
                Err(())
            })
            .await;
        assert!(matches!(
            result,
            McpCallOutcome::NotSent(NotSentReason::AuthorityChanged)
        ));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture
                .requests()
                .iter()
                .filter(|r| r["method"] == "tools/call")
                .count(),
            0
        );
        let result = session
            .call_recorded("fixture.echo", json!({"text":"已发送"}), || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        let McpCallOutcome::Responded(observed) = result else {
            panic!("complete fixture response expected")
        };
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(observed.response_kind, ToolResponseKind::Complete);
        assert_eq!(observed.returned_error, Some(false));
        let value: Value =
            serde_json::from_str(observed.model_visible_json.as_ref().unwrap()).unwrap();
        assert_eq!(value["content"][0]["text"], "已发送");
        assert!(value.get("_meta").is_none());
        session.close().await;
        assert_eq!(
            fixture
                .requests()
                .iter()
                .filter(|r| r["method"] == "tools/call")
                .count(),
            1
        );
    }

    #[test]
    fn observed_sdk_digest_is_independent_from_retained_projection_and_rpc_error_body() {
        let result: rmcp::model::CallToolResult = serde_json::from_value(json!({
            "content":[{"type":"text","text":"确认","annotations":{"audience":["assistant"]}}],
            "isError":true,"_meta":{"private":"不持久化"}
        }))
        .unwrap();
        let sdk = serde_json::to_value(&result).unwrap();
        let sdk_bytes = serde_json::to_vec(&sdk).unwrap();
        let observed = observe_complete(result).unwrap();
        assert_eq!(observed.observed_bytes, sdk_bytes.len() as u64);
        assert_eq!(
            observed.observed_sha256,
            format!("{:x}", Sha256::digest(&sdk_bytes))
        );
        assert_eq!(observed.returned_error, Some(true));
        let public: Value =
            serde_json::from_str(observed.model_visible_json.as_ref().unwrap()).unwrap();
        assert!(public.get("_meta").is_none());
        assert!(public["content"][0].get("annotations").is_none());
        assert_ne!(
            observed.observed_sha256,
            format!("{:x}", Sha256::digest(serde_json::to_vec(&public).unwrap()))
        );
        let error = observe_metadata(
            ToolResponseKind::RpcError,
            json!({"code":-32602,"message":"private error","data":{"value":"secret"}}),
        )
        .unwrap();
        assert_eq!(error.response_kind, ToolResponseKind::RpcError);
        assert_eq!(error.returned_error, None);
        assert!(error.model_visible_json.is_none());
        assert!(matches!(
            error.retained_content,
            JournalContent::Omitted { .. }
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_drop_and_cancel_during_connect_kill_the_entire_owned_tree() {
        let _serial = FIXTURE_TESTS.lock().await;
        let fixture = Fixture::new("tree");
        let session = connect(&fixture.command).await.unwrap();
        let pids = fixture.started().await;
        let parent = ProcessProof::open(pids["pid"].as_u64().unwrap() as u32);
        let child = ProcessProof::open(pids["children"][0].as_u64().unwrap() as u32);
        drop(session);
        parent.assert_exited().await;
        child.assert_exited().await;

        let fixture = Fixture::new("hang-init");
        let mut pending = Box::pin(connect(&fixture.command));
        let pids = tokio::select! {
            _ = &mut pending => panic!("fixture should still be waiting on initialization"),
            pids = fixture.started() => pids,
        };
        let parent = ProcessProof::open(pids["pid"].as_u64().unwrap() as u32);
        let child = ProcessProof::open(pids["children"][0].as_u64().unwrap() as u32);
        drop(pending);
        parent.assert_exited().await;
        child.assert_exited().await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn canceled_run_with_inflight_call_stops_its_process_and_sends_no_second_call() {
        let _serial = FIXTURE_TESTS.lock().await;
        let fixture = Fixture::new("hang-call");
        let mut session = connect(&fixture.command).await.unwrap();
        session.catalog().await.unwrap();
        let pids = fixture.started().await;
        let parent = ProcessProof::open(pids["pid"].as_u64().unwrap() as u32);
        let mut run =
            Box::pin(async move { session.call("fixture.echo", json!({"text":"x"})).await });
        tokio::select! {
            _ = &mut run => panic!("fixture should still be waiting"),
            _ = async {
                for _ in 0..200 {
                    if fixture.requests().iter().any(|r| r["method"] == "tools/call") { return; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                panic!("tool request was not sent");
            } => {},
        }
        drop(run);
        parent.assert_exited().await;
        assert_eq!(
            fixture
                .requests()
                .iter()
                .filter(|r| r["method"] == "tools/call")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn process_concurrency_limit_is_global_and_waiter_can_be_canceled() {
        let _serial = FIXTURE_TESTS.lock().await;
        let mut sessions = Vec::new();
        let fixtures: Vec<_> = (0..5).map(|_| Fixture::new("modern")).collect();
        for fixture in &fixtures[..4] {
            sessions.push(connect(&fixture.command).await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(80), connect(&fixtures[4].command))
                .await
                .is_err()
        );
        assert!(!fixtures[4].directory.join("started.json").exists());
        sessions.pop().unwrap().close().await;
        let fifth = connect(&fixtures[4].command).await.unwrap();
        fifth.close().await;
        for session in sessions {
            session.close().await;
        }
    }

    #[test]
    fn quiesce_epoch_does_not_revive_waiters_or_accept_stale_resume() {
        let mut registry = Registry::default();
        let original = registry.admission_epoch().unwrap();
        let first = registry.begin().unwrap();
        assert!(!registry.admits(original));
        assert!(registry.admission_epoch().is_err());
        assert!(registry.begin().is_err());
        registry.resume(&first).unwrap();
        assert!(!registry.admits(original));
        let fresh = registry.admission_epoch().unwrap();
        assert!(registry.admits(fresh));
        let second = registry.begin().unwrap();
        assert!(registry.resume(&first).is_err());
        assert!(!registry.admits(fresh));
        registry.resume(&second).unwrap();
        registry.permanent = true;
        assert!(registry.begin().is_err());
        assert!(registry.resume(&second).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn quiesce_waits_for_real_io_and_rejects_old_epoch_queued_connection() {
        let _serial = FIXTURE_TESTS.lock().await;
        assert!(wait_until_idle(Duration::from_secs(3)).await);
        let fixtures: Vec<_> = (0..5).map(|_| Fixture::new("tree")).collect();
        let mut sessions = Vec::new();
        for fixture in &fixtures[..4] {
            sessions.push(connect(&fixture.command).await.unwrap());
        }
        let state = sessions[0].process.state.clone();
        let (release, held) = std::sync::mpsc::channel();
        state.track_io(std::thread::spawn(move || {
            let _ = held.recv();
        }));
        // Poll to the actual acquire_owned wait and retain that precise future.
        let mut queued = Box::pin(connect(&fixtures[4].command));
        assert!(tokio::time::timeout(Duration::from_millis(40), &mut queued)
            .await
            .is_err());
        let started = fixtures[0].started().await;
        let parent = ProcessProof::open(started["pid"].as_u64().unwrap() as u32);
        let descendants: Vec<_> = started["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pid| ProcessProof::open(pid.as_u64().unwrap() as u32))
            .collect();
        let ticket = begin_quiesce().unwrap();
        assert!(drain(&ticket, Duration::ZERO).await.is_err());
        parent.assert_exited().await;
        for child in &descendants {
            child.assert_exited().await;
        }
        // The kernel process tree has exited, but a tracked owner is still live.
        assert!(!state.fully_exited());
        assert!(!wait_until_idle(Duration::from_millis(40)).await);
        assert!(final_shutdown(&ticket).is_err());
        resume_after_failed_quiesce(&ticket).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), &mut queued)
            .await
            .unwrap()
            .is_err());
        assert!(!fixtures[4].directory.join("started.json").exists());
        assert!(sessions[0].process.state.check().is_err());
        let fresh = connect(&fixtures[4].command).await.unwrap();
        fresh.close().await;
        assert!(drain(&ticket, Duration::ZERO).await.is_err());
        release.send(()).unwrap();
        for session in sessions {
            session.close().await;
        }
        assert!(wait_until_idle(Duration::from_secs(3)).await);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn parent_exit_does_not_certify_nonempty_job_and_unknown_witness_is_not_idle() {
        let _serial = FIXTURE_TESTS.lock().await;
        assert!(wait_until_idle(Duration::from_secs(3)).await);
        let fixture = Fixture::new("parent-only-job-proof");
        // No SDK service owns these pipes: EOF must not turn this Job witness
        // test into a transport-cleanup race. Keep the same suspended attach /
        // resume wrappers, real permit, and three tracked OS File threads.
        let state = ProcessState::new(process_slots().clone().acquire_owned().await.unwrap());
        let process = ProcessGuard {
            state: state.clone(),
        };
        let mut command = Command::new(&fixture.command.executable);
        command
            .args(&fixture.command.args)
            .current_dir(&fixture.directory);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command = CommandWrap::from(command);
        command
            .wrap(KillOnDrop)
            .wrap(process_wrap::tokio::CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ))
            .wrap(ObserveSuspended(state.clone()))
            .wrap(process_wrap::tokio::JobObject);
        *state.child.lock().unwrap() = Some(command.spawn().unwrap());
        let (stdin, stdout, stderr) = {
            let mut child = state.child.lock().unwrap();
            let child = child.as_mut().unwrap();
            (
                child.stdin().take().unwrap(),
                child.stdout().take().unwrap(),
                child.stderr().take().unwrap(),
            )
        };
        let input = pipe_writer(native_stdin(stdin).unwrap(), &state).unwrap();
        let output = pipe_reader(native_stdout(stdout).unwrap(), &state).unwrap();
        pipe_stderr(native_stderr(stderr).unwrap(), &state).unwrap();
        state.setup_done.store(true, Ordering::Release);
        assert_eq!(state.io.lock().unwrap().len(), 3);
        let started = fixture.started().await;
        let parent = ProcessProof::open(started["pid"].as_u64().unwrap() as u32);
        let child = ProcessProof::open(started["children"][0].as_u64().unwrap() as u32);
        {
            let witness = state.witness.lock().unwrap();
            let Witness::Captured(witness) = &*witness else {
                panic!("captured native witness required");
            };
            assert!(witness.active_processes_for_test().unwrap() >= 2);
            witness.terminate_parent_for_test();
        }
        parent.assert_exited().await;
        assert_eq!(
            unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(child.0, 0) },
            windows_sys::Win32::Foundation::WAIT_TIMEOUT
        );
        {
            let witness = state.witness.lock().unwrap();
            let Witness::Captured(witness) = &*witness else {
                panic!("captured native witness required");
            };
            assert!(witness.active_processes_for_test().unwrap() >= 1);
            assert!(!witness.fully_exited());
        }
        assert!(!state.fully_exited());
        assert!(!wait_until_idle(Duration::ZERO).await);
        drop(input);
        drop(output);
        // Even after every physical pipe thread ends, parent-only completion
        // cannot return this permit while the real Job member remains alive.
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if state
                .io
                .lock()
                .unwrap()
                .iter()
                .all(std::thread::JoinHandle::is_finished)
            {
                break;
            }
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!state.fully_exited());
        assert!(!wait_until_idle(Duration::ZERO).await);
        drop(process);
        child.assert_exited().await;
        assert!(wait_until_idle(Duration::from_secs(3)).await);
        assert!(state.io.lock().unwrap().is_empty());
        let unknown = ProcessState::new(process_slots().clone().acquire_owned().await.unwrap());
        *unknown.witness.lock().unwrap() = Witness::Unknown;
        unknown.setup_done.store(true, Ordering::Release);
        assert!(!unknown.fully_exited());
        // No child was created by this fault fixture; restore only the fixture.
        *unknown.witness.lock().unwrap() = Witness::Unstarted;
        unknown.kill();
        assert!(wait_until_idle(Duration::from_secs(3)).await);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn file_bridge_flush_is_physical_and_full_read_queue_cancellation_joins_owner() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _serial = FIXTURE_TESTS.lock().await;
        assert!(wait_until_idle(Duration::from_secs(3)).await);
        let fixture = Fixture::new("modern");
        let state = ProcessState::new(process_slots().clone().acquire_owned().await.unwrap());
        let output = fixture.directory.join("bridge-only.bin");
        let mut writer = pipe_writer(std::fs::File::create(&output).unwrap(), &state).unwrap();
        let expected = "中文🙂\n".repeat(PIPE_CHUNK);
        writer.write_all(expected.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), expected.as_bytes());
        let mut reader = pipe_reader(std::fs::File::open(&output).unwrap(), &state).unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let buffered = reader.queue.inner.lock().unwrap().chunks.len();
            assert!(buffered <= PIPE_QUEUE);
            if buffered == PIPE_QUEUE {
                break;
            }
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut prefix = [0u8; 17];
        reader.read_exact(&mut prefix).await.unwrap();
        assert_eq!(&prefix, &expected.as_bytes()[..17]);
        state.setup_done.store(true, Ordering::Release);
        assert!(!state.fully_exited());
        state.kill();
        assert!(writer.write_all(b"late").await.is_err());
        assert!(reader.read(&mut prefix).await.is_err());
        drop(writer);
        drop(reader);
        assert!(wait_until_idle(Duration::from_secs(3)).await);
        assert!(state.io.lock().unwrap().is_empty());
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn schemas_are_offline_bounded_and_reject_unsafe_or_unsupported_constraints() {
        let schema = json!({"type":"object","properties":{"count":{"$ref":"#/$defs/count"}},"required":["count"],"$defs":{"count":{"type":"integer","minimum":1}}});
        let validator = validate_schema(&schema).unwrap();
        assert!(validator.is_valid(&json!({"count":2})));
        assert!(!validator.is_valid(&json!({"count":0})));
        for schema in [
            json!({"type":"object","$ref":"https://example.invalid/private"}),
            json!({"type":"object","$ref":"file:///private"}),
            json!({"type":"object","$defs":{"x":{"$ref":"#/$defs/x"}},"$ref":"#/$defs/x"}),
            json!({"type":"object","properties":{"x":{"type":"string","pattern":"(?<=secret)value"}}}),
            json!({"type":"object","description":"x".repeat(MAX_SCHEMA)}),
        ] {
            assert!(validate_schema(&schema).is_err());
        }
        assert!(validate_command(&McpCommand {
            executable: "node".into(),
            args: vec![],
            cwd: None
        })
        .is_err());
    }
}
