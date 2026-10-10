// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection as SqlConnection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb {
    directory: PathBuf,
    path: PathBuf,
}
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-core-test-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("scenes.sqlite3");
        Self { directory, path }
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        // Only delete this test's known file and then its empty directory.
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

fn draft(store: &mut Store, scene_id: &str, text: &str) {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.into(),
            draft: text.into(),
        })
        .unwrap();
}
fn image() -> Asset {
    Asset {
        id: Uuid::new_v4().to_string(),
        name: "截图.png".into(),
        kind: AssetKind::Image,
        path: "assets/capture.png".into(),
        width: Some(1920),
        height: Some(1080),
        origin_x: Some(-1920),
        origin_y: Some(0),
        scale_factor: Some(1.25),
    }
}
fn region(id: &str) -> Region {
    Region {
        id: id.into(),
        x: 12.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
        ..Default::default()
    }
}

#[test]
fn public_reasoning_is_optional_exact_persistent_and_never_part_of_request_text() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let id = store.snapshot().active_scene_id;
    draft(&mut store, &id, "问题");
    let run = store.begin_run(&id).unwrap();
    let thought = "公开摘要🙂\n保留原文 <&>";
    let result = store
        .finish_run_with_reasoning_and_assets(
            &id,
            &run.run_id,
            "回答",
            Some(thought.into()),
            vec![],
        )
        .unwrap();
    let message = result.scenes[0].messages.last().unwrap();
    assert_eq!(message.reasoning.as_deref(), Some(thought));
    assert_eq!(message.text, "回答");
    let user = serde_json::to_value(&result.scenes[0].messages[0]).unwrap();
    assert!(user.get("reasoning").is_none());
    assert!(user.get("conversationStart").is_none());
    drop(store);
    let mut store = Store::open(&file.path).unwrap();
    assert_eq!(
        store.snapshot().scenes[0]
            .messages
            .last()
            .unwrap()
            .reasoning
            .as_deref(),
        Some(thought)
    );
    draft(&mut store, &id, "后续");
    let run = store.begin_run(&id).unwrap();
    assert_eq!(run.messages[1].text, "回答");
    let result = store.finish_run(&id, &run.run_id, "普通回复").unwrap();
    assert!(result.scenes[0]
        .messages
        .last()
        .unwrap()
        .reasoning
        .is_none());
    let db = SqlConnection::open(&file.path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        9
    );
}

#[test]
fn reasoning_budget_and_late_run_failure_are_atomic() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.snapshot().active_scene_id;
    draft(&mut store, &id, "问题");
    let run = store.begin_run(&id).unwrap();
    let before = store.snapshot();
    assert!(store
        .finish_run_with_reasoning_and_assets(
            &id,
            &run.run_id,
            "回答",
            Some("x".repeat(2 * 1024 * 1024 + 1)),
            vec![image()]
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    store.cancel_run(&id).unwrap();
    let before = store.snapshot();
    assert!(matches!(
        store.finish_run_with_reasoning_and_assets(
            &id,
            &run.run_id,
            "回答",
            Some("迟到".into()),
            vec![]
        ),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(store.snapshot(), before);
}

#[test]
fn new_conversation_preserves_space_and_archive_but_request_starts_at_the_first_new_user() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let id = store.snapshot().active_scene_id;
    store.set_background(&id, image()).unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: id.clone(),
            region: region("same-region"),
        })
        .unwrap();
    store.add_asset(&id, image()).unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: "same-region".into(),
            }],
        })
        .unwrap();
    draft(&mut store, &id, "旧问题");
    let old = store.begin_run(&id).unwrap();
    store
        .finish_run_with_reasoning_and_assets(
            &id,
            &old.run_id,
            "旧回答",
            Some("旧公开摘要".into()),
            vec![],
        )
        .unwrap();
    draft(&mut store, &id, "未发送草稿");
    let before = store.snapshot();
    let command: SceneCommand =
        serde_json::from_value(serde_json::json!({"type":"new_conversation","sceneId":id}))
            .unwrap();
    let after = store.apply(command).unwrap();
    let old_scene = &before.scenes[0];
    let scene = &after.scenes[0];
    assert_eq!(after.scenes.len(), 1);
    assert_eq!(scene.id, old_scene.id);
    assert_eq!(scene.background, old_scene.background);
    assert_eq!(scene.regions, old_scene.regions);
    assert_eq!(scene.items, old_scene.items);
    assert_eq!(scene.refs, old_scene.refs);
    assert_eq!(scene.agent_id, old_scene.agent_id);
    assert_eq!(scene.connection_id, old_scene.connection_id);
    assert_eq!(scene.messages, old_scene.messages);
    assert_eq!(scene.conversation_start, Some(2));
    assert!(scene.draft.is_empty());
    assert!(scene.run.is_none());
    let repeated = store
        .apply(SceneCommand::NewConversation {
            scene_id: id.clone(),
        })
        .unwrap();
    assert_eq!(repeated, after);
    // A rejected input may not fabricate the first-user marker or discard the boundary.
    store
        .apply(SceneCommand::SetRefs {
            scene_id: id.clone(),
            refs: vec![],
        })
        .unwrap();
    let before = store.snapshot();
    assert!(matches!(store.begin_run(&id), Err(CoreError::EmptyInput)));
    assert_eq!(store.snapshot(), before);
    drop(store);
    let mut store = Store::open(&file.path).unwrap();
    draft(&mut store, &id, "新问题");
    let new = store.begin_run(&id).unwrap();
    assert_eq!(new.messages.len(), 1);
    assert_eq!(new.messages[0].text, "新问题");
    assert_eq!(new.messages[0].conversation_start, Some(2));
    assert_eq!(new.background, old_scene.background);
    assert_eq!(new.regions, old_scene.regions);
    assert_eq!(new.items, old_scene.items);
    store.finish_run(&id, &new.run_id, "新回答").unwrap();
    draft(&mut store, &id, "新后续");
    let follow = store.begin_run(&id).unwrap();
    assert_eq!(
        follow
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["新问题", "新回答", "新后续"]
    );
}

