// SPDX-License-Identifier: MPL-2.0
//! Native protocol adapters. Opaque continuation stays private; public provider
//! reasoning text has an independent, explicit display callback.
//! https://developers.openai.com/api/docs/guides/migrate-to-responses
//! https://developers.openai.com/api/reference/resources/responses/streaming-events
//! https://platform.claude.com/docs/en/build-with-claude/streaming
use crate::ai::{self, ModelToolCall, ModelTurn};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use mewu_core::{ConnectionAuthMode, ConnectionProfile, ConnectionProtocol};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

const MAX_WIRE: usize = 32 * 1024 * 1024;
const MAX_CONTINUATION: usize = 4 * 1024 * 1024;
const MAX_ARGS: usize = 16 * 1024;
const MAX_TOTAL_ARGS: usize = 64 * 1024;
const MAX_BLOCKS: usize = 128;
const REFUSAL_ERROR: &str = "模型拒绝了此请求";

#[derive(Debug, Clone)]
pub struct Transport {
    pub url: url::Url,
    pub protocol: ConnectionProtocol,
    pub auth_mode: ConnectionAuthMode,
}
impl Transport {
    pub fn chat(url: url::Url) -> Self {
        Self {
            url,
            protocol: ConnectionProtocol::ChatCompletions,
            auth_mode: ConnectionAuthMode::Bearer,
        }
    }
}

