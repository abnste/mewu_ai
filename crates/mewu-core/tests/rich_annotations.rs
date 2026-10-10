// SPDX-License-Identifier: MPL-2.0
//! These are domain/real SQLite tests. Tiny fixed PNGs are byte identity
//! fixtures, not evidence that a native table/formula renderer has run.
use mewu_core::*;
use rusqlite::{params, types::Value, Connection};
use serde_json::json;
use std::path::PathBuf;

struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mewu-rich-{}.sqlite", uid())))
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn png() -> Vec<u8> {
    // A complete 1x1 PNG, fixed bytes independent of the production renderer.
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 154, 55, 219, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}
fn content() -> RichContent {
    RichContent::Table {
        version: 1,
        header: vec!["中文".into(), "值".into(), "空".into()],
        rows: vec![vec!["=1+2\n第二行".into(), "0001".into(), String::new()]],
        align: vec![
            None,
            Some(RichAlignment::Center),
            Some(RichAlignment::Right),
        ],
    }
}
fn renderer() -> RichRendererIdentity {
    RichRendererIdentity {
        id: "native-table".into(),
        version: "1".into(),
        font_sha256: "a".repeat(64),
        style_version: 1,
    }
}
fn layout() -> VerifiedRichLayout {
    VerifiedRichLayout::new(uid(), content(), renderer(), 1, 1, png()).unwrap()
}
fn drawing(layout: &VerifiedRichLayout) -> Drawing {
    Drawing {
        id: uid(),
        kind: DrawingKind::Rich,
        color: RICH_DRAWING_COLOR.into(),
        stroke_width: RICH_DRAWING_STROKE_WIDTH,
        points: vec![
            DrawingPoint { x: 100., y: 100. },
            DrawingPoint { x: 140., y: 140. },
        ],
        text: None,
        font_size: None,
        origin: None,
        rich: Some(layout.reference().clone()),
    }
}
fn vector() -> Drawing {
    Drawing {
        id: uid(),
        kind: DrawingKind::Line,
        color: "#AABBCC".into(),
        stroke_width: 2.,
        points: vec![
            DrawingPoint { x: 50., y: 50. },
            DrawingPoint { x: 60., y: 60. },
        ],
        text: None,
        font_size: None,
        origin: None,
        rich: None,
    }
}
struct Fixture {
    scene: String,
    region: String,
    background: String,
    run: String,
    input: VerifiedVisualInput,
    authority: VerifiedVisualAuthority,
    grant: VisualAnnotationGrant,
}
fn prepare(store: &mut Store) -> Fixture {
    let scene = store.snapshot().active_scene_id;
    let background = uid();
    store
        .set_background(
            &scene,
            Asset {
                id: background.clone(),
                kind: AssetKind::Image,
                name: "source.png".into(),
                path: "source.png".into(),
                width: Some(800),
                height: Some(600),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            },
        )
        .unwrap();
    let region = uid();
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
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: scene.clone(),
            region_id: region.clone(),
            background_id: background.clone(),
            expected_revision: 0,
            drawing: vector(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "原位表格".into(),
        })
        .unwrap();
    let context = store.begin_run(&scene).unwrap();
    let attachment = Asset {
        id: uid(),
        kind: AssetKind::Image,
        name: "input.jpg".into(),
        path: "input.jpg".into(),
        width: Some(600),
        height: Some(400),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    let input = VerifiedVisualInput::new(VisualInputManifest {
        version: 1,
        targets: vec![VisualInputTarget {
            handle: uid(),
            attachment_id: attachment.id.clone(),
            attachment_ordinal: 0,
            attachment_sha256: "b".repeat(64),
            region_id: region.clone(),
            source: visual_source_fence(context.background.as_ref().unwrap(), &context.regions[0])
                .unwrap(),
            crop: VisualPixelRect {
                x: 20,
                y: 30,
                width: 600,
                height: 400,
            },
            resized: VisualPixelSize {
                width: 600,
                height: 400,
            },
            tile: VisualPixelRect {
                x: 0,
                y: 0,
                width: 600,
                height: 400,
            },
            encoded: VisualPixelSize {
                width: 600,
                height: 400,
            },
            profile_id: "explicit-normalized1000-v2".into(),
        }],
    })
    .unwrap();
    let grant = VisualAnnotationGrant {
        plugin_id: "mewu.annotations".into(),
        plugin_revision: 1,
        contribution_id: "answer".into(),
    };
    let authority = VerifiedVisualAuthority::new(grant.clone(), input.sha256().into()).unwrap();
    store
        .attach_run_visual_assets(
            &scene,
            &context.run_id,
            vec![attachment],
            grant.clone(),
            input.clone(),
        )
        .unwrap();
    Fixture {
        scene,
        region,
        background,
        run: context.run_id,
        input,
        authority,
        grant,
    }
}
fn leases(store: &mut Store, f: &Fixture, count: usize) -> Vec<DispatchLease> {
    let lease = store
        .begin_model_request(
            &f.scene,
            &f.run,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: "c".repeat(64),
            },
        )
        .unwrap();
    store
        .record_model_turn(
            &lease,
            CompletePublicTurn {
                text: String::new(),
                calls: (0..count)
                    .map(|_| ProposedTool {
                        call_id: uid(),
                        binding: f.grant.binding(f.input.sha256()),
                        arguments_wire_sha256: "d".repeat(64),
                    })
                    .collect(),
            },
        )
        .unwrap()
        .tools
        .into_iter()
        .map(|t| t.lease)
        .collect()
}
fn batch(f: &Fixture, drawings: Vec<Drawing>) -> VisualAppendBatch {
    VisualAppendBatch {
        groups: vec![VisualAppendGroup {
            target_handle: f.input.manifest().targets[0].handle.clone(),
            drawings,
        }],
    }
}
fn region(store: &Store, f: &Fixture) -> Region {
    store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == f.scene)
        .unwrap()
        .regions
        .iter()
        .find(|r| r.id == f.region)
        .unwrap()
        .clone()
}
fn edit(store: &mut Store, f: &Fixture, drawing: Drawing) -> Result<Snapshot, CoreError> {
    store.apply(SceneCommand::UpdateDrawing {
        scene_id: f.scene.clone(),
        region_id: f.region.clone(),
        background_id: f.background.clone(),
        expected_revision: region(store, f).drawing_revision,
        drawing,
    })
}
fn undo(store: &mut Store, f: &Fixture, redo: bool) {
    let command = if redo {
        SceneCommand::RedoDrawing {
            scene_id: f.scene.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(store, f).drawing_revision,
        }
    } else {
        SceneCommand::UndoDrawing {
            scene_id: f.scene.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(store, f).drawing_revision,
        }
    };
    store.apply(command).unwrap();
}
fn rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let sql = format!(
        "SELECT * FROM \"{}\" ORDER BY rowid",
        table.replace('"', "\"\"")
    );
    let mut statement = db.prepare(&sql).unwrap();
    let columns = statement.column_count();
    let result = statement
        .query_map([], |row| {
            (0..columns).map(|i| row.get::<_, Value>(i)).collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    result
}
fn proof(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "app_state",
        "visual_run_inputs",
        "drawing_layouts",
        "agent_run_records",
        "agent_run_events",
    ]
    .into_iter()
    .map(|table| rows(db, table))
    .collect()
}
fn commit(
    store: &mut Store,
    f: &Fixture,
    lease: &DispatchLease,
    layout: VerifiedRichLayout,
) -> Drawing {
    let object = drawing(&layout);
    store
        .apply_visual_annotations_with_rich_receipt(
            lease,
            &f.authority,
            batch(f, vec![object.clone()]),
            vec![layout],
        )
        .unwrap();
    assert_eq!(
        store
            .visual_input_for_run(&f.scene, &f.run, &f.authority)
            .unwrap(),
        f.input
    );
    region(store, f)
        .drawings
        .iter()
        .find(|d| d.id == object.id)
        .unwrap()
        .clone()
}

