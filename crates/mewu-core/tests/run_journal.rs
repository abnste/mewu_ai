// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mewu-journal-{}.sqlite", uuid::Uuid::new_v4())))
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
    }
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn start(s: &mut Store, text: &str) -> RunContext {
    let id = s.snapshot().active_scene_id;
    s.apply(SceneCommand::SetDraft {
        scene_id: id.clone(),
        draft: text.into(),
    })
    .unwrap();
    s.begin_run(&id).unwrap()
}
fn model(s: &mut Store, c: &RunContext, round: u32) -> DispatchLease {
    s.begin_model_request(
        &c.scene_id,
        &c.run_id,
        ModelDispatchInput {
            round,
            projected_request_sha256: hash("public request"),
        },
    )
    .unwrap()
}
fn call(name: &str) -> ProposedTool {
    ProposedTool {
        call_id: format!("call-{name}"),
        binding: ToolBinding::Local { name: name.into() },
        arguments_wire_sha256: hash("{}"),
    }
}
fn turn(s: &mut Store, c: &RunContext, text: &str, calls: Vec<ProposedTool>) -> TurnCommit {
    let m = model(s, c, 0);
    s.record_model_turn(
        &m,
        CompletePublicTurn {
            text: text.into(),
            calls,
        },
    )
    .unwrap()
}
fn result(text: &str) -> ObservedToolResult {
    let value = json!({"text":text});
    let raw = value.to_string();
    ObservedToolResult {
        response_kind: ToolResponseKind::Complete,
        returned_error: Some(false),
        observed_encoding: "fixture-sdk-json-v1".into(),
        observed_bytes: raw.len() as u64,
        observed_sha256: hash(&raw),
        retained_content: JournalContent::Json { value },
        references: vec![],
        model_visible_json: Some(raw),
    }
}
fn page(s: &Store, c: &RunContext) -> JournalPage {
    s.run_journal_page(&c.scene_id, &c.run_id, None, 25)
        .unwrap()
        .unwrap()
}
fn continue_args(s: &Store, c: &RunContext, selection: Option<Vec<String>>) -> ContinueFromRecord {
    let p = page(s, c);
    let conn = s.connection_for_scene(&c.scene_id).unwrap();
    ContinueFromRecord {
        scene_id: c.scene_id.clone(),
        source_run_id: p.continuation.source_run_id,
        expected_journal_revision: p.continuation.journal_revision,
        expected_checkpoint_seq: p.continuation.checkpoint_seq,
        expected_connection_id: conn.id,
        expected_connection_revision: conn.revision,
        selected_event_ids: selection,
    }
}
fn revision(db: &Connection) -> i64 {
    db.query_row("SELECT revision FROM app_state WHERE id=1", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn extended_workflow_rounds_survive_restart_and_still_reject_the_limit() {
    let file=Db::new(); let mut store=Store::open(&file.0).unwrap();
    let context=start(&mut store,"多步骤插件开发");
    for round in 0..MAX_AGENT_MODEL_ROUNDS {
        let lease=model(&mut store,&context,round);
        store.record_model_turn(&lease,CompletePublicTurn{text:format!("步骤 {round}"),calls:vec![]}).unwrap();
    }
    assert!(store.begin_model_request(&context.scene_id,&context.run_id,ModelDispatchInput{round:MAX_AGENT_MODEL_ROUNDS,projected_request_sha256:hash("bounded")}).is_err());
    store.fail_run(&context.scene_id,&context.run_id,"已保留草稿").unwrap(); drop(store);
    let store=Store::open(&file.0).unwrap();
    assert_eq!(page(&store,&context).entries.len(),MAX_AGENT_MODEL_ROUNDS as usize);
}

#[test]
fn not_sent_reason_is_read_only_detail_metadata_and_survives_restart() {
    let file = Db::new();
    let reasons = [
        NotSentReason::PreparationFailed,
        NotSentReason::InvalidArguments,
        NotSentReason::NotAdvertised,
        NotSentReason::AuthorityChanged,
        NotSentReason::CatalogChanged,
        NotSentReason::RunEnded,
        NotSentReason::BudgetExhausted,
        NotSentReason::DuplicateCallId,
    ];
    let mut store = Store::open(&file.0).unwrap();
    let context = start(&mut store, "查看未执行原因");
    let calls = (0..reasons.len())
        .map(|index| call(&format!("reason-{index}")))
        .collect();
    let committed = turn(&mut store, &context, "准备工具", calls);
    for (tool, reason) in committed.tools.iter().zip(&reasons) {
        store
            .record_tool_not_sent(&tool.lease, reason.clone())
            .unwrap();
    }
    // A model turn may advertise at most eight calls. Use a second real turn
    // for the unknown-result control rather than weakening the store's budget.
    let next_model = model(&mut store, &context, 1);
    let unknown_call = store
        .record_model_turn(
            &next_model,
            CompletePublicTurn {
                text: "核对未知结果".into(),
                calls: vec![call("unknown-control")],
            },
        )
        .unwrap();
    store
        .mark_tool_dispatched(&unknown_call.tools[0].lease)
        .unwrap();
    store
        .fail_run(&context.scene_id, &context.run_id, "合成中断")
        .unwrap();
    drop(store);
    let store = Store::open(&file.0).unwrap();
    let db = Connection::open(&file.0).unwrap();
    let snapshot = || -> (i64, Vec<(String, String)>, i64) {
        let rows = db
            .prepare("SELECT metadata,content FROM agent_run_events ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        (
            revision(&db),
            rows,
            db.query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap(),
        )
    };
    let before = snapshot();
    let page = page(&store, &context);
    for (index, reason) in reasons.iter().enumerate() {
        let summary = &page.entries[index + 1];
        let detail = store
            .run_journal_entry(
                &context.scene_id,
                &context.run_id,
                &summary.id,
                page.summary.revision,
            )
            .unwrap();
        assert_eq!(detail.not_sent_reason.as_ref(), Some(reason));
        let wire = serde_json::to_value(&detail).unwrap();
        assert_eq!(wire["notSentReason"], serde_json::to_value(reason).unwrap());
        assert!(serde_json::to_value(summary)
            .unwrap()
            .get("notSentReason")
            .is_none());
        let mut older_wire = wire;
        older_wire.as_object_mut().unwrap().remove("notSentReason");
        let older: JournalEntryDetail = serde_json::from_value(older_wire).unwrap();
        assert_eq!(older.not_sent_reason, None);
    }
    let unknown = page.entries.last().unwrap();
    assert_eq!(unknown.phase, ReceiptPhase::Unknown);
    let detail = store
        .run_journal_entry(
            &context.scene_id,
            &context.run_id,
            &unknown.id,
            page.summary.revision,
        )
        .unwrap();
    assert_eq!(detail.not_sent_reason, None);
    assert!(serde_json::to_value(detail)
        .unwrap()
        .get("notSentReason")
        .is_none());
    assert_eq!(snapshot(), before);
}

fn annotation_call(video: bool, id: &str) -> ProposedTool {
    let binding = if video {
        ToolBinding::VideoAnnotation {
            plugin_id: "mewu.annotations".into(),
            plugin_revision: 1,
            contribution_id: "answer".into(),
            name: VIDEO_ANNOTATION_TOOL.into(),
            target_manifest_sha256: hash("synthetic targets"),
        }
    } else {
        ToolBinding::VisualAnnotation {
            plugin_id: "mewu.annotations".into(),
            plugin_revision: 1,
            contribution_id: "answer".into(),
            name: VISUAL_ANNOTATION_TOOL.into(),
            target_manifest_sha256: hash("synthetic targets"),
        }
    };
    ProposedTool {
        call_id: id.into(),
        binding,
        arguments_wire_sha256: hash("PRIVATE_RAW_MUST_NOT_BE_STORED"),
    }
}

#[test]
fn safe_argument_diagnostics_persist_without_raw_values_or_historical_backfill() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let context = start(&mut store, "合成诊断");
    let committed = turn(
        &mut store,
        &context,
        "",
        vec![
            annotation_call(true, "video"),
            annotation_call(false, "image"),
            annotation_call(true, "legacy"),
        ],
    );
    let diagnostics = [
        ArgumentDiagnostic::new(
            "$/objects/0/primitive/style/opacity".into(),
            ArgumentDiagnosticReason::Type,
            Some(ArgumentExpectedType::Number),
        )
        .unwrap(),
        ArgumentDiagnostic::new(
            "$/groups/0/objects/0/fontSize".into(),
            ArgumentDiagnosticReason::Required,
            None,
        )
        .unwrap(),
    ];
    let db = Connection::open(&file.0).unwrap();
    for (tool, diagnostic) in committed.tools.iter().zip(&diagnostics) {
        let first = store
            .record_tool_not_sent_with_diagnostic(&tool.lease, diagnostic)
            .unwrap();
        let before = revision(&db);
        let again = store
            .record_tool_not_sent_with_diagnostic(&tool.lease, diagnostic)
            .unwrap();
        assert_eq!(first, again);
        assert_eq!(revision(&db), before);
        let changed =
            ArgumentDiagnostic::new("$".into(), ArgumentDiagnosticReason::Required, None).unwrap();
        assert!(store
            .record_tool_not_sent_with_diagnostic(&tool.lease, &changed)
            .is_err());
        assert_eq!(revision(&db), before);
    }
    store
        .record_tool_not_sent(&committed.tools[2].lease, NotSentReason::InvalidArguments)
        .unwrap();
    let before = revision(&db);
    assert!(
        store
            .record_tool_not_sent_with_diagnostic(&committed.tools[2].lease, &diagnostics[0])
            .is_err(),
        "may not invent a missing historical cause"
    );
    assert_eq!(revision(&db), before);
    store
        .fail_run(&context.scene_id, &context.run_id, "合成结束")
        .unwrap();
    drop(store);
    let store = Store::open(&file.0).unwrap();
    let rows = || {
        db.prepare("SELECT metadata,content FROM agent_run_events ORDER BY sequence")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = (revision(&db), rows());
    let p = page(&store, &context);
    for (index, wanted) in diagnostics
        .iter()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        let summary = &p.entries[index + 1];
        let detail = store
            .run_journal_entry(
                &context.scene_id,
                &context.run_id,
                &summary.id,
                p.summary.revision,
            )
            .unwrap();
        assert_eq!(detail.argument_diagnostic.as_ref(), wanted);
        assert_eq!(
            detail.not_sent_reason,
            Some(NotSentReason::InvalidArguments)
        );
        assert!(matches!(
            detail.content,
            JournalContent::Omitted {
                reason: ContentUnavailable::NotReceived
            }
        ));
        assert!(serde_json::to_value(summary)
            .unwrap()
            .get("argumentDiagnostic")
            .is_none());
        let mut wire = serde_json::to_value(detail).unwrap();
        wire.as_object_mut().unwrap().remove("argumentDiagnostic");
        assert_eq!(
            serde_json::from_value::<JournalEntryDetail>(wire)
                .unwrap()
                .argument_diagnostic,
            None
        );
    }
    assert_eq!(
        (revision(&db), rows()),
        before,
        "detail reads changed storage"
    );
    assert!(!serde_json::to_string(&before.1)
        .unwrap()
        .contains("PRIVATE_RAW_MUST_NOT_BE_STORED"));
}

#[test]
fn argument_diagnostics_reject_unknown_fields_unbounded_indices_and_wrong_bindings() {
    for path in [
        "",
        "/objects/0",
        "$/PRIVATE_UNKNOWN_KEY",
        "$/objects/48",
        "$/groups/6",
        "$/objects/00",
        "$/objects/-1",
        "$/objects/0/secret",
        "$/text/0",
        "$/rows/199",
        "$/rows/0/32",
        "$/rows/0/0/0",
    ] {
        assert!(
            ArgumentDiagnostic::new(
                path.into(),
                ArgumentDiagnosticReason::Type,
                Some(ArgumentExpectedType::Number)
            )
            .is_none(),
            "accepted {path}"
        );
    }
    assert!(ArgumentDiagnostic::new(
        "$/objects/47/primitive/bounds/x".into(),
        ArgumentDiagnosticReason::Minimum,
        None
    )
    .is_some());
    assert!(ArgumentDiagnostic::new(
        "$/groups/5/objects/47/table/rows/198/31".into(),
        ArgumentDiagnosticReason::Type,
        Some(ArgumentExpectedType::String)
    )
    .is_some());
    assert!(ArgumentDiagnostic::new(
        "$".into(),
        ArgumentDiagnosticReason::Required,
        Some(ArgumentExpectedType::Number)
    )
    .is_none());
    let mut store = Store::open_in_memory().unwrap();
    let context = start(&mut store, "错误绑定");
    let committed = turn(&mut store, &context, "", vec![call("local")]);
    let diagnostic =
        ArgumentDiagnostic::new("$".into(), ArgumentDiagnosticReason::Syntax, None).unwrap();
    let before = store.snapshot();
    assert!(store
        .record_tool_not_sent_with_diagnostic(&committed.tools[0].lease, &diagnostic)
        .is_err());
    assert_eq!(store.snapshot(), before);
    assert_eq!(
        page(&store, &context).entries[1].phase,
        ReceiptPhase::Prepared
    );
}

#[test]
fn diagnostic_receipt_failure_and_corrupt_metadata_never_expose_or_commit_values() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let context = start(&mut store, "原子诊断");
    let committed = turn(
        &mut store,
        &context,
        "",
        vec![annotation_call(true, "video")],
    );
    let diagnostic = ArgumentDiagnostic::new(
        "$/objects/0/primitive/fontSize".into(),
        ArgumentDiagnosticReason::Required,
        None,
    )
    .unwrap();
    let db = Connection::open(&file.0).unwrap();
    let before = (store.snapshot(), revision(&db));
    db.execute_batch("CREATE TRIGGER reject_safe_diagnostic BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'synthetic failure');END").unwrap();
    assert!(store
        .record_tool_not_sent_with_diagnostic(&committed.tools[0].lease, &diagnostic)
        .is_err());
    assert_eq!((store.snapshot(), revision(&db)), before);
    assert_eq!(
        page(&store, &context).entries[1].phase,
        ReceiptPhase::Prepared
    );
    db.execute_batch("DROP TRIGGER reject_safe_diagnostic")
        .unwrap();
    store
        .record_tool_not_sent_with_diagnostic(&committed.tools[0].lease, &diagnostic)
        .unwrap();
    store
        .fail_run(&context.scene_id, &context.run_id, "合成结束")
        .unwrap();
    let p = page(&store, &context);
    let id = &p.entries[1].id;
    let original: String = db
        .query_row(
            "SELECT metadata FROM agent_run_events WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let mut invalid: serde_json::Value = serde_json::from_str(&original).unwrap();
    invalid["argumentDiagnostic"]["fieldPath"] = json!("$/PRIVATE_CORRUPT_VALUE");
    let corrupted = invalid.to_string();
    db.execute(
        "UPDATE agent_run_events SET metadata=?1 WHERE id=?2",
        [&corrupted, id],
    )
    .unwrap();
    assert!(store
        .run_journal_entry(&context.scene_id, &context.run_id, id, p.summary.revision)
        .is_err());
    drop(store);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT metadata FROM agent_run_events WHERE id=?1",
            [id],
            |row| row.get(0)
        )
        .unwrap(),
        corrupted,
        "corruption was overwritten"
    );
}

