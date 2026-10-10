// SPDX-License-Identifier: MPL-2.0
//! Bounded Chat Completions tool loop. Tools remain host-owned capabilities;
//! model output never selects a callable that was not advertised for this run.
//! https://api-docs.deepseek.com/guides/tool_calls/
//! https://api-docs.deepseek.com/guides/thinking_mode/
use crate::ai::{self, ModelToolCall};
use crate::journal_runtime::{CommittedToolOutput, RunJournal};
use crate::provider_transport::{self, Transport};
use mewu_core::{DispatchLease, NoResponseReason, NotSentReason};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, future::Future, sync::Arc, time::Duration};

const MAX_ROUNDS: usize = 6;
const MAX_CALLS: usize = 16;
const MAX_TOOL_OUTPUT: usize = 128 * 1024;
const MAX_TOTAL_TOOL_OUTPUT: usize = 512 * 1024;
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Dropping this future cancels its current network request/tool future. The
/// executor must independently fence every store side effect against the run ID.
/// Only visible content is returned. Reasoning is transient protocol state.
pub async fn run<F, Fut>(
    url: url::Url,
    key: &str,
    body: Value,
    tools: Vec<Value>,
    on_delta: impl FnMut(&str),
    execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    run_with_transport(Transport::chat(url), key, body, tools, on_delta, execute).await
}

pub async fn run_with_transport<F, Fut>(
    transport: Transport,
    key: &str,
    body: Value,
    tools: Vec<Value>,
    on_delta: impl FnMut(&str),
    execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    tokio::time::timeout(
        RUN_TIMEOUT,
        run_inner(transport, key, body, tools, on_delta, execute),
    )
    .await
    .map_err(|_| "本次处理已超过时间上限".to_string())?
}

/// Store-owned Runs always use this entry point, including text-only Runs.
/// Each returned tool body must already have its durable observed receipt.
pub async fn run_recorded_with_transport<F, Fut>(
    transport: Transport,
    key: &str,
    body: Value,
    tools: Vec<Value>,
    on_delta: impl FnMut(&str),
    journal: Arc<RunJournal>,
    execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall, DispatchLease) -> Fut,
    Fut: Future<Output = Result<CommittedToolOutput, String>>,
{
    run_recorded_with_reasoning(
        transport,
        key,
        body,
        tools,
        on_delta,
        |_| {},
        journal,
        execute,
    )
    .await
}

/// Public provider reasoning is streamed separately; opaque continuation and
/// journal turn receipts retain their original private/body-only contracts.
pub async fn run_recorded_with_reasoning<F, Fut>(
    transport: Transport,
    key: &str,
    body: Value,
    tools: Vec<Value>,
    on_delta: impl FnMut(&str),
    on_reasoning_delta: impl FnMut(&str),
    journal: Arc<RunJournal>,
    execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall, DispatchLease) -> Fut,
    Fut: Future<Output = Result<CommittedToolOutput, String>>,
{
    tokio::time::timeout(
        RUN_TIMEOUT,
        recorded_inner_with_reasoning(
            transport,
            key,
            body,
            tools,
            on_delta,
            on_reasoning_delta,
            journal,
            execute,
        ),
    )
    .await
    .map_err(|_| "本次处理已超过时间上限".to_string())?
}

trait RunObservation: Send + Sync {
    fn active(&self) -> Result<(), String>;
    fn begin_model(&self, round: u32, hash: String) -> Result<DispatchLease, String>;
    fn complete(
        &self,
        lease: &DispatchLease,
        text: &str,
        calls: &[ModelToolCall],
    ) -> Result<mewu_core::TurnCommit, String>;
    fn unknown(&self, lease: &DispatchLease, reason: NoResponseReason) -> Result<(), String>;
    fn not_sent(&self, lease: &DispatchLease, reason: NotSentReason) -> Result<(), String>;
}
impl RunObservation for RunJournal {
    fn active(&self) -> Result<(), String> {
        RunJournal::active(self)
    }
    fn begin_model(&self, round: u32, hash: String) -> Result<DispatchLease, String> {
        RunJournal::begin_model(self, round, hash)
    }
    fn complete(
        &self,
        lease: &DispatchLease,
        text: &str,
        calls: &[ModelToolCall],
    ) -> Result<mewu_core::TurnCommit, String> {
        RunJournal::complete(self, lease, text, calls)
    }
    fn unknown(&self, lease: &DispatchLease, reason: NoResponseReason) -> Result<(), String> {
        RunJournal::unknown(self, lease, reason)
    }
    fn not_sent(&self, lease: &DispatchLease, reason: NotSentReason) -> Result<(), String> {
        RunJournal::not_sent(self, lease, reason)
    }
}

async fn recorded_inner<F, Fut, J: RunObservation>(
    transport: Transport,
    key: &str,
    body: Value,
    tools: Vec<Value>,
    on_delta: impl FnMut(&str),
    journal: Arc<J>,
    execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall, DispatchLease) -> Fut,
    Fut: Future<Output = Result<CommittedToolOutput, String>>,
{
    recorded_inner_with_reasoning(
        transport,
        key,
        body,
        tools,
        on_delta,
        |_| {},
        journal,
        execute,
    )
    .await
}

