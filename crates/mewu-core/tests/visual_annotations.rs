// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mewu-visual-{}.sqlite", uid())))
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
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn shape() -> Drawing {
    Drawing {
        id: uid(),
        kind: DrawingKind::Text,
        color: "#5B85E8".into(),
        stroke_width: 3.,
        points: vec![DrawingPoint { x: 50., y: 60. }],
        text: Some("答案：42".into()),
        font_size: Some(22.),
        origin: None,
        rich: None,
    }
}
struct Fixture {
    run: RunContext,
    region: String,
    background: String,
    input: VerifiedVisualInput,
    grant: VisualAnnotationGrant,
    authority: VerifiedVisualAuthority,
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
                origin_x: Some(-800),
                origin_y: Some(0),
                scale_factor: Some(1.75),
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
    let manual = Drawing {
        kind: DrawingKind::Line,
        points: vec![
            DrawingPoint { x: 25., y: 35. },
            DrawingPoint { x: 40., y: 50. },
        ],
        text: None,
        font_size: None,
        ..shape()
    };
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: scene.clone(),
            region_id: region.clone(),
            background_id: background.clone(),
            expected_revision: 0,
            drawing: manual,
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "把答案填在图中".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    let asset = Asset {
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
    let target = VisualInputTarget {
        handle: uid(),
        attachment_id: asset.id.clone(),
        attachment_ordinal: 0,
        attachment_sha256: hash("fixture bytes"),
        region_id: region.clone(),
        source: visual_source_fence(run.background.as_ref().unwrap(), &run.regions[0]).unwrap(),
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
        profile_id: "encoded-pixels-best-effort-v1".into(),
    };
    let input = VerifiedVisualInput::new(VisualInputManifest {
        version: 1,
        targets: vec![target],
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
            &run.run_id,
            vec![asset],
            grant.clone(),
            input.clone(),
        )
        .unwrap();
    Fixture {
        run,
        region,
        background,
        input,
        grant,
        authority,
    }
}
fn leases(store: &mut Store, f: &Fixture, count: usize) -> Vec<DispatchLease> {
    let model = store
        .begin_model_request(
            &f.run.scene_id,
            &f.run.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: hash("request"),
            },
        )
        .unwrap();
    store
        .record_model_turn(
            &model,
            CompletePublicTurn {
                text: String::new(),
                calls: (0..count)
                    .map(|_| ProposedTool {
                        call_id: uid(),
                        binding: f.grant.binding(f.input.sha256()),
                        arguments_wire_sha256: hash("arguments"),
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
fn batch(f: &Fixture, count: usize) -> VisualAppendBatch {
    VisualAppendBatch {
        groups: vec![VisualAppendGroup {
            target_handle: f.input.manifest().targets[0].handle.clone(),
            drawings: (0..count).map(|_| shape()).collect(),
        }],
    }
}
fn region(store: &Store, f: &Fixture) -> Region {
    store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == f.run.scene_id)
        .unwrap()
        .regions
        .iter()
        .find(|r| r.id == f.region)
        .unwrap()
        .clone()
}
fn history(store: &mut Store, f: &Fixture, redo: bool) {
    let revision = region(store, f).drawing_revision;
    let command = if redo {
        SceneCommand::RedoDrawing {
            scene_id: f.run.scene_id.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: revision,
        }
    } else {
        SceneCommand::UndoDrawing {
            scene_id: f.run.scene_id.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: revision,
        }
    };
    store.apply(command).unwrap();
}

#[test]
fn batch_has_real_editable_objects_one_undo_and_preserves_hand_drawings_and_origin() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let before = region(&store, &f);
    let lease = leases(&mut store, &f, 1).remove(0);
    let result = store
        .apply_visual_annotations_with_receipt(&lease, &f.authority, batch(&f, 2))
        .unwrap();
    assert_eq!(result.result.response_kind, ToolResponseKind::LocalCommit);
    let after = region(&store, &f);
    assert_eq!(after.drawings.len(), 3);
    assert_eq!(after.drawings[0], before.drawings[0]);
    assert_eq!(
        after.drawing_history.undo.len(),
        before.drawing_history.undo.len() + 1
    );
    assert_eq!(after.drawing_history.undo.last().unwrap().edits().len(), 2);
    assert_eq!(
        after.drawings[1].origin.as_ref().unwrap().group_id,
        after.drawings[2].origin.as_ref().unwrap().group_id
    );
    history(&mut store, &f, false);
    assert_eq!(region(&store, &f).drawings, before.drawings);
    history(&mut store, &f, true);
    assert_eq!(region(&store, &f).drawings, after.drawings);
    let mut edited = after.drawings[1].clone();
    edited.text = Some("用户修正".into());
    store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: f.run.scene_id.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(&store, &f).drawing_revision,
            drawing: edited.clone(),
        })
        .unwrap();
    assert_eq!(region(&store, &f).drawings[1], edited);
    store
        .finish_run(&f.run.scene_id, &f.run.run_id, "已写入")
        .unwrap();
    drop(store);
    let reopened = Store::open(&file.0).unwrap();
    assert_eq!(region(&reopened, &f).drawings[1], edited);
}

#[test]
fn own_effect_advances_cursor_but_manual_aba_revokes_and_duplicate_lease_never_draws_twice() {
    let mut store = Store::open_in_memory().unwrap();
    let f = prepare(&mut store);
    let calls = leases(&mut store, &f, 3);
    store
        .apply_visual_annotations_with_receipt(&calls[0], &f.authority, batch(&f, 1))
        .unwrap();
    let once = store.snapshot();
    assert!(store
        .apply_visual_annotations_with_receipt(&calls[0], &f.authority, batch(&f, 1))
        .is_err());
    assert_eq!(store.snapshot(), once);
    store
        .apply_visual_annotations_with_receipt(&calls[1], &f.authority, batch(&f, 1))
        .unwrap();
    let drawings = region(&store, &f).drawings;
    history(&mut store, &f, false);
    history(&mut store, &f, true);
    assert_eq!(region(&store, &f).drawings, drawings);
    let before = store.snapshot();
    assert!(store
        .apply_visual_annotations_with_receipt(&calls[2], &f.authority, batch(&f, 1))
        .is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn frozen_origin_scene_receives_batch_without_selecting_or_changing_other_scene() {
    let mut store = Store::open_in_memory().unwrap();
    let f = prepare(&mut store);
    let call = leases(&mut store, &f, 1).remove(0);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: f.run.scene_id.clone(),
        })
        .unwrap();
    let other = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: other.clone(),
            draft: "另一会话草稿".into(),
        })
        .unwrap();
    let before = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == other)
        .unwrap();
    store
        .apply_visual_annotations_with_receipt(&call, &f.authority, batch(&f, 1))
        .unwrap();
    assert_eq!(store.snapshot().active_scene_id, other);
    assert_eq!(
        store
            .snapshot()
            .scenes
            .into_iter()
            .find(|s| s.id == other)
            .unwrap(),
        before
    );
    assert_eq!(region(&store, &f).drawings.len(), 2);
}