#[test]
fn conversation_boundary_preserves_old_journal_projection_but_blocks_its_continuation() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let first = start(&mut s, "第一会话原问题");
    turn(&mut s, &first, "第一已观察正文", vec![]);
    s.fail_run(&first.scene_id, &first.run_id, "合成中断")
        .unwrap();
    let old_page = page(&s, &first);
    let old_args = continue_args(&s, &first, None);
    s.apply(SceneCommand::NewConversation {
        scene_id: first.scene_id.clone(),
    })
    .unwrap();
    let after = page(&s, &first);
    assert_eq!(after.entries, old_page.entries);
    assert_eq!(
        after.continuation.projection_bytes,
        old_page.continuation.projection_bytes
    );
    assert_eq!(
        after.summary.continue_blocked_reason,
        Some(ContinueBlockedReason::ConversationChanged)
    );
    assert!(!after.continuation.ready);
    let before = s.snapshot();
    assert!(s.begin_recorded_continuation(old_args).is_err());
    assert_eq!(s.snapshot(), before);
    let second = start(&mut s, "第二会话原问题");
    assert_eq!(second.messages.len(), 1);
    turn(&mut s, &second, "第二已观察正文", vec![]);
    s.fail_run(&second.scene_id, &second.run_id, "合成中断")
        .unwrap();
    let before_second = page(&s, &second);
    let continuation = s
        .begin_recorded_continuation(continue_args(&s, &second, None))
        .unwrap();
    assert_eq!(
        continuation
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["第二会话原问题", "继续回答"]
    );
    let RunProjection::RecordedContinuation(projection) = &continuation.projection else {
        panic!("recorded projection")
    };
    assert_eq!(
        projection
            .original_messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["第二会话原问题"]
    );
    s.fail_run(&continuation.scene_id, &continuation.run_id, "合成中断")
        .unwrap();
    s.apply(SceneCommand::NewConversation {
        scene_id: first.scene_id.clone(),
    })
    .unwrap();
    drop(s);
    let mut s = Store::open(&file.0).unwrap();
    let after_second = page(&s, &second);
    assert_eq!(
        after_second.continuation.projection_bytes,
        before_second.continuation.projection_bytes
    );
    assert_eq!(
        after_second.summary.continue_blocked_reason,
        Some(ContinueBlockedReason::ConversationChanged)
    );
    assert_eq!(
        page(&s, &first).continuation.projection_bytes,
        old_page.continuation.projection_bytes
    );
    let third = start(&mut s, "第三会话原问题");
    assert_eq!(third.messages.len(), 1);
    assert_eq!(third.messages[0].text, "第三会话原问题");
    assert_eq!(third.messages[0].conversation_start, Some(3));
}

