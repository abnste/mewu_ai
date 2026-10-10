// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

const SECOND: u64 = 10_000_000;
struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-video-edit-{}", Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn saved(&self) -> (String, i64) {
        Connection::open(&self.0)
            .unwrap()
            .query_row(
                "SELECT payload,revision FROM app_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }
    fn fail_updates(&self, fail: bool) {
        Connection::open(&self.0).unwrap().execute_batch(if fail {
            "CREATE TRIGGER fail_video BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic'); END;"
        } else { "DROP TRIGGER fail_video;" }).unwrap();
    }
    fn replace_payload(&self, value: serde_json::Value) {
        Connection::open(&self.0)
            .unwrap()
            .execute(
                "UPDATE app_state SET payload=?1 WHERE id=1",
                [serde_json::to_string(&value).unwrap()],
            )
            .unwrap();
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        for suffix in ["spaces.db", "spaces.db-wal", "spaces.db-shm"] {
            let _ = std::fs::remove_file(self.0.with_file_name(suffix));
        }
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}
fn setup(store: &mut Store, duration: u64) -> (VideoTarget, VerifiedVideoSource) {
    let scene_id = store.snapshot().active_scene_id;
    let asset = Asset {
        id: Uuid::new_v4().to_string(),
        name: "synthetic.mp4".into(),
        kind: AssetKind::Video,
        path: "synthetic-owned-video.mp4".into(),
        width: Some(320),
        height: Some(180),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    let next = store.add_asset(&scene_id, asset.clone()).unwrap();
    let item = next
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .unwrap()
        .items
        .last()
        .unwrap();
    (
        VideoTarget {
            scene_id,
            item_id: item.id.clone(),
            source_id: asset.id.clone(),
            expected_revision: 0,
        },
        VerifiedVideoSource {
            asset,
            duration_ticks: duration,
            width: 320,
            height: 180,
        },
    )
}
fn view(store: &Store, target: &VideoTarget) -> VideoSourceView {
    store
        .video_source(&target.scene_id, &target.item_id)
        .unwrap()
}
fn current(store: &Store, target: &VideoTarget) -> VideoTarget {
    VideoTarget {
        expected_revision: view(store, target).edit.map_or(0, |e| e.revision),
        ..target.clone()
    }
}
fn range(start: u64, end: u64) -> Option<VideoRange> {
    Some(VideoRange {
        start_ticks: start,
        end_ticks: end,
    })
}
fn set(
    store: &mut Store,
    target: &VideoTarget,
    proof: &VerifiedVideoSource,
    to: Option<VideoRange>,
) {
    let from = view(store, target).edit.and_then(|e| e.range);
    store
        .set_video_range(&current(store, target), proof, from, to)
        .unwrap();
}
fn replay(store: &mut Store, target: &VideoTarget, redo: bool) {
    let edit = view(store, target).edit.unwrap();
    let head = if redo {
        edit.redo.last()
    } else {
        edit.undo.last()
    }
    .unwrap();
    if redo {
        store
            .redo_video_range(&current(store, target), &head.id, edit.range)
            .unwrap();
    } else {
        store
            .undo_video_range(&current(store, target), &head.id, edit.range)
            .unwrap();
    }
}
fn item(store: &Store, target: &VideoTarget) -> SpaceItem {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == target.scene_id)
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.id == target.item_id)
        .unwrap()
}

#[test]
fn source_inspection_and_full_noop_do_not_write_metadata_but_actual_edit_does() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    // Actual synthetic AAC-container duration, deliberately not rounded to 6s.
    let (target, proof) = setup(&mut store, 60_139_455);
    let before = db.saved();
    db.fail_updates(true);
    assert!(view(&store, &target).edit.is_none());
    store.set_video_range(&target, &proof, None, None).unwrap();
    store
        .set_video_range(&target, &proof, None, range(0, proof.duration_ticks))
        .unwrap();
    assert_eq!(db.saved(), before);
    assert!(view(&store, &target).edit.is_none());
    db.fail_updates(false);
    set(&mut store, &target, &proof, range(SECOND, 4 * SECOND));
    let edited = view(&store, &target);
    assert_eq!(edited.asset, proof.asset);
    let edit = edited.edit.unwrap();
    assert_eq!(edit.source_duration_ticks, 60_139_455);
    assert_eq!(edit.revision, 1);
    assert_eq!(edit.undo.len(), 1);
    assert_eq!(db.saved().1, before.1 + 1);
}

