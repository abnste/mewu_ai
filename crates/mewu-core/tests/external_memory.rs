// SPDX-License-Identifier: MPL-2.0
//! Isolated SQLite/ledger tests. No HTTP, credentials, model calls or real user data.
use mewu_core::*;
use rusqlite::{params, Connection as Database};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

fn journal_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn journal_memory_call(b: &MemoryBindingView, name: &str) -> ProposedTool {
    ProposedTool {
        call_id: format!("call-{name}"),
        binding: ToolBinding::ExternalMemory {
            binding_id: b.id.clone(),
            binding_revision: b.revision,
            plugin_id: b.plugin_id.clone(),
            plugin_revision: b.plugin_revision,
            contribution_id: b.contribution_id.clone(),
            name: name.into(),
        },
        arguments_wire_sha256: journal_hash("{}"),
    }
}

#[test]
fn journal_external_queue_rolls_back_with_receipt_failure() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let scene = store.snapshot().active_scene_id;
    let c = start(&mut store, &scene, &b, "记住我喜欢中文");
    let model = store
        .begin_model_request(
            &scene,
            &c.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: journal_hash("request"),
            },
        )
        .unwrap();
    let turn = store
        .record_model_turn(
            &model,
            CompletePublicTurn {
                text: String::new(),
                calls: vec![journal_memory_call(&b, "memory_save")],
            },
        )
        .unwrap();
    let db = Database::open(&temp.0).unwrap();
    let before = counts(&db);
    let rev = revision(&db);
    let snapshot = store.snapshot();
    db.execute_batch("CREATE TRIGGER reject_journal_queue BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store
        .queue_external_quote_with_receipt(&turn.tools[0].lease, "喜欢中文")
        .is_err());
    assert_eq!(counts(&db), before);
    assert_eq!(revision(&db), rev);
    assert_eq!(store.snapshot(), snapshot);
    db.execute_batch("DROP TRIGGER reject_journal_queue")
        .unwrap();
    let result = store
        .queue_external_quote_with_receipt(&turn.tools[0].lease, "喜欢中文")
        .unwrap();
    assert_eq!(revision(&db), rev + 1);
    assert_eq!(
        result.snapshot.scenes[0].run.as_ref().unwrap().steps[0].status,
        ToolStepStatus::Completed
    );
    let value: Value =
        serde_json::from_str(result.result.model_visible_json.as_deref().unwrap()).unwrap();
    let evidence = value["evidenceId"].as_str().unwrap();
    assert!(store
        .external_memory_entry(&agent, &b.id, evidence)
        .unwrap()
        .is_some());
}

#[test]
fn journal_forget_self_cancellation_keeps_known_receipt_and_other_unknowns_atomic() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let written = queue(&mut store, &b, &uid(), "待遗忘证据");
    let scene = store.snapshot().active_scene_id;
    let c = start(&mut store, &scene, &b, "忘记这条事实");
    let model = store
        .begin_model_request(
            &scene,
            &c.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: journal_hash("request"),
            },
        )
        .unwrap();
    let other = ProposedTool {
        call_id: "call-other".into(),
        binding: ToolBinding::Local {
            name: "other".into(),
        },
        arguments_wire_sha256: journal_hash("{}"),
    };
    let turn = store
        .record_model_turn(
            &model,
            CompletePublicTurn {
                text: "正在处理".into(),
                calls: vec![journal_memory_call(&b, "memory_forget"), other],
            },
        )
        .unwrap();
    store.mark_tool_dispatched(&turn.tools[1].lease).unwrap();
    let db = Database::open(&temp.0).unwrap();
    let rev = revision(&db);
    let before = store.snapshot();
    let before_counts = counts(&db);
    db.execute_batch("CREATE TRIGGER reject_journal_forget BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store
        .forget_external_evidence_with_receipt(
            &turn.tools[0].lease,
            &written.evidence_id,
            written.revision
        )
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(counts(&db), before_counts);
    assert_eq!(revision(&db), rev);
    db.execute_batch("DROP TRIGGER reject_journal_forget")
        .unwrap();
    let result = store
        .forget_external_evidence_with_receipt(
            &turn.tools[0].lease,
            &written.evidence_id,
            written.revision,
        )
        .unwrap();
    assert_eq!(revision(&db), rev + 1);
    assert!(!store.run_is_active(&scene, &c.run_id));
    assert_eq!(
        result.snapshot.scenes[0].run.as_ref().unwrap().status,
        RunStatus::Canceled
    );
    let page = store
        .run_journal_page(&scene, &c.run_id, None, 25)
        .unwrap()
        .unwrap();
    assert_eq!(page.entries[1].phase, ReceiptPhase::Responded);
    assert_eq!(page.entries[2].phase, ReceiptPhase::Unknown);
    assert_eq!(page.summary.status, RecordedRunStatus::Canceled);
    assert_eq!(
        result.snapshot.scenes[0].messages[0].tool_steps[0].status,
        ToolStepStatus::Completed
    );
    assert_eq!(
        result.snapshot.scenes[0].messages[0].tool_steps[1].status,
        ToolStepStatus::Unknown
    );
    assert!(store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap()
        .attributed_text
        .is_none());
}

