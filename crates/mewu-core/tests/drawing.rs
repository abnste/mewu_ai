// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("mewu-drawing-{}", Uuid::new_v4()));
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

fn prepare(store: &mut Store) -> (String, String, String) {
    let scene = store.snapshot().active_scene_id;
    let background = Uuid::new_v4().to_string();
    store
        .set_background(
            &scene,
            Asset {
                id: background.clone(),
                name: "drawing-test.png".into(),
                kind: AssetKind::Image,
                path: "drawing-test.png".into(),
                width: Some(800),
                height: Some(600),
                origin_x: Some(-800),
                origin_y: Some(0),
                scale_factor: Some(1.5),
            },
        )
        .unwrap();
    let region = Uuid::new_v4().to_string();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene.clone(),
            region: Region {
                id: region.clone(),
                x: 20.,
                y: 30.,
                width: 600.,
                height: 400.,
                ..Default::default()
            },
        })
        .unwrap();
    (scene, background, region)
}
fn object(kind: DrawingKind) -> Drawing {
    Drawing {
        origin: None,
        rich: None,
        id: Uuid::new_v4().to_string(),
        kind,
        color: "#FF3355".into(),
        stroke_width: if kind == DrawingKind::Mosaic { 12. } else { 3. },
        points: if matches!(kind, DrawingKind::Text | DrawingKind::Number) {
            vec![DrawingPoint { x: 100., y: 110. }]
        } else {
            vec![
                DrawingPoint { x: 100., y: 110. },
                DrawingPoint { x: 180., y: 160. },
            ]
        },
        text: match kind {
            DrawingKind::Text => Some("你好🙂\n第二行".into()),
            DrawingKind::Number => Some("1234".into()),
            _ => None,
        },
        font_size: matches!(kind, DrawingKind::Text | DrawingKind::Number).then_some(24.),
    }
}
fn region(store: &Store, scope: &(String, String, String)) -> Region {
    store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == scope.0)
        .unwrap()
        .regions
        .iter()
        .find(|r| r.id == scope.2)
        .unwrap()
        .clone()
}
fn add(
    store: &mut Store,
    scope: &(String, String, String),
    drawing: Drawing,
) -> Result<Snapshot, CoreError> {
    store.apply(SceneCommand::AddDrawing {
        scene_id: scope.0.clone(),
        region_id: scope.2.clone(),
        background_id: scope.1.clone(),
        expected_revision: region(store, scope).drawing_revision,
        drawing,
    })
}
fn history(
    store: &mut Store,
    scope: &(String, String, String),
    redo: bool,
) -> Result<Snapshot, CoreError> {
    let expected_revision = region(store, scope).drawing_revision;
    store.apply(if redo {
        SceneCommand::RedoDrawing {
            scene_id: scope.0.clone(),
            region_id: scope.2.clone(),
            background_id: scope.1.clone(),
            expected_revision,
        }
    } else {
        SceneCommand::UndoDrawing {
            scene_id: scope.0.clone(),
            region_id: scope.2.clone(),
            background_id: scope.1.clone(),
            expected_revision,
        }
    })
}

#[test]
fn five_editable_tools_keep_layer_order_and_undo_redo_branching() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    for kind in [
        DrawingKind::Pen,
        DrawingKind::Arrow,
        DrawingKind::Rect,
        DrawingKind::Ellipse,
        DrawingKind::Text,
    ] {
        add(&mut store, &scope, object(kind)).unwrap();
    }
    let original = region(&store, &scope);
    let mut changed = original.drawings[0].clone();
    changed.color = "#00AAFF".into();
    changed.points.iter_mut().for_each(|p| p.x += 12.);
    store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: scope.0.clone(),
            region_id: scope.2.clone(),
            background_id: scope.1.clone(),
            expected_revision: original.drawing_revision,
            drawing: changed.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: scope.0.clone(),
            region_id: scope.2.clone(),
            background_id: scope.1.clone(),
            expected_revision: region(&store, &scope).drawing_revision,
            drawing_id: original.drawings[3].id.clone(),
        })
        .unwrap();
    assert_eq!(region(&store, &scope).drawings.len(), 4);
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings[3], original.drawings[3]);
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, original.drawings);
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings[0], changed);
    history(&mut store, &scope, false).unwrap();
    add(&mut store, &scope, object(DrawingKind::Text)).unwrap();
    assert!(region(&store, &scope).drawing_history.redo.is_empty());
    let before = region(&store, &scope);
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope), before);
}

