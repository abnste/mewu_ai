// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-ocr-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self(directory.join("spaces.db"))
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

fn background() -> Asset {
    Asset {
        id: Uuid::new_v4().to_string(),
        name: "synthetic.png".into(),
        kind: AssetKind::Image,
        path: "synthetic.png".into(),
        width: Some(800),
        height: Some(600),
        origin_x: Some(-800),
        origin_y: Some(0),
        scale_factor: Some(1.5),
    }
}
fn prepare(store: &mut Store) -> OcrTarget {
    let scene_id = store.snapshot().active_scene_id;
    let asset = background();
    let target = OcrTarget {
        scene_id: scene_id.clone(),
        region_id: Uuid::new_v4().to_string(),
        background_id: asset.id.clone(),
        drawing_revision: 0,
        x: 20.25,
        y: 30.25,
        width: 600.25,
        height: 400.25,
    };
    store.set_background(&scene_id, asset).unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id,
            region: Region {
                id: target.region_id.clone(),
                x: target.x,
                y: target.y,
                width: target.width,
                height: target.height,
                ..Default::default()
            },
        })
        .unwrap();
    target
}
fn document() -> OcrDocument {
    OcrDocument {
        engine: "windows.media.ocr".into(),
        language: "zh-Hans-CN".into(),
        width: 600,
        height: 400,
        text_angle: None,
        lines: vec![OcrLine {
            text: "你好 世界 <svg>".into(),
            words: vec![
                OcrWord {
                    text: "你好".into(),
                    x: 12.5,
                    y: 24.,
                    width: 40.,
                    height: 20.,
                },
                OcrWord {
                    text: "世界 <svg>".into(),
                    x: 60.,
                    y: 24.,
                    width: 130.,
                    height: 20.,
                },
            ],
        }],
    }
}
fn region(store: &Store, target: &OcrTarget) -> Region {
    store
        .snapshot()
        .scenes
        .iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap()
        .regions
        .iter()
        .find(|region| region.id == target.region_id)
        .unwrap()
        .clone()
}
fn current_target(store: &Store, old: &OcrTarget) -> OcrTarget {
    let region = region(store, old);
    OcrTarget {
        drawing_revision: region.drawing_revision,
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
        ..old.clone()
    }
}
fn drawing() -> Drawing {
    Drawing {
        origin: None,
        rich: None,
        id: Uuid::new_v4().to_string(),
        kind: DrawingKind::Pen,
        color: "#112233".into(),
        stroke_width: 2.,
        points: vec![DrawingPoint { x: 100., y: 100. }],
        text: None,
        font_size: None,
    }
}

#[test]
fn background_completion_preserves_frozen_scene_and_survives_close_restore_and_restart() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "后台处理".into(),
        })
        .unwrap();
    let run = store.begin_run(&target.scene_id).unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let visible = store.snapshot().active_scene_id;
    assert_ne!(visible, target.scene_id);
    store.set_region_ocr(&target, document()).unwrap();
    assert_eq!(store.snapshot().active_scene_id, visible);
    assert!(store.run_is_active(&target.scene_id, &run.run_id));
    let frozen = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap();
    assert!(frozen.frozen);
    assert_eq!(frozen.regions[0].ocr.as_ref().unwrap().document, document());
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        store.set_region_ocr(&target, document()),
        Err(CoreError::OcrConflict)
    ));
    drop(store);
    let mut reopened = Store::open(&db.0).unwrap();
    assert_eq!(region(&reopened, &target).ocr.unwrap().document, document());
    reopened
        .apply(SceneCommand::ActivateScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert_eq!(region(&reopened, &target).ocr.unwrap().document, document());
    let sql = Connection::open(&db.0).unwrap();
    let version: u32 = sql
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
    assert_eq!(reopened.snapshot().schema_version, 1);
}

#[test]
fn stale_identity_geometry_and_revision_cannot_write_or_clear_results() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    store.set_region_ocr(&target, document()).unwrap();
    let before = store.snapshot();
    let mut alternatives = Vec::new();
    for field in 0..8 {
        let mut stale = target.clone();
        match field {
            0 => stale.scene_id = Uuid::new_v4().to_string(),
            1 => stale.region_id = Uuid::new_v4().to_string(),
            2 => stale.background_id = Uuid::new_v4().to_string(),
            3 => stale.drawing_revision += 1,
            4 => stale.x += 1.,
            5 => stale.y += 1.,
            6 => stale.width += 1.,
            _ => stale.height += 1.,
        }
        alternatives.push(stale);
    }
    for stale in alternatives {
        assert!(matches!(
            store.set_region_ocr(&stale, document()),
            Err(CoreError::OcrConflict)
        ));
        assert!(matches!(
            store.clear_region_ocr(&stale),
            Err(CoreError::OcrConflict)
        ));
        assert_eq!(store.snapshot(), before);
    }
    store
        .apply(SceneCommand::RemoveRegion {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        store.set_region_ocr(&target, document()),
        Err(CoreError::OcrConflict)
    ));
    store
        .set_background(&target.scene_id, background())
        .unwrap();
    assert!(matches!(
        store.set_region_ocr(&target, document()),
        Err(CoreError::OcrConflict)
    ));
}

