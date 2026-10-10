// SPDX-License-Identifier: MPL-2.0
use super::*;
use mewu_core::{ConnectionAdvanced, ConnectionParameters, SceneCommand, LEGACY_CONNECTION_ID, LEGACY_CREDENTIAL_ID};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

fn profile(name: &str) -> ConnectionProfile {
    ConnectionProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        provider_id: "custom".into(),
        base_url: "https://example.test/v1".into(),
        model: "fixture".into(),
        has_key: false,
        credential_id: None,
        revision: 1,
        advanced: ConnectionAdvanced::default(),
    }
}

#[test]
fn plugin_templates_reuse_actual_host_protocol_endpoint_and_parameter_admission() {
    use crate::plugins::PluginContribution;
    let mut count = 0;
    for package in crate::plugins::bundled_manifests() {
        for contribution in package.contributions {
            let PluginContribution::ModelConnection { title, template, .. } = contribution else { continue; };
            let mut profile = template.profile(&title).unwrap();
            profile.model = "synthetic-model".into();
            validate_request_options(&profile).unwrap();
            if profile.base_url.is_empty() { profile.base_url = "http://127.0.0.1:1234/v1".into(); }
            let endpoint = provider_transport::endpoint(&profile).unwrap();
            let suffix = match profile.advanced.protocol {
                ConnectionProtocol::ChatCompletions => "/chat/completions",
                ConnectionProtocol::AnthropicMessages => "/messages",
                ConnectionProtocol::OpenAiResponses => "/responses",
            };
            assert!(endpoint.path().ends_with(suffix)); count += 1;
        }
    }
    assert_eq!(count, 23);
}

