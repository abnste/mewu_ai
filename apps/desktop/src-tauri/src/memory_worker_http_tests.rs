// SPDX-License-Identifier: MPL-2.0
//! Worker action -> real loopback HTTP -> durable Store receipt. Synthetic data only.
use super::*;
use mewu_core::{
    BindingStatus, EvidenceState, HostNewMemoryBinding, MemoryPolicy, SceneCommand, Store,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    body: Value,
}
struct Fixture {
    endpoint: String,
    replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
fn read_request(mut stream: &TcpStream) -> Request {
    // An accepted Windows socket can inherit the nonblocking listener mode.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    let end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 600 * 1024);
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
    let mut first = headers.lines().next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let path = first.next().unwrap().to_owned();
    let length: usize = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse().unwrap())
        .unwrap_or(0);
    assert!(length <= 512 * 1024);
    while bytes.len() < end + length {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[end..end + length]).unwrap()
    };
    Request { method, path, body }
}
impl Fixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/fixture", listener.local_addr().unwrap());
        let replies = Arc::new(Mutex::new(VecDeque::<(u16, Value)>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (responses, received, stopped) = (replies.clone(), requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&stream);
                        received.lock().unwrap().push(request);
                        let (status, body) = responses
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or((500, json!({"unexpected_request":true})));
                        let bytes = serde_json::to_vec(&body).unwrap();
                        write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",bytes.len()).unwrap();
                        let _ = stream.write_all(&bytes);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            }
        });
        Self {
            endpoint,
            replies,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn respond(&self, replies: impl IntoIterator<Item = (u16, Value)>) {
        self.replies.lock().unwrap().extend(replies);
    }
    fn finish(mut self) -> Vec<Request> {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
        assert!(
            self.replies.lock().unwrap().is_empty(),
            "expected HTTP request was not sent"
        );
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn version() -> Value {
    json!({"api_version":"0.10.3","features":{
        "observations":true,"mcp":false,"worker":false,"bank_config_api":false,
        "bank_llm_health":false,"file_upload_api":false,"document_export_api":false,
        "document_import_api":false,"audit_log":false,"llm_trace":false,"store_document_text":false
    }})
}
fn probe() -> Vec<(u16, Value)> {
    vec![(200, version()), (200, json!({"status":"healthy"}))]
}
fn config(bank: &str, enabled: bool) -> Value {
    json!({"bank_id":bank,"config":{"enable_observations":enabled},"overrides":{}})
}
fn clock() -> u64 {
    now() + 60_000
}
fn store() -> (Store, String) {
    let mut store = Store::open_in_memory().unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    let id = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    (store, id)
}
fn reserve(store: &mut Store, agent: &str, endpoint: &str) -> mewu_core::ReserveBindingReceipt {
    let selection = store
        .memory_provider_status(agent, None, 100)
        .unwrap()
        .selection;
    store
        .reserve_memory_binding(
            selection.revision,
            HostNewMemoryBinding {
                id: Uuid::new_v4().to_string(),
                agent_id: agent.into(),
                endpoint: endpoint.into(),
                credential_id: None,
                policy: MemoryPolicy {
                    recall: true,
                    explicit_retain: true,
                    completed_turn_sync: false,
                },
                plugin_id: "mewu.memory-hindsight".into(),
                contribution_id: "hindsight".into(),
                plugin_revision: 1,
            },
        )
        .unwrap()
}
fn claim(store: &mut Store, operation: &str) -> MemoryDispatch {
    let pending = store
        .pending_memory_work(clock(), 32)
        .unwrap()
        .into_iter()
        .find(|w| w.operation_id == operation)
        .unwrap();
    store
        .claim_memory_work(
            &pending.operation_id,
            pending.revision,
            &pending.grant,
            clock(),
        )
        .unwrap()
        .unwrap()
}
fn record(store: &mut Store, dispatch: MemoryDispatch, observation: Observation) {
    // A valid outcome must be accepted; otherwise dispatch_one would stop the worker.
    store
        .record_memory_work(dispatch.lease, observation, now())
        .unwrap();
}
fn select(store: &mut Store, agent: &str, binding: &str) {
    let view = store.memory_binding(binding).unwrap();
    let selection = store
        .memory_provider_status(agent, None, 100)
        .unwrap()
        .selection;
    store
        .select_memory_provider(
            agent,
            selection.revision,
            Some(binding),
            Some(view.revision),
        )
        .unwrap();
}
fn ready_locally(store: &mut Store, agent: &str, endpoint: &str) -> String {
    let reserved = reserve(store, agent, endpoint);
    let work = claim(store, &reserved.operation_id);
    let observation = Observation::BankReady {
        bank_id: work.route.bank_id.clone(),
        service_version: "0.10.3".into(),
        observations_enabled: false,
    };
    record(store, work, observation);
    select(store, agent, &reserved.binding.id);
    reserved.binding.id
}
fn retain(
    store: &mut Store,
    agent: &str,
    binding: &str,
) -> (mewu_core::WriteReceipt, MemoryDispatch) {
    let b = store.memory_binding(binding).unwrap();
    let receipt = store
        .queue_manual_memory_evidence(
            agent,
            binding,
            b.revision,
            &Uuid::new_v4().to_string(),
            "Synthetic user-authorized memory.",
        )
        .unwrap();
    let work = claim(store, receipt.operation_id.as_deref().unwrap());
    (receipt, work)
}
async fn perform(dispatch: &MemoryDispatch, authorized: impl Fn() -> bool) -> Observation {
    let (_sender, mut cancel) = oneshot::channel();
    let mut control = http::Control {
        deadline: tokio::time::Instant::now() + Duration::from_secs(5),
        cancel: &mut cancel,
    };
    execute(dispatch, None, &mut control, authorized).await
}

#[tokio::test(flavor = "current_thread")]
async fn create_read_failure_records_no_dispatch_and_does_not_poison_other_work() {
    let f = Fixture::new();
    f.respond(probe());
    f.respond([(503, json!({}))]);
    let (mut store, agent) = store();
    let reserved = reserve(&mut store, &agent, &f.endpoint);
    let work = claim(&mut store, &reserved.operation_id);
    let result = perform(&work, || true).await;
    assert!(matches!(result, Observation::NotDispatched { .. }));
    record(&mut store, work, result);
    // One provisioning bank per Agent is intentional. A new explicit setup
    // retires the failed setup; it must not retry that original operation.
    let failed = store.memory_binding(&reserved.binding.id).unwrap();
    store
        .retire_memory_binding(&failed.id, failed.revision)
        .unwrap();
    let next = reserve(&mut store, &agent, &f.endpoint);
    assert_ne!(next.binding.id, reserved.binding.id);
    assert_ne!(next.operation_id, reserved.operation_id);
    assert!(matches!(
        claim(&mut store, &next.operation_id).action,
        Action::CreateBank
    ));
    let requests = f.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|r| r.method == "GET"));
    assert!(requests[2].path.ends_with("/config"));
}