#[test]
fn short_sources_and_invalid_ranges_never_create_partial_or_clamped_edits() {
    let mut store = Store::open_in_memory().unwrap();
    let (short, proof) = setup(&mut store, 500_000); // 50ms source
    let before = store.snapshot();
    store.set_video_range(&short, &proof, None, None).unwrap();
    for to in [range(1, 500_000), range(0, 499_999), range(0, 500_001)] {
        assert!(store.set_video_range(&short, &proof, None, to).is_err());
    }
    assert_eq!(store.snapshot(), before);
    let (target, proof) = setup(&mut store, 6 * SECOND);
    let before = store.snapshot();
    for to in [
        range(0, 999_999),
        range(1, 1),
        range(2, 1),
        range(0, 6 * SECOND + 1),
    ] {
        assert!(store.set_video_range(&target, &proof, None, to).is_err());
        assert_eq!(store.snapshot(), before);
    }
    let mut excessive = proof.clone();
    excessive.duration_ticks = 30 * 60 * SECOND + 1;
    assert!(store
        .set_video_range(&target, &excessive, None, range(0, SECOND))
        .is_err());
    set(&mut store, &target, &proof, range(0, 1_000_000)); // exactly100ms
    assert_eq!(
        view(&store, &target).edit.unwrap().range,
        range(0, 1_000_000)
    );
}

#[test]
fn stale_geometry_preserves_host_edit_and_aba_cannot_reuse_old_revision() {
    let mut store = Store::open_in_memory().unwrap();
    let (target, proof) = setup(&mut store, 6 * SECOND);
    let mut stale = item(&store, &target);
    set(&mut store, &target, &proof, range(SECOND, 5 * SECOND));
    let edited = view(&store, &target).edit;
    stale.x = 0.2;
    stale.video_edit = None;
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: target.scene_id.clone(),
            item: stale,
        })
        .unwrap();
    assert_eq!(item(&store, &target).x, 0.2);
    assert_eq!(view(&store, &target).edit, edited);
    let before = store.snapshot();
    assert!(store
        .set_video_range(&target, &proof, None, range(0, SECOND))
        .is_err());
    assert_eq!(store.snapshot(), before);
    replay(&mut store, &target, false); // full again, but revision2
    assert_eq!(view(&store, &target).edit.as_ref().unwrap().range, None);
    assert!(store
        .set_video_range(&target, &proof, None, range(0, SECOND))
        .is_err());
    let mut forged = item(&store, &target);
    forged.video_edit.as_mut().unwrap().source_duration_ticks = 123;
    let committed = view(&store, &target);
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: target.scene_id.clone(),
            item: forged,
        })
        .unwrap();
    assert_eq!(view(&store, &target), committed);
}