struct TestDb(PathBuf);
impl TestDb {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("mewu-external-memory-{}", Uuid::new_v4()));
        assert!(!directory
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&directory).unwrap();
        Self(directory.join("spaces.db"))
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
fn uid() -> String {
    Uuid::new_v4().to_string()
}
fn tick() -> u64 {
    static OFFSET: AtomicU64 = AtomicU64::new(60_000);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + OFFSET.fetch_add(60_000, Ordering::Relaxed)
}
fn enable(store: &mut Store) -> String {
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    let id = agent.id.clone();
    store.apply(SceneCommand::SaveAgent { agent }).unwrap();
    id
}
fn binding(store: &Store, agent: &str, id: &str) -> MemoryBindingView {
    store
        .memory_provider_status(agent, None, 100)
        .unwrap()
        .bindings
        .into_iter()
        .find(|b| b.id == id)
        .unwrap()
}
fn grant(view: &MemoryBindingView) -> MemoryAuthorityGrant {
    MemoryAuthorityGrant {
        binding_id: view.id.clone(),
        binding_revision: view.revision,
        plugin_id: view.plugin_id.clone(),
        contribution_id: view.contribution_id.clone(),
        plugin_revision: view.plugin_revision,
    }
}
fn reserve(
    store: &mut Store,
    agent: &str,
    endpoint: &str,
    completed_turn_sync: bool,
) -> ReserveBindingReceipt {
    let selection = store
        .memory_provider_status(agent, None, 100)
        .unwrap()
        .selection;
    store
        .reserve_memory_binding(
            selection.revision,
            HostNewMemoryBinding {
                id: uid(),
                agent_id: agent.into(),
                endpoint: endpoint.into(),
                credential_id: Some(uid()),
                policy: MemoryPolicy {
                    recall: true,
                    explicit_retain: true,
                    completed_turn_sync,
                },
                plugin_id: "mewu.memory-hindsight".into(),
                contribution_id: "hindsight".into(),
                plugin_revision: 1,
            },
        )
        .unwrap()
}
fn pending(store: &Store, operation_id: &str) -> PendingMemoryWork {
    store
        .pending_memory_work(tick(), 16)
        .unwrap()
        .into_iter()
        .find(|p| p.operation_id == operation_id)
        .unwrap()
}
fn claim(store: &mut Store, operation_id: &str) -> MemoryDispatch {
    let p = pending(store, operation_id);
    store
        .claim_memory_work(&p.operation_id, p.revision, &p.grant, tick())
        .unwrap()
        .unwrap()
}
fn ready(store: &mut Store, receipt: ReserveBindingReceipt) -> MemoryBindingView {
    let work = claim(store, &receipt.operation_id);
    assert!(matches!(work.action, MemoryWorkAction::CreateBank));
    let bank_id = work.route.bank_id;
    assert_eq!(
        bank_id,
        format!(
            "mewu-{}",
            Uuid::parse_str(&receipt.binding.id).unwrap().simple()
        )
    );
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::BankReady {
                bank_id,
                service_version: "0.10.3".into(),
                observations_enabled: false,
            },
            tick(),
        )
        .unwrap();
    let view = binding(store, &receipt.binding.agent_id, &receipt.binding.id);
    assert_eq!(view.status, BindingStatus::Ready);
    let selection = store
        .memory_provider_status(&view.agent_id, None, 100)
        .unwrap()
        .selection;
    store
        .select_memory_provider(
            &view.agent_id,
            selection.revision,
            Some(&view.id),
            Some(view.revision),
        )
        .unwrap();
    binding(store, &view.agent_id, &view.id)
}
fn setup(store: &mut Store, agent: &str, completed_turn_sync: bool) -> MemoryBindingView {
    let reserved = reserve(
        store,
        agent,
        "https://memory.example.test",
        completed_turn_sync,
    );
    ready(store, reserved)
}
fn queue(store: &mut Store, b: &MemoryBindingView, request: &str, text: &str) -> WriteReceipt {
    store
        .queue_manual_memory_evidence(&b.agent_id, &b.id, b.revision, request, text)
        .unwrap()
}
fn start(store: &mut Store, scene: &str, b: &MemoryBindingView, text: &str) -> RunContext {
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.into(),
            draft: text.into(),
        })
        .unwrap();
    store.begin_run_with_memory(scene, Some(grant(b))).unwrap()
}
fn commit(store: &mut Store, b: &MemoryBindingView, text: &str) -> (String, String) {
    let receipt = queue(store, b, &uid(), text);
    let work = claim(store, receipt.operation_id.as_deref().unwrap());
    let (document, operation) = match work.action {
        MemoryWorkAction::Retain {
            document_id,
            operation_id,
            attributed_text,
            payload_sha256,
        } => {
            assert!(attributed_text.contains(text));
            assert_eq!(payload_sha256.len(), 64);
            (document_id, operation_id)
        }
        _ => panic!("new evidence must dispatch Retain"),
    };
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::RetainAccepted {
                bank_id: work.route.bank_id,
                operation_id: operation.clone(),
            },
            tick(),
        )
        .unwrap();
    let poll = claim(store, &operation);
    assert!(matches!(poll.action, MemoryWorkAction::PollRetain { .. }));
    store
        .record_memory_work(
            poll.lease,
            MemoryWorkObservation::Operation {
                bank_id: poll.route.bank_id,
                operation_id: operation,
                state: RemoteOperationState::Completed,
            },
            tick(),
        )
        .unwrap();
    assert_eq!(
        store
            .external_memory_entry(&b.agent_id, &b.id, &receipt.evidence_id)
            .unwrap()
            .unwrap()
            .summary
            .state,
        EvidenceState::Committed
    );
    (receipt.evidence_id, document)
}
fn counts(db: &Database) -> (i64, i64, i64, i64) {
    db.query_row("SELECT (SELECT count(*) FROM external_memory_bindings),(SELECT count(*) FROM external_memory_evidence),(SELECT count(*) FROM external_memory_outbox),(SELECT count(*) FROM external_memory_tombstones)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
fn payload(db: &Database) -> String {
    db.query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
        .unwrap()
}
fn revision(db: &Database) -> i64 {
    db.query_row("SELECT revision FROM app_state WHERE id=1", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn assert_not_claimed(result: Result<Option<MemoryDispatch>, CoreError>) {
    assert!(
        !matches!(result, Ok(Some(_))),
        "revoked or stale work must not dispatch"
    );
}

#[test]
fn manual_evidence_and_outbox_are_atomic_and_request_identity_is_idempotent() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let db = Database::open(&temp.0).unwrap();
    let before = counts(&db);
    let before_revision = revision(&db);
    let request = uid();
    let first = queue(&mut store, &b, &request, "明确保存的事实");
    let once = counts(&db);
    let committed_revision = revision(&db);
    let duplicate = queue(&mut store, &b, &request, "明确保存的事实");
    assert_eq!(first.evidence_id, duplicate.evidence_id);
    assert_eq!(first.operation_id, duplicate.operation_id);
    assert_eq!(once, (before.0, before.1 + 1, before.2 + 1, before.3));
    assert_eq!(counts(&db), once);
    assert_eq!(revision(&db), committed_revision);
    assert!(committed_revision > before_revision);
    assert!(store
        .queue_manual_memory_evidence(&agent, &b.id, b.revision, &request, "同请求不同内容")
        .is_err());
    assert_eq!(counts(&db), once);

    db.execute_batch("CREATE TRIGGER reject_external_outbox BEFORE INSERT ON external_memory_outbox BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let snapshot = store.snapshot();
    let failed_request = uid();
    assert!(store
        .queue_manual_memory_evidence(&agent, &b.id, b.revision, &failed_request, "失败后可重试")
        .is_err());
    assert_eq!(counts(&db), once);
    assert_eq!(revision(&db), committed_revision);
    assert_eq!(store.snapshot(), snapshot);
    db.execute_batch("DROP TRIGGER reject_external_outbox")
        .unwrap();
    queue(&mut store, &b, &failed_request, "失败后可重试");
    assert_eq!(counts(&db).1, once.1 + 1);
    assert_eq!(counts(&db).2, once.2 + 1);
}

#[test]
fn completed_turn_and_intent_commit_together_and_retry_does_not_duplicate_answer() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, true);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &b, "用户本轮证据");
    assert!(matches!(
        run.memory_authority,
        Some(RunMemoryAuthority::External {
            completed_turn_sync: true,
            ..
        })
    ));
    let db = Database::open(&temp.0).unwrap();
    let before = store.snapshot();
    let before_counts = counts(&db);
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_completed_evidence BEFORE INSERT ON external_memory_evidence BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store.finish_run(&scene, &run.run_id, "最终回答").is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(payload(&db), before_payload);
    assert_eq!(counts(&db), before_counts);
    assert!(store.run_is_active(&scene, &run.run_id));
    db.execute_batch("DROP TRIGGER reject_completed_evidence")
        .unwrap();
    store.finish_run(&scene, &run.run_id, "最终回答").unwrap();
    let page = store
        .external_memory_page(&agent, &b.id, "", None, 25)
        .unwrap();
    assert_eq!(page.total, 1);
    let entry = &page.entries[0];
    assert_eq!(entry.source.origin, EvidenceOrigin::CompletedTurn);
    assert_eq!(entry.source.run_id.as_deref(), Some(run.run_id.as_str()));
    assert!(entry.source.user_message_id.is_some());
    assert!(entry.source.assistant_message_id.is_some());
    let detail = store
        .external_memory_entry(&agent, &b.id, &entry.id)
        .unwrap()
        .unwrap();
    let text = detail.attributed_text.unwrap();
    assert!(text.contains("用户本轮证据"));
    assert!(text.contains("最终回答"));
    assert!(store.finish_run(&scene, &run.run_id, "重复的回答").is_err());
    assert_eq!(
        store
            .external_memory_page(&agent, &b.id, "", None, 25)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count(),
        1
    );
}

#[test]
fn workflow_does_not_capture_provider_authority_or_automatically_retain() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, true);
    let scene = store.snapshot().active_scene_id;
    let region = uid();
    store
        .set_background(
            &scene,
            Asset {
                id: uid(),
                kind: AssetKind::Image,
                name: "synthetic.png".into(),
                path: "synthetic.png".into(),
                width: Some(100),
                height: Some(80),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            },
        )
        .unwrap();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene.clone(),
            region: Region {
                id: region.clone(),
                x: 1.0,
                y: 1.0,
                width: 90.0,
                height: 60.0,
                ..Default::default()
            },
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "保持原草稿".into(),
        })
        .unwrap();
    let run = store
        .begin_workflow(&scene, "图片工作流提示", &region)
        .unwrap();
    assert!(run.memory_authority.is_none());
    assert!(run.memories.is_empty());
    assert!(!run.agent.memory_enabled);
    assert!(store.memories_for_run(&scene, &run.run_id).is_err());
    assert!(store
        .search_history_for_run(&scene, &run.run_id, "图片")
        .is_err());
    store.finish_run(&scene, &run.run_id, "工作流回答").unwrap();
    assert_eq!(
        store
            .external_memory_page(&agent, &b.id, "", None, 25)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .unwrap()
            .draft,
        "保持原草稿"
    );
}