#[test]
fn additional_tools_keep_cas_undo_and_frozen_restart_documents() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let scope = prepare(&mut store);
    for kind in [
        DrawingKind::Line,
        DrawingKind::Highlighter,
        DrawingKind::Number,
        DrawingKind::Mosaic,
    ] {
        add(&mut store, &scope, object(kind)).unwrap();
    }
    let original = region(&store, &scope);
    let mut changed = original.drawings[2].clone();
    changed.text = Some("9999".into());
    changed.font_size = Some(64.);
    let update = SceneCommand::UpdateDrawing {
        scene_id: scope.0.clone(),
        region_id: scope.2.clone(),
        background_id: scope.1.clone(),
        expected_revision: original.drawing_revision,
        drawing: changed.clone(),
    };
    store.apply(update.clone()).unwrap();
    assert!(matches!(
        store.apply(update),
        Err(CoreError::DrawingConflict)
    ));
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, original.drawings);
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings[2], changed);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    history(&mut store, &scope, false).unwrap();
    let saved = region(&store, &scope);
    drop(store);
    let mut reopened = Store::open(&file.0).unwrap();
    assert_eq!(region(&reopened, &scope), saved);
    assert_eq!(reopened.snapshot().active_scene_id, active);
    history(&mut reopened, &scope, true).unwrap();
    assert_eq!(region(&reopened, &scope).drawings[2], changed);
    assert_eq!(reopened.snapshot().active_scene_id, active);
}

#[test]
fn additional_tool_validation_rejects_ambiguous_numbers_and_invalid_mosaic_without_mutation() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let before = store.snapshot();
    let mut cases = Vec::new();
    for text in [
        "", "0", "00", "01", "+1", "-1", "1.0", "1 ", " 1", "1\n", "１", "10000", "<1>",
    ] {
        let mut value = object(DrawingKind::Number);
        value.text = Some(text.into());
        cases.push(value);
    }
    for size in [21.99, 64.01, f64::NAN, f64::INFINITY] {
        let mut value = object(DrawingKind::Number);
        value.font_size = Some(size);
        cases.push(value);
    }
    let mut missing = object(DrawingKind::Number);
    missing.font_size = None;
    cases.push(missing);
    let mut extra = object(DrawingKind::Number);
    extra.points.push(DrawingPoint { x: 120., y: 130. });
    cases.push(extra);
    for size in [5., 6.5, 40.1, 41., f64::NAN] {
        let mut value = object(DrawingKind::Mosaic);
        value.stroke_width = size;
        cases.push(value);
    }
    for kind in [DrawingKind::Line, DrawingKind::Mosaic] {
        let mut value = object(kind);
        value.points[1] = value.points[0];
        cases.push(value);
    }
    for kind in [
        DrawingKind::Line,
        DrawingKind::Highlighter,
        DrawingKind::Mosaic,
    ] {
        let mut value = object(kind);
        value.text = Some("1".into());
        cases.push(value);
        let mut value = object(kind);
        value.font_size = Some(24.);
        cases.push(value);
    }
    for count in [0, 4097] {
        let mut value = object(DrawingKind::Highlighter);
        value.points.resize(count, DrawingPoint { x: 10., y: 10. });
        cases.push(value);
    }
    for value in cases {
        assert!(add(&mut store, &scope, value).is_err());
        assert_eq!(store.snapshot(), before);
    }
    for (text, diameter) in [("1", 22.), ("9999", 64.)] {
        let mut value = object(DrawingKind::Number);
        value.text = Some(text.into());
        value.font_size = Some(diameter);
        add(&mut store, &scope, value).unwrap();
    }
    for block in [6., 40.] {
        let mut value = object(DrawingKind::Mosaic);
        value.stroke_width = block;
        add(&mut store, &scope, value).unwrap();
    }
    let mut dot = object(DrawingKind::Highlighter);
    dot.points.truncate(1);
    add(&mut store, &scope, dot).unwrap();
    let mut full_path = object(DrawingKind::Highlighter);
    full_path
        .points
        .resize(4096, DrawingPoint { x: 10., y: 10. });
    add(&mut store, &scope, full_path).unwrap();
}

