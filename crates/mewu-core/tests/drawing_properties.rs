// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-drawing-properties-{}", Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn stored(&self) -> (String, i64) {
        Connection::open(&self.0)
            .unwrap()
            .query_row(
                "SELECT payload, revision FROM app_state WHERE id=1",
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
fn setup(store: &mut Store) -> OcrTarget {
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
            scene_id: scene_id.clone(),
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
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: "待发送草稿".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id,
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: target.region_id.clone(),
            }],
        })
        .unwrap();
    target
}
fn region(store: &Store, target: &OcrTarget) -> Region {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap()
        .regions
        .into_iter()
        .find(|region| region.id == target.region_id)
        .unwrap()
}
fn current(store: &Store, target: &OcrTarget) -> OcrTarget {
    let value = region(store, target);
    OcrTarget {
        drawing_revision: value.drawing_revision,
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
        ..target.clone()
    }
}
fn object(kind: DrawingKind) -> Drawing {
    Drawing {
        origin: None,
        rich: None,
        id: Uuid::new_v4().to_string(),
        kind,
        color: "#EE4848".into(),
        stroke_width: 3.,
        points: if kind == DrawingKind::Text {
            vec![DrawingPoint { x: 100., y: 110. }]
        } else {
            vec![
                DrawingPoint { x: 100., y: 110. },
                DrawingPoint { x: 180., y: 160. },
            ]
        },
        text: (kind == DrawingKind::Text).then(|| "原文🙂\n第二行".into()),
        font_size: (kind == DrawingKind::Text).then_some(24.),
    }
}
fn add(store: &mut Store, target: &OcrTarget, drawing: Drawing) {
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region_id.clone(),
            background_id: target.background_id.clone(),
            expected_revision: region(store, target).drawing_revision,
            drawing,
        })
        .unwrap();
}
fn update(target: &OcrTarget, drawing: Drawing) -> SceneCommand {
    SceneCommand::UpdateDrawing {
        scene_id: target.scene_id.clone(),
        region_id: target.region_id.clone(),
        background_id: target.background_id.clone(),
        expected_revision: target.drawing_revision,
        drawing,
    }
}
fn history(store: &mut Store, target: &OcrTarget, redo: bool) {
    let revision = region(store, target).drawing_revision;
    store
        .apply(if redo {
            SceneCommand::RedoDrawing {
                scene_id: target.scene_id.clone(),
                region_id: target.region_id.clone(),
                background_id: target.background_id.clone(),
                expected_revision: revision,
            }
        } else {
            SceneCommand::UndoDrawing {
                scene_id: target.scene_id.clone(),
                region_id: target.region_id.clone(),
                background_id: target.background_id.clone(),
                expected_revision: revision,
            }
        })
        .unwrap();
}
fn empty_ocr(width: u32, height: u32) -> OcrDocument {
    OcrDocument {
        engine: "fixture".into(),
        language: "zh-Hans".into(),
        width,
        height,
        text_angle: None,
        lines: vec![],
    }
}

#[test]
fn selected_shape_properties_are_one_edit_without_moving_or_reordering_objects() {
    for kind in [
        DrawingKind::Pen,
        DrawingKind::Line,
        DrawingKind::Arrow,
        DrawingKind::Rect,
        DrawingKind::Ellipse,
        DrawingKind::Highlighter,
    ] {
        let mut store = Store::open_in_memory().unwrap();
        let target = setup(&mut store);
        add(&mut store, &target, object(DrawingKind::Text));
        add(&mut store, &target, object(kind));
        let before = region(&store, &target);
        let snapshot = store.snapshot();
        let mut changed = before.drawings[1].clone();
        changed.color = "#1245AB".into();
        changed.stroke_width = 9.5;
        store
            .apply(update(&current(&store, &target), changed.clone()))
            .unwrap();
        let after = region(&store, &target);
        assert_eq!(
            after.drawings,
            vec![before.drawings[0].clone(), changed.clone()]
        );
        assert_eq!(RegionGeometry::from(&after), RegionGeometry::from(&before));
        assert_eq!(after.drawing_revision, before.drawing_revision + 1);
        assert_eq!(
            after.drawing_history.undo.len(),
            before.drawing_history.undo.len() + 1
        );
        assert_eq!(
            after.drawing_history.undo.last().unwrap(),
            &DrawingStep::Single(DrawingEdit {
                index: 1,
                before: Some(before.drawings[1].clone()),
                after: Some(changed.clone()),
            })
        );
        let scene = &store.snapshot().scenes[0];
        assert_eq!(scene.draft, snapshot.scenes[0].draft);
        assert_eq!(scene.refs, snapshot.scenes[0].refs);
        assert_eq!(scene.geometry_history, snapshot.scenes[0].geometry_history);
        history(&mut store, &target, false);
        assert_eq!(region(&store, &target).drawings, before.drawings);
        history(&mut store, &target, true);
        assert_eq!(region(&store, &target).drawings[1], changed);
    }
}