#[test]
fn confirmed_body_and_unknown_receipt_survive_restart_without_replaying() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let c = start(&mut s, "记录工具结果");
    let t = turn(
        &mut s,
        &c,
        "开始检查",
        vec![call("one"), call("two"), call("three")],
    );
    s.mark_tool_dispatched(&t.tools[0].lease).unwrap();
    s.record_tool_response(&t.tools[0].lease, result("actual response"))
        .unwrap();
    s.mark_tool_dispatched(&t.tools[1].lease).unwrap();
    drop(s);
    let s = Store::open(&file.0).unwrap();
    let p = page(&s, &c);
    assert_eq!(p.summary.status, RecordedRunStatus::Interrupted);
    assert_eq!(p.summary.checkpoint_seq, 4);
    assert_eq!(p.entries[1].phase, ReceiptPhase::Responded);
    assert_eq!(p.entries[2].phase, ReceiptPhase::Unknown);
    assert_eq!(p.entries[3].phase, ReceiptPhase::NotSent);
    let d = s
        .run_journal_entry(&c.scene_id, &c.run_id, &p.entries[1].id, p.summary.revision)
        .unwrap();
    assert_eq!(d.content, result("actual response").retained_content);
    let snap = s.snapshot();
    assert_eq!(
        snap.scenes[0].messages[0].tool_steps[1].status,
        ToolStepStatus::Unknown
    );
    assert!(!serde_json::to_string(&snap)
        .unwrap()
        .contains("actual response"));
    let db = Connection::open(&file.0).unwrap();
    let before = revision(&db);
    drop(s);
    drop(Store::open(&file.0).unwrap());
    assert_eq!(revision(&db), before);
}