#[tokio::test(flavor = "current_thread")]
async fn existing_bank_and_invalid_config_never_trigger_create_upsert() {
    for malformed in [false, true] {
        let f = Fixture::new();
        let (mut store, agent) = store();
        let reserved = reserve(&mut store, &agent, &f.endpoint);
        let work = claim(&mut store, &reserved.operation_id);
        f.respond(probe());
        let mut value = config(&work.route.bank_id, false);
        if malformed {
            value["config"]["enable_observations"] = json!("false");
        }
        f.respond([(200, value)]);
        let result = perform(&work, || true).await;
        assert!(matches!(result, Observation::NotDispatched { .. }));
        record(&mut store, work, result);
        assert_eq!(
            store.memory_binding(&reserved.binding.id).unwrap().status,
            BindingStatus::Provisioning
        );
        let requests = f.finish();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.method == "GET"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn create_put_then_failed_verification_recovers_by_inspection_only() {
    let f = Fixture::new();
    let (mut store, agent) = store();
    let reserved = reserve(&mut store, &agent, &f.endpoint);
    let work = claim(&mut store, &reserved.operation_id);
    f.respond(probe());
    f.respond([
        (404, json!({})),
        (200, json!({"bank_id":work.route.bank_id})),
        (503, json!({})),
    ]);
    let result = perform(&work, || true).await;
    assert!(matches!(result, Observation::MutationOutcomeUnknown { .. }));
    record(&mut store, work, result);
    let recovery = claim(&mut store, &reserved.operation_id);
    assert!(matches!(recovery.action, Action::InspectBank));
    f.respond(probe());
    // Observations enabled is an unsupported configuration, not BankReady.
    f.respond([(200, config(&recovery.route.bank_id, true))]);
    let result = perform(&recovery, || true).await;
    assert!(matches!(
        result,
        Observation::ReadUnavailable {
            reason: Reason::InvalidResponse
        }
    ));
    record(&mut store, recovery, result);
    assert_eq!(
        store.memory_binding(&reserved.binding.id).unwrap().status,
        BindingStatus::Provisioning
    );
    let requests = f.finish();
    assert_eq!(requests.len(), 8);
    let writes: Vec<_> = requests.iter().filter(|r| r.method != "GET").collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].method, "PUT");
    assert_eq!(writes[0].body, json!({"enable_observations":false}));
}