fn base_url(profile: &ConnectionProfile) -> Result<url::Url, String> {
    let url = url::Url::parse(profile.base_url.trim()).map_err(|_| "连接地址无效")?;
    let local = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("连接需要 HTTPS，只有本机服务允许 HTTP；地址不能包含凭据或查询参数".into());
    }
    Ok(url)
}
fn api_prefix(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    for suffix in ["/chat/completions", "/responses", "/messages"] {
        if let Some(prefix) = path.strip_suffix(suffix) {
            return prefix;
        }
    }
    path
}
pub fn endpoint(profile: &ConnectionProfile) -> Result<url::Url, String> {
    let mut url = base_url(profile)?;
    if let Some(path) = profile.advanced.request_path.as_deref() {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.contains(['?', '#', '\\'])
            || path.chars().any(char::is_control)
            || path.split('/').any(|p| matches!(p, "." | ".."))
        {
            return Err("自定义请求路径必须是以 / 开头的站内路径".into());
        }
        url.set_path(path);
    } else {
        let suffix = match profile.advanced.protocol {
            ConnectionProtocol::ChatCompletions => "chat/completions",
            ConnectionProtocol::AnthropicMessages => "messages",
            ConnectionProtocol::OpenAiResponses => "responses",
        };
        url.set_path(&format!("{}/{suffix}", api_prefix(url.path())));
    }
    Ok(url)
}
pub fn models_endpoint(profile: &ConnectionProfile) -> Result<url::Url, String> {
    let mut url = base_url(profile)?;
    url.set_path(&format!("{}/models", api_prefix(url.path())));
    Ok(url)
}
pub fn authenticate(
    mut request: reqwest::RequestBuilder,
    transport: &Transport,
    key: &str,
) -> reqwest::RequestBuilder {
    if transport.protocol == ConnectionProtocol::AnthropicMessages {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if !key.is_empty() {
        request = match transport.auth_mode {
            ConnectionAuthMode::Bearer => request.bearer_auth(key),
            ConnectionAuthMode::ApiKey => request.header("x-api-key", key),
            ConnectionAuthMode::None => request,
        };
    }
    request
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| "服务返回了不完整的协议字段".into())
}
fn append(target: &mut String, fragment: &str, limit: usize) -> Result<(), String> {
    if target.len().saturating_add(fragment.len()) > limit {
        return Err("回复内容超过长度上限".into());
    }
    target.push_str(fragment);
    Ok(())
}
fn content_parts(
    content: &Value,
    protocol: ConnectionProtocol,
    assistant: bool,
) -> Result<Vec<Value>, String> {
    let mut parts = if let Some(text) = content.as_str() {
        vec![json!({"type":"text","text":text})]
    } else {
        content.as_array().ok_or("消息内容格式无效")?.clone()
    };
    // Image-only turns contain an empty canonical text part; Claude rejects empty text blocks.
    if protocol == ConnectionProtocol::AnthropicMessages {
        parts.retain(|part| !(part["type"] == "text" && part["text"] == ""));
        if parts.is_empty() {
            return Err("此协议不接受空消息".into());
        }
    }
    parts.iter().map(|part| match string(part, "type")? {
        "text" => Ok(json!({"type":if protocol == ConnectionProtocol::OpenAiResponses { if assistant {"output_text"} else {"input_text"} } else {"text"}, "text":string(part,"text")?})),
        "image_url" if !assistant => {
            let data = part.pointer("/image_url/url").and_then(Value::as_str).ok_or("图片引用格式无效")?;
            let (mime, bytes) = data.strip_prefix("data:").and_then(|v| v.split_once(";base64,")).ok_or("仅支持本次消息内的图片数据")?;
            if !matches!(mime, "image/png" | "image/jpeg") || bytes.is_empty() { return Err("图片格式不受支持".into()); }
            if protocol == ConnectionProtocol::OpenAiResponses {
                Ok(json!({"type":"input_image","image_url":data}))
            } else { Ok(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":bytes}})) }
        }
        _ => Err("消息内容类型不受支持".into()),
    }).collect()
}

/// Convert the host's canonical messages once. Later rounds extend this native body.
pub fn request_body(protocol: ConnectionProtocol, canonical: Value) -> Result<Value, String> {
    for (name, maximum) in [
        (
            "temperature",
            if protocol == ConnectionProtocol::AnthropicMessages {
                1.0
            } else {
                2.0
            },
        ),
        ("top_p", 1.0),
    ] {
        if let Some(value) = canonical.get(name) {
            if !value
                .as_f64()
                .is_some_and(|v| v.is_finite() && v >= 0.0 && v <= maximum)
            {
                return Err(format!("此协议的 {name} 必须在 0 到 {maximum} 之间"));
            }
        }
    }
    if let Some(value) = canonical.get("service_tier") {
        if !value
            .as_str()
            .is_some_and(|v| matches!(v, "standard" | "default" | "priority"))
        {
            return Err("service_tier 参数无效".into());
        }
    }
    if protocol == ConnectionProtocol::ChatCompletions {
        return Ok(canonical);
    }
    let messages = canonical
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("模型请求缺少消息")?;
    let mut body = json!({"model":string(&canonical,"model")?,"stream":true});
    for key in ["temperature", "top_p", "service_tier"] {
        if let Some(value) = canonical.get(key) {
            body[key] = value.clone();
        }
    }
    // Our user-facing tiers are provider-independent, wire enum values are not.
    if let Some(tier) = body.get("service_tier").and_then(Value::as_str) {
        body["service_tier"] = json!(match (protocol, tier) {
            (ConnectionProtocol::AnthropicMessages, "standard" | "default") => "standard_only",
            (ConnectionProtocol::AnthropicMessages, "priority") =>
                return Err("此协议不支持 priority，请删除此参数".into()),
            (ConnectionProtocol::OpenAiResponses, "standard") => "default",
            (_, other) => other,
        });
    }
    let mut input = Vec::new();
    let mut system = Vec::new();
    for message in messages {
        let role = string(message, "role")?;
        if !matches!(role, "system" | "developer" | "user" | "assistant") {
            return Err("初始消息包含不支持的角色".into());
        }
        let parts = content_parts(
            message.get("content").ok_or("消息缺少内容")?,
            protocol,
            role == "assistant",
        )?;
        if protocol == ConnectionProtocol::AnthropicMessages
            && matches!(role, "system" | "developer")
        {
            if parts.iter().any(|p| p["type"] != "text") {
                return Err("系统提示只接受文本".into());
            }
            system.extend(parts);
        } else if protocol == ConnectionProtocol::OpenAiResponses && role == "assistant" {
            // EasyInputMessage's plain-string form avoids inventing output item IDs.
            // Native outputs from this run are replayed intact by append_turn instead.
            let text = parts
                .iter()
                .map(|part| string(part, "text"))
                .collect::<Result<Vec<_>, _>>()?
                .join("\n");
            input.push(json!({"role":role,"content":text}));
        } else {
            input.push(json!({"role":role,"content":parts}));
        }
    }
    if protocol == ConnectionProtocol::OpenAiResponses {
        body["input"] = json!(input);
        body["store"] = json!(false);
        body["include"] = json!(["reasoning.encrypted_content"]);
    } else {
        body["messages"] = json!(input);
        body["max_tokens"] = json!(4096);
        if !system.is_empty() {
            body["system"] = json!(system);
        }
    }
    if let Some(tools) = canonical.get("tools").and_then(Value::as_array) {
        let mut converted = Vec::new();
        for tool in tools {
            let function = tool.get("function").ok_or("工具定义无效")?;
            let schema = function
                .get("parameters")
                .filter(|v| v.is_object())
                .ok_or("工具参数定义无效")?;
            let name = string(function, "name")?;
            let description = function
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            converted.push(if protocol == ConnectionProtocol::OpenAiResponses {
                json!({"type":"function","name":name,"description":description,"parameters":schema,"strict":false})
            } else { json!({"name":name,"description":description,"input_schema":schema}) });
        }
        if !converted.is_empty() {
            body["tools"] = json!(converted);
        }
    }
    Ok(body)
}

pub fn append_turn(
    protocol: ConnectionProtocol,
    body: &mut Value,
    turn: &ModelTurn,
) -> Result<(), String> {
    let key = if protocol == ConnectionProtocol::OpenAiResponses {
        "input"
    } else {
        "messages"
    };
    let messages = body
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or("模型请求缺少消息")?;
    match protocol {
        ConnectionProtocol::ChatCompletions => {
            let calls: Vec<Value> = turn.tool_calls.iter().map(|call| json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}})).collect();
            let content = turn.provider_continuation.as_ref().and_then(|value| value.get("content")).and_then(Value::as_str).unwrap_or(&turn.content);
            let mut message = json!({"role":"assistant","content":content,"tool_calls":calls});
            if let Some(reasoning) = &turn.reasoning_content { message["reasoning_content"] = json!(reasoning); }
            messages.push(message);
        }
        ConnectionProtocol::OpenAiResponses => messages.extend(turn.provider_continuation.as_ref().and_then(Value::as_array).ok_or("回复缺少续接状态")?.iter().cloned()),
        ConnectionProtocol::AnthropicMessages => messages.push(json!({"role":"assistant","content":turn.provider_continuation.as_ref().and_then(Value::as_array).ok_or("回复缺少续接状态")?})),
    }
    Ok(())
}
pub fn append_result(
    protocol: ConnectionProtocol,
    body: &mut Value,
    call_id: &str,
    content: String,
) -> Result<(), String> {
    let key = if protocol == ConnectionProtocol::OpenAiResponses {
        "input"
    } else {
        "messages"
    };
    let messages = body
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or("模型请求缺少消息")?;
    match protocol {
        ConnectionProtocol::ChatCompletions => {
            messages.push(json!({"role":"tool","tool_call_id":call_id,"content":content}))
        }
        ConnectionProtocol::OpenAiResponses => {
            messages.push(json!({"type":"function_call_output","call_id":call_id,"output":content}))
        }
        ConnectionProtocol::AnthropicMessages => {
            let result = json!({"type":"tool_result","tool_use_id":call_id,"content":content});
            if let Some(previous) = messages.last_mut().filter(|p| p["role"] == "user") {
                previous
                    .get_mut("content")
                    .and_then(Value::as_array_mut)
                    .ok_or("工具结果格式无效")?
                    .push(result);
            } else {
                messages.push(json!({"role":"user","content":[result]}));
            }
        }
    }
    Ok(())
}

