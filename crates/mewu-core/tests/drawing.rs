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
                capture_coordinates: None,
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
        font_size: matches!(kind, DrawingKind::Text | DrawingKind::Number).then_some(24.), text_style: None,
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

fn erase_command(scope: &(String, String, String), revision: u64, erasures: Vec<DrawingStrokeErase>) -> SceneCommand {
    SceneCommand::EraseDrawingStrokes {
        scene_id: scope.0.clone(), region_id: scope.2.clone(), background_id: scope.1.clone(),
        expected_revision: revision, erasures,
    }
}

fn split_stroke(drawing: &Drawing) -> DrawingStrokeErase {
    DrawingStrokeErase { drawing_id: drawing.id.clone(), fragments: vec![
        DrawingStrokeFragment { id: drawing.id.clone(), points: vec![drawing.points[0], DrawingPoint { x: 130., y: 128.75 }] },
        DrawingStrokeFragment { id: Uuid::new_v4().to_string(), points: vec![DrawingPoint { x: 150., y: 141.25 }, drawing.points[1]] },
    ] }
}

fn remove_batch_command(scope: &(String, String, String), revision: u64, ids: Vec<String>) -> SceneCommand {
    SceneCommand::RemoveDrawings { scene_id: scope.0.clone(), region_id: scope.2.clone(),
        background_id: scope.1.clone(), expected_revision: revision, drawing_ids: ids }
}
fn database_revision(path: &std::path::Path) -> i64 {
    Connection::open(path).unwrap().query_row("SELECT revision FROM app_state WHERE id=1", [], |row| row.get(0)).unwrap()
}

#[test]
fn batch_remove_commits_one_cas_and_one_reversible_history_step_then_reopens() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let objects = [object(DrawingKind::Pen), object(DrawingKind::Text), object(DrawingKind::Rect), object(DrawingKind::Line)];
    for value in &objects { add(&mut store, &scope, value.clone()).unwrap(); }
    let before = region(&store, &scope); let revision = database_revision(&file.0);
    let command = remove_batch_command(&scope, before.drawing_revision, vec![objects[2].id.clone(), objects[0].id.clone(), objects[3].id.clone()]);
    let wire = serde_json::to_value(&command).unwrap();
    assert_eq!(wire["type"], "remove_drawings");
    assert_eq!(wire["drawingIds"], serde_json::json!([objects[2].id, objects[0].id, objects[3].id]));
    let mut forged = wire.clone(); forged["allowPartial"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SceneCommand>(forged).is_err());
    store.apply(serde_json::from_value(wire).unwrap()).unwrap();
    let removed = region(&store, &scope);
    assert_eq!(database_revision(&file.0), revision + 1);
    assert_eq!(removed.drawing_revision, before.drawing_revision + 1);
    assert_eq!(removed.drawings, vec![objects[1].clone()]);
    assert_eq!(removed.drawing_history.undo.len(), before.drawing_history.undo.len() + 1);
    let DrawingStep::Batch(batch) = removed.drawing_history.undo.last().unwrap() else { panic!("one atomic history batch required") };
    assert_eq!(batch.batch.iter().map(|edit| edit.index).collect::<Vec<_>>(), vec![3, 2, 0]);
    history(&mut store, &scope, false).unwrap(); assert_eq!(region(&store, &scope).drawings, before.drawings);
    history(&mut store, &scope, true).unwrap(); assert_eq!(region(&store, &scope).drawings, removed.drawings);
    let expected = store.snapshot(); drop(store);
    assert_eq!(Store::open(&file.0).unwrap().snapshot(), expected);
}

#[test]
fn batch_remove_rejects_the_entire_invalid_or_stale_selection_without_partial_changes() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let first = object(DrawingKind::Pen); let second = object(DrawingKind::Text);
    add(&mut store, &scope, first.clone()).unwrap(); add(&mut store, &scope, second.clone()).unwrap();
    let before = store.snapshot(); let revision = database_revision(&file.0); let drawing_revision = region(&store, &scope).drawing_revision;
    for ids in [vec![], vec![first.id.clone(), first.id.clone()], vec![first.id.clone(), Uuid::new_v4().to_string()], (0..257).map(|_| Uuid::new_v4().to_string()).collect()] {
        assert!(store.apply(remove_batch_command(&scope, drawing_revision, ids)).is_err());
        assert_eq!(store.snapshot(), before); assert_eq!(database_revision(&file.0), revision);
    }
    assert!(store.apply(remove_batch_command(&scope, drawing_revision - 1, vec![first.id.clone(), second.id.clone()])).is_err());
    let mut wrong = remove_batch_command(&scope, drawing_revision, vec![first.id, second.id]);
    if let SceneCommand::RemoveDrawings { background_id, .. } = &mut wrong { *background_id = Uuid::new_v4().to_string(); }
    assert!(store.apply(wrong).is_err()); assert_eq!(store.snapshot(), before); assert_eq!(database_revision(&file.0), revision);
}