#[test]
fn new_conversation_rejects_running_and_frozen_then_survives_activation_and_reopen() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let id = store.snapshot().active_scene_id;
    draft(&mut store, &id, "原问题");
    let run = store.begin_run(&id).unwrap();
    let before = store.snapshot();
    assert!(matches!(
        store.apply(SceneCommand::NewConversation {
            scene_id: id.clone()
        }),
        Err(CoreError::RunInProgress)
    ));
    assert_eq!(store.snapshot(), before);
    store.finish_run(&id, &run.run_id, "原回答").unwrap();
    store
        .apply(SceneCommand::NewConversation {
            scene_id: id.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: id.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    assert!(store
        .apply(SceneCommand::NewConversation {
            scene_id: id.clone()
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    drop(store);
    let mut store = Store::open(&file.path).unwrap();
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: id.clone(),
        })
        .unwrap();
    draft(&mut store, &id, "恢复后新问题");
    let run = store.begin_run(&id).unwrap();
    assert_eq!(run.messages.len(), 1);
    assert_eq!(run.messages[0].conversation_start, Some(2));
}

#[test]
fn frozen_a_finishes_while_b_remains_active() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store.snapshot().active_scene_id;
    draft(&mut store, &a, "分析 A");
    let run = store.begin_run(&a).unwrap();
    let frozen = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap();
    let b = frozen.active_scene_id;
    assert_ne!(a, b);
    assert_eq!(
        frozen.scenes[0].run.as_ref().unwrap().status,
        RunStatus::Running
    );
    draft(&mut store, &b, "继续 B");
    let result = store.finish_run(&a, &run.run_id, "A 已完成").unwrap();
    assert_eq!(result.active_scene_id, b);
    assert!(result.scenes[0].frozen);
    assert_eq!(result.scenes[0].messages.last().unwrap().text, "A 已完成");
    assert_eq!(result.scenes[1].draft, "继续 B");
    assert_eq!(result.scenes[0].agent_id, result.scenes[1].agent_id);
    store.add_asset(&a, image()).unwrap();
    assert_eq!(store.snapshot().active_scene_id, b);
}

#[test]
fn canceled_and_replaced_runs_cannot_accept_late_results() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "第一条");
    let old = store.begin_run(&scene_id).unwrap();
    assert!(store.run_is_active(&scene_id, &old.run_id));
    assert!(!store.run_is_active("missing-scene", &old.run_id));
    assert!(!store.run_is_active(&scene_id, "missing-run"));
    let canceled = store.cancel_run(&scene_id).unwrap();
    assert!(!store.run_is_active(&scene_id, &old.run_id));
    assert!(matches!(
        store.finish_run(&scene_id, &old.run_id, "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(store.snapshot(), canceled);
    draft(&mut store, &scene_id, "第二条");
    let new = store.begin_run(&scene_id).unwrap();
    assert!(store.run_is_active(&scene_id, &new.run_id));
    assert!(!store.run_is_active(&scene_id, &old.run_id));
    let before = store.snapshot();
    assert!(matches!(
        store.fail_run(&scene_id, &old.run_id, "旧请求网络错误"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(store.snapshot(), before);
    store
        .finish_run(&scene_id, &new.run_id, "正确结果")
        .unwrap();
    assert!(!store.run_is_active(&scene_id, &new.run_id));
    assert!(matches!(
        store.finish_run(&scene_id, &new.run_id, "重复结果"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(
        store.snapshot().scenes[0]
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count(),
        1
    );
}

#[test]
fn restart_keeps_scenes_and_context_and_settles_interrupted_run() {
    let file = TestDb::new();
    let (a, b, agent_id, run_id, asset_id);
    {
        let mut store = Store::open(&file.path).unwrap();
        a = store.snapshot().active_scene_id;
        agent_id = store.snapshot().agents[0].id.clone();
        let background = image();
        asset_id = background.id.clone();
        store.set_background(&a, background).unwrap();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: a.clone(),
                region: region("selected-region"),
            })
            .unwrap();
        store
            .apply(SceneCommand::SetRefs {
                scene_id: a.clone(),
                refs: vec![Reference {
                    kind: ReferenceKind::Region,
                    id: "selected-region".into(),
                }],
            })
            .unwrap();
        draft(&mut store, &a, "请分析这里");
        run_id = store.begin_run(&a).unwrap().run_id;
        b = store
            .apply(SceneCommand::FreezeScene {
                scene_id: a.clone(),
            })
            .unwrap()
            .active_scene_id;
        draft(&mut store, &b, "未发送草稿");
    }
    let mut reopened = Store::open(&file.path).unwrap();
    let state = reopened.snapshot();
    assert_eq!(state.active_scene_id, b);
    assert_eq!(state.agents[0].id, agent_id);
    assert_eq!(state.scenes[0].background.as_ref().unwrap().id, asset_id);
    assert_eq!(state.scenes[0].regions.len(), 1);
    assert_eq!(
        state.scenes[0].messages[0].refs.as_ref().unwrap()[0].id,
        "selected-region"
    );
    let interrupted = state.scenes[0].run.as_ref().unwrap();
    assert_eq!(interrupted.status, RunStatus::Failed);
    assert_eq!(interrupted.error.as_deref(), Some("上次运行已中断"));
    assert!(state.scenes[0].frozen);
    assert_eq!(state.scenes[1].draft, "未发送草稿");
    assert!(matches!(
        reopened.finish_run(&a, &run_id, "旧进程结果"),
        Err(CoreError::StaleRun)
    ));
    drop(reopened);
    assert_eq!(Store::open(&file.path).unwrap().snapshot(), state);
}

#[test]
fn future_database_version_is_not_overwritten() {
    let file = TestDb::new();
    {
        let db = SqlConnection::open(&file.path).unwrap();
        db.execute_batch("CREATE TABLE future_data (value TEXT); INSERT INTO future_data VALUES ('keep-me'); PRAGMA user_version=99;").unwrap();
    }
    assert!(matches!(
        Store::open(&file.path),
        Err(CoreError::UnsupportedSchema(99))
    ));
    let db = SqlConnection::open(&file.path).unwrap();
    let retained: String = db
        .query_row("SELECT value FROM future_data", [], |r| r.get(0))
        .unwrap();
    assert_eq!(retained, "keep-me");
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn malformed_or_unknown_snapshot_schema_is_not_reset() {
    for replacement in [
        "{bad-json".to_string(),
        "{\"schemaVersion\":99}".to_string(),
    ] {
        let file = TestDb::new();
        drop(Store::open(&file.path).unwrap());
        {
            let db = SqlConnection::open(&file.path).unwrap();
            db.execute("UPDATE app_state SET payload=?1", [&replacement])
                .unwrap();
        }
        assert!(Store::open(&file.path).is_err());
        let db = SqlConnection::open(&file.path).unwrap();
        let retained: String = db
            .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(retained, replacement);
    }
}

#[test]
fn unrecognized_existing_database_is_not_initialized() {
    let file = TestDb::new();
    {
        let db = SqlConnection::open(&file.path).unwrap();
        db.execute_batch(
            "CREATE TABLE other_app (value TEXT); INSERT INTO other_app VALUES ('existing');",
        )
        .unwrap();
    }
    assert!(matches!(
        Store::open(&file.path),
        Err(CoreError::Invalid(_))
    ));
    let db = SqlConnection::open(&file.path).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM other_app", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "existing"
    );
}

#[test]
fn connection_metadata_never_accepts_secret_fields_or_credential_urls() {
    let secret = "sk-example-private-secret";
    let injected = serde_json::json!({ "baseUrl": "https://example.com/v1", "model": "test", "hasKey": true, "apiKey": secret });
    assert!(serde_json::from_value::<Connection>(injected).is_err());
    let mut store = Store::open_in_memory().unwrap();
    for url in [
        format!("https://{secret}@example.com/v1"),
        format!("https://example.com/v1?key={secret}"),
    ] {
        assert!(store
            .save_connection(Connection {
                base_url: url,
                model: "test".into(),
                has_key: true
            })
            .is_err());
    }
    store
        .save_connection(Connection {
            base_url: "https://example.com/v1".into(),
            model: "test".into(),
            has_key: true,
        })
        .unwrap();
    let serialized = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(!serialized.contains(secret));
    assert!(!serialized.contains("apiKey"));
    assert!(serialized.contains("\"hasKey\":true"));
}

#[test]
fn invalid_region_update_is_atomic_and_valid_update_preserves_reference() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.snapshot().active_scene_id;
    store.set_background(&id, image()).unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: id.clone(),
            region: region("r"),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: "r".into(),
            }],
        })
        .unwrap();
    let before = store.snapshot();
    let mut edited = region("r");
    edited.width = 9999.0;
    assert!(store
        .apply(SceneCommand::UpdateRegion {
            scene_id: id.clone(),
            region: edited.clone()
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    edited.width = 400.0;
    let updated = store
        .apply(SceneCommand::UpdateRegion {
            scene_id: id.clone(),
            region: edited,
        })
        .unwrap();
    assert_eq!(updated.scenes[0].refs[0].id, "r");
    assert_eq!(updated.scenes[0].regions[0].width, 400.0);
    assert!(store
        .apply(SceneCommand::UpdateRegion {
            scene_id: id,
            region: region("missing")
        })
        .is_err());
}

#[test]
fn run_input_is_immutable_and_switching_agent_requires_fresh_context() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let original_agent = store.snapshot().agents[0].clone();
    let other = AgentProfile {
        id: Uuid::new_v4().to_string(),
        name: "研究员".into(),
        instructions: "研究".into(),
        memory: "手动资料".into(),
        memory_enabled: false,
        memory_provider: Default::default(),
        default_connection_id: None,
    };
    store
        .apply(SceneCommand::SaveAgent {
            agent: other.clone(),
        })
        .unwrap();
    draft(&mut store, &scene_id, "开始");
    let context = store.begin_run(&scene_id).unwrap();
    assert!(matches!(
        store.apply(SceneCommand::SetAgent {
            scene_id: scene_id.clone(),
            agent_id: other.id.clone()
        }),
        Err(CoreError::RunInProgress)
    ));
    let mut edited = original_agent.clone();
    edited.instructions = "下一次运行使用".into();
    store
        .apply(SceneCommand::SaveAgent { agent: edited })
        .unwrap();
    assert_eq!(context.agent, original_agent);
    store
        .finish_run(&scene_id, &context.run_id, "完成")
        .unwrap();
    assert!(matches!(
        store.apply(SceneCommand::SetAgent {
            scene_id: scene_id.clone(),
            agent_id: other.id.clone()
        }),
        Err(CoreError::AgentContextConflict)
    ));
    let next = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    store
        .apply(SceneCommand::SetAgent {
            scene_id: next,
            agent_id: other.id,
        })
        .unwrap();
}

#[test]
fn second_writer_cannot_silently_overwrite_changes() {
    let file = TestDb::new();
    let mut first = Store::open(&file.path).unwrap();
    let mut second = Store::open(&file.path).unwrap();
    let id = first.snapshot().active_scene_id;
    let second_before = second.snapshot();
    draft(&mut first, &id, "first writer");
    assert!(matches!(
        second.apply(SceneCommand::SetDraft {
            scene_id: id,
            draft: "second writer".into()
        }),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(second.snapshot(), second_before);
    drop(first);
    drop(second);
    assert_eq!(
        Store::open(&file.path).unwrap().snapshot().scenes[0].draft,
        "first writer"
    );
}

#[test]
fn scene_command_json_matches_the_typescript_contract() {
    let command: SceneCommand = serde_json::from_value(serde_json::json!({
        "type": "update_region", "sceneId": "scene", "region": { "id": "r", "x": 0, "y": 0, "width": 100, "height": 100 }
    })).unwrap();
    assert!(matches!(command, SceneCommand::UpdateRegion { .. }));
    let value = serde_json::to_value(SceneCommand::SetAgent {
        scene_id: "scene".into(),
        agent_id: "agent".into(),
    })
    .unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "type": "set_agent", "sceneId": "scene", "agentId": "agent" })
    );
}

#[test]
fn historical_images_survive_region_removal_background_replacement_and_restart() {
    let file = TestDb::new();
    let historical_image = image();
    let scene_id;
    {
        let mut store = Store::open(&file.path).unwrap();
        scene_id = store.snapshot().active_scene_id;
        store.set_background(&scene_id, image()).unwrap();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene_id.clone(),
                region: region("original-region"),
            })
            .unwrap();
        store
            .apply(SceneCommand::SetRefs {
                scene_id: scene_id.clone(),
                refs: vec![Reference {
                    kind: ReferenceKind::Region,
                    id: "original-region".into(),
                }],
            })
            .unwrap();
        draft(&mut store, &scene_id, "观察旧画面");
        let run = store.begin_run(&scene_id).unwrap();
        store
            .attach_run_assets(&scene_id, &run.run_id, vec![historical_image.clone()])
            .unwrap();
        // Same registration can be retried without changing the immutable capture.
        store
            .attach_run_assets(&scene_id, &run.run_id, vec![historical_image.clone()])
            .unwrap();
        assert!(store
            .attach_run_assets(&scene_id, &run.run_id, vec![image()])
            .is_err());
        store
            .finish_run(&scene_id, &run.run_id, "旧画面结果")
            .unwrap();
        store
            .apply(SceneCommand::RemoveRegion {
                scene_id: scene_id.clone(),
                region_id: "original-region".into(),
            })
            .unwrap();
        store.set_background(&scene_id, image()).unwrap();
    }
    let mut restored = Store::open(&file.path).unwrap();
    draft(&mut restored, &scene_id, "和之前比较");
    let new_run = restored.begin_run(&scene_id).unwrap();
    assert_eq!(
        new_run.messages[0].attachments.as_ref().unwrap(),
        &vec![historical_image]
    );
    assert!(new_run.messages.last().unwrap().attachments.is_none());
    assert!(new_run.regions.is_empty());
}

