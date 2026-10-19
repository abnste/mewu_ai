// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::{path::PathBuf, sync::{Arc,Mutex},thread,time::Instant};

struct Fixture(PathBuf);
impl Fixture {
 fn new()->Self{let root=std::env::temp_dir().join(format!("mewu-checkpoint-store-{}",Uuid::new_v4()));std::fs::create_dir(&root).unwrap();Self(root)}
 fn path(&self)->PathBuf{self.0.join("spaces.db")}
}
impl Drop for Fixture{
 fn drop(&mut self){for name in ["spaces.db","spaces.db-wal","spaces.db-shm"]{let _=std::fs::remove_file(self.0.join(name));}let _=std::fs::remove_dir(&self.0);}
}
fn settings(store:&Store)->(String,i64,i64){(
 store.db.pragma_query_value(None,"journal_mode",|r|r.get(0)).unwrap(),
 store.db.pragma_query_value(None,"synchronous",|r|r.get(0)).unwrap(),
 store.db.pragma_query_value(None,"wal_autocheckpoint",|r|r.get(0)).unwrap())}
fn durable(store:&Store)->(String,i64){store.db.query_row("SELECT payload,revision FROM app_state WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()}
fn draft(store:&mut Store,text:&str)->Result<Snapshot>{let scene_id=store.state.active_scene_id.clone();store.apply(SceneCommand::SetDraft{scene_id,draft:text.into()})}
fn wait_for_backfill(store:&Store){
 let limit=Instant::now()+Duration::from_secs(35);
 loop{let (_busy,frames,copied):(i64,i64,i64)=store.db.query_row("PRAGMA wal_checkpoint(NOOP)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();if frames==copied{return;}assert!(Instant::now()<limit,"background checkpoint did not finish");thread::sleep(Duration::from_millis(5));}
}

#[test]
fn checkpoint_store_memory_missing_file_and_paused_fallback_keep_original_policy(){
 let mut memory=Store::open_in_memory().unwrap();assert!(memory.checkpoint_maintenance.is_none());assert!(!memory.resume_checkpoint_maintenance());assert!(memory.pause_checkpoint_maintenance().wait());
 let fixture=Fixture::new();let missing=fixture.0.join("missing.db");assert!(Store::open_checkpoint_connection(&missing).is_err());assert!(!missing.exists());
 let mut store=Store::open(fixture.path()).unwrap();assert_eq!(settings(&store),("wal".into(),2,8192));assert!(store.checkpoint_maintenance.as_ref().unwrap().is_healthy());
 assert!(store.pause_checkpoint_maintenance().wait());assert_eq!(settings(&store),("wal".into(),2,1000));
 assert!(Store::open_checkpoint_connection(&missing).is_err());assert_eq!(settings(&store),("wal".into(),2,1000));assert!(!missing.exists());
 assert!(store.resume_checkpoint_maintenance());assert_eq!(settings(&store),("wal".into(),2,8192));
}

#[test]
fn checkpoint_store_real_commit_rollback_compaction_and_drop_release_handles(){
 let fixture=Fixture::new();let mut store=Store::open(fixture.path()).unwrap();
 draft(&mut store,"retained synthetic draft").unwrap();wait_for_backfill(&store);
 let before=durable(&store);let snapshot=store.snapshot();
 store.db.execute_batch("CREATE TRIGGER checkpoint_reject_update BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
 assert!(draft(&mut store,"must not commit").is_err());assert_eq!(store.snapshot(),snapshot);assert_eq!(durable(&store),before);
 store.db.execute_batch("DROP TRIGGER checkpoint_reject_update;").unwrap();
 store.compact_history_storage().unwrap();assert_eq!(durable(&store),before);assert_eq!(settings(&store),("wal".into(),2,8192));
 let retained=store.snapshot();drop(store);
 let renamed=fixture.0.join("released.db");std::fs::rename(fixture.path(),&renamed).unwrap();std::fs::rename(renamed,fixture.path()).unwrap();
 let reopened=Store::open(fixture.path()).unwrap();assert_eq!(reopened.snapshot(),retained);assert_eq!(durable(&reopened),before);
}

#[test]
fn checkpoint_store_long_export_never_rejects_drawing_or_mixes_revisions(){
 let fixture=Fixture::new();let mut store=Store::open(fixture.path()).unwrap();draft(&mut store,"old synthetic").unwrap();wait_for_backfill(&store);
 let view=ExportView::open(fixture.path()).unwrap();let old=view.snapshot().clone();
 let old_payload:String=view.db.query_row("SELECT payload FROM app_state WHERE id=1",[],|r|r.get(0)).unwrap();
 for index in 0..16{draft(&mut store,&format!("new synthetic {index}")).unwrap();}
 assert!(store.checkpoint_maintenance.as_ref().unwrap().is_healthy());assert_eq!(view.snapshot(),&old);
 assert_eq!(view.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|r|r.get(0)).unwrap(),old_payload);
 drop(view);store.notify_checkpoint_maintenance();wait_for_backfill(&store);
 let expected=store.snapshot();drop(store);assert_eq!(Store::open(fixture.path()).unwrap().snapshot(),expected);
}

#[test]
fn checkpoint_store_exit_ticket_waits_without_borrowing_engine_and_abort_can_resume(){
 let fixture=Fixture::new();let store=Arc::new(Mutex::new(Store::open(fixture.path()).unwrap()));
 let ticket={let mut lock=store.lock().unwrap();draft(&mut lock,"before exit").unwrap();lock.pause_checkpoint_maintenance()};
 assert!(thread::spawn(move||ticket.wait()).join().unwrap());
 let mut lock=store.try_lock().unwrap();assert_eq!(settings(&lock),("wal".into(),2,1000));
 assert!(lock.resume_checkpoint_maintenance());draft(&mut lock,"after aborted exit").unwrap();wait_for_backfill(&lock);assert_eq!(settings(&lock),("wal".into(),2,8192));
}

#[test]
fn checkpoint_store_failed_worker_keeps_FULL_restores_default_and_can_restart(){
 let fixture=Fixture::new();let mut store=Store::open(fixture.path()).unwrap();assert!(store.pause_checkpoint_maintenance().wait());
 // A real non-WAL connection makes PASSIVE fail without a domain mutation.
 let connection=SqlConnection::open_in_memory().unwrap();
 store.checkpoint_maintenance=Some(crate::checkpoint_worker::CheckpointWorker::start(connection).unwrap());
 store.db.pragma_update(None,"wal_autocheckpoint",8192i64).unwrap();
 store.notify_checkpoint_maintenance();
 let until=Instant::now()+Duration::from_secs(3);
 while store.checkpoint_maintenance.as_ref().unwrap().is_healthy(){assert!(Instant::now()<until);thread::sleep(Duration::from_millis(5));}
 assert!(!store.resume_checkpoint_maintenance());
 draft(&mut store,"commit survives failed maintenance").unwrap();assert_eq!(settings(&store),("wal".into(),2,1000));
 let before=durable(&store);assert!(store.pause_checkpoint_maintenance().wait());
 assert!(store.resume_checkpoint_maintenance());wait_for_backfill(&store);assert_eq!(durable(&store),before);assert_eq!(settings(&store),("wal".into(),2,8192));
}

#[test]
fn checkpoint_store_process_exit_recovers_the_committed_wal() {
    const CHILD_PATH: &str = "MEWU_CHECKPOINT_CRASH_TEST_PATH";
    const TEXT: &str = "durable before process exit";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let mut store = Store::open(PathBuf::from(path)).unwrap();
        draft(&mut store, TEXT).unwrap();
        // Skip Store Drop, worker drain and SQLite close, as a process crash
        // would. Successful FULL COMMIT must suffice regardless of checkpoint.
        std::process::exit(0);
    }
    let fixture = Fixture::new();
    let store = Store::open(fixture.path()).unwrap();
    let original = store.snapshot();
    let expected_revision = store.revision + 1;
    // Keep an older reader alive, so maintenance cannot copy the child's new
    // app_state page into the main database before that child exits.
    let old_view = ExportView::open(fixture.path()).unwrap();
    drop(store);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "store::checkpoint_store_tests::checkpoint_store_process_exit_recovers_the_committed_wal", "--test-threads=1"])
        .env(CHILD_PATH, fixture.path())
        .status().unwrap();
    assert!(status.success());
    assert_eq!(old_view.snapshot(), &original);
    let reopened = Store::open(fixture.path()).unwrap();
    assert_eq!(reopened.revision, expected_revision);
    assert_eq!(reopened.scene(&original.active_scene_id).unwrap().draft, TEXT);
    assert_eq!(reopened.snapshot().active_scene_id, original.active_scene_id);
    assert_eq!(reopened.snapshot().agents, original.agents);
    assert_eq!(reopened.snapshot().scenes.len(), original.scenes.len());
    assert_eq!(reopened.db.query_row::<String, _, _>("PRAGMA quick_check", [], |row| row.get(0)).unwrap(), "ok");
    drop(old_view);
}
