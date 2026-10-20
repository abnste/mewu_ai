// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::transport::{self, Wire};
use interprocess::{local_socket::ConnectOptions, ConnectWaitMode};
use serde_json::json;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(3);
fn request(id: &str) -> Request {
    Request::new(id, "app.status", json!({}))
}
fn until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "local IPC fixture did not reach its expected state"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn raw(endpoint: &Endpoint) -> Wire {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match ConnectOptions::new()
            .name(endpoint.name().unwrap())
            .nonblocking_stream(true)
            .wait_mode(ConnectWaitMode::Timeout(TIMEOUT))
            .connect_sync()
        {
            Ok(stream) => return Wire(stream),
            Err(error) => {
                assert!(Instant::now() < deadline, "raw IPC connect failed: {error}");
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
fn options() -> ServerOptions {
    ServerOptions {
        io_timeout: TIMEOUT,
        ..Default::default()
    }
}

#[test]
fn protocol_preserves_null_and_rejects_ambiguous_or_unknown_envelopes() {
    let encoded = serde_json::to_vec(&Response::success("one", serde_json::Value::Null)).unwrap();
    let response: Response = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(response.result(), Some(&serde_json::Value::Null));
    for source in [
        r#"{"id":"one"}"#,
        r#"{"id":"one","result":{},"error":{"code":"INTERNAL","message":"x","outcome":"unknown","retryable":false}}"#,
        r#"{"id":"one","result":{},"extra":true}"#,
        r#"{"id":"one","id":"two","result":{}}"#,
    ] {
        assert!(
            serde_json::from_str::<Response>(source).is_err(),
            "{source}"
        );
    }
    assert!(serde_json::from_str::<Request>(
        r#"{"id":"one","method":"app.status","params":{},"unknown":1}"#
    )
    .is_err());
    assert!(Request::new("one", "app.status", json!([]))
        .validate()
        .is_err());
}

#[test]
fn actual_local_ipc_verified_server_roundtrips_multiple_calls_and_typed_errors() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            if request.method == "app.unsupported" {
                Response::error(
                    request.id,
                    ControlError::not_sent(ErrorCode::Unsupported, "Unsupported operation"),
                )
            } else {
                Response::success(
                    request.id,
                    json!({"protocolVersion": PROTOCOL_VERSION, "text": "中文\nresult"}),
                )
            }
        }),
        options(),
    )
    .unwrap();
    let mut client =
        Client::connect_verified(&endpoint, &std::env::current_exe().unwrap(), TIMEOUT).unwrap();
    assert_eq!(
        client
            .call(&request("one"), TIMEOUT)
            .unwrap()
            .result()
            .unwrap()["text"],
        "中文\nresult"
    );
    let response = client
        .call(&Request::new("two", "app.unsupported", json!({})), TIMEOUT)
        .unwrap();
    let error = response.error_ref().unwrap();
    assert_eq!(error.outcome, ExecutionOutcome::NotSent);
    assert!(!error.retryable);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    drop(client);
    server.shutdown(TIMEOUT).unwrap();
}

#[test]
fn actual_local_ipc_wrong_server_executable_is_rejected_before_dispatch() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Response::success(request.id, json!({}))
        }),
        options(),
    )
    .unwrap();
    // An existing directory is deliberately not the running server image.
    let wrong = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let error = Client::connect_verified(&endpoint, &wrong, TIMEOUT)
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert_eq!(error.outcome, ExecutionOutcome::NotSent);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.shutdown(TIMEOUT).unwrap();
}

#[test]
fn actual_local_ipc_revoke_rejects_a_previously_authenticated_connection() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Response::success(request.id, json!({}))
        }),
        options(),
    )
    .unwrap();
    let mut client = Client::connect(&endpoint, TIMEOUT).unwrap();
    client.call(&request("before"), TIMEOUT).unwrap();
    server.revoke();
    assert!(!server.is_enabled());
    let response = client.call(&request("after"), TIMEOUT).unwrap();
    assert_eq!(response.error_ref().unwrap().code, ErrorCode::Disabled);
    assert_eq!(
        response.error_ref().unwrap().outcome,
        ExecutionOutcome::NotSent
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(client);
    server.shutdown(TIMEOUT).unwrap();
}

#[test]
fn actual_local_ipc_limits_frames_before_allocating_or_dispatching() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Response::success(request.id, json!({}))
        }),
        options(),
    )
    .unwrap();
    let mut client = Client::connect(&endpoint, TIMEOUT).unwrap();
    let oversized = Request::new(
        "large",
        "app.status",
        json!({"text":"x".repeat(MAX_FRAME_BYTES)}),
    );
    let error = client.call(&oversized, TIMEOUT).unwrap_err();
    assert_eq!(error.code, ErrorCode::FrameTooLarge);
    assert_eq!(error.outcome, ExecutionOutcome::NotSent);
    client.call(&request("valid"), TIMEOUT).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut wire = raw(&endpoint);
    wire.0
        .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
        .unwrap();
    until(|| server.active_clients() == 1); // invalid wire has been closed; valid client remains
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(wire);
    drop(client);
    server.shutdown(TIMEOUT).unwrap();
}

