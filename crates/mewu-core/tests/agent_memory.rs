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
        let directory = std::env::temp_dir().join(format!("mewu-memory-test-{}", Uuid::new_v4()));
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

fn enable(store: &mut Store, enabled: bool) -> String {
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = enabled;
    let id = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    id
}
fn start(store: &mut Store, scene_id: &str, text: &str) -> RunContext {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.into(),
            draft: text.into(),
        })
        .unwrap();
    store.begin_run(scene_id).unwrap()
}
fn save(text: &str) -> MemoryMutation {
    MemoryMutation::Save {
        id: None,
        text: text.into(),
        expected_revision: None,
    }
}
fn manual(store: &mut Store, agent_id: &str, text: &str) -> Snapshot {
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent_id.into(),
            id: None,
            text: text.into(),
            expected_revision: None,
        })
        .unwrap()
}
fn other_agent(store: &mut Store) -> AgentProfile {
    let agent = AgentProfile {
        id: Uuid::new_v4().to_string(),
        name: "另一个 Agent".into(),
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
    agent
}
fn payload(db: &SqlConnection) -> String {
    db.query_row("SELECT payload FROM app_state WHERE id = 1", [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn entries(store: &Store, agent: &str) -> Vec<MemoryEntry> {
    store.memory_page(agent, "", None, 25).unwrap().entries
}

#[test]
fn older_fields_default_to_enabled_without_reimporting_memory_or_writing_on_open() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory = "旧的手写资料".into();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene_id, "旧消息");
    store.finish_run(&scene_id, &run.run_id, "旧回答").unwrap();
    drop(store);
    let db = SqlConnection::open(&temp.path).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&payload(&db)).unwrap();
    value.as_object_mut().unwrap().remove("memories");
    for agent in value["agents"].as_array_mut().unwrap() {
        agent.as_object_mut().unwrap().remove("memoryEnabled");
    }
    for scene in value["scenes"].as_array_mut().unwrap() {
        scene["run"].as_object_mut().unwrap().remove("steps");
        for message in scene["messages"].as_array_mut().unwrap() {
            message.as_object_mut().unwrap().remove("toolSteps");
        }
    }
    db.execute(
        "UPDATE app_state SET payload = ?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    let raw_before_open = payload(&db);
    drop(db);
    let mut reopened = Store::open(&temp.path).unwrap();
    let state = reopened.snapshot();
    assert!(state.memories.is_empty());
    assert!(state.agents[0].memory_enabled);
    let db = SqlConnection::open(&temp.path).unwrap();
    assert_eq!(payload(&db), raw_before_open);
    drop(db);
    assert_eq!(state.agents[0].memory, "旧的手写资料");
    assert!(state.scenes[0].run.as_ref().unwrap().steps.is_empty());
    assert!(state.scenes[0]
        .messages
        .iter()
        .all(|message| message.tool_steps.is_empty()));
    let next = start(&mut reopened, &scene_id, "新消息");
    assert!(next.memories.is_empty());
    reopened
        .memory_for_run(&scene_id, &next.run_id, save("本轮明确保存"))
        .unwrap();
    assert_eq!(entries(&reopened, &state.agents[0].id).len(), 1);
    assert_eq!(
        entries(&reopened, &state.agents[0].id)[0].text,
        "本轮明确保存"
    );
}

#[test]
fn new_install_enables_memory_and_explicit_disable_survives_reopen() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    assert!(store.snapshot().agents[0].memory_enabled);
    let id = enable(&mut store, false);
    let disabled = store.snapshot();
    drop(store);
    let mut store = Store::open(&temp.path).unwrap();
    assert_eq!(store.snapshot(), disabled);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "关闭后仍关闭");
    assert!(matches!(
        store.memory_for_run(&scene, &run.run_id, save("不能保存")),
        Err(CoreError::MemoryDisabled)
    ));
    assert!(entries(&store, &id).is_empty());
}