#[test]
fn history_head_and_budget_survive_freeze_restart_without_replaying_geometry() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (target, proof) = setup(&mut store, 6 * SECOND);
    for index in 1..=60 {
        set(
            &mut store,
            &target,
            &proof,
            range(index * 10_000, 6 * SECOND),
        );
    }
    assert_eq!(view(&store, &target).edit.as_ref().unwrap().undo.len(), 50);
    let before = store.snapshot();
    let last = view(&store, &target).edit.unwrap();
    assert!(store
        .undo_video_range(
            &current(&store, &target),
            &Uuid::new_v4().to_string(),
            last.range
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    for _ in 0..5 {
        replay(&mut store, &target, false);
    }
    for _ in 0..2 {
        replay(&mut store, &target, true);
    }
    let stored = view(&store, &target);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(store
        .set_video_range(
            &current(&store, &target),
            &proof,
            stored.edit.as_ref().unwrap().range,
            None
        )
        .is_err());
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(view(&store, &target), stored);
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    let position = item(&store, &target);
    set(&mut store, &target, &proof, range(SECOND, 4 * SECOND));
    assert!(view(&store, &target).edit.unwrap().redo.is_empty());
    replay(&mut store, &target, false);
    let after = item(&store, &target);
    assert_eq!(
        (after.x, after.y, after.width, after.height, after.state),
        (
            position.x,
            position.y,
            position.width,
            position.height,
            position.state
        )
    );
    assert_eq!(after.asset, proof.asset);
}

#[test]
fn database_failures_preserve_range_history_and_noop_rejects_second_writer() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (target, proof) = setup(&mut store, 6 * SECOND);
    let before = store.snapshot();
    let raw = db.saved();
    db.fail_updates(true);
    assert!(store
        .set_video_range(&target, &proof, None, range(SECOND, 5 * SECOND))
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(db.saved(), raw);
    db.fail_updates(false);
    set(&mut store, &target, &proof, range(SECOND, 5 * SECOND));
    let before = store.snapshot();
    let raw = db.saved();
    let edited = view(&store, &target).edit.unwrap();
    db.fail_updates(true);
    assert!(store
        .undo_video_range(
            &current(&store, &target),
            &edited.undo.last().unwrap().id,
            edited.range
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(db.saved(), raw);
    db.fail_updates(false);
    let mut other = Store::open(&db.0).unwrap();
    other
        .apply(SceneCommand::SetDraft {
            scene_id: target.scene_id.clone(),
            draft: "second writer".into(),
        })
        .unwrap();
    assert!(matches!(
        store.set_video_range(
            &current(&store, &target),
            &proof,
            edited.range,
            edited.range
        ),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(store.snapshot(), before);
    drop(other);
    drop(store);
    assert_eq!(
        Store::open(&db.0)
            .unwrap()
            .video_source(&target.scene_id, &target.item_id)
            .unwrap()
            .edit
            .unwrap(),
        edited
    );
}

#[test]
fn shared_asset_has_independent_item_ranges_and_export_ignores_card_position() {
    let mut store = Store::open_in_memory().unwrap();
    let (first, proof) = setup(&mut store, 6 * SECOND);
    let snapshot = store
        .add_asset(&first.scene_id, proof.asset.clone())
        .unwrap();
    let second = VideoTarget {
        item_id: snapshot.scenes[0].items.last().unwrap().id.clone(),
        ..first.clone()
    };
    set(&mut store, &first, &proof, range(0, SECOND));
    let origin = VideoExportOrigin {
        scene_id: first.scene_id.clone(),
        item_id: first.item_id.clone(),
        source: view(&store, &first),
    };
    set(&mut store, &second, &proof, range(2 * SECOND, 5 * SECOND));
    let mut moved = item(&store, &first);
    moved.x = 0.1;
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: first.scene_id.clone(),
            item: moved,
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: first.scene_id.clone(),
            draft: "unrelated".into(),
        })
        .unwrap();
    assert!(store.video_export_matches(&origin));
    set(&mut store, &first, &proof, range(0, 2 * SECOND));
    assert!(!store.video_export_matches(&origin));
    assert_eq!(
        view(&store, &second).edit.unwrap().range,
        range(2 * SECOND, 5 * SECOND)
    );
    store
        .apply(SceneCommand::RemoveItem {
            scene_id: first.scene_id.clone(),
            item_id: first.item_id.clone(),
        })
        .unwrap();
    assert!(!store.video_export_matches(&origin));
    assert!(store
        .set_video_range(&first, &proof, None, range(0, SECOND))
        .is_err());
}

#[test]
fn late_or_changed_native_duration_proof_never_retargets_existing_edits() {
    let mut store = Store::open_in_memory().unwrap();
    let (target, proof) = setup(&mut store, 60_139_455);
    set(&mut store, &target, &proof, range(SECOND, 5 * SECOND));
    let before = store.snapshot();
    let edited = view(&store, &target).edit.unwrap();
    for changed in [
        VerifiedVideoSource {
            duration_ticks: 6 * SECOND,
            ..proof.clone()
        },
        VerifiedVideoSource {
            width: 640,
            ..proof.clone()
        },
        VerifiedVideoSource {
            asset: Asset {
                path: "other.mp4".into(),
                ..proof.asset.clone()
            },
            ..proof.clone()
        },
    ] {
        assert!(store
            .set_video_range(&current(&store, &target), &changed, edited.range, None)
            .is_err());
        assert_eq!(store.snapshot(), before);
    }
    store
        .apply(SceneCommand::CloseScene {
            scene_id: target.scene_id.clone(),
        })
        .unwrap();
    assert!(store
        .set_video_range(&current(&store, &target), &proof, edited.range, None)
        .is_err());
    assert_eq!(view(&store, &target).edit.unwrap(), edited);
}

#[test]
fn old_db4_payload_stays_exact_and_corrupt_history_is_rejected_without_repair() {
    let db = TestDb::new();
    let mut store = Store::open(&db.0).unwrap();
    let (target, proof) = setup(&mut store, 6 * SECOND);
    let saved = db.saved();
    let legacy: serde_json::Value = serde_json::from_str(&saved.0).unwrap();
    assert!(legacy["scenes"][0]["items"][0].get("videoEdit").is_none());
    drop(store);
    let mut store = Store::open(&db.0).unwrap();
    assert_eq!(db.saved(), saved);
    assert!(view(&store, &target).edit.is_none());
    set(&mut store, &target, &proof, range(SECOND, 5 * SECOND));
    set(&mut store, &target, &proof, range(2 * SECOND, 4 * SECOND));
    let healthy: serde_json::Value = serde_json::from_str(&db.saved().0).unwrap();
    drop(store);
    for kind in 0..5 {
        let mut corrupt = healthy.clone();
        let edit = &mut corrupt["scenes"][0]["items"][0]["videoEdit"];
        match kind {
            0 => edit["sourceDurationTicks"] = serde_json::json!(0),
            1 => edit["sourceId"] = serde_json::json!(Uuid::new_v4().to_string()),
            2 => edit["undo"][0]["to"] = serde_json::json!({"startTicks": 0, "endTicks": SECOND}),
            3 => edit["range"] = serde_json::json!({"startTicks": 0, "endTicks": 6 * SECOND}),
            _ => {
                edit["undo"][0].as_object_mut().unwrap().remove("from");
            }
        }
        db.replace_payload(corrupt);
        let bytes = db.saved();
        assert!(Store::open(&db.0).is_err());
        assert_eq!(db.saved(), bytes);
    }
}