#[test]
fn local_effect_receipt_and_step_are_one_sqlite_commit() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let mut agent = s.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    s.apply(SceneCommand::SaveAgent {
        agent: agent.clone(),
    })
    .unwrap();
    let c = start(&mut s, "我喜欢中文回答");
    let t = turn(&mut s, &c, "", vec![call("memory_save")]);
    let db = Connection::open(&file.0).unwrap();
    let before = s.snapshot();
    let rev = revision(&db);
    db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    let effect = LocalMemoryEffect::Save {
        id: None,
        expected_revision: None,
        source_quote: "喜欢中文".into(),
        source_message_id: None,
    };
    assert!(serde_json::from_value::<LocalMemoryEffect>(json!({
        "type":"save", "id":null, "expectedRevision":null,
        "sourceQuote":"喜欢中文", "text":"unrelated tool-generated claim"
    })).is_err(), "a valid citation cannot authorize a separate invented fact");
    assert!(s
        .apply_local_memory_with_receipt(&t.tools[0].lease, effect.clone())
        .is_err());
    assert_eq!(s.snapshot(), before);
    assert_eq!(revision(&db), rev);
    assert_eq!(s.memory_page(&agent.id, "", None, 25).unwrap().total, 0);
    db.execute_batch("DROP TRIGGER fail_receipt").unwrap();
    let commit = s
        .apply_local_memory_with_receipt(&t.tools[0].lease, effect)
        .unwrap();
    assert_eq!(revision(&db), rev + 1);
    assert_eq!(s.memory_page(&agent.id, "", None, 25).unwrap().total, 1);
    assert_eq!(
        commit.snapshot.scenes[0].run.as_ref().unwrap().steps[0].status,
        ToolStepStatus::Completed
    );
    assert_eq!(page(&s, &c).entries[1].phase, ReceiptPhase::Responded);
}

#[test]
fn memory_follow_up_binds_the_exact_prior_user_source_and_survives_reopen() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    store.apply(SceneCommand::SaveAgent { agent: agent.clone() }).unwrap();
    let first = start(&mut store, "我是stephen");
    let source = first.messages.last().unwrap().id.clone();
    store.finish_run(&first.scene_id, &first.run_id, "你好").unwrap();
    let follow_up = start(&mut store, "记着啊");
    let proposal = turn(&mut store, &follow_up, "", vec![call("memory_save")]);
    store.apply_local_memory_with_receipt(&proposal.tools[0].lease, LocalMemoryEffect::Save {
        id: None, expected_revision: None,
        source_quote: "我是stephen".into(), source_message_id: Some(source.clone()),
    }).unwrap();
    let saved = store.memory_page(&agent.id, "", None, 25).unwrap().entries.remove(0);
    assert_eq!(saved.text, "我是stephen");
    assert_eq!(saved.source, Some(MemorySource { scene_id: first.scene_id.clone(), message_id: source }));
    assert_ne!(saved.source.as_ref().unwrap().message_id, follow_up.messages.last().unwrap().id);
    assert_eq!(page(&store, &follow_up).entries[1].phase, ReceiptPhase::Responded);
    store.finish_run(&follow_up.scene_id, &follow_up.run_id, "已记住").unwrap();
    drop(store);
    let reopened = Store::open(&file.0).unwrap();
    assert_eq!(reopened.memory_entry(&agent.id, &saved.id).unwrap(), Some(saved));
}

