// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-geometry-history-{}", uuid()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn sql(&self) -> Connection {
        Connection::open(&self.0).unwrap()
    }
    fn stored(&self) -> (i64, String) {
        self.sql()
            .query_row(
                "SELECT revision,payload FROM app_state WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    fn replace(&self, value: &Value) {
        self.sql()
            .execute(
                "UPDATE app_state SET payload=?1 WHERE id=1",
                [serde_json::to_string(value).unwrap()],
            )
            .unwrap();
    }
    fn deny_writes(&self) {
        self.sql().execute_batch("CREATE TRIGGER deny_geometry BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    }
    fn allow_writes(&self) {
        self.sql()
            .execute_batch("DROP TRIGGER deny_geometry;")
            .unwrap();
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
fn uuid() -> String {
    Uuid::new_v4().to_string()
}
fn image(width: u32, height: u32) -> Asset {
    let id = uuid();
    Asset {
        path: format!("{id}.png"),
        id,
        name: "synthetic.png".into(),
        kind: AssetKind::Image,
        width: Some(width),
        height: Some(height),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn scene(store: &Store, id: &str) -> Scene {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
}
fn region(store: &Store, scene_id: &str, id: &str) -> Region {
    scene(store, scene_id)
        .regions
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
}
fn setup(store: &mut Store) -> (String, String, String) {
    let scene_id = store.snapshot().active_scene_id;
    store.set_background(&scene_id, image(800, 600)).unwrap();
    let a = add_region(store, &scene_id, uuid(), 20.);
    let b = add_region(store, &scene_id, uuid(), 240.);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: "用户尚未发送的问题".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: a.clone(),
            }],
        })
        .unwrap();
    (scene_id, a, b)
}
fn add_region(store: &mut Store, scene_id: &str, id: String, x: f64) -> String {
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.into(),
            region: Region {
                id: id.clone(),
                x,
                y: 30.,
                width: 180.,
                height: 160.,
                ..Default::default()
            },
        })
        .unwrap();
    id
}
fn target(store: &Store, scene_id: &str, id: &str) -> OcrTarget {
    let s = scene(store, scene_id);
    let r = s.regions.iter().find(|r| r.id == id).unwrap();
    OcrTarget {
        scene_id: scene_id.into(),
        region_id: id.into(),
        background_id: s.background.unwrap().id,
        drawing_revision: r.drawing_revision,
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
fn edit(store: &Store, scene_id: &str, id: &str, dx: f64, edit_id: Option<&str>) -> SceneCommand {
    let s = scene(store, scene_id);
    let r = s.regions.iter().find(|r| r.id == id).unwrap();
    let from = RegionGeometry::from(r);
    SceneCommand::SetRegionGeometry {
        scene_id: scene_id.into(),
        region_id: id.into(),
        background_id: s.background.as_ref().unwrap().id.clone(),
        source_id: r
            .image_override
            .as_ref()
            .or(s.background.as_ref())
            .unwrap()
            .id
            .clone(),
        expected_revision: r.drawing_revision,
        from,
        to: RegionGeometry {
            x: from.x + dx,
            ..from
        },
        edit_id: edit_id.map(str::to_owned),
    }
}
fn move_by(store: &mut Store, scene_id: &str, id: &str, dx: f64, edit_id: Option<&str>) {
    store.apply(edit(store, scene_id, id, dx, edit_id)).unwrap();
}
fn replay(store: &Store, scene_id: &str, redo: bool) -> SceneCommand {
    let s = scene(store, scene_id);
    let history = &s.geometry_history;
    let e = if redo {
        history.redo.last()
    } else {
        history.undo.last()
    }
    .unwrap();
    let r = s.regions.iter().find(|r| r.id == e.region_id).unwrap();
    let mut value = json!({"type":if redo {"redo_region_geometry"} else {"undo_region_geometry"},
        "sceneId":scene_id, "expectedHistoryRevision":history.revision,"expectedOperationId":e.id,
        "regionId":r.id,"backgroundId":e.background_id,"sourceId":e.source_id,
        "expectedRevision":r.drawing_revision,"from":RegionGeometry::from(r)});
    // Exercise the actual typed command boundary, including its camelCase fields.
    serde_json::from_value(value.take()).unwrap()
}
fn undo(store: &mut Store, scene_id: &str) {
    store.apply(replay(store, scene_id, false)).unwrap();
}
fn redo(store: &mut Store, scene_id: &str) {
    store.apply(replay(store, scene_id, true)).unwrap();
}
fn finish(store: &mut Store, scene_id: &str, edit_id: &str) {
    store
        .apply(SceneCommand::FinishRegionGeometryEdit {
            scene_id: scene_id.into(),
            edit_id: edit_id.into(),
        })
        .unwrap();
}
fn add_drawing(store: &mut Store, scene_id: &str, id: &str) {
    let t = target(store, scene_id, id);
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: scene_id.into(),
            region_id: id.into(),
            background_id: t.background_id,
            expected_revision: t.drawing_revision,
            drawing: Drawing {
                origin: None,
                rich: None,
                id: uuid(),
                kind: DrawingKind::Pen,
                color: "#2277ff".into(),
                stroke_width: 3.,
                points: vec![
                    DrawingPoint { x: 40., y: 50. },
                    DrawingPoint { x: 90., y: 100. },
                ],
                text: None,
                font_size: None,
            },
        })
        .unwrap();
}
fn ocr(width: u32, height: u32, text: &str) -> OcrDocument {
    OcrDocument {
        engine: "synthetic".into(),
        language: "zh-Hans".into(),
        width,
        height,
        text_angle: None,
        lines: vec![OcrLine {
            text: text.into(),
            words: vec![OcrWord {
                text: text.into(),
                x: 10.,
                y: 10.,
                width: 100.,
                height: 20.,
            }],
        }],
    }
}
fn derived(store: &mut Store, scene_id: &str, id: &str, text: &str) {
    let t = target(store, scene_id, id);
    let (_, input) = store.region_for_translation(&t).unwrap();
    let (width, height) = (input.width.round() as u32, input.height.round() as u32);
    let mut selection = ocr(width, height, text);
    selection.engine = "mewu.translation.layout.v1".into();
    store
        .set_region_translation(
            &t,
            TranslationDocument {
                version: 1,
                target_language: "zh-Hans".into(),
                width,
                height,
                text_angle: None,
                lines: vec![TranslationLine {
                    id: "line_0".into(),
                    source: "source".into(),
                    text: text.into(),
                    r#box: TranslationBox {
                        x: 10.,
                        y: 10.,
                        width: 100.,
                        height: 20.,
                    },
                }],
            },
            image(width, height),
            selection,
        )
        .unwrap();
    store.set_region_ocr(&t, ocr(width, height, text)).unwrap();
}