#[test]
fn typed_content_and_binary_identity_are_validated_without_guessing_markup() {
    let object = layout();
    assert_eq!(object.content(), &content());
    assert_eq!(object.png(), png());
    assert_eq!(object.reference().kind, RichKind::Table);
    let mut changed = renderer();
    changed.style_version = 2;
    let changed = VerifiedRichLayout::new(
        object.reference().layout_id.clone(),
        content(),
        changed,
        1,
        1,
        png(),
    )
    .unwrap();
    assert_ne!(
        object.reference().layout_sha256,
        changed.reference().layout_sha256
    );
    let formula = RichContent::Formula {
        version: 1,
        tex: "\\frac{x}{2} + 中文".into(),
        color: "#223344".into(),
        font_size: 20.,
    };
    let formula = VerifiedRichLayout::new(uid(), formula, renderer(), 1, 1, png()).unwrap();
    assert_eq!(formula.reference().kind, RichKind::Formula);
    for bad in [
        json!({"kind":"html","version":1}),
        json!({"kind":"table","version":1,"header":["x"],"rows":[],"align":["justify"]}),
        json!({"kind":"formula","version":1,"tex":"x","color":"#000000","fontSize":20,"path":"bad"}),
    ] {
        assert!(serde_json::from_value::<RichContent>(bad).is_err());
    }
    let bad = RichContent::Table {
        version: 1,
        header: vec!["x".into()],
        rows: vec![vec![]],
        align: vec![None],
    };
    assert!(VerifiedRichLayout::new(uid(), bad, renderer(), 1, 1, png()).is_err());
    let bad = RichContent::Table {
        version: 1,
        header: vec!["\0".into()],
        rows: vec![],
        align: vec![None],
    };
    assert!(VerifiedRichLayout::new(uid(), bad, renderer(), 1, 1, png()).is_err());
    assert!(VerifiedRichLayout::new(uid(), content(), renderer(), 2, 1, png()).is_err());
    let mut bad = png();
    bad[12] = b'X';
    assert!(VerifiedRichLayout::new(uid(), content(), renderer(), 1, 1, bad).is_err());
}

