// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-translation-{}", Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
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
fn image(width: u32, height: u32) -> Asset {
    let id = Uuid::new_v4().to_string();
    Asset {
        name: "synthetic.png".into(),
        path: format!("{id}.png"),
        id,
        kind: AssetKind::Image,
        width: Some(width),
        height: Some(height),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn prepare(store: &mut Store) -> OcrTarget {
    let scene_id = store.snapshot().active_scene_id;
    let asset = image(800, 600);
    let target = OcrTarget {
        scene_id: scene_id.clone(),
        region_id: Uuid::new_v4().to_string(),
        background_id: asset.id.clone(),
        drawing_revision: 0,
        x: 20.,
        y: 30.,
        width: 600.,
        height: 400.,
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
fn region(store: &Store, target: &OcrTarget) -> Region {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == target.scene_id)
        .unwrap()
        .regions
        .into_iter()
        .find(|r| r.id == target.region_id)
        .unwrap()
}
fn current(store: &Store, target: &OcrTarget) -> OcrTarget {
    let region = region(store, target);
    OcrTarget {
        drawing_revision: region.drawing_revision,
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
        ..target.clone()
    }
}
fn document(width: u32, height: u32) -> TranslationDocument {
    TranslationDocument {
        version: 1,
        target_language: "zh-Hans".into(),
        width,
        height,
        text_angle: None,
        lines: vec![TranslationLine {
            id: "line_0".into(),
            source: "Hello <world>".into(),
            text: "你好 世界".into(),
            r#box: TranslationBox {
                x: 12.,
                y: 24.,
                width: 100.,
                height: 20.,
            },
        }],
    }
}
fn selection(doc: &TranslationDocument) -> OcrDocument {
    OcrDocument {
        engine: "mewu.translation.layout.v1".into(),
        language: doc.target_language.clone(),
        width: doc.width,
        height: doc.height,
        text_angle: doc.text_angle,
        lines: doc
            .lines
            .iter()
            .map(|line| OcrLine {
                text: line.text.clone(),
                words: vec![OcrWord {
                    text: line.text.clone(),
                    x: 12.,
                    y: 24.,
                    width: 100.,
                    height: 20.,
                }],
            })
            .collect(),
    }
}
fn save(store: &mut Store, target: &OcrTarget) -> SavedTranslation {
    let (_, input) = store.region_for_translation(target).unwrap();
    let doc = document(input.width.round() as u32, input.height.round() as u32);
    store
        .set_region_translation(
            target,
            doc.clone(),
            image(doc.width, doc.height),
            selection(&doc),
        )
        .unwrap();
    region(store, target).translation.unwrap()
}

#[test]
fn pin_source_keeps_screen_placement_and_all_layers_without_mutating_a_running_scene() {
    let mut store = Store::open_in_memory().unwrap();
    let mut target = prepare(&mut store);
    let replacement = image(300, 1200);
    store
        .set_region_image(&target, replacement.clone())
        .unwrap();
    target = current(&store, &target);
    add(&mut store, &target, drawing(DrawingKind::Pen));
    target = current(&store, &target);
    save(&mut store, &target);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "运行中的问题".into(),
        })
        .unwrap();
    let run = store.begin_run(&target.scene_id).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "保留下一条草稿".into(),
        })
        .unwrap();
    let before = store.snapshot();
    let copied = store.region_for_pin(&target).unwrap();
    assert_eq!(copied.0.id, target.background_id);
    assert_eq!(copied.1, region(&store, &target));
    assert_eq!(copied.1.image_override, Some(replacement));
    assert_eq!(
        (copied.1.x, copied.1.y, copied.1.width, copied.1.height),
        (target.x, target.y, target.width, target.height)
    );
    assert!(!copied.1.drawing_history.is_empty());
    assert!(copied.1.translation.is_some());
    assert_eq!(store.snapshot(), before);
    assert!(store.run_is_active(&target.scene_id, &run.run_id));

    // Same OcrTarget, different visible pixels: host CAS must compare the
    // complete returned image document, not only the drawing revision.
    store.clear_region_translation(&target).unwrap();
    let changed = store.region_for_pin(&target).unwrap();
    assert_eq!(changed.1.drawing_revision, copied.1.drawing_revision);
    assert_ne!(changed, copied);
    assert!(copied.1.translation.is_some());
}