#[test]
fn memory_is_agent_scoped_with_internal_provenance_and_immutable_run_input() {
    let mut store = Store::open_in_memory().unwrap();
    let agent_id = enable(&mut store, true);
    manual(&mut store, &agent_id, "手动资料一");
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "手动资料一，请记住偏好");
    let before = run.memories.clone();
    let snapshot = store
        .memory_for_run(&a, &run.run_id, save("偏好中文"))
        .unwrap();
    assert!(snapshot.memories.is_empty());
    let memory = entries(&store, &agent_id)
        .into_iter()
        .find(|m| m.text == "偏好中文")
        .unwrap();
    assert_eq!(memory.agent_id, agent_id);
    assert_eq!(
        memory.source,
        Some(MemorySource {
            scene_id: a.clone(),
            message_id: run.messages.last().unwrap().id.clone()
        })
    );
    assert_eq!(run.memories, before);
    assert_eq!(run.memories.len(), 1);
    assert_eq!(store.memories_for_run(&a, &run.run_id).unwrap().len(), 2);
    let frozen = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap();
    let b = frozen.active_scene_id;
    assert_eq!(frozen.scenes.last().unwrap().agent_id, agent_id);
    let other = other_agent(&mut store);
    store
        .apply(SceneCommand::SetAgent {
            scene_id: b.clone(),
            agent_id: other.id.clone(),
        })
        .unwrap();
    let second = start(&mut store, &b, "另一个身份");
    assert!(second.memories.is_empty());
    assert!(store
        .memories_for_run(&b, &second.run_id)
        .unwrap()
        .is_empty());
    let unchanged = store.snapshot();
    for mutation in [
        MemoryMutation::Save {
            id: Some(memory.id.clone()),
            text: "偷改".into(),
            expected_revision: Some(1),
        },
        MemoryMutation::Delete {
            id: memory.id,
            expected_revision: 1,
        },
    ] {
        assert!(matches!(
            store.memory_for_run(&b, &second.run_id, mutation),
            Err(CoreError::NotFound { .. })
        ));
        assert_eq!(store.snapshot(), unchanged);
    }
    assert!(serde_json::from_value::<MemoryMutation>(
        serde_json::json!({"type":"save","text":"伪造","source":{"sceneId":a,"messageId":"fake"}})
    )
    .is_err());
    assert!(matches!(
        store.apply(SceneCommand::SetAgent {
            scene_id: a.clone(),
            agent_id: other.id
        }),
        Err(CoreError::RunInProgress)
    ));
    store
        .memory_for_run(&a, &run.run_id, save("冻结后仍归原 Agent"))
        .unwrap();
    assert_eq!(store.snapshot().active_scene_id, b);
}

#[test]
fn compare_and_swap_conflicts_preserve_manual_and_tool_edits() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store, true);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "记住");
    store
        .memory_for_run(&scene, &run.run_id, save("第一版"))
        .unwrap();
    let entry = entries(&store, &agent)[0].clone();
    let manual_state = store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: Some(entry.id.clone()),
            text: "第二版".into(),
            expected_revision: Some(1),
        })
        .unwrap();
    assert_eq!(entries(&store, &agent)[0].revision, 2);
    assert!(entries(&store, &agent)[0].source.is_none());
    for revision in [None, Some(1)] {
        assert!(matches!(
            store.memory_for_run(
                &scene,
                &run.run_id,
                MemoryMutation::Save {
                    id: Some(entry.id.clone()),
                    text: "过期覆盖".into(),
                    expected_revision: revision
                }
            ),
            Err(CoreError::MemoryConflict)
        ));
    }
    assert!(matches!(
        store.apply(SceneCommand::DeleteMemory {
            agent_id: agent.clone(),
            id: entry.id.clone(),
            expected_revision: 1
        }),
        Err(CoreError::MemoryConflict)
    ));
    assert_eq!(store.snapshot(), manual_state);
    store
        .memory_for_run(
            &scene,
            &run.run_id,
            MemoryMutation::Save {
                id: Some(entry.id.clone()),
                text: "第三版".into(),
                expected_revision: Some(2),
            },
        )
        .unwrap();
    assert_eq!(entries(&store, &agent)[0].revision, 3);
    assert!(entries(&store, &agent)[0].source.is_some());
    store
        .apply(SceneCommand::DeleteMemory {
            agent_id: agent.clone(),
            id: entry.id,
            expected_revision: 3,
        })
        .unwrap();
    assert!(store.snapshot().memories.is_empty());
    assert!(entries(&store, &agent).is_empty());
}