async fn recorded_inner_with_reasoning<F, Fut, J: RunObservation>(
    transport: Transport,
    key: &str,
    mut body: Value,
    tools: Vec<Value>,
    mut on_delta: impl FnMut(&str),
    mut on_reasoning_delta: impl FnMut(&str),
    journal: Arc<J>,
    mut execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall, DispatchLease) -> Fut,
    Fut: Future<Output = Result<CommittedToolOutput, String>>,
{
    let advertised = advertised_names(&tools)?;
    let request = body.as_object_mut().ok_or("模型请求格式无效")?;
    if !request.get("messages").is_some_and(Value::is_array) {
        return Err("模型请求缺少消息".into());
    }
    request.insert("stream".into(), json!(true));
    if tools.is_empty() {
        request.remove("tools");
        request.remove("tool_choice");
        request.remove("parallel_tool_calls");
    } else {
        request.insert("tools".into(), Value::Array(tools));
    }
    let mut body = provider_transport::request_body(transport.protocol, body)?;
    let mut answer = String::new();
    let mut call_count = 0usize;
    let mut seen_ids = HashSet::new();
    let mut output_bytes = 0usize;
    let mut reasoning_bytes = 0usize;
    for round in 0..MAX_ROUNDS {
        journal.active()?;
        let request_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&body).map_err(|_| "模型请求格式无效")?)
        );
        let mut lease = None;
        let mut first_text = true;
        let mut overflow = false;
        let mut reasoning_overflow = false;
        let mut first_reasoning = true;
        let received = provider_transport::stream_turn_with_reasoning_before_send(
            &transport,
            key,
            body.clone(),
            |delta| {
                if delta.is_empty() || overflow {
                    return;
                }
                let separator = if first_text && !answer.is_empty() {
                    "\n\n"
                } else {
                    ""
                };
                if answer
                    .len()
                    .saturating_add(separator.len())
                    .saturating_add(delta.len())
                    > ai::MAX_ANSWER_BYTES
                {
                    overflow = true;
                    return;
                }
                first_text = false;
                if !separator.is_empty() {
                    answer.push_str(separator);
                    on_delta(separator);
                }
                answer.push_str(delta);
                on_delta(delta);
            },
            |delta| {
                if delta.is_empty() || reasoning_overflow {
                    return;
                }
                let separator = if first_reasoning && reasoning_bytes > 0 {
                    "\n\n"
                } else {
                    ""
                };
                let next = reasoning_bytes
                    .saturating_add(separator.len())
                    .saturating_add(delta.len());
                if next > ai::MAX_ANSWER_BYTES {
                    reasoning_overflow = true;
                    return;
                }
                first_reasoning = false;
                reasoning_bytes = next;
                if !separator.is_empty() {
                    on_reasoning_delta(separator);
                }
                on_reasoning_delta(delta);
            },
            || {
                lease = Some(journal.begin_model(round as u32, request_hash)?);
                Ok(())
            },
        )
        .await;
        let turn = match received {
            Ok(turn) => turn,
            Err(error) => {
                if let Some(lease) = &lease {
                    journal.unknown(lease, NoResponseReason::IncompleteResponse)?;
                }
                return Err(error);
            }
        };
        let lease = lease.ok_or("模型请求缺少执行记录")?;
        // Never serialize ModelTurn or opaque continuation into the journal.
        // Its durable observed body remains independent of public reasoning.
        let committed = journal.complete(&lease, &turn.content, &turn.tool_calls)?;
        if committed.tools.len() != turn.tool_calls.len()
            || committed
                .tools
                .iter()
                .zip(&turn.tool_calls)
                .any(|(p, c)| p.call_id != c.id)
        {
            return Err("工具执行记录不一致".into());
        }
        journal.active()?;
        let duplicate = turn
            .tool_calls
            .iter()
            .any(|call| seen_ids.contains(&call.id));
        let budget = overflow
            || reasoning_overflow
            || (round + 1 == MAX_ROUNDS && !turn.tool_calls.is_empty())
            || call_count.saturating_add(turn.tool_calls.len()) > MAX_CALLS;
        if duplicate || budget {
            for prepared in &committed.tools {
                journal.not_sent(
                    &prepared.lease,
                    if duplicate {
                        NotSentReason::DuplicateCallId
                    } else {
                        NotSentReason::BudgetExhausted
                    },
                )?;
            }
            return Err(if duplicate {
                "服务重复了工具调用标识，已停止处理"
            } else {
                "本次处理已达到长度或工具预算"
            }
            .into());
        }
        if turn.tool_calls.is_empty() {
            return Ok(answer);
        }
        call_count += turn.tool_calls.len();
        for call in &turn.tool_calls {
            seen_ids.insert(call.id.clone());
        }
        provider_transport::append_turn(transport.protocol, &mut body, &turn)?;
        for (call, prepared) in turn.tool_calls.into_iter().zip(committed.tools) {
            journal.active()?;
            let call_id = call.id.clone();
            let content = if advertised.contains(&call.name) {
                execute(call, prepared.lease).await?.model_json
            } else {
                journal.not_sent(&prepared.lease, NotSentReason::NotAdvertised)?;
                json!({"error":"本次处理没有启用该工具，请仅使用已提供的工具"}).to_string()
            };
            if content.len() > MAX_TOOL_OUTPUT {
                return Err("工具结果超过长度上限".into());
            }
            output_bytes = output_bytes.saturating_add(content.len());
            if output_bytes > MAX_TOTAL_TOOL_OUTPUT {
                return Err("工具结果总量超过上限".into());
            }
            journal.active()?;
            provider_transport::append_result(transport.protocol, &mut body, &call_id, content)?;
        }
    }
    Err("工具处理已达到轮次上限".into())
}