#[test]
fn actual_local_ipc_partial_frame_has_absolute_deadline_and_shutdown_does_not_wait_for_peer() {
    let endpoint = Endpoint::for_test();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(|request| Response::success(request.id, json!({}))),
        ServerOptions {
            io_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap();
    let mut wire = raw(&endpoint);
    until(|| server.active_clients() == 1);
    wire.0.write_all(&[0]).unwrap();
    until(|| server.active_clients() == 0);
    drop(wire);
    let _stalled_peer = raw(&endpoint);
    until(|| server.active_clients() == 1);
    server.shutdown(TIMEOUT).unwrap();
    assert_eq!(server.active_clients(), 0);
}

#[test]
fn actual_local_ipc_timeout_after_dispatch_is_unknown_and_never_replayed() {
    let endpoint = Endpoint::for_test();
    let (entered, observed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_release = release.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            entered.send(()).unwrap();
            let (lock, wake) = &*worker_release;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            Response::success(request.id, json!({"committed":true}))
        }),
        options(),
    )
    .unwrap();
    let endpoint_client = endpoint.clone();
    let client = thread::spawn(move || {
        let mut client = Client::connect(&endpoint_client, TIMEOUT).unwrap();
        client
            .call(&request("effect"), Duration::from_millis(150))
            .unwrap_err()
    });
    observed.recv_timeout(TIMEOUT).unwrap();
    let error = client.join().unwrap();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert_eq!(error.outcome, ExecutionOutcome::Unknown);
    assert!(!error.retryable);
    assert!(server.shutdown(Duration::from_millis(20)).is_err());
    let (lock, wake) = &*release;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    server.shutdown(TIMEOUT).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn actual_local_ipc_rejects_second_host_and_bounds_all_accepted_clients() {
    let endpoint = Endpoint::for_test();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(|request| Response::success(request.id, json!({}))),
        ServerOptions {
            max_clients: 2,
            ..options()
        },
    )
    .unwrap();
    assert!(Server::start(
        endpoint.clone(),
        Arc::new(|request| Response::success(request.id, json!({}))),
        options()
    )
    .is_err());
    let mut peers = Vec::new();
    for _ in 0..8 {
        peers.push(raw(&endpoint));
    }
    until(|| server.active_clients() == 2);
    assert!(server.active_clients() <= 2);
    drop(peers);
    server.shutdown(TIMEOUT).unwrap();
    let mut next = Server::start(
        endpoint,
        Arc::new(|request| Response::success(request.id, json!({}))),
        options(),
    )
    .unwrap();
    next.shutdown(TIMEOUT).unwrap();
}

#[test]
fn frame_deadline_does_not_reset_when_a_peer_drips_bytes() {
    let stop = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_millis(20);
    while Instant::now() < deadline {
        let _ = transport::wait(deadline, &stop);
    }
    assert!(matches!(
        transport::wait(deadline, &stop),
        Err(transport::FrameError::Deadline)
    ));
    assert!(transport::encode(&json!({"large":"x".repeat(1000)}), 100).is_err());
}

#[test]
fn actual_local_ipc_unread_large_response_expires_without_background_drain() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Response::success(request.id, json!({"large": "x".repeat(900_000)}))
        }),
        ServerOptions {
            io_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap();
    let mut wire = raw(&endpoint);
    let bytes = transport::encode(&request("backpressure"), MAX_FRAME_BYTES).unwrap();
    transport::write_frame(
        &mut wire,
        &bytes,
        Instant::now() + TIMEOUT,
        &AtomicBool::new(false),
        &mut false,
    )
    .unwrap();
    until(|| calls.load(Ordering::SeqCst) == 1);
    // Keep the peer alive without consuming its response. Its filled pipe must
    // neither pin a worker indefinitely nor launch a peer-dependent limbo job.
    until(|| server.active_clients() == 0);
    server.shutdown(TIMEOUT).unwrap();
    drop(wire);
}

#[test]
fn actual_local_ipc_partial_body_drips_cannot_renew_the_frame_deadline() {
    let endpoint = Endpoint::for_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut server = Server::start(
        endpoint.clone(),
        Arc::new(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Response::success(request.id, json!({}))
        }),
        ServerOptions {
            io_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap();
    let mut wire = raw(&endpoint);
    until(|| server.active_clients() == 1);
    wire.0.write_all(&2000u32.to_be_bytes()).unwrap();
    let started = Instant::now();
    let mut disconnected = false;
    // One byte every 20ms would keep an inactivity timer alive for a second.
    // The absolute deadline must close this connection during the drips.
    for _ in 0..50 {
        if wire.0.write(&[b' ']).is_err() {
            disconnected = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
        if server.active_clients() == 0 {
            disconnected = true;
            break;
        }
    }
    assert!(
        disconnected,
        "dripping bytes incorrectly extended the absolute frame deadline"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(wire);
    server.shutdown(TIMEOUT).unwrap();
}
