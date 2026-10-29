// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::{path::PathBuf, sync::OnceLock};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-json-padding-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> PathBuf {
        self.0.join("spaces.db")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["spaces.db", "spaces.db-wal", "spaces.db-shm"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn large_history() -> Snapshot {
    static TEMPLATE: OnceLock<Snapshot> = OnceLock::new();
    TEMPLATE
        .get_or_init(|| {
            let mut state = initial_snapshot();
            let agent = state.agents[0].id.clone();
            state.scenes = (0..193)
                .map(|_| {
                    let mut target = new_scene(&agent, state.default_connection_id.clone());
                    target.frozen = true;
                    target
                })
                .collect();
            let active = state.scenes.last_mut().unwrap();
            active.frozen = false;
            state.active_scene_id = active.id.clone();
            for ordinal in 0..134 {
                let target = &mut state.scenes[ordinal];
                target.background = Some(Asset {
                    id: id(),
                    name: "synthetic.png".into(),
                    kind: AssetKind::Image,
                    path: "synthetic.png".into(),
                    width: Some(1920),
                    height: Some(1080),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                    capture_coordinates: None,
                });
                let mut region = Region {
                    id: id(),
                    x: 20.,
                    y: 20.,
                    width: 600.,
                    height: 500.,
                    drawing_revision: 1,
                    ..Default::default()
                };
                for stroke in 0..12 {
                    region.drawings.push(Drawing {
                        id: id(),
                        kind: DrawingKind::Pen,
                        color: "#2488CC".into(),
                        stroke_width: 3.,
                        points: (0..150)
                            .map(|point| DrawingPoint {
                                x: (40 + point) as f64,
                                y: (40 + (point * 17 + stroke * 13) % 400) as f64,
                            })
                            .collect(),
                        text: None,
                        font_size: None,
                        text_style: None,
                        origin: None,
                        rich: None,
                     text_runs: None, });
                }
                region.drawing_history.undo.push(
                    DrawingEdit {
                        index: 0,
                        before: None,
                        after: Some(region.drawings[0].clone()),
                    }
                    .into(),
                );
                target.refs.push(Reference {
                    kind: ReferenceKind::Region,
                    id: region.id.clone(),
                });
                target.regions.push(region);
            }
            validate_snapshot(&state).unwrap();
            let bytes = serde_json::to_string(&state).unwrap().len();
            assert!((4 * 1024 * 1024..8 * 1024 * 1024).contains(&bytes));
            state
        })
        .clone()
}

fn seeded() -> (Fixture, Store) {
    let fixture = Fixture::new();
    let mut store = Store::open(fixture.path()).unwrap();
    assert!(store.pause_checkpoint_maintenance().wait());
    store.persist(large_history()).unwrap();
    assert_eq!(
        store
            .db
            .pragma_query_value::<i64, _>(None, "synchronous", |r| r.get(0))
            .unwrap(),
        2
    );
    (fixture, store)
}

fn durable(store: &Store) -> (i64, String) {
    store
        .db
        .query_row(
            "SELECT revision,payload FROM app_state WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn draft(store: &mut Store, text: String) {
    let scene_id = store.state.active_scene_id.clone();
    store
        .apply(SceneCommand::SetDraft {
            scene_id,
            draft: text,
        })
        .unwrap();
}

fn reset_wal(store: &Store) {
    store
        .db
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
}

fn frames(store: &Store) -> i64 {
    store
        .db
        .query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| row.get(1))
        .unwrap()
}

#[test]
fn snapshot_padding_native_sqlite_uses_the_reviewed_wal_reset_fix() {
    let db = SqlConnection::open_in_memory().unwrap();
    let version: String = db
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .unwrap();
    let source: String = db
        .query_row("SELECT sqlite_source_id()", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "3.53.2");
    assert_eq!(
        source,
        "2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24"
    );
}

#[test]
fn snapshot_padding_preserves_small_bytes_unicode_json_and_checked_bounds() {
    let small = serde_json::to_string(&initial_snapshot()).unwrap();
    let mut actual = small.clone();
    pad_snapshot_payload(&mut actual);
    assert_eq!(actual, small);
    for size in [
        SNAPSHOT_PADDING_MIN_BYTES - 1,
        SNAPSHOT_PADDING_MIN_BYTES,
        SNAPSHOT_PADDING_MIN_BYTES + 1,
        SNAPSHOT_PADDING_MIN_BYTES + SNAPSHOT_PADDING_BUCKET_BYTES - 1,
    ] {
        let mut value =
            serde_json::to_string(&serde_json::json!({"text": "中文🙂", "data": "x".repeat(size)}))
                .unwrap();
        let original: serde_json::Value = serde_json::from_str(&value).unwrap();
        let before = value.len();
        pad_snapshot_payload(&mut value);
        assert!(value.len() - before < SNAPSHOT_PADDING_BUCKET_BYTES);
        assert_eq!(value.len() % SNAPSHOT_PADDING_BUCKET_BYTES, 0);
        assert!(value.as_bytes()[before..].iter().all(|byte| *byte == b' '));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&value).unwrap(),
            original
        );
    }
    assert_eq!(snapshot_padding_bytes(usize::MAX), 0);
    assert_eq!(snapshot_padding_bytes(isize::MAX as usize), 0);
    assert_eq!(snapshot_padding_bytes(SNAPSHOT_PADDING_MIN_BYTES - 1), 0);
    assert_eq!(snapshot_padding_bytes(SNAPSHOT_PADDING_MIN_BYTES), 0);
    assert_eq!(
        snapshot_padding_bytes(SNAPSHOT_PADDING_MIN_BYTES + 1),
        SNAPSHOT_PADDING_BUCKET_BYTES - 1
    );
}

