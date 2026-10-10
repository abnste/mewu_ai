// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::{params, Connection as Database};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

struct TempDb {
    directory: PathBuf,
    path: PathBuf,
}
impl TempDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-connections-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        Self {
            path: directory.join("spaces.db"),
            directory,
        }
    }
}
impl Drop for TempDb {
    fn drop(&mut self) {
        for file in ["spaces.db", "spaces.db-wal", "spaces.db-shm"] {
            let _ = std::fs::remove_file(self.directory.join(file));
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}
fn profile(name: &str) -> ConnectionProfile {
    ConnectionProfile {
        id: Uuid::new_v4().to_string(),
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
fn bind(store: &mut Store, scene: &str, connection: Option<&str>) {
    store
        .apply(SceneCommand::SetSceneConnection {
            scene_id: scene.into(),
            connection_id: connection.map(Into::into),
        })
        .unwrap();
}
fn start(store: &mut Store, scene: &str) -> RunContext {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.into(),
            draft: "当前问题".into(),
        })
        .unwrap();
    store.begin_run(scene).unwrap()
}
fn payload(db: &Database) -> String {
    db.query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap()
}
fn legacy_v2(temp: &TempDb, has_key: bool) -> Value {
    let mut store = Store::open(&temp.path).unwrap();
    let state = store.snapshot();
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: state.agents[0].id.clone(),
            id: None,
            text: "不会被连接迁移丢掉".into(),
            expected_revision: None,
        })
        .unwrap();
    let scene = state.active_scene_id;
    let run = start(&mut store, &scene);
    store.finish_run(&scene, &run.run_id, "旧回复").unwrap();
    drop(store);
    let db = Database::open(&temp.path).unwrap();
    let mut value: Value = serde_json::from_str(&payload(&db)).unwrap();
    value.as_object_mut().unwrap().remove("connections");
    value.as_object_mut().unwrap().remove("defaultConnectionId");
    value["connection"] =
        json!({"baseUrl":"https://legacy.example/v1","model":"old-model","hasKey":has_key});
    for scene in value["scenes"].as_array_mut().unwrap() {
        scene.as_object_mut().unwrap().remove("connectionId");
        if let Some(run) = scene.get_mut("run").and_then(Value::as_object_mut) {
            run.remove("connectionId");
            run.remove("connectionRevision");
        }
    }
    for agent in value["agents"].as_array_mut().unwrap() {
        agent.as_object_mut().unwrap().remove("defaultConnectionId");
    }
    db.execute(
        "UPDATE app_state SET payload=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    db.execute_batch("DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;DROP TABLE visual_run_inputs;DROP TABLE agent_run_events; DROP TABLE agent_run_records; DROP TABLE external_memory_tombstones; DROP TABLE external_memory_outbox; DROP TABLE external_memory_evidence; DROP TABLE external_memory_bindings;").unwrap();
    db.pragma_update(None, "user_version", 2).unwrap();
    value
}

#[test]
fn scene_routes_are_stable_and_new_scenes_use_agent_then_global_defaults() {
    let mut store = Store::open_in_memory().unwrap();
    let first = store.snapshot().active_scene_id;
    let a = profile("图像");
    let b = profile("文本");
    store.save_connection_profile(a.clone(), None).unwrap();
    store.save_connection_profile(b.clone(), None).unwrap();
    bind(&mut store, &first, Some(&a.id));
    store
        .apply(SceneCommand::SetDefaultConnection {
            connection_id: Some(b.id.clone()),
        })
        .unwrap();
    assert_eq!(store.connection_for_scene(&first).unwrap(), a);
    let context = start(&mut store, &first);
    assert_eq!(context.connection_profile, a);
    assert_eq!(context.connection, a.connection());
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: first.clone(),
        })
        .unwrap();
    let second = store.snapshot().active_scene_id;
    assert_eq!(store.connection_for_scene(&second).unwrap(), b);
    let mut agent = store.snapshot().agents[0].clone();
    agent.default_connection_id = Some(a.id.clone());
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    store.apply(SceneCommand::NewScene).unwrap();
    let third = store.snapshot().active_scene_id;
    assert_eq!(store.connection_for_scene(&third).unwrap(), a);
    assert_eq!(store.connection_for_scene(&second).unwrap(), b);
    assert!(store.run_is_active(&first, &context.run_id));
    assert_eq!(store.snapshot().connection, b.connection());
}