#[test]
fn selected_external_without_native_grant_never_falls_back_to_local_memory() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: None,
            text: "隐藏本地事实，精确查询".into(),
            expected_revision: None,
        })
        .unwrap();
    let b = setup(&mut store, &agent, false);
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "精确查询".into(),
        })
        .unwrap();
    let run = store.begin_run_with_memory(&scene, None).unwrap();
    assert!(run.memories.is_empty());
    assert!(run.memory_authority.is_none());
    assert!(store
        .memory_route_for_run(&scene, &run.run_id, &grant(&b))
        .is_err());
    store.cancel_run(&scene).unwrap();
    let run = start(&mut store, &scene, &b, "精确查询");
    assert!(run.memories.is_empty());
    assert!(matches!(
        run.memory_authority,
        Some(RunMemoryAuthority::External { .. })
    ));
}

#[test]
fn policy_revocation_cancels_frozen_run_and_never_reauthorizes_old_queued_work() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, true);
    let receipt = queue(&mut store, &b, &uid(), "撤权前排队");
    let p = pending(&store, receipt.operation_id.as_deref().unwrap());
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &b, "后台问题");
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: scene.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    store
        .set_memory_binding_policy(
            &b.id,
            b.revision,
            b.plugin_revision,
            false,
            b.policy.clone(),
        )
        .unwrap();
    assert!(!store.run_is_active(&scene, &run.run_id));
    assert_eq!(store.snapshot().active_scene_id, active);
    assert!(store.finish_run(&scene, &run.run_id, "迟到回答").is_err());
    assert_not_claimed(store.claim_memory_work(&p.operation_id, p.revision, &p.grant, tick()));
    let disabled = binding(&store, &agent, &b.id);
    store
        .set_memory_binding_policy(
            &b.id,
            disabled.revision,
            b.plugin_revision,
            true,
            b.policy.clone(),
        )
        .unwrap();
    let enabled = binding(&store, &agent, &b.id);
    assert_not_claimed(store.claim_memory_work(
        &p.operation_id,
        p.revision,
        &grant(&enabled),
        tick(),
    ));
    let entry = store
        .external_memory_entry(&agent, &b.id, &receipt.evidence_id)
        .unwrap()
        .unwrap();
    assert_eq!(entry.summary.state, EvidenceState::Blocked);
}