#[tokio::test(flavor = "current_thread")]
async fn retain_rechecks_authority_after_read_and_rejects_observation_banks() {
    for revoke in [true, false] {
        let f = Fixture::new();
        let (mut store, agent) = store();
        let binding = ready_locally(&mut store, &agent, &f.endpoint);
        let (receipt, work) = retain(&mut store, &agent, &binding);
        f.respond([(200, config(&work.route.bank_id, !revoke))]);
        let checks = AtomicUsize::new(0);
        let result = perform(&work, || {
            !revoke || checks.fetch_add(1, Ordering::SeqCst) == 0
        })
        .await;
        assert!(matches!(
            result,
            Observation::NotDispatched {
                reason: Reason::AuthorityChanged | Reason::ProviderRejected
            }
        ));
        record(&mut store, work, result);
        assert_eq!(
            store
                .external_memory_entry(&agent, &binding, &receipt.evidence_id)
                .unwrap()
                .unwrap()
                .summary
                .state,
            EvidenceState::Blocked
        );
        let requests = f.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_setup_retain_and_poll_persist_one_complete_evidence_chain() {
    let f = Fixture::new();
    let (mut store, agent) = store();
    let reserved = reserve(&mut store, &agent, &f.endpoint);
    let work = claim(&mut store, &reserved.operation_id);
    let bank = work.route.bank_id.clone();
    f.respond(probe());
    f.respond([
        (404, json!({})),
        (200, json!({"bank_id":bank})),
        (200, config(&bank, false)),
    ]);
    let result = perform(&work, || true).await;
    assert!(matches!(
        result,
        Observation::BankReady {
            observations_enabled: false,
            ..
        }
    ));
    record(&mut store, work, result);
    select(&mut store, &agent, &reserved.binding.id);
    let (receipt, work) = retain(&mut store, &agent, &reserved.binding.id);
    let operation = receipt.operation_id.clone().unwrap();
    let Action::Retain {
        document_id,
        payload_sha256,
        ..
    } = &work.action
    else {
        panic!()
    };
    let document = document_id.clone();
    let hash = payload_sha256.clone();
    f.respond([(200,config(&bank,false)),(200,json!({"success":true,"bank_id":bank,"items_count":1,"async":true,"operation_id":operation}))]);
    let result = perform(&work, || true).await;
    assert!(matches!(result, Observation::RetainAccepted { .. }));
    record(&mut store, work, result);
    let poll = claim(&mut store, &operation);
    assert!(matches!(poll.action, Action::PollRetain { .. }));
    f.respond([(
        200,
        json!({"operation_id":operation,"status":"completed","operation_type":"retain"}),
    )]);
    let result = perform(&poll, || true).await;
    record(&mut store, poll, result);
    assert_eq!(
        store
            .external_memory_entry(&agent, &reserved.binding.id, &receipt.evidence_id)
            .unwrap()
            .unwrap()
            .summary
            .state,
        EvidenceState::Committed
    );
    let requests = f.finish();
    assert_eq!(requests.len(), 8);
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body["operation_id"], operation);
    assert_eq!(posts[0].body["items"][0]["document_id"], document);
    assert_eq!(
        posts[0].body["items"][0]["metadata"]["mewu_payload_sha256"],
        hash
    );
    assert_eq!(requests[7].method, "GET");
    assert!(requests[7]
        .path
        .ends_with(&format!("/operations/{operation}")));
}
