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
    if arguments.len() > 16 * 1024 {
        return Err("工具参数过长".into());
    }
    serde_json::from_str(arguments).map_err(|_| "工具参数格式无效".into())
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

fn local_effect(
    store: &mut Store,
    lease: &DispatchLease,
    name: &str,
    arguments: &str,
) -> Result<LocalToolCommit, String> {
    let effect = agent_tools::recorded_local_effect(name, arguments)?;
    store
        .apply_local_memory_with_receipt(lease, effect)
        .map_err(agent_tools::domain_error)
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
                match memory_tools::recorded_effect(&call.name, &call.arguments)? {
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
                local_effect(&mut engine.store, &lease, &call.name, &call.arguments)?
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
                let receipt = engine
                    .store
                    .record_tool_not_sent(&lease, NotSentReason::InvalidArguments)
                    .map_err(journal_runtime::store_error)?;
                journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
                return Err(error);
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
        let committed = local_effect(&mut store, &lease, "memory_save", &args).unwrap();
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
        // Parsing/grounding errors cannot mutate memory or leave a fake receipt.
        let bad_lease = planned(&mut store, &run, "memory_save", 1);
        assert!(local_effect(
            &mut store,
            &bad_lease,
            "memory_save",
            r#"{"id":null,"expectedRevision":null,"text":"不是用户原文","sourceQuote":"来自附件"}"#
        )
        .is_err());
        assert_eq!(
            store
                .memory_page(&run.agent.id, "", None, 25)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            tool_detail(&store, &run).entry.phase,
            ReceiptPhase::Prepared
        );
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
        ] {
            assert!(search(bad).is_err());
        }
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