#[test]
fn wrong_plugin_identity_and_stale_claim_cannot_dispatch_twice() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let receipt = queue(&mut store, &b, &uid(), "单次派发");
    let p = pending(&store, receipt.operation_id.as_deref().unwrap());
    let mut wrong = p.grant.clone();
    wrong.plugin_id = "other.memory-provider".into();
    assert_not_claimed(store.claim_memory_work(&p.operation_id, p.revision, &wrong, tick()));
    wrong = p.grant.clone();
    wrong.contribution_id = "other".into();
    assert_not_claimed(store.claim_memory_work(&p.operation_id, p.revision, &wrong, tick()));
    let work = store
        .claim_memory_work(&p.operation_id, p.revision, &p.grant, tick())
        .unwrap()
        .unwrap();
    assert_not_claimed(store.claim_memory_work(&p.operation_id, p.revision, &p.grant, tick()));
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::MutationOutcomeUnknown {
                reason: MemoryReason::OutcomeUnknown,
            },
            tick(),
        )
        .unwrap();
    let retry = claim(&mut store, &p.operation_id);
    assert!(matches!(retry.action, MemoryWorkAction::PollRetain { .. }));
}

#[test]
fn failed_revocation_keeps_selection_run_and_queued_authority_unchanged() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let written = queue(&mut store, &b, &uid(), "撤权事务失败不能半丢队列");
    let p = pending(&store, written.operation_id.as_deref().unwrap());
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &b, "继续运行");
    let state = store.snapshot();
    let db = Database::open(&temp.0).unwrap();
    let before = counts(&db);
    let before_revision = revision(&db);
    let before_payload = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_memory_revoke BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store
        .set_memory_binding_policy(
            &b.id,
            b.revision,
            b.plugin_revision,
            false,
            b.policy.clone()
        )
        .is_err());
    assert_eq!(store.snapshot(), state);
    assert_eq!(payload(&db), before_payload);
    assert_eq!(revision(&db), before_revision);
    assert_eq!(counts(&db), before);
    let live = binding(&store, &agent, &b.id);
    assert!(live.enabled);
    assert_eq!(live.revision, b.revision);
    assert!(store.run_is_active(&scene, &run.run_id));
    db.execute_batch("DROP TRIGGER reject_memory_revoke")
        .unwrap();
    assert!(store
        .claim_memory_work(&p.operation_id, p.revision, &p.grant, tick())
        .unwrap()
        .is_some());
}

#[test]
fn late_ack_after_forget_and_retirement_is_audited_without_resurrecting_evidence() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let written = queue(&mut store, &b, &uid(), "不再允许使用的证据");
    let work = claim(&mut store, written.operation_id.as_deref().unwrap());
    let operation = match &work.action {
        MemoryWorkAction::Retain { operation_id, .. } => operation_id.clone(),
        _ => panic!(),
    };
    let entry = store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap();
    store
        .forget_memory_evidence(&agent, &b.id, &written.evidence_id, entry.summary.revision)
        .unwrap();
    store.retire_memory_binding(&b.id, b.revision).unwrap();
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::RetainAccepted {
                bank_id: work.route.bank_id,
                operation_id: operation,
            },
            tick(),
        )
        .unwrap();
    let entry = store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap();
    assert_eq!(entry.summary.state, EvidenceState::Suppressed);
    assert!(entry.attributed_text.is_none());
    let forgotten = entry.summary.forgotten.unwrap();
    assert!(forgotten.local_suppressed);
    assert_eq!(forgotten.quiescence, Quiescence::Unproven);
    assert_eq!(
        forgotten.remote_state,
        RemoteDeleteState::NeedsAuthorization
    );
    assert_eq!(
        binding(&store, &agent, &b.id).status,
        BindingStatus::Retired
    );
    assert!(store
        .memory_provider_status(&agent, None, 100)
        .unwrap()
        .selection
        .binding_id
        .is_none());
}

#[test]
fn restart_recovers_dispatched_retain_only_as_poll_with_same_route_and_operation() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let receipt = queue(&mut store, &b, &uid(), "已可能送达但无回执");
    let operation = receipt.operation_id.unwrap();
    let work = claim(&mut store, &operation);
    let document = match &work.action {
        MemoryWorkAction::Retain { document_id, .. } => document_id.clone(),
        _ => panic!(),
    };
    let bank = work.route.bank_id.clone();
    let credential = work.route.credential_id.clone();
    drop(work);
    drop(store);
    let mut reopened = Store::open(&temp.0).unwrap();
    reopened.recover_memory_work().unwrap();
    let recovered = claim(&mut reopened, &operation);
    match recovered.action {
        MemoryWorkAction::PollRetain {
            document_id,
            operation_id,
        } => {
            assert_eq!(document_id, document);
            assert_eq!(operation_id, operation)
        }
        _ => panic!("uncertain retain must never POST again"),
    }
    assert_eq!(recovered.route.bank_id, bank);
    assert_eq!(recovered.route.credential_id, credential);
    assert_eq!(
        reopened
            .external_memory_page(&agent, &b.id, "", None, 25)
            .unwrap()
            .total,
        1
    );
}

#[test]
fn restart_recovers_bank_creation_only_as_inspection_and_missing_is_not_permission_to_put() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let receipt = reserve(&mut store, &agent, "https://memory.example.test", false);
    let operation = receipt.operation_id.clone();
    let work = claim(&mut store, &operation);
    assert!(matches!(work.action, MemoryWorkAction::CreateBank));
    drop(work);
    drop(store);
    let mut reopened = Store::open(&temp.0).unwrap();
    reopened.recover_memory_work().unwrap();
    let work = claim(&mut reopened, &operation);
    assert!(matches!(work.action, MemoryWorkAction::InspectBank));
    reopened
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::BankAbsent {
                bank_id: work.route.bank_id,
            },
            tick(),
        )
        .unwrap();
    for p in reopened.pending_memory_work(tick(), 16).unwrap() {
        if p.operation_id == operation {
            if let Some(work) = reopened
                .claim_memory_work(&p.operation_id, p.revision, &p.grant, tick())
                .unwrap()
            {
                assert!(!matches!(work.action, MemoryWorkAction::CreateBank));
            }
        }
    }
    assert_ne!(
        binding(&reopened, &agent, &receipt.binding.id).status,
        BindingStatus::Ready
    );
}