#[test]
fn old_vector_wire_stays_identical_and_rich_fields_are_strict() {
    let object = vector();
    let value = serde_json::to_value(&object).unwrap();
    assert!(value.get("rich").is_none());
    assert_eq!(
        serde_json::from_value::<Drawing>(value.clone()).unwrap(),
        object
    );
    let mut bad = value;
    bad["richTypo"] = json!({});
    assert!(serde_json::from_value::<Drawing>(bad).is_err());
    let mut reference = serde_json::to_value(layout().reference()).unwrap();
    reference["path"] = json!("source.png");
    assert!(serde_json::from_value::<RichDrawingRef>(reference).is_err());
}

#[test]
fn mixed_batch_history_geometry_and_closed_scene_keep_immutable_pixels() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let lease = leases(&mut store, &f, 1).remove(0);
    let layout = layout();
    let reference = layout.reference().clone();
    let table = drawing(&layout);
    let line = vector();
    let original = region(&store, &f);
    store
        .apply_visual_annotations_with_rich_receipt(
            &lease,
            &f.authority,
            batch(&f, vec![table, line]),
            vec![layout],
        )
        .unwrap();
    assert_eq!(region(&store, &f).drawings.len(), 3);
    assert!(matches!(
        region(&store, &f).drawing_history.undo.last(),
        Some(DrawingStep::Batch(_))
    ));
    assert_ne!(
        visual_source_fence(
            store.snapshot().scenes[0].background.as_ref().unwrap(),
            &original
        )
        .unwrap(),
        visual_source_fence(
            store.snapshot().scenes[0].background.as_ref().unwrap(),
            &region(&store, &f)
        )
        .unwrap()
    );
    undo(&mut store, &f, false);
    assert_eq!(region(&store, &f).drawings, original.drawings);
    assert_eq!(
        Store::read_rich_layout(&file.0, &reference).unwrap().png(),
        png()
    );
    undo(&mut store, &f, true);
    let before = region(&store, &f).drawings;
    let mut moved = before[1].clone();
    moved.points.iter_mut().for_each(|p| {
        p.x += 10.;
        p.y += 20.;
    });
    edit(&mut store, &f, moved.clone()).unwrap();
    undo(&mut store, &f, false);
    assert_eq!(region(&store, &f).drawings, before);
    undo(&mut store, &f, true);
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: f.scene.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(&store, &f).drawing_revision,
            drawing_id: moved.id.clone(),
        })
        .unwrap();
    assert_eq!(store.rich_layout(&reference).unwrap().content(), &content());
    undo(&mut store, &f, false);
    assert_eq!(region(&store, &f).drawings[1], moved);
    store.finish_run(&f.scene, &f.run, "完整表格").unwrap();
    store
        .apply(SceneCommand::CloseScene {
            scene_id: f.scene.clone(),
        })
        .unwrap();
    let saved = store.snapshot();
    drop(store);
    let reopened = Store::open(&file.0).unwrap();
    assert_eq!(reopened.snapshot(), saved);
    assert_eq!(reopened.rich_layout(&reference).unwrap().png(), png());
}

