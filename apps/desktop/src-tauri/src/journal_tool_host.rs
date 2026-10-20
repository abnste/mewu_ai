// SPDX-License-Identifier: MPL-2.0
//! Host-owned memory/history execution. Local effects and their receipts share
//! one Store transaction; retrieved text is transient and never journal body.
use crate::{
    agent_tools,
    ai::ModelToolCall,
    journal_runtime::{self, CommittedToolOutput},
    memory_host, memory_tools, Host,
};
use mewu_core::{
    ContentUnavailable, DispatchLease, EvidenceReference, JournalContent, LocalToolCommit,
    MemoryAuthorityGrant, NoResponseReason, NotSentReason, ObservedToolResult, RunMemoryAuthority,
    Store, ToolResponseKind,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
}

fn search(arguments: &str) -> Result<Search, String> {
    agent_tools::parse(arguments)
}

fn live(host: &Host, engine: &crate::Engine, scene: &str, run: &str) -> Result<(), String> {
    host.exit.ensure_background()?;
    if !crate::run_is_authorized(engine, scene, run, None) {
        return Err("运行已取消或结束".into());
    }
    Ok(())
}

fn read_observation(
    name: &str,
    value: Value,
    references: Vec<EvidenceReference>,
    failed: bool,
) -> Result<ObservedToolResult, String> {
    let body = serde_json::to_string(&value).map_err(|_| "工具结果格式无效")?;
    if body.len() > journal_runtime::MAX_TOOL_OUTPUT || references.len() > 64 {
        return Err("工具结果超过长度上限".into());
    }
    let reason = match name {
        "memory_recall" => ContentUnavailable::MemoryPolicy,
        "history_search" => ContentUnavailable::HistoryPolicy,
        _ => return Err("此工具不是只读检索".into()),
    };
    Ok(ObservedToolResult {
        response_kind: ToolResponseKind::LocalRead,
        returned_error: Some(failed),
        observed_encoding: "mewu-local-json-v1".into(),
        observed_bytes: body.len() as u64,
        observed_sha256: format!("{:x}", Sha256::digest(body.as_bytes())),
        retained_content: JournalContent::Omitted { reason },
        references,
        model_visible_json: Some(body),
    })
}

fn local_read(
    store: &Store,
    scene: &str,
    run: &str,
    name: &str,
    query: &str,
) -> Result<(Value, Vec<EvidenceReference>), String> {
    match name {
        "memory_recall" => {
            let entries = store
                .recall_for_run(scene, run, query)
                .map_err(agent_tools::domain_error)?;
            let refs = entries
                .iter()
                .map(|entry| EvidenceReference::LocalMemory {
                    id: entry.id.clone(),
                    revision: entry.revision,
                })
                .collect();
            Ok((json!({"memories":entries,"selection":"retrieved"}), refs))
        }
        "history_search" => {
            let entries = store
                .search_history_for_run(scene, run, query)
                .map_err(agent_tools::domain_error)?;
            let refs = entries
                .iter()
                .map(|entry| EvidenceReference::History {
                    scene_id: entry.scene_id.clone(),
                    message_id: entry.message_id.clone(),
                })
                .collect();
            Ok((json!({"matches":entries}), refs))
        }
        _ => Err("此工具不是只读检索".into()),
    }
}

// This is a native filtered result from memory_tools::recall, never arbitrary
// model/server JSON. It contains only evidence previously allowed by Store.
fn external_references(
    grant: &MemoryAuthorityGrant,
    result: &Value,
) -> Result<Vec<EvidenceReference>, String> {
    #[derive(Deserialize)]
    struct Identity {
        id: String,
        revision: u64,
    }
    let entries = result
        .get("memories")
        .and_then(Value::as_array)
        .ok_or("记忆结果无效")?;
    if entries.len() > 8 {
        return Err("记忆结果过多".into());
    }
    entries
        .iter()
        .map(|entry| {
            let identity: Identity =
                serde_json::from_value(entry.clone()).map_err(|_| "记忆结果无效")?;
            if uuid::Uuid::parse_str(&identity.id).is_err() || identity.revision == 0 {
                return Err("记忆来源无效".into());
            }
            Ok(EvidenceReference::ExternalMemory {
                binding_id: grant.binding_id.clone(),
                evidence_id: identity.id,
                operation_id: None,
                revision: identity.revision,
            })
        })
        .collect()
}