#[test]
fn cancelled_unknown_retain_and_delete_404_never_remove_tombstone_or_claim_quiescence() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let request = uid();
    let written = queue(&mut store, &b, &request, "即将遗忘的证据");
    let operation = written.operation_id.clone().unwrap();
    let work = claim(&mut store, &operation);
    let document = match &work.action {
        MemoryWorkAction::Retain { document_id, .. } => document_id.clone(),
        _ => panic!(),
    };
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::MutationOutcomeUnknown {
                reason: MemoryReason::OutcomeUnknown,
            },
            tick(),
        )
        .unwrap();
    let entry = store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap();
    let forgotten = store
        .forget_memory_evidence(&agent, &b.id, &written.evidence_id, entry.summary.revision)
        .unwrap();
    assert!(forgotten.local_suppressed);
    assert_eq!(forgotten.quiescence, Quiescence::Unproven);
    for _ in 0..8 {
        let Some(p) = store
            .pending_memory_work(tick(), 16)
            .unwrap()
            .into_iter()
            .next()
        else {
            break;
        };
        let Some(work) = store
            .claim_memory_work(&p.operation_id, p.revision, &p.grant, tick())
            .unwrap()
        else {
            continue;
        };
        let observation = match work.action {
            MemoryWorkAction::CancelRetain { operation_id, .. } => {
                MemoryWorkObservation::CancelIntentObserved {
                    bank_id: work.route.bank_id,
                    operation_id,
                }
            }
            MemoryWorkAction::PollRetain { operation_id, .. } => MemoryWorkObservation::Operation {
                bank_id: work.route.bank_id,
                operation_id,
                state: RemoteOperationState::Cancelled,
            },
            MemoryWorkAction::DeleteDocument { document_id } => {
                MemoryWorkObservation::DocumentDeleteObserved {
                    bank_id: work.route.bank_id,
                    document_id,
                    absent: true,
                }
            }
            MemoryWorkAction::InspectDocument { document_id } => {
                MemoryWorkObservation::DocumentObserved {
                    bank_id: work.route.bank_id,
                    document_id,
                    present: false,
                }
            }
            _ => panic!("forget must not create new bank or resend retained evidence"),
        };
        store
            .record_memory_work(work.lease, observation, tick())
            .unwrap();
    }
    let detail = store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap();
    assert!(detail.attributed_text.is_none());
    let forgotten = detail.summary.forgotten.unwrap();
    assert!(forgotten.local_suppressed);
    assert_eq!(forgotten.quiescence, Quiescence::Unproven);
    assert_ne!(forgotten.remote_state, RemoteDeleteState::ConfirmedDeleted);
    let duplicate = queue(&mut store, &b, &request, "即将遗忘的证据");
    assert_eq!(duplicate.evidence_id, written.evidence_id);
    assert_eq!(duplicate.state, EvidenceState::Suppressed);
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &b, "新问题");
    assert!(store
        .allowed_memory_documents_for_run(&scene, &run.run_id, &grant(&b), &[document])
        .unwrap()
        .is_empty());
}

#[test]
fn recall_only_allows_same_agent_committed_evidence_and_forget_invalidates_old_run() {
    let mut store = Store::open_in_memory().unwrap();
    let a = enable(&mut store);
    let a_binding = setup(&mut store, &a, false);
    let (evidence_a, document_a) = commit(&mut store, &a_binding, "身份A的事实");
    let mut b_agent = store.snapshot().agents[0].clone();
    b_agent.id = uid();
    b_agent.name = "身份 B".into();
    b_agent.memory_provider = MemoryProviderSelection::default();
    let b = b_agent.id.clone();
    store
        .apply(SceneCommand::SaveAgent { agent: b_agent })
        .unwrap();
    let b_binding = setup(&mut store, &b, false);
    let (_, document_b) = commit(&mut store, &b_binding, "身份B的事实");
    assert!(store
        .external_memory_page(&b, &a_binding.id, "", None, 25)
        .is_err());
    assert!(store
        .external_memory_entry(&b, &a_binding.id, &evidence_a)
        .is_err());
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &a_binding, "身份A检索");
    let docs = store
        .allowed_memory_documents_for_run(
            &scene,
            &run.run_id,
            &grant(&a_binding),
            &[document_a.clone(), document_b, uid()],
        )
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].evidence_id, evidence_a);
    let entry = store
        .external_memory_entry(&a, &a_binding.id, &evidence_a)
        .unwrap()
        .unwrap();
    store
        .forget_memory_evidence(&a, &a_binding.id, &evidence_a, entry.summary.revision)
        .unwrap();
    assert!(!store.run_is_active(&scene, &run.run_id));
    assert!(store
        .allowed_memory_documents_for_run(
            &scene,
            &run.run_id,
            &grant(&a_binding),
            &[document_a.clone()]
        )
        .is_err());
    let next = start(&mut store, &scene, &a_binding, "遗忘后的新问题");
    assert!(store
        .allowed_memory_documents_for_run(&scene, &next.run_id, &grant(&a_binding), &[document_a])
        .unwrap()
        .is_empty());
}