#[test]
fn batch_remove_sqlite_abort_and_stale_writer_leave_every_object_and_history_intact() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let objects = [object(DrawingKind::Pen), object(DrawingKind::Text)];
    for drawing in &objects { add(&mut store, &scope, drawing.clone()).unwrap(); }
    let mut stale = Store::open(&file.0).unwrap(); let before = store.snapshot(); let revision = database_revision(&file.0);
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER reject_batch_remove BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'batch remove rollback'); END;").unwrap();
    let command = remove_batch_command(&scope, region(&store, &scope).drawing_revision, objects.iter().map(|drawing| drawing.id.clone()).collect());
    assert!(store.apply(command.clone()).is_err()); assert_eq!(store.snapshot(), before); assert_eq!(database_revision(&file.0), revision);
    db.execute_batch("DROP TRIGGER reject_batch_remove").unwrap(); store.apply(command.clone()).unwrap();
    let durable = store.snapshot(); let stale_before = stale.snapshot();
    assert!(matches!(stale.apply(command), Err(CoreError::ConcurrentModification)));
    assert_eq!(stale.snapshot(), stale_before); assert_eq!(database_revision(&file.0), revision + 1);
    drop(stale); drop(db); drop(store); assert_eq!(Store::open(&file.0).unwrap().snapshot(), durable);
}

#[test]
fn batch_remove_image_annotations_uses_one_revision_and_undo_preserves_the_media_item() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let source = Asset { id: Uuid::new_v4().to_string(), name: "import.png".into(), kind: AssetKind::Image,
        path: "synthetic-import.png".into(), width: Some(800), height: Some(600), origin_x: None, origin_y: None, scale_factor: None, capture_coordinates: None };
    store.add_asset(&scope.0, source.clone()).unwrap();
    let item_id = store.scene(&scope.0).unwrap().items[0].id.clone();
    let command = |revision, action| SceneCommand::EditImageAnnotations { scene_id: scope.0.clone(), item_id: item_id.clone(), source_asset_id: source.id.clone(), expected_revision: revision, action };
    let values = [object(DrawingKind::Pen), object(DrawingKind::Text)];
    for (index, drawing) in values.iter().enumerate() { store.apply(command(index as u64, ImageAnnotationEdit::AddDrawing { drawing: drawing.clone() })).unwrap(); }
    let before = store.scene(&scope.0).unwrap().items[0].clone(); let revision = database_revision(&file.0);
    let action = ImageAnnotationEdit::RemoveDrawings { drawing_ids: values.iter().map(|drawing| drawing.id.clone()).collect() };
    let wire = serde_json::to_value(action).unwrap();
    assert_eq!(wire, serde_json::json!({ "type": "remove_drawings", "drawingIds": [values[0].id, values[1].id] }));
    let mut forged = wire.clone(); forged["allowPartial"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ImageAnnotationEdit>(forged).is_err());
    store.apply(command(2, serde_json::from_value(wire).unwrap())).unwrap();
    let removed = store.scene(&scope.0).unwrap().items[0].clone(); let document = removed.image_annotations.as_ref().unwrap();
    assert_eq!(database_revision(&file.0), revision + 1); assert_eq!(document.revision, 3); assert!(document.drawings.is_empty());
    assert_eq!(document.history.undo.len(), 3); assert_eq!(removed.asset, source); assert_eq!(store.scene(&scope.0).unwrap().items.len(), 1);
    store.apply(command(3, ImageAnnotationEdit::UndoDrawing)).unwrap(); assert_eq!(store.scene(&scope.0).unwrap().items[0].image_annotations.as_ref().unwrap().drawings, before.image_annotations.as_ref().unwrap().drawings);
    store.apply(command(4, ImageAnnotationEdit::RedoDrawing)).unwrap(); assert!(store.scene(&scope.0).unwrap().items[0].image_annotations.as_ref().unwrap().drawings.is_empty());
    let expected = store.snapshot(); drop(store); assert_eq!(Store::open(&file.0).unwrap().snapshot(), expected);
}