pub async fn stream_turn(
    transport: &Transport,
    key: &str,
    body: Value,
    on_delta: impl FnMut(&str),
) -> Result<ModelTurn, String> {
    stream_turn_before_send(transport, key, body, on_delta, || Ok(())).await
}

/// The gate runs after local HTTP construction and before its first send. A
/// journaled Run uses it to persist dispatch; non-Run callers use the wrapper.
pub async fn stream_turn_before_send(
    transport: &Transport,
    key: &str,
    body: Value,
    on_delta: impl FnMut(&str),
    before_send: impl FnOnce() -> Result<(), String> + Send,
) -> Result<ModelTurn, String> {
    stream_turn_with_reasoning_before_send(transport, key, body, on_delta, |_| {}, before_send)
        .await
}

pub async fn stream_turn_with_reasoning_before_send(
    transport: &Transport,
    key: &str,
    body: Value,
    on_delta: impl FnMut(&str),
    on_reasoning_delta: impl FnMut(&str),
    before_send: impl FnOnce() -> Result<(), String> + Send,
) -> Result<ModelTurn, String> {
    if serde_json::to_vec(&body).map_err(|_| "模型请求无效")?.len() > MAX_WIRE {
        return Err("模型请求过大".into());
    }
    let has_tools = body.get("tools").is_some();
    let response =
        ai::request_stream_with_transport_before_send(transport, key, body, has_tools, before_send)
            .await?;
    let turn = if transport.protocol == ConnectionProtocol::ChatCompletions {
        ai::consume_turn_bytes_with_reasoning(
            response.bytes_stream(),
            on_delta,
            on_reasoning_delta,
            has_tools,
        )
        .await?
    } else {
        consume_with_reasoning(
            response.bytes_stream(),
            transport.protocol,
            on_delta,
            on_reasoning_delta,
        )
        .await?
    };
    if !has_tools && !turn.tool_calls.is_empty() {
        return Err("此请求没有启用工具".into());
    }
    Ok(turn)
}

fn validate_turn(turn: &ModelTurn) -> Result<(), String> {
    if turn.content.len() > ai::MAX_ANSWER_BYTES || turn.tool_calls.len() > ai::MAX_TURN_CALLS {
        return Err("回复超过长度上限".into());
    }
    let mut ids = HashSet::new();
    let mut total = 0;
    for call in &turn.tool_calls {
        total += call.arguments.len();
        if !ai::valid_tool_name(&call.name)
            || call.id.is_empty()
            || call.id.len() > 128
            || !call
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || !ids.insert(&call.id)
            || call.arguments.len() > MAX_ARGS
            || total > MAX_TOTAL_ARGS
            || !serde_json::from_str::<Value>(&call.arguments).is_ok_and(|v| v.is_object())
        {
            return Err("工具调用未完整接收或格式无效".into());
        }
    }
    if turn.content.trim().is_empty() && turn.tool_calls.is_empty() {
        return Err("服务没有返回正文".into());
    }
    if serde_json::to_vec(&turn.provider_continuation)
        .map_err(|_| "续接状态无效")?
        .len()
        > MAX_CONTINUATION
    {
        return Err("续接状态超过长度上限".into());
    }
    Ok(())
}