#[test]
fn evidence_paging_and_export_include_more_than_64_records_at_one_consistent_revision() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    for i in 0..75 {
        queue(&mut store, &b, &uid(), &format!("分页证据 {i:03}"));
    }
    let export = ExportView::open(&temp.0).unwrap();
    let first = export
        .external_memory_page(&agent, &b.id, "", None, 25)
        .unwrap();
    assert_eq!(first.total, 75);
    let stale = store
        .external_memory_page(&agent, &b.id, "", None, 25)
        .unwrap();
    queue(&mut store, &b, &uid(), "导出开始后新增");
    assert!(store
        .external_memory_page(&agent, &b.id, "", stale.next_cursor.as_deref(), 25)
        .is_err());
    let mut ids = HashSet::new();
    let mut cursor = None;
    let mut revision_seen = None;
    loop {
        let page = export
            .external_memory_page(&agent, &b.id, "", cursor.as_deref(), 25)
            .unwrap();
        assert_eq!(page.total, 75);
        if let Some(revision) = revision_seen {
            assert_eq!(page.revision, revision)
        } else {
            revision_seen = Some(page.revision)
        };
        for entry in page.entries {
            assert!(ids.insert(entry.id.clone()));
            assert!(export
                .external_memory_entry(&agent, &b.id, &entry.id)
                .unwrap()
                .unwrap()
                .attributed_text
                .unwrap()
                .contains("分页证据"));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 75);
    assert_eq!(
        store
            .external_memory_page(&agent, &b.id, "", None, 100)
            .unwrap()
            .total,
        76
    );
    assert!(export.snapshot().memories.is_empty());
    let public =
        serde_json::to_value(export.memory_provider_status(&agent, None, 100).unwrap()).unwrap();
    assert!(public["bindings"][0].get("credentialId").is_none());
    assert!(public["bindings"][0].get("bankId").is_none());
}

#[test]
fn changing_binding_does_not_migrate_old_queue_endpoint_bank_or_credential() {
    let temp = TestDb::new();
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let a = setup(&mut store, &agent, false);
    let old = queue(&mut store, &a, &uid(), "旧服务的排队证据");
    let old_pending = pending(&store, old.operation_id.as_deref().unwrap());
    let db = Database::open(&temp.0).unwrap();
    let route = |db: &Database, id: &str| {
        db.query_row(
            "SELECT endpoint,bank_id,credential_id FROM external_memory_bindings WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap()
    };
    let original_route = route(&db, &a.id);
    let reserved = reserve(
        &mut store,
        &agent,
        "https://replacement.example.test",
        false,
    );
    let b = ready(&mut store, reserved);
    assert_eq!(route(&db, &a.id), original_route);
    assert_ne!(route(&db, &b.id).1, original_route.1);
    assert_ne!(route(&db, &b.id).2, original_route.2);
    assert_not_claimed(store.claim_memory_work(
        &old_pending.operation_id,
        old_pending.revision,
        &grant(&b),
        tick(),
    ));
    assert_not_claimed(store.claim_memory_work(
        &old_pending.operation_id,
        old_pending.revision,
        &old_pending.grant,
        tick(),
    ));
    assert_eq!(
        store
            .external_memory_page(&agent, &a.id, "", None, 25)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .external_memory_page(&agent, &b.id, "", None, 25)
            .unwrap()
            .total,
        0
    );
    let bound: String = db
        .query_row(
            "SELECT binding_id FROM external_memory_outbox WHERE id=?1",
            [old.operation_id.unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bound, a.id);
    let selection = store
        .memory_provider_status(&agent, None, 100)
        .unwrap()
        .selection;
    let mut stale_agent = store.snapshot().agents[0].clone();
    stale_agent.memory_provider = MemoryProviderSelection::default();
    assert!(store
        .apply(SceneCommand::SaveAgent { agent: stale_agent })
        .is_err());
    assert_eq!(
        store
            .memory_provider_status(&agent, None, 100)
            .unwrap()
            .selection,
        selection
    );
}

#[test]
fn full_queue_blocks_automatic_memory_without_losing_completed_answer() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, true);
    for i in 0..128 {
        queue(&mut store, &b, &uid(), &format!("尚未派发 {i}"));
    }
    assert!(store
        .queue_manual_memory_evidence(
            &agent,
            &b.id,
            b.revision,
            &uid(),
            "显式超额应保持输入并报错"
        )
        .is_err());
    let scene = store.snapshot().active_scene_id;
    let run = start(&mut store, &scene, &b, "队列已满时的问题");
    store
        .finish_run(&scene, &run.run_id, "回答必须保存")
        .unwrap();
    let state = store.snapshot();
    let target = state.scenes.iter().find(|s| s.id == scene).unwrap();
    assert_eq!(target.messages.last().unwrap().text, "回答必须保存");
    assert_eq!(target.run.as_ref().unwrap().status, RunStatus::Completed);
    let mut cursor = None;
    let mut blocked = None;
    loop {
        let page = store
            .external_memory_page(&agent, &b.id, "", cursor.as_deref(), 100)
            .unwrap();
        for item in page.entries {
            if item.source.run_id.as_deref() == Some(run.run_id.as_str()) {
                blocked = Some(item);
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let blocked = blocked.unwrap();
    assert_eq!(blocked.state, EvidenceState::Blocked);
    assert!(matches!(blocked.reason, Some(MemoryReason::QueueFull)));
}

#[test]
fn retired_and_forgotten_unknown_payloads_release_active_budget_without_resending() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let old = setup(&mut store, &agent, false);
    let original = queue(&mut store, &old, &uid(), "旧来源已经可能送达");
    let work = claim(&mut store, original.operation_id.as_deref().unwrap());
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::MutationOutcomeUnknown {
                reason: MemoryReason::OutcomeUnknown,
            },
            tick(),
        )
        .unwrap();
    for i in 1..128 {
        queue(&mut store, &old, &uid(), &format!("旧待发 {i}"));
    }
    assert!(store
        .queue_manual_memory_evidence(&agent, &old.id, old.revision, &uid(), "超过活跃预算")
        .is_err());
    store.retire_memory_binding(&old.id, old.revision).unwrap();
    let retired = store
        .external_memory_entry(&agent, &old.id, &original.evidence_id)
        .unwrap()
        .unwrap();
    assert_eq!(retired.summary.state, EvidenceState::Unknown);
    assert!(retired
        .attributed_text
        .unwrap()
        .contains("旧来源已经可能送达"));
    assert!(store
        .pending_memory_work(tick(), 32)
        .unwrap()
        .iter()
        .all(|p| p.grant.binding_id != old.id));

    let current = setup(&mut store, &agent, false);
    let forgotten = queue(&mut store, &current, &uid(), "新来源将要遗忘的未知写入");
    let work = claim(&mut store, forgotten.operation_id.as_deref().unwrap());
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::MutationOutcomeUnknown {
                reason: MemoryReason::OutcomeUnknown,
            },
            tick(),
        )
        .unwrap();
    for i in 1..128 {
        queue(&mut store, &current, &uid(), &format!("新待发 {i}"));
    }
    let evidence = store
        .external_memory_entry(&agent, &current.id, &forgotten.evidence_id)
        .unwrap()
        .unwrap();
    let deletion = store
        .forget_memory_evidence(
            &agent,
            &current.id,
            &forgotten.evidence_id,
            evidence.summary.revision,
        )
        .unwrap();
    assert_eq!(deletion.quiescence, Quiescence::Unproven);
    queue(&mut store, &current, &uid(), "遗忘释放的一个外发位置");
    assert!(store
        .queue_manual_memory_evidence(
            &agent,
            &current.id,
            current.revision,
            &uid(),
            "仍然只能128个活跃请求"
        )
        .is_err());
    let detail = store
        .external_memory_entry(&agent, &current.id, &forgotten.evidence_id)
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.state, EvidenceState::Suppressed);
    assert!(detail.attributed_text.is_none());
    assert_eq!(
        store
            .external_memory_page(&agent, &old.id, "", None, 100)
            .unwrap()
            .total,
        128
    );
    assert_eq!(
        store
            .external_memory_page(&agent, &current.id, "", None, 100)
            .unwrap()
            .total,
        129
    );
}

