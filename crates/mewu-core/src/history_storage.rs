// SPDX-License-Identifier: MPL-2.0
//! Explicit history maintenance. No startup cleanup and no filesystem authority.
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCleanupPlan {
    pub revision: i64,
    pub count: u64,
    pub bytes: u64,
    pub cleanable_count: u64,
    pub cleanable_bytes: u64,
    pub scene_ids: Vec<String>,
}

fn plan(db: &SqlConnection, state: &Snapshot, revision: i64) -> Result<HistoryCleanupPlan> {
    let root = |scene: &Scene| {
        scene
            .blackboard_link
            .as_ref()
            .map_or(scene.id.as_str(), |l| l.parent_scene_id.as_str())
            .to_owned()
    };
    let mut protected = HashSet::new();
    for scene in &state.scenes {
        // Backing boards are frozen implementation scenes. Their parent's open/
        // closed status decides eligibility; an active/running board protects it.
        if scene.id == state.active_scene_id
            || is_running(scene)
            || (scene.blackboard_link.is_none() && !scene.closed)
        {
            protected.insert(root(scene));
        }
    }
    for source in state.memories.iter().filter_map(|m| m.source.as_ref()) {
        if let Some(scene) = state.scenes.iter().find(|s| s.id == source.scene_id) {
            protected.insert(root(scene));
        }
    }
    for id in db
        .prepare(
            "SELECT DISTINCT source_scene_id FROM memory_records WHERE source_scene_id IS NOT NULL",
        )?
        .query_map([], |r| r.get::<_, String>(0))?
    {
        let id = id?;
        if let Some(scene) = state.scenes.iter().find(|s| s.id == id) {
            protected.insert(root(scene));
        }
    }
    // Include suppressed and queued external evidence as well: deletion of
    // history is not permission to sever memory provenance or remote receipts.
    for raw in db
        .prepare("SELECT source_json FROM external_memory_evidence")?
        .query_map([], |r| r.get::<_, String>(0))?
    {
        let source: EvidenceSource = serde_json::from_str(&raw?)?;
        if let Some(scene) = source
            .scene_id
            .and_then(|id| state.scenes.iter().find(|s| s.id == id))
        {
            protected.insert(root(scene));
        }
    }
    let candidates: HashSet<_> = state
        .scenes
        .iter()
        .filter(|s| s.blackboard_link.is_none() && s.closed && !protected.contains(&s.id))
        .map(|s| s.id.clone())
        .collect();
    let mut sizes = BTreeMap::<String, u64>::new();
    for scene in &state.scenes {
        sizes.insert(scene.id.clone(), serde_json::to_vec(scene)?.len() as u64);
    }
    // Count stored UTF-8 payloads, never pretend logical bytes equal SQLite's
    // page allocation. Assets and immutable raster layouts are separate storage.
    let sql = "SELECT r.scene_id,length(CAST(r.data AS BLOB)) + coalesce((SELECT sum(length(CAST(e.metadata AS BLOB))+length(CAST(e.content AS BLOB))) FROM agent_run_events e WHERE e.run_id=r.id),0) + coalesce((SELECT length(CAST(v.data AS BLOB)) FROM visual_run_inputs v WHERE v.run_id=r.id),0) + coalesce((SELECT length(CAST(v.data AS BLOB)) FROM video_run_inputs v WHERE v.run_id=r.id),0) FROM agent_run_records r";
    for row in db
        .prepare(sql)?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
    {
        let (scene, bytes) = row?;
        *sizes.entry(scene).or_default() +=
            u64::try_from(bytes).map_err(|_| invalid("历史占用无效"))?;
    }
    let mut scene_ids: Vec<_> = state
        .scenes
        .iter()
        .filter(|s| candidates.contains(&root(s)))
        .map(|s| s.id.clone())
        .collect();
    scene_ids.sort();
    Ok(HistoryCleanupPlan {
        revision,
        count: state
            .scenes
            .iter()
            .filter(|s| s.blackboard_link.is_none())
            .count() as u64,
        bytes: sizes.values().sum(),
        cleanable_count: candidates.len() as u64,
        cleanable_bytes: scene_ids
            .iter()
            .map(|id| sizes.get(id).copied().unwrap_or(0))
            .sum(),
        scene_ids,
    })
}

