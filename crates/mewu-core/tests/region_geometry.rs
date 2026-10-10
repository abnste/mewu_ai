// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-geometry-{}", Uuid::new_v4()));
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
fn setup(store: &mut Store, replacement: bool) -> OcrTarget {
    let scene_id = store.snapshot().active_scene_id;
    let background = image(800, 600);
    let target = OcrTarget {
        scene_id: scene_id.clone(),
        region_id: Uuid::new_v4().to_string(),
        background_id: background.id.clone(),
        drawing_revision: 0,
        x: 20.,
        y: 30.,
        width: 600.,
        height: 400.,
    };
    store.set_background(&scene_id, background).unwrap();
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
    if replacement {
        store.set_region_image(&target, image(300, 1200)).unwrap();
    }
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: "保留草稿".into(),
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
    current(store, &target)
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
    let r = region(store, target);
    OcrTarget {
        drawing_revision: r.drawing_revision,
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
        ..target.clone()
    }
}
fn command(store: &Store, target: &OcrTarget, to: RegionGeometry) -> SceneCommand {
    let r = region(store, target);
    SceneCommand::SetRegionGeometry {
        scene_id: target.scene_id.clone(),
        region_id: target.region_id.clone(),
        background_id: target.background_id.clone(),
        source_id: r
            .image_override
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| target.background_id.clone()),
        expected_revision: r.drawing_revision,
        from: (&r).into(),
        to,
        edit_id: None,
    }
}
fn shifted(store: &Store, target: &OcrTarget) -> RegionGeometry {
    let mut geometry = RegionGeometry::from(&region(store, target));
    geometry.x += 1.;
    geometry
}
fn add_drawing(store: &mut Store, target: &OcrTarget) {
    let target = current(store, target);
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: target.scene_id,
            region_id: target.region_id,
            background_id: target.background_id,
            expected_revision: target.drawing_revision,
            drawing: Drawing {
                origin: None,
                rich: None,
                id: Uuid::new_v4().to_string(),
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
fn save_derived(store: &mut Store, target: &OcrTarget, text: &str) {
    let (_, input) = store.region_for_translation(target).unwrap();
    let (width, height) = (input.width.round() as u32, input.height.round() as u32);
    let mut selection = ocr(width, height, text);
    selection.engine = "mewu.translation.layout.v1".into();
    store
        .set_region_translation(
            target,
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
    store
        .set_region_ocr(target, ocr(width, height, text))
        .unwrap();
}
fn db_revision(db: &Connection) -> i64 {
    db.query_row("SELECT revision FROM app_state WHERE id = 1", [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn geometry_wire_is_typed_camel_case_and_rejects_extra_or_duplicate_fields() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store, false);
    let command = command(&store, &target, shifted(&store, &target));
    let value = serde_json::to_value(&command).unwrap();
    assert_eq!(value["type"], "set_region_geometry");
    assert_eq!(value["sceneId"], target.scene_id);
    assert_eq!(value["sourceId"], target.background_id);
    assert_eq!(
        serde_json::from_value::<SceneCommand>(value.clone()).unwrap(),
        command
    );
    for field in ["ocr", "translation", "drawings", "imageOverride"] {
        let mut bad = value.clone();
        bad["to"][field] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<SceneCommand>(bad).is_err(),
            "{field}"
        );
    }
    let mut bad = value;
    bad["region"] = serde_json::json!({});
    assert!(serde_json::from_value::<SceneCommand>(bad).is_err());
    assert!(serde_json::from_str::<RegionGeometry>(
        r#"{"x":1,"x":2,"y":0,"width":10,"height":10}"#
    )
    .is_err());
    assert!(serde_json::from_str::<RegionGeometry>(r#"{"x":1,"y":0,"width":10}"#).is_err());
}

#[test]
fn ordinary_crop_clears_derived_pixels_but_keeps_absolute_drawing_history_and_refs() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store, false);
    add_drawing(&mut store, &target);
    let target = current(&store, &target);
    save_derived(&mut store, &target, "旧裁图");
    let before = store.snapshot();
    let old = region(&store, &target);
    let to = shifted(&store, &target);
    store.apply(command(&store, &target, to)).unwrap();
    let actual = region(&store, &target);
    let mut expected = old.clone();
    expected.x = to.x;
    expected.drawing_revision += 1;
    expected.ocr = None;
    expected.translation = None;
    assert_eq!(actual, expected);
    let after = store.snapshot();
    assert_eq!(after.active_scene_id, before.active_scene_id);
    assert_eq!(after.scenes[0].draft, before.scenes[0].draft);
    assert_eq!(after.scenes[0].refs, before.scenes[0].refs);
    assert!(matches!(
        store.set_region_ocr(&target, old.ocr.unwrap().document),
        Err(CoreError::OcrConflict)
    ));
    let saved = old.translation.unwrap();
    assert!(matches!(
        store.set_region_translation(&target, saved.document, saved.overlay, saved.selection),
        Err(CoreError::TranslationConflict)
    ));
}

#[test]
fn override_move_and_resize_keep_latest_same_revision_documents_and_restart_exactly() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = setup(&mut store, true);
    add_drawing(&mut store, &target);
    let target = current(&store, &target);
    save_derived(&mut store, &target, "此前内容");
    let mut to = shifted(&store, &target);
    to.width += 10.;
    to.height -= 20.;
    let pending = command(&store, &target, to);
    // Both derived layers can finish without advancing drawing_revision.
    save_derived(&mut store, &target, "请求到达前刚完成的新内容");
    let mut expected = region(&store, &target);
    assert_eq!(expected.drawing_revision, target.drawing_revision);
    expected.x = to.x;
    expected.width = to.width;
    expected.height = to.height;
    expected.drawing_revision += 1;
    expected.ocr.as_mut().unwrap().drawing_revision = expected.drawing_revision;
    expected.translation.as_mut().unwrap().drawing_revision = expected.drawing_revision;
    store.apply(pending).unwrap();
    assert_eq!(region(&store, &target), expected);
    let snapshot = store.snapshot();
    drop(store);
    let reopened = Store::open(&db.0).unwrap();
    assert_eq!(reopened.snapshot(), snapshot);
}

#[test]
fn every_target_component_and_competing_edit_is_an_exact_cas_fence() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store, false);
    let original = command(&store, &target, shifted(&store, &target));
    let before = store.snapshot();
    for field in [
        "sceneId",
        "regionId",
        "backgroundId",
        "sourceId",
        "expectedRevision",
        "from",
    ] {
        let mut bad = serde_json::to_value(&original).unwrap();
        match field {
            "expectedRevision" => bad[field] = serde_json::json!(1),
            "from" => bad["from"]["x"] = serde_json::json!(target.x + 0.000000001),
            _ => bad[field] = serde_json::json!(Uuid::new_v4().to_string()),
        }
        let result = store.apply(serde_json::from_value(bad).unwrap());
        assert!(
            matches!(result, Err(CoreError::GeometryConflict)),
            "{field}: {result:?}"
        );
        assert_eq!(store.snapshot(), before);
    }
    store.apply(original.clone()).unwrap();
    let committed = store.snapshot();
    assert!(matches!(
        store.apply(original),
        Err(CoreError::GeometryConflict)
    ));
    assert_eq!(store.snapshot(), committed);
    let pending = command(&store, &target, shifted(&store, &target));
    add_drawing(&mut store, &target);
    let drawn = store.snapshot();
    assert!(matches!(
        store.apply(pending),
        Err(CoreError::GeometryConflict)
    ));
    assert_eq!(store.snapshot(), drawn);
}

