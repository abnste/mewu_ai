// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use uuid::Uuid;
fn asset(preview: bool) -> Asset {
    let id = Uuid::new_v4().to_string();
    Asset {
        path: format!(
            "D:/assets/{id}.{}.png",
            if preview { "board-preview" } else { "board" }
        ),
        id,
        name: "黑板.png".into(),
        kind: AssetKind::Image,
        width: Some(1280),
        height: Some(720),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn active(store: &Store) -> Scene {
    let snapshot = store.snapshot();
    snapshot
        .scenes
        .into_iter()
        .find(|scene| scene.id == snapshot.active_scene_id)
        .unwrap()
}
fn draw(store: &mut Store) {
    let scene = active(store);
    let region = &scene.regions[0];
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: scene.id,
            background_id: scene.background.unwrap().id,
            region_id: region.id.clone(),
            expected_revision: region.drawing_revision,
            drawing: Drawing {
                id: Uuid::new_v4().to_string(),
                kind: DrawingKind::Pen,
                color: "#ffffff".into(),
                stroke_width: 4.,
                points: vec![
                    DrawingPoint { x: 20., y: 20. },
                    DrawingPoint { x: 80., y: 80. },
                ],
                text: None,
                font_size: None,
                origin: None,
                rich: None,
            },
        })
        .unwrap();
}
#[test]
fn finish_returns_to_original_space_with_reference_and_keeps_editable_ink() {
    let mut store = Store::open_in_memory().unwrap();
    let parent_id = active(&store).id;
    store
        .set_background(
            &parent_id,
            Asset {
                name: "截图.png".into(),
                ..asset(true)
            },
        )
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: parent_id.clone(),
            draft: "原草稿".into(),
        })
        .unwrap();
    let parent_before = active(&store);
    store
        .create_blackboard_document(&parent_id, asset(false))
        .unwrap();
    draw(&mut store);
    let board = active(&store);
    let image = asset(true);
    let result = store
        .finish_blackboard_document(&board, image.clone())
        .unwrap();
    assert_eq!(result.active_scene_id, parent_id);
    let parent = active(&store);
    assert_eq!(parent.background, parent_before.background);
    assert_eq!(parent.draft, parent_before.draft);
    assert_eq!(parent.regions, parent_before.regions);
    assert_eq!(parent.items.len(), 1);
    assert_eq!(parent.items[0].asset, image);
    assert!(parent.refs.contains(&Reference {
        kind: ReferenceKind::Item,
        id: parent.items[0].id.clone()
    }));
    store
        .open_blackboard_document(&parent_id, &parent.items[0].id)
        .unwrap();
    let reopened = active(&store);
    assert_eq!(reopened.regions, board.regions);
    assert_eq!(reopened.background, board.background);
    store
        .apply(SceneCommand::UndoDrawing {
            scene_id: reopened.id.clone(),
            region_id: reopened.regions[0].id.clone(),
            background_id: reopened.background.unwrap().id,
            expected_revision: 1,
        })
        .unwrap();
    assert!(active(&store).regions[0].drawings.is_empty());
    let edited = active(&store);
    store
        .finish_blackboard_document(&edited, asset(true))
        .unwrap();
    assert_eq!(
        active(&store)
            .refs
            .iter()
            .filter(|reference| reference.kind == ReferenceKind::Item)
            .count(),
        1
    );
}
#[test]
fn separate_boards_reopen_the_corresponding_document_and_preserve_card_geometry() {
    let mut store = Store::open_in_memory().unwrap();
    let parent = active(&store).id;
    store
        .create_blackboard_document(&parent, asset(false))
        .unwrap();
    draw(&mut store);
    let first = active(&store);
    store
        .finish_blackboard_document(&first, asset(true))
        .unwrap();
    let mut card = active(&store).items[0].clone();
    card.x = 0.34;
    card.width = 0.25;
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: parent.clone(),
            item: card.clone(),
        })
        .unwrap();
    store
        .create_blackboard_document(&parent, asset(false))
        .unwrap();
    let second = active(&store);
    assert_ne!(first.id, second.id);
    store
        .finish_blackboard_document(&second, asset(true))
        .unwrap();
    let before = active(&store);
    store.open_blackboard_document(&parent, &card.id).unwrap();
    assert_eq!(active(&store).regions, first.regions);
    let scene = active(&store);
    store
        .finish_blackboard_document(&scene, asset(true))
        .unwrap();
    let after = active(&store);
    assert_eq!(after.items[0].x, 0.34);
    assert_eq!(after.items[0].width, 0.25);
    assert_eq!(after.items[1], before.items[1]);
}
#[test]
fn stale_finish_wrong_size_running_parent_and_forged_hint_are_rejected_atomically() {
    let mut store = Store::open_in_memory().unwrap();
    let parent = active(&store).id;
    store
        .create_blackboard_document(&parent, asset(false))
        .unwrap();
    let stale = active(&store);
    draw(&mut store);
    let before = store.snapshot();
    assert!(store
        .finish_blackboard_document(&stale, asset(true))
        .is_err());
    assert_eq!(store.snapshot(), before);
    let mut wrong = asset(true);
    wrong.width = Some(12);
    assert!(store
        .finish_blackboard_document(&active(&store), wrong)
        .is_err());
    assert_eq!(store.snapshot(), before);
    store
        .finish_blackboard_document(&active(&store), asset(true))
        .unwrap();
    let actual_card = active(&store).items[0].id.clone();
    let image = asset(true);
    store.add_asset(&parent, image).unwrap();
    let mut forged = active(&store).items[1].clone();
    forged.state = active(&store).items[0].state.clone();
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: parent.clone(),
            item: forged.clone(),
        })
        .unwrap();
    let before = store.snapshot();
    assert!(store.open_blackboard_document(&parent, &forged.id).is_err());
    assert_eq!(store.snapshot(), before);
    store
        .open_blackboard_document(&parent, &actual_card)
        .unwrap();
}
#[test]
fn interrupted_editing_and_finished_cards_survive_database_reopen() {
    let root = std::env::temp_dir().join(format!("mewu-board-document-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("spaces.db");
    let mut store = Store::open(&path).unwrap();
    let parent = active(&store).id;
    store
        .create_blackboard_document(&parent, asset(false))
        .unwrap();
    draw(&mut store);
    let before = store.snapshot();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot(), before);
    store
        .finish_blackboard_document(&active(&store), asset(true))
        .unwrap();
    let card = active(&store).items[0].id.clone();
    let before = store.snapshot();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot(), before);
    store.open_blackboard_document(&parent, &card).unwrap();
    assert_eq!(active(&store).regions[0].drawings.len(), 1);
    drop(store);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[test]
fn failed_save_does_not_replace_preview_reference_or_leave_editor() {
    let root = std::env::temp_dir().join(format!("mewu-board-rollback-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("spaces.db");
    let mut store = Store::open(&path).unwrap();
    let parent = active(&store).id;
    store
        .create_blackboard_document(&parent, asset(false))
        .unwrap();
    draw(&mut store);
    let before = store.snapshot();
    let database = rusqlite::Connection::open(&path).unwrap();
    database.execute_batch("CREATE TRIGGER reject_board BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture-write-failure'); END;").unwrap();
    assert!(store
        .finish_blackboard_document(&active(&store), asset(true))
        .is_err());
    assert_eq!(store.snapshot(), before);
    drop(database);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot(), before);
    drop(store);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[test]
fn old_blackboard_is_adopted_only_on_finish_without_losing_chat_draft_or_ink() {
    let mut store = Store::open_in_memory().unwrap();
    let parent = active(&store).id;
    store.create_blackboard(&parent, asset(false)).unwrap();
    draw(&mut store);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: parent.clone(),
            draft: "旧黑板草稿".into(),
        })
        .unwrap();
    let legacy = active(&store);
    store
        .finish_blackboard_document(&legacy, asset(true))
        .unwrap();
    let scene = active(&store);
    assert_eq!(scene.id, parent);
    assert_eq!(scene.draft, legacy.draft);
    assert!(scene.background.is_none());
    assert!(scene.regions.is_empty());
    store
        .open_blackboard_document(&parent, &scene.items[0].id)
        .unwrap();
    assert_eq!(active(&store).regions, legacy.regions);
}

#[test]
fn board_carries_all_asset_kinds_independently_and_keeps_original_sources() {
    let mut store = Store::open_in_memory().unwrap();
    let parent = active(&store).id;
    for kind in [AssetKind::Image,AssetKind::Html,AssetKind::Svg,AssetKind::Video,AssetKind::Text,AssetKind::File] {
        let mut source = asset(true);source.name="素材".into();source.kind=kind;
        if matches!(source.kind,AssetKind::Text|AssetKind::File){source.width=None;source.height=None;}
        store.add_asset(&parent,source).unwrap();
    }
    let before=active(&store);
    store.apply(SceneCommand::SetRefs{scene_id:parent.clone(),refs:vec![Reference{kind:ReferenceKind::Item,id:before.items[1].id.clone()}]}).unwrap();
    store.create_blackboard_document(&parent,asset(false)).unwrap();
    let board=active(&store);
    assert_eq!(board.items.len(),6);
    for (source,copy) in before.items.iter().zip(&board.items) {
        assert_ne!(source.id,copy.id);assert_eq!(source.asset,copy.asset);
        assert_eq!((source.x,source.y,source.width,source.height),(copy.x,copy.y,copy.width,copy.height));
        assert_eq!(copy.state.as_ref().unwrap()["__mewuBoardSource"]["itemId"],source.id);
    }
    assert!(board.refs.contains(&Reference{kind:ReferenceKind::Item,id:board.items[1].id.clone()}));
    let mut forged=board.items[1].clone();forged.state.as_mut().unwrap().remove("__mewuBoardSource");
    let snapshot=store.snapshot();
    assert!(store.apply(SceneCommand::UpdateItem{scene_id:board.id.clone(),item:forged}).is_err());
    assert_eq!(store.snapshot(),snapshot);
    store.apply(SceneCommand::RemoveItem{scene_id:board.id.clone(),item_id:board.items[1].id.clone()}).unwrap();
    assert_eq!(active(&store).items.len(),5);
    let source=store.snapshot().scenes.into_iter().find(|s|s.id==parent).unwrap();
    assert_eq!(&source.items[..6],before.items.as_slice());
    assert!(!active(&store).refs.iter().any(|r|r.id==board.items[1].id));
}

#[test]
fn background_anchored_recording_keeps_screen_placement_on_board() {
    let mut store=Store::open_in_memory().unwrap();let parent=active(&store).id;
    let mut screen=asset(true);screen.name="截图.png".into();screen.width=Some(1000);screen.height=Some(500);
    store.set_background(&parent,screen.clone()).unwrap();
    let mut recording=asset(true);recording.kind=AssetKind::Video;
    store.add_asset(&parent,recording).unwrap();let mut item=active(&store).items[0].clone();
    item.state=Some(serde_json::from_value(serde_json::json!({"coordinateSpace":"background","backgroundId":screen.id})).unwrap());
    store.apply(SceneCommand::UpdateItem{scene_id:parent.clone(),item:item.clone()}).unwrap();
    store.create_blackboard_document(&parent,asset(false)).unwrap();let copy=&active(&store).items[0];
    assert!((copy.x-item.x).abs()<1e-12);assert!((copy.y-(40.+item.y*640.)/720.).abs()<1e-12);
    assert!((copy.height-item.height*640./720.).abs()<1e-12);
    assert!(!copy.state.as_ref().unwrap().contains_key("coordinateSpace"));
    assert_eq!(store.snapshot().scenes.iter().find(|s|s.id==parent).unwrap().items[0],item);
}
