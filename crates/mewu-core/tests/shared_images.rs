// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
fn image() -> Asset {
    Asset {
        id: uuid::Uuid::new_v4().to_string(),
        kind: AssetKind::Image,
        path: "D:/assets/durable.png".into(),
        name: "贴图.png".into(),
        width: Some(300),
        height: Some(200),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
#[test]
fn independent_conversations_reference_same_immutable_object_without_duplicates() {
    let mut store = Store::open_in_memory().unwrap();
    let first = store.snapshot().active_scene_id;
    let asset = image();
    let snapshot = store.reference_shared_image(&first, asset.clone()).unwrap();
    assert_eq!(snapshot.scenes[0].items.len(), 1);
    assert_eq!(snapshot.scenes[0].refs.len(), 1);
    assert_eq!(
        snapshot,
        store.reference_shared_image(&first, asset.clone()).unwrap()
    );
    let next = store.apply(SceneCommand::NewScene).unwrap();
    let second = next.active_scene_id.clone();
    let attached = store
        .reference_shared_image(&second, asset.clone())
        .unwrap();
    let old = attached.scenes.iter().find(|s| s.id == first).unwrap();
    let new = attached.scenes.iter().find(|s| s.id == second).unwrap();
    assert_eq!(old.items[0].asset, new.items[0].asset);
    assert_eq!(new.refs.len(), 1);
    assert_ne!(old.items[0].id, new.items[0].id);
    let mut replaced = asset.clone();
    replaced.path = "D:/assets/replaced.png".into();
    assert!(store.reference_shared_image(&second, replaced).is_err());
    assert_eq!(store.snapshot(), attached);
    assert!(store.reference_shared_image(&first, asset).is_err());
    assert_eq!(store.snapshot(), attached);
}
#[test]
fn object_reference_survives_scene_close_and_store_reopen() {
    let root = std::env::temp_dir().join(format!("mewu-shared-image-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("storage.db");
    let mut store = Store::open(&path).unwrap();
    let first = store.snapshot().active_scene_id;
    let asset = image();
    store.reference_shared_image(&first, asset.clone()).unwrap();
    let closed = store
        .apply(SceneCommand::CloseScene {
            scene_id: first.clone(),
        })
        .unwrap();
    let next = closed.active_scene_id;
    store.reference_shared_image(&next, asset).unwrap();
    let snapshot = store.snapshot();
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.snapshot(), snapshot);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