impl Store {
    pub fn history_cleanup_plan(&self) -> Result<HistoryCleanupPlan> {
        plan(&self.db, &self.state, self.revision)
    }

    /// All typed asset references including original message attachments. A
    /// visual/video receipt's binding remains present in those immutable messages.
    pub fn storage_assets(&self) -> Vec<Asset> {
        self.state
            .scenes
            .iter()
            .flat_map(|s| {
                s.background
                    .iter()
                    .chain(s.regions.iter().flat_map(|r| {
                        r.image_override
                            .iter()
                            .chain(r.translation.iter().map(|t| &t.overlay))
                    }))
                    .chain(s.items.iter().map(|i| &i.asset))
                    .chain(
                        s.messages
                            .iter()
                            .flat_map(|m| m.attachments.iter().flatten()),
                    )
            })
            .cloned()
            .collect()
    }

    pub fn clean_history(&mut self, expected: &HistoryCleanupPlan) -> Result<Snapshot> {
        let revision = self.revision;
        self.journal_transaction(|db, state| {
            if plan(db, state, revision)? != *expected { return Err(CoreError::ConcurrentModification); }
            if expected.scene_ids.is_empty() { return Ok(()); }
            for scene in &expected.scene_ids {
                for table in ["agent_run_events", "visual_run_inputs", "video_run_inputs"] {
                    db.execute(&format!("DELETE FROM {table} WHERE run_id IN(SELECT id FROM agent_run_records WHERE scene_id=?1)"), [scene])?;
                }
                db.execute("DELETE FROM agent_run_records WHERE scene_id=?1", [scene])?;
            }
            state.scenes.retain(|s| !expected.scene_ids.contains(&s.id));
            crate::journal::validate(db, state)?;
            crate::memory::validate(db, state)?;
            crate::external_memory::validate(db, state)?;
            crate::visual_annotations::validate(db, state)?;
            crate::video_annotations::validate_ledger(db, state)?;
            Ok(())
        })?;
        Ok(self.snapshot())
    }