fn make_db3(temp: &TestDb) -> (String, String, Value) {
    let mut store = Store::open(&temp.0).unwrap();
    let agent = enable(&mut store);
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SaveMemory {
            agent_id: agent.clone(),
            id: None,
            text: "旧库本地记忆不可重导或丢失".into(),
            expected_revision: None,
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "旧库用户消息".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    store
        .finish_run(&scene, &run.run_id, "旧库最终回复")
        .unwrap();
    drop(store);
    let db = Database::open(&temp.0).unwrap();
    let mut value: Value = serde_json::from_str(&payload(&db)).unwrap();
    for agent in value["agents"].as_array_mut().unwrap() {
        agent.as_object_mut().unwrap().remove("memoryProvider");
    }
    for scene in value["scenes"].as_array_mut().unwrap() {
        if let Some(run) = scene.get_mut("run").and_then(Value::as_object_mut) {
            run.remove("memoryAuthority");
        }
    }
    db.execute(
        "UPDATE app_state SET payload=?1",
        [serde_json::to_string(&value).unwrap()],
    )
    .unwrap();
    db.execute_batch("DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;DROP TABLE visual_run_inputs;DROP TABLE agent_run_events; DROP TABLE agent_run_records; DROP TABLE external_memory_tombstones;DROP TABLE external_memory_outbox;DROP TABLE external_memory_evidence;DROP TABLE external_memory_bindings;PRAGMA user_version=3;").unwrap();
    (agent, scene, value)
}

#[test]
fn db3_migration_preserves_local_memory_history_and_defaults_without_reimporting_connections() {
    let temp = TestDb::new();
    let (agent, scene, old) = make_db3(&temp);
    let store = Store::open(&temp.0).unwrap();
    let state = store.snapshot();
    assert_eq!(
        state.agents[0].memory_provider,
        MemoryProviderSelection::default()
    );
    assert!(state
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .unwrap()
        .run
        .as_ref()
        .unwrap()
        .memory_authority
        .is_none());
    assert_eq!(
        serde_json::to_value(&state.connections).unwrap(),
        old["connections"]
    );
    assert_eq!(
        state
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .unwrap()
            .messages
            .last()
            .unwrap()
            .text,
        "旧库最终回复"
    );
    assert_eq!(store.memory_page(&agent, "", None, 25).unwrap().total, 1);
    assert!(store
        .memory_provider_status(&agent, None, 100)
        .unwrap()
        .bindings
        .is_empty());
    let db = Database::open(&temp.0).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        9
    );
    assert_eq!(counts(&db), (0, 0, 0, 0));
    drop(store);
    let reopened = Store::open(&temp.0).unwrap();
    assert_eq!(reopened.snapshot(), state);
    assert_eq!(reopened.memory_page(&agent, "", None, 25).unwrap().total, 1);
}