#[test]
fn normal_add_style_retarget_and_distorted_rectangles_cannot_bypass_rich_authority() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let lease = leases(&mut store, &f, 1).remove(0);
    let first_layout = layout();
    let raw = drawing(&first_layout);
    let before = store.snapshot();
    assert!(store
        .apply(SceneCommand::AddDrawing {
            scene_id: f.scene.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(&store, &f).drawing_revision,
            drawing: raw
        })
        .is_err());
    assert_eq!(store.snapshot(), before);
    let object = commit(&mut store, &f, &lease, first_layout);
    let before = store.snapshot();
    for changed in 0..6 {
        let mut bad = object.clone();
        match changed {
            0 => bad.rich = Some(layout().reference().clone()),
            1 => bad.color = "#AABBCC".into(),
            2 => bad.kind = DrawingKind::Rect,
            3 => bad.points[1].x += 10.,
            4 => bad.points.reverse(),
            _ => bad.origin = None,
        }
        assert!(edit(&mut store, &f, bad).is_err());
        assert_eq!(store.snapshot(), before);
    }
}

#[test]
fn late_invalid_object_rolls_back_blob_history_cursors_receipt_and_app_revision() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let lease = leases(&mut store, &f, 1).remove(0);
    let layout = layout();
    let db = Connection::open(&file.0).unwrap();
    let before = proof(&db);
    let state = store.snapshot();
    let mut invalid = vector();
    invalid.stroke_width = 1000.;
    assert!(store
        .apply_visual_annotations_with_rich_receipt(
            &lease,
            &f.authority,
            batch(&f, vec![drawing(&layout), invalid]),
            vec![layout]
        )
        .is_err());
    assert_eq!(proof(&db), before);
    assert_eq!(store.snapshot(), state);
}

#[test]
fn duplicate_id_and_forged_layout_reference_are_rejected_atomically() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let calls = leases(&mut store, &f, 3);
    let first = layout();
    let id = first.reference().layout_id.clone();
    commit(&mut store, &f, &calls[0], first);
    let db = Connection::open(&file.0).unwrap();
    let before = proof(&db);
    let mut changed = renderer();
    changed.version = "different".into();
    let reused = VerifiedRichLayout::new(id, content(), changed, 1, 1, png()).unwrap();
    assert!(store
        .apply_visual_annotations_with_rich_receipt(
            &calls[1],
            &f.authority,
            batch(&f, vec![drawing(&reused)]),
            vec![reused]
        )
        .is_err());
    assert_eq!(proof(&db), before);
    let next = layout();
    let mut forged = drawing(&next);
    forged.rich.as_mut().unwrap().layout_sha256 = "f".repeat(64);
    assert!(store
        .apply_visual_annotations_with_rich_receipt(
            &calls[2],
            &f.authority,
            batch(&f, vec![forged]),
            vec![next]
        )
        .is_err());
    assert_eq!(proof(&db), before);
    assert!(store
        .apply_visual_annotations_with_rich_receipt(
            &calls[2],
            &f.authority,
            batch(&f, vec![vector()]),
            vec![layout()]
        )
        .is_err());
    assert_eq!(proof(&db), before);
}