#[test]
fn memory_source_rejects_assistant_cross_scene_reset_and_unlabelled_old_messages() {
    let mut store = Store::open_in_memory().unwrap();
    let mut agent = store.snapshot().agents[0].clone();
    agent.memory_enabled = true;
    store.apply(SceneCommand::SaveAgent { agent: agent.clone() }).unwrap();
    let first = start(&mut store, "我是stephen");
    let source = first.messages.last().unwrap().id.clone();
    store.finish_run(&first.scene_id, &first.run_id, "我是stephen").unwrap();
    let assistant = store.snapshot().scenes.iter().find(|scene| scene.id == first.scene_id)
        .unwrap().messages.last().unwrap().id.clone();
    store.apply(SceneCommand::NewScene).unwrap();
    let other = start(&mut store, "我是stephen");
    let cross_scene = other.messages.last().unwrap().id.clone();
    store.finish_run(&other.scene_id, &other.run_id, "知道了").unwrap();
    store.apply(SceneCommand::ActivateScene { scene_id: first.scene_id.clone() }).unwrap();
    let follow_up = start(&mut store, "记着啊");
    let proposal = turn(&mut store, &follow_up, "", vec![call("memory_save")]);
    let effect = |source_message_id| LocalMemoryEffect::Save {
        id: None, expected_revision: None,
        source_quote: "我是stephen".into(), source_message_id,
    };
    for invalid in [None, Some(assistant), Some(cross_scene), Some(uuid::Uuid::new_v4().to_string())] {
        assert!(matches!(store.apply_local_memory_with_receipt(&proposal.tools[0].lease, effect(invalid)), Err(CoreError::MemorySourceRequired)));
    }
    store.fail_run(&follow_up.scene_id, &follow_up.run_id, "fixture").unwrap();
    store.apply(SceneCommand::NewConversation { scene_id: first.scene_id.clone() }).unwrap();
    let reset = start(&mut store, "记着啊");
    let proposal = turn(&mut store, &reset, "", vec![call("memory_save")]);
    assert!(matches!(store.apply_local_memory_with_receipt(&proposal.tools[0].lease, effect(Some(source.clone()))), Err(CoreError::MemorySourceRequired)));
    store.finish_run(&reset.scene_id, &reset.run_id, "fixture").unwrap();
    let recent = start(&mut store, "我是stephen");
    let recent_source = recent.messages.last().unwrap().id.clone();
    store.finish_run(&recent.scene_id, &recent.run_id, "知道了").unwrap();
    for _ in 0..LOCAL_MEMORY_SOURCE_MESSAGE_LIMIT {
        let run = start(&mut store, "继续");
        store.finish_run(&run.scene_id, &run.run_id, "继续").unwrap();
    }
    let old = start(&mut store, "记着啊");
    let proposal = turn(&mut store, &old, "", vec![call("memory_save")]);
    assert!(matches!(store.apply_local_memory_with_receipt(&proposal.tools[0].lease, effect(Some(recent_source))), Err(CoreError::MemorySourceRequired)));
    assert_eq!(store.memory_page(&agent.id, "", None, 25).unwrap().total, 0);
    assert_eq!(page(&store, &old).entries[1].phase, ReceiptPhase::Prepared);
}

#[test]
fn continuation_preserves_draft_refs_prefix_and_disables_all_tools() {
    let mut s = Store::open_in_memory().unwrap();
    let prior = start(&mut s, "先前约束：只中文");
    s.finish_run(&prior.scene_id, &prior.run_id, "知道了")
        .unwrap();
    let c = start(&mut s, "照前面的方式继续");
    turn(&mut s, &c, "确认的数据", vec![]);
    s.fail_run(&c.scene_id, &c.run_id, "网络断开").unwrap();
    s.apply(SceneCommand::SetDraft {
        scene_id: c.scene_id.clone(),
        draft: "这是下一轮草稿".into(),
    })
    .unwrap();
    let refs = s.snapshot().scenes[0].refs.clone();
    let args = continue_args(&s, &c, None);
    let continued = s.begin_recorded_continuation(args).unwrap();
    assert!(continued.mcp_servers.is_empty());
    assert!(continued.memory_authority.is_none());
    assert!(continued.memories.is_empty());
    assert!(!continued.agent.memory_enabled);
    let RunProjection::RecordedContinuation(p) = &continued.projection else {
        panic!()
    };
    assert_eq!(p.original_messages.len(), 3);
    assert_eq!(p.original_messages[0].text, "先前约束：只中文");
    assert_eq!(p.original_messages.last().unwrap().text, "照前面的方式继续");
    assert!(p.quoted_evidence_json.contains("确认的数据"));
    assert_eq!(s.snapshot().scenes[0].draft, "这是下一轮草稿");
    assert_eq!(s.snapshot().scenes[0].refs, refs);
    s.fail_run(&continued.scene_id, &continued.run_id, "再次中断")
        .unwrap();
    let retry = page(&s, &continued);
    assert_eq!(retry.continuation.source_run_id, c.run_id);
    assert!(retry.summary.can_continue);
    let again = s
        .begin_recorded_continuation(continue_args(
            &s,
            &continued,
            Some(retry.continuation.selected_event_ids),
        ))
        .unwrap();
    assert_ne!(again.run_id, continued.run_id);
}

#[test]
fn late_response_updates_only_original_record_and_invalidates_stale_selection() {
    let mut s = Store::open_in_memory().unwrap();
    let c = start(&mut s, "问题");
    let t = turn(&mut s, &c, "已确认部分", vec![call("remote")]);
    s.mark_tool_dispatched(&t.tools[0].lease).unwrap();
    s.cancel_run(&c.scene_id).unwrap();
    let args = continue_args(&s, &c, None);
    s.record_tool_response(&t.tools[0].lease, result("late observed"))
        .unwrap();
    assert!(matches!(
        s.begin_recorded_continuation(args),
        Err(CoreError::JournalConflict)
    ));
    let next = start(&mut s, "新问题");
    let active = s.snapshot().active_scene_id;
    s.record_tool_response(&t.tools[0].lease, result("late observed"))
        .unwrap();
    assert!(s.run_is_active(&next.scene_id, &next.run_id));
    assert_eq!(s.snapshot().active_scene_id, active);
    assert!(s
        .record_tool_response(&t.tools[0].lease, result("conflicting"))
        .is_err());
}

