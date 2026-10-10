// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-text-import-{}", Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn stored(&self) -> (i64, String) {
        Connection::open(&self.0)
            .unwrap()
            .query_row(
                "SELECT revision,payload FROM app_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
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
fn asset(kind: AssetKind, name: &str) -> Asset {
    let id = Uuid::new_v4().to_string();
    let image = kind == AssetKind::Image;
    Asset {
        path: format!("assets/{id}.synthetic"),
        id,
        name: name.into(),
        kind,
        width: image.then_some(640),
        height: image.then_some(480),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn active(store: &Store) -> Scene {
    let snapshot = store.snapshot();
    snapshot
        .scenes
        .into_iter()
        .find(|s| s.id == snapshot.active_scene_id)
        .unwrap()
}
fn draft(store: &mut Store, scene_id: &str, text: &str) {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.into(),
            draft: text.into(),
        })
        .unwrap();
}

#[test]
fn batch_import_adds_all_items_and_references_in_one_commit_preserving_draft() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    store
        .add_asset(&scene_id, asset(AssetKind::Image, "existing.png"))
        .unwrap();
    let existing = active(&store).items[0].id.clone();
    let original_refs = vec![Reference {
        kind: ReferenceKind::Item,
        id: existing,
    }];
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene_id.clone(),
            refs: original_refs.clone(),
        })
        .unwrap();
    draft(&mut store, &scene_id, "保留未发送草稿");
    let before = db.stored();
    let batch = vec![
        asset(AssetKind::Text, "资料.md"),
        asset(AssetKind::Image, "图片.png"),
        asset(AssetKind::Html, "页面.html"),
        asset(AssetKind::Svg, "图示.svg"),
    ];
    let snapshot = store.import_assets(&scene_id, batch.clone()).unwrap();
    let after = active(&store);
    assert_eq!(snapshot.active_scene_id, scene_id);
    assert_eq!(after.draft, "保留未发送草稿");
    assert_eq!(&after.refs[..1], original_refs);
    assert_eq!(after.items.len(), 5);
    assert_eq!(after.refs.len(), 5);
    for (index, expected) in batch.iter().enumerate() {
        let item = &after.items[index + 1];
        assert_eq!(&item.asset, expected);
        assert_eq!(
            after.refs[index + 1],
            Reference {
                kind: ReferenceKind::Item,
                id: item.id.clone()
            }
        );
    }
    assert_eq!(db.stored().0, before.0 + 1);
    assert_eq!(serde_json::to_value(&batch[0]).unwrap()["kind"], "text");
    let pragma: u32 = Connection::open(&db.0)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(pragma, 9);
    drop(store);
    assert_eq!(Store::open(&db.0).unwrap().snapshot(), snapshot);
}

#[test]
fn importing_during_a_run_changes_only_future_references_and_rejects_stale_spaces() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "正在执行的问题");
    let context = store.begin_run(&scene_id).unwrap();
    let running = active(&store).run.clone();
    let messages = active(&store).messages;
    draft(&mut store, &scene_id, "下一轮草稿");
    store
        .import_assets(&scene_id, vec![asset(AssetKind::Text, "next.txt")])
        .unwrap();
    let after = active(&store);
    assert_eq!(after.run, running);
    assert_eq!(after.messages, messages);
    assert_eq!(after.draft, "下一轮草稿");
    assert_eq!(after.refs.len(), 1);
    assert!(context.refs.is_empty() && context.items.is_empty());
    assert!(store.run_is_active(&scene_id, &context.run_id));
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scene_id.clone(),
        })
        .unwrap();
    let frozen = store.snapshot();
    assert!(store
        .import_assets(&scene_id, vec![asset(AssetKind::Text, "late.txt")])
        .is_err());
    assert_eq!(store.snapshot(), frozen);
    assert!(store.run_is_active(&scene_id, &context.run_id));
    store
        .apply(SceneCommand::CloseScene {
            scene_id: scene_id.clone(),
        })
        .unwrap();
    let closed = store.snapshot();
    assert!(store
        .import_assets(&scene_id, vec![asset(AssetKind::Text, "closed.txt")])
        .is_err());
    assert_eq!(store.snapshot(), closed);
    assert!(store
        .import_assets("missing-scene", vec![asset(AssetKind::Text, "missing.txt")])
        .is_err());
    assert_eq!(store.snapshot(), closed);
}

