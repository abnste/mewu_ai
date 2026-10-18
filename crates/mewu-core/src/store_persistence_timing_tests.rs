// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::{PersistencePhase, PersistenceTiming};
use std::sync::Mutex;

static OBSERVATIONS: Mutex<Vec<PersistenceTiming>> = Mutex::new(Vec::new());
fn observe(timing: PersistenceTiming) { OBSERVATIONS.lock().unwrap().push(timing); }

#[test]
fn observation_reports_actual_commit_and_failures_without_changing_transaction_outcomes() {
    OBSERVATIONS.lock().unwrap().clear();
    let mut store=Store::open_in_memory().unwrap();
    let scene=store.active_scene().unwrap().id.clone();
    assert!(store.persistence_observer.is_none());
    store.apply(SceneCommand::SetDraft{scene_id:scene.clone(),draft:"before observing".into()}).unwrap();
    assert!(OBSERVATIONS.lock().unwrap().is_empty());
    store.set_persistence_observer(Some(observe));
    store.apply(SceneCommand::SetDraft{scene_id:scene.clone(),draft:"durably accepted".into()}).unwrap();
    let committed=store.snapshot();
    let durable=store.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|row|row.get(0)).unwrap();
    let revision=store.revision;
    {
        let observations=OBSERVATIONS.lock().unwrap();
        assert_eq!(observations.len(),1);
        assert!(observations[0].success);
        let phases:Vec<_>=observations[0].completed.iter().flatten().map(|mark|mark.phase).collect();
        assert_eq!(phases,vec![PersistencePhase::GeometryCleanup,PersistencePhase::ValidateCandidate,
            PersistencePhase::TransactionAndLedger,PersistencePhase::ValidateReconciled,
            PersistencePhase::ValidateLedgers,PersistencePhase::Serialize,PersistencePhase::SqlUpdate,
            PersistencePhase::DurableCommit,PersistencePhase::AdoptState,PersistencePhase::CloneResult]);
    }
    // A genuine deferred SQLite constraint fails at COMMIT, after UPDATE
    // succeeded. The observation must never report that attempt as durable.
    store.db.execute_batch("PRAGMA foreign_keys=ON;
        CREATE TABLE observer_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE observer_child(parent_id INTEGER NOT NULL REFERENCES observer_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER reject_observed_commit AFTER UPDATE ON app_state BEGIN INSERT INTO observer_child(parent_id) VALUES(123); END;").unwrap();
    assert!(matches!(store.apply(SceneCommand::SetDraft{scene_id:scene.clone(),draft:"must roll back".into()}),Err(CoreError::Database(_))));
    assert_eq!(store.snapshot(),committed);
    assert_eq!(store.revision,revision);
    assert_eq!(store.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|row|row.get(0)).unwrap(),durable);
    assert_eq!(store.db.query_row::<u32,_,_>("SELECT count(*) FROM observer_child",[],|row|row.get(0)).unwrap(),0);
    {
        let observations=OBSERVATIONS.lock().unwrap();
        assert_eq!(observations.len(),2);
        assert!(!observations[1].success);
        let phases:Vec<_>=observations[1].completed.iter().flatten().map(|mark|mark.phase).collect();
        assert_eq!(phases.last(),Some(&PersistencePhase::SqlUpdate));
        assert!(!phases.contains(&PersistencePhase::DurableCommit));
    }
    store.db.execute_batch("DROP TRIGGER reject_observed_commit").unwrap();
    // A invalid candidate stops before reconciliation and cannot skip to the
    // second validation, mutate business state, or produce a success receipt.
    assert!(store.apply(SceneCommand::SetRefs{scene_id:scene.clone(),refs:vec![Reference{kind:ReferenceKind::Region,id:"missing-region".into()}]}).is_err());
    assert_eq!(store.snapshot(),committed);
    {
        let observations=OBSERVATIONS.lock().unwrap();
        assert_eq!(observations.len(),3);
        assert!(!observations[2].success);
        assert_eq!(observations[2].completed.iter().flatten().count(),1);
        assert_eq!(observations[2].completed[0].unwrap().phase,PersistencePhase::GeometryCleanup);
    }
    store.set_persistence_observer(None);
    store.apply(SceneCommand::SetDraft{scene_id:scene,draft:"measurement disabled".into()}).unwrap();
    assert_eq!(OBSERVATIONS.lock().unwrap().len(),3);
}

#[test]
fn failing_diagnostic_callback_cannot_change_a_successfully_committed_result() {
    fn faulty_observer(_:PersistenceTiming){panic!("synthetic diagnostic callback failure")}
    let mut store=Store::open_in_memory().unwrap();
    let scene=store.active_scene().unwrap().id.clone();
    store.set_persistence_observer(Some(faulty_observer));
    let result=store.apply(SceneCommand::SetDraft{scene_id:scene,draft:"still accepted".into()}).unwrap();
    assert_eq!(store.snapshot(),result);
    assert_eq!(store.active_scene().unwrap().draft,"still accepted");
    let stored:Snapshot=serde_json::from_str(&store.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|row|row.get(0)).unwrap()).unwrap();
    assert_eq!(stored,result);
}

#[test]
fn reconciled_journal_projection_is_validated_again_and_failure_is_observed_atomically() {
    static RESULTS:Mutex<Vec<PersistenceTiming>>=Mutex::new(Vec::new());
    fn observe_reconciliation(timing:PersistenceTiming){RESULTS.lock().unwrap().push(timing)}
    RESULTS.lock().unwrap().clear();
    let mut store=Store::open_in_memory().unwrap();
    let scene=store.active_scene().unwrap().id.clone();
    store.apply(SceneCommand::SetDraft{scene_id:scene.clone(),draft:"synthetic reconciliation".into()}).unwrap();
    let run=store.begin_run(&scene).unwrap();
    let record=crate::journal::required(&store.db,&run.run_id).unwrap();
    // Simulate a changed/corrupt ledger entry that will be projected into the
    // candidate only during on_snapshot_change, after the first validation.
    let (event,_)=crate::journal::create_event(&store.db,&record,ReceiptKind::Tool,0,String::new(),
        Some("synthetic-call".into()),Some(ToolBinding::Local{name:"synthetic".into()}),
        Some("0".repeat(64)),None).unwrap();
    let before=store.snapshot();
    let revision=store.revision;
    let payload=store.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|row|row.get(0)).unwrap();
    let metadata=store.db.query_row::<String,_,_>("SELECT metadata FROM agent_run_events WHERE id=?1",[&event.summary.id],|row|row.get(0)).unwrap();
    store.set_persistence_observer(Some(observe_reconciliation));
    assert!(matches!(store.cancel_run(&scene),Err(CoreError::Invalid(_))));
    assert_eq!(store.snapshot(),before);
    assert_eq!(store.revision,revision);
    assert_eq!(store.db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|row|row.get(0)).unwrap(),payload);
    assert_eq!(store.db.query_row::<String,_,_>("SELECT metadata FROM agent_run_events WHERE id=?1",[&event.summary.id],|row|row.get(0)).unwrap(),metadata);
    assert_eq!(crate::journal::required(&store.db,&run.run_id).unwrap().status,RecordedRunStatus::Running);
    let result=RESULTS.lock().unwrap();
    assert_eq!(result.len(),1);
    assert!(!result[0].success);
    let phases:Vec<_>=result[0].completed.iter().flatten().map(|mark|mark.phase).collect();
    assert_eq!(phases.last(),Some(&PersistencePhase::TransactionAndLedger));
    assert!(!phases.contains(&PersistencePhase::ValidateReconciled));
}
