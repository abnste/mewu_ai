// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mewu-text-styles-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path.join("spaces.db"))
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
fn text() -> Drawing {
    Drawing {
        id: Uuid::new_v4().to_string(),
        kind: DrawingKind::Text,
        color: "#EE4848".into(),
        stroke_width: 2.,
        points: vec![DrawingPoint { x: 40., y: 40. }],
        text: Some("中文 Abc\n第二行".into()),
        font_size: Some(32.),
        text_style: None,
        origin: None,
        rich: None,
     text_runs: None, }
}
fn all_styles() -> DrawingTextStyle {
    DrawingTextStyle {
        bold: true,
        italic: true,
        underline: true,
        strikethrough: true,
    }
}
fn setup(store: &mut Store) -> (String, String, String) {
    let scene = store.snapshot().active_scene_id;
    let background = Uuid::new_v4().to_string();
    let region = Uuid::new_v4().to_string();
    store
        .set_background(
            &scene,
            Asset {
                id: background.clone(),
                name: "synthetic.png".into(),
                path: "synthetic.png".into(),
                kind: AssetKind::Image,
                width: Some(800),
                height: Some(600),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
                capture_coordinates: None,
            },
        )
        .unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene.clone(),
            region: Region {
                id: region.clone(),
                x: 0.,
                y: 0.,
                width: 800.,
                height: 600.,
                ..Default::default()
            },
        })
        .unwrap();
    (scene, region, background)
}
fn region(store: &Store, target: &(String, String, String)) -> Region {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|scene| scene.id == target.0)
        .unwrap()
        .regions
        .into_iter()
        .find(|region| region.id == target.1)
        .unwrap()
}
fn command(
    store: &Store,
    target: &(String, String, String),
    drawing: Drawing,
    add: bool,
) -> SceneCommand {
    let revision = region(store, target).drawing_revision;
    if add {
        SceneCommand::AddDrawing {
            scene_id: target.0.clone(),
            region_id: target.1.clone(),
            background_id: target.2.clone(),
            expected_revision: revision,
            drawing,
        }
    } else {
        SceneCommand::UpdateDrawing {
            scene_id: target.0.clone(),
            region_id: target.1.clone(),
            background_id: target.2.clone(),
            expected_revision: revision,
            drawing,
        }
    }
}

#[test]
fn drawing_text_styles_legacy_wire_stays_identical_and_rejects_untyped_style() {
    let legacy = serde_json::to_value(text()).unwrap();
    assert!(legacy.get("textStyle").is_none());
    let decoded: Drawing = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(decoded.text_style, None);
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    // The pre-style native descriptor uses f64, which serializes 32 as 32.0.
    // Compare the actual legacy schema's wire bytes, not an integer JSON fixture.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct LegacyVideoTextContent {
        version: u32,
        text: &'static str,
        color: &'static str,
        font_size: f64,
    }
    let legacy_wire = serde_json::to_vec(&LegacyVideoTextContent {
        version: 1,
        text: "旧视频",
        color: "#EE4848",
        font_size: 32.,
    })
    .unwrap();
    let legacy_video = serde_json::from_slice::<serde_json::Value>(&legacy_wire).unwrap();
    let decoded: VideoTextContent = serde_json::from_value(legacy_video.clone()).unwrap();
    assert_eq!(decoded.text_style, None);
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), legacy_wire);
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy_video);
    for style in [
        serde_json::json!({"italic":"yes"}),
        serde_json::json!({"fontFamily":"file:///secret"}),
        serde_json::json!({"bold":1}),
        serde_json::json!([]),
        serde_json::json!([true, false, false, false]),
    ] {
        let mut value = legacy.clone();
        value["textStyle"] = style.clone();
        assert!(serde_json::from_value::<Drawing>(value).is_err());
        let mut value = legacy_video.clone();
        value["textStyle"] = style;
        assert!(serde_json::from_value::<VideoTextContent>(value).is_err());
    }
    let mut styled = text();
    styled.text_style = Some(all_styles());
    assert_eq!(
        serde_json::from_value::<Drawing>(serde_json::to_value(&styled).unwrap()).unwrap(),
        styled
    );
    for invalid in [
        r#"{"bold":true,"bold":false}"#,
        r#"{"italic":false,"italic":true}"#,
        r#"{"underline":true,"underline":true}"#,
        r#"{"strikethrough":false,"strikethrough":true}"#,
    ] {
        assert!(serde_json::from_str::<DrawingTextStyle>(invalid).is_err());
    }
    assert_eq!(
        serde_json::from_str::<DrawingTextStyle>("{}").unwrap(),
        DrawingTextStyle::default()
    );
    assert_eq!(
        serde_json::from_str::<DrawingTextStyle>(
            r#"{"bold":true,"italic":true,"underline":true,"strikethrough":true}"#
        )
        .unwrap(),
        all_styles()
    );
}