#[test]
fn disabling_memory_revokes_running_tools_without_deleting_data() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store, true);
    manual(&mut store, &agent, "保留的记忆");
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "保留的记忆");
    assert_eq!(run.memories.len(), 1);
    enable(&mut store, false);
    assert!(matches!(
        store.memory_for_run(&scene, &run.run_id, save("被撤销")),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.memories_for_run(&scene, &run.run_id),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.search_history_for_run(&scene, &run.run_id, "保留"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(entries(&store, &agent).len(), 1);
    assert!(matches!(
        store.finish_run(&scene, &run.run_id, "迟到"),
        Err(CoreError::StaleRun)
    ));
    let next = start(&mut store, &scene, "记忆关闭的新轮次");
    assert!(next.memories.is_empty());
    assert_eq!(next.agent.id, agent);
    assert!(matches!(
        store.memories_for_run(&scene, &next.run_id),
        Err(CoreError::MemoryDisabled)
    ));
}

#[test]
fn disabling_memory_cancels_all_identity_runs_including_frozen_but_not_other_agent() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store, true);
    let a = store.snapshot().active_scene_id;
    let run_a = start(&mut store, &a, "首个运行");
    store
        .begin_tool(&a, &run_a.run_id, "a", "memory_list", "读取记忆")
        .unwrap();
    let b = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    let run_b = start(&mut store, &b, "同身份第二个运行");
    store
        .begin_tool(&b, &run_b.run_id, "b", "history_search", "搜索历史")
        .unwrap();
    let c = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    let other = other_agent(&mut store);
    store
        .apply(SceneCommand::SetAgent {
            scene_id: c.clone(),
            agent_id: other.id,
        })
        .unwrap();
    let run_c = start(&mut store, &c, "另一身份运行");
    store
        .begin_tool(&c, &run_c.run_id, "c", "memory_list", "读取记忆")
        .unwrap();
    enable(&mut store, false);
    let state = store.snapshot();
    assert_eq!(state.active_scene_id, c);
    for scene in state.scenes.iter().filter(|scene| scene.agent_id == agent) {
        let run = scene.run.as_ref().unwrap();
        assert!(scene.frozen);
        assert_eq!(run.status, RunStatus::Canceled);
        assert_eq!(run.error.as_deref(), Some("长期记忆已关闭"));
        assert_eq!(run.steps[0].status, ToolStepStatus::Failed);
        assert_eq!(scene.messages[0].tool_steps, run.steps);
    }
    assert_eq!(
        state.scenes[2].run.as_ref().unwrap().status,
        RunStatus::Running
    );
    assert_eq!(
        state.scenes[2].run.as_ref().unwrap().steps[0].status,
        ToolStepStatus::Running
    );
    store
        .memory_for_run(&c, &run_c.run_id, save("另一身份不受影响"))
        .unwrap();
}

#[test]
fn revocation_database_failure_does_not_disable_agent_or_half_cancel_runs() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    enable(&mut store, true);
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "事务撤权");
    store
        .begin_tool(&a, &run.run_id, "a", "memory_list", "读取记忆")
        .unwrap();
    let b = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    start(&mut store, &b, "第二个事务撤权");
    let before = store.snapshot();
    let mut disabled = before.agents[0].clone();
    disabled.memory_enabled = false;
    let db = SqlConnection::open(&temp.path).unwrap();
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER fail_revoke BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.apply(SceneCommand::SaveAgent {
            agent: disabled.clone()
        }),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    db.execute_batch("DROP TRIGGER fail_revoke;").unwrap();
    store
        .apply(SceneCommand::SaveAgent { agent: disabled })
        .unwrap();
    assert!(store
        .snapshot()
        .scenes
        .iter()
        .all(|scene| scene.run.as_ref().unwrap().status == RunStatus::Canceled));
}

