// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::{types::Value, Connection};
use std::{collections::BTreeMap, path::PathBuf};

struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mewu-coordinate-{}.db", uuid::Uuid::new_v4())))
    }
    fn conn(&self) -> Connection {
        Connection::open(&self.0).unwrap()
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn coordinates() -> CaptureCoordinates {
    CaptureCoordinates {
        space: CaptureCoordinateSpace::QuartzPoints,
        origin_x: -1280.25,
        origin_y: 20.5,
        width: 1280.,
        height: 720.,
    }
}

#[test]
fn portal_logical_coordinates_preserve_their_own_units_and_round_trip() {
    let c=CaptureCoordinates{space:CaptureCoordinateSpace::PortalLogical,..coordinates()};
    assert_eq!(c.point(2560,1440,100,200),Some((-1230.25,120.5)));
    assert_eq!(serde_json::from_str::<CaptureCoordinates>(&serde_json::to_string(&c).unwrap()).unwrap(),c);
    assert!(!CaptureCoordinates{width:f64::NAN,..c}.valid_for(2560,1440));
}
fn asset(capture_coordinates: Option<CaptureCoordinates>) -> Asset {
    Asset {
        id: uuid::Uuid::new_v4().to_string(),
        name: "capture.png".into(),
        kind: AssetKind::Image,
        path: "assets/capture.png".into(),
        width: Some(2560),
        height: Some(1440),
        origin_x: Some(-1280),
        origin_y: Some(20),
        scale_factor: Some(2.),
        capture_coordinates,
    }
}
fn payload(db: &Connection) -> (String, i64) {
    db.query_row(
        "SELECT payload,revision FROM app_state WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}
fn rows(db: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    let names: Vec<String> = db
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name!='app_state' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let n = statement.column_count();
            let data = statement
                .query_map([], |r| (0..n).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<Result<Vec<Vec<Value>>, _>>()
                .unwrap();
            (name, data)
        })
        .collect()
}

#[test]
fn independent_axes_preserve_fractional_negative_origins_and_windows_integer_behavior() {
    let c = coordinates();
    assert_eq!(c.point(2560, 1440, 100, 200), Some((-1230.25, 120.5)));
    assert_eq!(c.point(2560, 1080, 100, 270), Some((-1230.25, 200.5)));
    let win = CaptureCoordinates {
        space: CaptureCoordinateSpace::PhysicalPixels,
        origin_x: -1920.,
        origin_y: -1080.,
        width: 2560.,
        height: 1440.,
    };
    for (x, y) in [(0, 0), (100, 200), (2559, 1439)] {
        assert_eq!(
            win.point(2560, 1440, x, y),
            Some((-1920. + f64::from(x), -1080. + f64::from(y)))
        );
    }
    for origin in [i32::MIN, i32::MAX] {
        let extreme = CaptureCoordinates {
            origin_x: f64::from(origin),
            ..win
        };
        assert_eq!(
            extreme.point(2560, 1440, 2559, 0),
            Some((f64::from(origin) + 2559., -1080.))
        );
    }
    assert!(c.point(2560, 1440, 2560, 0).is_none());
    for changed in [
        CaptureCoordinates {
            origin_x: f64::NAN,
            ..c
        },
        CaptureCoordinates { width: 0., ..c },
        CaptureCoordinates {
            height: f64::INFINITY,
            ..c
        },
        CaptureCoordinates {
            space: CaptureCoordinateSpace::PhysicalPixels,
            ..c
        },
    ] {
        assert!(!changed.valid_for(2560, 1440));
    }
    let mut malformed = serde_json::to_value(c).unwrap();
    malformed["space"] = "logicalPoints".into();
    assert!(serde_json::from_value::<CaptureCoordinates>(malformed).is_err());
}

#[test]
fn db9_migration_preserves_raw_document_tables_fts_blobs_and_does_not_repeat() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let scene = store.snapshot().active_scene_id;
    store.set_background(&scene, asset(None)).unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene,
            draft: "旧库保留中文内容".into(),
        })
        .unwrap();
    let state = store.snapshot();
    drop(store);
    let db = file.conn();
    // Independent nonempty text/FTS/BLOB values prove preservation of more than
    // the document, while all real application tables are compared as well.
    db.execute_batch("CREATE TABLE fixture_blob(id INTEGER PRIMARY KEY, body BLOB); INSERT INTO fixture_blob VALUES(1,X'00FF1234'); CREATE VIRTUAL TABLE fixture_fts USING fts5(body); INSERT INTO fixture_fts VALUES('旧资料 searchable'); PRAGMA user_version=9;").unwrap();
    let raw = format!("\n {} \n", serde_json::to_string_pretty(&state).unwrap());
    db.execute("UPDATE app_state SET payload=?1 WHERE id=1", [&raw])
        .unwrap();
    let before = payload(&db);
    let all_rows = rows(&db);
    let migrated = Store::open(&file.0).unwrap();
    assert_eq!(payload(&db), (raw.clone(), before.1 + 1));
    assert_eq!(rows(&db), all_rows);
    assert_eq!(migrated.snapshot(), state);
    assert!(migrated.snapshot().scenes[0]
        .background
        .as_ref()
        .unwrap()
        .capture_coordinates
        .is_none());
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        mewu_core::STORAGE_SCHEMA_VERSION
    );
    assert_eq!(
        db.query_row(
            "SELECT body FROM fixture_fts WHERE fixture_fts MATCH 'searchable'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "旧资料 searchable"
    );
    drop(migrated);
    drop(Store::open(&file.0).unwrap());
    assert_eq!(payload(&db), (raw, before.1 + 1));
    assert_eq!(rows(&db), all_rows);
}

#[test]
fn migration_failure_is_atomic_and_future_schema_does_not_touch_data() {
    let file = Db::new();
    drop(Store::open(&file.0).unwrap());
    let db = file.conn();
    db.execute_batch("PRAGMA user_version=9;CREATE TRIGGER reject_coordinate_migration BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    let before = payload(&db);
    let all_rows = rows(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(payload(&db), before);
    assert_eq!(rows(&db), all_rows);
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    db.execute_batch("DROP TRIGGER reject_coordinate_migration; PRAGMA user_version=99;")
        .unwrap();
    assert!(matches!(
        Store::open(&file.0),
        Err(CoreError::UnsupportedSchema(99))
    ));
    assert_eq!(payload(&db), before);
    assert_eq!(rows(&db), all_rows);
}

#[test]
fn imports_and_generic_moves_cannot_forge_capture_geometry() {
    let mut store = Store::open_in_memory().unwrap();
    let scene = store.snapshot().active_scene_id;
    assert!(store
        .import_assets(&scene, vec![asset(Some(coordinates()))])
        .is_err());
    store.add_asset(&scene, asset(None)).unwrap();
    let mut item = store.snapshot().scenes[0].items[0].clone();
    item.asset.capture_coordinates = Some(coordinates());
    assert!(store
        .apply(SceneCommand::UpdateItem {
            scene_id: scene,
            item
        })
        .is_err());
    let serialized = serde_json::to_value(asset(None)).unwrap();
    assert!(serialized.get("captureCoordinates").is_none());
}