fn settle_read(
    store: &mut Store,
    lease: &DispatchLease,
    name: &str,
    result: Result<(Value, Vec<EvidenceReference>), String>,
) -> Result<(mewu_core::ReceiptCommit, CommittedToolOutput), String> {
    let (value, refs, failed) = match result {
        Ok((value, refs)) => (value, refs, false),
        Err(error) => (json!({"error":error}), vec![], true),
    };
    let observed = read_observation(name, value, refs, failed)?;
    let content = observed
        .model_visible_json
        .clone()
        .ok_or("工具结果不可用")?;
    let commit = store
        .record_tool_response(lease, observed)
        .map_err(journal_runtime::store_error)?;
    Ok((
        commit,
        CommittedToolOutput {
            model_json: content,
        },
    ))
}

enum LocalEffectOutcome {
    Applied(LocalToolCommit),
    Rejected(mewu_core::ReceiptCommit, CommittedToolOutput),
}

fn reject_arguments(
    store: &mut Store,
    lease: &DispatchLease,
    name: &str,
    error: String,
) -> Result<(mewu_core::ReceiptCommit, CommittedToolOutput), String> {
    // Preserve the actual rejection before returning feedback to the bounded
    // model loop. A malformed proposal is not a storage failure or a cancelled
    // run, and must not settle its sibling proposals as run_ended.
    let receipt = store
        .record_tool_not_sent(lease, NotSentReason::InvalidArguments)
        .map_err(journal_runtime::store_error)?;
    let correction = if matches!(name, "memory_recall" | "history_search") {
        "请按工具 schema 提供字符串 query，不添加其他字段；不要原样重复失败调用。"
    } else {
        "请按当前工具 schema 更正参数，只使用其中已声明的字段，并遵循该工具规定的用户原文来源范围；不要原样重复失败调用。修改前请检索当前记录的 id 和 revision。依据不足时向用户说明，不能声称已保存。"
    };
    Ok((receipt, CommittedToolOutput {
        model_json: json!({
            "isError":true,"error":error,"executed":false,
            "correction":correction
        }).to_string(),
    }))
}

fn local_effect(
    store: &mut Store,
    lease: &DispatchLease,
    name: &str,
    arguments: &str,
) -> Result<LocalEffectOutcome, String> {
    let effect = match agent_tools::recorded_local_effect(name, arguments) {
        Ok(effect) => effect,
        Err(error) => {
            let (receipt, output) = reject_arguments(store, lease, name, error)?;
            return Ok(LocalEffectOutcome::Rejected(receipt, output));
        }
    };
    match store.apply_local_memory_with_receipt(lease, effect) {
        Ok(result) => Ok(LocalEffectOutcome::Applied(result)),
        Err(error @ (mewu_core::CoreError::MemorySourceRequired | mewu_core::CoreError::MemoryConflict
            | mewu_core::CoreError::NotFound { kind: "记忆", .. })) => {
            let (receipt, output) = reject_arguments(store, lease, name, agent_tools::domain_error(error))?;
            Ok(LocalEffectOutcome::Rejected(receipt, output))
        }
        Err(error) => Err(agent_tools::domain_error(error)),
    }
}

fn settle_external_read(
    store: &mut Store,
    lease: &DispatchLease,
    result: Result<(Value, Vec<EvidenceReference>), String>,
) -> Result<
    (
        mewu_core::ReceiptCommit,
        Result<CommittedToolOutput, String>,
    ),
    String,
> {
    match result {
        Ok(value) => {
            let (receipt, output) = settle_read(store, lease, "memory_recall", Ok(value))?;
            Ok((receipt, Ok(output)))
        }
        Err(error) => {
            // The current remote-read wrapper can fail before HTTP, during it,
            // or at its final authority fence. An error proves no complete
            // authorized result; it must not fabricate a received response.
            let receipt = store
                .record_request_unknown(lease, NoResponseReason::IncompleteResponse)
                .map_err(journal_runtime::store_error)?;
            Ok((receipt, Err(error)))
        }
    }
}

