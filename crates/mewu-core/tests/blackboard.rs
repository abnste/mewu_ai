// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use uuid::Uuid;

fn board() -> Asset {
    Asset {
        id: Uuid::new_v4().to_string(),
        name: "黑板.png".into(),
        kind: AssetKind::Image,
        path: "board.png".into(),
        width: Some(1280),
        height: Some(720),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
#[test]
fn blackboard_empty_scene_adopts_background_region_and_ai_reference_together() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.snapshot().active_scene_id;
    let asset = board();
    let result = store.create_blackboard(&id, asset.clone()).unwrap();
    assert_eq!(result.active_scene_id, id);
    assert_eq!(result.scenes.len(), 1);
    let scene = &result.scenes[0];
    assert_eq!(scene.background, Some(asset));
    assert_eq!(scene.regions.len(), 1);
    assert_eq!(scene.regions[0].width, 1280.);
    assert_eq!(scene.regions[0].height, 720.);
    assert_eq!(
        scene.refs,
        vec![Reference {
            kind: ReferenceKind::Region,
            id: scene.regions[0].id.clone()
        }]
    );
}
#[test]
fn blackboard_preserves_existing_draft_and_screenshot_in_a_frozen_scene() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.snapshot().active_scene_id;
    let mut original_image = board();
    original_image.name = "截图.png".into();
    store.set_background(&id, original_image).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: id.clone(),
            draft: "原有内容".into(),
        })
        .unwrap();
    let mut original = store.snapshot().scenes[0].clone();
    original.frozen = true;
    let result = store.create_blackboard(&id, board()).unwrap();
    assert_ne!(result.active_scene_id, id);
    assert_eq!(result.scenes[0], original);
    assert_eq!(result.scenes.len(), 2);
}
#[test]
fn blackboard_rejects_stale_running_and_invalid_targets_without_writes() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store.snapshot().active_scene_id;
    let before = store.snapshot();
    assert!(store.create_blackboard("stale", board()).is_err());
    assert_eq!(store.snapshot(), before);
    let mut invalid = board();
    invalid.width = Some(0);
    assert!(store.create_blackboard(&id, invalid).is_err());
    assert_eq!(store.snapshot(), before);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: id.clone(),
            draft: "正在回答".into(),
        })
        .unwrap();
    store.begin_run(&id).unwrap();
    let before = store.snapshot();
    assert!(store.create_blackboard(&id, board()).is_err());
    assert_eq!(store.snapshot(), before);
}
#[test]
fn blackboard_pixels_drawings_history_and_reference_survive_reopen() {
    let root = std::env::temp_dir().join(format!("mewu-blackboard-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("spaces.db");
    let mut store = Store::open(&path).unwrap();
    let id = store.snapshot().active_scene_id;
    let result = store.create_blackboard(&id, board()).unwrap();
    let region = result.scenes[0].regions[0].id.clone();
    let background = result.scenes[0].background.as_ref().unwrap().id.clone();
    let stroke = Drawing {
        id: Uuid::new_v4().to_string(),
        kind: DrawingKind::Pen,
        color: "#E8ECF4".into(),
        stroke_width: 4.,
        points: vec![
            DrawingPoint { x: 100., y: 100. },
            DrawingPoint { x: 300., y: 200. },
        ],
        text: None,
        font_size: None,
        origin: None,
        rich: None,
    };
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: id.clone(),
            region_id: region.clone(),
            background_id: background.clone(),
            expected_revision: 0,
            drawing: stroke.clone(),
        })
        .unwrap();
    let saved = store.snapshot();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot(), saved);
    store
        .apply(SceneCommand::UndoDrawing {
            scene_id: id.clone(),
            region_id: region.clone(),
            background_id: background.clone(),
            expected_revision: 1,
        })
        .unwrap();
    assert!(store.snapshot().scenes[0].regions[0].drawings.is_empty());
    store
        .apply(SceneCommand::RedoDrawing {
            scene_id: id,
            region_id: region,
            background_id: background,
            expected_revision: 2,
        })
        .unwrap();
    assert_eq!(store.snapshot().scenes[0].regions[0].drawings, vec![stroke]);
    drop(store);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[test]
fn blackboard_database_failure_leaves_no_partially_created_scene() {
    let root = std::env::temp_dir().join(format!("mewu-blackboard-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("spaces.db");
    let mut store = Store::open(&path).unwrap();
    let id = store.snapshot().active_scene_id;
    let before = store.snapshot();
    let database = Connection::open(&path).unwrap();
    database.execute_batch("CREATE TRIGGER reject_board BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture-write-failure'); END;").unwrap();
    assert!(store.create_blackboard(&id, board()).is_err());
    assert_eq!(store.snapshot(), before);
    drop(database);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot(), before);
    drop(store);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
