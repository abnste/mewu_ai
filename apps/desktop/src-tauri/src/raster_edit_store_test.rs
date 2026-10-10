// SPDX-License-Identifier: MPL-2.0
//! Real local algorithm → PNG → SQLite → immutable compositor. Synthetic image only.
use super::*;
use image::{DynamicImage, Rgba};
#[tokio::test]
async fn raster_cancel_waits_for_real_worker_and_never_revives_after_exit_abort() {
    let jobs = Arc::new(crate::capture_delay_work::CaptureJobs::default());
    let work = jobs.begin().unwrap().unwrap();
    let invoke = work.cancel_on_drop();
    let (ready_send, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        ready_send.send(()).unwrap();
        hold.recv().unwrap();
        work.check()
    });
    ready.await.unwrap();
    jobs.cancel();
    assert!(jobs.begin().unwrap().is_none());
    assert!(jobs.drain(Duration::from_millis(5)).await.is_err());
    // Restoring the app does not replace the worker's cancel flag.
    release.send(()).unwrap();
    assert!(worker.await.unwrap().is_err());
    jobs.drain(Duration::from_secs(1)).await.unwrap();
    drop(invoke);
    jobs.begin().unwrap().unwrap().check().unwrap();
}
#[tokio::test]
async fn raster_abandoned_invoke_cancels_but_keeps_worker_occupancy() {
    let jobs = Arc::new(crate::capture_delay_work::CaptureJobs::default());
    let work = jobs.begin().unwrap().unwrap();
    let invoke = work.cancel_on_drop();
    drop(invoke);
    assert!(work.check().is_err());
    assert!(jobs.begin().unwrap().is_none());
    assert!(jobs.drain(Duration::from_millis(5)).await.is_err());
    drop(work);
    jobs.drain(Duration::from_secs(1)).await.unwrap();
    jobs.begin().unwrap().unwrap().check().unwrap();
}
use mewu_core::{SceneCommand, Store};
struct Db(std::path::PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mewu-raster-edit-{}.db", uuid::Uuid::new_v4())))
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn setup(db: &Db) -> (Store, Target, VisualSourceFence) {
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let background = Asset {
        id: uuid::Uuid::new_v4().to_string(),
        name: "synthetic source.png".into(),
        kind: mewu_core::AssetKind::Image,
        path: "synthetic source.png".into(),
        width: Some(96),
        height: Some(80),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    store.set_background(&scene_id, background.clone()).unwrap();
    let region = Region {
        id: uuid::Uuid::new_v4().to_string(),
        x: 0.,
        y: 0.,
        width: 96.,
        height: 80.,
        ..Default::default()
    };
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.clone(),
            region: region.clone(),
        })
        .unwrap();
    let expected = mewu_core::visual_source_fence(&background, &region).unwrap();
    (
        store,
        Target {
            scene_id,
            background,
            region,
        },
        expected,
    )
}
fn synthetic() -> RgbaImage {
    let mut image = RgbaImage::from_pixel(96, 80, Rgba([240, 240, 240, 255]));
    for y in 24..34 {
        for x in 24..34 {
            image.put_pixel(x, y, Rgba([20, 40, 60, 255]));
        }
    }
    image
}
fn prepare(expected: &VisualSourceFence) -> (Vec<Drawing>, Vec<VerifiedRichLayout>) {
    let source = synthetic();
    let area = crate::raster_edit_algorithm::Area {
        x: 20,
        y: 20,
        width: 20,
        height: 20,
    };
    let patch = crate::raster_edit_algorithm::rectangle_patch(&source, area, &|| Ok(())).unwrap();
    let extracted =
        crate::raster_edit_algorithm::extract_content(&source, area, &patch, &|| Ok(())).unwrap();
    let layouts = vec![
        layout(patch, RasterRole::Repair, expected).unwrap(),
        layout(extracted, RasterRole::Extracted, expected).unwrap(),
    ];
    let drawings = layouts.iter().map(|r| drawing(r, 20., 20.)).collect();
    (drawings, layouts)
}
fn render(store: &Store, target: &Target) -> RgbaImage {
    let region = &store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == target.scene_id)
        .unwrap()
        .regions[0];
    crate::drawing_render::crop_region_with_layouts(
        &DynamicImage::ImageRgba8(synthetic()),
        region,
        None,
        |reference| {
            crate::rich_annotation_host::decode(
                &store.rich_layout(reference).map_err(|e| e.to_string())?,
            )
        },
    )
    .unwrap()
    .into_rgba8()
}
fn action(target: &Target, revision: u64, redo: bool) -> SceneCommand {
    if redo {
        SceneCommand::RedoDrawing {
            scene_id: target.scene_id.clone(),
            background_id: target.background.id.clone(),
            region_id: target.region.id.clone(),
            expected_revision: revision,
        }
    } else {
        SceneCommand::UndoDrawing {
            scene_id: target.scene_id.clone(),
            background_id: target.background.id.clone(),
            region_id: target.region.id.clone(),
            expected_revision: revision,
        }
    }
}
#[test]
fn raster_extract_two_layers_one_undo_move_redo_and_reopen() {
    let db = Db::new();
    let (mut store, target, expected) = setup(&db);
    let original = store.snapshot();
    let (drawings, layouts) = prepare(&expected);
    let saved = drawings.clone();
    store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings,
            layouts.clone(),
        )
        .unwrap();
    let scene = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == target.scene_id)
        .unwrap();
    assert_eq!(scene.regions[0].drawing_history.undo.len(), 1);
    assert_eq!(scene.regions[0].drawings, saved);
    assert_eq!(scene.background, Some(target.background.clone()));
    assert_eq!(render(&store, &target), synthetic());
    let mut moved = saved[1].clone();
    for p in &mut moved.points {
        p.x += 40.;
    }
    store
        .apply(SceneCommand::UpdateDrawing {
            scene_id: target.scene_id.clone(),
            background_id: target.background.id.clone(),
            region_id: target.region.id.clone(),
            expected_revision: 1,
            drawing: moved.clone(),
        })
        .unwrap();
    let output = render(&store, &target);
    assert_eq!(output.get_pixel(28, 28), &Rgba([240, 240, 240, 255]));
    assert_eq!(output.get_pixel(68, 28), &Rgba([20, 40, 60, 255]));
    store.apply(action(&target, 2, false)).unwrap();
    assert_eq!(render(&store, &target), synthetic());
    store.apply(action(&target, 3, false)).unwrap();
    let undone = store.snapshot();
    let scene = &undone.scenes[0];
    assert!(scene.regions[0].drawings.is_empty());
    assert_eq!(scene.background, original.scenes[0].background);
    assert_eq!(render(&store, &target), synthetic());
    store.apply(action(&target, 4, true)).unwrap();
    store.apply(action(&target, 5, true)).unwrap();
    let final_snapshot = store.snapshot();
    drop(store);
    let reopened = Store::open(&db.0).unwrap();
    assert_eq!(reopened.snapshot(), final_snapshot);
    assert_eq!(render(&reopened, &target), output);
    for l in &layouts {
        assert_eq!(reopened.rich_layout(l.reference()).unwrap().png(), l.png());
    }
    let mut patch = saved[0].clone();
    patch.points[0].x += 1.;
    patch.points[1].x += 1.;
    let mut reopened = reopened;
    assert!(reopened
        .apply(SceneCommand::UpdateDrawing {
            scene_id: target.scene_id.clone(),
            background_id: target.background.id.clone(),
            region_id: target.region.id.clone(),
            expected_revision: 6,
            drawing: patch
        })
        .is_err());
    assert!(reopened
        .apply(SceneCommand::RemoveDrawing {
            scene_id: target.scene_id.clone(),
            background_id: target.background.id.clone(),
            region_id: target.region.id.clone(),
            expected_revision: 6,
            drawing_id: saved[0].id.clone()
        })
        .is_err());
    assert_eq!(reopened.snapshot(), final_snapshot);
}
#[test]
fn attached_repair_follows_parent_move_scale_delete_history_and_reopen() {
    let db = Db::new();
    let (mut store, target, expected) = setup(&db);
    let (drawings, layouts) = prepare(&expected);
    let parent = drawings[1].clone();
    let root_repair = drawings[0].clone();
    store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings,
            layouts,
        )
        .unwrap();
    let region = store.snapshot().scenes[0].regions[0].clone();
    let fence = mewu_core::visual_source_fence(&target.background, &region).unwrap();
    let layout = layout_with_parent(
        RgbaImage::from_pixel(6, 6, Rgba([240, 240, 240, 255])),
        RasterRole::Repair,
        &fence,
        Some(parent.id.clone()),
    )
    .unwrap();
    let patch = drawing(&layout, 24., 24.);
    store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &fence,
            vec![patch.clone()],
            vec![layout.clone()],
        )
        .unwrap();
    assert_eq!(
        render(&store, &target).get_pixel(28, 28),
        &Rgba([240, 240, 240, 255])
    );
    let mut moved = parent.clone();
    for p in &mut moved.points {
        p.x += 40.;
    }
    let update = |revision, drawing| SceneCommand::UpdateDrawing {
        scene_id: target.scene_id.clone(),
        region_id: target.region.id.clone(),
        background_id: target.background.id.clone(),
        expected_revision: revision,
        drawing,
    };
    store.apply(update(2, moved.clone())).unwrap();
    let region = store.snapshot().scenes[0].regions[0].clone();
    assert_eq!(region.drawings[0], root_repair);
    assert_eq!(
        region.drawings[2].points[0],
        DrawingPoint { x: 64., y: 24. }
    );
    assert_eq!(
        serde_json::to_value(region.drawing_history.undo.last().unwrap()).unwrap()["batch"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        render(&store, &target).get_pixel(68, 28),
        &Rgba([240, 240, 240, 255])
    );
    let scaled = Drawing {
        points: vec![
            DrawingPoint { x: -10., y: -10. },
            DrawingPoint { x: 30., y: 30. },
        ],
        ..parent.clone()
    };
    store.apply(update(3, scaled.clone())).unwrap();
    let region = store.snapshot().scenes[0].regions[0].clone();
    assert_eq!(
        region.drawings[2].points,
        vec![
            DrawingPoint { x: -2., y: -2. },
            DrawingPoint { x: 10., y: 10. }
        ]
    );
    render(&store, &target);
    store.apply(action(&target, 4, false)).unwrap();
    assert_eq!(store.snapshot().scenes[0].regions[0].drawings[1], moved);
    store.apply(action(&target, 5, true)).unwrap();
    let saved = store.snapshot();
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(store.snapshot(), saved);
    assert_eq!(
        store.rich_layout(layout.reference()).unwrap().png(),
        layout.png()
    );
    let before = store.snapshot();
    assert!(store.apply(update(6, patch)).is_err());
    assert_eq!(store.snapshot(), before);
    store
        .apply(SceneCommand::RemoveDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region.id.clone(),
            background_id: target.background.id.clone(),
            expected_revision: 6,
            drawing_id: parent.id.clone(),
        })
        .unwrap();
    assert_eq!(
        store.snapshot().scenes[0].regions[0].drawings,
        vec![root_repair]
    );
    store.apply(action(&target, 7, false)).unwrap();
    assert_eq!(
        store.snapshot().scenes[0].regions[0].drawings,
        saved.scenes[0].regions[0].drawings
    );
}
#[test]
fn marquee_batch_is_one_atomic_geometry_step_and_rejects_forgery() {
    let db = Db::new();
    let (mut store, target, expected) = setup(&db);
    let (drawings, layouts) = prepare(&expected);
    let parent = drawings[1].clone();
    store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings,
            layouts,
        )
        .unwrap();
    let ink = Drawing {
        id: uuid::Uuid::new_v4().to_string(),
        kind: DrawingKind::Pen,
        color: "#123456".into(),
        stroke_width: 4.,
        points: vec![DrawingPoint { x: 5., y: 5. }],
        text: None,
        font_size: None,
        origin: None,
        rich: None,
    };
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: target.scene_id.clone(),
            region_id: target.region.id.clone(),
            background_id: target.background.id.clone(),
            expected_revision: 1,
            drawing: ink.clone(),
        })
        .unwrap();
    let mut values = vec![parent, ink];
    for d in &mut values {
        for p in &mut d.points {
            p.x += 10.;
            p.y += 5.;
        }
    }
    let batch = |revision, drawings| SceneCommand::UpdateDrawings {
        scene_id: target.scene_id.clone(),
        region_id: target.region.id.clone(),
        background_id: target.background.id.clone(),
        expected_revision: revision,
        drawings,
    };
    let original = store.snapshot();
    let mut forged = values.clone();
    forged[1].color = "#FF0000".into();
    assert!(store.apply(batch(2, forged)).is_err());
    assert_eq!(store.snapshot(), original);
    store.apply(batch(2, values.clone())).unwrap();
    let saved = store.snapshot();
    assert_eq!(
        serde_json::to_value(
            saved.scenes[0].regions[0]
                .drawing_history
                .undo
                .last()
                .unwrap()
        )
        .unwrap()["batch"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(store.apply(batch(2, values)).is_err());
    assert_eq!(store.snapshot(), saved);
    store.apply(action(&target, 3, false)).unwrap();
    assert_eq!(
        store.snapshot().scenes[0].regions[0].drawings,
        original.scenes[0].regions[0].drawings
    );
}
#[test]
fn raster_png_and_history_roll_back_together_on_real_sqlite_failure() {
    let db = Db::new();
    let (mut store, target, expected) = setup(&db);
    let before = store.snapshot();
    let (drawings, layouts) = prepare(&expected);
    let connection = rusqlite::Connection::open(&db.0).unwrap();
    connection.execute_batch("CREATE TRIGGER raster_fail BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings,
            layouts
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM drawing_layouts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    drop(store);
    assert_eq!(Store::open(&db.0).unwrap().snapshot(), before);
}
#[test]
fn raster_source_revision_frozen_closed_run_and_forged_png_identity_reject() {
    let db = Db::new();
    let (mut store, target, expected) = setup(&db);
    let (drawings, layouts) = prepare(&expected);
    let before = store.snapshot();
    let mut forged = drawings.clone();
    forged[1].rich.as_mut().unwrap().raster_sha256 = "a".repeat(64);
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            forged,
            layouts.clone()
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    let mut stale = expected.clone();
    stale.drawing_revision = 1;
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &stale,
            drawings.clone(),
            layouts.clone()
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "synthetic".into(),
        })
        .unwrap();
    store.begin_run(&target.scene_id).unwrap();
    let running = store.snapshot();
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings.clone(),
            layouts.clone()
        )
        .is_err());
    assert_eq!(store.snapshot(), running);
    store.cancel_run(&target.scene_id).unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let frozen = store.snapshot();
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings.clone(),
            layouts.clone()
        )
        .is_err());
    assert_eq!(store.snapshot(), frozen);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let closed = store.snapshot();
    assert!(store
        .append_raster_edit(
            &target.scene_id,
            &target.region.id,
            &expected,
            drawings,
            layouts
        )
        .is_err());
    assert_eq!(store.snapshot(), closed);
}