#[test]
fn batch_remove_image_annotation_failure_retains_the_entire_document_and_history() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let source = Asset { id: Uuid::new_v4().to_string(), name: "import.png".into(), kind: AssetKind::Image,
        path: "synthetic-import.png".into(), width: Some(800), height: Some(600), origin_x: None, origin_y: None, scale_factor: None, capture_coordinates: None };
    store.add_asset(&scope.0, source.clone()).unwrap(); let item_id = store.scene(&scope.0).unwrap().items[0].id.clone();
    let command = |revision, action| SceneCommand::EditImageAnnotations { scene_id: scope.0.clone(), item_id: item_id.clone(), source_asset_id: source.id.clone(), expected_revision: revision, action };
    let values = [object(DrawingKind::Pen), object(DrawingKind::Text)];
    for (index, drawing) in values.iter().enumerate() { store.apply(command(index as u64, ImageAnnotationEdit::AddDrawing { drawing: drawing.clone() })).unwrap(); }
    let before = store.snapshot(); let revision = database_revision(&file.0);
    assert!(store.apply(command(2, ImageAnnotationEdit::RemoveDrawings { drawing_ids: vec![values[0].id.clone(), Uuid::new_v4().to_string()] })).is_err());
    assert_eq!(store.snapshot(), before); assert_eq!(database_revision(&file.0), revision);
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("CREATE TRIGGER reject_image_batch BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'image batch rollback'); END;").unwrap();
    assert!(store.apply(command(2, ImageAnnotationEdit::RemoveDrawings { drawing_ids: values.iter().map(|drawing| drawing.id.clone()).collect() })).is_err());
    assert_eq!(store.snapshot(), before); assert_eq!(database_revision(&file.0), revision);
    drop(db); drop(store); assert_eq!(Store::open(&file.0).unwrap().snapshot(), before);
}

#[test]
fn batch_remove_extracted_parent_keeps_root_repair_and_atomically_removes_attached_repairs() {
    let file = TestDb::new(); let mut store = Store::open(&file.0).unwrap(); let scope = prepare(&mut store);
    let background = store.scene(&scope.0).unwrap().background.clone().unwrap();
    let png = vec![137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,4,0,0,0,181,28,12,2,0,0,0,11,73,68,65,84,120,218,99,252,255,31,0,3,3,2,0,239,154,55,219,0,0,0,0,73,69,78,68,174,66,96,130];
    let layout = |role, source: &VisualSourceFence, parent_id| VerifiedRichLayout::new(Uuid::new_v4().to_string(), RichContent::Raster { version: 1, role, source_sha256: source.visual_sha256.clone(), parent_id },
        RichRendererIdentity { id: "mewu.local-raster-edit".into(), version: "1".into(), font_sha256: "a".repeat(64), style_version: 1 }, 1, 1, png.clone()).unwrap();
    let drawing = |layout: &VerifiedRichLayout| Drawing { id: Uuid::new_v4().to_string(), kind: DrawingKind::Rich, color: RICH_DRAWING_COLOR.into(), stroke_width: RICH_DRAWING_STROKE_WIDTH,
        points: vec![DrawingPoint { x: 30., y: 40. }, DrawingPoint { x: 31., y: 41. }], text: None, font_size: None, text_style: None, origin: None, rich: Some(layout.reference().clone()) };
    let fence = visual_source_fence(&background, &region(&store, &scope)).unwrap();
    let root_layout = layout(RasterRole::Repair, &fence, None); let parent_layout = layout(RasterRole::Extracted, &fence, None);
    let root_repair = drawing(&root_layout); let parent = drawing(&parent_layout);
    store.append_raster_edit(&scope.0, &scope.2, &fence, vec![root_repair.clone(), parent.clone()], vec![root_layout, parent_layout]).unwrap();
    let fence = visual_source_fence(&background, &region(&store, &scope)).unwrap();
    let child_layout = layout(RasterRole::Repair, &fence, Some(parent.id.clone())); let child = drawing(&child_layout);
    store.append_raster_edit(&scope.0, &scope.2, &fence, vec![child.clone()], vec![child_layout]).unwrap();
    let ink = object(DrawingKind::Pen); add(&mut store, &scope, ink.clone()).unwrap();
    let before = region(&store, &scope); let revision = database_revision(&file.0);
    assert!(store.apply(remove_batch_command(&scope, before.drawing_revision, vec![ink.id.clone(), child.id.clone()])).is_err());
    assert_eq!(region(&store, &scope), before); assert_eq!(database_revision(&file.0), revision);
    store.apply(remove_batch_command(&scope, before.drawing_revision, vec![parent.id.clone(), ink.id])).unwrap();
    assert_eq!(region(&store, &scope).drawings, vec![root_repair]); assert_eq!(database_revision(&file.0), revision + 1);
    history(&mut store, &scope, false).unwrap(); assert_eq!(region(&store, &scope).drawings, before.drawings);
    history(&mut store, &scope, true).unwrap(); let expected = store.snapshot(); drop(store);
    assert_eq!(Store::open(&file.0).unwrap().snapshot(), expected);
}