#[test]
fn scene_history_orders_interleaved_regions_and_only_replays_geometry() {
    let mut store = Store::open_in_memory().unwrap();
    let (s, a, b) = setup(&mut store);
    let original = scene(&store, &s);
    move_by(&mut store, &s, &a, 1., None);
    move_by(&mut store, &s, &b, 2., None);
    move_by(&mut store, &s, &a, 3., None);
    let history = scene(&store, &s).geometry_history;
    assert_eq!(
        history
            .undo
            .iter()
            .map(|e| e.region_id.as_str())
            .collect::<Vec<_>>(),
        vec![a.as_str(), b.as_str(), a.as_str()]
    );
    for (x_a, x_b) in [(21., 242.), (21., 240.), (20., 240.)] {
        undo(&mut store, &s);
        assert_eq!(region(&store, &s, &a).x, x_a);
        assert_eq!(region(&store, &s, &b).x, x_b);
    }
    assert_eq!(scene(&store, &s).geometry_history.redo.len(), 3);
    for (x_a, x_b) in [(21., 240.), (21., 242.), (24., 242.)] {
        redo(&mut store, &s);
        assert_eq!(region(&store, &s, &a).x, x_a);
        assert_eq!(region(&store, &s, &b).x, x_b);
    }
    let current = scene(&store, &s);
    assert_eq!(current.geometry_history.revision, 9);
    assert_eq!(current.geometry_history.undo, history.undo);
    assert_eq!(current.draft, original.draft);
    assert_eq!(current.refs, original.refs);
    assert_eq!(current.messages, original.messages);
    assert_eq!(current.agent_id, original.agent_id);
    assert_eq!(current.run, original.run);
}

