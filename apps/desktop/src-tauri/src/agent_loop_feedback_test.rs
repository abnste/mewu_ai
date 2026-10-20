// SPDX-License-Identifier: MPL-2.0
// Exercise the real HTTP recorded loop and SQLite journal with a synthetic
// MCP effect executor. No subprocess, external tool or user configuration runs.
mod feedback {
    use super::*;
    use crate::journal_runtime::not_sent_feedback;
    use mewu_core::{
        JournalContent, McpCommand, McpServer, McpTool, ObservedToolResult, ReceiptKind,
        ReceiptPhase, SceneCommand, ToolBinding, ToolResponseKind,
    };

    struct FeedbackJournal {
        base: Arc<StoreJournal>,
        server: McpServer,
    }
    impl FeedbackJournal {
        fn new() -> Arc<Self> {
            let mut store = mewu_core::Store::open_in_memory().unwrap();
            let snapshot = store.snapshot();
            let scene = snapshot.active_scene_id;
            let agent = snapshot.agents[0].id.clone();
            let server = store.upsert_mcp_server(None,None,"Synthetic feedback".into(),McpCommand {
                executable:"/synthetic/never-launched".into(),args:vec![],cwd:None,
            },vec![McpTool { name:"remember".into(),description:"Synthetic echo".into(),input_schema:json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}},"additionalProperties":false}) }]).unwrap().mcp_servers.remove(0);
            let server = store
                .apply(SceneCommand::SetMcpGrants {
                    server_id: server.id.clone(),
                    expected_revision: server.revision,
                    agent_id: agent,
                    tool_names: vec!["remember".into()],
                })
                .unwrap()
                .mcp_servers
                .remove(0);
            store
                .apply(SceneCommand::SetDraft {
                    scene_id: scene.clone(),
                    draft: "Synthetic tool feedback".into(),
                })
                .unwrap();
            let run = store.begin_run(&scene).unwrap().run_id;
            Arc::new(Self {
                base: Arc::new(StoreJournal {
                    store: Mutex::new(store),
                    scene,
                    run,
                    fail_complete: AtomicBool::new(false),
                }),
                server,
            })
        }
        fn responded(
            &self,
            lease: &DispatchLease,
            value: Value,
        ) -> Result<CommittedToolOutput, String> {
            let body = value.to_string();
            let mut store = self.base.store.lock().unwrap();
            store
                .mark_tool_dispatched(lease)
                .map_err(|e| e.to_string())?;
            store
                .record_tool_response(
                    lease,
                    ObservedToolResult {
                        response_kind: ToolResponseKind::Complete,
                        returned_error: Some(false),
                        observed_encoding: "synthetic-json-v1".into(),
                        observed_bytes: body.len() as u64,
                        observed_sha256: format!("{:x}", Sha256::digest(body.as_bytes())),
                        retained_content: JournalContent::Json { value },
                        references: vec![],
                        model_visible_json: Some(body.clone()),
                    },
                )
                .map_err(|e| e.to_string())?;
            Ok(CommittedToolOutput { model_json: body })
        }
        fn revoke(&self) {
            let mut store = self.base.store.lock().unwrap();
            let agent = store.snapshot().agents[0].id.clone();
            store
                .apply(SceneCommand::SetMcpGrants {
                    server_id: self.server.id.clone(),
                    expected_revision: self.server.revision,
                    agent_id: agent,
                    tool_names: vec![],
                })
                .unwrap();
        }
    }
    impl RunObservation for FeedbackJournal {
        fn active(&self) -> Result<(), String> {
            self.base.active()?;
            self.base
                .store
                .lock()
                .unwrap()
                .mcp_for_run(
                    &self.base.scene,
                    &self.base.run,
                    &self.server.id,
                    self.server.revision,
                    "remember",
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        fn begin_model(&self, round: u32, hash: String) -> Result<DispatchLease, String> {
            self.base.begin_model(round, hash)
        }
        fn complete(
            &self,
            lease: &DispatchLease,
            text: &str,
            calls: &[ModelToolCall],
        ) -> Result<mewu_core::TurnCommit, String> {
            self.base
                .store
                .lock()
                .unwrap()
                .record_model_turn(
                    lease,
                    mewu_core::CompletePublicTurn {
                        text: text.into(),
                        calls: calls
                            .iter()
                            .map(|call| mewu_core::ProposedTool {
                                call_id: call.id.clone(),
                                binding: ToolBinding::Mcp {
                                    server_id: self.server.id.clone(),
                                    revision: self.server.revision,
                                    name: "remember".into(),
                                    advertised_alias: call.name.clone(),
                                },
                                arguments_wire_sha256: format!(
                                    "{:x}",
                                    Sha256::digest(call.arguments.as_bytes())
                                ),
                            })
                            .collect(),
                    },
                )
                .map_err(|e| e.to_string())
        }
        fn unknown(&self, lease: &DispatchLease, reason: NoResponseReason) -> Result<(), String> {
            self.base.unknown(lease, reason)
        }
        fn not_sent(&self, lease: &DispatchLease, reason: NotSentReason) -> Result<(), String> {
            self.base.not_sent(lease, reason)
        }
    }
    fn turn(calls: &[(&str, Value)]) -> String {
        let calls:Vec<_> = calls.iter().enumerate().map(|(index,(id,args))|json!({"index":index,"id":id,"type":"function","function":{"name":"remember","arguments":args.to_string()}})).collect();
        format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":"tool_calls"}]})
        )
    }

    #[test]
    fn feedback_is_only_for_the_two_explicit_notsent_reasons_and_has_no_private_data() {
        for (reason, code, correctable) in [
            (NotSentReason::InvalidArguments, "invalid_arguments", true),
            (
                NotSentReason::PreparationFailed,
                "preparation_failed",
                false,
            ),
        ] {
            let output = not_sent_feedback(reason).unwrap();
            let value: Value = serde_json::from_str(&output.model_json).unwrap();
            assert_eq!(value["isError"], true);
            assert_eq!(value["executed"], false);
            assert_eq!(value["code"], code);
            assert_eq!(value["correctable"], correctable);
            assert_eq!(value["retryable"], true);
            assert!(value["correction"].is_string());
            for field in [
                "arguments",
                "stack",
                "path",
                "sourceQuote",
                "sourceMessageId",
            ] {
                assert!(value.get(field).is_none());
            }
        }
        for reason in [
            NotSentReason::AuthorityChanged,
            NotSentReason::CatalogChanged,
            NotSentReason::RunEnded,
            NotSentReason::BudgetExhausted,
            NotSentReason::DuplicateCallId,
            NotSentReason::NotAdvertised,
        ] {
            assert!(not_sent_feedback(reason).is_none());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recorded_http_error_feedback_keeps_the_valid_sibling_and_accepts_the_models_correction(
    ) {
        let fixture = Fixture::new(vec![
            turn(&[
                ("bad", json!({"text":5})),
                ("sibling", json!({"text":"valid sibling"})),
            ]),
            turn(&[("corrected", json!({"text":"corrected argument"}))]),
            final_text("完成"),
        ]);
        let journal = FeedbackJournal::new();
        let host = journal.clone();
        let effects = Arc::new(Mutex::new(Vec::new()));
        let actual = effects.clone();
        let answer = recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            move |call, lease| {
                let host = host.clone();
                let actual = actual.clone();
                async move {
                    let args: Value =
                        serde_json::from_str(&call.arguments).map_err(|e| e.to_string())?;
                    if !args.get("text").is_some_and(Value::is_string) {
                        host.not_sent(&lease, NotSentReason::InvalidArguments)?;
                        host.active()?;
                        return not_sent_feedback(NotSentReason::InvalidArguments)
                            .ok_or_else(|| "hard failure".into());
                    }
                    actual.lock().unwrap().push(call.id.clone());
                    host.responded(&lease, json!({"ok":true,"text":args["text"]}))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(answer, "完成");
        assert_eq!(*effects.lock().unwrap(), ["sibling", "corrected"]);
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let results: Vec<_> = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_call_id"], "bad");
        let feedback: Value =
            serde_json::from_str(results[0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(feedback["executed"], false);
        assert_eq!(feedback["code"], "invalid_arguments");
        assert_eq!(results[1]["tool_call_id"], "sibling");
        let page = journal.base.page();
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

    #[tokio::test(flavor = "current_thread")]
    async fn recorded_http_unknown_outcome_stops_without_replay_or_executing_a_sibling() {
        let fixture = Fixture::new(vec![
            turn(&[
                ("uncertain", json!({"text":"effect"})),
                ("sibling", json!({"text":"must not run"})),
            ]),
            final_text("must not request"),
        ]);
        let journal = FeedbackJournal::new();
        let host = journal.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let actual = calls.clone();
        let result = recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            move |_, lease| {
                let host = host.clone();
                let actual = actual.clone();
                async move {
                    actual.fetch_add(1, Ordering::SeqCst);
                    host.base
                        .store
                        .lock()
                        .unwrap()
                        .mark_tool_dispatched(&lease)
                        .map_err(|e| e.to_string())?;
                    host.unknown(&lease, NoResponseReason::TimedOut)?;
                    Err("tool outcome unknown".into())
                }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert!(journal
            .base
            .page()
            .entries
            .iter()
            .any(|entry| entry.kind == ReceiptKind::Tool && entry.phase == ReceiptPhase::Unknown));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recorded_http_permission_revocation_after_rejection_prevents_feedback_and_the_next_model_request(
    ) {
        let fixture = Fixture::new(vec![
            turn(&[
                ("bad", json!({"text":5})),
                ("sibling", json!({"text":"must not run"})),
            ]),
            final_text("must not request"),
        ]);
        let journal = FeedbackJournal::new();
        let host = journal.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let actual = calls.clone();
        let result = recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            move |_, lease| {
                let host = host.clone();
                let actual = actual.clone();
                async move {
                    actual.fetch_add(1, Ordering::SeqCst);
                    host.not_sent(&lease, NotSentReason::InvalidArguments)?;
                    host.revoke();
                    host.active()?;
                    not_sent_feedback(NotSentReason::InvalidArguments)
                        .ok_or_else(|| "hard failure".into())
                }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert!(!journal
            .base
            .store
            .lock()
            .unwrap()
            .run_is_active(&journal.base.scene, &journal.base.run));
    }
}
