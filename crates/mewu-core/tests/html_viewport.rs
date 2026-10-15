// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-html-viewport-{}", Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn stored(&self) -> (i64, String) {
        Connection::open(&self.0)
            .unwrap()
            .query_row(
                "SELECT revision,payload FROM app_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
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
fn html(width: u32, height: u32) -> Asset {
    let id = Uuid::new_v4().to_string();
    Asset {
        path: format!("assets/{id}.html"),
        id,
        name: "widget.html".into(),
        kind: AssetKind::Html,
        width: Some(width),
        height: Some(height),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
        capture_coordinates: None,
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

#[test]
fn declared_html_viewport_is_saved_once_and_survives_reopen() {
    for (vw, vh, width, height) in [
        (1536, 864, 440, 120),
        (864, 1536, 600, 900),
        (1536, 864, 1600, 1200),
    ] {
        let db = TestDb::new();
        let mut store = Store::open(&db.0).unwrap();
        let scene_id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "keep draft".into(),
            })
            .unwrap();
        let before = db.stored();
        let asset = html(width, height);
        let snapshot = store
            .add_html_asset_in_viewport(&scene_id, asset.clone(), vw, vh)
            .unwrap();
        let scene = active(&store);
        let item = &scene.items[0];
        assert!((item.width * f64::from(vw) - f64::from(width)).abs() < 1e-10);
        assert!((item.height * f64::from(vh) - f64::from(height)).abs() < 1e-10);
        assert!(item.x >= 0. && item.y >= 0.);
        assert_eq!(item.asset, asset);
        assert_eq!(scene.draft, "keep draft");
        assert!(scene.refs.is_empty());
        assert_eq!(db.stored().0, before.0 + 1);
        drop(store);
        assert_eq!(Store::open(&db.0).unwrap().snapshot(), snapshot);
    }
}

#[test]
fn ordinary_html_add_and_import_keep_their_existing_geometry() {
    let mut store = Store::open_in_memory().unwrap();
    let scene_id = store.snapshot().active_scene_id;
    store.add_asset(&scene_id, html(440, 120)).unwrap();
    store
        .import_assets_in_viewport(&scene_id, vec![html(440, 120)], 1536, 864)
        .unwrap();
    let scene = active(&store);
    for item in &scene.items {
        assert_eq!(
            (item.x, item.y, item.width, item.height),
            (0.32, 0.2, 0.36, 0.5)
        );
    }
    assert_eq!(scene.refs.len(), 1);
}

#[test]
fn invalid_html_dimensions_or_native_viewport_never_commit_an_item() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    let before = db.stored();
    let snapshot = store.snapshot();
    for (vw, vh) in [(0, 864), (1536, 1), (32769, 864), (1536, u32::MAX), (2, 2)] {
        assert!(store
            .add_html_asset_in_viewport(&scene_id, html(440, 120), vw, vh)
            .is_err());
        assert_eq!(store.snapshot(), snapshot);
        assert_eq!(db.stored(), before);
    }
    for field in ["width", "height", "zero", "kind", "origin", "scale"] {
        let mut asset = html(440, 120);
        match field {
            "width" => asset.width = None,
            "height" => asset.height = None,
            "zero" => asset.height = Some(0),
            "kind" => asset.kind = AssetKind::Svg,
            "origin" => asset.origin_x = Some(0),
            "scale" => asset.scale_factor = Some(1.),
            _ => unreachable!(),
        }
        assert!(
            store
                .add_html_asset_in_viewport(&scene_id, asset, 1536, 864)
                .is_err(),
            "{field}"
        );
        assert_eq!(store.snapshot(), snapshot);
        assert_eq!(db.stored(), before);
    }
}

#[test]
fn stale_scene_and_sqlite_failure_leave_no_partially_sized_html_item() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let scene_id = store.snapshot().active_scene_id;
    store.apply(SceneCommand::NewScene).unwrap();
    let before = db.stored();
    let snapshot = store.snapshot();
    assert!(store
        .add_html_asset_in_viewport(&scene_id, html(440, 120), 1536, 864)
        .is_err());
    assert_eq!(store.snapshot(), snapshot);
    assert_eq!(db.stored(), before);
    let scene_id = snapshot.active_scene_id.clone();
    let database = Connection::open(&db.0).unwrap();
    database.execute_batch("CREATE TRIGGER reject_html BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic HTML failure'); END;").unwrap();
    assert!(store
        .add_html_asset_in_viewport(&scene_id, html(440, 120), 1536, 864)
        .is_err());
    assert_eq!(store.snapshot(), snapshot);
    assert_eq!(db.stored(), before);
}