#[test]
fn contiguous_gesture_merges_only_until_matching_finish_and_finish_never_writes() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    let group = uuid();
    for _ in 0..3 {
        move_by(&mut store, &s, &a, 1., Some(&group));
    }
    let h = scene(&store, &s).geometry_history;
    assert_eq!(h.undo.len(), 1);
    assert_eq!(h.revision, 3);
    assert_eq!((h.undo[0].from.x, h.undo[0].to.x), (20., 23.));
    assert_eq!(
        serde_json::from_str::<Snapshot>(&db.stored().1).unwrap(),
        store.snapshot()
    );
    // A late different gesture/scene cannot close the actual current group.
    finish(&mut store, &s, &uuid());
    finish(&mut store, "already-deleted-scene", &group);
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 1);
    db.deny_writes();
    let before = (db.stored(), store.snapshot());
    finish(&mut store, &s, &group);
    finish(&mut store, &s, &group);
    assert_eq!((db.stored(), store.snapshot()), before);
    db.allow_writes();
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 2);
    assert_ne!(scene(&store, &s).geometry_history.undo[1].id, h.undo[0].id);
}

#[test]
fn gestures_do_not_merge_across_another_region_drawing_or_structure_change() {
    let mut store = Store::open_in_memory().unwrap();
    let (s, a, b) = setup(&mut store);
    let group = uuid();
    move_by(&mut store, &s, &a, 1., Some(&group));
    move_by(&mut store, &s, &b, 1., None);
    move_by(&mut store, &s, &a, 1., Some(&group));
    add_drawing(&mut store, &s, &a);
    move_by(&mut store, &s, &a, 1., Some(&group));
    add_region(&mut store, &s, uuid(), 460.);
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 5);
    let drawing = region(&store, &s, &a);
    undo(&mut store, &s);
    assert_eq!(region(&store, &s, &a).drawings, drawing.drawings);
    assert_eq!(
        region(&store, &s, &a).drawing_history,
        drawing.drawing_history
    );
    let t = target(&store, &s, &a);
    let geometry = scene(&store, &s).geometry_history;
    store
        .apply(SceneCommand::UndoDrawing {
            scene_id: s.clone(),
            region_id: a.clone(),
            background_id: t.background_id,
            expected_revision: t.drawing_revision,
        })
        .unwrap();
    assert!(region(&store, &s, &a).drawings.is_empty());
    assert_eq!(scene(&store, &s).geometry_history, geometry);
    redo(&mut store, &s);
    assert!(region(&store, &s, &a).drawings.is_empty());
    assert_eq!(region(&store, &s, &a).drawing_history.redo.len(), 1);
}

#[test]
fn zero_net_gesture_removes_only_its_entry_and_does_not_resurrect_redo() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    move_by(&mut store, &s, &a, 2., None);
    undo(&mut store, &s);
    let group = uuid();
    move_by(&mut store, &s, &a, 1., Some(&group));
    move_by(&mut store, &s, &a, -1., Some(&group));
    let h = scene(&store, &s).geometry_history;
    assert!(h.undo.is_empty() && h.redo.is_empty());
    assert_eq!(h.revision, 4);
    assert_eq!(
        serde_json::to_value(scene(&store, &s)).unwrap()["geometryHistory"]["revision"],
        4
    );
    let saved = store.snapshot();
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(store.snapshot(), saved);
    assert_eq!(scene(&store, &s).geometry_history, h);
    move_by(&mut store, &s, &a, 3., Some(&group));
    let h = scene(&store, &s).geometry_history;
    assert_eq!(h.undo.len(), 1);
    assert_eq!((h.undo[0].from.x, h.undo[0].to.x), (20., 23.));
    undo(&mut store, &s);
    assert_eq!(region(&store, &s, &a).x, 20.);
}

#[test]
fn noop_invalid_and_failed_new_edits_keep_redo_and_the_database_unchanged() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    move_by(&mut store, &s, &a, 1., None);
    undo(&mut store, &s);
    let before = (store.snapshot(), db.stored());
    db.deny_writes();
    move_by(&mut store, &s, &a, 0., None);
    for command in [
        edit(&store, &s, &a, -100., None),
        edit(&store, &s, &a, 1., Some("not-a-uuid")),
        edit(&store, &s, &a, 1., None),
    ] {
        assert!(store.apply(command).is_err());
        assert_eq!((store.snapshot(), db.stored()), before);
    }
    db.allow_writes();
    move_by(&mut store, &s, &a, 2., None);
    assert!(scene(&store, &s).geometry_history.redo.is_empty());
}