#[test]
fn drawing_text_styles_persist_reopen_and_undo_without_replacing_text_or_geometry() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let original = text();
    store
        .apply(command(&store, &target, original.clone(), true))
        .unwrap();
    let mut styled = original.clone();
    styled.text_style = Some(all_styles());
    store
        .apply(command(&store, &target, styled.clone(), false))
        .unwrap();
    assert_eq!(region(&store, &target).drawings, vec![styled.clone()]);
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(region(&store, &target).drawings, vec![styled.clone()]);
    for (redo, expected) in [(false, original), (true, styled)] {
        let expected_revision = region(&store, &target).drawing_revision;
        let scene_id = target.0.clone();
        let region_id = target.1.clone();
        let background_id = target.2.clone();
        let action = if redo {
            SceneCommand::RedoDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
            }
        } else {
            SceneCommand::UndoDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
            }
        };
        store.apply(action).unwrap();
        assert_eq!(region(&store, &target).drawings, vec![expected]);
    }
}

#[test]
fn drawing_text_styles_reject_nontext_and_keep_failed_transaction_unchanged() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let original = text();
    store
        .apply(command(&store, &target, original.clone(), true))
        .unwrap();
    let before = serde_json::to_value(store.snapshot()).unwrap();
    let mut invalid = original.clone();
    invalid.kind = DrawingKind::Pen;
    invalid.text = None;
    invalid.font_size = None;
    invalid.id = Uuid::new_v4().to_string();
    invalid.text_style = Some(all_styles());
    assert!(store
        .apply(command(&store, &target, invalid, true))
        .is_err());
    assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
    let mut styled = original;
    styled.text_style = Some(all_styles());
    Connection::open(&file.0).unwrap().execute_batch("CREATE TRIGGER text_style_failure BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
    assert!(store
        .apply(command(&store, &target, styled, false))
        .is_err());
    assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
    drop(store);
    let store = Store::open(&file.0).unwrap();
    assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
}

#[test]
fn drawing_text_box_resize_persists_reopens_and_undo_restores_legacy_anchor() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.private/remake-review/text-box-20261020-001")
        .join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let file = TestDb(directory.join("spaces.db"));
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let mut original = text();
    original.text_style = Some(all_styles());
    store.apply(command(&store, &target, original.clone(), true)).unwrap();
    let mut framed = original.clone();
    framed.points.push(DrawingPoint { x: 190., y: 180. });
    store.apply(command(&store, &target, framed.clone(), false)).unwrap();
    assert_eq!(region(&store, &target).drawings, vec![framed.clone()]);
    assert_eq!(region(&store, &target).drawing_history.undo.len(), 2);
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(region(&store, &target).drawings, vec![framed.clone()]);
    for (redo, expected) in [(false, original), (true, framed.clone())] {
        let expected_revision = region(&store, &target).drawing_revision;
        let scene_id = target.0.clone();
        let region_id = target.1.clone();
        let background_id = target.2.clone();
        store.apply(if redo {
            SceneCommand::RedoDrawing { scene_id, region_id, background_id, expected_revision }
        } else {
            SceneCommand::UndoDrawing { scene_id, region_id, background_id, expected_revision }
        }).unwrap();
        assert_eq!(region(&store, &target).drawings, vec![expected]);
    }
    let before = serde_json::to_value(store.snapshot()).unwrap();
    for point in [DrawingPoint { x: 40., y: 180. }, DrawingPoint { x: 190., y: 39. }, DrawingPoint { x: 801., y: 180. }] {
        let mut invalid = framed.clone();
        invalid.points[1] = point;
        assert!(store.apply(command(&store, &target, invalid, false)).is_err());
        assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
    }
}