#[test]
fn oversized_and_sensitive_results_are_explicit_and_never_rehydrated() {
    let mut s = Store::open_in_memory().unwrap();
    let c = start(&mut s, "问题");
    let t = turn(
        &mut s,
        &c,
        "可用的回答",
        vec![call("memory_recall"), call("big")],
    );
    for (i, reason) in [
        ContentUnavailable::MemoryPolicy,
        ContentUnavailable::Oversized,
    ]
    .into_iter()
    .enumerate()
    {
        s.mark_tool_dispatched(&t.tools[i].lease).unwrap();
        let mut observed = result("private body must not persist");
        observed.retained_content = JournalContent::Omitted { reason };
        observed.model_visible_json = Some("ephemeral memory body".into());
        s.record_tool_response(&t.tools[i].lease, observed).unwrap();
    }
    s.fail_run(&c.scene_id, &c.run_id, "预算结束").unwrap();
    let p = page(&s, &c);
    assert_eq!(p.continuation.default_event_ids.len(), 1);
    let context = s
        .begin_recorded_continuation(continue_args(&s, &c, None))
        .unwrap();
    let RunProjection::RecordedContinuation(p) = context.projection else {
        panic!()
    };
    assert!(!p.quoted_evidence_json.contains("private body"));
    assert!(!p.quoted_evidence_json.contains("ephemeral memory"));
    assert!(p.quoted_evidence_json.contains("memory_policy"));
    assert!(p.quoted_evidence_json.contains("oversized"));
}

#[test]
fn budget_uses_actual_encoding_and_selects_only_when_needed() {
    let mut s = Store::open_in_memory().unwrap();
    let c = start(&mut s, "问题");
    let m = model(&mut s, &c, 0);
    s.record_model_turn(
        &m,
        CompletePublicTurn {
            text: "\"".repeat(150_000),
            calls: vec![],
        },
    )
    .unwrap();
    let m = model(&mut s, &c, 1);
    s.record_model_turn(
        &m,
        CompletePublicTurn {
            text: "简短确认".into(),
            calls: vec![],
        },
    )
    .unwrap();
    s.fail_run(&c.scene_id, &c.run_id, "中断").unwrap();
    let p = page(&s, &c);
    assert!(p.summary.can_continue);
    assert!(p.continuation.selection_required);
    assert!(!p.continuation.ready);
    let ids = vec![p.entries[1].id.clone()];
    let decision = s
        .preview_run_continuation(&ContinuationSelection {
            scene_id: c.scene_id.clone(),
            source_run_id: c.run_id.clone(),
            expected_journal_revision: p.summary.revision,
            expected_checkpoint_seq: p.summary.checkpoint_seq,
            selected_event_ids: Some(ids.clone()),
        })
        .unwrap();
    assert!(decision.ready);
    assert!(decision.selection_required);
    assert!(decision.projection_bytes < decision.budget_bytes);
    s.begin_recorded_continuation(continue_args(&s, &c, Some(ids)))
        .unwrap();
}

#[test]
fn migration_and_recovery_failures_do_not_partially_rewrite_existing_db() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let c = start(&mut s, "legacy");
    s.finish_run(&c.scene_id, &c.run_id, "旧回答").unwrap();
    drop(s);
    let db = Connection::open(&file.0).unwrap();
    db.execute_batch("DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;DROP TABLE drawing_layouts;DROP TABLE visual_run_inputs;DROP TABLE agent_run_events;DROP TABLE agent_run_records;PRAGMA user_version=4;CREATE TRIGGER reject_upgrade BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    let rev = revision(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(revision(&db), rev);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        4
    );
    db.execute_batch("DROP TRIGGER reject_upgrade").unwrap();
    let mut s = Store::open(&file.0).unwrap();
    assert!(s
        .run_journal_summary(&c.scene_id, &c.run_id)
        .unwrap()
        .is_none());
    let active = start(&mut s, "new");
    let lease = model(&mut s, &active, 0);
    drop(lease);
    drop(s);
    let rev = revision(&db);
    db.execute_batch("CREATE TRIGGER reject_recovery BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(Store::open(&file.0).is_err());
    assert_eq!(revision(&db), rev);
    let phase: String = db
        .query_row(
            "SELECT json_extract(metadata,'$.summary.phase') FROM agent_run_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(phase, "dispatched");
    db.execute_batch("DROP TRIGGER reject_recovery").unwrap();
    assert_eq!(
        page(&Store::open(&file.0).unwrap(), &active).entries[0].phase,
        ReceiptPhase::Unknown
    );
}

#[test]
fn stale_writer_cannot_dispatch_and_cross_scene_ids_are_rejected() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let c = start(&mut s, "问题");
    let t = turn(&mut s, &c, "", vec![call("one")]);
    let second = Store::open(&file.0).unwrap();
    assert!(matches!(
        s.mark_tool_dispatched(&t.tools[0].lease),
        Err(CoreError::ConcurrentModification)
    ));
    assert!(second
        .run_journal_page("wrong-scene", &c.run_id, None, 25)
        .is_err());
}

#[test]
fn run_ledger_keeps_more_than_fixed_history_pages_and_export_is_consistent() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let mut first = None;
    for i in 0..70 {
        let c = start(&mut s, &format!("问题{i}"));
        turn(&mut s, &c, &format!("结果{i}"), vec![]);
        s.finish_run(&c.scene_id, &c.run_id, format!("结果{i}"))
            .unwrap();
        if first.is_none() {
            first = Some(c)
        }
    }
    let first = first.unwrap();
    let export = ExportView::open(&file.0).unwrap();
    let mut cursor = None;
    let mut total = 0;
    loop {
        let p = export
            .scene_run_journal_page(&first.scene_id, cursor.as_deref(), 25)
            .unwrap();
        total += p.runs.len();
        cursor = p.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(total, 70);
    let later = start(&mut s, "later");
    s.finish_run(&later.scene_id, &later.run_id, "later")
        .unwrap();
    assert!(export
        .run_journal_summary(&later.scene_id, &later.run_id)
        .unwrap()
        .is_none());
    assert_eq!(page(&s, &first).entries[0].phase, ReceiptPhase::Responded);
}