#[derive(Default)]
struct Responses {
    id: Option<String>,
    sequence: Option<u64>,
    text: String,
    items: BTreeMap<usize, Value>,
    done: HashSet<usize>,
    arguments: BTreeMap<usize, String>,
    reasoning_bytes: usize,
}
impl Responses {
    fn event(
        &mut self,
        event: &Value,
        delta: &mut impl FnMut(&str),
        reasoning_delta: &mut impl FnMut(&str),
    ) -> Result<Option<ModelTurn>, String> {
        let kind = string(event, "type")?;
        if kind == "error" || matches!(kind, "response.failed" | "response.incomplete") {
            return Err("服务未完整完成回复".into());
        }
        let seq = event
            .get("sequence_number")
            .and_then(Value::as_u64)
            .ok_or("回复缺少事件序号")?;
        if self.sequence.is_some_and(|old| seq <= old) {
            return Err("回复事件顺序无效".into());
        }
        self.sequence = Some(seq);
        if kind == "response.created" {
            if self.id.is_some() {
                return Err("回复重复开始".into());
            }
            self.id = Some(string(event.get("response").ok_or("回复缺少标识")?, "id")?.to_string());
            return Ok(None);
        }
        if self.id.is_none() {
            return Err("回复缺少开始事件".into());
        }
        match kind {
            "response.refusal.delta" | "response.refusal.done" => return Err(REFUSAL_ERROR.into()),
            "response.output_item.added" => {
                let index = index(event, "output_index")?;
                let item = event
                    .get("item")
                    .filter(|i| i.is_object())
                    .ok_or("回复项无效")?;
                if index != self.items.len() || self.items.insert(index, item.clone()).is_some() {
                    return Err("回复项顺序无效".into());
                }
            }
            "response.output_text.delta" => {
                self.item(event, "message")?;
                let text = string(event, "delta")?;
                append(&mut self.text, text, ai::MAX_ANSWER_BYTES)?;
                delta(text);
            }
            "response.function_call_arguments.delta" => {
                let index = self.item(event, "function_call")?;
                append(
                    self.arguments.entry(index).or_default(),
                    string(event, "delta")?,
                    MAX_ARGS,
                )?;
            }
            "response.function_call_arguments.done" => {
                let index = self.item(event, "function_call")?;
                let args = string(event, "arguments")?;
                if self.arguments.get(&index).is_some_and(|s| s != args) {
                    return Err("工具参数前后不一致".into());
                }
                self.arguments.insert(index, args.to_string());
            }
            "response.output_item.done" => {
                let index = index(event, "output_index")?;
                let item = event.get("item").ok_or("回复项无效")?;
                let old = self.items.get(&index).ok_or("回复项缺少开始事件")?;
                if old["id"] != item["id"]
                    || old["type"] != item["type"]
                    || !self.done.insert(index)
                {
                    return Err("回复项结束状态无效".into());
                }
                if item["type"] == "function_call"
                    && (old["call_id"] != item["call_id"]
                        || old["name"] != item["name"]
                        || self
                            .arguments
                            .get(&index)
                            .is_some_and(|a| item["arguments"].as_str() != Some(a)))
                {
                    return Err("工具调用前后不一致".into());
                }
                self.items.insert(index, item.clone());
            }
            "response.completed" => {
                let response = event.get("response").ok_or("回复结束状态无效")?;
                if response["id"].as_str() != self.id.as_deref()
                    || response["status"] != "completed"
                    || response.get("error").is_some_and(|e| !e.is_null())
                {
                    return Err("回复结束状态无效".into());
                }
                let output = response
                    .get("output")
                    .and_then(Value::as_array)
                    .ok_or("回复缺少完整输出")?;
                if output.len() != self.items.len()
                    || self.done.len() != self.items.len()
                    || output
                        .iter()
                        .enumerate()
                        .any(|(i, item)| self.items.get(&i) != Some(item))
                {
                    return Err("回复输出未完整接收".into());
                }
                let mut turn = ModelTurn::default();
                for item in output {
                    match string(item, "type")? {
                        "message" => {
                            if item["role"] != "assistant" || item["status"] != "completed" {
                                return Err("回复消息状态无效".into());
                            }
                            for part in item
                                .get("content")
                                .and_then(Value::as_array)
                                .ok_or("回复消息内容无效")?
                            {
                                if part["type"] == "refusal" {
                                    return Err(REFUSAL_ERROR.into());
                                }
                                if part["type"] != "output_text" {
                                    return Err("服务未返回可用正文".into());
                                }
                                append(
                                    &mut turn.content,
                                    string(part, "text")?,
                                    ai::MAX_ANSWER_BYTES,
                                )?;
                            }
                        }
                        "function_call" => {
                            if item
                                .get("status")
                                .is_some_and(|status| status != "completed")
                            {
                                return Err("工具调用未完成".into());
                            }
                            turn.tool_calls.push(ModelToolCall {
                                id: string(item, "call_id")?.into(),
                                name: string(item, "name")?.into(),
                                arguments: string(item, "arguments")?.into(),
                            });
                        }
                        "reasoning" => {
                            if item
                                .get("encrypted_content")
                                .is_some_and(|e| !e.is_null() && !e.is_string())
                            {
                                return Err("推理续接状态无效".into());
                            }
                        }
                        _ => return Err("服务返回了未启用的输出类型".into()),
                    }
                }
                if turn.content != self.text {
                    return Err("回复正文未完整接收".into());
                }
                turn.provider_continuation = Some(json!(output));
                validate_turn(&turn)?;
                return Ok(Some(turn));
            }
            "response.reasoning_summary_text.delta" => {
                self.item(event, "reasoning")?;
                index(event, "summary_index")?;
                let text = string(event, "delta")?;
                self.reasoning_bytes = self.reasoning_bytes.saturating_add(text.len());
                if self.reasoning_bytes > ai::MAX_ANSWER_BYTES {
                    return Err("思考文字超过长度上限".into());
                }
                if !text.is_empty() {
                    reasoning_delta(text);
                }
            }
            // Metadata/opaque events never enter either public text channel.
            "response.in_progress"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.output_text.annotation.added"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done" => {}
            _ => return Err("服务返回了不支持的回复事件".into()),
        }
        Ok(None)
    }
    fn item(&self, event: &Value, kind: &str) -> Result<usize, String> {
        let index = index(event, "output_index")?;
        let item = self.items.get(&index).ok_or("回复项缺少开始事件")?;
        if item["type"] != kind || item["id"] != event["item_id"] || self.done.contains(&index) {
            return Err("回复增量不属于活动输出项".into());
        }
        Ok(index)
    }
}
fn index(event: &Value, name: &str) -> Result<usize, String> {
    let index = event
        .get(name)
        .and_then(Value::as_u64)
        .ok_or("回复缺少内容索引")?;
    if index >= MAX_BLOCKS as u64 {
        return Err("回复内容项过多".into());
    }
    Ok(index as usize)
}

