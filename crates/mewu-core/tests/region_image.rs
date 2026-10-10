// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-region-image-{}", Uuid::new_v4()));
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
fn setup(store: &mut Store) -> OcrTarget {
    let scene_id = store.snapshot().active_scene_id;
    let asset = image(800, 600);
    let target = OcrTarget {
        scene_id: scene_id.clone(),
        region_id: Uuid::new_v4().to_string(),
        background_id: asset.id.clone(),
        drawing_revision: 0,
        x: 120.,
        y: 180.,
        width: 600.,
        height: 300.,
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
        .iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap()
        .regions
        .iter()
        .find(|region| region.id == target.region_id)
        .unwrap()
        .clone()
}
fn target_now(store: &Store, old: &OcrTarget) -> OcrTarget {
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
fn drawing(point: DrawingPoint) -> Drawing {
    Drawing {
        origin: None,
        rich: None,
        id: Uuid::new_v4().to_string(),
        kind: DrawingKind::Pen,
        color: "#00aaff".into(),
        stroke_width: 4.,
        points: vec![point],
        text: None,
        font_size: None,
    }
}
fn add_drawing(
    store: &mut Store,
    target: &OcrTarget,
    value: Drawing,
) -> Result<Snapshot, CoreError> {
    store.apply(SceneCommand::AddDrawing {
        scene_id: target.scene_id.clone(),
        region_id: target.region_id.clone(),
        background_id: target.background_id.clone(),
        expected_revision: target.drawing_revision,
        drawing: value,
    })
}
fn ocr(width: u32, height: u32) -> OcrDocument {
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
fn replacement_fits_without_distortion_preserves_refs_draft_background_run_and_restart() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let original = setup(&mut store);
    add_drawing(
        &mut store,
        &original,
        drawing(DrawingPoint { x: 150., y: 220. }),
    )
    .unwrap();
    let target = target_now(&store, &original);
    store.set_region_ocr(&target, ocr(600, 300)).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "原图问题".into(),
        })
        .unwrap();
    let run = store.begin_run(&target.scene_id).unwrap();
    let refs = vec![Reference {
        kind: ReferenceKind::Region,
        id: target.region_id.clone(),
    }];
    store
        .apply(SceneCommand::SetRefs {
            scene_id: target.scene_id.clone(),
            refs: refs.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "继续编辑的草稿".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    let asset = image(600, 3000);
    let snapshot = store.set_region_image(&target, asset.clone()).unwrap();
    let scene = snapshot
        .scenes
        .iter()
        .find(|scene| scene.id == target.scene_id)
        .unwrap();
    let converted = &scene.regions[0];
    let wire_target = OcrTarget {
        x: converted.x,
        y: converted.y,
        width: converted.width,
        height: converted.height,
        drawing_revision: converted.drawing_revision,
        ..target.clone()
    };
    let decoded: OcrTarget =
        serde_json::from_str(&serde_json::to_string(&wire_target).unwrap()).unwrap();
    for (before, after) in [
        (wire_target.x, decoded.x),
        (wire_target.y, decoded.y),
        (wire_target.width, decoded.width),
        (wire_target.height, decoded.height),
    ] {
        assert_eq!(
            before.to_bits(),
            after.to_bits(),
            "target CAS must survive JSON exactly"
        );
    }
    assert_eq!(converted.id, target.region_id);
    assert_eq!(converted.image_override, Some(asset));
    assert!((converted.width - 102.).abs() < 1e-9 && (converted.height - 510.).abs() < 1e-9);
    assert!((converted.x - 120.).abs() < 1e-9 && (converted.y - 90.).abs() < 1e-9);
    assert_eq!(converted.drawing_revision, target.drawing_revision + 1);
    assert!(
        converted.drawings.is_empty()
            && converted.drawing_history.is_empty()
            && converted.ocr.is_none()
    );
    assert_eq!(scene.refs, refs);
    assert_eq!(scene.draft, "继续编辑的草稿");
    assert!(scene.frozen);
    assert_eq!(snapshot.active_scene_id, active);
    assert!(store.run_is_active(&target.scene_id, &run.run_id));
    assert!(run.regions[0].image_override.is_none());
    store
        .finish_run(&target.scene_id, &run.run_id, "后台完成")
        .unwrap();
    let saved = store.snapshot();
    drop(store);
    let mut reopened = Store::open(&db.0).unwrap();
    assert_eq!(reopened.snapshot(), saved);
    // The exact pre-restart target remains valid for a host operation.
    reopened
        .set_region_ocr(&wire_target, ocr(600, 3000))
        .unwrap();
}

#[test]
fn replacement_uses_source_pixels_for_drawings_ocr_and_preview_fences() {
    let mut store = Store::open_in_memory().unwrap();
    let original = setup(&mut store);
    let asset = image(200, 2400);
    store.set_region_image(&original, asset.clone()).unwrap();
    let target = target_now(&store, &original);
    let paint = drawing(DrawingPoint { x: 180., y: 2300. });
    add_drawing(&mut store, &target, paint.clone()).unwrap();
    let target = target_now(&store, &target);
    let before = store.snapshot();
    assert!(add_drawing(
        &mut store,
        &target,
        drawing(DrawingPoint { x: 201., y: 10. })
    )
    .is_err());
    assert_eq!(store.snapshot(), before);
    let (source, virtual_region) = store.region_for_ocr(&target).unwrap();
    assert_eq!(source, asset);
    assert_eq!(
        (
            virtual_region.x,
            virtual_region.y,
            virtual_region.width,
            virtual_region.height
        ),
        (0., 0., 200., 2400.)
    );
    assert_eq!(virtual_region.drawings, vec![paint]);
    assert!(
        virtual_region.image_override.is_none()
            && virtual_region.ocr.is_none()
            && virtual_region.drawing_history.is_empty()
    );
    assert!(store
        .set_region_ocr(
            &target,
            ocr(target.width.round() as u32, target.height.round() as u32)
        )
        .is_err());
    store.set_region_ocr(&target, ocr(200, 2400)).unwrap();
    assert_eq!(
        store
            .region_image_for_preview(
                &target.scene_id,
                &target.region_id,
                &target.background_id,
                &asset.id,
                target.drawing_revision
            )
            .unwrap(),
        asset
    );
    for (scene, region, background, source, revision) in [
        (
            "missing",
            target.region_id.as_str(),
            target.background_id.as_str(),
            asset.id.as_str(),
            target.drawing_revision,
        ),
        (
            target.scene_id.as_str(),
            "missing",
            target.background_id.as_str(),
            asset.id.as_str(),
            target.drawing_revision,
        ),
        (
            target.scene_id.as_str(),
            target.region_id.as_str(),
            "missing",
            asset.id.as_str(),
            target.drawing_revision,
        ),
        (
            target.scene_id.as_str(),
            target.region_id.as_str(),
            target.background_id.as_str(),
            "missing",
            target.drawing_revision,
        ),
        (
            target.scene_id.as_str(),
            target.region_id.as_str(),
            target.background_id.as_str(),
            asset.id.as_str(),
            target.drawing_revision - 1,
        ),
    ] {
        assert!(store
            .region_image_for_preview(scene, region, background, source, revision)
            .is_err());
    }
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(store
        .region_image_for_preview(
            &target.scene_id,
            &target.region_id,
            &target.background_id,
            &asset.id,
            target.drawing_revision
        )
        .is_ok());
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(store
        .region_image_for_preview(
            &target.scene_id,
            &target.region_id,
            &target.background_id,
            &asset.id,
            target.drawing_revision
        )
        .is_err());
}

#[test]
fn generic_geometry_cannot_inject_replace_or_remove_host_image() {
    let mut store = Store::open_in_memory().unwrap();
    let original = setup(&mut store);
    assert!(store
        .region_image_for_preview(
            &original.scene_id,
            &original.region_id,
            &original.background_id,
            &original.background_id,
            0
        )
        .is_err());
    let mut forged = region(&store, &original);
    forged.id = Uuid::new_v4().to_string();
    forged.image_override = Some(image(20, 40));
    assert!(store
        .apply(SceneCommand::AddRegion {
            scene_id: original.scene_id.clone(),
            region: forged.clone()
        })
        .is_err());
    forged.id = original.region_id.clone();
    forged.x = 100.;
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: original.scene_id.clone(),
            region: forged,
        })
        .unwrap();
    assert!(region(&store, &original).image_override.is_none());
    let target = target_now(&store, &original);
    let asset = image(200, 1000);
    store.set_region_image(&target, asset.clone()).unwrap();
    let target = target_now(&store, &target);
    add_drawing(
        &mut store,
        &target,
        drawing(DrawingPoint { x: 100., y: 900. }),
    )
    .unwrap();
    let target = target_now(&store, &target);
    store.set_region_ocr(&target, ocr(200, 1000)).unwrap();
    let old = region(&store, &target);
    let mut moved = old.clone();
    moved.image_override = None;
    moved.drawings.clear();
    moved.width /= 2.;
    moved.height /= 2.;
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: target.scene_id.clone(),
            region: moved,
        })
        .unwrap();
    let current = region(&store, &target);
    assert_eq!(current.image_override, Some(asset));
    assert_eq!(current.drawings, old.drawings);
    assert_eq!(current.drawing_history, old.drawing_history);
    assert_eq!(current.drawing_revision, old.drawing_revision + 1);
    assert!(current.ocr.is_none());
    assert!(store.region_for_ocr(&target).is_err());
}