#[test]
fn corrupted_receipt_body_is_rejected_without_overwriting_file() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let c = start(&mut s, "问题");
    turn(&mut s, &c, "正文", vec![]);
    s.finish_run(&c.scene_id, &c.run_id, "正文").unwrap();
    drop(s);
    let db = Connection::open(&file.0).unwrap();
    db.execute(
        "UPDATE agent_run_events SET content=?1",
        [json!({"type":"text","text":"篡改"}).to_string()],
    )
    .unwrap();
    let rev = revision(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(revision(&db), rev);
}

#[test]
fn continuation_provenance_cannot_reference_another_runs_evidence() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let a = start(&mut s, "甲");
    turn(&mut s, &a, "甲证据", vec![]);
    s.finish_run(&a.scene_id, &a.run_id, "甲回答").unwrap();
    let b = start(&mut s, "乙");
    turn(&mut s, &b, "乙证据", vec![]);
    s.fail_run(&b.scene_id, &b.run_id, "中断").unwrap();
    let foreign = page(&s, &a).entries[0].id.clone();
    let context = s
        .begin_recorded_continuation(continue_args(&s, &b, None))
        .unwrap();
    s.fail_run(&context.scene_id, &context.run_id, "中断")
        .unwrap();
    drop(s);
    let db = Connection::open(&file.0).unwrap();
    let raw: String = db
        .query_row(
            "SELECT data FROM agent_run_records WHERE id=?1",
            [&context.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut changed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    changed["provenance"]["selectedEventIds"] = json!([foreign]);
    db.execute(
        "UPDATE agent_run_records SET data=?1 WHERE id=?2",
        rusqlite::params![changed.to_string(), context.run_id],
    )
    .unwrap();
    let rev = revision(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(revision(&db), rev);
    db.execute(
        "UPDATE agent_run_records SET data=?1 WHERE id=?2",
        rusqlite::params![raw, context.run_id],
    )
    .unwrap();
    assert!(Store::open(&file.0).is_ok());
}

#[test]
fn continuation_original_ids_require_the_complete_exact_conversation_range_after_reset() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let old = start(&mut store, "以前会话");
    store
        .finish_run(&old.scene_id, &old.run_id, "旧回答")
        .unwrap();
    let old_id = old.messages[0].id.clone();
    store
        .apply(SceneCommand::NewConversation {
            scene_id: old.scene_id.clone(),
        })
        .unwrap();
    let first = start(&mut store, "当前会话第一问");
    store
        .finish_run(&first.scene_id, &first.run_id, "当前第一答")
        .unwrap();
    let second = start(&mut store, "当前会话第二问");
    turn(&mut store, &second, "当前证据", vec![]);
    store
        .fail_run(&second.scene_id, &second.run_id, "中断")
        .unwrap();
    let continued = store
        .begin_recorded_continuation(continue_args(&store, &second, None))
        .unwrap();
    store
        .fail_run(&continued.scene_id, &continued.run_id, "中断")
        .unwrap();
    drop(store);
    let db = Connection::open(&file.0).unwrap();
    let raw: String = db
        .query_row(
            "SELECT data FROM agent_run_records WHERE id=?1",
            [&continued.run_id],
            |row| row.get(0),
        )
        .unwrap();
    let original: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let ids = original["provenance"]["originalMessageIds"]
        .as_array()
        .unwrap();
    assert_eq!(ids.len(), 3);
    let mut crossed = vec![json!(old_id)];
    crossed.extend(ids.iter().cloned());
    for bad_ids in [
        ids[1..].to_vec(),
        ids[..2].to_vec(),
        crossed,
        ids.iter().rev().cloned().collect(),
    ] {
        let mut changed = original.clone();
        changed["provenance"]["originalMessageIds"] = json!(bad_ids);
        db.execute(
            "UPDATE agent_run_records SET data=?1 WHERE id=?2",
            rusqlite::params![changed.to_string(), continued.run_id],
        )
        .unwrap();
        let before = revision(&db);
        assert!(Store::open(&file.0).is_err());
        assert_eq!(revision(&db), before);
    }
    db.execute(
        "UPDATE agent_run_records SET data=?1 WHERE id=?2",
        rusqlite::params![raw, continued.run_id],
    )
    .unwrap();
    assert!(Store::open(&file.0).is_ok());
}

#[test]
fn metadata_projection_sizes_equal_full_encoding_and_preserve_provenance() {
    for text in [
        "汉字🙂\"引号\"\\路径\n\r\t\u{0001}\u{2028}".repeat(11),
        "\"\\".repeat(40_000),
    ] {
        let mut s = Store::open_in_memory().unwrap();
        let c = start(&mut s, "原问题：\"路径\\\"，🙂\n保留前缀");
        let t = turn(
            &mut s,
            &c,
            &text,
            vec![call("known"), call("unknown"), call("not_sent")],
        );
        s.mark_tool_dispatched(&t.tools[0].lease).unwrap();
        let mut observed = result("值\"\\🙂\n\t\u{0000}");
        observed.retained_content = JournalContent::Json {
            value: json!({"嵌套":[null,true,false,-0.0,{"quoted\\\"":"\u{0002}文本🙂"}]}),
        };
        s.record_tool_response(&t.tools[0].lease, observed).unwrap();
        s.mark_tool_dispatched(&t.tools[1].lease).unwrap();
        s.fail_run(&c.scene_id, &c.run_id, "合成中断").unwrap();
        let p = page(&s, &c);
        let original: Vec<RecordedMessage> = c
            .messages
            .iter()
            .map(|m| RecordedMessage {
                id: m.id.clone(),
                role: m.role.clone(),
                text: m.text.clone(),
                attachments: m.attachments.clone().unwrap_or_default(),
            })
            .collect();
        // Independent reconstruction of the former full-body budget path.
        let quote = |ids: &[String]| {
            let records: Vec<_> = p.entries.iter().map(|e| {
                let content = ids.contains(&e.id).then(|| s.run_journal_entry(
                    &c.scene_id, &c.run_id, &e.id, p.summary.revision,
                ).unwrap().content);
                json!({"id":e.id,"kind":e.kind,"phase":e.phase,"label":e.label,"responseKind":e.response_kind,"returnedError":e.returned_error,"contentUnavailable":e.content_unavailable,"contentOmitted":e.selectable&&!ids.contains(&e.id),"content":content})
            }).collect();
            json!({"version":1,"sourceRunId":c.run_id,"records":records}).to_string()
        };
        let size = |quoted: &str| {
            serde_json::to_vec(&json!({"originalMessages":original,"quotedEvidence":quoted,"continuationRequest":"根据以上已确认记录继续回答。工具不可用；不要声称未知操作已完成。"})).unwrap().len() as u64
        };
        let default_size = size(&quote(&p.continuation.default_event_ids));
        let empty_size = size(&quote(&[]));
        assert_eq!(p.continuation.default_projection_bytes, default_size);
        let first = p.entries[0].id.clone();
        let second = p.entries[1].id.clone();
        for ids in [
            None,
            Some(vec![]),
            Some(vec![first.clone()]),
            Some(vec![second.clone()]),
            Some(vec![second.clone(), first.clone()]),
        ] {
            let decision = s
                .preview_run_continuation(&ContinuationSelection {
                    scene_id: c.scene_id.clone(),
                    source_run_id: c.run_id.clone(),
                    expected_journal_revision: p.summary.revision,
                    expected_checkpoint_seq: p.summary.checkpoint_seq,
                    selected_event_ids: ids.clone(),
                })
                .unwrap();
            let selected = ids.as_deref().unwrap_or(&p.continuation.default_event_ids);
            assert_eq!(decision.projection_bytes, size(&quote(selected)));
            assert_eq!(decision.default_projection_bytes, default_size);
            assert_eq!(
                decision.selection_required,
                default_size > decision.budget_bytes
            );
            let reason = if empty_size > decision.budget_bytes {
                Some(ContinueBlockedReason::MandatoryContextTooLarge)
            } else if selected.is_empty() {
                Some(ContinueBlockedReason::NoConfirmedContent)
            } else if decision.projection_bytes > decision.budget_bytes {
                Some(ContinueBlockedReason::SelectionTooLarge)
            } else {
                None
            };
            assert_eq!(decision.blocked_reason, reason);
            assert_eq!(decision.ready, reason.is_none());
        }
        let selected = vec![second];
        let expected_quote = quote(&selected);
        let expected_hash =
            hash(&json!({"originalMessages":original,"quotedEvidence":expected_quote}).to_string());
        let resumed = s
            .begin_recorded_continuation(continue_args(&s, &c, Some(selected)))
            .unwrap();
        let RunProjection::RecordedContinuation(projection) = resumed.projection else {
            panic!("continuation")
        };
        assert_eq!(projection.quoted_evidence_json, expected_quote);
        assert_eq!(projection.provenance.projection_sha256, expected_hash);
        assert_eq!(projection.original_messages, original);
    }
}

#[test]
fn corrupted_derived_projection_size_is_rejected_without_rewriting_database() {
    let file = Db::new();
    let mut s = Store::open(&file.0).unwrap();
    let c = start(&mut s, "合成问题");
    turn(&mut s, &c, "已保存的 \"正文\\🙂", vec![]);
    s.fail_run(&c.scene_id, &c.run_id, "中断").unwrap();
    let event = page(&s, &c).entries[0].id.clone();
    drop(s);
    let db = Connection::open(&file.0).unwrap();
    let raw: String = db
        .query_row(
            "SELECT metadata FROM agent_run_events WHERE id=?1",
            [&event],
            |r| r.get(0),
        )
        .unwrap();
    let mut changed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    changed["projectionContentBytes"] = json!(4);
    db.execute(
        "UPDATE agent_run_events SET metadata=?1 WHERE id=?2",
        rusqlite::params![changed.to_string(), event],
    )
    .unwrap();
    let rev = revision(&db);
    assert!(Store::open(&file.0).is_err());
    assert_eq!(revision(&db), rev);
    db.execute(
        "UPDATE agent_run_events SET metadata=?1 WHERE id=?2",
        rusqlite::params![raw, event],
    )
    .unwrap();
    assert!(Store::open(&file.0).is_ok());
}

#[test]
fn cached_projection_budget_keeps_exact_256_kib_boundary() {
    let mut probe = Store::open_in_memory().unwrap();
    let source = start(&mut probe, "边界问题");
    turn(&mut probe, &source, "A", vec![]);
    probe
        .fail_run(&source.scene_id, &source.run_id, "中断")
        .unwrap();
    let base = page(&probe, &source).continuation;
    // UUIDs have fixed encoded length, so plain ASCII changes this envelope's
    // exact byte count one-for-one without depending on implementation metadata.
    let at_limit = (base.budget_bytes - base.projection_bytes + 1) as usize;
    for (length, delta) in [(at_limit - 1, -1i64), (at_limit, 0), (at_limit + 1, 1)] {
        let mut store = Store::open_in_memory().unwrap();
        let run = start(&mut store, "边界问题");
        turn(&mut store, &run, &"A".repeat(length), vec![]);
        store.fail_run(&run.scene_id, &run.run_id, "中断").unwrap();
        let decision = page(&store, &run).continuation;
        assert_eq!(
            decision.projection_bytes as i64,
            decision.budget_bytes as i64 + delta
        );
        assert_eq!(decision.ready, delta <= 0);
        assert_eq!(decision.selection_required, delta > 0);
        let before = store.snapshot();
        let result = store.begin_recorded_continuation(continue_args(&store, &run, None));
        if delta > 0 {
            assert!(result.is_err());
            assert_eq!(store.snapshot(), before);
        } else {
            let RunProjection::RecordedContinuation(projection) = result.unwrap().projection else {
                panic!("continuation")
            };
            let actual = serde_json::to_vec(&json!({"originalMessages":projection.original_messages,"quotedEvidence":projection.quoted_evidence_json,"continuationRequest":"根据以上已确认记录继续回答。工具不可用；不要声称未知操作已完成。"})).unwrap().len() as u64;
            assert_eq!(actual, decision.projection_bytes);
        }
    }
}