#[test]
fn replay_full_target_and_history_head_are_independent_exact_fences() {
    let mut store = Store::open_in_memory().unwrap();
    let (s, a, b) = setup(&mut store);
    move_by(&mut store, &s, &a, 1., None);
    move_by(&mut store, &s, &b, 1., None);
    let command = replay(&store, &s, false);
    let before = store.snapshot();
    for field in [
        "sceneId",
        "expectedHistoryRevision",
        "expectedOperationId",
        "regionId",
        "backgroundId",
        "sourceId",
        "expectedRevision",
        "from",
    ] {
        let mut bad = serde_json::to_value(&command).unwrap();
        match field {
            "expectedRevision" | "expectedHistoryRevision" => bad[field] = json!(0),
            "from" => bad[field]["x"] = json!(241.000000000001),
            "regionId" => bad[field] = json!(a),
            _ => bad[field] = json!(uuid()),
        }
        assert!(
            store.apply(serde_json::from_value(bad).unwrap()).is_err(),
            "{field}"
        );
        assert_eq!(store.snapshot(), before);
    }
    let mut injection = serde_json::to_value(&command).unwrap();
    injection["to"] = json!({"x":0,"y":0,"width":1,"height":1});
    assert!(serde_json::from_value::<SceneCommand>(injection).is_err());
    store.apply(command.clone()).unwrap();
    let committed = store.snapshot();
    assert!(matches!(
        store.apply(command),
        Err(CoreError::GeometryHistoryConflict)
    ));
    assert_eq!(store.snapshot(), committed);
}

#[test]
fn ordinary_replay_never_revives_old_layers_and_override_keeps_latest_same_revision_layers() {
    for replacement in [false, true] {
        let mut store = Store::open_in_memory().unwrap();
        let (s, a, _) = setup(&mut store);
        if replacement {
            let t = target(&store, &s, &a);
            store.set_region_image(&t, image(300, 1200)).unwrap();
        }
        add_drawing(&mut store, &s, &a);
        derived(&mut store, &s, &a, "旧文档");
        move_by(&mut store, &s, &a, 1., None);
        let pending = replay(&store, &s, false);
        // Completion can replace derived documents without changing the target revision.
        derived(&mut store, &s, &a, "撤销请求后刚完成的新文档");
        let latest = region(&store, &s, &a);
        store.apply(pending).unwrap();
        let current = region(&store, &s, &a);
        assert_eq!(current.drawings, latest.drawings);
        assert_eq!(current.drawing_history, latest.drawing_history);
        assert_eq!(current.image_override, latest.image_override);
        assert_eq!(current.drawing_revision, latest.drawing_revision + 1);
        if replacement {
            assert_eq!(
                current.ocr.as_ref().unwrap().document,
                latest.ocr.unwrap().document
            );
            assert_eq!(
                current.translation.as_ref().unwrap().overlay,
                latest.translation.unwrap().overlay
            );
            assert_eq!(
                current.ocr.as_ref().unwrap().drawing_revision,
                current.drawing_revision
            );
            assert_eq!(
                current.translation.as_ref().unwrap().drawing_revision,
                current.drawing_revision
            );
        } else {
            assert!(current.ocr.is_none() && current.translation.is_none());
        }
        redo(&mut store, &s);
        let redone = region(&store, &s, &a);
        if !replacement {
            assert!(redone.ocr.is_none() && redone.translation.is_none());
        }
        assert_eq!(redone.drawings, current.drawings);
        assert_eq!(redone.drawing_revision, current.drawing_revision + 1);
    }
}