#[test]
fn explicit_scene_switch_preserves_history_and_draft_and_rejects_inflight_change() {
    let mut store = Store::open_in_memory().unwrap();
    let scene = store.snapshot().active_scene_id;
    let next = profile("第二个");
    store.save_connection_profile(next.clone(), None).unwrap();
    let context = start(&mut store, &scene);
    let before = store.snapshot();
    assert!(matches!(
        store.apply(SceneCommand::SetSceneConnection {
            scene_id: scene.clone(),
            connection_id: Some(next.id.clone())
        }),
        Err(CoreError::RunInProgress)
    ));
    assert_eq!(store.snapshot(), before);
    store
        .finish_run(&scene, &context.run_id, "已完成回复")
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "未发送草稿".into(),
        })
        .unwrap();
    let old_scene = store.snapshot().scenes[0].clone();
    bind(&mut store, &scene, Some(&next.id));
    let new_scene = store.snapshot().scenes[0].clone();
    assert_eq!(new_scene.messages, old_scene.messages);
    assert_eq!(new_scene.draft, old_scene.draft);
    assert_eq!(new_scene.refs, old_scene.refs);
    let next_run = store.begin_run(&scene).unwrap();
    assert_eq!(next_run.connection_profile.id, next.id);
    assert_eq!(next_run.messages[1].text, "已完成回复");
}

#[test]
fn profile_cas_cancels_only_bound_runs_and_immutable_context_keeps_old_config() {
    let mut store = Store::open_in_memory().unwrap();
    let first = store.snapshot().active_scene_id;
    let mut a = profile("A");
    let b = profile("B");
    store.save_connection_profile(a.clone(), None).unwrap();
    store.save_connection_profile(b.clone(), None).unwrap();
    bind(&mut store, &first, Some(&a.id));
    let old = start(&mut store, &first);
    store
        .begin_tool(&first, &old.run_id, "call-1", "memory_recall", "查找记忆")
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: first.clone(),
        })
        .unwrap();
    let second = store.snapshot().active_scene_id;
    bind(&mut store, &second, Some(&b.id));
    let other = start(&mut store, &second);
    let before = store.snapshot();
    a.revision = 2;
    a.model = "new-model".into();
    assert!(matches!(
        store.save_connection_profile(a.clone(), Some(0)),
        Err(CoreError::ConnectionConflict)
    ));
    assert_eq!(store.snapshot(), before);
    store.save_connection_profile(a.clone(), Some(1)).unwrap();
    assert!(!store.run_is_active(&first, &old.run_id));
    assert!(store.run_is_active(&second, &other.run_id));
    assert_eq!(old.connection_profile.model, "fixture");
    assert_eq!(old.connection_profile.revision, 1);
    assert_eq!(store.snapshot().active_scene_id, second);
    assert_eq!(
        store.snapshot().scenes[0]
            .messages
            .last()
            .unwrap()
            .tool_steps[0]
            .status,
        ToolStepStatus::Failed
    );
    assert!(matches!(
        store.finish_run(&first, &old.run_id, "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.save_connection_profile(a, Some(1)),
        Err(CoreError::ConnectionConflict)
    ));
}

#[test]
fn delete_last_connection_clears_routes_without_fallback_or_resurrection() {
    let temp = TempDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let initial = store.snapshot();
    let mut agent = initial.agents[0].clone();
    agent.default_connection_id = Some(LEGACY_CONNECTION_ID.into());
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    let run = start(&mut store, &initial.active_scene_id);
    store
        .remove_connection_profile(LEGACY_CONNECTION_ID, 1)
        .unwrap();
    let state = store.snapshot();
    assert!(state.connections.is_empty());
    assert!(state.default_connection_id.is_none());
    assert!(state.agents[0].default_connection_id.is_none());
    assert!(state.scenes[0].connection_id.is_none());
    assert!(!store.run_is_active(&initial.active_scene_id, &run.run_id));
    assert!(matches!(
        store.begin_run(&initial.active_scene_id),
        Err(CoreError::ConnectionUnselected)
    ));
    drop(store);
    let mut reopened = Store::open(&temp.path).unwrap();
    assert_eq!(reopened.snapshot(), state);
    let created = profile("重新添加");
    reopened.save_connection_profile(created, None).unwrap();
    assert!(reopened.snapshot().default_connection_id.is_none());
    assert!(reopened.snapshot().scenes[0].connection_id.is_none());
    let mut reserved = profile("不能复活旧密钥");
    reserved.id = LEGACY_CONNECTION_ID.into();
    assert!(reopened.save_connection_profile(reserved, None).is_err());
}

