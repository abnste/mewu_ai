// SPDX-License-Identifier: MPL-2.0
use mewu_core::{CoreError, ExportView, MemoryEntry, RunStatus, SceneCommand, Store};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-export-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self(directory.join("spaces.db"))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        for name in ["spaces.db", "spaces.db-wal", "spaces.db-shm"] {
            let _ = std::fs::remove_file(self.0.with_file_name(name));
        }
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}
fn all_memories(store: &Store, agent: &str) -> Vec<MemoryEntry> {
    let mut entries = vec![];
    let mut cursor = None;
    loop {
        let page = store.memory_page(agent, "", cursor.as_deref(), 25).unwrap();
        entries.extend(page.entries);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return entries;
        }
    }
}

#[test]
fn export_keeps_one_snapshot_while_live_writer_updates_and_revokes_runs() {
    let file = TestDb::new();
    let mut store = Store::open(file.path()).unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    store
        .apply(SceneCommand::SaveAgent {
            agent: agent.clone(),
        })
        .unwrap();
    for index in 0..55 {
        store
            .apply(SceneCommand::SaveMemory {
                agent_id: agent.id.clone(),
                id: None,
                text: format!("导出时的旧事实 {index}"),
                expected_revision: None,
            })
            .unwrap();
    }
    let before = all_memories(&store, &agent.id);
    let scene_id = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: "继续工作".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene_id).unwrap();
    let snapshot = store.snapshot();
    let view = ExportView::open(file.path()).unwrap();
    assert_eq!(view.snapshot(), &snapshot);
    assert!(store.run_is_active(&scene_id, &run.run_id));
    let first = view.memory_page(&agent.id, "", None, 25).unwrap();
    assert_eq!(first.total, 55);

    // All writes must finish while the export reader is still alive. A rollback
    // journal reader would block these commits and eventually return SQLITE_BUSY.
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.id.clone(),
            id: Some(before[30].id.clone()),
            text: "导出开始之后才改写".into(),
            expected_revision: Some(before[30].revision),
        })
        .unwrap();
    store
        .apply(SceneCommand::DeleteMemory {
            agent_id: agent.id.clone(),
            id: before[40].id.clone(),
            expected_revision: before[40].revision,
        })
        .unwrap();
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.id.clone(),
            id: None,
            text: "导出开始之后才新增".into(),
            expected_revision: None,
        })
        .unwrap();
    agent.memory_enabled = false;
    store
        .apply(SceneCommand::SaveAgent {
            agent: agent.clone(),
        })
        .unwrap();
    assert!(!store.run_is_active(&scene_id, &run.run_id));

    let mut exported = first.entries;
    let mut cursor = first.next_cursor;
    while let Some(value) = cursor {
        let page = view.memory_page(&agent.id, "", Some(&value), 25).unwrap();
        assert_eq!(page.total, 55);
        exported.extend(page.entries);
        cursor = page.next_cursor;
    }
    assert_eq!(exported, before);
    assert_eq!(view.snapshot(), &snapshot);
    assert_eq!(
        view.snapshot().scenes[0].run.as_ref().unwrap().status,
        RunStatus::Running
    );
    let latest = ExportView::open(file.path()).unwrap();
    assert!(!latest.snapshot().agents[0].memory_enabled);
    assert_eq!(
        latest.snapshot().scenes[0].run.as_ref().unwrap().status,
        RunStatus::Canceled
    );
    assert_ne!(
        latest
            .memory_page(&agent.id, "", None, 25)
            .unwrap()
            .revision,
        view.memory_page(&agent.id, "", None, 25).unwrap().revision
    );
    assert!(view.memory_page("another-agent", "", None, 25).is_err());
}

#[test]
fn export_rejects_missing_unknown_future_and_corrupt_databases_without_overwriting() {
    let missing = TestDb::new();
    assert!(ExportView::open(missing.path()).is_err());
    assert!(!missing.path().exists());

    let unknown = TestDb::new();
    {
        let db = Connection::open(unknown.path()).unwrap();
        db.execute_batch(
            "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('keep');",
        )
        .unwrap();
    }
    let original = std::fs::read(unknown.path()).unwrap();
    assert!(ExportView::open(unknown.path()).is_err());
    assert!(Store::open(unknown.path()).is_err());
    assert_eq!(std::fs::read(unknown.path()).unwrap(), original);

    let future = TestDb::new();
    drop(Store::open(future.path()).unwrap());
    {
        let db = Connection::open(future.path()).unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
    }
    let original = std::fs::read(future.path()).unwrap();
    assert!(matches!(
        ExportView::open(future.path()),
        Err(CoreError::UnsupportedSchema(99))
    ));
    assert!(Store::open(future.path()).is_err());
    assert_eq!(std::fs::read(future.path()).unwrap(), original);

    let corrupt = TestDb::new();
    drop(Store::open(corrupt.path()).unwrap());
    {
        let db = Connection::open(corrupt.path()).unwrap();
        db.execute("UPDATE app_state SET payload='not json'", [])
            .unwrap();
    }
    let original = std::fs::read(corrupt.path()).unwrap();
    assert!(ExportView::open(corrupt.path()).is_err());
    assert!(Store::open(corrupt.path()).is_err());
    assert_eq!(std::fs::read(corrupt.path()).unwrap(), original);
}

#[test]
fn export_refuses_rollback_journal_and_never_changes_its_mode() {
    let file = TestDb::new();
    drop(Store::open(file.path()).unwrap());
    let db = Connection::open(file.path()).unwrap();
    let mode: String = db
        .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    assert!(ExportView::open(file.path()).is_err());
    let mode: String = db
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    drop(db);
    let store = Store::open(file.path()).unwrap();
    let view = ExportView::open(file.path()).unwrap();
    assert_eq!(view.snapshot(), &store.snapshot());
}