#[test]
fn source_removal_recording_and_legacy_geometry_prune_affected_entries_atomically() {
    for mode in ["source", "remove", "recording", "legacy", "background"] {
        let mut store = Store::open_in_memory().unwrap();
        let (s, a, b) = setup(&mut store);
        move_by(&mut store, &s, &a, 1., None);
        move_by(&mut store, &s, &b, 1., None);
        move_by(&mut store, &s, &a, 1., None);
        undo(&mut store, &s);
        let old = scene(&store, &s).geometry_history;
        match mode {
            "source" => {
                store
                    .set_region_image(&target(&store, &s, &a), image(300, 1200))
                    .unwrap();
            }
            "remove" => {
                store
                    .apply(SceneCommand::RemoveRegion {
                        scene_id: s.clone(),
                        region_id: a.clone(),
                    })
                    .unwrap();
            }
            "recording" => {
                let r = region(&store, &s, &a);
                let mut video = image(180, 160);
                video.kind = AssetKind::Video;
                video.name = "synthetic.mp4".into();
                video.path = format!("{}.mp4", video.id);
                store
                    .finish_recording(&s, &target(&store, &s, &a).background_id, &r, video)
                    .unwrap();
            }
            "legacy" => {
                let mut r = region(&store, &s, &a);
                r.x += 1.;
                store
                    .apply(SceneCommand::UpdateRegion {
                        scene_id: s.clone(),
                        region: r,
                    })
                    .unwrap();
            }
            "background" => {
                store.set_background(&s, image(800, 600)).unwrap();
            }
            _ => unreachable!(),
        }
        let h = scene(&store, &s).geometry_history;
        assert!(h.redo.is_empty(), "{mode}");
        assert_eq!(h.revision, old.revision + 1, "{mode}");
        if mode == "background" {
            assert!(h.undo.is_empty());
        } else {
            assert_eq!(h.undo.len(), 1, "{mode}");
            assert_eq!(h.undo[0].region_id, b);
            undo(&mut store, &s);
        }
    }
}

#[test]
fn adding_a_region_keeps_undo_clears_redo_and_does_not_record_creation() {
    let mut store = Store::open_in_memory().unwrap();
    let (s, a, b) = setup(&mut store);
    move_by(&mut store, &s, &a, 1., None);
    move_by(&mut store, &s, &b, 1., None);
    undo(&mut store, &s);
    let before = scene(&store, &s).geometry_history;
    let added = add_region(&mut store, &s, uuid(), 460.);
    let after = scene(&store, &s).geometry_history;
    assert_eq!(after.undo, before.undo);
    assert!(after.redo.is_empty());
    assert_eq!(after.revision, before.revision + 1);
    undo(&mut store, &s);
    assert!(scene(&store, &s).regions.iter().any(|r| r.id == added));
}

#[test]
fn failed_merge_replay_and_source_cleanup_leave_persistent_and_transient_state_intact() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    let group = uuid();
    move_by(&mut store, &s, &a, 1., Some(&group));
    let before = (store.snapshot(), db.stored());
    db.deny_writes();
    assert!(matches!(
        store.apply(edit(&store, &s, &a, 1., Some(&group))),
        Err(CoreError::Database(_))
    ));
    assert_eq!((store.snapshot(), db.stored()), before);
    assert!(matches!(
        store.apply(replay(&store, &s, false)),
        Err(CoreError::Database(_))
    ));
    assert_eq!((store.snapshot(), db.stored()), before);
    assert!(store
        .set_region_image(&target(&store, &s, &a), image(300, 1200))
        .is_err());
    assert_eq!((store.snapshot(), db.stored()), before);
    db.allow_writes();
    move_by(&mut store, &s, &a, 1., Some(&group));
    let h = scene(&store, &s).geometry_history;
    assert_eq!(h.undo.len(), 1);
    assert_eq!((h.undo[0].from.x, h.undo[0].to.x), (20., 22.));
}

#[test]
fn a_second_writer_cannot_commit_replay_or_merge_over_new_derived_state() {
    let db = TestDb::new();
    let mut first = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut first);
    let group = uuid();
    move_by(&mut first, &s, &a, 1., Some(&group));
    let mut stale = Store::open(&db.0).unwrap();
    let before = stale.snapshot();
    derived(&mut first, &s, &a, "另一实例的最新内容");
    let winning = db.stored();
    assert!(matches!(
        stale.apply(replay(&stale, &s, false)),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), before);
    assert_eq!(db.stored(), winning);
    assert!(matches!(
        stale.apply(edit(&stale, &s, &a, 1., Some(&group))),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), before);
    assert_eq!(db.stored(), winning);
}