#[test]
fn snapshot_padding_reduces_real_wal_pages_against_canonical_growth_without_weakening_full() {
    let (_fixture, mut store) = seeded();
    let canonical = serde_json::to_string(&store.state).unwrap();
    let alignment = (SNAPSHOT_PADDING_BUCKET_BYTES / 2 + SNAPSHOT_PADDING_BUCKET_BYTES
        - canonical.len() % SNAPSHOT_PADDING_BUCKET_BYTES)
        % SNAPSHOT_PADDING_BUCKET_BYTES;
    draft(&mut store, "x".repeat(alignment));
    // Emulate a genuine legacy canonical record; no domain state or revision
    // is changed by this test-only formatting replacement.
    store
        .db
        .execute(
            "UPDATE app_state SET payload=?1 WHERE id=1",
            [serde_json::to_string(&store.state).unwrap()],
        )
        .unwrap();
    reset_wal(&store);
    let mut control = store.state.clone();
    control.scenes.last_mut().unwrap().draft.push('c');
    validate_snapshot(&control).unwrap();
    let transaction = store
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(transaction.execute("UPDATE app_state SET payload=?1,revision=revision+1 WHERE id=1 AND revision=?2 AND schema_version=1",
        params![serde_json::to_string(&control).unwrap(), store.revision]).unwrap(), 1);
    transaction.commit().unwrap();
    let canonical_frames = frames(&store);
    assert!(
        canonical_frames >= 1000,
        "control must really rewrite the multi-MiB overflow record"
    );
    store.revision += 1;
    store.state = control;
    // First adoption can rewrite the full record; it is deliberately excluded
    // from the stable-bucket assertion, never hidden as a claimed fast sample.
    let adopted = format!("{}p", store.state.scenes.last().unwrap().draft);
    draft(&mut store, adopted);
    for index in 0..6 {
        reset_wal(&store);
        let revision = store.revision;
        draft(&mut store, format!("{}-{index}", "x".repeat(alignment)));
        let padded_frames = frames(&store);
        assert!(
            padded_frames > 0 && padded_frames <= 8,
            "same-bucket tail edit should dirty only a few pages: {padded_frames}"
        );
        assert!(canonical_frames >= padded_frames * 100);
        assert_eq!(store.revision, revision + 1);
        assert_eq!(
            serde_json::from_str::<Snapshot>(&durable(&store).1).unwrap(),
            store.state
        );
    }
    assert_eq!(
        store
            .db
            .pragma_query_value::<i64, _>(None, "synchronous", |r| r.get(0))
            .unwrap(),
        2
    );
}