#[test]
fn pixel_eraser_is_one_persisted_history_step_and_keeps_style_order_and_unrelated_objects() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scope = prepare(&mut store);
    let pen = object(DrawingKind::Pen);
    let shape = object(DrawingKind::Rect);
    let highlight = object(DrawingKind::Highlighter);
    let line = object(DrawingKind::Line);
    for value in [&pen, &shape, &highlight, &line] { add(&mut store, &scope, value.clone()).unwrap(); }
    let original = region(&store, &scope);
    let pen_cut = split_stroke(&pen);
    let highlight_cut = split_stroke(&highlight);
    store.apply(erase_command(&scope, original.drawing_revision, vec![
        pen_cut.clone(), highlight_cut.clone(), DrawingStrokeErase { drawing_id: line.id.clone(), fragments: vec![] },
    ])).unwrap();
    let erased = region(&store, &scope);
    assert_eq!(erased.drawing_revision, original.drawing_revision + 1);
    assert_eq!(erased.drawing_history.undo.len(), original.drawing_history.undo.len() + 1);
    let expected: Vec<Drawing> = pen_cut.fragments.iter().map(|part| Drawing { id: part.id.clone(), points: part.points.clone(), ..pen.clone() })
        .chain(std::iter::once(shape.clone()))
        .chain(highlight_cut.fragments.iter().map(|part| Drawing { id: part.id.clone(), points: part.points.clone(), ..highlight.clone() })).collect();
    assert_eq!(erased.drawings, expected);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(region(&store, &scope), erased);
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, original.drawings);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings, expected);
    assert!(store.apply(erase_command(&scope, original.drawing_revision, vec![pen_cut])).is_err());
    assert_eq!(region(&store, &scope).drawings, expected);
}