#[test]
fn additional_tools_rollback_and_corrupt_persisted_document_is_not_overwritten() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let scope = prepare(&mut store);
    add(&mut store, &scope, object(DrawingKind::Number)).unwrap();
    let before = store.snapshot();
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER fail_drawing BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    for kind in [
        DrawingKind::Line,
        DrawingKind::Highlighter,
        DrawingKind::Mosaic,
    ] {
        assert!(add(&mut store, &scope, object(kind)).is_err());
        assert_eq!(store.snapshot(), before);
    }
    assert!(history(&mut store, &scope, false).is_err());
    assert_eq!(store.snapshot(), before);
    db.execute_batch("DROP TRIGGER fail_drawing").unwrap();
    let mut corrupt = before;
    let region = &mut corrupt
        .scenes
        .iter_mut()
        .find(|s| s.id == scope.0)
        .unwrap()
        .regions[0];
    region.drawings[0].text = Some("0".into());
    let DrawingStep::Single(edit) = &mut region.drawing_history.undo[0] else {
        panic!("single edit")
    };
    edit.after.as_mut().unwrap().text = Some("0".into());
    let payload = serde_json::to_string(&corrupt).unwrap();
    db.execute("UPDATE app_state SET payload=?", [&payload])
        .unwrap();
    drop(store);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(
        db.query_row("SELECT payload FROM app_state", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        payload
    );
}