async fn run_inner<F, Fut>(
    transport: Transport,
    key: &str,
    mut body: Value,
    tools: Vec<Value>,
    mut on_delta: impl FnMut(&str),
    mut execute: F,
) -> Result<String, String>
where
    F: FnMut(ModelToolCall) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let advertised = advertised_names(&tools)?;
    let request = body.as_object_mut().ok_or("模型请求格式无效")?;
    if !request.get("messages").is_some_and(Value::is_array) {
        return Err("模型请求缺少消息".into());
    }
    request.insert("stream".into(), json!(true));
    if tools.is_empty() {
        request.remove("tools");
        request.remove("tool_choice");
        request.remove("parallel_tool_calls");
        // Preserve legacy text streaming, including providers that terminate
        // text with [DONE] only. Never retry a rejected paid request.
        let body = provider_transport::request_body(transport.protocol, body)?;
        return provider_transport::stream_turn(&transport, key, body, on_delta)
            .await
            .map(|turn| turn.content);
    }
    request.insert("tools".into(), Value::Array(tools));
    let mut body = provider_transport::request_body(transport.protocol, body)?;
    let mut answer = String::new();
    let mut call_count = 0;
    let mut seen_ids = HashSet::new();
    let mut output_bytes = 0usize;
    for round in 0..MAX_ROUNDS {
        let mut first_text = true;
        let mut overflow = false;
        let turn = provider_transport::stream_turn(&transport, key, body.clone(), |delta| {
            if delta.is_empty() || overflow {
                return;
            }
            let separator = if first_text && !answer.is_empty() {
                "\n\n"
            } else {
                ""
            };
            if answer
                .len()
                .saturating_add(separator.len())
                .saturating_add(delta.len())
                > ai::MAX_ANSWER_BYTES
            {
                overflow = true;
                return;
            }
            first_text = false;
            if !separator.is_empty() {
                answer.push_str(separator);
                on_delta(separator);
            }
            answer.push_str(delta);
            on_delta(delta);
        })
        .await?;
        if overflow {
            return Err("回复超过长度上限".into());
        }
        if turn.tool_calls.is_empty() {
            return Ok(answer);
        }
        // Check the complete batch before any side effects. Do not dispatch the
        // last round's tools when there is no remaining round to read results.
        if round + 1 == MAX_ROUNDS {
            return Err("工具处理已达到轮次上限".into());
        }
        if call_count + turn.tool_calls.len() > MAX_CALLS {
            return Err("工具调用已达到次数上限".into());
        }
        for call in &turn.tool_calls {
            if !seen_ids.insert(call.id.clone()) {
                return Err("服务重复了工具调用标识，已停止处理".into());
            }
        }
        call_count += turn.tool_calls.len();
        provider_transport::append_turn(transport.protocol, &mut body, &turn)?;
        for call in turn.tool_calls {
            let call_id = call.id.clone();
            let result = if !advertised.contains(&call.name) {
                json!({"error":"本次处理没有启用该工具，请仅使用已提供的工具"})
            } else {
                match execute(call).await {
                    Ok(value) => value,
                    // Err is reserved for a lost run fence/unrecoverable error.
                    // Safe, recoverable tool errors are returned as Ok(Value).
                    Err(_) => return Err("工具执行已中止".into()),
                }
            };
            let mut content = serde_json::to_string(&result).map_err(|_| "工具结果格式无效")?;
            if content.len() > MAX_TOOL_OUTPUT {
                content = json!({"error":"工具结果超过长度上限，请缩小查询范围"}).to_string();
            }
            output_bytes = output_bytes.saturating_add(content.len());
            if output_bytes > MAX_TOTAL_TOOL_OUTPUT {
                return Err("工具结果总量超过上限".into());
            }
            provider_transport::append_result(transport.protocol, &mut body, &call_id, content)?;
        }
    }
    Err("工具处理已达到轮次上限".into())
}