#[test]
fn pin_source_rejects_inactive_frozen_closed_and_stale_selection() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    let initial = store.region_for_pin(&target).unwrap();
    let mut stale = target.clone();
    stale.x += 1.;
    assert!(store.region_for_pin(&stale).is_err());
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(store.region_for_pin(&target).is_err());
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert_eq!(store.region_for_pin(&target).unwrap(), initial);
    store.apply(SceneCommand::NewScene).unwrap();
    assert!(store.region_for_pin(&target).is_err());
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let closed = store.snapshot();
    assert!(store.region_for_pin(&target).is_err());
    assert_eq!(store.snapshot(), closed);
}

fn drawing(kind: DrawingKind) -> Drawing {
    Drawing {
        origin: None,
        rich: None,
        id: Uuid::new_v4().to_string(),
        kind,
        color: "#112233".into(),
        stroke_width: 8.,
        points: vec![
            DrawingPoint { x: 40., y: 50. },
            DrawingPoint { x: 80., y: 90. },
        ],
        text: None,
        font_size: None,
    }
}
fn add(store: &mut Store, target: &OcrTarget, object: Drawing) {
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: target.drawing_revision,
            drawing: object,
        })
        .unwrap();
}

#[test]
fn completion_preserves_frozen_conversation_and_restart_without_switching_scene() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "保留草稿".into(),
        })
        .unwrap();
    let run = store.begin_run(&target.scene_id).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "下一条".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    let saved = save(&mut store, &target);
    assert_eq!(store.snapshot().active_scene_id, before.active_scene_id);
    assert!(store.run_is_active(&target.scene_id, &run.run_id));
    let scene = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == target.scene_id)
        .unwrap();
    assert!(scene.frozen);
    assert_eq!(scene.draft, "下一条");
    assert_eq!(saved.source_id, target.background_id);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        store.clear_region_translation(&target),
        Err(CoreError::TranslationConflict)
    ));
    drop(store);
    let reopened = Store::open(&db.0).unwrap();
    assert_eq!(region(&reopened, &target).translation, Some(saved));
    let sql = Connection::open(&db.0).unwrap();
    let version: u32 = sql
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
    assert_eq!(reopened.snapshot().schema_version, 1);
}

#[test]
fn drawings_preserve_completed_translation_but_stale_completion_is_rejected() {
    let mut store = Store::open_in_memory().unwrap();
    let mut target = prepare(&mut store);
    let original_target = target.clone();
    let saved = save(&mut store, &target);
    let mut object = drawing(DrawingKind::Pen);
    add(&mut store, &target, object.clone());
    for action in 0..4 {
        target = current(&store, &target);
        let kept = region(&store, &target).translation.unwrap();
        assert_eq!(kept.overlay, saved.overlay);
        assert_eq!(kept.drawing_revision, target.drawing_revision);
        let command = match action {
            0 => {
                object.color = "#998877".into();
                SceneCommand::UpdateDrawing {
                    scene_id: target.scene_id.clone(),
                    region_id: target.region_id.clone(),
                    background_id: target.background_id.clone(),
                    expected_revision: target.drawing_revision,
                    drawing: object.clone(),
                }
            }
            1 => SceneCommand::UndoDrawing {
                scene_id: target.scene_id.clone(),
                region_id: target.region_id.clone(),
                background_id: target.background_id.clone(),
                expected_revision: target.drawing_revision,
            },
            2 => SceneCommand::RedoDrawing {
                scene_id: target.scene_id.clone(),
                region_id: target.region_id.clone(),
                background_id: target.background_id.clone(),
                expected_revision: target.drawing_revision,
            },
            _ => SceneCommand::RemoveDrawing {
                scene_id: target.scene_id.clone(),
                region_id: target.region_id.clone(),
                background_id: target.background_id.clone(),
                expected_revision: target.drawing_revision,
                drawing_id: object.id.clone(),
            },
        };
        store.apply(command).unwrap();
    }
    let kept = region(&store, &target).translation.unwrap();
    assert_eq!(kept.overlay, saved.overlay);
    assert_eq!(
        kept.drawing_revision,
        current(&store, &target).drawing_revision
    );
    assert!(matches!(
        store.set_region_translation(
            &original_target,
            saved.document.clone(),
            image(600, 400),
            saved.selection
        ),
        Err(CoreError::TranslationConflict)
    ));
}