#[test]
fn freeze_restart_restore_preserve_stacks_but_never_reopen_the_previous_gesture() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    let group = uuid();
    move_by(&mut store, &s, &a, 1., Some(&group));
    let h = scene(&store, &s).geometry_history;
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: s.clone(),
        })
        .unwrap();
    let frozen = store.snapshot();
    let pending = replay(&store, &s, false);
    assert!(store.apply(pending).is_err());
    assert_eq!(store.snapshot(), frozen);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(store.snapshot(), frozen);
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: s.clone(),
        })
        .unwrap();
    assert_eq!(scene(&store, &s).geometry_history, h);
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 2);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 3);
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: s.clone(),
        })
        .unwrap();
    move_by(&mut store, &s, &a, 1., Some(&group));
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 4);
}

#[test]
fn only_idle_active_open_scenes_can_replay_without_canceling_the_run() {
    for mode in ["inactive", "closed", "running"] {
        let mut store = Store::open_in_memory().unwrap();
        let (s, a, _) = setup(&mut store);
        move_by(&mut store, &s, &a, 1., None);
        let pending = replay(&store, &s, false);
        match mode {
            "inactive" => {
                store.apply(SceneCommand::NewScene).unwrap();
            }
            "closed" => {
                store
                    .apply(SceneCommand::CloseScene {
                        scene_id: s.clone(),
                    })
                    .unwrap();
            }
            "running" => {
                store.begin_run(&s).unwrap();
            }
            _ => unreachable!(),
        }
        let before = store.snapshot();
        assert!(store.apply(pending).is_err(), "{mode}");
        assert_eq!(store.snapshot(), before);
    }
}

#[test]
fn count_and_json_budgets_evict_oldest_only_and_oversized_single_entry_is_rejected() {
    let mut store = Store::open_in_memory().unwrap();
    let (s, a, _) = setup(&mut store);
    for _ in 0..55 {
        move_by(&mut store, &s, &a, 1., None);
    }
    let h = scene(&store, &s).geometry_history;
    assert_eq!(h.undo.len(), 50);
    assert_eq!(h.undo[0].from.x, 25.);
    for _ in 0..50 {
        undo(&mut store, &s);
    }
    assert_eq!(region(&store, &s, &a).x, 25.);
    let empty = json!({"type":"undo_region_geometry","sceneId":s,"expectedHistoryRevision":105,
        "expectedOperationId":uuid(),"regionId":a,"backgroundId":target(&store,&s,&a).background_id,
        "sourceId":target(&store,&s,&a).background_id,"expectedRevision":region(&store,&s,&a).drawing_revision,
        "from":RegionGeometry::from(&region(&store,&s,&a))});
    assert!(matches!(
        store.apply(serde_json::from_value(empty).unwrap()),
        Err(CoreError::GeometryHistoryConflict)
    ));
    // Legacy core identities are not length-capped; the history budget must still be a hard bound.
    let large = add_region(&mut store, &s, "r".repeat(3000), 460.);
    for _ in 0..25 {
        move_by(&mut store, &s, &large, 1., None);
    }
    let h = scene(&store, &s).geometry_history;
    assert!(h.undo.len() < 25);
    assert!(serde_json::to_vec(&h).unwrap().len() <= 64 * 1024);
    let huge = add_region(&mut store, &s, "h".repeat(64 * 1024), 460.);
    let before = store.snapshot();
    assert!(store.apply(edit(&store, &s, &huge, 1., None)).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn old_missing_history_is_default_and_corrupt_history_is_never_silently_repaired() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, _) = setup(&mut store);
    assert!(serde_json::to_value(scene(&store, &s))
        .unwrap()
        .get("geometryHistory")
        .is_none());
    let before = db.stored();
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(db.stored(), before);
    move_by(&mut store, &s, &a, 1., None);
    move_by(&mut store, &s, &a, 1., None);
    let good = serde_json::to_value(store.snapshot()).unwrap();
    drop(store);
    for mode in [
        "bad-chain",
        "duplicate",
        "source",
        "zero-revision",
        "unknown",
        "missing-region",
        "out-of-bounds",
    ] {
        let mut bad = good.clone();
        let h = &mut bad["scenes"][0]["geometryHistory"];
        match mode {
            "bad-chain" => h["undo"][0]["to"]["x"] = json!(22.5),
            "duplicate" => h["undo"][1]["id"] = h["undo"][0]["id"].clone(),
            "source" => h["undo"][0]["sourceId"] = json!(uuid()),
            "zero-revision" => h["revision"] = json!(0),
            "unknown" => h["undo"][0]["translation"] = json!({}),
            "missing-region" => h["undo"][0]["regionId"] = json!(uuid()),
            "out-of-bounds" => h["undo"][0]["from"]["x"] = json!(-0.1),
            _ => unreachable!(),
        }
        db.replace(&bad);
        let stored = db.stored();
        assert!(Store::open(&db.0).is_err(), "{mode}");
        assert_eq!(db.stored(), stored, "{mode}");
        assert!(ExportView::open(&db.0).is_err(), "{mode}");
        assert_eq!(db.stored(), stored);
    }
}