#[test]
fn immutable_sql_and_readonly_hash_check_refuse_binary_or_descriptor_substitution() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let lease = leases(&mut store, &f, 1).remove(0);
    let layout = layout();
    let reference = layout.reference().clone();
    commit(&mut store, &f, &lease, layout);
    store.finish_run(&f.scene, &f.run, "已完成").unwrap();
    let db = Connection::open(&file.0).unwrap();
    let before = proof(&db);
    assert!(db
        .execute("UPDATE drawing_layouts SET descriptor='{}'", [])
        .is_err());
    assert!(db.execute("DELETE FROM drawing_layouts", []).is_err());
    assert_eq!(proof(&db), before);
    let mut forged = reference.clone();
    forged.kind = RichKind::Formula;
    assert!(Store::read_rich_layout(&file.0, &forged).is_err());
    assert_eq!(proof(&db), before);
    // A malicious external editor can drop SQL protection; byte identity is
    // still independently checked by open, every mutation and the read path.
    db.execute_batch("DROP TRIGGER drawing_layouts_no_update")
        .unwrap();
    let mut swapped = png();
    swapped[45] ^= 1;
    db.execute(
        "UPDATE drawing_layouts SET raster_png=?1 WHERE layout_id=?2",
        params![swapped, reference.layout_id],
    )
    .unwrap();
    assert!(Store::read_rich_layout(&file.0, &reference).is_err());
    let state = store.snapshot();
    assert!(store
        .apply(SceneCommand::SetDraft {
            scene_id: f.scene.clone(),
            draft: "不能默默忽略坏PNG".into()
        })
        .is_err());
    assert_eq!(store.snapshot(), state);
    drop(store);
    assert!(Store::open(&file.0).is_err());
}

#[test]
fn history_only_layout_is_checked_and_cannot_disappear_on_reopen() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let lease = leases(&mut store, &f, 1).remove(0);
    let layout = layout();
    commit(&mut store, &f, &lease, layout);
    undo(&mut store, &f, false);
    store.finish_run(&f.scene, &f.run, "保留重做").unwrap();
    drop(store);
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("DROP TRIGGER drawing_layouts_no_delete;DELETE FROM drawing_layouts;")
        .unwrap();
    let before = proof(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(proof(&db), before);
}

#[test]
fn sqlite_failure_and_second_writer_never_register_half_a_layout() {
    for stale_writer in [false, true] {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = prepare(&mut store);
        let lease = leases(&mut store, &f, 1).remove(0);
        let layout = layout();
        let db = Connection::open(&file.0).unwrap();
        if stale_writer {
            db.execute("UPDATE app_state SET revision=revision+1", [])
                .unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER fail_rich_commit BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        }
        let before = proof(&db);
        let state = store.snapshot();
        assert!(store
            .apply_visual_annotations_with_rich_receipt(
                &lease,
                &f.authority,
                batch(&f, vec![drawing(&layout)]),
                vec![layout]
            )
            .is_err());
        assert_eq!(proof(&db), before);
        assert_eq!(store.snapshot(), state);
    }
}

#[test]
fn cancel_or_manual_source_edit_rejects_late_rendered_layout() {
    for canceled in [false, true] {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = prepare(&mut store);
        let lease = leases(&mut store, &f, 1).remove(0);
        let layout = layout();
        if canceled {
            store.cancel_run(&f.scene).unwrap();
        } else {
            store
                .apply(SceneCommand::AddDrawing {
                    scene_id: f.scene.clone(),
                    region_id: f.region.clone(),
                    background_id: f.background.clone(),
                    expected_revision: region(&store, &f).drawing_revision,
                    drawing: vector(),
                })
                .unwrap();
        }
        let db = Connection::open(&file.0).unwrap();
        let before = proof(&db);
        assert!(store
            .visual_input_for_run(&f.scene, &f.run, &f.authority)
            .is_err());
        assert!(store
            .apply_visual_annotations_with_rich_receipt(
                &lease,
                &f.authority,
                batch(&f, vec![drawing(&layout)]),
                vec![layout]
            )
            .is_err());
        assert_eq!(proof(&db), before);
    }
}

