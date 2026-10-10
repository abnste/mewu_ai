// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection as SqlConnection;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb {
    directory: PathBuf,
    path: PathBuf,
}
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-close-test-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self {
            path: directory.join("scenes.sqlite3"),
            directory,
        }
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
fn draft(store: &mut Store, scene: &str, text: &str) {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.into(),
            draft: text.into(),
        })
        .unwrap();
}
fn start(store: &mut Store, scene: &str, text: &str) -> RunContext {
    draft(store, scene, text);
    store.begin_run(scene).unwrap()
}
fn scene<'a>(state: &'a Snapshot, id: &str) -> &'a Scene {
    state.scenes.iter().find(|scene| scene.id == id).unwrap()
}
fn image() -> Asset {
    Asset {
        id: Uuid::new_v4().to_string(),
        name: "保留的截图.png".into(),
        kind: AssetKind::Image,
        path: "assets/fixture.png".into(),
        width: Some(100),
        height: Some(100),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn payload(db: &SqlConnection) -> String {
    db.query_row("SELECT payload FROM app_state WHERE id = 1", [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn closing_active_retains_everything_and_creates_same_agent_blank_scene() {
    let mut store = Store::open_in_memory().unwrap();
    let original = store.snapshot().active_scene_id;
    let active = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    let agent = AgentProfile {
        id: Uuid::new_v4().to_string(),
        name: "研究员".into(),
        instructions: String::new(),
        memory: String::new(),
        memory_enabled: true,
        memory_provider: Default::default(),
        default_connection_id: None,
    };
    store
        .apply(SceneCommand::SaveAgent {
            agent: agent.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetAgent {
            scene_id: active.clone(),
            agent_id: agent.id.clone(),
        })
        .unwrap();
    store.set_background(&active, image()).unwrap();
    store.add_asset(&active, image()).unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: active.clone(),
            region: Region {
                id: "region".into(),
                x: 1.0,
                y: 2.0,
                width: 20.0,
                height: 30.0,
                ..Default::default()
            },
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: active.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: "region".into(),
            }],
        })
        .unwrap();
    let run = start(&mut store, &active, "这段历史要保留");
    store
        .attach_run_assets(&active, &run.run_id, vec![image()])
        .unwrap();
    store
        .begin_tool(
            &active,
            &run.run_id,
            "pending",
            "history_search",
            "查找历史",
        )
        .unwrap();
    draft(&mut store, &active, "下一条草稿");
    let before = store.snapshot();
    let closed = store
        .apply(SceneCommand::CloseScene {
            scene_id: active.clone(),
        })
        .unwrap();
    let old = scene(&closed, &active);
    let prior = scene(&before, &active);
    assert!(old.closed && old.frozen);
    assert_eq!(old.agent_id, agent.id);
    assert_eq!(old.background, prior.background);
    assert_eq!(old.regions, prior.regions);
    assert_eq!(old.items, prior.items);
    assert_eq!(old.refs, prior.refs);
    assert_eq!(old.draft, prior.draft);
    assert_eq!(old.messages[0].id, prior.messages[0].id);
    assert_eq!(old.messages[0].text, prior.messages[0].text);
    assert_eq!(old.messages[0].attachments, prior.messages[0].attachments);
    assert_eq!(old.run.as_ref().unwrap().status, RunStatus::Canceled);
    assert_eq!(
        old.run.as_ref().unwrap().error.as_deref(),
        Some("会话已关闭")
    );
    assert_eq!(old.messages[0].tool_steps[0].status, ToolStepStatus::Failed);
    assert_eq!(
        old.messages[0].tool_steps[0].summary.as_deref(),
        Some("会话已关闭")
    );
    assert!(!store.run_is_active(&active, &run.run_id));
    assert_ne!(closed.active_scene_id, active);
    assert_ne!(closed.active_scene_id, original);
    assert!(scene(&closed, &original).frozen);
    let fresh = scene(&closed, &closed.active_scene_id);
    assert_eq!(fresh.agent_id, agent.id);
    assert!(!fresh.closed && !fresh.frozen);
    assert!(fresh.messages.is_empty() && fresh.items.is_empty() && fresh.draft.is_empty());
    assert_eq!(
        store
            .apply(SceneCommand::CloseScene { scene_id: active })
            .unwrap(),
        closed
    );
}

#[test]
fn closing_frozen_run_leaves_active_alone_and_rejects_mcp_and_memory_late_results() {
    let mut store = Store::open_in_memory().unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    store
        .apply(SceneCommand::SaveAgent {
            agent: agent.clone(),
        })
        .unwrap();
    let server = store
        .upsert_mcp_server(
            None,
            None,
            "工具".into(),
            McpCommand {
                executable: "/fixture/server".into(),
                args: vec![],
                cwd: None,
            },
            vec![McpTool {
                name: "read".into(),
                description: String::new(),
                input_schema: json!({"type":"object"}),
            }],
        )
        .unwrap()
        .mcp_servers[0]
        .clone();
    let server = store
        .apply(SceneCommand::SetMcpGrants {
            server_id: server.id.clone(),
            expected_revision: server.revision,
            agent_id: agent.id,
            tool_names: vec!["read".into()],
        })
        .unwrap()
        .mcp_servers[0]
        .clone();
    let a = store.snapshot().active_scene_id;
    let run_a = start(&mut store, &a, "关键词属于历史");
    store
        .begin_mcp_tool(
            &a,
            &run_a.run_id,
            "call",
            &server.id,
            server.revision,
            "read",
        )
        .unwrap();
    let b = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap()
        .active_scene_id;
    let run_b = start(&mut store, &b, "另一个场景继续");
    let before = store.snapshot();
    let state = store
        .apply(SceneCommand::CloseScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert_eq!(state.active_scene_id, b);
    assert_eq!(state.scenes.len(), before.scenes.len());
    assert_eq!(scene(&state, &b), scene(&before, &b));
    assert!(store.run_is_active(&b, &run_b.run_id));
    assert!(matches!(
        store.mcp_for_run(&a, &run_a.run_id, &server.id, server.revision, "read"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.finish_tool(&a, &run_a.run_id, "call", ToolStepStatus::Completed, "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.finish_run(&a, &run_a.run_id, "迟到回复"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.memory_for_run(
            &a,
            &run_a.run_id,
            MemoryMutation::Save {
                id: None,
                text: "迟到记忆".into(),
                expected_revision: None
            }
        ),
        Err(CoreError::StaleRun)
    ));
    let history = store
        .search_history_for_run(&b, &run_b.run_id, "关键词")
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].scene_id, a);
    assert!(store.begin_run(&a).is_err());
    assert!(store
        .apply(SceneCommand::FreezeScene { scene_id: a })
        .is_err());
    assert_eq!(store.snapshot(), state);
}

#[test]
fn restore_closed_scene_keeps_old_run_canceled_until_a_new_message() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store.snapshot().active_scene_id;
    let old = start(&mut store, &a, "之前的消息");
    draft(&mut store, &a, "未发送内容");
    let closed = store
        .apply(SceneCommand::CloseScene {
            scene_id: a.clone(),
        })
        .unwrap();
    let b = closed.active_scene_id;
    let restored = store
        .apply(SceneCommand::ActivateScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert_eq!(restored.active_scene_id, a);
    assert!(!scene(&restored, &a).closed && !scene(&restored, &a).frozen);
    assert!(scene(&restored, &b).frozen);
    assert_eq!(
        scene(&restored, &a).run.as_ref().unwrap().status,
        RunStatus::Canceled
    );
    assert!(!store.run_is_active(&a, &old.run_id));
    assert_eq!(scene(&restored, &a).draft, "未发送内容");
    let next = store.begin_run(&a).unwrap();
    assert_ne!(next.run_id, old.run_id);
    assert!(store.run_is_active(&a, &next.run_id));
    assert_eq!(next.messages.len(), 2);
}

#[test]
fn completed_run_and_history_survive_close_and_restart() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "问题");
    store.finish_run(&a, &run.run_id, "已经完成的回答").unwrap();
    let state = store
        .apply(SceneCommand::CloseScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert_eq!(
        scene(&state, &a).run.as_ref().unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(scene(&state, &a).messages.len(), 2);
    drop(store);
    let mut reopened = Store::open(&temp.path).unwrap();
    assert_eq!(reopened.snapshot(), state);
    assert_eq!(
        reopened
            .apply(SceneCommand::CloseScene {
                scene_id: a.clone()
            })
            .unwrap(),
        state
    );
    let restored = reopened
        .apply(SceneCommand::ActivateScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert!(!scene(&restored, &a).closed);
    assert_eq!(scene(&restored, &a).messages, scene(&state, &a).messages);
}

#[test]
fn older_snapshot_missing_closed_defaults_false_without_changing_history() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "兼容旧记录");
    store.finish_run(&a, &run.run_id, "原来的回答").unwrap();
    store.apply(SceneCommand::NewScene).unwrap();
    let state = store.snapshot();
    drop(store);
    let db = SqlConnection::open(&temp.path).unwrap();
    let mut old: serde_json::Value = serde_json::from_str(&payload(&db)).unwrap();
    for scene in old["scenes"].as_array_mut().unwrap() {
        scene.as_object_mut().unwrap().remove("closed");
    }
    db.execute(
        "UPDATE app_state SET payload = ?1",
        [serde_json::to_string(&old).unwrap()],
    )
    .unwrap();
    let restored = Store::open(&temp.path).unwrap().snapshot();
    assert_eq!(restored, state);
    assert!(restored.scenes.iter().all(|scene| !scene.closed));
}

#[test]
fn failed_close_commit_does_not_cancel_archive_or_replace_active_scene() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "失败事务");
    store
        .begin_tool(&a, &run.run_id, "call", "fixture", "处理中")
        .unwrap();
    let before = store.snapshot();
    let db = SqlConnection::open(&temp.path).unwrap();
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER fail_close BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.apply(SceneCommand::CloseScene {
            scene_id: a.clone()
        }),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    assert!(store.run_is_active(&a, &run.run_id));
    db.execute_batch("DROP TRIGGER fail_close;").unwrap();
    let closed = store
        .apply(SceneCommand::CloseScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert!(scene(&closed, &a).closed);
    assert_eq!(
        scene(&closed, &a).messages[0].tool_steps[0].status,
        ToolStepStatus::Failed
    );
}

#[test]
fn invalid_closed_active_or_running_snapshot_is_not_silently_recovered() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let a = store.snapshot().active_scene_id;
    start(&mut store, &a, "不能关闭后继续运行");
    store.apply(SceneCommand::NewScene).unwrap();
    drop(store);
    let db = SqlConnection::open(&temp.path).unwrap();
    let base: serde_json::Value = serde_json::from_str(&payload(&db)).unwrap();
    for index in [0, 1] {
        let mut invalid = base.clone();
        invalid["scenes"][index]["closed"] = json!(true);
        let text = serde_json::to_string(&invalid).unwrap();
        db.execute("UPDATE app_state SET payload = ?1", [&text])
            .unwrap();
        assert!(Store::open(&temp.path).is_err());
        assert_eq!(payload(&db), text);
    }
}