#[test]
fn text_edit_and_style_share_one_undo_and_interleaved_move_replays_after_restart() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let original = object(DrawingKind::Text);
    add(&mut store, &target, original.clone());
    let mut edited = original.clone();
    edited.text = Some("<svg>纯文本 & e\u{301} 👩‍💻\n修改后第二行".into());
    edited.color = "#123456".into();
    edited.font_size = Some(32.);
    store
        .apply(update(&current(&store, &target), edited.clone()))
        .unwrap();
    assert_eq!(region(&store, &target).drawing_history.undo.len(), 2);
    let mut moved = edited.clone();
    moved.points[0].x += 14.;
    store
        .apply(update(&current(&store, &target), moved.clone()))
        .unwrap();
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    history(&mut store, &target, false);
    assert_eq!(region(&store, &target).drawings, vec![edited.clone()]);
    history(&mut store, &target, false);
    assert_eq!(region(&store, &target).drawings, vec![original]);
    history(&mut store, &target, true);
    assert_eq!(region(&store, &target).drawings, vec![edited]);
    history(&mut store, &target, true);
    assert_eq!(region(&store, &target).drawings, vec![moved]);
}

#[test]
fn same_value_is_a_true_noop_preserving_redo_ocr_and_database_revision() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let original = object(DrawingKind::Text);
    add(&mut store, &target, original.clone());
    let mut changed = original.clone();
    changed.font_size = Some(36.);
    store
        .apply(update(&current(&store, &target), changed))
        .unwrap();
    history(&mut store, &target, false);
    let now = current(&store, &target);
    store.set_region_ocr(&now, empty_ocr(600, 400)).unwrap();
    let before = store.snapshot();
    let persisted = file.stored();
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER forbid_edit BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(store.apply(update(&now, original)).unwrap(), before);
    assert_eq!(file.stored(), persisted);
    let mut stale = now;
    stale.drawing_revision -= 1;
    assert!(matches!(
        store.apply(update(&stale, region(&store, &target).drawings[0].clone())),
        Err(CoreError::DrawingConflict)
    ));
    assert_eq!(store.snapshot(), before);
}

#[test]
fn frozen_override_properties_keep_latest_translation_and_source_but_invalidate_old_targets() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let source = image(300, 1200);
    store.set_region_image(&target, source.clone()).unwrap();
    let mut value = object(DrawingKind::Text);
    value.points[0] = DrawingPoint { x: 125., y: 950. };
    add(&mut store, &target, value.clone());
    let captured = current(&store, &target);
    let document = TranslationDocument {
        version: 1,
        target_language: "en".into(),
        width: 300,
        height: 1200,
        text_angle: None,
        lines: vec![TranslationLine {
            id: "line_0".into(),
            source: "原文".into(),
            text: "translated".into(),
            r#box: TranslationBox {
                x: 10.,
                y: 20.,
                width: 100.,
                height: 20.,
            },
        }],
    };
    let selection = OcrDocument {
        engine: "mewu.translation.layout.v1".into(),
        language: "en".into(),
        width: 300,
        height: 1200,
        text_angle: None,
        lines: vec![OcrLine {
            text: "translated".into(),
            words: vec![OcrWord {
                text: "translated".into(),
                x: 10.,
                y: 20.,
                width: 100.,
                height: 20.,
            }],
        }],
    };
    store
        .set_region_translation(&captured, document, image(300, 1200), selection)
        .unwrap();
    store
        .set_region_ocr(&captured, empty_ocr(300, 1200))
        .unwrap();
    let saved = region(&store, &target);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    let export = ExportView::open(&file.0).unwrap();
    value.color = "#00AAFF".into();
    value.text = Some("编辑已有文字".into());
    store.apply(update(&captured, value.clone())).unwrap();
    let after = region(&store, &target);
    assert_eq!(after.image_override, Some(source.clone()));
    assert_eq!(RegionGeometry::from(&after), RegionGeometry::from(&saved));
    assert_eq!(after.drawings, vec![value.clone()]);
    assert!(after.ocr.is_none());
    let mut expected_translation = saved.translation.clone().unwrap();
    expected_translation.drawing_revision += 1;
    assert_eq!(after.translation, Some(expected_translation));
    assert_eq!(store.snapshot().active_scene_id, active);
    assert_eq!(
        export
            .snapshot()
            .scenes
            .iter()
            .find(|scene| scene.id == target.scene_id)
            .unwrap()
            .regions[0],
        saved
    );
    assert!(store.region_for_ocr(&captured).is_err());
    assert!(store.apply(update(&captured, value.clone())).is_err());
    let now = current(&store, &target);
    let (actual_source, rendered_region) = store.region_for_ocr(&now).unwrap();
    assert_eq!(actual_source, source);
    assert_eq!(
        (
            rendered_region.x,
            rendered_region.y,
            rendered_region.width,
            rendered_region.height
        ),
        (0., 0., 300., 1200.)
    );
    assert_eq!(rendered_region.drawings, vec![value.clone()]);
    let mut wrong_source = now.clone();
    wrong_source.background_id = actual_source.id;
    assert!(store.apply(update(&wrong_source, value)).is_err());
    history(&mut store, &target, false);
    assert_eq!(region(&store, &target).drawings, saved.drawings);
    assert_eq!(
        region(&store, &target).translation.unwrap().overlay,
        saved.translation.unwrap().overlay
    );
    assert_eq!(store.snapshot().active_scene_id, active);
}

