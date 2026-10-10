// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mewu-provider-{}", Uuid::new_v4()));
        assert!(!path
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("spaces.db")
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path());
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn enable(store: &mut Store) -> String {
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    let id = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    id
}
fn save(store: &mut Store, agent: &str, text: &str) {
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.into(),
            id: None,
            text: text.into(),
            expected_revision: None,
        })
        .unwrap();
}
fn run(store: &mut Store, text: &str) -> RunContext {
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: text.into(),
        })
        .unwrap();
    store.begin_run(&scene).unwrap()
}
fn legacy(path: &std::path::Path, state: &Snapshot) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY CHECK(id=1),schema_version INTEGER NOT NULL CHECK(schema_version=1),revision INTEGER NOT NULL,payload TEXT NOT NULL);PRAGMA user_version=1;PRAGMA application_id=1296389973;").unwrap();
    let mut value = serde_json::to_value(state).unwrap();
    value.as_object_mut().unwrap().remove("memoryStats");
    db.execute(
        "INSERT INTO app_state VALUES(1,1,9,?1)",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
}
fn old_entry(agent: &str, text: &str) -> MemoryEntry {
    MemoryEntry {
        id: Uuid::new_v4().to_string(),
        agent_id: agent.into(),
        text: text.into(),
        revision: 7,
        created_at: 1,
        updated_at: 2,
        source: None,
        origin: None,
    }
}
fn payload(db: &Connection) -> String {
    db.query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn storage_exceeds_form_limits_while_snapshots_and_run_recall_stay_bounded() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    for i in 0..75 {
        save(
            &mut store,
            &agent,
            &format!("深海方案 {i:03} {}", "中🙂".repeat(400)),
        );
    }
    let state = store.snapshot();
    assert!(state.memories.is_empty());
    assert_eq!(state.memory_stats[0].count, 75);
    assert_eq!(
        serde_json::to_value(&state).unwrap()["memories"],
        serde_json::json!([])
    );
    let context = run(&mut store, "深海方案");
    assert!(!context.memories.is_empty());
    assert!(context.memories.len() <= 8);
    assert!(
        context
            .memories
            .iter()
            .map(|m| m.text.chars().count())
            .sum::<usize>()
            <= 6000
    );
    assert!(serde_json::to_vec(&context.memories).unwrap().len() <= 24 * 1024);
    let page = store.memory_page(&agent, "", None, 25).unwrap();
    assert_eq!(page.total, 75);
    assert_eq!(page.entries.len(), 25);
    assert!(page.next_cursor.is_some());
    let absent = store
        .recall_for_run(&context.scene_id, &context.run_id, "不存在的独角兽")
        .unwrap();
    assert!(absent.is_empty());
}

#[test]
fn chinese_short_queries_and_quoted_fts_characters_are_literal_and_agent_scoped() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    save(&mut store, &agent, "项目代号紫色海风，喜欢中文");
    save(&mut store, &agent, "alpha AND beta");
    save(&mut store, &agent, "alpha beta");
    save(&mut store, &agent, "他说\"海风\"计划");
    let mut other = store.snapshot().agents[0].clone();
    other.id = Uuid::new_v4().to_string();
    other.name = "Other".into();
    let other_id = other.id.clone();
    store
        .apply(SceneCommand::SaveAgent { agent: other })
        .unwrap();
    save(&mut store, &other_id, "紫色海风，其他身份秘密");
    for query in ["紫色海风", "海风", "中", "中文"] {
        let page = store.memory_page(&agent, query, None, 25).unwrap();
        assert!(!page.entries.is_empty());
        assert!(page.entries.iter().all(|e| e.agent_id == agent));
    }
    assert_eq!(store.memory_page(&agent, "AND", None, 25).unwrap().total, 1);
    assert_eq!(
        store.memory_page(&agent, "ALPHA", None, 25).unwrap().total,
        2
    );
    assert_eq!(
        store
            .memory_page(&agent, "\"海风\"", None, 25)
            .unwrap()
            .total,
        1
    );
    assert!(store
        .memory_page(&agent, "alpha OR hidden", None, 25)
        .unwrap()
        .entries
        .is_empty());
    let context = run(&mut store, "我想查项目代号的记录");
    assert!(context.memories.iter().any(|m| m.text.contains("紫色海风")));
    assert!(context.memories.iter().all(|m| !m.text.contains("秘密")));
}

