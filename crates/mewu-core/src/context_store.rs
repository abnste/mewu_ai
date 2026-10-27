// SPDX-License-Identifier: MPL-2.0
use super::*;
use mewu_agent_runtime::{ContextCheckpoint, MAX_SUMMARY_BYTES};
use rusqlite::OptionalExtension;

// JSON may escape one byte as six (for example a control character). This
// bound applies to the serialized cache, not the decoded summary byte limit.
const MAX_CHECKPOINT_BYTES: usize = MAX_SUMMARY_BYTES * 6 + 2048;

pub(super) fn migrate(db: &SqlConnection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_context_checkpoints (
        scene_id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL
    );",
    )?;
    Ok(())
}
pub(super) fn validate(db: &SqlConnection) -> Result<()> {
    db.prepare("SELECT scene_id,payload FROM agent_context_checkpoints LIMIT 0")?;
    Ok(())
}

pub(super) fn prune(db: &SqlConnection, before: &Snapshot, after: &Snapshot) -> Result<()> {
    let remaining: std::collections::HashMap<_, _> =
        after.scenes.iter().map(|s| (&s.id, s)).collect();
    for scene in &before.scenes {
        if !remaining.get(&scene.id).is_some_and(|s| {
            s.agent_id == scene.agent_id
                && s.conversation_start == scene.conversation_start
                && s.messages.len() >= scene.messages.len()
        }) {
            db.execute(
                "DELETE FROM agent_context_checkpoints WHERE scene_id=?1",
                [&scene.id],
            )?;
        }
    }
    Ok(())
}

impl Store {
    /// Private cache, deliberately absent from UI snapshots and memory tools.
    pub fn agent_context_checkpoint(
        &self,
        scene_id: &str,
        run_id: &str,
    ) -> Result<Option<ContextCheckpoint>> {
        if !self.run_is_active(scene_id, run_id) {
            return Err(CoreError::JournalConflict);
        }
        let payload: Option<String> = self
            .db
            .query_row(
                "SELECT CASE WHEN length(CAST(payload AS BLOB))<=?2 THEN payload ELSE NULL END
                 FROM agent_context_checkpoints WHERE scene_id=?1",
                params![scene_id, MAX_CHECKPOINT_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        // A corrupt derived cache is disposable. Never damage authoritative history.
        Ok(payload
            .filter(|p| p.len() <= MAX_CHECKPOINT_BYTES)
            .and_then(|p| serde_json::from_str(&p).ok()))
    }
    pub fn save_agent_context_checkpoint(
        &mut self,
        scene_id: &str,
        run_id: &str,
        checkpoint: &ContextCheckpoint,
    ) -> Result<()> {
        if !self.run_is_active(scene_id, run_id) {
            return Err(CoreError::JournalConflict);
        }
        let scene = self.scene(scene_id)?;
        let history = &scene.messages[scene.conversation_start.unwrap_or(0)..];
        if checkpoint.summary.trim().is_empty()
            || checkpoint.summary.len() > MAX_SUMMARY_BYTES
            || checkpoint.message_count == 0
            || checkpoint.scope.len() != 64
            || checkpoint.prefix_hash.len() != 64
            || history
                .get(checkpoint.message_count)
                .is_none_or(|m| m.role != MessageRole::User)
        {
            return Err(invalid("上下文检查点无效"));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 = tx.query_row("SELECT revision FROM app_state WHERE id=1", [], |r| {
            r.get(0)
        })?;
        if revision != self.revision {
            return Err(CoreError::ConcurrentModification);
        }
        tx.execute(
            "INSERT INTO agent_context_checkpoints(scene_id,payload) VALUES(?1,?2)
            ON CONFLICT(scene_id) DO UPDATE SET payload=excluded.payload",
            params![scene_id, serde_json::to_string(checkpoint)?],
        )?;
        tx.commit()?;
        self.notify_checkpoint_maintenance();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn begin(store: &mut Store, scene: &str, text: &str) -> RunContext {
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.into(),
                draft: text.into(),
            })
            .unwrap();
        store.begin_run(scene).unwrap()
    }
    #[test]
    fn checkpoint_survives_reopen_but_cannot_be_written_by_a_canceled_run() {
        let path = std::env::temp_dir().join(format!("mewu-context-{}.db", Uuid::new_v4()));
        let mut store = Store::open(&path).unwrap();
        let scene = store.snapshot().active_scene_id;
        let first = begin(&mut store, &scene, "first");
        store.finish_run(&scene, &first.run_id, "answer").unwrap();
        let next = begin(&mut store, &scene, "second");
        let before = serde_json::to_value(store.snapshot()).unwrap();
        let checkpoint = ContextCheckpoint {
            scope: "a".repeat(64),
            prefix_hash: "b".repeat(64),
            message_count: 2,
            summary: "\"\\\n".repeat(4000),
        };
        store
            .save_agent_context_checkpoint(&scene, &next.run_id, &checkpoint)
            .unwrap();
        assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), before);
        assert_eq!(
            store
                .agent_context_checkpoint(&scene, &next.run_id)
                .unwrap(),
            Some(checkpoint.clone())
        );
        store.cancel_run(&scene).unwrap();
        assert!(store
            .save_agent_context_checkpoint(&scene, &next.run_id, &checkpoint)
            .is_err());
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let resumed = begin(&mut store, &scene, "continue");
        assert_eq!(
            store
                .agent_context_checkpoint(&scene, &resumed.run_id)
                .unwrap(),
            Some(checkpoint)
        );
        assert!(store
            .scene(&scene)
            .unwrap()
            .messages
            .iter()
            .any(|m| m.text == "first"));
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn schema_migration_adds_private_cache_without_changing_history_payload() {
        let mut store = Store::open_in_memory().unwrap();
        let payload: String = store
            .db
            .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
            .unwrap();
        store
            .db
            .execute_batch("DROP TABLE agent_context_checkpoints; PRAGMA user_version=11;")
            .unwrap();
        let db = std::mem::replace(&mut store.db, SqlConnection::open_in_memory().unwrap());
        let reopened = Store::from_connection(db).unwrap();
        let after: String = reopened
            .db
            .query_row("SELECT payload FROM app_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(payload, after);
        assert!(reopened
            .agent_context_checkpoint("missing", "missing")
            .is_err());
    }
}