#[test]
fn equal_update_from_a_stale_store_rejects_without_writing_or_returning_a_stale_success() {
    let file = TestDb::new();
    let mut winner = Store::open(&file.0).unwrap();
    let target = setup(&mut winner);
    let original = object(DrawingKind::Text);
    add(&mut winner, &target, original.clone());
    let now = current(&winner, &target);
    let mut stale = Store::open(&file.0).unwrap();
    let stale_snapshot = stale.snapshot();
    let mut changed = original.clone();
    changed.color = "#012345".into();
    winner.apply(update(&now, changed)).unwrap();
    let committed = file.stored();
    assert!(matches!(
        stale.apply(update(&now, original)),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), stale_snapshot);
    assert_eq!(file.stored(), committed);
}

#[test]
fn failed_property_commit_is_atomic_and_second_writer_cannot_overwrite_it() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    add(&mut store, &target, object(DrawingKind::Text));
    let now = current(&store, &target);
    let mut changed = region(&store, &target).drawings[0].clone();
    changed.text = Some("提交后文本".into());
    changed.font_size = Some(42.);
    let command = update(&now, changed.clone());
    let before = store.snapshot();
    let persisted = file.stored();
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER fail_properties BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store.apply(command.clone()).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(file.stored(), persisted);
    db.execute_batch("DROP TRIGGER fail_properties;").unwrap();
    let mut stale = Store::open(&file.0).unwrap();
    store.apply(command).unwrap();
    let committed = file.stored();
    let mut conflicting = changed;
    conflicting.color = "#112233".into();
    assert!(matches!(
        stale.apply(update(&now, conflicting)),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), before);
    assert_eq!(file.stored(), committed);
    assert_eq!(region(&store, &target).drawing_history.undo.len(), 2);
}

#[test]
fn invalid_values_deleted_identity_closed_scene_and_replaced_source_reject_without_partial_style() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store);
    let original = object(DrawingKind::Text);
    add(&mut store, &target, original.clone());
    let now = current(&store, &target);
    let before = store.snapshot();
    let mut cases = Vec::new();
    let mut value = original.clone();
    value.color = "red".into();
    cases.push(value);
    let mut value = original.clone();
    value.font_size = Some(257.);
    cases.push(value);
    let mut value = original.clone();
    value.text = Some(" ".into());
    cases.push(value);
    let mut value = original.clone();
    value.text = Some("x".repeat(2001));
    cases.push(value);
    let mut value = original.clone();
    value.stroke_width = f64::NAN;
    cases.push(value);
    let mut value = original.clone();
    value.id = Uuid::new_v4().to_string();
    cases.push(value);
    for value in cases {
        assert!(store.apply(update(&now, value)).is_err());
        assert_eq!(store.snapshot(), before);
    }
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: now.scene_id.clone(),
            region_id: now.region_id.clone(),
            background_id: now.background_id.clone(),
            expected_revision: now.drawing_revision,
            drawing_id: original.id.clone(),
        })
        .unwrap();
    assert!(store
        .apply(update(&current(&store, &target), original.clone()))
        .is_err());
    history(&mut store, &target, false);
    let before_replacement = current(&store, &target);
    store
        .set_region_image(&before_replacement, image(300, 1200))
        .unwrap();
    let replacement = store.snapshot();
    assert!(store
        .apply(update(&before_replacement, original.clone()))
        .is_err());
    assert_eq!(store.snapshot(), replacement);
    add(&mut store, &target, original.clone());
    let closed_target = current(&store, &target);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let closed = store.snapshot();
    assert!(store.apply(update(&closed_target, original)).is_err());
    assert_eq!(store.snapshot(), closed);
}