#[test]
fn management_cursor_is_revision_query_and_agent_bound_with_no_skipped_ids() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    for i in 0..57 {
        save(&mut store, &agent, &format!("分页内容 {i}"));
    }
    let mut cursor = None;
    let mut ids = std::collections::HashSet::new();
    let mut first = None;
    loop {
        let page = store.memory_page(&agent, "", cursor.as_deref(), 7).unwrap();
        assert_eq!(page.total, 57);
        for entry in page.entries {
            assert!(ids.insert(entry.id));
        }
        if first.is_none() {
            first = page.next_cursor.clone();
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 57);
    assert!(matches!(
        store.memory_page(&agent, "分页", first.as_deref(), 7),
        Err(CoreError::MemoryConflict)
    ));
    save(&mut store, &agent, "新数据改变游标");
    assert!(matches!(
        store.memory_page(&agent, "", first.as_deref(), 7),
        Err(CoreError::MemoryConflict)
    ));
    assert!(store.memory_page(&agent, "", None, 26).is_err());
    assert!(store
        .memory_page(&agent, &"中".repeat(201), None, 5)
        .is_err());
    assert!(store
        .memory_page(&agent, "", Some("{bad cursor}"), 5)
        .is_err());
}

#[test]
fn migration_preserves_entries_sources_and_long_references_without_enabling_memory() {
    let temp = TestDb::new();
    let mut state = Store::open_in_memory().unwrap().snapshot();
    state.agents[0].memory_enabled = false;
    let agent = state.agents[0].id.clone();
    let text = format!("开头 {} 结尾", "🙂中文".repeat(1800));
    state.agents[0].memory = text.clone();
    let message_id = Uuid::new_v4().to_string();
    let scene = state.active_scene_id.clone();
    state.scenes[0].messages.push(Message {
        id: message_id.clone(),
        role: MessageRole::User,
        text: "最初原文".into(),
        reasoning: None,
        conversation_start: None,
        created_at: 1,
        run_id: None,
        refs: None,
        attachments: None,
        tool_steps: vec![],
    });
    let mut entry = old_entry(&agent, "旧版完整事实");
    entry.source = Some(MemorySource {
        scene_id: scene,
        message_id,
    });
    state.memories.push(entry.clone());
    legacy(&temp.path(), &state);
    let mut store = Store::open(temp.path()).unwrap();
    let page = store.memory_page(&agent, "", None, 25).unwrap();
    let migrated = page.entries.iter().find(|m| m.id == entry.id).unwrap();
    assert_eq!(migrated.text, entry.text);
    assert_eq!(migrated.revision, 7);
    assert_eq!(migrated.source, entry.source);
    assert_eq!(migrated.created_at, 1);
    assert_eq!(migrated.updated_at, 2);
    assert_eq!(migrated.origin, Some(MemoryOrigin::LegacyMemory));
    assert_eq!(page.total, 4);
    assert!(!store.snapshot().agents[0].memory_enabled);
    assert_eq!(store.snapshot().agents[0].memory, text);
    assert!(store.snapshot().memories.is_empty());
    let db = Connection::open(temp.path()).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
    assert_eq!(store.snapshot().schema_version, 1);
    let chunks = db
        .prepare("SELECT text FROM memory_records WHERE origin='legacy_reference' ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(chunks.concat(), text);
    assert!(chunks.iter().all(|c| c.chars().count() <= 2000));
    store
        .apply(SceneCommand::DeleteMemory {
            agent_id: agent.clone(),
            id: entry.id.clone(),
            expected_revision: 7,
        })
        .unwrap();
    for item in store.memory_page(&agent, "", None, 25).unwrap().entries {
        store
            .apply(SceneCommand::DeleteMemory {
                agent_id: agent.clone(),
                id: item.id,
                expected_revision: item.revision,
            })
            .unwrap();
    }
    drop(store);
    let reopened = Store::open(temp.path()).unwrap();
    assert_eq!(reopened.memory_page(&agent, "", None, 25).unwrap().total, 0);
    assert_eq!(reopened.snapshot().agents[0].memory, text);
    assert_eq!(
        db.query_row("SELECT count(*) FROM memory_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn failed_migration_rolls_back_ddl_index_rows_version_and_original_payload() {
    let temp = TestDb::new();
    let mut state = Store::open_in_memory().unwrap().snapshot();
    let agent = state.agents[0].id.clone();
    state.memories.push(old_entry(&agent, "先写入的事实"));
    let mut invalid = old_entry(&agent, "后续失败事实");
    invalid.revision = u64::MAX;
    state.memories.push(invalid);
    legacy(&temp.path(), &state);
    let db = Connection::open(temp.path()).unwrap();
    let before = payload(&db);
    assert!(Store::open(temp.path()).is_err());
    assert_eq!(payload(&db), before);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'memory_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn memory_index_stats_and_app_revision_roll_back_together_and_reject_stale_writer() {
    let temp = TestDb::new();
    let mut store = Store::open(temp.path()).unwrap();
    let agent = enable(&mut store);
    save(&mut store, &agent, "现有完整数据");
    let mut stale = Store::open(temp.path()).unwrap();
    let db = Connection::open(temp.path()).unwrap();
    let before = store.snapshot();
    let raw = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_memory_commit BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: None,
            text: "失败孤岛".into(),
            expected_revision: None
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), raw);
    assert_eq!(store.memory_page(&agent, "", None, 25).unwrap().total, 1);
    assert_eq!(
        store
            .memory_page(&agent, "失败孤岛", None, 25)
            .unwrap()
            .total,
        0
    );
    db.execute_batch("DROP TRIGGER reject_memory_commit;")
        .unwrap();
    save(&mut store, &agent, "胜出的第二条");
    assert!(matches!(
        stale.apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: None,
            text: "旧实例写入".into(),
            expected_revision: None
        }),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(
        store.memory_page(&agent, "旧实例", None, 25).unwrap().total,
        0
    );
    assert_eq!(store.memory_page(&agent, "", None, 25).unwrap().total, 2);
    assert_eq!(
        db.query_row(
            "SELECT record_count FROM memory_state WHERE agent_id=?1",
            [&agent],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn edits_and_forgets_update_fts_and_do_not_reappear_after_restart() {
    let temp = TestDb::new();
    let mut store = Store::open(temp.path()).unwrap();
    let agent = enable(&mut store);
    save(&mut store, &agent, "旧关键词橙色灯塔");
    let item = store
        .memory_page(&agent, "橙色灯塔", None, 1)
        .unwrap()
        .entries
        .remove(0);
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: Some(item.id.clone()),
            text: "新关键词蓝色灯塔".into(),
            expected_revision: Some(item.revision),
        })
        .unwrap();
    assert_eq!(
        store
            .memory_page(&agent, "橙色灯塔", None, 25)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .memory_page(&agent, "蓝色灯塔", None, 25)
            .unwrap()
            .total,
        1
    );
    store
        .apply(SceneCommand::DeleteMemory {
            agent_id: agent.clone(),
            id: item.id,
            expected_revision: item.revision + 1,
        })
        .unwrap();
    drop(store);
    let reopened = Store::open(temp.path()).unwrap();
    assert_eq!(
        reopened
            .memory_page(&agent, "蓝色灯塔", None, 25)
            .unwrap()
            .total,
        0
    );
    assert_eq!(reopened.snapshot().memory_stats[0].count, 0);
}

#[test]
fn malformed_legacy_source_fails_without_creating_new_memory_storage() {
    let temp = TestDb::new();
    let mut state = Store::open_in_memory().unwrap().snapshot();
    let agent = state.agents[0].id.clone();
    let mut entry = old_entry(&agent, "错误来源");
    entry.source = Some(MemorySource {
        scene_id: state.active_scene_id.clone(),
        message_id: "missing".into(),
    });
    state.memories.push(entry);
    legacy(&temp.path(), &state);
    assert!(Store::open(temp.path()).is_err());
    let db = Connection::open(temp.path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='memory_records'",
            params![],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn exact_lookup_is_agent_scoped_and_deleted_entries_return_none() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    save(&mut store, &agent, "原位恢复草稿");
    let entry = store
        .memory_page(&agent, "", None, 1)
        .unwrap()
        .entries
        .remove(0);
    let mut other = store.snapshot().agents[0].clone();
    other.id = Uuid::new_v4().to_string();
    let other_id = other.id.clone();
    store
        .apply(SceneCommand::SaveAgent { agent: other })
        .unwrap();
    assert_eq!(
        store.memory_entry(&agent, &entry.id).unwrap(),
        Some(entry.clone())
    );
    assert!(store.memory_entry(&other_id, &entry.id).unwrap().is_none());
    assert!(matches!(
        store.memory_entry("unknown", &entry.id),
        Err(CoreError::NotFound { .. })
    ));
    store
        .apply(SceneCommand::DeleteMemory {
            agent_id: agent.clone(),
            id: entry.id.clone(),
            expected_revision: entry.revision,
        })
        .unwrap();
    assert!(store.memory_entry(&agent, &entry.id).unwrap().is_none());
}
