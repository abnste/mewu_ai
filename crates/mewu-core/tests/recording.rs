// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TestDb {
    directory: PathBuf,
    path: PathBuf,
}

impl TestDb {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("mewu-recording-test-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self {
            path: directory.join("scenes.sqlite3"),
            directory,
        }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

fn persisted(path: &Path) -> (i64, Snapshot) {
    let connection = Connection::open(path).unwrap();
    let (revision, payload): (i64, String) = connection
        .query_row(
            "SELECT revision, payload FROM app_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    (revision, serde_json::from_str(&payload).unwrap())
}

fn asset(kind: AssetKind) -> Asset {
    let id = Uuid::new_v4().to_string();
    let is_video = kind == AssetKind::Video;
    let extension = if is_video { "mp4" } else { "png" };
    Asset {
        path: format!("assets/{id}.{extension}"),
        id,
        name: format!("fixture.{extension}"),
        kind,
        width: Some(if is_video { 960 } else { 1920 }),
        height: Some(if is_video { 540 } else { 1080 }),
        origin_x: (!is_video).then_some(-1920),
        origin_y: (!is_video).then_some(-1080),
        scale_factor: (!is_video).then_some(1.5),
    }
}

fn prepare(store: &mut Store) -> (String, Asset, Region) {
    let scene_id = store.snapshot().active_scene_id;
    let background = asset(AssetKind::Image);
    store.set_background(&scene_id, background.clone()).unwrap();
    let region = Region {
        id: Uuid::new_v4().to_string(),
        x: 192.,
        y: 108.,
        width: 960.,
        height: 540.,
        ..Default::default()
    };
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.clone(),
            region: region.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: "保留录屏后的问题".into(),
        })
        .unwrap();
    (scene_id, background, region)
}

fn scene<'a>(snapshot: &'a Snapshot, scene_id: &str) -> &'a Scene {
    snapshot
        .scenes
        .iter()
        .find(|scene| scene.id == scene_id)
        .unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn recording_replaces_only_its_region_and_refs_in_one_persisted_revision() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let (scene_id, background, region) = prepare(&mut store);
    let other_region = Region {
        id: Uuid::new_v4().to_string(),
        x: 10.,
        y: 20.,
        width: 50.,
        height: 60.,
        ..Default::default()
    };
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.clone(),
            region: other_region.clone(),
        })
        .unwrap();
    let snapshot = store.add_asset(&scene_id, asset(AssetKind::Image)).unwrap();
    let other_item = scene(&snapshot, &scene_id).items[0].clone();
    let surviving_refs = vec![
        Reference {
            kind: ReferenceKind::Region,
            id: other_region.id.clone(),
        },
        Reference {
            kind: ReferenceKind::Item,
            id: other_item.id.clone(),
        },
    ];
    let mut refs = surviving_refs.clone();
    refs.insert(
        0,
        Reference {
            kind: ReferenceKind::Region,
            id: region.id.clone(),
        },
    );
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene_id.clone(),
            refs,
        })
        .unwrap();
    let (revision, before) = persisted(&file.path);
    let mut video = asset(AssetKind::Video);
    video.width = Some(960);
    video.height = Some(540);
    let after = store
        .finish_recording(&scene_id, &background.id, &region, video.clone())
        .unwrap();
    let recorded = scene(&after, &scene_id);
    assert_eq!(after.active_scene_id, before.active_scene_id);
    assert_eq!(recorded.draft, "保留录屏后的问题");
    assert_eq!(recorded.background, Some(background.clone()));
    assert_eq!(recorded.regions, vec![other_region]);
    assert_eq!(recorded.items.len(), 2);
    assert_eq!(recorded.items[0], other_item);
    let item = &recorded.items[1];
    assert_eq!(item.asset, video);
    // Image-relative pixels become space fractions; desktop origins and DPI
    // never enter this conversion, including monitors left/above the primary.
    near(item.x, 0.1);
    near(item.y, 0.1);
    near(item.width, 0.5);
    near(item.height, 0.5);
    let mut expected_refs = surviving_refs;
    expected_refs.push(Reference {
        kind: ReferenceKind::Item,
        id: item.id.clone(),
    });
    assert_eq!(recorded.refs, expected_refs);
    assert_eq!(persisted(&file.path), (revision + 1, after.clone()));
    assert!(store
        .finish_recording(&scene_id, &background.id, &region, video)
        .is_err());
    assert_eq!(store.snapshot(), after);
    assert_eq!(persisted(&file.path), (revision + 1, after));
}