#[test]
fn background_preview_requires_current_scene_identity_but_allows_frozen_reading() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let expected = store.background_for_preview(&scope.0, &scope.1).unwrap();
    let before = store.snapshot();
    assert!(store.background_for_preview("missing", &scope.1).is_err());
    assert!(store
        .background_for_preview(&scope.0, "other-background")
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let frozen = store.snapshot();
    assert_eq!(
        store.background_for_preview(&scope.0, &scope.1).unwrap(),
        expected
    );
    assert!(store
        .background_for_preview(&frozen.active_scene_id, &scope.1)
        .is_err());
    assert_eq!(store.snapshot(), frozen);
    let mut replacement = expected.clone();
    replacement.id = Uuid::new_v4().to_string();
    store.set_background(&scope.0, replacement.clone()).unwrap();
    assert!(store.background_for_preview(&scope.0, &scope.1).is_err());
    assert_eq!(
        store
            .background_for_preview(&scope.0, &replacement.id)
            .unwrap(),
        replacement
    );
    store
        .apply(SceneCommand::CloseScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    assert!(store
        .background_for_preview(&scope.0, &replacement.id)
        .is_err());
}

#[test]
fn oversized_background_still_supports_vectors_but_cannot_commit_an_unpreviewable_mosaic() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let mut background = store.background_for_preview(&scope.0, &scope.1).unwrap();
    background.width = Some(8000);
    background.height = Some(5000);
    let old_region = region(&store, &scope);
    store.set_background(&scope.0, background).unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scope.0.clone(),
            region: old_region,
        })
        .unwrap();
    add(&mut store, &scope, object(DrawingKind::Line)).unwrap();
    let before = store.snapshot();
    assert!(add(&mut store, &scope, object(DrawingKind::Mosaic)).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn active_background_sampling_is_fenced_and_never_mutates_memory_or_storage() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scope = prepare(&mut store);
    let original = store.background_for_preview(&scope.0, &scope.1).unwrap();
    let sql = Connection::open(&db.0).unwrap();
    let persisted = || -> (String, i64) {
        sql.query_row(
            "SELECT payload, revision FROM app_state WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    let check = |store: &Store, scene: &str, background: &str, expected: Option<&Asset>| {
        let before = store.snapshot();
        let database = persisted();
        let result = store.active_background_for_preview(scene, background);
        if let Some(expected) = expected {
            assert_eq!(&result.unwrap(), expected);
        } else {
            assert!(result.is_err());
        }
        assert_eq!(store.snapshot(), before);
        assert_eq!(persisted(), database);
    };
    check(&store, &scope.0, &scope.1, Some(&original));
    check(&store, &scope.0, "replaced-background", None);
    check(&store, "other-scene", &scope.1, None);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    check(&store, &scope.0, &scope.1, None);
    assert_eq!(
        store.background_for_preview(&scope.0, &scope.1).unwrap(),
        original
    );
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    check(&store, &scope.0, &scope.1, Some(&original));
    let replacement = Asset {
        id: Uuid::new_v4().to_string(),
        ..original
    };
    store.set_background(&scope.0, replacement.clone()).unwrap();
    check(&store, &scope.0, &scope.1, None);
    check(&store, &scope.0, &replacement.id, Some(&replacement));
    store
        .apply(SceneCommand::CloseScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    check(&store, &scope.0, &replacement.id, None);
}

#[test]
fn geometry_updates_preserve_layers_and_reject_stale_region_background_and_revision() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let mut stale = region(&store, &scope);
    add(&mut store, &scope, object(DrawingKind::Arrow)).unwrap();
    let document = region(&store, &scope);
    stale.width = 40.; // Cropping does not move or discard background-pixel markup.
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: scope.0.clone(),
            region: stale,
        })
        .unwrap();
    let moved = region(&store, &scope);
    assert_eq!(moved.drawings, document.drawings);
    assert_eq!(moved.drawing_history, document.drawing_history);
    assert_eq!(moved.drawing_revision, document.drawing_revision + 1);
    for (background_id, region_id, expected_revision) in [
        (scope.1.clone(), scope.2.clone(), document.drawing_revision),
        (
            Uuid::new_v4().to_string(),
            scope.2.clone(),
            moved.drawing_revision,
        ),
        (
            scope.1.clone(),
            Uuid::new_v4().to_string(),
            moved.drawing_revision,
        ),
    ] {
        let before = store.snapshot();
        assert!(matches!(
            store.apply(SceneCommand::AddDrawing {
                scene_id: scope.0.clone(),
                region_id,
                background_id,
                expected_revision,
                drawing: object(DrawingKind::Pen),
            }),
            Err(CoreError::DrawingConflict)
        ));
        assert_eq!(store.snapshot(), before);
    }
    let before = store.snapshot();
    let mut forged = document;
    forged.id = Uuid::new_v4().to_string();
    assert!(store
        .apply(SceneCommand::AddRegion {
            scene_id: scope.0.clone(),
            region: forged
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    assert!(add(&mut store, &scope, object(DrawingKind::Pen)).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn frozen_background_drawing_is_persisted_and_does_not_cancel_or_retarget_run() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let scope = prepare(&mut store);
    add(&mut store, &scope, object(DrawingKind::Text)).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scope.0.clone(),
            draft: "分析图片".into(),
        })
        .unwrap();
    let run = store.begin_run(&scope.0).unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    add(&mut store, &scope, object(DrawingKind::Rect)).unwrap();
    history(&mut store, &scope, false).unwrap();
    assert_eq!(store.snapshot().active_scene_id, active);
    assert!(store.run_is_active(&scope.0, &run.run_id));
    assert_eq!(run.regions[0].drawings.len(), 1);
    let document = region(&store, &scope);
    drop(store);
    let mut restored = Store::open(&file.0).unwrap();
    assert_eq!(region(&restored, &scope), document);
    history(&mut restored, &scope, true).unwrap();
    assert_eq!(region(&restored, &scope).drawings.len(), 2);
    assert_eq!(restored.snapshot().active_scene_id, active);
}

#[test]
fn invalid_drawing_input_never_changes_document_or_history() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let before = store.snapshot();
    let mut cases = Vec::new();
    for value in [f64::NAN, f64::INFINITY, -1., 801.] {
        let mut drawing = object(DrawingKind::Pen);
        drawing.points[0].x = value;
        cases.push(drawing);
    }
    for value in [0., 0.49, 64.1, f64::NAN] {
        let mut drawing = object(DrawingKind::Pen);
        drawing.stroke_width = value;
        cases.push(drawing);
    }
    for value in ["red", "#fff", "#FF0000FF", "url(javascript:x)", "#gg0000"] {
        let mut drawing = object(DrawingKind::Pen);
        drawing.color = value.into();
        cases.push(drawing);
    }
    let mut drawing = object(DrawingKind::Pen);
    drawing.points.clear();
    cases.push(drawing);
    let mut drawing = object(DrawingKind::Pen);
    drawing.points.resize(4097, DrawingPoint { x: 1., y: 1. });
    cases.push(drawing);
    let mut drawing = object(DrawingKind::Rect);
    drawing.points[1] = drawing.points[0];
    cases.push(drawing);
    let mut drawing = object(DrawingKind::Arrow);
    drawing.text = Some("unexpected".into());
    cases.push(drawing);
    for value in [" ".into(), "秘密\0文本".into(), "字".repeat(2001)] {
        let mut drawing = object(DrawingKind::Text);
        drawing.text = Some(value);
        cases.push(drawing);
    }
    let mut drawing = object(DrawingKind::Text);
    drawing.font_size = None;
    cases.push(drawing);
    let mut drawing = object(DrawingKind::Text);
    drawing.font_size = Some(257.);
    cases.push(drawing);
    let mut drawing = object(DrawingKind::Pen);
    drawing.id = "not-an-object-id".into();
    cases.push(drawing);
    for drawing in cases {
        assert!(add(&mut store, &scope, drawing).is_err());
        assert_eq!(store.snapshot(), before);
    }
    let mut dot = object(DrawingKind::Pen);
    dot.points.truncate(1);
    add(&mut store, &scope, dot.clone()).unwrap();
    let before = store.snapshot();
    assert!(add(&mut store, &scope, dot).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn drawing_and_history_roll_back_together_on_database_failure_and_stale_writer() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let scope = prepare(&mut store);
    add(&mut store, &scope, object(DrawingKind::Pen)).unwrap();
    let mut stale = Store::open(&file.0).unwrap();
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER fail_drawing BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    let before = store.snapshot();
    let payload: String = db
        .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap();
    assert!(history(&mut store, &scope, false).is_err());
    assert!(add(&mut store, &scope, object(DrawingKind::Text)).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        db.query_row("SELECT payload FROM app_state", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        payload
    );
    db.execute_batch("DROP TRIGGER fail_drawing").unwrap();
    history(&mut store, &scope, false).unwrap();
    let stale_before = stale.snapshot();
    assert!(matches!(
        add(&mut stale, &scope, object(DrawingKind::Arrow)),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), stale_before);
    drop(stale);
    drop(db);
    drop(store);
    let restored = Store::open(&file.0).unwrap();
    assert!(region(&restored, &scope).drawings.is_empty());
    assert_eq!(region(&restored, &scope).drawing_history.redo.len(), 1);
}

#[test]
fn old_region_json_defaults_and_excess_history_is_trimmed_without_losing_objects() {
    let old: Region = serde_json::from_value(
        serde_json::json!({"id":"old-region","x":1,"y":2,"width":20,"height":30}),
    )
    .unwrap();
    assert!(old.drawings.is_empty());
    assert_eq!(old.drawing_revision, 0);
    assert!(old.drawing_history.is_empty());
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    for _ in 0..130 {
        add(&mut store, &scope, object(DrawingKind::Pen)).unwrap();
    }
    assert_eq!(region(&store, &scope).drawings.len(), 130);
    assert_eq!(region(&store, &scope).drawing_history.undo.len(), 128);
    for _ in 0..128 {
        history(&mut store, &scope, false).unwrap();
    }
    assert_eq!(region(&store, &scope).drawings.len(), 2);
    for _ in 0..128 {
        history(&mut store, &scope, true).unwrap();
    }
    assert_eq!(region(&store, &scope).drawings.len(), 130);
}

#[test]
fn region_workflow_preserves_composer_and_records_real_prompt_with_only_selected_reference() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    store
        .apply(SceneCommand::SaveAgent {
            agent: agent.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.id.clone(),
            id: None,
            text: "用户偏好表格".into(),
            expected_revision: None,
        })
        .unwrap();
    let server = store
        .upsert_mcp_server(
            None,
            None,
            "测试工具".into(),
            McpCommand {
                executable: "/fixture/server".into(),
                args: vec![],
                cwd: None,
            },
            vec![McpTool {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }],
        )
        .unwrap()
        .mcp_servers[0]
        .clone();
    store
        .apply(SceneCommand::SetMcpGrants {
            server_id: server.id,
            expected_revision: server.revision,
            agent_id: agent.id,
            tool_names: vec!["echo".into()],
        })
        .unwrap();
    let other = Region {
        id: Uuid::new_v4().to_string(),
        x: 1.,
        y: 2.,
        width: 30.,
        height: 40.,
        ..Default::default()
    };
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scope.0.clone(),
            region: other.clone(),
        })
        .unwrap();
    let composer_refs = vec![Reference {
        kind: ReferenceKind::Region,
        id: other.id,
    }];
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scope.0.clone(),
            refs: composer_refs.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scope.0.clone(),
            draft: "我还没有发送的独立问题".into(),
        })
        .unwrap();
    let prompt = "请将此区域的表格转为 Markdown，保留空单元格。";
    let run = store.begin_workflow(&scope.0, prompt, &scope.2).unwrap();
    let selected = vec![Reference {
        kind: ReferenceKind::Region,
        id: scope.2.clone(),
    }];
    assert_eq!(run.refs, selected);
    assert_eq!(run.messages.last().unwrap().text, prompt);
    assert_eq!(
        run.messages.last().unwrap().refs.as_ref().unwrap(),
        &selected
    );
    assert!(!run.agent.memory_enabled);
    assert!(run.memories.is_empty());
    assert!(run.mcp_servers.is_empty());
    let state = store.snapshot();
    assert!(state.agents[0].memory_enabled);
    assert_eq!(state.scenes[0].draft, "我还没有发送的独立问题");
    assert_eq!(state.scenes[0].refs, composer_refs);
    assert_eq!(state.scenes[0].messages.last().unwrap().text, prompt);
    assert!(matches!(
        store.begin_workflow(&scope.0, prompt, &scope.2),
        Err(CoreError::RunInProgress)
    ));
    assert_eq!(store.snapshot(), state);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    store.finish_run(&scope.0, &run.run_id, "表格结果").unwrap();
    assert_eq!(store.snapshot().active_scene_id, active);
    let normal = store.begin_run(&scope.0).unwrap();
    assert_eq!(
        normal.messages.last().unwrap().text,
        "我还没有发送的独立问题"
    );
    assert_eq!(normal.refs, composer_refs);
    assert!(normal.agent.memory_enabled);
    assert_eq!(normal.mcp_servers.len(), 1);
}