#[test]
fn snapshot_padding_legacy_reopen_growth_shrink_and_bucket_crossing_preserve_every_scene() {
    let (fixture, store) = seeded();
    let expected = store.state.clone();
    let revision = store.revision;
    let canonical = serde_json::to_string(&expected).unwrap();
    store
        .db
        .execute("UPDATE app_state SET payload=?1 WHERE id=1", [&canonical])
        .unwrap();
    drop(store);
    let mut reopened = Store::open(fixture.path()).unwrap();
    assert!(reopened.pause_checkpoint_maintenance().wait());
    assert_eq!(reopened.state, expected);
    assert_eq!(durable(&reopened), (revision, canonical));
    for length in [1, 4096, 90_000, 12, 65_536, 3, 130_000, 0] {
        let revision = reopened.revision;
        draft(&mut reopened, "z".repeat(length));
        let (actual_revision, payload) = durable(&reopened);
        assert_eq!(actual_revision, revision + 1);
        assert_eq!(payload.len() % SNAPSHOT_PADDING_BUCKET_BYTES, 0);
        assert_eq!(
            serde_json::from_str::<Snapshot>(&payload).unwrap(),
            reopened.state
        );
        assert_eq!(&reopened.state.scenes[..192], &expected.scenes[..192]);
    }
    let final_state = reopened.state.clone();
    drop(reopened);
    assert_eq!(Store::open(fixture.path()).unwrap().state, final_state);
}

#[test]
fn snapshot_padding_export_and_failed_commit_keep_the_durable_ledger_and_revision_atomic() {
    let (fixture, mut store) = seeded();
    draft(&mut store, "synthetic ledger input".into());
    let scene = store.state.active_scene_id.clone();
    let run = store.begin_run(&scene).unwrap();
    let old = ExportView::open(fixture.path()).unwrap();
    let old_state = old.snapshot().clone();
    let old_payload: String = old
        .db
        .query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    let before = durable(&store);
    let state = store.state.clone();
    store.db.execute_batch("PRAGMA foreign_keys=ON;
        CREATE TABLE padding_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE padding_child(parent_id INTEGER NOT NULL REFERENCES padding_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER reject_padded_commit AFTER UPDATE ON app_state BEGIN INSERT INTO padding_child(parent_id) VALUES(123); END;").unwrap();
    assert!(matches!(
        store.cancel_run(&scene),
        Err(CoreError::Database(_))
    ));
    assert_eq!(store.state, state);
    assert_eq!(durable(&store), before);
    assert_eq!(
        crate::journal::required(&store.db, &run.run_id)
            .unwrap()
            .status,
        RecordedRunStatus::Running
    );
    assert_eq!(
        store
            .db
            .query_row::<u32, _, _>("SELECT count(*) FROM padding_child", [], |r| r.get(0))
            .unwrap(),
        0
    );
    store
        .db
        .execute_batch("DROP TRIGGER reject_padded_commit")
        .unwrap();
    store.cancel_run(&scene).unwrap();
    assert_eq!(old.snapshot(), &old_state);
    assert_eq!(
        old.db
            .query_row::<String, _, _>("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
            .unwrap(),
        old_payload
    );
    assert_eq!(
        crate::journal::required(&store.db, &run.run_id)
            .unwrap()
            .status,
        RecordedRunStatus::Canceled
    );
    drop(old);
    let expected = store.state.clone();
    drop(store);
    assert_eq!(Store::open(fixture.path()).unwrap().state, expected);
}

#[test]
fn snapshot_padding_process_exit_recovers_the_committed_large_wal() {
    const CHILD_PATH: &str = "MEWU_PADDED_CRASH_TEST_PATH";
    const TEXT: &str = "synthetic durable padded tail";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let mut store = Store::open(PathBuf::from(path)).unwrap();
        draft(&mut store, TEXT.into());
        assert_eq!(durable(&store).1.len() % SNAPSHOT_PADDING_BUCKET_BYTES, 0);
        std::process::exit(0);
    }
    let (fixture, store) = seeded();
    let original = store.state.clone();
    let expected_revision = store.revision + 1;
    let old = ExportView::open(fixture.path()).unwrap();
    drop(store);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "store::payload_tests::snapshot_padding_process_exit_recovers_the_committed_large_wal",
            "--test-threads=1",
        ])
        .env(CHILD_PATH, fixture.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(old.snapshot(), &original);
    let reopened = Store::open(fixture.path()).unwrap();
    assert_eq!(reopened.revision, expected_revision);
    assert_eq!(reopened.active_scene().unwrap().draft, TEXT);
    assert_eq!(&reopened.state.scenes[..192], &original.scenes[..192]);
    assert_eq!(
        reopened
            .db
            .query_row::<String, _, _>("PRAGMA quick_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
    drop(old);
}