#[test]
fn literal_text_runs_validate_exact_unicode_and_closed_fields() {
    let runs = vec![DrawingTextRun { text: "中文 ".into(), color: Some("#3366CC".into()), text_style: Some(all_styles()) }, DrawingTextRun { text: "é 👨‍👩‍👧".into(), color: None, text_style: Some(DrawingTextStyle::default()) }];
    validate_text_runs("中文 é 👨‍👩‍👧", Some(&runs)).unwrap();
    for invalid in [vec![], vec![DrawingTextRun { text: "".into(), color: None, text_style: None }], vec![DrawingTextRun { text: "错文".into(), color: None, text_style: None }], vec![DrawingTextRun { text: "中文 é 👨‍👩‍👧".into(), color: Some("red;url(x)".into()), text_style: None }]] {
        assert!(validate_text_runs("中文 é 👨‍👩‍👧", Some(&invalid)).is_err());
    }
    for invalid in [r#"{"text":"a","html":"<b>a</b>"}"#, r#"{"text":"a","textStyle":{"bold":"true"}}"#, r#"{"text":"a","textStyle":{"fontFamily":"url(x)"}}"#] {
        assert!(serde_json::from_str::<DrawingTextRun>(invalid).is_err());
    }
}

#[test]
fn literal_text_runs_persist_migrate_reopen_and_undo_as_one_document() {
    let file = TestDb::new();
    let mut store = Store::open(&file.0).unwrap();
    let target = setup(&mut store);
    let original = text();
    store.apply(command(&store, &target, original.clone(), true)).unwrap();
    drop(store);
    // Simulate the immediately preceding writer. The upgrade must not rewrite
    // its payload, history or IDs. Migration advances CAS once so an old writer
    // cannot commit over a document using the new rich-text schema.
    let db = Connection::open(&file.0).unwrap();
    db.pragma_update(None, "user_version", 13).unwrap();
    let before: (String, i64) = db.query_row("SELECT payload,revision FROM app_state WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    drop(db);
    let mut store = Store::open(&file.0).unwrap();
    let db = Connection::open(&file.0).unwrap();
    let after: (String, i64) = db.query_row("SELECT payload,revision FROM app_state WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(before.0, after.0);
    assert_eq!(before.1 + 1, after.1);
    assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0)).unwrap(), STORAGE_SCHEMA_VERSION);
    drop(db);
    let mut styled = original.clone();
    styled.text_runs = Some(vec![DrawingTextRun { text: "中文".into(), color: Some("#3366CC".into()), text_style: Some(all_styles()) }, DrawingTextRun { text: " Abc\n第二行".into(), color: None, text_style: None }]);
    styled.points = vec![DrawingPoint { x: 20., y: 30. }, DrawingPoint { x: 280., y: 150. }];
    store.apply(command(&store, &target, styled.clone(), false)).unwrap();
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(region(&store, &target).drawings, vec![styled.clone()]);
    for (redo, expected) in [(false, original), (true, styled.clone())] {
        let (scene_id, region_id, background_id) = target.clone();
        let expected_revision = region(&store, &target).drawing_revision;
        store.apply(if redo { SceneCommand::RedoDrawing { scene_id, region_id, background_id, expected_revision } } else { SceneCommand::UndoDrawing { scene_id, region_id, background_id, expected_revision } }).unwrap();
        assert_eq!(region(&store, &target).drawings, vec![expected]);
    }
    let before = serde_json::to_value(store.snapshot()).unwrap();
    styled.text_runs.as_mut().unwrap()[0].text = "被替换".into();
    assert!(store.apply(command(&store, &target, styled, false)).is_err());
    assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
}