#[test]
fn workflow_failure_preserves_draft_refs_messages_and_database() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let scope = prepare(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scope.0.clone(),
            draft: "保留草稿".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scope.0.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: scope.2.clone(),
            }],
        })
        .unwrap();
    let before = store.snapshot();
    for (prompt, region_id) in [
        ("", scope.2.as_str()),
        ("测试", "missing"),
        ("bad\0prompt", scope.2.as_str()),
    ] {
        assert!(store.begin_workflow(&scope.0, prompt, region_id).is_err());
        assert_eq!(store.snapshot(), before);
    }
    assert!(store
        .begin_workflow(&scope.0, &"字".repeat(16001), &scope.2)
        .is_err());
    assert_eq!(store.snapshot(), before);
    let db = Connection::open(&file.0).unwrap();
    let payload: String = db
        .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail_workflow BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(store
        .begin_workflow(&scope.0, "工作流实际指令", &scope.2)
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        db.query_row("SELECT payload FROM app_state", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        payload
    );
    db.execute_batch("DROP TRIGGER fail_workflow").unwrap();
    store
        .apply(SceneCommand::CloseScene {
            scene_id: scope.0.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    assert!(store.begin_workflow(&scope.0, "测试", &scope.2).is_err());
    assert_eq!(store.snapshot(), before);
}