#[test]
fn historical_documents_survive_reference_clearing_background_replacement_and_restart() {
    let file = TestDb::new();
    let html = Asset {
        id: Uuid::new_v4().to_string(),
        name: "历史.html".into(),
        kind: AssetKind::Html,
        path: "assets/history.html".into(),
        width: None,
        height: None,
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    let svg = Asset {
        id: Uuid::new_v4().to_string(),
        name: "历史.svg".into(),
        kind: AssetKind::Svg,
        path: "assets/history.svg".into(),
        ..html.clone()
    };
    let attachments = vec![image(), html, svg];
    let scene_id;
    let original_refs;
    {
        let mut store = Store::open(&file.path).unwrap();
        scene_id = store.snapshot().active_scene_id;
        store.set_background(&scene_id, image()).unwrap();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene_id.clone(),
                region: region("original-region"),
            })
            .unwrap();
        for document in &attachments[1..] {
            store.add_asset(&scene_id, document.clone()).unwrap();
        }
        let item_ids: Vec<String> = store.snapshot().scenes[0]
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect();
        let mut refs = vec![Reference {
            kind: ReferenceKind::Region,
            id: "original-region".into(),
        }];
        refs.extend(item_ids.iter().map(|id| Reference {
            kind: ReferenceKind::Item,
            id: id.clone(),
        }));
        original_refs = refs.clone();
        store
            .apply(SceneCommand::SetRefs {
                scene_id: scene_id.clone(),
                refs,
            })
            .unwrap();
        draft(&mut store, &scene_id, "分析这些文档和截图");
        let run = store.begin_run(&scene_id).unwrap();
        store
            .attach_run_assets(&scene_id, &run.run_id, attachments.clone())
            .unwrap();
        let bound = store
            .attach_run_assets(&scene_id, &run.run_id, attachments.clone())
            .unwrap();
        let mut replacement = attachments.clone();
        replacement[1].path = "assets/replaced.html".into();
        assert!(store
            .attach_run_assets(&scene_id, &run.run_id, replacement)
            .is_err());
        assert_eq!(store.snapshot(), bound);
        store.finish_run(&scene_id, &run.run_id, "已分析").unwrap();
        store
            .apply(SceneCommand::SetRefs {
                scene_id: scene_id.clone(),
                refs: vec![],
            })
            .unwrap();
        for item_id in item_ids {
            store
                .apply(SceneCommand::RemoveItem {
                    scene_id: scene_id.clone(),
                    item_id,
                })
                .unwrap();
        }
        store.set_background(&scene_id, image()).unwrap();
        let changed = store.snapshot();
        assert_eq!(
            changed.scenes[0].messages[0].attachments.as_ref().unwrap(),
            &attachments
        );
        assert!(changed.scenes[0].refs.is_empty());
        assert!(changed.scenes[0].regions.is_empty());
        assert!(changed.scenes[0].items.is_empty());
    }
    let mut restored = Store::open(&file.path).unwrap();
    let reopened = restored.snapshot();
    assert_eq!(
        reopened.scenes[0].messages[0].attachments.as_ref().unwrap(),
        &attachments
    );
    assert_eq!(
        reopened.scenes[0].messages[0].refs.as_ref().unwrap(),
        &original_refs
    );
    draft(&mut restored, &scene_id, "继续讨论之前的文档");
    let next_run = restored.begin_run(&scene_id).unwrap();
    assert_eq!(
        next_run.messages[0].attachments.as_ref().unwrap(),
        &attachments
    );
    assert!(next_run.messages.last().unwrap().attachments.is_none());
    assert!(next_run.refs.is_empty());
    assert!(next_run.regions.is_empty());
    assert!(next_run.items.is_empty());
}