fn advertised_names(tools: &[Value]) -> Result<HashSet<String>, String> {
    if tools.len() > 64 || serde_json::to_vec(tools).map_err(|_| "工具定义无效")?.len() > 128 * 1024
    {
        return Err("工具定义超过长度上限".into());
    }
    let mut names = HashSet::new();
    for tool in tools {
        let name = tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .ok_or("工具定义缺少名称")?;
        if tool.get("type").and_then(Value::as_str) != Some("function")
            || !ai::valid_tool_name(name)
            || !names.insert(name.to_string())
        {
            return Err("工具定义无效或名称重复".into());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        thread,
    };
    use tiny_http::{Header, Response, Server};

    struct Fixture {
        url: url::Url,
        requests: Arc<Mutex<Vec<Value>>>,
        headers: Arc<Mutex<Vec<Vec<(String, String)>>>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new(responses: Vec<String>) -> Self {
            let server = Server::http((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            let url = url::Url::parse(&format!(
                "http://{}/v1/chat/completions",
                server.server_addr()
            ))
            .unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = requests.clone();
            let headers = Arc::new(Mutex::new(Vec::new()));
            let captured_headers = headers.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let worker = thread::spawn(move || {
                for body in responses {
                    let until = std::time::Instant::now() + Duration::from_secs(5);
                    let mut request = loop {
                        if stopping.load(Ordering::SeqCst) || std::time::Instant::now() > until {
                            return;
                        }
                        if let Some(request) =
                            server.recv_timeout(Duration::from_millis(20)).unwrap()
                        {
                            break request;
                        }
                    };
                    let mut bytes = Vec::new();
                    captured_headers.lock().unwrap().push(
                        request
                            .headers()
                            .iter()
                            .map(|header| {
                                (
                                    header.field.to_string().to_ascii_lowercase(),
                                    header.value.to_string(),
                                )
                            })
                            .collect(),
                    );
                    request
                        .as_reader()
                        .take(4 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .unwrap();
                    captured
                        .lock()
                        .unwrap()
                        .push(serde_json::from_slice(&bytes).unwrap());
                    let response = Response::from_string(body).with_header(
                        Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    );
                    let _ = request.respond(response);
                }
            });
            Self {
                url,
                requests,
                headers,
                stop,
                worker: Some(worker),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn tools() -> Vec<Value> {
        vec![
            json!({"type":"function","function":{"name":"remember","parameters":{"type":"object","properties":{}}}}),
        ]
    }
    fn body() -> Value {
        json!({"model":"fixture","messages":[{"role":"user","content":"test"}]})
    }
    fn calls(ids: &[&str], name: &str, content: &str) -> String {
        let calls: Vec<_> = ids.iter().enumerate().map(|(index,id)| json!({"index":index,"id":id,"type":"function","function":{"name":name,"arguments":"{}"}})).collect();
        format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"content":content,"reasoning_content":"opaque reasoning","tool_calls":calls},"finish_reason":"tool_calls"}]})
        )
    }
    fn final_text(content: &str) -> String {
        format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"content":content},"finish_reason":"stop"}]})
        )
    }

    struct StoreJournal {
        store: Mutex<mewu_core::Store>,
        scene: String,
        run: String,
        fail_complete: AtomicBool,
    }
    impl StoreJournal {
        fn new() -> Arc<Self> {
            let mut store = mewu_core::Store::open_in_memory().unwrap();
            let scene = store.snapshot().active_scene_id;
            store
                .apply(mewu_core::SceneCommand::SetDraft {
                    scene_id: scene.clone(),
                    draft: "synthetic question".into(),
                })
                .unwrap();
            let run = store.begin_run(&scene).unwrap().run_id;
            Arc::new(Self {
                store: Mutex::new(store),
                scene,
                run,
                fail_complete: AtomicBool::new(false),
            })
        }
        fn page(&self) -> mewu_core::JournalPage {
            self.store
                .lock()
                .unwrap()
                .run_journal_page(&self.scene, &self.run, None, 25)
                .unwrap()
                .unwrap()
        }
    }
    impl RunObservation for StoreJournal {
        fn active(&self) -> Result<(), String> {
            if self
                .store
                .lock()
                .unwrap()
                .run_is_active(&self.scene, &self.run)
            {
                Ok(())
            } else {
                Err("canceled".into())
            }
        }
        fn begin_model(&self, round: u32, hash: String) -> Result<DispatchLease, String> {
            self.store
                .lock()
                .unwrap()
                .begin_model_request(
                    &self.scene,
                    &self.run,
                    mewu_core::ModelDispatchInput {
                        round,
                        projected_request_sha256: hash,
                    },
                )
                .map_err(|e| e.to_string())
        }
        fn complete(
            &self,
            lease: &DispatchLease,
            text: &str,
            calls: &[ModelToolCall],
        ) -> Result<mewu_core::TurnCommit, String> {
            if self.fail_complete.load(Ordering::SeqCst) {
                return Err("injected durable write failure".into());
            }
            self.store
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
                                binding: mewu_core::ToolBinding::Rejected {
                                    advertised_name: call.name.clone(),
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
            self.store
                .lock()
                .unwrap()
                .record_request_unknown(lease, reason)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        fn not_sent(&self, lease: &DispatchLease, reason: NotSentReason) -> Result<(), String> {
            self.store
                .lock()
                .unwrap()
                .record_tool_not_sent(lease, reason)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn text_only_all_protocols_commit_visible_turn_without_protocol_secrets() {
        use mewu_core::{ConnectionAuthMode, ConnectionProtocol, JournalContent, ReceiptPhase};
        for (protocol, stream) in [
            (
                ConnectionProtocol::ChatCompletions,
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"<think>SYNTHETIC_SECRET</think>完成"}}]})
                ),
            ),
            (
                ConnectionProtocol::ChatCompletions,
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"完成","reasoning_content":"private thought"}}]})
                ),
            ),
            (
                ConnectionProtocol::OpenAiResponses,
                provider_transport::tests::responses("完成", None),
            ),
            (
                ConnectionProtocol::AnthropicMessages,
                provider_transport::tests::anthropic("完成", None),
            ),
        ] {
            let fixture = Fixture::new(vec![stream]);
            let journal = StoreJournal::new();
            let answer = recorded_inner(
                Transport {
                    url: fixture.url.clone(),
                    protocol,
                    auth_mode: ConnectionAuthMode::None,
                },
                "fixture-secret",
                body(),
                vec![],
                |_| {},
                journal.clone(),
                |_, _| async { panic!("text-only must not invoke tools") },
            )
            .await
            .unwrap();
            assert_eq!(answer, "完成");
            let page = journal.page();
            assert_eq!(page.entries.len(), 1);
            assert_eq!(page.entries[0].phase, ReceiptPhase::Responded);
            let detail = journal
                .store
                .lock()
                .unwrap()
                .run_journal_entry(
                    &journal.scene,
                    &journal.run,
                    &page.entries[0].id,
                    page.summary.revision,
                )
                .unwrap();
            assert!(matches!(&detail.content, JournalContent::Text { text } if text == "完成"));
            let serialized = serde_json::to_string(&detail).unwrap();
            for private in [
                "SYNTHETIC_SECRET",
                "private thought",
                "opaque encrypted reasoning",
                "signed opaque state",
                "fixture-secret",
            ] {
                assert!(!serialized.contains(private));
            }
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_turn_commit_failure_prevents_first_tool_and_truncation_stays_unknown() {
        use mewu_core::ReceiptPhase;
        let fixture = Fixture::new(vec![calls(&["a"], "remember", "先说明")]);
        let journal = StoreJournal::new();
        journal.fail_complete.store(true, Ordering::SeqCst);
        let result = recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            |_, _| async { panic!("failed durable model receipt cannot dispatch a tool") },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert_eq!(journal.page().entries.len(), 1);
        assert_eq!(journal.page().entries[0].phase, ReceiptPhase::Dispatched);

        let stream = format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"content":"半段","tool_calls":[{"index":0,"id":"a","type":"function","function":{"name":"remember","arguments":"{}"}}]}}]})
        );
        let fixture = Fixture::new(vec![stream]);
        let journal = StoreJournal::new();
        assert!(recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            |_, _| async { panic!("truncated tool call cannot dispatch") }
        )
        .await
        .is_err());
        assert_eq!(journal.page().entries.len(), 1);
        assert_eq!(journal.page().entries[0].phase, ReceiptPhase::Unknown);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recorded_public_reasoning_does_not_change_answer_receipts_or_private_continuation() {
        use mewu_core::{ConnectionAuthMode, ConnectionProtocol, JournalContent};
        for (protocol, stream) in [
            (
                ConnectionProtocol::ChatCompletions,
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"正文","reasoning_content":"公开摘要🙂","encrypted_content":"opaque secret"},"finish_reason":"stop"}]})
                ),
            ),
            (
                ConnectionProtocol::OpenAiResponses,
                provider_transport::tests::responses_with_reasoning(
                    "正文",
                    None,
                    Some("公开摘要🙂"),
                ),
            ),
            (
                ConnectionProtocol::AnthropicMessages,
                provider_transport::tests::anthropic_with_reasoning(
                    "正文",
                    None,
                    Some("公开摘要🙂"),
                ),
            ),
        ] {
            let fixture = Fixture::new(vec![stream]);
            let journal = StoreJournal::new();
            let mut visible = String::new();
            let mut reasoning = String::new();
            let answer = recorded_inner_with_reasoning(
                Transport {
                    url: fixture.url.clone(),
                    protocol,
                    auth_mode: ConnectionAuthMode::None,
                },
                "",
                body(),
                vec![],
                |text| visible.push_str(text),
                |text| reasoning.push_str(text),
                journal.clone(),
                |_, _| async { panic!("no optional tool calls") },
            )
            .await
            .unwrap();
            assert_eq!(answer, "正文");
            assert_eq!(visible, answer);
            assert_eq!(reasoning, "公开摘要🙂");
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            let page = journal.page();
            let detail = journal
                .store
                .lock()
                .unwrap()
                .run_journal_entry(
                    &journal.scene,
                    &journal.run,
                    &page.entries[0].id,
                    page.summary.revision,
                )
                .unwrap();
            assert!(matches!(&detail.content,JournalContent::Text{text}if text=="正文"));
            let recorded = serde_json::to_string(&detail).unwrap();
            assert!(!recorded.contains("公开摘要"));
            assert!(!recorded.contains("opaque"));
            assert!(!recorded.contains("signed"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn complete_turn_survives_duplicate_tool_budget_rejection() {
        use mewu_core::{ReceiptKind, ReceiptPhase};
        let fixture = Fixture::new(vec![
            calls(&["same"], "not_enabled", "第一轮"),
            calls(&["same"], "not_enabled", "第二轮"),
        ]);
        let journal = StoreJournal::new();
        assert!(recorded_inner(
            Transport::chat(fixture.url.clone()),
            "",
            body(),
            tools(),
            |_| {},
            journal.clone(),
            |_, _| async { panic!("unadvertised and repeated calls must not execute") }
        )
        .await
        .is_err());
        let page = journal.page();
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == ReceiptKind::Model && e.phase == ReceiptPhase::Responded)
                .count(),
            2
        );
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == ReceiptKind::Tool && e.phase == ReceiptPhase::NotSent)
                .count(),
            2
        );
        assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_protocols_continue_tools_with_opaque_state_and_correct_auth() {
        use mewu_core::{ConnectionAuthMode as Auth, ConnectionProtocol as Protocol};
        for (protocol, first, last) in [
            (
                Protocol::OpenAiResponses,
                provider_transport::tests::responses("准备", Some("call_a")),
                provider_transport::tests::responses("完成", None),
            ),
            (
                Protocol::AnthropicMessages,
                provider_transport::tests::anthropic("准备", Some("call_a")),
                provider_transport::tests::anthropic("完成", None),
            ),
        ] {
            let fixture = Fixture::new(vec![first, last]);
            let auth = if protocol == Protocol::AnthropicMessages {
                Auth::ApiKey
            } else {
                Auth::Bearer
            };
            let transport = Transport {
                url: fixture.url.clone(),
                protocol,
                auth_mode: auth,
            };
            let mut visible = String::new();
            let mut executed = Vec::new();
            let result = run_with_transport(
                transport,
                "fixture-secret",
                body(),
                tools(),
                |delta| visible.push_str(delta),
                |call| {
                    executed.push(call);
                    async { Ok(json!({"saved":true})) }
                },
            )
            .await
            .unwrap();
            assert_eq!(result, "准备\n\n完成");
            assert_eq!(visible, result);
            assert_eq!(executed.len(), 1);
            let requests = fixture.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            let headers = fixture.headers.lock().unwrap();
            for header in headers.iter() {
                let find = |name: &str| {
                    header
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| value.as_str())
                };
                if protocol == Protocol::AnthropicMessages {
                    assert_eq!(find("x-api-key"), Some("fixture-secret"));
                    assert!(find("authorization").is_none());
                    assert_eq!(find("anthropic-version"), Some("2023-06-01"));
                } else {
                    assert_eq!(find("authorization"), Some("Bearer fixture-secret"));
                    assert!(find("x-api-key").is_none());
                }
            }
            if protocol == Protocol::OpenAiResponses {
                assert_eq!(requests[1]["store"], false);
                assert!(requests[1].get("previous_response_id").is_none());
                assert_eq!(
                    requests[1]["input"][1]["encrypted_content"],
                    "opaque encrypted reasoning"
                );
                assert_eq!(requests[1]["input"][3]["call_id"], "call_a");
                assert_eq!(requests[1]["input"][4]["type"], "function_call_output");
                assert_eq!(requests[1]["input"][4]["call_id"], "call_a");
            } else {
                assert_eq!(
                    requests[1]["messages"][1]["content"][0]["signature"],
                    "signed opaque state"
                );
                assert_eq!(
                    requests[1]["messages"][1]["content"][0]["thinking"],
                    "private thought"
                );
                assert_eq!(
                    requests[1]["messages"][2]["content"][0]["type"],
                    "tool_result"
                );
                assert_eq!(
                    requests[1]["messages"][2]["content"][0]["tool_use_id"],
                    "call_a"
                );
            }
            assert!(!serde_json::to_string(&*requests)
                .unwrap()
                .contains("fixture-secret"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_cancel_drops_pending_tool_and_auth_none_never_sends_a_key() {
        use mewu_core::{ConnectionAuthMode as Auth, ConnectionProtocol as Protocol};
        for protocol in [Protocol::OpenAiResponses, Protocol::AnthropicMessages] {
            let first = if protocol == Protocol::OpenAiResponses {
                provider_transport::tests::responses("", Some("a"))
            } else {
                provider_transport::tests::anthropic("", Some("a"))
            };
            let fixture = Fixture::new(vec![first]);
            let (send, receive) = tokio::sync::oneshot::channel();
            let mut send = Some(send);
            let mut future = Box::pin(run_with_transport(
                Transport {
                    url: fixture.url.clone(),
                    protocol,
                    auth_mode: Auth::None,
                },
                "must-not-be-sent",
                body(),
                tools(),
                |_| {},
                |_| {
                    let send = send.take().unwrap();
                    async move {
                        let _ = send.send(());
                        std::future::pending::<Result<Value, String>>().await
                    }
                },
            ));
            tokio::select! { _ = &mut future => panic!("tool must wait"), _=receive=>{} }
            drop(future);
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            let headers = fixture.headers.lock().unwrap();
            assert!(headers[0]
                .iter()
                .all(|(name, value)| name != "authorization"
                    && name != "x-api-key"
                    && value != "must-not-be-sent"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_truncated_http_streams_never_execute_tools_or_retry() {
        use mewu_core::{ConnectionAuthMode as Auth, ConnectionProtocol as Protocol};
        for (protocol, data, terminal) in [
            (
                Protocol::OpenAiResponses,
                provider_transport::tests::responses("", Some("a")),
                "event: response.completed",
            ),
            (
                Protocol::AnthropicMessages,
                provider_transport::tests::anthropic("", Some("a")),
                "event: message_stop",
            ),
        ] {
            let end = data.rfind(terminal).unwrap();
            let fixture = Fixture::new(vec![data[..end].into()]);
            let result = run_with_transport(
                Transport {
                    url: fixture.url.clone(),
                    protocol,
                    auth_mode: Auth::None,
                },
                "",
                body(),
                tools(),
                |_| {},
                |_| async { panic!("must not execute incomplete tool") },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn responses_refusal_aborts_already_accumulated_tools_without_retry() {
        use mewu_core::{ConnectionAuthMode, ConnectionProtocol};
        for refusal_kind in [
            "response.refusal.delta",
            "response.refusal.done",
            "final_content",
        ] {
            let call = json!({"id":"fc_1","type":"function_call","call_id":"a","name":"remember","arguments":"{}","status":"completed"});
            let message = json!({"id":"msg_1","type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"fixture refusal"}]});
            let mut events = vec![
                json!({"type":"response.created","response":{"id":"resp_1"}}),
                json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","call_id":"a","name":"remember","arguments":"","status":"in_progress"}}),
                json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_1","delta":"{}"}),
                json!({"type":"response.output_item.done","output_index":0,"item":call}),
                json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","role":"assistant","content":[],"status":"in_progress"}}),
            ];
            if refusal_kind == "final_content" {
                events.push(
                    json!({"type":"response.output_item.done","output_index":1,"item":message}),
                );
                events.push(json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[call,message]}}));
            } else {
                events.push(json!({"type":refusal_kind,"output_index":1,"item_id":"msg_1","content_index":0,"delta":"fixture refusal","refusal":"fixture refusal"}));
            }
            for (i, event) in events.iter_mut().enumerate() {
                event["sequence_number"] = json!(i);
            }
            let fixture = Fixture::new(vec![provider_transport::tests::sse(events)]);
            let result = run_with_transport(
                Transport {
                    url: fixture.url.clone(),
                    protocol: ConnectionProtocol::OpenAiResponses,
                    auth_mode: ConnectionAuthMode::None,
                },
                "",
                body(),
                tools(),
                |_| {},
                |_| async { panic!("a refused batch cannot dispatch tools") },
            )
            .await;
            assert_eq!(result.unwrap_err(), "模型拒绝了此请求");
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn anthropic_refusal_can_interrupt_a_tool_block_and_aborts_the_entire_batch() {
        use mewu_core::{ConnectionAuthMode, ConnectionProtocol};
        let events = vec![
            json!({"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[]}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"remember","input":{}}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"remember","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"text\":"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"refusal"}}),
        ];
        let fixture = Fixture::new(vec![provider_transport::tests::sse(events)]);
        let result = run_with_transport(
            Transport {
                url: fixture.url.clone(),
                protocol: ConnectionProtocol::AnthropicMessages,
                auth_mode: ConnectionAuthMode::None,
            },
            "",
            body(),
            tools(),
            |_| {},
            |_| async { panic!("a refused batch cannot dispatch tools") },
        )
        .await;
        assert_eq!(result.unwrap_err(), "模型拒绝了此请求");
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn consecutive_rounds_keep_protocol_reasoning_private_and_join_visible_content() {
        let first_raw = "<think>FIRST_SYNTHETIC_SECRET</think>准备";
        let second_raw = "<think>SECOND_SYNTHETIC_SECRET</think>";
        let fixture = Fixture::new(vec![
            calls(&["a"], "remember", first_raw),
            calls(&["b"], "remember", second_raw),
            final_text("<think>FINAL_SYNTHETIC_SECRET</think>完成"),
        ]);
        let mut executed = Vec::new();
        let mut visible = String::new();
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |delta| visible.push_str(delta),
            |call| {
                executed.push(call.id);
                async { Ok(json!({"saved":true})) }
            },
        )
        .await
        .unwrap();
        assert_eq!(executed, vec!["a", "b"]);
        assert_eq!(result, "准备\n\n完成");
        assert_eq!(visible, result);
        let requests = fixture.requests.lock().unwrap();
        let messages = requests[2]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(requests[1]["messages"][1]["content"], first_raw);
        assert_eq!(messages[1]["content"], first_raw);
        assert_eq!(messages[3]["content"], second_raw);
        assert_eq!(messages[1]["reasoning_content"], "opaque reasoning");
        assert_eq!(messages[3]["reasoning_content"], "opaque reasoning");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "a");
        assert_eq!(messages[4]["tool_call_id"], "b");
        assert!(!result.contains("opaque reasoning"));
        assert!(!result.contains("SYNTHETIC_SECRET"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_tools_never_dispatch_and_executor_errors_do_not_leak() {
        let fixture = Fixture::new(vec![
            calls(&["a"], "not_enabled", ""),
            calls(&["b"], "remember", ""),
            final_text("已处理"),
        ]);
        let mut executed = 0;
        run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| {
                executed += 1;
                async { Ok(json!({"error":"参数需要更新"})) }
            },
        )
        .await
        .unwrap();
        assert_eq!(executed, 1);
        let requests = fixture.requests.lock().unwrap();
        let wire = serde_json::to_string(&*requests).unwrap();
        assert!(!wire.contains("secret-api-key-and-private-path"));
        assert!(requests[1]["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("没有启用"));
        assert!(requests[2]["messages"][4]["content"]
            .as_str()
            .unwrap()
            .contains("参数需要更新"));
        drop(requests);
        let fixture = Fixture::new(vec![calls(&["a"], "remember", "")]);
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| async { Err("secret-api-key-and-private-path".into()) },
        )
        .await;
        assert_eq!(result.unwrap_err(), "工具执行已中止");
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disabled_tools_keep_legacy_done_compatibility_without_tool_fields() {
        let fixture = Fixture::new(vec!["data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"普通回复\"}}]}\n\ndata: [DONE]\n\n".into()]);
        let mut request = body();
        request["tools"] = json!([]);
        request["tool_choice"] = json!("auto");
        let answer = run(
            fixture.url.clone(),
            "",
            request,
            vec![],
            |_| {},
            |_| async { panic!("no dispatch") },
        )
        .await
        .unwrap();
        assert_eq!(answer, "普通回复");
        let requests = fixture.requests.lock().unwrap();
        assert!(requests[0].get("tools").is_none());
        assert!(requests[0].get("tool_choice").is_none());
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn repeated_call_id_is_rejected_before_a_second_side_effect() {
        let fixture = Fixture::new(vec![
            calls(&["same"], "remember", ""),
            calls(&["same"], "remember", ""),
        ]);
        let mut executed = 0;
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| {
                executed += 1;
                async { Ok(json!({})) }
            },
        )
        .await;
        assert!(result.unwrap_err().contains("重复"));
        assert_eq!(executed, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn round_and_call_budgets_stop_before_dispatching_an_excess_batch() {
        let responses = (0..6)
            .map(|index| calls(&[&format!("call{index}")], "remember", ""))
            .collect();
        let fixture = Fixture::new(responses);
        let mut executed = 0;
        assert!(run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| {
                executed += 1;
                async { Ok(json!({})) }
            }
        )
        .await
        .unwrap_err()
        .contains("轮次上限"));
        assert_eq!(executed, 5);
        let fixture = Fixture::new(vec![
            calls(&["a", "b", "c", "d", "e", "f", "g", "h"], "remember", ""),
            calls(&["i", "j", "k", "l", "m", "n", "o", "p"], "remember", ""),
            calls(&["q"], "remember", ""),
        ]);
        let mut executed = 0;
        assert!(run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| {
                executed += 1;
                async { Ok(json!({})) }
            }
        )
        .await
        .unwrap_err()
        .contains("次数上限"));
        assert_eq!(executed, 16);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_during_a_tool_drops_it_and_never_starts_another_request() {
        let fixture = Fixture::new(vec![calls(&["a"], "remember", "")]);
        let (started_send, started_receive) = tokio::sync::oneshot::channel();
        let mut started_send = Some(started_send);
        let dropped = Arc::new(AtomicUsize::new(0));
        struct MarkDrop(Arc<AtomicUsize>);
        impl Drop for MarkDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut future = Box::pin(run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| {
                let sender = started_send.take().unwrap();
                let marker = MarkDrop(dropped.clone());
                async move {
                    let _marker = marker;
                    let _ = sender.send(());
                    std::future::pending::<Result<Value, String>>().await
                }
            },
        ));
        tokio::select! {
            _ = &mut future => panic!("tool should remain pending"),
            _ = started_receive => {},
        }
        drop(future);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn oversized_tool_output_is_not_forwarded_and_total_output_is_bounded() {
        let fixture = Fixture::new(vec![calls(&["a"], "remember", ""), final_text("缩小查询")]);
        run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| async { Ok(json!("x".repeat(MAX_TOOL_OUTPUT))) },
        )
        .await
        .unwrap();
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests[1]["messages"][2]["content"]
                .as_str()
                .unwrap()
                .len()
                < 256
        );
        drop(requests);
        let fixture = Fixture::new(vec![calls(&["a", "b", "c", "d", "e"], "remember", "")]);
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| async { Ok(json!("x".repeat(127 * 1024))) },
        )
        .await;
        assert!(result.unwrap_err().contains("工具结果总量"));
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn answer_budget_is_shared_by_model_rounds_and_visible_deltas() {
        let text = "x".repeat(ai::MAX_ANSWER_BYTES / 2 + 1);
        let fixture = Fixture::new(vec![calls(&["a"], "remember", &text), final_text(&text)]);
        let mut visible = String::new();
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |delta| visible.push_str(delta),
            |_| async { Ok(json!({})) },
        )
        .await;
        assert!(result.unwrap_err().contains("回复超过"));
        assert!(visible.len() <= ai::MAX_ANSWER_BYTES);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unfinished_tool_arguments_never_dispatch() {
        let data = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"one","type":"function","function":{"name":"remember","arguments":"{"}}]}}]})
        );
        let fixture = Fixture::new(vec![data]);
        let result = run(
            fixture.url.clone(),
            "",
            body(),
            tools(),
            |_| {},
            |_| async { panic!("partial arguments must never execute") },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    }
}