#[test]
fn scene_commands_cannot_inject_or_overwrite_host_owned_recognition() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    store.set_region_ocr(&target, document()).unwrap();
    let original = region(&store, &target);
    let (image, input) = store.region_for_ocr(&target).unwrap();
    assert_eq!(image.id, target.background_id);
    assert_eq!(input.id, target.region_id);
    assert!(input.ocr.is_none());
    assert!(input.drawing_history.is_empty());
    let mut injected = original.clone();
    injected.id = Uuid::new_v4().to_string();
    assert!(store
        .apply(SceneCommand::AddRegion {
            scene_id: target.scene_id.clone(),
            region: injected
        })
        .is_err());
    let mut forged_update = original.clone();
    forged_update.ocr.as_mut().unwrap().document.lines[0].text = "伪造替换".into();
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: forged_update,
        })
        .unwrap();
    assert_eq!(region(&store, &target), original);
    let mut moved = original;
    moved.x += 1.;
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: moved,
        })
        .unwrap();
    assert!(region(&store, &target).ocr.is_none());
    assert!(matches!(
        store.set_region_ocr(&target, document()),
        Err(CoreError::OcrConflict)
    ));
    let current = current_target(&store, &target);
    let mut wrong_size = document();
    wrong_size.width += 1;
    assert!(store.set_region_ocr(&current, wrong_size).is_err());
    store.set_region_ocr(&current, document()).unwrap();
    store
        .set_background(&target.scene_id, background())
        .unwrap();
    assert!(store
        .snapshot()
        .scenes
        .iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap()
        .regions
        .is_empty());
}

#[test]
fn drawing_changes_and_undo_redo_invalidate_ocr_but_noop_does_not() {
    let mut store = Store::open_in_memory().unwrap();
    let original_target = prepare(&mut store);
    let mut target = original_target.clone();
    store.set_region_ocr(&target, document()).unwrap();
    let mut object = drawing();
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: target.drawing_revision,
            drawing: object.clone(),
        })
        .unwrap();
    assert!(region(&store, &target).ocr.is_none());
    target = current_target(&store, &target);
    store.set_region_ocr(&target, document()).unwrap();
    let (_, input) = store.region_for_ocr(&target).unwrap();
    assert_eq!(input.drawings, vec![object.clone()]);
    assert!(input.ocr.is_none());
    assert!(input.drawing_history.is_empty());
    store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: target.drawing_revision,
            drawing: object.clone(),
        })
        .unwrap();
    assert!(region(&store, &target).ocr.is_some());
    object.color = "#FFEEDD".into();
    store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: target.drawing_revision,
            drawing: object.clone(),
        })
        .unwrap();
    assert!(region(&store, &target).ocr.is_none());
    for redo in [false, true] {
        target = current_target(&store, &target);
        store.set_region_ocr(&target, document()).unwrap();
        store
            .apply(if redo {
                SceneCommand::RedoDrawing {
                    scene_id: target.scene_id.clone(),
                    region_id: target.region_id.clone(),
                    background_id: target.background_id.clone(),
                    expected_revision: target.drawing_revision,
                }
            } else {
                SceneCommand::UndoDrawing {
                    scene_id: target.scene_id.clone(),
                    region_id: target.region_id.clone(),
                    background_id: target.background_id.clone(),
                    expected_revision: target.drawing_revision,
                }
            })
            .unwrap();
        assert!(region(&store, &target).ocr.is_none());
    }
    target = current_target(&store, &target);
    store.set_region_ocr(&target, document()).unwrap();
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: target.drawing_revision,
            drawing_id: object.id,
        })
        .unwrap();
    assert!(region(&store, &target).ocr.is_none());
    assert!(matches!(
        store.set_region_ocr(&original_target, document()),
        Err(CoreError::OcrConflict)
    ));
}