#[test]
fn source_replacement_and_background_replacement_reject_old_commands() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store, false);
    store.set_region_image(&target, image(300, 1200)).unwrap();
    let mut old_source =
        serde_json::to_value(command(&store, &target, shifted(&store, &target))).unwrap();
    old_source["sourceId"] = serde_json::json!(target.background_id);
    let before = store.snapshot();
    assert!(matches!(
        store.apply(serde_json::from_value(old_source).unwrap()),
        Err(CoreError::GeometryConflict)
    ));
    assert_eq!(store.snapshot(), before);
    let pending = command(&store, &target, shifted(&store, &target));
    store
        .set_background(&target.scene_id, image(800, 600))
        .unwrap();
    let replacement = store.snapshot();
    assert!(matches!(
        store.apply(pending),
        Err(CoreError::GeometryConflict)
    ));
    assert_eq!(store.snapshot(), replacement);
}

#[test]
fn only_the_idle_active_open_scene_accepts_geometry_changes() {
    for mode in ["inactive", "frozen", "closed", "running"] {
        let mut store = Store::open_in_memory().unwrap();
        let target = setup(&mut store, false);
        let pending = command(&store, &target, shifted(&store, &target));
        match mode {
            "inactive" => {
                store.apply(SceneCommand::NewScene).unwrap();
            }
            "frozen" => {
                store
                    .apply(SceneCommand::FreezeScene {
                        scene_id: target.scene_id.clone(),
                    })
                    .unwrap();
            }
            "closed" => {
                store
                    .apply(SceneCommand::CloseScene {
                        scene_id: target.scene_id.clone(),
                    })
                    .unwrap();
            }
            "running" => {
                store.begin_run(&target.scene_id).unwrap();
            }
            _ => unreachable!(),
        }
        let before = store.snapshot();
        let result = store.apply(pending);
        if mode == "running" {
            assert!(matches!(result, Err(CoreError::RunInProgress)));
        } else {
            assert!(
                matches!(result, Err(CoreError::GeometryConflict)),
                "{mode}: {result:?}"
            );
        }
        assert_eq!(store.snapshot(), before);
    }
}