#[derive(Default)]
struct Anthropic {
    started: bool,
    blocks: Vec<Value>,
    active: Option<usize>,
    arguments: String,
    stop: Option<String>,
    text: String,
    reasoning_bytes: usize,
}
impl Anthropic {
    fn event(
        &mut self,
        event: &Value,
        delta: &mut impl FnMut(&str),
        reasoning_delta: &mut impl FnMut(&str),
    ) -> Result<Option<ModelTurn>, String> {
        let kind = string(event, "type")?;
        if kind == "ping" {
            return Ok(None);
        }
        if kind == "error" {
            return Err("服务未完整完成回复".into());
        }
        if kind == "message_start" {
            if self.started
                || event.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
                || !event
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
            {
                return Err("回复开始状态无效".into());
            }
            self.started = true;
            return Ok(None);
        }
        if !self.started {
            return Err("回复缺少开始事件".into());
        }
        match kind {
            "content_block_start" => {
                let i = index(event, "index")?;
                if self.active.is_some() || self.stop.is_some() || i != self.blocks.len() {
                    return Err("回复内容块顺序无效".into());
                }
                let block = event
                    .get("content_block")
                    .filter(|v| v.is_object())
                    .ok_or("回复内容块无效")?
                    .clone();
                match string(&block, "type")? {
                    "text" => {
                        let text = string(&block, "text")?;
                        append(&mut self.text, text, ai::MAX_ANSWER_BYTES)?;
                        delta(text);
                    }
                    "tool_use" => {
                        string(&block, "id")?;
                        string(&block, "name")?;
                        if !block.get("input").is_some_and(Value::is_object) {
                            return Err("工具参数格式无效".into());
                        }
                    }
                    "thinking" => {
                        string(&block, "thinking")?;
                    }
                    "redacted_thinking" => {
                        string(&block, "data")?;
                    }
                    _ => return Err("服务返回了未启用的内容类型".into()),
                }
                self.blocks.push(block);
                self.active = Some(i);
                self.arguments.clear();
            }
            "content_block_delta" => {
                let i = index(event, "index")?;
                if self.active != Some(i) {
                    return Err("回复增量不属于活动内容块".into());
                }
                let d = event.get("delta").ok_or("回复缺少增量")?;
                let block = &mut self.blocks[i];
                let (field, value) = match (string(block, "type")?, string(d, "type")?) {
                    ("text", "text_delta") => {
                        let t = string(d, "text")?;
                        append(&mut self.text, t, ai::MAX_ANSWER_BYTES)?;
                        delta(t);
                        ("text", t)
                    }
                    ("tool_use", "input_json_delta") => {
                        append(&mut self.arguments, string(d, "partial_json")?, MAX_ARGS)?;
                        return Ok(None);
                    }
                    ("thinking", "thinking_delta") => {
                        let text = string(d, "thinking")?;
                        self.reasoning_bytes = self.reasoning_bytes.saturating_add(text.len());
                        if self.reasoning_bytes > ai::MAX_ANSWER_BYTES {
                            return Err("思考文字超过长度上限".into());
                        }
                        if !text.is_empty() {
                            reasoning_delta(text);
                        }
                        ("thinking", text)
                    }
                    ("thinking", "signature_delta") => ("signature", string(d, "signature")?),
                    _ => return Err("回复增量类型不匹配".into()),
                };
                let mut text = block
                    .get(field)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                append(&mut text, value, ai::MAX_ANSWER_BYTES)?;
                block[field] = json!(text);
            }
            "content_block_stop" => {
                let i = index(event, "index")?;
                if self.active != Some(i) {
                    return Err("回复内容块结束顺序无效".into());
                }
                let block = &mut self.blocks[i];
                if block["type"] == "tool_use" && !self.arguments.is_empty() {
                    let input: Value =
                        serde_json::from_str(&self.arguments).map_err(|_| "工具参数未完整接收")?;
                    if !input.is_object() {
                        return Err("工具参数必须是对象".into());
                    }
                    block["input"] = input;
                }
                if block["type"] == "thinking"
                    && !block
                        .get("signature")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                {
                    return Err("推理续接签名未完整接收".into());
                }
                self.active = None;
            }
            "message_delta" => {
                // The safety classifier can interrupt an unfinished tool block.
                // Refusal aborts the whole turn before any accumulated tool dispatch.
                if event.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("refusal") {
                    return Err(REFUSAL_ERROR.into());
                }
                if self.active.is_some() {
                    return Err("回复内容块尚未结束".into());
                }
                if let Some(reason) = event.pointer("/delta/stop_reason").filter(|v| !v.is_null()) {
                    if self.stop.is_some() {
                        return Err("回复重复结束".into());
                    }
                    self.stop = Some(reason.as_str().ok_or("回复结束状态无效")?.into());
                }
            }
            "message_stop" => {
                if self.active.is_some() {
                    return Err("回复内容未完整接收".into());
                }
                let mut turn = ModelTurn {
                    content: self.text.clone(),
                    ..Default::default()
                };
                for block in &self.blocks {
                    if block["type"] == "tool_use" {
                        turn.tool_calls.push(ModelToolCall {
                            id: string(block, "id")?.into(),
                            name: string(block, "name")?.into(),
                            arguments: serde_json::to_string(&block["input"])
                                .map_err(|_| "工具参数无效")?,
                        });
                    }
                }
                match self.stop.as_deref() {
                    Some("end_turn" | "stop_sequence") if turn.tool_calls.is_empty() => {}
                    Some("tool_use") if !turn.tool_calls.is_empty() => {}
                    Some("max_tokens") => return Err("回复达到模型长度限制".into()),
                    _ => return Err("回复未完整完成或结束状态无效".into()),
                }
                turn.provider_continuation = Some(json!(self.blocks));
                validate_turn(&turn)?;
                return Ok(Some(turn));
            }
            _ => return Err("服务返回了不支持的回复事件".into()),
        }
        Ok(None)
    }
}