#[test]
fn invalid_and_late_attachment_registration_do_not_mutate_messages() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "图片");
    let run = store.begin_run(&scene_id).unwrap();
    let before = store.snapshot();
    let mut unsupported_file = image();
    unsupported_file.kind = AssetKind::File;
    assert!(store
        .attach_run_assets(&scene_id, &run.run_id, vec![unsupported_file])
        .is_err());
    assert!(store
        .attach_run_assets(&scene_id, &run.run_id, (0..7).map(|_| image()).collect())
        .is_err());
    assert_eq!(store.snapshot(), before);
    let canceled = store.cancel_run(&scene_id).unwrap();
    assert!(matches!(
        store.attach_run_assets(&scene_id, &run.run_id, vec![image()]),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(store.snapshot(), canceled);
}

#[test]
fn generated_assets_and_reply_commit_atomically_without_activating_background_scene() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store.snapshot().active_scene_id;
    draft(&mut store, &a, "创建互动内容");
    let run = store.begin_run(&a).unwrap();
    let b = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap()
        .active_scene_id;
    let before = store.snapshot();
    let mut broken = image();
    broken.path.clear();
    assert!(store
        .finish_run_with_assets(&a, &run.run_id, "结果", vec![image(), broken])
        .is_err());
    assert_eq!(store.snapshot(), before);
    let mut generated = image();
    generated.kind = AssetKind::Html;
    generated.path = "assets/generated.html".into();
    let asset_id = generated.id.clone();
    let completed = store
        .finish_run_with_assets(&a, &run.run_id, "结果", vec![generated])
        .unwrap();
    assert_eq!(completed.active_scene_id, b);
    assert!(completed.scenes[0].frozen);
    assert_eq!(completed.scenes[0].items[0].asset.id, asset_id);
    assert_eq!(completed.scenes[0].messages.last().unwrap().text, "结果");
    assert_eq!(
        completed.scenes[0].run.as_ref().unwrap().status,
        RunStatus::Completed
    );
    assert!(completed.scenes[1].items.is_empty());
}