#[test]
fn replacement_rejects_stale_targets_closed_regions_and_unsupported_assets_atomically() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store);
    let before = store.snapshot();
    let mut targets = Vec::new();
    for field in 0..8 {
        let mut wrong = target.clone();
        match field {
            0 => wrong.scene_id = "missing".into(),
            1 => wrong.region_id = "missing".into(),
            2 => wrong.background_id = "missing".into(),
            3 => wrong.drawing_revision += 1,
            4 => wrong.x += 1.,
            5 => wrong.y += 1.,
            6 => wrong.width += 1.,
            _ => wrong.height += 1.,
        }
        targets.push(wrong);
    }
    for wrong in targets {
        assert!(store.set_region_image(&wrong, image(200, 1000)).is_err());
        assert_eq!(store.snapshot(), before);
    }
    let mut assets = Vec::new();
    for kind in [
        AssetKind::File,
        AssetKind::Video,
        AssetKind::Html,
        AssetKind::Svg,
    ] {
        let mut asset = image(200, 1000);
        asset.kind = kind;
        assets.push(asset);
    }
    for (width, height) in [(0, 100), (100, 0), (16385, 1), (1, 16385), (8192, 8192)] {
        assets.push(image(width, height));
    }
    let mut invalid = image(200, 1000);
    invalid.id = "not-uuid".into();
    assets.push(invalid);
    let mut invalid = image(200, 1000);
    invalid.id = target.background_id.clone();
    assets.push(invalid);
    let mut invalid = image(200, 1000);
    invalid.width = None;
    assets.push(invalid);
    let mut invalid = image(200, 1000);
    invalid.origin_x = Some(-1);
    assets.push(invalid);
    let mut invalid = image(200, 1000);
    invalid.origin_y = Some(5);
    assets.push(invalid);
    let mut invalid = image(200, 1000);
    invalid.scale_factor = Some(1.25);
    assets.push(invalid);
    for asset in assets {
        assert!(store.set_region_image(&target, asset).is_err());
        assert_eq!(store.snapshot(), before);
    }
    let mut accepted = image(200, 1000);
    accepted.origin_x = Some(0);
    accepted.origin_y = Some(0);
    accepted.scale_factor = Some(1.);
    store.set_region_image(&target, accepted).unwrap();
    let current = target_now(&store, &target);
    let before = store.snapshot();
    assert!(store.set_region_image(&current, image(200, 1500)).is_err());
    assert_eq!(store.snapshot(), before);
    let fresh = setup(&mut store);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: fresh.scene_id.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    assert!(store.set_region_image(&fresh, image(200, 1000)).is_err());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn replacement_storage_failure_and_concurrent_writer_never_half_commit() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = setup(&mut store);
    let before = store.snapshot();
    let sql = Connection::open(&db.0).unwrap();
    sql.execute_batch("CREATE TRIGGER deny_image BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let payload: String = sql
        .query_row("SELECT payload FROM app_state", [], |row| row.get(0))
        .unwrap();
    assert!(store.set_region_image(&target, image(200, 1000)).is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        sql.query_row("SELECT payload FROM app_state", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        payload
    );
    sql.execute_batch("DROP TRIGGER deny_image").unwrap();
    let mut other = Store::open(&db.0).unwrap();
    let image = image(200, 1000);
    other.set_region_image(&target, image.clone()).unwrap();
    assert!(matches!(
        store.set_region_image(&target, image),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(store);
    drop(other);
    drop(sql);
    assert!(region(&Store::open(&db.0).unwrap(), &target)
        .image_override
        .is_some());
}

#[test]
fn old_json_defaults_and_corrupt_saved_image_are_not_rewritten() {
    let legacy: Region =
        serde_json::from_value(serde_json::json!({"id":"old","x":0,"y":0,"width":20,"height":30}))
            .unwrap();
    assert!(legacy.image_override.is_none());
    assert!(serde_json::to_value(legacy)
        .unwrap()
        .get("imageOverride")
        .is_none());
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = setup(&mut store);
    store.set_region_image(&target, image(200, 1000)).unwrap();
    let mut snapshot = store.snapshot();
    snapshot.scenes[0].regions[0]
        .image_override
        .as_mut()
        .unwrap()
        .height = Some(20000);
    let payload = serde_json::to_string(&snapshot).unwrap();
    drop(store);
    let sql = Connection::open(&db.0).unwrap();
    sql.execute("UPDATE app_state SET payload=?", [&payload])
        .unwrap();
    assert!(Store::open(&db.0).is_err());
    assert_eq!(
        sql.query_row("SELECT payload FROM app_state", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        payload
    );
}