#[test]
fn v2_migration_preserves_memory_and_history_and_fixed_legacy_secret_reference() {
    let temp = TempDb::new();
    let old = legacy_v2(&temp, true);
    let mut store = Store::open(&temp.path).unwrap();
    let snapshot = store.snapshot();
    let migrated = &snapshot.connections[0];
    assert_eq!(migrated.id, LEGACY_CONNECTION_ID);
    assert_eq!(
        migrated.credential_id.as_deref(),
        Some(LEGACY_CREDENTIAL_ID)
    );
    assert_eq!(migrated.revision, 1);
    assert_eq!(
        migrated.connection(),
        serde_json::from_value::<Connection>(old["connection"].clone()).unwrap()
    );
    assert_eq!(snapshot.scenes[0].messages[1].text, "旧回复");
    assert_eq!(
        store
            .memory_page(&snapshot.agents[0].id, "", None, 25)
            .unwrap()
            .total,
        1
    );
    let db = Database::open(&temp.path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
    assert_eq!(snapshot.schema_version, 1);
    let mut cleared = migrated.clone();
    cleared.revision = 2;
    cleared.has_key = false;
    cleared.credential_id = None;
    store.save_connection_profile(cleared, Some(1)).unwrap();
    let cleared = store.snapshot();
    drop(store);
    assert_eq!(Store::open(&temp.path).unwrap().snapshot(), cleared);
    assert!(!cleared.connection.has_key);
}

#[test]
fn migration_failure_rolls_back_schema_payload_and_existing_memory() {
    let temp = TempDb::new();
    legacy_v2(&temp, false);
    let db = Database::open(&temp.path).unwrap();
    let before = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_connection_migration BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(Store::open(&temp.path).is_err());
    assert_eq!(payload(&db), before);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM memory_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER reject_connection_migration")
        .unwrap();
    assert_eq!(
        Store::open(&temp.path)
            .unwrap()
            .snapshot()
            .connections
            .len(),
        1
    );
}

#[test]
fn failed_profile_write_does_not_partially_revoke_run_or_change_secret_reference() {
    let temp = TempDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene);
    let before = store.snapshot();
    let db = Database::open(&temp.path).unwrap();
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_profile BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let mut changed = before.connections[0].clone();
    changed.revision += 1;
    changed.has_key = true;
    changed.credential_id = Some(Uuid::new_v4().to_string());
    assert!(store.save_connection_profile(changed, Some(1)).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    assert!(store.run_is_active(&scene, &run.run_id));
    assert!(store
        .remove_connection_profile(LEGACY_CONNECTION_ID, 1)
        .is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn restricted_options_and_identity_cannot_smuggle_secrets_or_override_messages() {
    let valid = profile("合法");
    for (key, value) in [
        ("apiKey", json!("secret")),
        ("customHeaders", json!({"Authorization":"secret"})),
    ] {
        let mut encoded = serde_json::to_value(&valid).unwrap();
        encoded[key] = value;
        assert!(serde_json::from_value::<ConnectionProfile>(encoded).is_err());
    }
    assert!(serde_json::from_value::<ConnectionParameters>(json!({"messages":[]})).is_err());
    assert!(
        serde_json::from_value::<ConnectionParameters>(json!({"service_tier":"invented"})).is_err()
    );
    assert!(serde_json::from_value::<SceneCommand>(
        json!({"type":"save_connection_profile","profile":valid})
    )
    .is_err());
    for path in [
        "https://evil.test",
        "//evil.test",
        "../messages",
        "/v1/../messages",
        "/v1/%2e%2e",
        "/v1?key=secret",
        "/v1\\messages",
    ] {
        let mut bad = valid.clone();
        bad.advanced.request_path = Some(path.into());
        assert!(validate_connection_profile(&bad).is_err(), "{path}");
    }
    for value in [-0.1, 2.1, f64::NAN, f64::INFINITY] {
        let mut bad = valid.clone();
        bad.advanced.request_parameters.temperature = Some(value);
        assert!(validate_connection_profile(&bad).is_err());
    }
    let mut bad = valid.clone();
    bad.has_key = true;
    assert!(validate_connection_profile(&bad).is_err());
    bad.credential_id = Some(LEGACY_CREDENTIAL_ID.into());
    assert!(validate_connection_profile(&bad).is_err());
    for protocol in [
        ConnectionProtocol::ChatCompletions,
        ConnectionProtocol::AnthropicMessages,
        ConnectionProtocol::OpenAiResponses,
    ] {
        let mut accepted = valid.clone();
        accepted.advanced.protocol = protocol;
        accepted.advanced.request_path = Some("/v1/messages".into());
        accepted.advanced.request_parameters = ConnectionParameters {
            temperature: Some(0.0),
            top_p: Some(1.0),
            service_tier: Some(ConnectionServiceTier::Priority),
        };
        validate_connection_profile(&accepted).unwrap();
    }
}

#[test]
fn corrupt_modern_connection_catalog_is_not_recreated_from_legacy_projection() {
    let temp = TempDb::new();
    drop(Store::open(&temp.path).unwrap());
    let db = Database::open(&temp.path).unwrap();
    let mut value: Value = serde_json::from_str(&payload(&db)).unwrap();
    value.as_object_mut().unwrap().remove("connections");
    let malformed = serde_json::to_string(&value).unwrap();
    db.execute("UPDATE app_state SET payload=?1", params![malformed])
        .unwrap();
    assert!(Store::open(&temp.path).is_err());
    assert_eq!(payload(&db), malformed);
}