pub async fn execute(
    app: AppHandle,
    scene: String,
    run: String,
    call: ModelToolCall,
    lease: DispatchLease,
) -> Result<CommittedToolOutput, String> {
    let host = app.state::<Host>();
    if !matches!(
        call.name.as_str(),
        "memory_save" | "memory_forget" | "memory_recall" | "history_search"
    ) {
        let mut engine = host.lock()?;
        let receipt = engine
            .store
            .record_tool_not_sent(&lease, NotSentReason::NotAdvertised)
            .map_err(journal_runtime::store_error)?;
        journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
        return Err("工具不可用".into());
    }
    let mutation = matches!(call.name.as_str(), "memory_save" | "memory_forget");
    // Only this lexical scope holds plugins->Engine. No guard survives an await.
    let external = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        let mut engine = host.lock()?;
        live(&host, &engine, &scene, &run)?;
        let authority = engine
            .store
            .run_memory_authority(&scene, &run)
            .map_err(agent_tools::domain_error)?;
        let grant = match authority {
            Some(RunMemoryAuthority::External { grant, .. }) => {
                memory_host::authorize(&plugins.store, &grant)?;
                engine
                    .store
                    .memory_route_for_run(&scene, &run, &grant)
                    .map_err(agent_tools::domain_error)?;
                Some(grant)
            }
            Some(RunMemoryAuthority::Local { .. }) => None,
            None => return Err("本轮未启用记忆工具".into()),
        };
        if mutation {
            let binding = grant
                .as_ref()
                .map(|grant| {
                    engine
                        .store
                        .memory_binding(&grant.binding_id)
                        .map_err(agent_tools::domain_error)
                })
                .transpose()?;
            // The core verifies the prepared lease's exact binding and performs
            // effect + receipt atomically. Do not mark dispatched first.
            let result = if grant.is_some() {
                let effect = match memory_tools::recorded_effect(&call.name, &call.arguments) {
                    Ok(effect) => effect,
                    Err(error) => {
                        let (receipt, output) = reject_arguments(&mut engine.store, &lease, &call.name, error)?;
                        journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
                        return Ok(output);
                    }
                };
                match effect {
                    memory_tools::RecordedEffect::Retain { quote } => engine
                        .store
                        .queue_external_quote_with_receipt(&lease, &quote),
                    memory_tools::RecordedEffect::Forget {
                        id,
                        expected_revision,
                    } => engine.store.forget_external_evidence_with_receipt(
                        &lease,
                        &id,
                        expected_revision,
                    ),
                }
                .map_err(agent_tools::domain_error)?
            } else {
                match local_effect(&mut engine.store, &lease, &call.name, &call.arguments)? {
                    LocalEffectOutcome::Applied(result) => result,
                    LocalEffectOutcome::Rejected(receipt, output) => {
                        journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
                        return Ok(output);
                    }
                }
            };
            journal_runtime::emit_changed(&app, &scene, &run, result.receipt.journal_revision);
            crate::cancel_ended_runs(&mut engine, &result.snapshot);
            crate::publish(&app, &result.snapshot);
            if let Some(binding) = &binding {
                if call.name == "memory_forget" {
                    host.memory
                        .cancel_where(|task| task.binding_id == binding.id);
                }
            }
            let active = live(&host, &engine, &scene, &run);
            let content = result.result.model_visible_json;
            drop(engine);
            drop(plugins);
            if let Some(binding) = binding {
                memory_host::changed(&app, &binding.agent_id);
            }
            // Forget may intentionally revoke this Run. Its confirmed receipt
            // survives, while the model cannot continue with stale memory.
            active?;
            return content
                .map(|model_json| CommittedToolOutput { model_json })
                .ok_or_else(|| "工具结果不可用".into());
        }
        let args = match search(&call.arguments) {
            Ok(args) => args,
            Err(error) => {
                let (receipt, output) = reject_arguments(&mut engine.store, &lease, &call.name, error)?;
                journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
                return Ok(output);
            }
        };
        let receipt = engine
            .store
            .mark_tool_dispatched(&lease)
            .map_err(journal_runtime::store_error)?;
        journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
        if call.name == "history_search" || grant.is_none() {
            let result = local_read(&engine.store, &scene, &run, &call.name, &args.query);
            let (receipt, output) = settle_read(&mut engine.store, &lease, &call.name, result)?;
            journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
            crate::publish(&app, &engine.store.snapshot());
            live(&host, &engine, &scene, &run)?;
            return Ok(output);
        }
        (grant.expect("external recall branch"), args.query)
    };
    let (grant, query) = external;
    let result = memory_tools::recall(&app, &scene, &run, &query)
        .await
        .and_then(|value| external_references(&grant, &value).map(|refs| (value, refs)));
    // Save observed read outcome before current authority checks. The body stays
    // omitted even for a late result, and no subsequent request is authorized.
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    let (receipt, output) = settle_external_read(&mut engine.store, &lease, result)?;
    journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
    crate::publish(&app, &engine.store.snapshot());
    live(&host, &engine, &scene, &run)?;
    memory_host::authorize(&plugins.store, &grant)?;
    engine
        .store
        .memory_route_for_run(&scene, &run, &grant)
        .map_err(agent_tools::domain_error)?;
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{
        CompletePublicTurn, ModelDispatchInput, ProposedTool, ReceiptKind, ReceiptPhase,
        RunContext, RunStatus, SceneCommand, ToolBinding,
    };

    fn active() -> (Store, RunContext) {
        active_store(Store::open_in_memory().unwrap())
    }
    fn active_store(mut store: Store) -> (Store, RunContext) {
        let mut agent = store.snapshot().agents[0].clone();
        agent.memory_enabled = true;
        store.apply(SceneCommand::SaveAgent { agent }).unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "请记住我偏好简短回答。".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene).unwrap();
        (store, run)
    }
    fn planned(store: &mut Store, run: &RunContext, name: &str, round: u32) -> DispatchLease {
        let lease = store
            .begin_model_request(
                &run.scene_id,
                &run.run_id,
                ModelDispatchInput {
                    round,
                    projected_request_sha256: "1".repeat(64),
                },
            )
            .unwrap();
        store
            .record_model_turn(
                &lease,
                CompletePublicTurn {
                    text: "准备处理".into(),
                    calls: vec![ProposedTool {
                        call_id: format!("call_{round}"),
                        binding: ToolBinding::Local { name: name.into() },
                        arguments_wire_sha256: "2".repeat(64),
                    }],
                },
            )
            .unwrap()
            .tools
            .remove(0)
            .lease
    }
    fn planned_saves(store: &mut Store, run: &RunContext, round: u32, arguments: &[&str]) -> Vec<DispatchLease> {
        let lease = store.begin_model_request(&run.scene_id, &run.run_id, ModelDispatchInput {
            round, projected_request_sha256: "1".repeat(64),
        }).unwrap();
        store.record_model_turn(&lease, CompletePublicTurn {
            text: String::new(),
            calls: arguments.iter().enumerate().map(|(index, args)| ProposedTool {
                call_id: format!("memory_{round}_{index}"),
                binding: ToolBinding::Local { name: "memory_save".into() },
                arguments_wire_sha256: format!("{:x}", Sha256::digest(args.as_bytes())),
            }).collect(),
        }).unwrap().tools.into_iter().map(|tool| tool.lease).collect()
    }
    fn tool_detail(store: &Store, run: &RunContext) -> mewu_core::JournalEntryDetail {
        let page = store
            .run_journal_page(&run.scene_id, &run.run_id, None, 25)
            .unwrap()
            .unwrap();
        let event = page
            .entries
            .iter()
            .rev()
            .find(|e| e.kind == ReceiptKind::Tool)
            .unwrap();
        store
            .run_journal_entry(&run.scene_id, &run.run_id, &event.id, page.summary.revision)
            .unwrap()
    }

    #[test]
    fn local_save_uses_prepared_atomic_effect_and_receipt() {
        let (mut store, run) = active();
        let lease = planned(&mut store, &run, "memory_save", 0);
        let args = json!({"id":null,"expectedRevision":null,"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}).to_string();
        let LocalEffectOutcome::Applied(committed) = local_effect(&mut store, &lease, "memory_save", &args).unwrap() else {
            panic!("valid memory must commit")
        };
        assert_eq!(
            store
                .memory_page(&run.agent.id, "", None, 25)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            committed.result.response_kind,
            ToolResponseKind::LocalCommit
        );
        assert_eq!(
            tool_detail(&store, &run).entry.phase,
            ReceiptPhase::Responded
        );
        assert!(store.mark_tool_dispatched(&lease).is_err());
        // Grounding errors cannot mutate memory or leave a fake success receipt.
        let bad_lease = planned(&mut store, &run, "memory_save", 1);
        assert!(matches!(local_effect(
            &mut store,
            &bad_lease,
            "memory_save",
            r#"{"id":null,"expectedRevision":null,"text":"不是用户原文","sourceQuote":"来自附件"}"#
        )
        .unwrap(), LocalEffectOutcome::Rejected(_, _)));
        assert_eq!(
            store
                .memory_page(&run.agent.id, "", None, 25)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            tool_detail(&store, &run).entry.phase,
            ReceiptPhase::NotSent
        );
        assert_eq!(tool_detail(&store, &run).not_sent_reason, Some(NotSentReason::InvalidArguments));
        assert!(store.run_is_active(&run.scene_id, &run.run_id));
    }

    #[test]
    fn malformed_memory_arguments_keep_siblings_and_run_live_for_correction() {
        let (mut store, run) = active();
        let bad = r#"{"text":{"value":"用户偏好简短回答"},"sourceQuote":"我偏好简短回答"}"#;
        let valid = r#"{"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}"#;
        let leases = planned_saves(&mut store, &run, 0, &[bad, valid]);
        let LocalEffectOutcome::Rejected(_, output) = local_effect(&mut store, &leases[0], "memory_save", bad).unwrap() else {
            panic!("wrong field type must be rejected")
        };
        let feedback: Value = serde_json::from_str(&output.model_json).unwrap();
        assert_eq!(feedback["executed"], false);
        assert!(feedback["correction"].is_string());
        assert!(store.run_is_active(&run.scene_id, &run.run_id));
        assert!(matches!(local_effect(&mut store, &leases[1], "memory_save", valid).unwrap(), LocalEffectOutcome::Applied(_)));
        let page = store.run_journal_page(&run.scene_id, &run.run_id, None, 25).unwrap().unwrap();
        assert_eq!(page.entries[1].phase, ReceiptPhase::NotSent);
        assert_eq!(page.entries[2].phase, ReceiptPhase::Responded);
        assert_eq!(store.run_journal_entry(&run.scene_id, &run.run_id, &page.entries[1].id, page.summary.revision).unwrap().not_sent_reason,
            Some(NotSentReason::InvalidArguments));
        assert_eq!(store.memory_page(&run.agent.id, "", None, 25).unwrap().total, 1);
    }

    #[test]
    fn creation_variants_are_idempotent_and_updates_still_require_exact_revision() {
        let (mut store, run) = active();
        let variants = [
            json!({"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}),
            json!({"id":null,"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}),
            json!({"expectedRevision":null,"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}),
            json!({"id":null,"expectedRevision":null,"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}),
        ];
        for (round, args) in variants.iter().enumerate() {
            let arguments = args.to_string();
            let lease = planned_saves(&mut store, &run, round as u32, &[&arguments]).remove(0);
            assert!(matches!(local_effect(&mut store, &lease, "memory_save", &arguments).unwrap(), LocalEffectOutcome::Applied(_)));
        }
        let mut entries = store.memory_page(&run.agent.id, "", None, 25).unwrap();
        assert_eq!(entries.total, 1);
        let original = entries.entries.remove(0);
        assert_eq!(original.revision, 1);
        let update = json!({"id":original.id,"expectedRevision":original.revision,"text":"用户偏好简短的回答","sourceQuote":"我偏好简短回答"}).to_string();
        let lease = planned_saves(&mut store, &run, 4, &[&update]).remove(0);
        assert!(matches!(local_effect(&mut store, &lease, "memory_save", &update).unwrap(), LocalEffectOutcome::Applied(_)));
        let lease = planned_saves(&mut store, &run, 5, &[&update]).remove(0);
        assert!(matches!(local_effect(&mut store, &lease, "memory_save", &update).unwrap(), LocalEffectOutcome::Rejected(_, _)));
        let current = store.memory_entry(&run.agent.id, &original.id).unwrap().unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.text, "用户偏好简短的回答");
        assert!(store.run_is_active(&run.scene_id, &run.run_id));
    }

    #[test]
    fn rejected_memory_arguments_do_not_coerce_or_write_entries() {
        let invalid = [
            r#"{"id":"existing","text":"x","sourceQuote":"我"}"#,
            r#"{"expectedRevision":1,"text":"x","sourceQuote":"我"}"#,
            r#"{"id":null,"expectedRevision":"null","text":"x","sourceQuote":"我"}"#,
            r#"{"text":"x","sourceQuote":"我","agentId":"another-agent"}"#,
            r#"{"text":"x","text":"again","sourceQuote":"我"}"#,
            r#"{"text":" ","sourceQuote":"我"}"#,
            r#"{"text":"x","sourceQuote":" ","sourceMessageId":null}"#,
            r#"{"id":"no-such-memory","expectedRevision":1,"text":"x","sourceQuote":"我"}"#,
            r#"[null,"用户偏好简短回答",null,"我偏好简短回答",null]"#,
            "[]",
        ];
        for arguments in invalid {
            // A real model turn permits at most eight proposals. Keep all ten
            // independent rejection cases, each in its own live run and lease.
            let (mut store, run) = active();
            let lease = planned_saves(&mut store, &run, 0, &[arguments]).remove(0);
            let LocalEffectOutcome::Rejected(_, output) = local_effect(&mut store, &lease, "memory_save", arguments).unwrap() else {
                panic!("invalid memory arguments must be rejected")
            };
            let feedback: Value = serde_json::from_str(&output.model_json).unwrap();
            assert_eq!(feedback["executed"], false);
            let correction = feedback["correction"].as_str().unwrap();
            assert!(correction.contains("当前工具 schema"));
            assert!(correction.contains("已声明的字段"));
            assert!(!correction.contains("sourceMessageId"));
            assert_eq!(tool_detail(&store, &run).not_sent_reason, Some(NotSentReason::InvalidArguments));
            assert_eq!(store.memory_page(&run.agent.id, "", None, 25).unwrap().total, 0);
            assert!(store.run_is_active(&run.scene_id, &run.run_id));
        }
    }

    #[test]
    fn follow_up_save_uses_the_advertised_prior_user_source() {
        let (mut store, first) = active();
        let source = first.messages.last().unwrap().id.clone();
        store.finish_run(&first.scene_id, &first.run_id, "知道了").unwrap();
        store.apply(SceneCommand::SetDraft { scene_id: first.scene_id.clone(), draft: "记着啊".into() }).unwrap();
        let run = store.begin_run(&first.scene_id).unwrap();
        let args = json!({"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答","sourceMessageId":source}).to_string();
        let lease = planned_saves(&mut store, &run, 0, &[&args]).remove(0);
        assert!(matches!(local_effect(&mut store, &lease, "memory_save", &args).unwrap(), LocalEffectOutcome::Applied(_)));
        let saved = store.memory_page(&run.agent.id, "", None, 25).unwrap().entries.remove(0);
        assert_eq!(saved.source.unwrap().message_id, source);
    }

    #[test]
    fn production_memory_effect_reopens_and_storage_failures_are_not_argument_feedback() {
        struct Database(std::path::PathBuf);
        impl Drop for Database {
            fn drop(&mut self) {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(self.0.join(format!("store.sqlite{suffix}")));
                }
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.private/remake-review/memory-tool-repair-20261020")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let database = Database(directory);
        let path = database.0.join("store.sqlite");
        let (mut store, run) = active_store(Store::open(&path).unwrap());
        let args = r#"{"text":"用户偏好简短回答","sourceQuote":"我偏好简短回答"}"#;
        let lease = planned_saves(&mut store, &run, 0, &[args]).remove(0);
        let disk = rusqlite::Connection::open(&path).unwrap();
        disk.execute_batch("CREATE TRIGGER reject_receipt BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
        assert!(local_effect(&mut store, &lease, "memory_save", args).is_err());
        assert!(local_effect(&mut store, &lease, "memory_save", "[]").is_err());
        assert_eq!(store.memory_page(&run.agent.id, "", None, 25).unwrap().total, 0);
        assert_eq!(tool_detail(&store, &run).entry.phase, ReceiptPhase::Prepared);
        assert!(store.run_is_active(&run.scene_id, &run.run_id));
        disk.execute_batch("DROP TRIGGER reject_receipt;").unwrap();
        assert!(matches!(local_effect(&mut store, &lease, "memory_save", args).unwrap(), LocalEffectOutcome::Applied(_)));
        let saved = store.memory_page(&run.agent.id, "", None, 25).unwrap().entries.remove(0);
        assert_eq!(saved.source.as_ref().unwrap().message_id, run.messages.last().unwrap().id);
        store.finish_run(&run.scene_id, &run.run_id, "已保存").unwrap();
        drop(disk);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.memory_entry(&run.agent.id, &saved.id).unwrap(), Some(saved));
        assert_eq!(tool_detail(&reopened, &run).entry.phase, ReceiptPhase::Responded);
    }

    #[test]
    fn recall_receipt_keeps_source_only_even_after_delete_and_late_cancel() {
        let (mut store, run) = active();
        store
            .apply(SceneCommand::SaveMemory {
                agent_id: run.agent.id.clone(),
                id: None,
                text: "PRIVATE-MEMORY-样本只供本机检索".into(),
                expected_revision: None,
            })
            .unwrap();
        let entry = store
            .memory_page(&run.agent.id, "", None, 25)
            .unwrap()
            .entries
            .remove(0);
        let lease = planned(&mut store, &run, "memory_recall", 0);
        store.mark_tool_dispatched(&lease).unwrap();
        let observed = local_read(
            &store,
            &run.scene_id,
            &run.run_id,
            "memory_recall",
            "PRIVATE-MEMORY",
        );
        store.cancel_run(&run.scene_id).unwrap();
        let (_, output) = settle_read(&mut store, &lease, "memory_recall", observed).unwrap();
        assert!(output.model_json.contains("PRIVATE-MEMORY"));
        assert!(!store.run_is_active(&run.scene_id, &run.run_id));
        assert_eq!(
            store.snapshot().scenes[0].run.as_ref().unwrap().status,
            RunStatus::Canceled
        );
        store
            .apply(SceneCommand::DeleteMemory {
                agent_id: run.agent.id.clone(),
                id: entry.id.clone(),
                expected_revision: entry.revision,
            })
            .unwrap();
        let detail = tool_detail(&store, &run);
        assert!(matches!(
            detail.content,
            JournalContent::Omitted {
                reason: ContentUnavailable::MemoryPolicy
            }
        ));
        assert!(
            matches!(&detail.references[0], EvidenceReference::LocalMemory { id, revision } if id == &entry.id && *revision == entry.revision)
        );
        assert!(!serde_json::to_string(&detail)
            .unwrap()
            .contains("PRIVATE-MEMORY"));
    }

    #[test]
    fn history_receipt_retains_identity_but_not_a_second_copy_of_messages() {
        let (mut store, first) = active();
        store
            .finish_run(&first.scene_id, &first.run_id, "PRIVATE-HISTORY 合成旧答复")
            .unwrap();
        store
            .apply(SceneCommand::SetDraft {
                scene_id: first.scene_id.clone(),
                draft: "找旧记录".into(),
            })
            .unwrap();
        let run = store.begin_run(&first.scene_id).unwrap();
        let lease = planned(&mut store, &run, "history_search", 0);
        store.mark_tool_dispatched(&lease).unwrap();
        let observed = local_read(
            &store,
            &run.scene_id,
            &run.run_id,
            "history_search",
            "PRIVATE-HISTORY",
        );
        let (_, output) = settle_read(&mut store, &lease, "history_search", observed).unwrap();
        assert!(output.model_json.contains("PRIVATE-HISTORY"));
        let detail = tool_detail(&store, &run);
        assert!(matches!(
            detail.content,
            JournalContent::Omitted {
                reason: ContentUnavailable::HistoryPolicy
            }
        ));
        assert!(
            matches!(&detail.references[0], EvidenceReference::History { scene_id, .. } if scene_id == &run.scene_id)
        );
        assert!(!serde_json::to_string(&detail)
            .unwrap()
            .contains("PRIVATE-HISTORY"));
    }

    #[test]
    fn external_reference_route_is_fixed_and_arguments_are_strict() {
        let grant = MemoryAuthorityGrant {
            binding_id: uuid::Uuid::new_v4().to_string(),
            binding_revision: 1,
            plugin_id: "mewu.memory-hindsight".into(),
            plugin_revision: 1,
            contribution_id: "memory".into(),
        };
        let evidence = uuid::Uuid::new_v4().to_string();
        let value = json!({"memories":[{"id":evidence,"revision":2,"text":"PRIVATE-REMOTE",
            "bindingId":"untrusted-route"}],"selection":"retrieved"});
        let refs = external_references(&grant, &value).unwrap();
        assert!(
            matches!(&refs[0], EvidenceReference::ExternalMemory { binding_id, evidence_id, operation_id, revision }
            if binding_id == &grant.binding_id && evidence_id == &evidence && operation_id.is_none() && *revision == 2)
        );
        let observed = read_observation("memory_recall", value, refs, false).unwrap();
        assert!(!serde_json::to_string(&observed)
            .unwrap()
            .contains("PRIVATE-REMOTE"));
        assert!(observed
            .model_visible_json
            .unwrap()
            .contains("PRIVATE-REMOTE"));
        for bad in [
            r#"{"query":"a","query":"b"}"#,
            r#"{"query":"a","agentId":"other"}"#,
            "[]",
            r#"["a"]"#,
        ] {
            assert!(search(bad).is_err());
        }
        assert!(memory_tools::recorded_effect("memory_save", r#"["quote"]"#).is_err());
        assert!(read_observation("shell", json!({}), vec![], false).is_err());
    }

    #[test]
    fn failed_remote_read_is_unknown_not_a_fabricated_response() {
        struct Database(std::path::PathBuf);
        impl Drop for Database {
            fn drop(&mut self) {
                for suffix in ["", "-wal", "-shm"] {
                    let mut path = self.0.as_os_str().to_os_string();
                    path.push(suffix);
                    let _ = std::fs::remove_file(std::path::PathBuf::from(path));
                }
            }
        }
        let database = Database(
            std::env::temp_dir().join(format!("mewu-journal-tool-{}.db", uuid::Uuid::new_v4())),
        );
        let (mut store, run) = active_store(Store::open(&database.0).unwrap());
        let lease = planned(&mut store, &run, "memory_recall", 0);
        store.mark_tool_dispatched(&lease).unwrap();
        let (_, output) =
            settle_external_read(&mut store, &lease, Err("PRIVATE-REMOTE-ERROR".into())).unwrap();
        assert!(output.is_err());
        let detail = tool_detail(&store, &run);
        assert_eq!(detail.entry.phase, ReceiptPhase::Unknown);
        let encoded = serde_json::to_string(&detail).unwrap();
        assert_eq!(detail.entry.response_kind, None);
        assert_eq!(detail.entry.returned_error, None);
        assert_eq!(detail.entry.observed_sha256, None);
        let disk = rusqlite::Connection::open_with_flags(
            &database.0,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let metadata: String = disk
            .query_row(
                "SELECT metadata FROM agent_run_events WHERE id=?1",
                [&detail.entry.id],
                |row| row.get(0),
            )
            .unwrap();
        let persisted: Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(persisted["unknown"], "incomplete_response");
        assert!(!encoded.contains("PRIVATE-REMOTE-ERROR"));
        assert!(!encoded.contains("local_read"));
        assert!(detail.references.is_empty());
    }
}