#[test]
fn frozen_recording_completion_keeps_active_scene_and_survives_restart_and_restore() {
    let file = TestDb::new();
    let (a, b, completed);
    {
        let mut store = Store::open(&file.path).unwrap();
        let (scene_id, background, region) = prepare(&mut store);
        a = scene_id;
        b = store
            .apply(SceneCommand::FreezeScene {
                scene_id: a.clone(),
            })
            .unwrap()
            .active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: b.clone(),
                draft: "另一会话仍在这里".into(),
            })
            .unwrap();
        let before = store.snapshot();
        completed = store
            .finish_recording(&a, &background.id, &region, asset(AssetKind::Video))
            .unwrap();
        assert_eq!(completed.active_scene_id, b);
        assert_eq!(scene(&completed, &b), scene(&before, &b));
        let recorded = scene(&completed, &a);
        assert!(recorded.frozen);
        assert!(recorded.regions.is_empty());
        // Recording need not have been an existing screenshot reference.
        assert_eq!(
            recorded.refs,
            vec![Reference {
                kind: ReferenceKind::Item,
                id: recorded.items[0].id.clone()
            }]
        );
    }
    let mut reopened = Store::open(&file.path).unwrap();
    assert_eq!(reopened.snapshot(), completed);
    let restored = reopened
        .apply(SceneCommand::ActivateScene {
            scene_id: a.clone(),
        })
        .unwrap();
    assert_eq!(restored.active_scene_id, a);
    assert!(!scene(&restored, &a).frozen);
    assert!(scene(&restored, &b).frozen);
    assert_eq!(scene(&restored, &a).items, scene(&completed, &a).items);
    assert_eq!(scene(&restored, &a).refs, scene(&completed, &a).refs);
    assert_eq!(scene(&restored, &a).draft, "保留录屏后的问题");
    drop(reopened);
    assert_eq!(Store::open(&file.path).unwrap().snapshot(), restored);
}

#[test]
fn changed_background_or_region_rejects_late_recordings_without_a_commit() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let (scene_id, background, region) = prepare(&mut store);
    let mut moved = region.clone();
    moved.x += 1.;
    store
        .apply(SceneCommand::UpdateRegion {
            scene_id: scene_id.clone(),
            region: moved.clone(),
        })
        .unwrap();
    let before = persisted(&file.path);
    assert!(store
        .finish_recording(&scene_id, &background.id, &region, asset(AssetKind::Video))
        .is_err());
    assert_eq!(store.snapshot(), before.1);
    assert_eq!(persisted(&file.path), before);
    store
        .apply(SceneCommand::RemoveRegion {
            scene_id: scene_id.clone(),
            region_id: region.id.clone(),
        })
        .unwrap();
    let before = persisted(&file.path);
    assert!(store
        .finish_recording(&scene_id, &background.id, &moved, asset(AssetKind::Video))
        .is_err());
    assert_eq!(store.snapshot(), before.1);
    assert_eq!(persisted(&file.path), before);

    let replacement = asset(AssetKind::Image);
    store.set_background(&scene_id, replacement).unwrap();
    // Even reusing the same region ID/geometry cannot bypass background identity.
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.clone(),
            region: region.clone(),
        })
        .unwrap();
    let before = persisted(&file.path);
    assert!(store
        .finish_recording(&scene_id, &background.id, &region, asset(AssetKind::Video))
        .is_err());
    assert_eq!(store.snapshot(), before.1);
    assert_eq!(persisted(&file.path), before);
}

#[test]
fn failed_sql_commit_leaves_recording_region_refs_and_memory_unchanged() {
    let file = TestDb::new();
    let mut store = Store::open(&file.path).unwrap();
    let (scene_id, background, region) = prepare(&mut store);
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: region.id.clone(),
            }],
        })
        .unwrap();
    let before = persisted(&file.path);
    let connection = Connection::open(&file.path).unwrap();
    connection.execute_batch("CREATE TRIGGER recording_write_failure BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT, 'recording fixture'); END;").unwrap();
    let video = asset(AssetKind::Video);
    assert!(matches!(
        store.finish_recording(&scene_id, &background.id, &region, video.clone()),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.snapshot(), before.1);
    assert_eq!(persisted(&file.path), before);
    connection
        .execute_batch("DROP TRIGGER recording_write_failure;")
        .unwrap();
    let after = store
        .finish_recording(&scene_id, &background.id, &region, video)
        .unwrap();
    assert_eq!(persisted(&file.path), (before.0 + 1, after));
}
