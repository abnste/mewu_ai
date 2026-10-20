// SPDX-License-Identifier: MPL-2.0
use crate::{
    transport::{self, Wire, TICK},
    ControlError, Endpoint, ErrorCode, ExecutionOutcome, Request, Response, MAX_FRAME_BYTES,
};
use interprocess::local_socket::{prelude::*, Listener};
use std::{
    io,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub type Dispatcher = dyn Fn(Request) -> Response + Send + Sync + 'static;

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub max_clients: usize,
    /// Absolute deadline per received frame and per response, never an idle
    /// timeout that a slow peer can refresh one byte at a time.
    pub io_timeout: Duration,
    pub frame_limit: usize,
}
impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            max_clients: 8,
            io_timeout: Duration::from_secs(10),
            frame_limit: MAX_FRAME_BYTES,
        }
    }
}

struct State {
    enabled: AtomicBool,
    stop: AtomicBool,
    clients: AtomicUsize,
}
pub struct Server;
pub struct ServerHandle {
    state: Arc<State>,
    threads: Vec<JoinHandle<()>>,
}
struct Connection {
    wire: Wire,
    state: Arc<State>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.state.clients.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Server {
    pub fn start(
        endpoint: Endpoint,
        dispatcher: Arc<Dispatcher>,
        options: ServerOptions,
    ) -> io::Result<ServerHandle> {
        if !(1..=8).contains(&options.max_clients)
            || !(1024..=MAX_FRAME_BYTES).contains(&options.frame_limit)
            || options.io_timeout.is_zero()
            || options.io_timeout > Duration::from_secs(60)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid local control resource limits",
            ));
        }
        let (listener, endpoint_guard) = endpoint.listen()?;
        let endpoint_guard = Arc::new(endpoint_guard);
        let state = Arc::new(State {
            enabled: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            clients: AtomicUsize::new(0),
        });
        let (send, receive) = mpsc::sync_channel::<Connection>(options.max_clients);
        let receive = Arc::new(Mutex::new(receive));
        let mut handle = ServerHandle {
            state: state.clone(),
            threads: Vec::with_capacity(options.max_clients + 1),
        };
        for index in 0..options.max_clients {
            let (state, receive, endpoint, dispatcher, options, endpoint_guard) = (
                state.clone(),
                receive.clone(),
                endpoint.clone(),
                dispatcher.clone(),
                options.clone(),
                endpoint_guard.clone(),
            );
            let worker = thread::Builder::new()
                .name(format!("mewu-control-{index}"))
                .spawn(move || {
                    let _endpoint_guard = endpoint_guard;
                    worker(receive, endpoint, dispatcher, options, state);
                });
            match worker {
                Ok(worker) => handle.threads.push(worker),
                Err(error) => {
                    handle.state.stop.store(true, Ordering::Release);
                    drop(send);
                    let _ = handle.shutdown(Duration::from_secs(1));
                    return Err(error);
                }
            }
        }
        let state = state.clone();
        let acceptor = thread::Builder::new()
            .name("mewu-control-accept".into())
            .spawn(move || {
                let _endpoint_guard = endpoint_guard;
                accept(listener, send, state, options.max_clients);
            });
        match acceptor {
            Ok(acceptor) => {
                handle.threads.push(acceptor);
                Ok(handle)
            }
            Err(error) => {
                handle.state.stop.store(true, Ordering::Release);
                let _ = handle.shutdown(Duration::from_secs(1));
                Err(error)
            }
        }
    }
}
impl ServerHandle {
    /// Irreversible for this handle. The host must also revoke its exact business
    /// grant so already-admitted work cannot commit after plugin disable.
    pub fn revoke(&self) {
        self.state.enabled.store(false, Ordering::Release);
    }
    pub fn is_enabled(&self) -> bool {
        self.state.enabled.load(Ordering::Acquire) && !self.state.stop.load(Ordering::Acquire)
    }
    pub fn active_clients(&self) -> usize {
        self.state.clients.load(Ordering::Acquire)
    }
    /// Stops transport admission and bounded I/O promptly. A synchronous business
    /// dispatcher cannot be forcibly killed safely: on timeout this handle keeps
    /// ownership of remaining joins and a later shutdown can finish draining.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), ControlError> {
        self.revoke();
        self.state.stop.store(true, Ordering::Release);
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        loop {
            let mut index = 0;
            while index < self.threads.len() {
                if self.threads[index].is_finished() {
                    let thread = self.threads.swap_remove(index);
                    if thread.join().is_err() {
                        return Err(ControlError::unknown(
                            ErrorCode::Internal,
                            "Control worker failed while stopping",
                        ));
                    }
                } else {
                    index += 1;
                }
            }
            if self.threads.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ControlError::unknown(
                    ErrorCode::Timeout,
                    "Control dispatcher has not stopped; keep the host alive and drain again",
                ));
            }
            thread::sleep(TICK.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Drop never waits on user work or a peer. Production exit must call
        // shutdown and honor its result before releasing the host/runtime.
        self.revoke();
        self.state.stop.store(true, Ordering::Release);
    }
}