    /// Explicit maintenance only, after a successful history transaction. FTS
    /// virtual tables and immutable shared layouts are left intact by VACUUM.
    pub fn compact_history_storage(&mut self) -> Result<()> {
        self.db
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(store: &mut Store, id: &str) {
        store
            .apply(SceneCommand::CloseScene {
                scene_id: id.into(),
            })
            .unwrap();
    }
    fn draft(store: &mut Store, text: &str) -> String {
        let id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: text.into(),
            })
            .unwrap();
        id
    }
    #[test]
    fn statistics_and_empty_cleanup_never_write_or_remove_open_frozen_scenes() {
        let mut store = Store::open_in_memory().unwrap();
        let id = draft(&mut store, "frozen draft");
        store
            .apply(SceneCommand::FreezeScene { scene_id: id })
            .unwrap();
        let before = store.snapshot();
        let revision = store.revision;
        let plan = store.history_cleanup_plan().unwrap();
        assert_eq!(plan.count, 2);
        assert_eq!(plan.cleanable_count, 0);
        assert_eq!(store.clean_history(&plan).unwrap(), before);
        assert_eq!(store.revision, revision);
    }
    #[test]
    fn cleanup_removes_closed_journal_transactionally_and_rejects_stale_review() {
        let mut store = Store::open_in_memory().unwrap();
        let id = draft(&mut store, "closed historical question");
        let run = store.begin_run(&id).unwrap();
        store
            .begin_model_request(
                &id,
                &run.run_id,
                ModelDispatchInput {
                    round: 0,
                    projected_request_sha256: "0".repeat(64),
                },
            )
            .unwrap();
        store
            .fail_run(&id, &run.run_id, "fixture interrupted")
            .unwrap();
        close(&mut store, &id);
        let plan = store.history_cleanup_plan().unwrap();
        assert_eq!(plan.cleanable_count, 1);
        assert!(plan.cleanable_bytes > 0);
        let new_id = draft(&mut store, "new active draft");
        assert!(store.clean_history(&plan).is_err());
        assert!(store.snapshot().scenes.iter().any(|s| s.id == id));
        let plan = store.history_cleanup_plan().unwrap();
        let after = store.clean_history(&plan).unwrap();
        assert_eq!(after.scenes.len(), 1);
        assert_eq!(after.active_scene_id, new_id);
        for table in [
            "agent_run_records",
            "agent_run_events",
            "visual_run_inputs",
            "video_run_inputs",
        ] {
            let n: i64 = store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
        }
        crate::journal::validate(&store.db, &after).unwrap();
        store.compact_history_storage().unwrap();
        crate::memory::validate(&store.db, &after).unwrap();
    }
    #[test]
    fn cleanup_keeps_local_memory_evidence_and_its_source_conversation() {
        let mut store = Store::open_in_memory().unwrap();
        let id = draft(&mut store, "remember this preference");
        let run = store.begin_run(&id).unwrap();
        store
            .memory_for_run(
                &id,
                &run.run_id,
                MemoryMutation::Save {
                    id: None,
                    text: "memory evidence".into(),
                    expected_revision: None,
                },
            )
            .unwrap();
        store.finish_run(&id, &run.run_id, "remembered").unwrap();
        close(&mut store, &id);
        let unprotected = draft(&mut store, "unprotected history");
        close(&mut store, &unprotected);
        let plan = store.history_cleanup_plan().unwrap();
        assert_eq!(plan.scene_ids, vec![unprotected]);
        let agent = store.snapshot().agents[0].id.clone();
        let memory = store.memory_page(&agent, "", None, 25).unwrap();
        store.clean_history(&plan).unwrap();
        assert!(store.snapshot().scenes.iter().any(|s| s.id == id));
        assert_eq!(store.memory_page(&agent, "", None, 25).unwrap(), memory);
        crate::memory::validate(&store.db, &store.state).unwrap();
    }
    #[test]
    fn cleanup_failure_rolls_back_journal_snapshot_and_revision_together() {
        let mut store = Store::open_in_memory().unwrap();
        let id = draft(&mut store, "rollback question");
        let run = store.begin_run(&id).unwrap();
        store.finish_run(&id, &run.run_id, "reply").unwrap();
        close(&mut store, &id);
        store.db.execute_batch("CREATE TRIGGER cleanup_test_failure BEFORE DELETE ON agent_run_records BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let before = store.snapshot();
        let plan = store.history_cleanup_plan().unwrap();
        assert!(store.clean_history(&plan).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(store.history_cleanup_plan().unwrap(), plan);
        crate::journal::validate(&store.db, &before).unwrap();
    }
    #[test]
    fn closed_parent_and_backing_blackboard_are_removed_as_one_group() {
        let mut store = Store::open_in_memory().unwrap();
        let id = draft(&mut store, "board parent");
        let asset_id = Uuid::new_v4().to_string();
        let asset = Asset {
            id: asset_id.clone(),
            name: "黑板.png".into(),
            kind: AssetKind::Image,
            path: format!("D:/assets/{asset_id}.board.png"),
            width: Some(1280),
            height: Some(720),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let result = store.create_blackboard_document(&id, asset).unwrap();
        let board = result
            .scenes
            .iter()
            .find(|s| s.blackboard_link.is_some())
            .unwrap()
            .id
            .clone();
        // A still-active backing board cannot be removed even if its parent is closed.
        close(&mut store, &id);
        let plan = store.history_cleanup_plan().unwrap();
        assert!(plan.scene_ids.is_empty());
        close(&mut store, &board);
        let plan = store.history_cleanup_plan().unwrap();
        assert_eq!(plan.cleanable_count, 1);
        assert_eq!(plan.scene_ids.len(), 2);
        let after = store.clean_history(&plan).unwrap();
        assert!(!after.scenes.iter().any(|s| s.id == id || s.id == board));
        validate_snapshot(&after).unwrap();
    }
}