async fn consume<S, B, E>(
    bytes: S,
    protocol: ConnectionProtocol,
    delta: impl FnMut(&str),
) -> Result<ModelTurn, String>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    consume_with_reasoning(bytes, protocol, delta, |_| {}).await
}

async fn consume_with_reasoning<S, B, E>(
    bytes: S,
    protocol: ConnectionProtocol,
    mut delta: impl FnMut(&str),
    mut reasoning_delta: impl FnMut(&str),
) -> Result<ModelTurn, String>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    let mut total = 0usize;
    let bounded = bytes.map(move |chunk| -> Result<B, &'static str> {
        let chunk = chunk.map_err(|_| "回复连接中断")?;
        total = total.saturating_add(chunk.as_ref().len());
        if total > MAX_WIRE {
            return Err("回复数据超过上限");
        }
        Ok(chunk)
    });
    let events = bounded.eventsource();
    futures_util::pin_mut!(events);
    let mut responses = Responses::default();
    let mut anthropic = Anthropic::default();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(90), events.next())
            .await
            .map_err(|_| "等待回复超时")?
            .ok_or("回复未完整接收")?
            .map_err(|_| "回复连接中断")?;
        if event.data.len() > MAX_CONTINUATION {
            return Err("回复事件过大".into());
        }
        let value: Value =
            serde_json::from_str(&event.data).map_err(|_| "服务返回了无效的回复格式")?;
        let kind = string(&value, "type")?;
        if !event.event.is_empty() && event.event != "message" && event.event != kind {
            return Err("回复事件类型不一致".into());
        }
        let turn = if protocol == ConnectionProtocol::OpenAiResponses {
            responses.event(&value, &mut delta, &mut reasoning_delta)?
        } else {
            anthropic.event(&value, &mut delta, &mut reasoning_delta)?
        };
        if let Some(turn) = turn {
            return Ok(turn);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn sse(events: Vec<Value>) -> String {
        events
            .iter()
            .map(|value| {
                format!(
                    "event: {}\ndata: {value}\n\n",
                    value["type"].as_str().unwrap()
                )
            })
            .collect()
    }
    pub(crate) fn responses(text: &str, call: Option<&str>) -> String {
        responses_with_reasoning(text, call, None)
    }
    pub(crate) fn responses_with_reasoning(
        text: &str,
        call: Option<&str>,
        summary: Option<&str>,
    ) -> String {
        let mut events = vec![json!({"type":"response.created","response":{"id":"resp_1"}})];
        let mut output = Vec::new();
        if call.is_some() || summary.is_some() {
            let reasoning = json!({"id":"rs_1","type":"reasoning","summary":summary.map(|text|vec![json!({"type":"summary_text","text":text})]).unwrap_or_default(),"encrypted_content":"opaque encrypted reasoning"});
            events.push(json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}));
            if let Some(text) = summary {
                events.push(json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"rs_1","summary_index":0,"delta":text}));
                events.push(json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"rs_1","content_index":0,"delta":"opaque raw internal text"}));
            }
            events.push(
                json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            );
            output.push(reasoning);
        }
        if !text.is_empty() {
            let i = output.len();
            let item = json!({"id":"msg_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]});
            events.push(json!({"type":"response.output_item.added","output_index":i,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}));
            events.push(json!({"type":"response.output_text.delta","output_index":i,"item_id":"msg_1","content_index":0,"delta":text}));
            events.push(json!({"type":"response.output_item.done","output_index":i,"item":item}));
            output.push(item);
        }
        if let Some(call) = call {
            let i = output.len();
            let item = json!({"id":"fc_1","type":"function_call","call_id":call,"name":"remember","arguments":"{\"text\":\"记住\"}","status":"completed"});
            events.push(json!({"type":"response.output_item.added","output_index":i,"item":{"id":"fc_1","type":"function_call","call_id":call,"name":"remember","arguments":"","status":"in_progress"}}));
            events.push(json!({"type":"response.function_call_arguments.delta","output_index":i,"item_id":"fc_1","delta":"{\"text\":"}));
            events.push(json!({"type":"response.function_call_arguments.delta","output_index":i,"item_id":"fc_1","delta":"\"记住\"}"}));
            events.push(json!({"type":"response.function_call_arguments.done","output_index":i,"item_id":"fc_1","arguments":item["arguments"]}));
            events.push(json!({"type":"response.output_item.done","output_index":i,"item":item}));
            output.push(item);
        }
        events.push(json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":output,"error":null}}));
        for (i, event) in events.iter_mut().enumerate() {
            event["sequence_number"] = json!(i);
        }
        sse(events)
    }
    pub(crate) fn anthropic(text: &str, call: Option<&str>) -> String {
        anthropic_with_reasoning(text, call, None)
    }
    pub(crate) fn anthropic_with_reasoning(
        text: &str,
        call: Option<&str>,
        thinking: Option<&str>,
    ) -> String {
        let mut events = vec![
            json!({"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[]}}),
        ];
        let mut i = 0;
        if call.is_some() || thinking.is_some() {
            events.extend([
                json!({"type":"content_block_start","index":i,"content_block":{"type":"thinking","thinking":""}}),
                json!({"type":"content_block_delta","index":i,"delta":{"type":"thinking_delta","thinking":thinking.unwrap_or("private thought")}}),
                json!({"type":"content_block_delta","index":i,"delta":{"type":"signature_delta","signature":"signed opaque state"}}),
                json!({"type":"content_block_stop","index":i}),
            ]);
            i += 1;
        }
        if !text.is_empty() {
            events.extend([
                json!({"type":"content_block_start","index":i,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":i,"delta":{"type":"text_delta","text":text}}),
                json!({"type":"content_block_stop","index":i}),
            ]);
            i += 1;
        }
        if let Some(call) = call {
            events.extend([
                json!({"type":"content_block_start","index":i,"content_block":{"type":"tool_use","id":call,"name":"remember","input":{}}}),
                json!({"type":"content_block_delta","index":i,"delta":{"type":"input_json_delta","partial_json":"{\"text\":"}}),
                json!({"type":"content_block_delta","index":i,"delta":{"type":"input_json_delta","partial_json":"\"记住\"}"}}),
                json!({"type":"content_block_stop","index":i}),
            ]);
        }
        events.extend([
            json!({"type":"message_delta","delta":{"stop_reason":if call.is_some(){"tool_use"}else{"end_turn"}}}),
            json!({"type":"message_stop"}),
        ]);
        sse(events)
    }
    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            id: "test".into(),
            name: "test".into(),
            provider_id: "custom".into(),
            base_url: "https://example.com/api/v3/chat/completions/".into(),
            model: "test".into(),
            has_key: false,
            credential_id: None,
            revision: 1,
            advanced: Default::default(),
        }
    }
    #[test]
    fn endpoints_preserve_prefix_and_custom_path_cannot_change_origin() {
        let mut p = profile();
        for (protocol, suffix) in [
            (ConnectionProtocol::ChatCompletions, "chat/completions"),
            (ConnectionProtocol::AnthropicMessages, "messages"),
            (ConnectionProtocol::OpenAiResponses, "responses"),
        ] {
            p.advanced.protocol = protocol;
            assert_eq!(
                endpoint(&p).unwrap().as_str(),
                format!("https://example.com/api/v3/{suffix}")
            );
            assert_eq!(
                models_endpoint(&p).unwrap().as_str(),
                "https://example.com/api/v3/models"
            );
        }
        p.advanced.request_path = Some("/v1/messages".into());
        assert_eq!(
            endpoint(&p).unwrap().as_str(),
            "https://example.com/v1/messages"
        );
        for invalid in [
            "https://evil.invalid/messages",
            "//evil.invalid/messages",
            "/v1/../messages",
            "/v1/messages?key=bad",
        ] {
            p.advanced.request_path = Some(invalid.into());
            assert!(endpoint(&p).is_err());
        }
        p.advanced.request_path = None;
        p.base_url = "http://192.0.2.1/v1".into();
        assert!(endpoint(&p).is_err());
        p.base_url = "http://127.0.0.1:1234/v1".into();
        assert!(endpoint(&p).is_ok());
    }
    #[test]
    fn native_bodies_preserve_message_attachments_and_reject_unsupported_priority() {
        let canonical = json!({"model":"fixture","temperature":0.3,"top_p":0.9,"service_tier":"standard","messages":[{"role":"system","content":"system"},{"role":"user","content":[{"type":"text","text":"old image"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}]},{"role":"assistant","content":"old answer"},{"role":"user","content":"now"}],"tools":[{"type":"function","function":{"name":"remember","parameters":{"type":"object"}}}]});
        let r = request_body(ConnectionProtocol::OpenAiResponses, canonical.clone()).unwrap();
        assert_eq!(r["store"], false);
        assert_eq!(r["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(r["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(r["input"][2]["content"], "old answer");
        assert_eq!(r["input"][3]["content"].as_array().unwrap().len(), 1);
        assert_eq!(r["tools"][0]["strict"], false);
        assert_eq!(r["service_tier"], "default");
        let a = request_body(ConnectionProtocol::AnthropicMessages, canonical.clone()).unwrap();
        assert_eq!(a["system"][0]["text"], "system");
        assert_eq!(a["max_tokens"], 4096);
        assert_eq!(
            a["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
        assert_eq!(a["messages"][2]["content"].as_array().unwrap().len(), 1);
        assert_eq!(a["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(a["service_tier"], "standard_only");
        let mut priority = canonical;
        priority["service_tier"] = json!("priority");
        assert!(
            request_body(ConnectionProtocol::AnthropicMessages, priority)
                .unwrap_err()
                .contains("priority")
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn native_streams_accept_split_utf8_and_keep_reasoning_out_of_visible_text() {
        for (protocol, data) in [
            (
                ConnectionProtocol::OpenAiResponses,
                responses("准备中文", Some("a")),
            ),
            (
                ConnectionProtocol::AnthropicMessages,
                anthropic("准备中文", Some("a")),
            ),
        ] {
            let chunks: Vec<Result<Vec<u8>, String>> = data.bytes().map(|b| Ok(vec![b])).collect();
            let mut visible = String::new();
            let turn = consume(futures_util::stream::iter(chunks), protocol, |text| {
                visible.push_str(text)
            })
            .await
            .unwrap();
            assert_eq!(visible, "准备中文");
            assert_eq!(turn.content, visible);
            assert_eq!(turn.tool_calls[0].id, "a");
            assert_eq!(
                serde_json::from_str::<Value>(&turn.tool_calls[0].arguments).unwrap(),
                json!({"text":"记住"})
            );
            assert!(turn.reasoning_content.is_none());
            assert!(turn.provider_continuation.is_some());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_reasoning_streams_separately_and_opaque_protocol_fields_remain_private() {
        for (protocol, data) in [
            (
                ConnectionProtocol::OpenAiResponses,
                responses_with_reasoning("正文中文", Some("a"), Some("公开摘要🙂")),
            ),
            (
                ConnectionProtocol::AnthropicMessages,
                anthropic_with_reasoning("正文中文", Some("a"), Some("公开摘要🙂")),
            ),
        ] {
            let chunks: Vec<Result<Vec<u8>, String>> =
                data.bytes().map(|byte| Ok(vec![byte])).collect();
            let mut body = String::new();
            let mut reasoning = String::new();
            let turn = consume_with_reasoning(
                futures_util::stream::iter(chunks),
                protocol,
                |text| body.push_str(text),
                |text| reasoning.push_str(text),
            )
            .await
            .unwrap();
            assert_eq!(body, "正文中文");
            assert_eq!(turn.content, body);
            assert_eq!(reasoning, "公开摘要🙂");
            assert!(!body.contains("公开摘要"));
            assert!(!reasoning.contains("opaque"));
            assert!(!reasoning.contains("signature"));
            let continuation = turn.provider_continuation.unwrap().to_string();
            assert!(
                continuation.contains(if protocol == ConnectionProtocol::OpenAiResponses {
                    "opaque encrypted reasoning"
                } else {
                    "signed opaque state"
                })
            );
        }
        let data = sse(vec![
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"secret redacted blob"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"正文"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
            json!({"type":"message_stop"}),
        ]);
        let turn = consume_with_reasoning(
            futures_util::stream::iter([Ok::<_, String>(data.into_bytes())]),
            ConnectionProtocol::AnthropicMessages,
            |_| {},
            |_| panic!("redacted thinking must remain private"),
        )
        .await
        .unwrap();
        assert_eq!(turn.content, "正文");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_http_cancel_mid_stream_drops_request_without_late_completion() {
        for (protocol, data, terminal) in [
            (
                ConnectionProtocol::OpenAiResponses,
                responses("开始", None),
                "event: response.completed",
            ),
            (
                ConnectionProtocol::AnthropicMessages,
                anthropic("开始", None),
                "event: message_stop",
            ),
        ] {
            let end = data.rfind(terminal).unwrap();
            let (_release, gate) = std::sync::mpsc::channel();
            let fixture = ai::tests::HttpFixture::start(
                vec![
                    data[..end].as_bytes().to_vec(),
                    data[end..].as_bytes().to_vec(),
                ],
                Some((1, gate)),
            );
            let transport = Transport {
                url: fixture.url.clone(),
                protocol,
                auth_mode: ConnectionAuthMode::None,
            };
            let (seen, observed) = tokio::sync::oneshot::channel();
            let mut seen = Some(seen);
            let mut future = Box::pin(stream_turn(
                &transport,
                "",
                json!({"model":"fixture"}),
                |text| {
                    if !text.is_empty() {
                        if let Some(seen) = seen.take() {
                            let _ = seen.send(());
                        }
                    }
                },
            ));
            tokio::select! { _=&mut future=>panic!("server is paused before terminal event"), _=observed=>{} }
            drop(future);
            drop(fixture); // The fixture observes its stop flag, not a next provider frame.
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn truncation_invalid_state_and_mismatched_tool_arguments_never_complete() {
        let mut cases = Vec::new();
        let data = responses("", Some("a"));
        let end = data.rfind("event: response.completed").unwrap();
        cases.push((ConnectionProtocol::OpenAiResponses, data[..end].to_owned()));
        cases.push((
            ConnectionProtocol::OpenAiResponses,
            data.replace(
                "\"status\":\"completed\",\"type\":\"function_call\"",
                "\"status\":\"in_progress\",\"type\":\"function_call\"",
            ),
        ));
        let a = anthropic("", Some("a"));
        cases.push((
            ConnectionProtocol::AnthropicMessages,
            a.replace(
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                "",
            ),
        ));
        cases.push((
            ConnectionProtocol::AnthropicMessages,
            a.replace("\"tool_use\"}", "\"max_tokens\"}"),
        ));
        cases.push((
            ConnectionProtocol::AnthropicMessages,
            a.replace("signed opaque state", ""),
        ));
        // Replace only the final argument string, leaving streamed deltas intact.
        cases.push((
            ConnectionProtocol::OpenAiResponses,
            data.replace(
                "\"arguments\":\"{\\\"text\\\":\\\"记住\\\"}\"",
                "\"arguments\":\"{}\"",
            ),
        ));
        for (protocol, data) in cases {
            assert!(consume(
                futures_util::stream::iter([Ok::<_, String>(data.into_bytes())]),
                protocol,
                |_| {}
            )
            .await
            .is_err());
        }
    }
}
