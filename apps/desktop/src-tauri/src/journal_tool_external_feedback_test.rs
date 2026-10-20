// SPDX-License-Identifier: MPL-2.0
// Actual Store transactions/outbox/receipts; no memory provider, HTTP or keys.
mod external_feedback {
    use super::*;
    use mewu_core::{HostNewMemoryBinding, MemoryBindingView, MemoryPolicy, MemoryWorkObservation};
    use rusqlite::Connection;
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture {
        store: Store,
        run: RunContext,
        binding: MemoryBindingView,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../.private/remake-review/agent-runtime-feedback-20261020")
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("spaces.db");
            let mut store = Store::open(&path).unwrap();
            let mut agent = store.snapshot().agents[0].clone();
            agent.memory_enabled = true;
            let agent_id = agent.id.clone();
            store.apply(SceneCommand::SaveAgent { agent }).unwrap();
            let selection = store
                .memory_provider_status(&agent_id, None, 100)
                .unwrap()
                .selection;
            let reserved = store
                .reserve_memory_binding(
                    selection.revision,
                    HostNewMemoryBinding {
                        id: uuid::Uuid::new_v4().to_string(),
                        agent_id: agent_id.clone(),
                        endpoint: "https://memory.example.test".into(),
                        credential_id: None,
                        policy: MemoryPolicy {
                            recall: true,
                            explicit_retain: true,
                            completed_turn_sync: false,
                        },
                        plugin_id: "mewu.memory-hindsight".into(),
                        contribution_id: "memory".into(),
                        plugin_revision: 1,
                    },
                )
                .unwrap();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60_000;
            let pending = store
                .pending_memory_work(now, 16)
                .unwrap()
                .into_iter()
                .find(|work| work.operation_id == reserved.operation_id)
                .unwrap();
            let work = store
                .claim_memory_work(&pending.operation_id, pending.revision, &pending.grant, now)
                .unwrap()
                .unwrap();
            store
                .record_memory_work(
                    work.lease,
                    MemoryWorkObservation::BankReady {
                        bank_id: work.route.bank_id,
                        service_version: "0.10.3".into(),
                        observations_enabled: false,
                    },
                    now + 1,
                )
                .unwrap();
            let binding = store.memory_binding(&reserved.binding.id).unwrap();
            store
                .select_memory_provider(
                    &agent_id,
                    selection.revision,
                    Some(&binding.id),
                    Some(binding.revision),
                )
                .unwrap();
            let scene = store.snapshot().active_scene_id;
            store
                .apply(SceneCommand::SetDraft {
                    scene_id: scene.clone(),
                    draft: "我偏好简短回答。".into(),
                })
                .unwrap();
            let grant = MemoryAuthorityGrant {
                binding_id: binding.id.clone(),
                binding_revision: binding.revision,
                plugin_id: binding.plugin_id.clone(),
                plugin_revision: binding.plugin_revision,
                contribution_id: binding.contribution_id.clone(),
            };
            let run = store.begin_run_with_memory(&scene, Some(grant)).unwrap();
            Self {
                store,
                run,
                binding,
                path,
            }
        }
        fn plan(&mut self, round: u32, calls: &[(&str, &str)]) -> Vec<DispatchLease> {
            let model = self
                .store
                .begin_model_request(
                    &self.run.scene_id,
                    &self.run.run_id,
                    ModelDispatchInput {
                        round,
                        projected_request_sha256: "a".repeat(64),
                    },
                )
                .unwrap();
            self.store
                .record_model_turn(
                    &model,
                    CompletePublicTurn {
                        text: String::new(),
                        calls: calls
                            .iter()
                            .enumerate()
                            .map(|(index, (name, wire))| ProposedTool {
                                call_id: format!("external-{round}-{index}"),
                                binding: ToolBinding::ExternalMemory {
                                    binding_id: self.binding.id.clone(),
                                    binding_revision: self.binding.revision,
                                    plugin_id: self.binding.plugin_id.clone(),
                                    plugin_revision: self.binding.plugin_revision,
                                    contribution_id: self.binding.contribution_id.clone(),
                                    name: (*name).into(),
                                },
                                arguments_wire_sha256: format!(
                                    "{:x}",
                                    Sha256::digest(wire.as_bytes())
                                ),
                            })
                            .collect(),
                    },
                )
                .unwrap()
                .tools
                .into_iter()
                .map(|tool| tool.lease)
                .collect()
        }
        fn counts(&self) -> (i64, i64, i64) {
            let db = Connection::open(&self.path).unwrap();
            let count = |table: &str| {
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
            };
            (
                count("external_memory_evidence"),
                count("external_memory_outbox"),
                count("external_memory_tombstones"),
            )
        }
        fn proof(&self) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
            let db = Connection::open(&self.path).unwrap();
            [
                "app_state",
                "external_memory_bindings",
                "external_memory_evidence",
                "external_memory_outbox",
                "external_memory_tombstones",
                "agent_run_events",
            ]
            .into_iter()
            .map(|table| {
                let mut statement = db
                    .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                    .unwrap();
                let columns = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get::<_, rusqlite::types::Value>(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                rows
            })
            .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            drop(std::mem::replace(
                &mut self.store,
                Store::open_in_memory().unwrap(),
            ));
            for suffix in ["", "-wal", "-shm"] {
                let mut name = self.path.as_os_str().to_os_string();
                name.push(suffix);
                let _ = std::fs::remove_file(PathBuf::from(name));
            }
            let _ = std::fs::remove_dir(self.path.parent().unwrap());
        }
    }

    #[test]
    fn source_rejection_is_typed_atomic_and_keeps_a_valid_sibling_for_correction() {
        let mut f = Fixture::new();
        let bad = r#"{"sourceQuote":"PRIVATE-NOT-USER"}"#;
        let good = r#"{"sourceQuote":"简短回答"}"#;
        let leases = f.plan(0, &[("memory_save", bad), ("memory_save", good)]);
        let snapshot = f.store.snapshot();
        let counts = f.counts();
        let proof = f.proof();
        assert!(matches!(
            f.store
                .queue_external_quote_with_receipt(&leases[0], "PRIVATE-NOT-USER"),
            Err(mewu_core::CoreError::MemorySourceRequired)
        ));
        assert_eq!(f.store.snapshot(), snapshot);
        assert_eq!(f.counts(), counts);
        assert_eq!(f.proof(), proof);
        let LocalEffectOutcome::Rejected(_, output) =
            external_effect(&mut f.store, &leases[0], "memory_save", bad).unwrap()
        else {
            panic!("expected source rejection")
        };
        let output: Value = serde_json::from_str(&output.model_json).unwrap();
        assert_eq!(output["isError"], true);
        assert_eq!(output["executed"], false);
        assert!(!output.to_string().contains("PRIVATE-NOT-USER"));
        assert!(!output.to_string().contains("sourceMessageId"));
        assert_eq!(f.counts(), counts);
        assert!(matches!(
            external_effect(&mut f.store, &leases[1], "memory_save", good).unwrap(),
            LocalEffectOutcome::Applied(_)
        ));
        assert!(f.store.run_is_active(&f.run.scene_id, &f.run.run_id));
        let lease = f.plan(1, &[("memory_save", good)]).remove(0);
        assert!(matches!(
            external_effect(&mut f.store, &lease, "memory_save", good).unwrap(),
            LocalEffectOutcome::Applied(_)
        ));
        let page = f
            .store
            .run_journal_page(&f.run.scene_id, &f.run.run_id, None, 25)
            .unwrap()
            .unwrap();
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == ReceiptKind::Tool && e.phase == ReceiptPhase::NotSent)
                .count(),
            1
        );
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == ReceiptKind::Tool && e.phase == ReceiptPhase::Responded)
                .count(),
            2
        );
    }

    #[test]
    fn stale_revision_rejection_does_not_forget_but_a_corrected_forget_keeps_its_real_cancel() {
        let mut f = Fixture::new();
        let evidence = f
            .store
            .queue_manual_memory_evidence(
                &f.run.agent.id,
                &f.binding.id,
                f.binding.revision,
                &uuid::Uuid::new_v4().to_string(),
                "合成用户事实",
            )
            .unwrap();
        let stale =
            json!({"id":evidence.evidence_id,"expectedRevision":evidence.revision+1}).to_string();
        let lease = f.plan(0, &[("memory_forget", &stale)]).remove(0);
        let snapshot = f.store.snapshot();
        let counts = f.counts();
        let proof = f.proof();
        assert!(matches!(
            f.store.forget_external_evidence_with_receipt(
                &lease,
                &evidence.evidence_id,
                evidence.revision + 1
            ),
            Err(mewu_core::CoreError::MemoryConflict)
        ));
        assert_eq!(f.store.snapshot(), snapshot);
        assert_eq!(f.counts(), counts);
        assert_eq!(f.proof(), proof);
        assert!(matches!(
            external_effect(&mut f.store, &lease, "memory_forget", &stale).unwrap(),
            LocalEffectOutcome::Rejected(_, _)
        ));
        assert!(f.store.run_is_active(&f.run.scene_id, &f.run.run_id));
        assert_eq!(f.counts(), counts);
        let corrected =
            json!({"id":evidence.evidence_id,"expectedRevision":evidence.revision}).to_string();
        let lease = f.plan(1, &[("memory_forget", &corrected)]).remove(0);
        assert!(matches!(
            external_effect(&mut f.store, &lease, "memory_forget", &corrected).unwrap(),
            LocalEffectOutcome::Applied(_)
        ));
        assert!(!f.store.run_is_active(&f.run.scene_id, &f.run.run_id));
    }

    #[test]
    fn sql_failure_unknown_and_cancellation_never_become_safe_argument_feedback() {
        for bad_source in [false, true] {
            let mut f = Fixture::new();
            let args = if bad_source {
                r#"{"sourceQuote":"NOT-USER"}"#
            } else {
                r#"{"sourceQuote":"简短回答"}"#
            };
            let lease = f.plan(0, &[("memory_save", args)]).remove(0);
            let snapshot = f.store.snapshot();
            let counts = f.counts();
            let proof = f.proof();
            let db = Connection::open(&f.path).unwrap();
            db.execute_batch("CREATE TRIGGER external_feedback_sql_failure BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'synthetic receipt failure'); END").unwrap();
            assert!(external_effect(&mut f.store, &lease, "memory_save", args).is_err());
            assert_eq!(f.store.snapshot(), snapshot);
            assert_eq!(f.counts(), counts);
            assert_eq!(f.proof(), proof);
            db.execute_batch("DROP TRIGGER external_feedback_sql_failure")
                .unwrap();
            if bad_source {
                f.store.cancel_run(&f.run.scene_id).unwrap();
            } else {
                f.store.mark_tool_dispatched(&lease).unwrap();
                f.store
                    .record_request_unknown(&lease, NoResponseReason::TimedOut)
                    .unwrap();
            }
            let snapshot = f.store.snapshot();
            assert!(external_effect(&mut f.store, &lease, "memory_save", args).is_err());
            assert_eq!(f.store.snapshot(), snapshot);
            assert_eq!(f.counts(), counts);
        }
    }
}