#[test]
fn clean_source_keeps_privacy_mosaics_and_override_pixel_coordinates() {
    let mut store = Store::open_in_memory().unwrap();
    let mut target = prepare(&mut store);
    let mosaic = drawing(DrawingKind::Mosaic);
    add(&mut store, &target, mosaic.clone());
    target = current(&store, &target);
    add(&mut store, &target, drawing(DrawingKind::Pen));
    target = current(&store, &target);
    save(&mut store, &target);
    let (source, input) = store.region_for_translation(&target).unwrap();
    assert_eq!(source.id, target.background_id);
    assert_eq!(input.drawings, vec![mosaic]);
    assert!(input.translation.is_none() && input.ocr.is_none() && input.drawing_history.is_empty());
    let replacement = image(300, 1200);
    store
        .set_region_image(&target, replacement.clone())
        .unwrap();
    target = current(&store, &target);
    assert!(region(&store, &target).translation.is_none());
    let (source, input) = store.region_for_translation(&target).unwrap();
    assert_eq!(source, replacement);
    assert_eq!(
        (input.x, input.y, input.width, input.height),
        (0., 0., 300., 1200.)
    );
    let saved = save(&mut store, &target);
    assert_eq!(saved.source_id, replacement.id);
    assert_eq!((saved.document.width, saved.document.height), (300, 1200));
}

#[test]
fn target_geometry_and_host_owned_data_cannot_be_forged() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    let saved = save(&mut store, &target);
    let before = store.snapshot();
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
        assert!(matches!(
            store.region_for_translation(&stale),
            Err(CoreError::TranslationConflict)
        ));
        assert!(matches!(
            store.clear_region_translation(&stale),
            Err(CoreError::TranslationConflict)
        ));
        assert!(matches!(
            store.set_region_translation(
                &stale,
                saved.document.clone(),
                image(600, 400),
                saved.selection.clone()
            ),
            Err(CoreError::TranslationConflict)
        ));
    }
    assert_eq!(store.snapshot(), before);
    let mut forged = region(&store, &target);
    forged.translation.as_mut().unwrap().source_id = "forged".into();
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: forged.clone(),
        })
        .unwrap();
    assert_eq!(region(&store, &target).translation, Some(saved));
    forged.id = Uuid::new_v4().to_string();
    assert!(store
        .apply(SceneCommand::AddRegion {
            scene_id: target.scene_id.clone(),
            region: forged
        })
        .is_err());
    let mut moved = region(&store, &target);
    moved.x += 1.;
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: moved,
        })
        .unwrap();
    assert!(region(&store, &target).translation.is_none());
}