#[test]
fn invalid_bounds_are_rejected_without_clamping_or_partial_changes() {
    let mut store = Store::open_in_memory().unwrap();
    let target = setup(&mut store, true);
    save_derived(&mut store, &target, "不应丢失");
    let geometry = RegionGeometry::from(&region(&store, &target));
    let before = store.snapshot();
    let invalid = [
        RegionGeometry {
            x: f64::NAN,
            ..geometry
        },
        RegionGeometry {
            y: f64::INFINITY,
            ..geometry
        },
        RegionGeometry {
            width: f64::NEG_INFINITY,
            ..geometry
        },
        RegionGeometry {
            height: f64::NAN,
            ..geometry
        },
        RegionGeometry { x: -1., ..geometry },
        RegionGeometry { y: -1., ..geometry },
        RegionGeometry {
            width: 0.,
            ..geometry
        },
        RegionGeometry {
            height: -1.,
            ..geometry
        },
        RegionGeometry {
            width: f64::MAX,
            ..geometry
        },
        RegionGeometry {
            x: 800. - geometry.width + 0.0000001,
            ..geometry
        },
        RegionGeometry {
            y: 600. - geometry.height + 0.0000001,
            ..geometry
        },
    ];
    for to in invalid {
        assert!(
            matches!(
                store.apply(command(&store, &target, to)),
                Err(CoreError::Invalid(_))
            ),
            "{to:?}"
        );
        assert_eq!(store.snapshot(), before);
    }
}

#[test]
fn no_change_neither_writes_nor_touches_even_when_database_writes_would_fail() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let target = setup(&mut store, true);
    save_derived(&mut store, &target, "保留全部文档");
    let sql = Connection::open(&db.0).unwrap();
    sql.execute_batch("CREATE TRIGGER deny_geometry BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let before = store.snapshot();
    let revision = db_revision(&sql);
    let same = RegionGeometry::from(&region(&store, &target));
    assert_eq!(store.apply(command(&store, &target, same)).unwrap(), before);
    assert_eq!(store.snapshot(), before);
    assert_eq!(db_revision(&sql), revision);
}

#[test]
fn storage_failure_keeps_geometry_revision_and_documents_atomic() {
    for replacement in [false, true] {
        let db = TestDb::new();
        let mut store = Store::open(&db.0).unwrap();
        let target = setup(&mut store, replacement);
        add_drawing(&mut store, &target);
        let target = current(&store, &target);
        save_derived(&mut store, &target, "不可半写");
        let sql = Connection::open(&db.0).unwrap();
        sql.execute_batch("CREATE TRIGGER deny_geometry BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let before = store.snapshot();
        let revision = db_revision(&sql);
        assert!(matches!(
            store.apply(command(&store, &target, shifted(&store, &target))),
            Err(CoreError::Database(_))
        ));
        assert_eq!(store.snapshot(), before);
        assert_eq!(db_revision(&sql), revision);
        let reopened = Store::open(&db.0).unwrap();
        assert_eq!(reopened.snapshot(), before);
    }
}

#[test]
fn another_writer_cannot_be_overwritten_even_when_region_revision_is_unchanged() {
    let db = TestDb::new();
    let mut first = Store::open(&db.0).unwrap();
    let target = setup(&mut first, true);
    let mut stale = Store::open(&db.0).unwrap();
    let pending = command(&stale, &target, shifted(&stale, &target));
    let stale_before = stale.snapshot();
    save_derived(&mut first, &target, "另一写者新文档");
    let winning = first.snapshot();
    assert!(matches!(
        stale.apply(pending),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(stale.snapshot(), stale_before);
    let reopened = Store::open(&db.0).unwrap();
    assert_eq!(reopened.snapshot(), winning);
}