#[test]
fn pixel_eraser_invalid_second_source_fragment_and_stale_revision_leave_database_atomic() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scope = prepare(&mut store);
    let pen = object(DrawingKind::Pen);
    let rect = object(DrawingKind::Rect);
    add(&mut store, &scope, pen.clone()).unwrap(); add(&mut store, &scope, rect.clone()).unwrap();
    let before = store.snapshot();
    let revision = region(&store, &scope).drawing_revision;
    let valid = split_stroke(&pen);
    let mut duplicate = valid.clone(); duplicate.fragments[1].id = rect.id.clone();
    let mut out_of_bounds = valid.clone(); out_of_bounds.fragments[1].points[0].x = 801.;
    let mut wrong_first_id = valid.clone(); wrong_first_id.fragments[0].id = Uuid::new_v4().to_string();
    for erasures in [
        vec![valid.clone(), DrawingStrokeErase { drawing_id: rect.id.clone(), fragments: vec![] }],
        vec![valid.clone(), valid.clone()], vec![duplicate], vec![out_of_bounds], vec![wrong_first_id],
    ] {
        assert!(store.apply(erase_command(&scope, revision, erasures)).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(Store::open(&db.0).unwrap().snapshot(), before);
    }
    assert!(store.apply(erase_command(&scope, revision - 1, vec![valid])).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn pixel_eraser_noop_keeps_revision_and_redo_and_does_not_delete_shapes() {
    let mut store = Store::open_in_memory().unwrap();
    let scope = prepare(&mut store);
    let pen = object(DrawingKind::Pen);
    let rect = object(DrawingKind::Rect);
    add(&mut store, &scope, pen.clone()).unwrap(); add(&mut store, &scope, rect.clone()).unwrap();
    history(&mut store, &scope, false).unwrap();
    let before = store.snapshot();
    let revision = region(&store, &scope).drawing_revision;
    store.apply(erase_command(&scope, revision, vec![DrawingStrokeErase { drawing_id: pen.id.clone(), fragments: vec![
        DrawingStrokeFragment { id: pen.id.clone(), points: pen.points.clone() },
    ] }])).unwrap();
    assert_eq!(store.snapshot(), before);
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings, vec![pen, rect]);
}

#[test]
fn pixel_eraser_budget_is_final_document_size_and_invalid_overflow_is_atomic() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scope = prepare(&mut store);
    for _ in 0..256 { add(&mut store, &scope, object(DrawingKind::Pen)).unwrap(); }
    let original = region(&store, &scope);
    // A high-index split and a low-index deletion must both fit the final
    // document and every persisted intermediate history index.
    store.apply(erase_command(&scope, original.drawing_revision, vec![
        DrawingStrokeErase { drawing_id: original.drawings[0].id.clone(), fragments: vec![] },
        split_stroke(&original.drawings[255]),
    ])).unwrap();
    assert_eq!(region(&store, &scope).drawings.len(), 256);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, original.drawings);
    let before = store.snapshot();
    let current = region(&store, &scope);
    assert!(store.apply(erase_command(&scope, current.drawing_revision, vec![split_stroke(&current.drawings[255])])).is_err());
    assert_eq!(store.snapshot(), before);
    let mut forged = serde_json::to_value(split_stroke(&current.drawings[0])).unwrap();
    forged["fragments"][0]["kind"] = serde_json::json!("rich");
    assert!(serde_json::from_value::<DrawingStrokeErase>(forged).is_err());
    // Every source can be shortened by the same gesture: its remove/insert
    // history needs 512 edits while both document snapshots remain 256 objects.
    let shortened: Vec<_> = current.drawings.iter().map(|drawing| {
        let mut points = drawing.points.clone(); points[1].x -= 5.; points[1].y -= 3.125;
        DrawingStrokeErase { drawing_id: drawing.id.clone(), fragments: vec![DrawingStrokeFragment { id: drawing.id.clone(), points }] }
    }).collect();
    store.apply(erase_command(&scope, current.drawing_revision, shortened)).unwrap();
    let after = region(&store, &scope);
    assert_eq!(after.drawings.len(), 256);
    let edits = after.drawing_history.undo.last().unwrap().edits();
    assert_eq!(edits.len(), 512); assert!(edits.iter().all(|edit| edit.index < 256));
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, current.drawings);
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings, after.drawings);
}

#[test]
fn rectangular_mark_keeps_revision_fences_and_persisted_edit_history() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scope = prepare(&mut store);
    let mark = object(DrawingKind::Mark);
    add(&mut store, &scope, mark.clone()).unwrap();
    let mut moved = mark.clone();
    moved.color = "#00AAFF".into();
    moved.points.iter_mut().for_each(|p| {
        p.x += 15.;
        p.y += 20.;
    });
    let update = SceneCommand::UpdateDrawing {
        scene_id: scope.0.clone(),
        region_id: scope.2.clone(),
        background_id: scope.1.clone(),
        expected_revision: region(&store, &scope).drawing_revision,
        drawing: moved.clone(),
    };
    store.apply(update.clone()).unwrap();
    assert!(store.apply(update).is_err());
    assert_eq!(region(&store, &scope).drawings, vec![moved.clone()]);
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, vec![mark.clone()]);
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    history(&mut store, &scope, true).unwrap();
    assert_eq!(region(&store, &scope).drawings, vec![moved.clone()]);
    let expected_revision = region(&store, &scope).drawing_revision;
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: scope.0.clone(),
            region_id: scope.2.clone(),
            background_id: scope.1.clone(),
            expected_revision,
            drawing_id: mark.id.clone(),
        })
        .unwrap();
    history(&mut store, &scope, false).unwrap();
    assert_eq!(region(&store, &scope).drawings, vec![moved]);
    let before = store.snapshot();
    let mut invalid = object(DrawingKind::Mark);
    invalid.points[1].x = invalid.points[0].x;
    assert!(add(&mut store, &scope, invalid).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        Connection::open(&db.0)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        11
    );
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