#[test]
fn malformed_documents_and_mismatched_raster_selection_are_atomic() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    let saved = save(&mut store, &target);
    let before = store.snapshot();
    for field in 0..10 {
        let mut doc = saved.document.clone();
        match field {
            0 => doc.version = 2,
            1 => doc.target_language = "xx".into(),
            2 => doc.width = 0,
            3 => doc.lines.push(doc.lines[0].clone()),
            4 => doc.lines[0].text = "x".repeat(2049),
            5 => doc.lines[0].r#box.x = f64::NAN,
            6 => doc.lines[0].r#box.width = 9999.,
            7 => doc.lines[0].id = "<script>".into(),
            8 => doc.lines[0].source = " ".into(),
            _ => doc.lines = vec![doc.lines[0].clone(); 129],
        }
        assert!(
            validate_translation_document(&doc).is_err(),
            "field {field}"
        );
    }
    for field in 0..6 {
        let mut overlay = image(600, 400);
        let mut selected = saved.selection.clone();
        match field {
            0 => overlay.width = Some(599),
            1 => overlay.id = target.background_id.clone(),
            2 => selected.lines[0].text = "伪造".into(),
            3 => selected.lines[0].words[0].text = "不完整".into(),
            4 => selected.engine = "other".into(),
            _ => selected.text_angle = Some(1.),
        }
        assert!(store
            .set_region_translation(&target, saved.document.clone(), overlay, selected)
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
}

#[test]
fn failed_sqlite_write_and_second_writer_do_not_replace_completed_translation() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    save(&mut store, &target);
    let before = store.snapshot();
    let sql = Connection::open(&db.0).unwrap();
    sql.execute_batch("CREATE TRIGGER deny_translation BEFORE UPDATE ON app_state BEGIN SELECT RAISE(FAIL, 'synthetic'); END;").unwrap();
    let doc = document(600, 400);
    assert!(store
        .set_region_translation(&target, doc.clone(), image(600, 400), selection(&doc))
        .is_err());
    assert!(store.clear_region_translation(&target).is_err());
    assert_eq!(store.snapshot(), before);
    sql.execute_batch("DROP TRIGGER deny_translation;").unwrap();
    let mut other = Store::open(&db.0).unwrap();
    let saved = save(&mut other, &target);
    assert!(matches!(
        store.clear_region_translation(&target),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(store);
    drop(other);
    assert_eq!(
        region(&Store::open(&db.0).unwrap(), &target).translation,
        Some(saved)
    );
}

#[test]
fn legacy_defaults_and_corrupt_persisted_translation_fail_without_overwriting() {
    let old: Region =
        serde_json::from_value(serde_json::json!({"id":"old","x":0,"y":0,"width":20,"height":20}))
            .unwrap();
    assert!(old.translation.is_none());
    assert!(serde_json::to_value(old)
        .unwrap()
        .get("translation")
        .is_none());
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = prepare(&mut store);
    save(&mut store, &target);
    drop(store);
    let sql = Connection::open(&db.0).unwrap();
    let payload: String = sql
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&payload).unwrap();
    json["scenes"][0]["regions"][0]["translation"]["sourceId"] = serde_json::json!("forged");
    let broken = serde_json::to_string(&json).unwrap();
    sql.execute("UPDATE app_state SET payload=?1 WHERE id=1", [&broken])
        .unwrap();
    assert!(Store::open(&db.0).is_err());
    let after: String = sql
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, broken);
}

#[test]
fn committing_and_removing_translation_invalidate_visible_ocr_only() {
    let mut store = Store::open_in_memory().unwrap();
    let target = prepare(&mut store);
    let doc = document(600, 400);
    let recognized = selection(&doc);
    store.set_region_ocr(&target, recognized.clone()).unwrap();
    let saved = save(&mut store, &target);
    assert!(region(&store, &target).ocr.is_none());
    assert_eq!(
        store.region_for_ocr(&target).unwrap().1.translation,
        Some(saved.clone())
    );
    assert!(store
        .region_for_translation(&target)
        .unwrap()
        .1
        .translation
        .is_none());
    store.set_region_ocr(&target, recognized.clone()).unwrap();
    assert_eq!(region(&store, &target).translation, Some(saved.clone()));
    store
        .set_region_translation(
            &target,
            saved.document.clone(),
            saved.overlay.clone(),
            saved.selection.clone(),
        )
        .unwrap();
    assert!(region(&store, &target).ocr.is_none());
    store.set_region_ocr(&target, recognized).unwrap();
    store.clear_region_translation(&target).unwrap();
    let region = region(&store, &target);
    assert!(region.ocr.is_none() && region.translation.is_none());
}
