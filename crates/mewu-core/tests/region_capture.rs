// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mewu-region-capture-{}", Uuid::new_v4()));
        assert!(!root.to_string_lossy().to_lowercase().starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        Self(root.join("spaces.db"))
    }
    fn durable(&self) -> (String, i64) {
        Connection::open(&self.0).unwrap().query_row(
            "SELECT payload,revision FROM app_state WHERE id=1", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap()
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
fn region() -> Region {
    Region { id:Uuid::new_v4().to_string(),x:20.,y:30.,width:120.,height:80.,..Default::default() }
}
fn reference(region: &Region) -> Reference {
    Reference { kind:ReferenceKind::Region,id:region.id.clone() }
}
fn setup(store: &mut Store) -> String {
    let scene = store.active_scene().unwrap().id.clone();
    store.set_background(&scene, Asset {
        id:Uuid::new_v4().to_string(),name:"synthetic.png".into(),path:"synthetic.png".into(),
        kind:AssetKind::Image,width:Some(1920),height:Some(1080),origin_x:None,origin_y:None,
        scale_factor:None,capture_coordinates:None,
    }).unwrap();
    scene
}

#[test]
fn capture_region_and_current_reference_draft_commit_once_and_reopen_together() {
    let db=TestDb::new();
    let mut store=Store::open(&db.0).unwrap();
    let scene=setup(&mut store);
    let a=region();
    let b=region();
    for r in [&a,&b] {
        store.apply(SceneCommand::AddRegion{scene_id:scene.clone(),region:r.clone()}).unwrap();
    }
    store.apply(SceneCommand::SetRefs{scene_id:scene.clone(),refs:vec![reference(&a)]}).unwrap();
    let before=db.durable();
    let c=region();
    // The UI draft removed a and added b before the new capture was saved.
    let refs=vec![reference(&b),reference(&c)];
    let snapshot=store.add_region_with_refs(&scene,c.clone(),refs.clone()).unwrap();
    assert_eq!(db.durable().1,before.1+1);
    assert_eq!(store.scene(&scene).unwrap().regions,vec![a,b,c]);
    assert_eq!(store.scene(&scene).unwrap().refs,refs);
    drop(store);
    let reopened=Store::open(&db.0).unwrap();
    assert_eq!(reopened.snapshot(),snapshot);
}

#[test]
fn capture_invalid_refs_or_region_never_commit_a_partial_selection() {
    let db=TestDb::new();
    let mut store=Store::open(&db.0).unwrap();
    let scene=setup(&mut store);
    let before=store.snapshot();
    let durable=db.durable();
    let r=region();
    let invalid_refs=vec![
        vec![],
        vec![reference(&r),reference(&r)],
        vec![reference(&r),Reference{kind:ReferenceKind::Item,id:Uuid::new_v4().to_string()}],
    ];
    for refs in invalid_refs {
        assert!(store.add_region_with_refs(&scene,r.clone(),refs).is_err());
        assert_eq!(store.snapshot(),before);
        assert_eq!(db.durable(),durable);
    }
    for bad in [Region{width:5000.,..r.clone()},Region{drawing_revision:1,..r.clone()}] {
        assert!(store.add_region_with_refs(&scene,bad.clone(),vec![reference(&bad)]).is_err());
        assert_eq!(store.snapshot(),before);
        assert_eq!(db.durable(),durable);
    }
}

#[test]
fn capture_sql_failure_rolls_back_both_region_and_references() {
    let db=TestDb::new();
    let mut store=Store::open(&db.0).unwrap();
    let scene=setup(&mut store);
    let before=store.snapshot();
    let durable=db.durable();
    let connection=Connection::open(&db.0).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_capture BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'forced capture failure'); END;").unwrap();
    let r=region();
    assert!(store.add_region_with_refs(&scene,r.clone(),vec![reference(&r)]).is_err());
    assert_eq!(store.snapshot(),before);
    assert_eq!(db.durable(),durable);
    connection.execute_batch("DROP TRIGGER reject_capture").unwrap();
    store.add_region_with_refs(&scene,r.clone(),vec![reference(&r)]).unwrap();
    assert_eq!(db.durable().1,durable.1+1);
}

#[test]
fn capture_rejects_duplicate_region_inactive_scene_and_stale_writer() {
    let db=TestDb::new();
    let mut store=Store::open(&db.0).unwrap();
    let scene=setup(&mut store);
    let mut stale=Store::open(&db.0).unwrap();
    let r=region();
    store.add_region_with_refs(&scene,r.clone(),vec![reference(&r)]).unwrap();
    let before=store.snapshot();
    let durable=db.durable();
    assert!(store.add_region_with_refs(&scene,r.clone(),vec![reference(&r)]).is_err());
    assert_eq!(store.snapshot(),before);
    let late=region();
    assert!(matches!(stale.add_region_with_refs(&scene,late.clone(),vec![reference(&late)]),Err(CoreError::ConcurrentModification)));
    assert_eq!(db.durable(),durable);
    let frozen=store.apply(SceneCommand::FreezeScene{scene_id:scene.clone()}).unwrap();
    let inactive=region();
    assert!(store.add_region_with_refs(&scene,inactive.clone(),vec![reference(&inactive)]).is_err());
    assert_eq!(store.snapshot(),frozen);
}