#[test]
fn sqlite_receipt_abort_rolls_back_effect_history_cursor_and_receipt_then_retry_succeeds() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let call = leases(&mut store, &f, 1).remove(0);
    let db = Connection::open(&file.0).unwrap();
    let before = store.snapshot();
    let input: String = db
        .query_row("SELECT data FROM visual_run_inputs", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER reject_visual_receipt BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store
        .apply_visual_annotations_with_receipt(&call, &f.authority, batch(&f, 2))
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        db.query_row("SELECT data FROM visual_run_inputs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        input
    );
    let payload: String = db
        .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(serde_json::from_str::<Snapshot>(&payload).unwrap(), before);
    db.execute_batch("DROP TRIGGER reject_visual_receipt")
        .unwrap();
    store
        .apply_visual_annotations_with_receipt(&call, &f.authority, batch(&f, 2))
        .unwrap();
    assert_eq!(region(&store, &f).drawings.len(), 3);
}

#[test]
fn authority_revision_revoke_and_cancel_reject_effects_without_canvas_change() {
    for mode in 0..3 {
        let mut store = Store::open_in_memory().unwrap();
        let f = prepare(&mut store);
        let call = leases(&mut store, &f, 1).remove(0);
        let mut grant = f.grant.clone();
        if mode == 0 {
            grant.plugin_revision += 1;
        }
        if mode == 1 {
            store
                .revoke_visual_authority(&f.run.scene_id, &f.run.run_id)
                .unwrap();
        }
        if mode == 2 {
            store.cancel_run(&f.run.scene_id).unwrap();
        }
        let authority = VerifiedVisualAuthority::new(grant, f.input.sha256().into()).unwrap();
        let before = store.snapshot();
        assert!(store
            .apply_visual_annotations_with_receipt(&call, &authority, batch(&f, 1))
            .is_err());
        assert_eq!(store.snapshot(), before);
        if mode != 0 {
            assert!(store
                .visual_input_for_run(&f.run.scene_id, &f.run.run_id, &f.authority)
                .is_err());
        }
    }
}