#[test]
fn migration_failure_rolls_back_and_corrupt_db3_catalog_is_not_recreated_from_legacy_projection() {
    let temp = TestDb::new();
    let (agent, _, _) = make_db3(&temp);
    let db = Database::open(&temp.0).unwrap();
    let before = payload(&db);
    db.execute_batch("CREATE TRIGGER reject_memory_migration BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(Store::open(&temp.0).is_err());
    assert_eq!(payload(&db), before);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    let tables:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='external_memory_bindings'",[],|r|r.get(0)).unwrap();
    assert_eq!(tables, 0);
    db.execute_batch("DROP TRIGGER reject_memory_migration")
        .unwrap();
    let mut bad: Value = serde_json::from_str(&before).unwrap();
    bad.as_object_mut().unwrap().remove("connections");
    bad.as_object_mut().unwrap().remove("defaultConnectionId");
    let bad = serde_json::to_string(&bad).unwrap();
    db.execute("UPDATE app_state SET payload=?1", params![bad])
        .unwrap();
    assert!(Store::open(&temp.0).is_err());
    assert_eq!(payload(&db), bad);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    db.execute("UPDATE app_state SET payload=?1", [before])
        .unwrap();
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(store.memory_page(&agent, "", None, 25).unwrap().total, 1);
}

#[test]
fn claimed_retain_loses_live_authority_but_late_receipt_remains_auditable() {
    for change in ["forget", "selection", "plugin_revision"] {
        let mut store = Store::open_in_memory().unwrap();
        let agent = enable(&mut store);
        let b = setup(&mut store, &agent, false);
        let written = queue(&mut store, &b, &uid(), "待处理的合成事实");
        let work = claim(&mut store, written.operation_id.as_deref().unwrap());
        assert!(store
            .memory_work_is_authorized(&work.lease, &work.grant)
            .unwrap());
        match change {
            "forget" => {
                store
                    .forget_memory_evidence(&agent, &b.id, &written.evidence_id, written.revision)
                    .unwrap();
            }
            "selection" => {
                let other = setup(&mut store, &agent, false);
                assert_ne!(other.id, b.id);
            }
            "plugin_revision" => {
                store
                    .set_memory_binding_policy(
                        &b.id,
                        b.revision,
                        b.plugin_revision + 1,
                        true,
                        b.policy.clone(),
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            !store
                .memory_work_is_authorized(&work.lease, &work.grant)
                .unwrap(),
            "{change}"
        );
        store
            .record_memory_work(
                work.lease,
                MemoryWorkObservation::RetainAccepted {
                    bank_id: work.route.bank_id,
                    operation_id: written.operation_id.unwrap(),
                },
                tick(),
            )
            .unwrap();
        let evidence = store
            .external_memory_entry(&agent, &b.id, &written.evidence_id)
            .unwrap()
            .unwrap();
        if change == "forget" {
            assert_eq!(evidence.summary.state, EvidenceState::Suppressed);
            assert!(evidence.attributed_text.is_none());
        } else {
            assert_eq!(evidence.summary.state, EvidenceState::Accepted);
        }
    }
}

#[test]
fn failed_poll_dispatch_cannot_prove_the_original_retain_never_reached_server() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let written = queue(&mut store, &b, &uid(), "结果未知的远程记忆");
    let op = written.operation_id.as_deref().unwrap();
    let work = claim(&mut store, op);
    store
        .record_memory_work(
            work.lease,
            MemoryWorkObservation::MutationOutcomeUnknown {
                reason: MemoryReason::OutcomeUnknown,
            },
            tick(),
        )
        .unwrap();
    let poll = claim(&mut store, op);
    assert!(matches!(poll.action, MemoryWorkAction::PollRetain { .. }));
    store
        .record_memory_work(
            poll.lease,
            MemoryWorkObservation::NotDispatched {
                reason: MemoryReason::CancelledBeforeDispatch,
            },
            tick(),
        )
        .unwrap();
    let entry = store
        .external_memory_entry(&agent, &b.id, &written.evidence_id)
        .unwrap()
        .unwrap();
    assert_eq!(entry.summary.state, EvidenceState::Unknown);
    let forgotten = store
        .forget_memory_evidence(&agent, &b.id, &written.evidence_id, entry.summary.revision)
        .unwrap();
    assert_eq!(forgotten.quiescence, Quiescence::Unproven);
    assert_ne!(forgotten.remote_state, RemoteDeleteState::ConfirmedDeleted);
}

#[test]
fn corrupt_db4_action_kind_and_source_reference_are_rejected_without_overwrite() {
    for corruption in ["action_kind", "source"] {
        let temp = TestDb::new();
        let mut store = Store::open(&temp.0).unwrap();
        let agent = enable(&mut store);
        let b = setup(&mut store, &agent, false);
        let written = queue(&mut store, &b, &uid(), "保持原账本");
        drop(store);
        let db = Database::open(&temp.0).unwrap();
        let before = payload(&db);
        let revision_before = revision(&db);
        let original: String = db
            .query_row(
                "SELECT source_json FROM external_memory_evidence WHERE id=?1",
                [&written.evidence_id],
                |r| r.get(0),
            )
            .unwrap();
        if corruption == "action_kind" {
            // Both strings are valid SQL enum values; their combination is not.
            db.execute(
                "UPDATE external_memory_outbox SET action='inspect_bank' WHERE id=?1",
                [written.operation_id.as_deref().unwrap()],
            )
            .unwrap();
        } else {
            let mut source: Value = serde_json::from_str(&original).unwrap();
            source["origin"] = Value::String("user_quote".into());
            source["sceneId"] = Value::String(uid());
            source["runId"] = Value::String(uid());
            source["userMessageId"] = Value::String(uid());
            db.execute(
                "UPDATE external_memory_evidence SET source_json=?1 WHERE id=?2",
                params![serde_json::to_string(&source).unwrap(), written.evidence_id],
            )
            .unwrap();
        }
        assert!(Store::open(&temp.0).is_err(), "{corruption}");
        assert_eq!(payload(&db), before);
        assert_eq!(revision(&db), revision_before);
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            9
        );
        // Repair only our injected corruption; the valid database remains readable.
        db.execute(
            "UPDATE external_memory_outbox SET action='retain' WHERE id=?1",
            [written.operation_id.as_deref().unwrap()],
        )
        .unwrap();
        db.execute(
            "UPDATE external_memory_evidence SET source_json=?1 WHERE id=?2",
            params![original, written.evidence_id],
        )
        .unwrap();
        let recovered = Store::open(&temp.0).unwrap();
        assert!(recovered
            .external_memory_entry(&agent, &b.id, &written.evidence_id)
            .unwrap()
            .is_some());
    }
}

#[test]
fn external_provider_keeps_same_agent_history_search_without_local_memory_fallback() {
    let mut store = Store::open_in_memory().unwrap();
    let agent = enable(&mut store);
    let b = setup(&mut store, &agent, false);
    let original = store.snapshot().active_scene_id;
    let first = start(&mut store, &original, &b, "上次讨论青瓷花瓶");
    store
        .finish_run(&original, &first.run_id, "青瓷花瓶是此测试的历史内容")
        .unwrap();
    store.apply(SceneCommand::NewScene).unwrap();
    let next = store.snapshot().active_scene_id;
    let current = start(&mut store, &next, &b, "检索先前讨论");
    let hits = store
        .search_history_for_run(&next, &current.run_id, "青瓷花瓶")
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.scene_id == original));
    assert!(store
        .recall_for_run(&next, &current.run_id, "青瓷花瓶")
        .is_err());
    assert!(store.memories_for_run(&next, &current.run_id).is_err());
    let selection = store
        .memory_provider_status(&agent, None, 100)
        .unwrap()
        .selection;
    store
        .select_memory_provider(&agent, selection.revision, None, None)
        .unwrap();
    assert!(store
        .search_history_for_run(&next, &current.run_id, "青瓷花瓶")
        .is_err());
}