#[cfg(windows)]
struct TestRoot(PathBuf);
#[cfg(windows)]
impl TestRoot {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("mewu-connection-host-{}", uuid::Uuid::new_v4()));
        assert!(!path
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn blobs(&self, id: &str) -> BTreeSet<PathBuf> {
        let directory = self.0.join("connections").join(id);
        if !directory.exists() {
            return BTreeSet::new();
        }
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}
#[cfg(windows)]
impl Drop for TestRoot {
    fn drop(&mut self) {
        let absolute = self.0.canonicalize().unwrap();
        let parent = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(absolute.parent(), Some(parent.as_path()));
        assert!(absolute
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mewu-connection-host-"));
        let _ = std::fs::remove_dir_all(absolute);
    }
}

#[cfg(windows)]
#[test]
fn profile_save_ignores_frontend_secret_references_and_keeps_each_dpapi_key_isolated() {
    let root = TestRoot::new();
    let mut store = Store::open_in_memory().unwrap();
    let a = profile("A");
    save_profile(
        &mut store,
        &root.0,
        a.clone(),
        None,
        Some(Zeroizing::new("fixture-key-a".into())),
    )
    .unwrap();
    let saved_a = current_profile(&store, &a.id, Some(1)).unwrap().unwrap();
    let a_blobs = root.blobs(&a.id);
    let bytes = std::fs::read(a_blobs.first().unwrap()).unwrap();
    assert!(!bytes
        .windows(b"fixture-key-a".len())
        .any(|part| part == b"fixture-key-a"));
    assert_eq!(
        &**credentials::read_profile(&root.0, &saved_a).unwrap(),
        "fixture-key-a"
    );

    let mut b = profile("B");
    b.credential_id = saved_a.credential_id.clone();
    b.has_key = true;
    b.revision = 900;
    save_profile(&mut store, &root.0, b.clone(), None, None).unwrap();
    let saved_b = current_profile(&store, &b.id, Some(1)).unwrap().unwrap();
    assert!(!saved_b.has_key);
    assert!(saved_b.credential_id.is_none());
    assert!(credentials::read_profile(&root.0, &saved_b)
        .unwrap()
        .is_empty());
    assert!(root.blobs(&b.id).is_empty());

    save_profile(
        &mut store,
        &root.0,
        b.clone(),
        Some(1),
        Some(Zeroizing::new("fixture-key-b".into())),
    )
    .unwrap();
    save_profile(&mut store, &root.0, b, Some(2), None).unwrap();
    let saved_b = current_profile(&store, &saved_b.id, Some(3))
        .unwrap()
        .unwrap();
    assert_eq!(
        &**credentials::read_profile(&root.0, &saved_b).unwrap(),
        "fixture-key-b"
    );
    assert_ne!(saved_b.credential_id, saved_a.credential_id);
    assert_eq!(root.blobs(&a.id), a_blobs);
    assert_eq!(
        &**credentials::read_profile(&root.0, &saved_a).unwrap(),
        "fixture-key-a"
    );
    let mut cross_directory = saved_b.clone();
    cross_directory.credential_id = saved_a.credential_id.clone();
    assert!(credentials::read_profile(&root.0, &cross_directory).is_err());
}

#[cfg(windows)]
#[test]
fn failed_sql_rotation_removes_only_its_new_blob_and_preserves_running_key_snapshot() {
    let root = TestRoot::new();
    let database = root.0.join("spaces.db");
    let mut store = Store::open(&database).unwrap();
    let saved = profile("事务验证");
    save_profile(
        &mut store,
        &root.0,
        saved.clone(),
        None,
        Some(Zeroizing::new("fixture-original".into())),
    )
    .unwrap();
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetSceneConnection {
            scene_id: scene.clone(),
            connection_id: Some(saved.id.clone()),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "运行中".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    let before = store.snapshot();
    let before_blobs = root.blobs(&saved.id);
    let db = rusqlite::Connection::open(&database).unwrap();
    let persisted: String = db
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER reject_rotation BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let error = save_profile(
        &mut store,
        &root.0,
        saved.clone(),
        Some(1),
        Some(Zeroizing::new("fixture-replacement".into())),
    )
    .unwrap_err();
    assert!(!error.contains("fixture-replacement"));
    assert_eq!(store.snapshot(), before);
    assert_eq!(root.blobs(&saved.id), before_blobs);
    assert!(store.run_is_active(&scene, &run.run_id));
    assert_eq!(
        &**credentials::read_profile(&root.0, &run.connection_profile).unwrap(),
        "fixture-original"
    );
    assert_eq!(
        db.query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        persisted
    );
    assert!(save_profile(
        &mut store,
        &root.0,
        saved,
        Some(999),
        Some(Zeroizing::new("fixture-stale".into()))
    )
    .is_err());
    assert_eq!(root.blobs(&run.connection_profile.id), before_blobs);
    drop(db);
    drop(store);
}

#[cfg(windows)]
#[test]
fn clearing_migrated_key_stops_legacy_fallback_and_forged_reference_cannot_restore_it() {
    let root = TestRoot::new();
    let mut store = Store::open_in_memory().unwrap();
    credentials::write(&root.0, "fixture-legacy-only").unwrap();
    let mut legacy = store.snapshot().connections[0].clone();
    assert_eq!(legacy.id, LEGACY_CONNECTION_ID);
    legacy.revision = 2;
    legacy.has_key = true;
    legacy.credential_id = Some(LEGACY_CREDENTIAL_ID.into());
    store
        .save_connection_profile(legacy.clone(), Some(1))
        .unwrap();
    assert_eq!(
        &**credentials::read_profile(&root.0, &legacy).unwrap(),
        "fixture-legacy-only"
    );
    save_profile(
        &mut store,
        &root.0,
        legacy.clone(),
        Some(2),
        Some(Zeroizing::new(String::new())),
    )
    .unwrap();
    let cleared = current_profile(&store, LEGACY_CONNECTION_ID, Some(3))
        .unwrap()
        .unwrap();
    assert!(!cleared.has_key);
    assert!(cleared.credential_id.is_none());
    assert!(credentials::read_profile(&root.0, &cleared)
        .unwrap()
        .is_empty());
    assert!(root.0.join("connection.bin").is_file());
    save_profile(&mut store, &root.0, legacy, Some(3), None).unwrap();
    let unchanged = current_profile(&store, LEGACY_CONNECTION_ID, Some(4))
        .unwrap()
        .unwrap();
    assert!(unchanged.credential_id.is_none());
    assert!(credentials::read_profile(&root.0, &unchanged)
        .unwrap()
        .is_empty());
    let mut foreign = profile("Foreign");
    foreign.has_key = true;
    foreign.credential_id = Some(LEGACY_CREDENTIAL_ID.into());
    assert!(credentials::read_profile(&root.0, &foreign).is_err());
}

#[test]
fn starting_new_probes_does_not_refresh_finished_request_tombstones() {
    let mut registry = ProbeRegistry::default();
    let old = uuid::Uuid::new_v4().to_string();
    let _old_receiver = registry.start("settings", &old, "connection").unwrap();
    registry.finish("settings", &old);
    let touched = Instant::now() - Duration::from_secs(100);
    registry.entries.get_mut(&("settings".into(), old.clone())).unwrap().touched = touched;
    let new = uuid::Uuid::new_v4().to_string();
    let _new_receiver = registry.start("settings", &new, "connection").unwrap();
    assert_eq!(registry.entries[&("settings".into(), old.clone())].touched, touched);
    registry.entries.get_mut(&("settings".into(), old.clone())).unwrap().touched = Instant::now() - TOMBSTONE_TIME - Duration::from_secs(1);
    registry.prune();
    assert!(!registry.entries.contains_key(&("settings".into(), old)));
    assert!(registry.entries.contains_key(&("settings".into(), new)));
}

#[tokio::test]
async fn early_cancel_tombstone_rejects_registration_and_replacement_is_owner_scoped() {
    let mut registry = ProbeRegistry::default();
    let early = uuid::Uuid::new_v4().to_string();
    registry.cancel("settings", &early).unwrap();
    assert!(registry.start("settings", &early, "a").is_err());
    let space_id = uuid::Uuid::new_v4().to_string();
    let mut space = registry.start("space", &space_id, "b").unwrap();
    let first_id = uuid::Uuid::new_v4().to_string();
    let first = registry.start("settings", &first_id, "a").unwrap();
    let second_id = uuid::Uuid::new_v4().to_string();
    let second = registry.start("settings", &second_id, "a").unwrap();
    assert!(first.await.is_ok());
    assert!(matches!(
        space.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    registry.cancel_connection("a");
    assert!(second.await.is_ok());
    assert!(matches!(
        space.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    registry.cancel_owner("space");
    assert!(space.await.is_ok());
    assert!(registry.start("settings", &first_id, "a").is_err());
}

#[tokio::test]
async fn cancellation_wins_before_any_probe_io_and_lease_drop_prevents_reuse() {
    let registry = Arc::new(Mutex::new(ProbeRegistry::default()));
    let id = uuid::Uuid::new_v4().to_string();
    let mut receiver = registry
        .lock()
        .unwrap()
        .start("settings", &id, "profile")
        .unwrap();
    let lease = ProbeLease {
        registry: registry.clone(),
        owner: "settings".into(),
        id: id.clone(),
    };
    registry.lock().unwrap().cancel("settings", &id).unwrap();
    let touched = AtomicBool::new(false);
    tokio::select! {
        biased;
        _ = &mut receiver => (),
        _ = async { touched.store(true, Ordering::SeqCst); } => panic!("canceled probe reached I/O"),
    }
    assert!(!touched.load(Ordering::SeqCst));
    drop(lease);
    assert!(registry
        .lock()
        .unwrap()
        .start("settings", &id, "profile")
        .is_err());
    let next = uuid::Uuid::new_v4().to_string();
    let next_receiver = registry
        .lock()
        .unwrap()
        .start("settings", &next, "profile")
        .unwrap();
    drop(ProbeLease {
        registry: registry.clone(),
        owner: "settings".into(),
        id: next.clone(),
    });
    assert!(next_receiver.await.is_err());
    assert!(registry
        .lock()
        .unwrap()
        .start("settings", &next, "profile")
        .is_err());
}

#[test]
fn model_catalog_validation_is_bounded_and_does_not_treat_cursor_as_url() {
    let mut models = BTreeSet::new();
    assert!(parse_models_page(
        &json!([{"id":"z-model"},{"id":"a-model"},{"id":"z-model"}]),
        &mut models
    )
    .unwrap()
    .is_none());
    assert_eq!(
        models.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["a-model", "z-model"]
    );
    let cursor = "https://untrusted.invalid/next?key=not-a-request";
    assert_eq!(
        parse_models_page(
            &json!({"data":[{"id":"other"}],"has_more":true,"last_id":cursor}),
            &mut models
        )
        .unwrap()
        .as_deref(),
        Some(cursor)
    );
    for malformed in [
        json!({"data":[] ,"has_more":true,"last_id":"cursor"}),
        json!({"data":[{"id":"valid"}],"has_more":true}),
        json!({"data":[{"id":"bad\nname"}]}),
        json!({"data":[{"id":"x".repeat(257)}]}),
    ] {
        assert!(parse_models_page(&malformed, &mut BTreeSet::new()).is_err());
    }
    let too_many: Vec<Value> = (0..=MAX_MODELS)
        .map(|n| json!({"id":format!("model-{n}")}))
        .collect();
    assert!(parse_models_page(&json!({"data":too_many}), &mut BTreeSet::new()).is_err());
    let mut full: BTreeSet<String> = (0..MAX_MODELS).map(|n| format!("model-{n}")).collect();
    assert!(parse_models_page(&json!({"data":[{"id":"over-total"}]}), &mut full).is_err());
}

struct HttpFixture {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl HttpFixture {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let log = requests.clone();
        let signal = stop.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut responses = responses.into_iter();
            while !signal.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                // Winsock accepts inherit nonblocking state; restore blocking
                // with deadlines so concurrent fixture tests cannot race reads.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while request.len() < 32 * 1024 && !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                let response = responses.next().unwrap_or_else(|| b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
                let _ = stream.write_all(&response);
            }
        });
        Self {
            base,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn json(body: Value) -> Vec<u8> {
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).into_bytes();
        response.extend(bytes);
        response
    }
    fn profile(&self) -> ConnectionProfile {
        let mut value = profile("Local fixture");
        value.base_url = format!("{}/v1", self.base);
        value
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn local_catalog_pages_use_only_original_origin_and_encode_cursor_as_data() {
    let cursor = "https://untrusted.invalid/next?token=fixture&x=1";
    let server = HttpFixture::new(vec![
        HttpFixture::json(json!({"data":[{"id":"z"}],"has_more":true,"last_id":cursor})),
        HttpFixture::json(json!({"data":[{"id":"a"},{"id":"z"}],"has_more":false})),
    ]);
    let mut profile = server.profile();
    profile.advanced.protocol = ConnectionProtocol::AnthropicMessages;
    profile.advanced.auth_mode = ConnectionAuthMode::ApiKey;
    assert_eq!(
        discover_models(&profile, "fixture-local-key")
            .await
            .unwrap(),
        vec!["a", "z"]
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let path = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = url::Url::parse(&server.base).unwrap().join(path).unwrap();
        assert_eq!(
            url.origin(),
            url::Url::parse(&server.base).unwrap().origin()
        );
        assert_eq!(url.path(), "/v1/models");
        assert!(request
            .to_ascii_lowercase()
            .contains("x-api-key: fixture-local-key"));
        assert!(request
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }
    let next = requests[1]
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let next = url::Url::parse(&server.base).unwrap().join(next).unwrap();
    assert_eq!(
        next.query_pairs()
            .find(|(name, _)| name == "after_id")
            .unwrap()
            .1,
        cursor
    );
}

#[tokio::test]
async fn catalog_redirect_is_not_followed_and_provider_error_body_is_not_disclosed() {
    let trap = HttpFixture::new(vec![HttpFixture::json(
        json!({"data":[{"id":"should-never-reach"}]}),
    )]);
    let redirect = format!("HTTP/1.1 302 Found\r\nLocation: {}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", trap.base).into_bytes();
    let source = HttpFixture::new(vec![redirect]);
    let error = discover_models(&source.profile(), "fixture-redirection-key")
        .await
        .unwrap_err();
    assert!(error.contains("302"));
    assert_eq!(source.requests.lock().unwrap().len(), 1);
    assert!(trap.requests.lock().unwrap().is_empty());
    assert!(!error.contains("fixture-redirection-key"));
    let body = b"sensitive-provider-debug-fixture";
    let mut response = format!(
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend(body);
    let rejected = HttpFixture::new(vec![response]);
    let error = discover_models(&rejected.profile(), "fixture-key")
        .await
        .unwrap_err();
    assert!(error.contains("401"));
    assert!(!error.contains("sensitive-provider"));
}

#[tokio::test]
async fn catalog_enforces_declared_and_streamed_byte_limits_and_repeated_cursor_limit() {
    let declared = HttpFixture::new(vec![format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_CATALOG_BYTES + 1
    )
    .into_bytes()]);
    assert!(discover_models(&declared.profile(), "")
        .await
        .unwrap_err()
        .contains("过大"));
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    response.extend(format!("{:X}\r\n", MAX_CATALOG_BYTES + 1).as_bytes());
    response.extend(vec![b' '; MAX_CATALOG_BYTES + 1]);
    response.extend(b"\r\n0\r\n\r\n");
    let chunked = HttpFixture::new(vec![response]);
    assert!(discover_models(&chunked.profile(), "")
        .await
        .unwrap_err()
        .contains("过大"));
    let page = json!({"data":[{"id":"model"}],"has_more":true,"last_id":"repeat"});
    let repeated = HttpFixture::new(vec![
        HttpFixture::json(page.clone()),
        HttpFixture::json(page),
    ]);
    assert!(discover_models(&repeated.profile(), "")
        .await
        .unwrap_err()
        .contains("分页重复"));
    assert_eq!(repeated.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn auth_none_catalog_never_sends_a_saved_key_and_plain_remote_http_is_rejected() {
    let server = HttpFixture::new(vec![HttpFixture::json(json!({"data":[{"id":"fixture"}]}))]);
    let mut profile = server.profile();
    profile.advanced.auth_mode = ConnectionAuthMode::None;
    assert_eq!(
        discover_models(&profile, "fixture-key-must-not-be-sent")
            .await
            .unwrap(),
        vec!["fixture"]
    );
    let request = server.requests.lock().unwrap()[0].to_ascii_lowercase();
    assert!(
        !request.contains("authorization")
            && !request.contains("x-api-key")
            && !request.contains("fixture-key-must-not-be-sent")
    );
    profile.base_url = "http://example.test/v1".into();
    assert!(discover_models(&profile, "fixture-key-must-not-be-sent")
        .await
        .is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn advanced_parameters_cannot_override_host_message_and_tool_fields() {
    let mut profile = profile("Options");
    profile.advanced.request_parameters = ConnectionParameters {
        temperature: Some(0.5),
        top_p: Some(0.9),
        service_tier: Some(mewu_core::ConnectionServiceTier::Standard),
    };
    let mut body = json!({"model":"host-model","stream":true,"messages":[{"role":"user","content":"host-input"}],"tools":[]});
    let old_messages = body["messages"].clone();
    let old_tools = body["tools"].clone();
    apply_parameters(&mut body, &profile).unwrap();
    assert_eq!(body["messages"], old_messages);
    assert_eq!(body["tools"], old_tools);
    assert_eq!(body["model"], "host-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["temperature"], 0.5);
    assert_eq!(body["top_p"], 0.9);
    assert_eq!(body["service_tier"], "standard");
}