#[test]
fn documents_are_bounded_without_clipping_valid_rotated_text_or_plain_markup() {
    let valid = document();
    validate_ocr_document(&valid).unwrap();
    let mut empty = valid.clone();
    empty.lines.clear();
    validate_ocr_document(&empty).unwrap();
    let mut rotated = valid.clone();
    rotated.text_angle = Some(20.);
    rotated.lines[0].words[0].x = -20.;
    validate_ocr_document(&rotated).unwrap();
    rotated.text_angle = None;
    assert!(validate_ocr_document(&rotated).is_err());
    for field in 0..10 {
        let mut malformed = valid.clone();
        match field {
            0 => malformed.width = 0,
            1 => malformed.text_angle = Some(f64::NAN),
            2 => malformed.lines[0].words[0].x = f64::INFINITY,
            3 => malformed.lines[0].words[0].width = 0.,
            4 => malformed.lines[0].words[0].text.push('\0'),
            5 => malformed.lines[0].words[0].text = "字".repeat(513),
            6 => malformed.lines[0].text = "字".repeat(8193),
            7 => malformed.lines[0].words.clear(),
            8 => malformed.engine.push('\n'),
            _ => malformed.language = " ".into(),
        }
        assert!(validate_ocr_document(&malformed).is_err(), "field {field}");
    }
    let mut lines = valid.clone();
    lines.lines = vec![valid.lines[0].clone(); 513];
    assert!(validate_ocr_document(&lines)
        .unwrap_err()
        .to_string()
        .contains("行数"));
    let mut words = valid.clone();
    words.lines[0].words = vec![valid.lines[0].words[0].clone(); 4097];
    assert!(validate_ocr_document(&words)
        .unwrap_err()
        .to_string()
        .contains("词数"));
    let mut text = valid.clone();
    text.lines = vec![
        OcrLine {
            text: "字".repeat(8192),
            words: valid.lines[0].words.clone()
        };
        6
    ];
    assert!(validate_ocr_document(&text)
        .unwrap_err()
        .to_string()
        .contains("文本"));
    let word = OcrWord {
        text: "a\t".repeat(8),
        x: 12.123456789012345,
        y: 25.123456789012345,
        width: 10.123456789012345,
        height: 11.123456789012345,
    };
    let mut large = valid;
    large.lines = vec![
        OcrLine {
            text: "x".into(),
            words: vec![word; 512]
        };
        8
    ];
    assert!(validate_ocr_document(&large)
        .unwrap_err()
        .to_string()
        .contains("文档"));
}

#[test]
fn failed_storage_and_second_writer_leave_prior_result_and_revision_unchanged() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    store.set_region_ocr(&target, document()).unwrap();
    let before = store.snapshot();
    let sql = Connection::open(&db.0).unwrap();
    sql.execute_batch("CREATE TRIGGER deny_ocr BEFORE UPDATE ON app_state BEGIN SELECT RAISE(FAIL, 'synthetic failure'); END;").unwrap();
    let revision: i64 = sql
        .query_row("SELECT revision FROM app_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut replacement = document();
    replacement.lines[0].text = "识别的新内容".into();
    assert!(store.set_region_ocr(&target, replacement.clone()).is_err());
    assert!(store.clear_region_ocr(&target).is_err());
    let mut moved = region(&store, &target);
    moved.x += 2.;
    assert!(store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: moved
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    let unchanged: i64 = sql
        .query_row("SELECT revision FROM app_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(revision, unchanged);
    sql.execute_batch("DROP TRIGGER deny_ocr;").unwrap();
    let mut other = Store::open(&db.0).unwrap();
    other.set_region_ocr(&target, replacement.clone()).unwrap();
    assert!(matches!(
        store.clear_region_ocr(&target),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(store);
    drop(other);
    drop(sql);
    let mut reopened = Store::open(&db.0).unwrap();
    assert_eq!(
        region(&reopened, &target).ocr.unwrap().document,
        replacement
    );
    reopened.clear_region_ocr(&target).unwrap();
    assert!(region(&reopened, &target).ocr.is_none());
}

#[test]
fn old_json_defaults_to_none_and_corrupt_saved_ocr_is_not_silently_replaced() {
    let legacy: Region = serde_json::from_value(
        serde_json::json!({"id":"legacy","x":0,"y":0,"width":10,"height":10}),
    )
    .unwrap();
    assert!(legacy.ocr.is_none());
    assert!(serde_json::to_value(legacy).unwrap().get("ocr").is_none());
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    store.set_region_ocr(&target, document()).unwrap();
    drop(store);
    let sql = Connection::open(&db.0).unwrap();
    let payload: String = sql
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&payload).unwrap();
    json["scenes"][0]["regions"][0]["ocr"]["drawingRevision"] = serde_json::json!(99);
    let corrupt = serde_json::to_string(&json).unwrap();
    sql.execute("UPDATE app_state SET payload=?1 WHERE id=1", [&corrupt])
        .unwrap();
    assert!(Store::open(&db.0).is_err());
    let saved: String = sql
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(saved, corrupt);
}
