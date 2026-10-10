// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "mewu-agent-cleanup-{}.sqlite",
            uuid::Uuid::new_v4()
        )))
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn profiles(store: &mut Store) -> (String, String, String) {
    let fallback = store.snapshot().agents[0].id.clone();
    let scene = store.snapshot().active_scene_id;
    let mut agent = store.snapshot().agents[0].clone();
    agent.id = uuid::Uuid::new_v4().to_string();
    agent.name = "临时配置".into();
    agent.memory_enabled = true;
    let removed = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    store
        .apply(SceneCommand::SetAgent {
            scene_id: scene.clone(),
            agent_id: removed.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: removed.clone(),
            id: None,
            text: "本地事实".into(),
            expected_revision: None,
        })
        .unwrap();
    (removed, fallback, scene)
}
#[test]
fn local_profile_delete_preserves_scene_draft_and_assets_and_removes_only_its_memory() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let (removed, fallback, scene) = profiles(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "保留草稿".into(),
        })
        .unwrap();
    let before = store.snapshot().scenes[0].clone();
    store.delete_agent(&removed, &fallback).unwrap();
    let after = store.snapshot();
    assert!(!after.agents.iter().any(|a| a.id == removed));
    assert_eq!(after.scenes[0].agent_id, fallback);
    assert_eq!(after.scenes[0].draft, before.draft);
    assert_eq!(after.scenes[0].items, before.items);
    assert_eq!(after.scenes[0].messages, before.messages);
    let db = Connection::open(&file.0).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM memory_records WHERE agent_id=?1",
            [removed],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(store);
    assert_eq!(Store::open(&file.0).unwrap().snapshot(), after);
}
#[test]
fn execution_history_blocks_profile_deletion_and_preserves_every_record() {
    let mut store = Store::open_in_memory().unwrap();
    let (removed, fallback, scene) = profiles(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "问题".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    store.finish_run(&scene, &run.run_id, "回答").unwrap();
    let before = store.snapshot();
    assert!(store.delete_agent(&removed, &fallback).is_err());
    assert_eq!(store.snapshot(), before);
    assert!(store
        .run_journal_summary(&scene, &run.run_id)
        .unwrap()
        .is_some());
}
#[test]
fn profile_delete_sqlite_failure_rolls_back_local_memory_and_scene_identity() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let (removed, fallback, _) = profiles(&mut store);
    let before = store.snapshot();
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER reject_agent_delete BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store.delete_agent(&removed, &fallback).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(store.memory_page(&removed, "", None, 25).unwrap().total, 1);
}