#[test]
fn db6_upgrade_and_failed_upgrade_preserve_the_old_document_and_version() {
    for fail in [false, true] {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = prepare(&mut store);
        store.finish_run(&f.scene, &f.run, "旧矢量").unwrap();
        let snapshot = store.snapshot();
        drop(store);
        let db = Connection::open(&file.0).unwrap();
        db.execute_batch("DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;PRAGMA user_version=6;")
            .unwrap();
        // A valid DB6 payload need not be the current serializer's canonical
        // view. Derived statistics and legacy inline memories may still be
        // present; adding an empty layout table must preserve their raw bytes.
        let raw: String = db
            .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
            .unwrap();
        let mut document: serde_json::Value = serde_json::from_str(&raw).unwrap();
        document["memoryStats"] = json!([{
            "agentId": snapshot.agents[0].id, "count": 42, "revision": 97
        }]);
        document["memories"] = json!([{
            "id": uid(), "agentId": snapshot.agents[0].id,
            "text": "DB6 合法旧字段，仅保留原始载荷", "revision": 1,
            "createdAt": 1, "updatedAt": 1, "origin": "legacy_reference"
        }]);
        let noncanonical = format!("\n{}\n", serde_json::to_string_pretty(&document).unwrap());
        let decoded: Snapshot = serde_json::from_str(&noncanonical).unwrap();
        assert_eq!(decoded.memories.len(), 1);
        assert_eq!(decoded.memory_stats[0].count, 42);
        db.execute(
            "UPDATE app_state SET payload=?1 WHERE id=1",
            [&noncanonical],
        )
        .unwrap();
        let memory_rows = rows(&db, "memory_records");
        if fail {
            db.execute_batch("CREATE TRIGGER fail_rich_upgrade BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        }
        let before = rows(&db, "app_state");
        let migrated = Store::open(&file.0);
        if fail {
            assert!(migrated.is_err());
            assert_eq!(rows(&db, "app_state"), before);
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                6
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='drawing_layouts'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        } else {
            assert_eq!(migrated.unwrap().snapshot(), snapshot);
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                9
            );
            let after = rows(&db, "app_state");
            assert_eq!(after[0][3], before[0][3]);
            assert_eq!(after[0][3], Value::Text(noncanonical));
            assert_eq!(rows(&db, "memory_records"), memory_rows);
            assert_eq!(
                after[0][2],
                match before[0][2] {
                    Value::Integer(v) => Value::Integer(v + 1),
                    _ => panic!("revision type"),
                }
            );
        }
    }
}

#[test]
fn cumulative_raster_budget_is_transactional_and_readonly_does_not_recover_runs() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let calls = leases(&mut store, &f, 2);
    let first = layout();
    let reference = first.reference().clone();
    commit(&mut store, &f, &calls[0], first);
    let db = Connection::open(&file.0).unwrap();
    let before = proof(&db);
    assert_eq!(
        Store::read_rich_layout(&file.0, &reference).unwrap().png(),
        png()
    );
    assert_eq!(proof(&db), before);
    assert!(store.run_is_active(&f.scene, &f.run));
    let raw: String = db
        .query_row(
            "SELECT data FROM visual_run_inputs WHERE run_id=?1",
            [&f.run],
            |r| r.get(0),
        )
        .unwrap();
    let mut input: serde_json::Value = serde_json::from_str(&raw).unwrap();
    input["richPngBytes"] = json!(MAX_RICH_RUN_PNG_BYTES);
    db.execute(
        "UPDATE visual_run_inputs SET data=?1 WHERE run_id=?2",
        params![serde_json::to_string(&input).unwrap(), f.run],
    )
    .unwrap();
    let before = proof(&db);
    let second = layout();
    assert!(store
        .apply_visual_annotations_with_rich_receipt(
            &calls[1],
            &f.authority,
            batch(&f, vec![drawing(&second)]),
            vec![second]
        )
        .is_err());
    assert_eq!(proof(&db), before);
    db.pragma_update(None, "user_version", 6).unwrap();
    assert!(matches!(
        Store::read_rich_layout(&file.0, &reference),
        Err(CoreError::UnsupportedSchema(6))
    ));
}

#[test]
fn individually_valid_layouts_cannot_exceed_the_run_content_budget() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let calls = leases(&mut store, &f, 5);
    let db = Connection::open(&file.0).unwrap();
    for (index, call) in calls.iter().enumerate() {
        let source = RichContent::Table {
            version: 1,
            header: vec!["x".repeat(32000)],
            rows: vec![],
            align: vec![None],
        };
        let layout = VerifiedRichLayout::new(uid(), source, renderer(), 1, 1, png()).unwrap();
        let before = proof(&db);
        let snapshot = store.snapshot();
        let result = store.apply_visual_annotations_with_rich_receipt(
            call,
            &f.authority,
            batch(&f, vec![drawing(&layout)]),
            vec![layout],
        );
        if index < 4 {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
            assert_eq!(proof(&db), before);
            assert_eq!(store.snapshot(), snapshot);
        }
    }
    store
        .finish_run(&f.scene, &f.run, "保留前四次已提交对象")
        .unwrap();
    drop(store);
    let reopened = Store::open(&file.0).unwrap();
    assert_eq!(region(&reopened, &f).drawings.len(), 5);
}