#[test]
fn history_or_region_revision_overflow_rejects_the_whole_operation() {
    for field in ["history", "region"] {
        let db = TestDb::new();
        let mut store = Store::open(&db.0).unwrap();
        let (s, a, _) = setup(&mut store);
        move_by(&mut store, &s, &a, 1., None);
        let mut value = serde_json::to_value(store.snapshot()).unwrap();
        drop(store);
        if field == "history" {
            value["scenes"][0]["geometryHistory"]["revision"] = json!(u64::MAX);
        } else {
            value["scenes"][0]["regions"][0]["drawingRevision"] = json!(u64::MAX);
        }
        db.replace(&value);
        let mut store = Store::open(&db.0).unwrap();
        let before = (store.snapshot(), db.stored());
        assert!(store.apply(replay(&store, &s, false)).is_err());
        assert_eq!((store.snapshot(), db.stored()), before);
        assert!(store.apply(edit(&store, &s, &a, 1., None)).is_err());
        assert_eq!((store.snapshot(), db.stored()), before);
    }
}

#[test]
fn exact_byte_boundary_reserves_future_revision_digits_and_stack_separators() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (s, a, b) = setup(&mut store);
    move_by(&mut store, &s, &a, 1., None);
    move_by(&mut store, &s, &b, 1., None);
    let mut value = serde_json::to_value(store.snapshot()).unwrap();
    drop(store);
    value["scenes"][0]["geometryHistory"]["revision"] = json!(u64::MAX);
    let bytes = serde_json::to_vec(&value["scenes"][0]["geometryHistory"])
        .unwrap()
        .len();
    // This field appears once in history, so the worst-case JSON is exactly 64KiB.
    let long_b = format!("{b}{}", "b".repeat(64 * 1024 - bytes));
    value["scenes"][0]["regions"][1]["id"] = json!(long_b);
    value["scenes"][0]["geometryHistory"]["undo"][1]["regionId"] = json!(long_b);
    assert_eq!(
        serde_json::to_vec(&value["scenes"][0]["geometryHistory"])
            .unwrap()
            .len(),
        64 * 1024
    );
    value["scenes"][0]["geometryHistory"]["revision"] = json!(99);
    db.replace(&value);
    let mut store = Store::open(&db.0).unwrap();
    undo(&mut store, &s); // 99 -> 100, two nonempty stacks.
    assert_eq!(scene(&store, &s).geometry_history.revision, 100);
    let split = store.snapshot();
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(store.snapshot(), split);
    redo(&mut store, &s); // Recombining stacks adds a comma.
    assert_eq!(region(&store, &s, &long_b).x, 241.);
    assert!(
        serde_json::to_vec(&scene(&store, &s).geometry_history)
            .unwrap()
            .len()
            <= 64 * 1024
    );
    move_by(&mut store, &s, &long_b, 1., None);
    // The new push uses the same reserved budget and prunes old facts only.
    assert_eq!(scene(&store, &s).geometry_history.undo.len(), 1);
    undo(&mut store, &s);
    assert_eq!(region(&store, &s, &long_b).x, 241.);
    drop(store);

    // Actual JSON can fit today yet lack the reserved byte needed by replay.
    let too_long = format!("{long_b}x");
    value["scenes"][0]["regions"][1]["id"] = json!(too_long);
    value["scenes"][0]["geometryHistory"]["undo"][1]["regionId"] = json!(too_long);
    assert!(
        serde_json::to_vec(&value["scenes"][0]["geometryHistory"])
            .unwrap()
            .len()
            < 64 * 1024
    );
    db.replace(&value);
    let before = db.stored();
    assert!(Store::open(&db.0).is_err());
    assert_eq!(db.stored(), before);
}