#[test]
fn frozen_tools_remain_scoped_and_cancel_rejects_late_results() {
    let mut store = Store::open_in_memory().unwrap();
    enable(&mut store, true);
    let a = store.snapshot().active_scene_id;
    let run = start(&mut store, &a, "后台运行");
    store
        .begin_tool(&a, &run.run_id, "call-1", "memory_save", "保存记忆")
        .unwrap();
    assert!(store
        .begin_tool(&a, &run.run_id, "call-1", "memory_save", "重复执行")
        .is_err());
    let b = store
        .apply(SceneCommand::FreezeScene {
            scene_id: a.clone(),
        })
        .unwrap()
        .active_scene_id;
    store
        .finish_tool(
            &a,
            &run.run_id,
            "call-1",
            ToolStepStatus::Completed,
            "记忆已保存",
        )
        .unwrap();
    store
        .begin_tool(&a, &run.run_id, "call-2", "history_search", "查询历史")
        .unwrap();
    let canceled = store.cancel_run(&a).unwrap();
    assert_eq!(canceled.active_scene_id, b);
    let old = &canceled.scenes[0];
    assert_eq!(
        old.run.as_ref().unwrap().steps[0].status,
        ToolStepStatus::Completed
    );
    assert_eq!(
        old.run.as_ref().unwrap().steps[1].status,
        ToolStepStatus::Failed
    );
    assert_eq!(old.messages[0].tool_steps, old.run.as_ref().unwrap().steps);
    assert!(matches!(
        store.finish_tool(&a, &run.run_id, "call-2", ToolStepStatus::Completed, "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.memory_for_run(&a, &run.run_id, save("迟到")),
        Err(CoreError::StaleRun)
    ));
    assert!(matches!(
        store.memories_for_run(&a, &run.run_id),
        Err(CoreError::StaleRun)
    ));
    let next = start(&mut store, &a, "下一轮");
    assert!(next.messages[0].tool_steps.len() == 2);
    assert!(matches!(
        store.begin_tool(&a, &run.run_id, "late", "memory_save", "迟到"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(store.snapshot().active_scene_id, b);
}

#[test]
fn terminal_runs_archive_steps_and_recovery_persists_interrupted_trace() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let scene = store.snapshot().active_scene_id;
    for fail in [false, true] {
        let run = start(&mut store, &scene, "保留步骤");
        store
            .begin_tool(&scene, &run.run_id, "pending", "fixture", "进行中")
            .unwrap();
        if fail {
            store.fail_run(&scene, &run.run_id, "受控失败").unwrap();
        } else {
            store.finish_run(&scene, &run.run_id, "完成回复").unwrap();
        }
        let state = store.snapshot();
        let user = state.scenes[0]
            .messages
            .iter()
            .find(|message| {
                message.run_id.as_deref() == Some(&run.run_id) && message.role == MessageRole::User
            })
            .unwrap();
        assert_eq!(user.tool_steps[0].status, ToolStepStatus::Failed);
    }
    let interrupted = start(&mut store, &scene, "重启前");
    store
        .begin_tool(
            &scene,
            &interrupted.run_id,
            "interrupted",
            "fixture",
            "等待",
        )
        .unwrap();
    drop(store);
    let reopened = Store::open(&temp.path).unwrap();
    let state = reopened.snapshot();
    let current = state.scenes[0].run.as_ref().unwrap();
    assert_eq!(current.status, RunStatus::Failed);
    assert_eq!(current.steps[0].status, ToolStepStatus::Failed);
    assert_eq!(current.steps[0].summary.as_deref(), Some("上次运行已中断"));
    assert_eq!(
        state.scenes[0].messages.last().unwrap().tool_steps,
        current.steps
    );
    assert_eq!(
        state.scenes[0]
            .messages
            .iter()
            .filter(|message| !message.tool_steps.is_empty())
            .count(),
        3
    );
    drop(reopened);
    assert_eq!(Store::open(&temp.path).unwrap().snapshot(), state);
}

#[test]
fn memory_and_step_limits_count_unicode_characters_and_fail_atomically() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store, true);
    for index in 0..8 {
        manual(
            &mut store,
            &agent,
            &format!("{}{}", "中".repeat(1999), index),
        );
    }
    let full = store.snapshot();
    for text in [" ".into(), "中".repeat(2001)] {
        assert!(store
            .apply(SceneCommand::SaveMemory {
                agent_id: agent.clone(),
                id: None,
                text,
                expected_revision: None
            })
            .is_err());
        assert_eq!(store.snapshot(), full);
    }
    let second = other_agent(&mut store);
    for index in 0..64 {
        manual(&mut store, &second.id, &format!("短{index}"));
    }
    assert!(store
        .apply(SceneCommand::SaveMemory {
            agent_id: second.id,
            id: None,
            text: "第65条".into(),
            expected_revision: None
        })
        .is_ok());
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "工具限制");
    assert!(store
        .begin_tool(&scene, &run.run_id, "id", "fixture", &"中".repeat(81))
        .is_err());
    store
        .begin_tool(&scene, &run.run_id, "id", "fixture", &"中".repeat(80))
        .unwrap();
    let before = store.snapshot();
    assert!(store
        .finish_tool(
            &scene,
            &run.run_id,
            "id",
            ToolStepStatus::Completed,
            "中".repeat(2001)
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .finish_tool(
            &scene,
            &run.run_id,
            "id",
            ToolStepStatus::Completed,
            "中".repeat(2000),
        )
        .unwrap();
    assert!(matches!(
        store.finish_tool(
            &scene,
            &run.run_id,
            "id",
            ToolStepStatus::Failed,
            "重复结算"
        ),
        Err(CoreError::StaleTool)
    ));
}

#[test]
fn history_search_is_bounded_same_agent_text_without_attachment_metadata() {
    let mut store = Store::open_in_memory().unwrap();
    enable(&mut store, true);
    let a = store.snapshot().active_scene_id;
    for index in 0..7 {
        let run = start(
            &mut store,
            &a,
            &format!("{}关键词{}{}", "前".repeat(900), index, "后".repeat(900)),
        );
        store.finish_run(&a, &run.run_id, "回答").unwrap();
    }
    let b = store.apply(SceneCommand::NewScene).unwrap().active_scene_id;
    let other = other_agent(&mut store);
    store
        .apply(SceneCommand::SetAgent {
            scene_id: b.clone(),
            agent_id: other.id,
        })
        .unwrap();
    let secret = start(&mut store, &b, "关键词其他 Agent 秘密");
    store
        .finish_run(&b, &secret.run_id, "关键词其他回复秘密")
        .unwrap();
    let current = start(&mut store, &a, "关键词本轮不应被返回");
    let hits = store
        .search_history_for_run(&a, &current.run_id, "关键词")
        .unwrap();
    assert_eq!(hits.len(), 5);
    for hit in &hits {
        assert_eq!(hit.scene_id, a);
        assert!(hit.text.contains("关键词"));
        assert!(hit.text.chars().count() <= 800);
        assert!(!hit.text.contains("秘密"));
        assert_ne!(hit.message_id, current.messages.last().unwrap().id);
        let value = serde_json::to_value(hit).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert!(value.get("attachments").is_none());
        assert!(value.get("toolSteps").is_none());
    }
    assert!(store
        .search_history_for_run(&a, &current.run_id, &"查".repeat(201))
        .is_err());
    assert!(store
        .search_history_for_run(&a, &current.run_id, " ")
        .is_err());
}

#[test]
fn database_write_failure_preserves_memory_tool_and_terminal_state() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    enable(&mut store, true);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "事务故障");
    store
        .begin_tool(&scene, &run.run_id, "step", "memory_save", "保存")
        .unwrap();
    let before = store.snapshot();
    let db = SqlConnection::open(&temp.path).unwrap();
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER fail_writes BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.memory_for_run(&scene, &run.run_id, save("不应保存")),
        Err(CoreError::Database(_))
    ));
    assert!(matches!(
        store.begin_tool(&scene, &run.run_id, "other", "fixture", "不应保存"),
        Err(CoreError::Database(_))
    ));
    assert!(matches!(
        store.finish_tool(
            &scene,
            &run.run_id,
            "step",
            ToolStepStatus::Completed,
            "不应保存"
        ),
        Err(CoreError::Database(_))
    ));
    assert!(matches!(
        store.cancel_run(&scene),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    db.execute_batch("DROP TRIGGER fail_writes;").unwrap();
    store
        .memory_for_run(&scene, &run.run_id, save("可重新保存"))
        .unwrap();
    store
        .finish_tool(
            &scene,
            &run.run_id,
            "step",
            ToolStepStatus::Completed,
            "已保存",
        )
        .unwrap();
    store.finish_run(&scene, &run.run_id, "已完成").unwrap();
}

#[test]
fn corrupt_memory_source_is_rejected_without_overwriting_database() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.path).unwrap();
    let agent = enable(&mut store, true);
    manual(&mut store, &agent, "记忆");
    drop(store);
    let db = SqlConnection::open(&temp.path).unwrap();
    let before = payload(&db);
    let scene = serde_json::from_str::<serde_json::Value>(&before).unwrap()["activeSceneId"]
        .as_str()
        .unwrap()
        .to_string();
    db.execute(
        "UPDATE memory_records SET source_scene_id=?1,source_message_id='nonexistent'",
        [scene],
    )
    .unwrap();
    assert!(Store::open(&temp.path).is_err());
    assert_eq!(payload(&db), before);
}

#[test]
fn repeated_fact_creation_is_idempotent_and_does_not_rewrite_provenance() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store, true);
    manual(&mut store, &agent, "  喜欢中文  ");
    let first = entries(&store, &agent)[0].clone();
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, "同一个事实");
    store
        .memory_for_run(&scene, &run.run_id, save("喜欢中文  "))
        .unwrap();
    assert_eq!(entries(&store, &agent), vec![first.clone()]);
    manual(&mut store, &agent, "喜欢短回答");
    let second = entries(&store, &agent)
        .into_iter()
        .find(|m| m.text == "喜欢短回答")
        .unwrap();
    let unchanged = store.snapshot();
    assert!(store
        .memory_for_run(
            &scene,
            &run.run_id,
            MemoryMutation::Save {
                id: Some(second.id),
                text: first.text,
                expected_revision: Some(second.revision)
            }
        )
        .is_err());
    assert_eq!(store.snapshot(), unchanged);
}