#[test]
fn renderer_cannot_inject_or_remove_origin_and_invalid_batch_is_all_or_none() {
    let mut store = Store::open_in_memory().unwrap();
    let f = prepare(&mut store);
    let calls = leases(&mut store, &f, 2);
    store
        .apply_visual_annotations_with_receipt(&calls[0], &f.authority, batch(&f, 1))
        .unwrap();
    let mut ai = region(&store, &f).drawings[1].clone();
    let before = store.snapshot();
    ai.id = uid();
    assert!(store
        .apply(SceneCommand::AddDrawing {
            scene_id: f.run.scene_id.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(&store, &f).drawing_revision,
            drawing: ai
        })
        .is_err());
    let mut ai = region(&store, &f).drawings[1].clone();
    ai.origin = None;
    assert!(store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: f.run.scene_id.clone(),
            region_id: f.region.clone(),
            background_id: f.background.clone(),
            expected_revision: region(&store, &f).drawing_revision,
            drawing: ai
        })
        .is_err());
    let mut invalid = batch(&f, 2);
    invalid.groups[0].drawings[1].points[0].x = 0.;
    assert!(store
        .apply_visual_annotations_with_receipt(&calls[1], &f.authority, invalid)
        .is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn db5_migration_preserves_legacy_single_step_and_rejects_ambiguous_or_empty_batch() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    store.cancel_run(&f.run.scene_id).unwrap();
    let before = store.snapshot();
    drop(store);
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch(
        "DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;DROP TABLE visual_run_inputs;PRAGMA user_version=5;",
    )
    .unwrap();
    let payload: String = db
        .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap();
    assert!(!payload.contains("\"origin\""));
    assert!(!payload.contains("\"batch\""));
    let migrated = Store::open(&file.0).unwrap();
    assert_eq!(migrated.snapshot(), before);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
    let edit = region(&migrated, &f).drawing_history.undo[0].edits()[0].clone();
    let mut ambiguous = serde_json::to_value(&edit).unwrap();
    ambiguous["batch"] = json!([edit]);
    assert!(serde_json::from_value::<DrawingStep>(ambiguous).is_err());
    drop(migrated);
    let mut corrupt = before;
    corrupt.scenes[0].regions[0]
        .drawing_history
        .undo
        .push(DrawingStep::Batch(DrawingBatchEdit { batch: vec![] }));
    db.execute(
        "UPDATE app_state SET payload=?1",
        [serde_json::to_string(&corrupt).unwrap()],
    )
    .unwrap();
    assert!(Store::open(&file.0).is_err());
}

#[test]
fn migration_abort_keeps_db5_and_second_writer_conflict_does_not_publish_batch() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = prepare(&mut store);
    let call = leases(&mut store, &f, 1).remove(0);
    let db = Connection::open(&file.0).unwrap();
    let before = store.snapshot();
    db.execute("UPDATE app_state SET revision=revision+1", [])
        .unwrap();
    assert!(matches!(
        store.apply_visual_annotations_with_receipt(&call, &f.authority, batch(&f, 1)),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(store);
    let mut recovered = Store::open(&file.0).unwrap();
    let id = recovered.snapshot().active_scene_id;
    recovered
        .apply(SceneCommand::SetDraft {
            scene_id: id,
            draft: "保留".into(),
        })
        .unwrap();
    drop(recovered);
    db.execute_batch("DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;DROP TABLE visual_run_inputs;PRAGMA user_version=5;CREATE TRIGGER fail_visual_upgrade BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    let before: String = db
        .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
        .unwrap();
    assert!(Store::open(&file.0).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='visual_run_inputs'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT payload FROM app_state", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        before
    );
}

#[test]
fn pixel_mapping_is_exact_for_rounding_long_tiles_and_override_without_display_origin() {
    let mut store = Store::open_in_memory().unwrap();
    let f = prepare(&mut store);
    let mut target = f.input.manifest().targets[0].clone();
    target.crop = VisualPixelRect {
        x: 100,
        y: 200,
        width: 3000,
        height: 6000,
    };
    target.resized = VisualPixelSize {
        width: 2048,
        height: 4096,
    };
    target.tile = VisualPixelRect {
        x: 0,
        y: 1920,
        width: 2048,
        height: 2048,
    };
    target.encoded = VisualPixelSize {
        width: 1024,
        height: 1024,
    };
    assert_eq!(
        target
            .source_point(DrawingPoint { x: 256., y: 512. })
            .unwrap(),
        DrawingPoint { x: 850., y: 4512.5 }
    );
    target.crop.x = 0;
    target.crop.y = 0;
    assert_eq!(
        target
            .source_point(DrawingPoint { x: 256., y: 512. })
            .unwrap(),
        DrawingPoint { x: 750., y: 4312.5 }
    );
    target.crop = VisualPixelRect {
        x: 7,
        y: 11,
        width: 3001,
        height: 1999,
    };
    target.resized = VisualPixelSize {
        width: 2048,
        height: 1364,
    };
    target.tile = VisualPixelRect {
        x: 0,
        y: 0,
        width: 2048,
        height: 1364,
    };
    target.encoded = VisualPixelSize {
        width: 1024,
        height: 682,
    };
    assert_eq!(
        target
            .source_point(DrawingPoint { x: 1024., y: 682. })
            .unwrap(),
        DrawingPoint { x: 3008., y: 2010. }
    );
    assert!(target
        .source_point(DrawingPoint { x: 1024.1, y: 20. })
        .is_err());
    assert!(target
        .source_point(DrawingPoint {
            x: f64::NAN,
            y: 20.
        })
        .is_err());
}