#[test]
fn any_invalid_asset_rejects_the_entire_batch_and_text_never_has_pixel_metadata() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "不能丢失");
    let before = store.snapshot();
    for field in [
        "width", "height", "origin_x", "origin_y", "scale", "name", "path", "id", "video", "file",
    ] {
        let mut invalid = asset(AssetKind::Text, "bad.txt");
        match field {
            "width" => invalid.width = Some(1),
            "height" => invalid.height = Some(1),
            "origin_x" => invalid.origin_x = Some(0),
            "origin_y" => invalid.origin_y = Some(0),
            "scale" => invalid.scale_factor = Some(1.0),
            "name" => invalid.name.clear(),
            "path" => invalid.path.clear(),
            "id" => invalid.id.clear(),
            "video" => invalid.kind = AssetKind::Video,
            "file" => invalid.kind = AssetKind::File,
            _ => unreachable!(),
        }
        assert!(
            store
                .import_assets(
                    &scene_id,
                    vec![asset(AssetKind::Text, "valid.txt"), invalid]
                )
                .is_err(),
            "{field}"
        );
        assert_eq!(store.snapshot(), before, "{field}");
    }
    assert!(store.import_assets(&scene_id, vec![]).is_err());
    assert!(store
        .import_assets(
            &scene_id,
            (0..13)
                .map(|_| asset(AssetKind::Text, "many.txt"))
                .collect()
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .import_assets(
            &scene_id,
            (0..12)
                .map(|_| asset(AssetKind::Text, "twelve.txt"))
                .collect(),
        )
        .unwrap();
    assert_eq!(active(&store).refs.len(), 12);
}

#[test]
fn asset_ids_cannot_alias_the_batch_other_scenes_background_or_history() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let background = asset(AssetKind::Image, "background.png");
    store.set_background(&scene_id, background.clone()).unwrap();
    let historical = asset(AssetKind::Text, "history.json");
    draft(&mut store, &scene_id, "仅保存在消息附件");
    let run = store.begin_run(&scene_id).unwrap();
    store
        .attach_run_assets(&scene_id, &run.run_id, vec![historical.clone()])
        .unwrap();
    store.finish_run(&scene_id, &run.run_id, "完成").unwrap();
    let other_item = asset(AssetKind::Text, "other.csv");
    store.add_asset(&scene_id, other_item.clone()).unwrap();
    store.apply(SceneCommand::NewScene).unwrap();
    let current = store.snapshot().active_scene_id;
    let before = store.snapshot();
    for existing in [background, historical, other_item] {
        let mut collision = asset(AssetKind::Text, "different.txt");
        collision.id = existing.id;
        assert!(store
            .import_assets(
                &current,
                vec![asset(AssetKind::Text, "valid.txt"), collision]
            )
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
    let duplicate = asset(AssetKind::Text, "duplicate.txt");
    assert!(store
        .import_assets(&current, vec![duplicate.clone(), duplicate])
        .is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn failed_sqlite_commit_and_stale_writer_leave_no_partial_items_or_refs() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "完整保留");
    let before = store.snapshot();
    let disk_before = db.stored();
    let injected = Connection::open(&db.0).unwrap();
    injected.execute_batch("CREATE TRIGGER reject_import BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic import failure'); END;").unwrap();
    assert!(store
        .import_assets(
            &scene_id,
            vec![
                asset(AssetKind::Text, "one.txt"),
                asset(AssetKind::Html, "two.html")
            ]
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(db.stored(), disk_before);
    injected
        .execute_batch("DROP TRIGGER reject_import;")
        .unwrap();
    drop(injected);
    let mut other = Store::open(&db.0).unwrap();
    draft(&mut other, &scene_id, "另一写者的新草稿");
    let newer = other.snapshot();
    assert!(matches!(
        store.import_assets(&scene_id, vec![asset(AssetKind::Text, "stale.txt")]),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(other);
    drop(store);
    assert_eq!(Store::open(&db.0).unwrap().snapshot(), newer);
}

#[test]
fn text_history_survives_item_removal_freeze_restart_and_readonly_export() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let text = asset(AssetKind::Text, "资料.csv");
    store.import_assets(&scene_id, vec![text.clone()]).unwrap();
    let item_id = active(&store).items[0].id.clone();
    let run = store.begin_run(&scene_id).unwrap();
    store
        .attach_run_assets(&scene_id, &run.run_id, vec![text.clone()])
        .unwrap();
    let bound = store.snapshot();
    let mut replacement = text.clone();
    replacement.path = "other.txt".into();
    assert!(store
        .attach_run_assets(&scene_id, &run.run_id, vec![replacement])
        .is_err());
    assert_eq!(store.snapshot(), bound);
    store
        .finish_run(&scene_id, &run.run_id, "保留文本历史")
        .unwrap();
    store
        .apply(SceneCommand::RemoveItem {
            scene_id: scene_id.clone(),
            item_id,
        })
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scene_id.clone(),
        })
        .unwrap();
    let completed = store.snapshot();
    let view = ExportView::open(&db.0).unwrap();
    assert_eq!(view.snapshot(), &completed);
    let next_scene = completed.active_scene_id.clone();
    store
        .import_assets(&next_scene, vec![asset(AssetKind::Text, "new.md")])
        .unwrap();
    assert_eq!(view.snapshot(), &completed);
    let original = view
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .unwrap();
    assert!(original.frozen && original.items.is_empty() && original.refs.is_empty());
    assert_eq!(
        original.messages[0].attachments.as_ref().unwrap(),
        &vec![text]
    );
    let exported: Snapshot =
        serde_json::from_str(&serde_json::to_string(view.snapshot()).unwrap()).unwrap();
    assert_eq!(exported, completed);
    drop(view);
    let after = store.snapshot();
    drop(store);
    assert_eq!(Store::open(&db.0).unwrap().snapshot(), after);
}

#[test]
fn text_is_allowed_in_bounded_attachments_but_video_file_and_seven_items_are_not() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    draft(&mut store, &scene_id, "分析文档");
    let run = store.begin_run(&scene_id).unwrap();
    let before = store.snapshot();
    for invalid in [
        vec![asset(AssetKind::Video, "video.mp4")],
        vec![asset(AssetKind::File, "file.bin")],
        (0..7)
            .map(|_| asset(AssetKind::Text, "too-many.txt"))
            .collect(),
    ] {
        assert!(store
            .attach_run_assets(&scene_id, &run.run_id, invalid)
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
    store
        .attach_run_assets(
            &scene_id,
            &run.run_id,
            (0..6)
                .map(|_| asset(AssetKind::Text, "valid.txt"))
                .collect(),
        )
        .unwrap();
    assert_eq!(
        active(&store).messages[0]
            .attachments
            .as_ref()
            .unwrap()
            .len(),
        6
    );
}

#[test]
fn invalid_persisted_text_metadata_is_rejected_without_rewriting_the_database() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    store
        .import_assets(&scene_id, vec![asset(AssetKind::Text, "valid.txt")])
        .unwrap();
    drop(store);
    let connection = Connection::open(&db.0).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&db.stored().1).unwrap();
    payload["scenes"][0]["items"][0]["asset"]["width"] = serde_json::json!(100);
    connection
        .execute(
            "UPDATE app_state SET payload=?1 WHERE id=1",
            [serde_json::to_string(&payload).unwrap()],
        )
        .unwrap();
    let bad = db.stored();
    drop(connection);
    assert!(Store::open(&db.0).is_err());
    assert_eq!(db.stored(), bad);
}