fn accept(listener: Listener, send: mpsc::SyncSender<Connection>, state: Arc<State>, max: usize) {
    while !state.stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok(stream) => {
                let Ok(wire) = Wire::new(stream) else { continue; };
                if !acquire_client(&state.clients, max) {
                    continue;
                }
                let connection = Connection {
                    wire,
                    state: state.clone(),
                };
                match send.try_send(connection) {
                    Ok(()) => (),
                    Err(TrySendError::Full(_)) => (),
                    Err(TrySendError::Disconnected(_)) => break,
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(TICK)
            }
            Err(_) => {
                state.stop.store(true, Ordering::Release);
                break;
            }
        }
    }
}
fn acquire_client(clients: &AtomicUsize, max: usize) -> bool {
    let mut count = clients.load(Ordering::Acquire);
    while count < max {
        match clients.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(actual) => count = actual,
        }
    }
    false
}
fn worker(
    receive: Arc<Mutex<Receiver<Connection>>>,
    endpoint: Endpoint,
    dispatcher: Arc<Dispatcher>,
    options: ServerOptions,
    state: Arc<State>,
) {
    while !state.stop.load(Ordering::Acquire) {
        let received = match receive.lock() {
            Ok(receive) => receive.recv_timeout(TICK),
            Err(_) => {
                state.stop.store(true, Ordering::Release);
                return;
            }
        };
        match received {
            Ok(mut connection) => serve(
                &mut connection.wire,
                &endpoint,
                &dispatcher,
                &options,
                &state,
            ),
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
fn serve(
    wire: &mut Wire,
    endpoint: &Endpoint,
    dispatcher: &Arc<Dispatcher>,
    options: &ServerOptions,
    state: &State,
) {
    while !state.stop.load(Ordering::Acquire) {
        let deadline = Instant::now() + options.io_timeout;
        let bytes = match transport::read_frame(wire, options.frame_limit, deadline, &state.stop) {
            Ok(bytes) => bytes,
            Err(error) => {
                // Do not send detailed transport/OS errors or a guessed call id.
                if matches!(error, transport::FrameError::TooLarge) {
                    reply(
                        wire,
                        Response::error("", error.public(ExecutionOutcome::NotSent)),
                        options,
                        state,
                    );
                }
                return;
            }
        };
        if endpoint.verify_peer(&wire.0, true).is_err() {
            return;
        }
        let request = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                reply(
                    wire,
                    Response::error(
                        "",
                        ControlError::not_sent(
                            ErrorCode::InvalidRequest,
                            "Invalid control request",
                        ),
                    ),
                    options,
                    state,
                );
                return;
            }
        };
        if let Err(error) = request.validate() {
            reply(wire, Response::error(request.id, error), options, state);
            return;
        }
        let id = request.id.clone();
        // This check controls transport admission only. The host owns the
        // authoritative permission fence at dispatch and at business commit.
        let response = if !state.enabled.load(Ordering::Acquire) {
            Response::error(
                &id,
                ControlError::not_sent(ErrorCode::Disabled, "CLI plugin is disabled"),
            )
        } else if state.stop.load(Ordering::Acquire) {
            return;
        } else {
            match catch_unwind(AssertUnwindSafe(|| dispatcher(request))) {
                Ok(response) if response.id == id => response,
                _ => Response::error(
                    &id,
                    ControlError::unknown(
                        ErrorCode::Internal,
                        "Control dispatcher failed; execution outcome is unknown",
                    ),
                ),
            }
        };
        if !reply(wire, response, options, state) {
            return;
        }
    }
}
fn reply(wire: &mut Wire, response: Response, options: &ServerOptions, state: &State) -> bool {
    let bytes = transport::encode(&response, options.frame_limit).or_else(|_| {
        transport::encode(
            &Response::error(
                response.id,
                ControlError::unknown(
                    ErrorCode::ResultTooLarge,
                    "Control result exceeds the frame limit; do not replay the operation",
                ),
            ),
            options.frame_limit,
        )
    });
    let Ok(bytes) = bytes else {
        return false;
    };
    transport::write_frame(
        wire,
        &bytes,
        Instant::now() + options.io_timeout,
        &state.stop,
        &mut false,
    )
    .is_ok()
}
