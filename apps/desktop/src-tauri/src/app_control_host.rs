// SPDX-License-Identifier: MPL-2.0
//! Authenticated local control is a first-party plugin, separate from document
//! HTTP and WebView capabilities. Mutations enter the same renderer flush lane
//! as UI actions; native receipts never infer success from an emitted event.
use mewu_control::{ControlError, ErrorCode, ExecutionOutcome, Request, Response, Server, ServerHandle, ServerOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex}, time::{Duration, Instant}};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const MAX_PENDING: usize = 32;
const MAX_URGENT: usize = 8;
const MAX_RECEIPTS: usize = 128;
const MAX_RESULT: usize = 256 * 1024;
const TTL: Duration = Duration::from_secs(600);
const CLAIM_TIMEOUT: Duration = Duration::from_secs(300);
const METHODS: &[&str] = &[
    "capture.open", "region.add", "chat.setDraft", "chat.send", "chat.cancel",
    "blackboard.create", "blackboard.finish", "blackboard.open", "drawing.apply",
    "recording.start", "recording.control", "scroll.start", "scroll.control",
    "scene.activate", "scene.new", "scene.newConversation", "scene.freeze",
    "scene.close", "scene.command", "references.set", "settings.open",
    "asset.import", "image.export", "ocr.run", "translation.run", "pin.create",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Context { scene_id: Option<String>, revision: Option<u64> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    id: String, nonce: String, method: String, params: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Queued, Claimed, Complete }
struct Operation {
    fingerprint: [u8; 32], nonce: String, renderer_id: String,
    plugin_revision: u64, phase: Phase, since: Instant,
    result: Option<Result<Value, ControlError>>,
}
#[derive(Default)]
struct Inner { renderer_id: Option<String>, operations: HashMap<String, Operation> }
#[derive(Default)]
pub(crate) struct Runtime {
    revision: AtomicU64, stopped: AtomicBool,
    #[cfg(unix)]
    integration_started: AtomicBool,
    inner: Mutex<Inner>, server: Mutex<Option<ServerHandle>>,
    streams: Mutex<crate::control_run_stream::Streams>,
}

fn error(code: ErrorCode, message: &'static str) -> ControlError { ControlError::not_sent(code, message) }
fn output<T: Serialize>(value: T) -> Result<Value, ControlError> {
    struct Bounded { bytes: Vec<u8>, exceeded: bool }
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_RESULT.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other("control result too large"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut writer = Bounded { bytes: Vec::new(), exceeded: false };
    if serde_json::to_writer(&mut writer, &value).is_err() {
        return Err(if writer.exceeded { error(ErrorCode::ResultTooLarge, "结果过大，请缩小查询范围") }
            else { error(ErrorCode::Internal, "无法读取结果") });
    }
    serde_json::from_slice(&writer.bytes).map_err(|_| error(ErrorCode::Internal, "无法读取结果"))
}
fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ControlError> {
    serde_json::from_value(value).map_err(|_| error(ErrorCode::InvalidRequest, "参数格式无效，请查看 schema"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Id { id: String }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordingStatusQuery { #[serde(default)] id: Option<String> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SceneQuery { scene_id: String, #[serde(default = "default_limit")] limit: usize }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneView<'a> {
    scene_id: &'a str, title: &'a str, agent_id: &'a str, connection_id: &'a Option<String>, revision: u64,
    background: &'a Option<mewu_core::Asset>, regions: &'a [mewu_core::Region],
    items: &'a [mewu_core::SpaceItem], refs: &'a [mewu_core::Reference], draft: &'a str,
    frozen: bool, closed: bool, run: &'a Option<mewu_core::Run>, messages: &'a [mewu_core::Message],
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunView<'a> {
    scene_id: &'a str, run: &'a Option<mewu_core::Run>, messages: &'a [mewu_core::Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<crate::control_run_stream::StreamView>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page { #[serde(default)] offset: usize, #[serde(default = "default_limit")] limit: usize }
fn default_limit() -> usize { 20 }
fn page_limit(limit: usize) -> Result<usize, ControlError> {
    (1..=100).contains(&limit).then_some(limit).ok_or_else(|| error(ErrorCode::InvalidRequest, "limit 必须为 1–100"))
}
fn current_revision(app: &AppHandle) -> Result<u64, ControlError> {
    let host = app.state::<crate::Host>();
    host.exit.ensure_new_operation().map_err(|_| error(ErrorCode::ShuttingDown, "Mewu 正在退出"))?;
    if !host.storage.is_ready() { return Err(error(ErrorCode::Unavailable, "Mewu 尚未就绪")); }
    let revision = host.plugins.lock().map_err(|_| error(ErrorCode::Internal, "插件状态不可用"))?
        .store.cli_revision().map_err(|_| error(ErrorCode::Disabled, "命令行插件不可用"))?
        .ok_or_else(|| error(ErrorCode::Disabled, "命令行插件已停用或卸载"));
    revision
}
pub(crate) fn require_enabled(app: &AppHandle) -> Result<u64, String> {
    current_revision(app).map_err(|_| "命令行插件不可用".into())
}
// These hooks run inside the already-authorized Agent path. They never acquire
// plugins/Engine again, never persist text, and never alter event delivery.
pub(crate) fn begin_run_stream(app: &AppHandle, scene: &str, run: &str) {
    let Some(runtime)=app.try_state::<Runtime>() else{return};
    if runtime.revision.load(Ordering::Acquire)==0 {return;}
    if let Ok(mut streams)=runtime.streams.lock() {streams.begin(scene,run,Instant::now());};
}
pub(crate) fn observe_run_event(app: &AppHandle, event: &Value) {
    let Some(runtime)=app.try_state::<Runtime>() else{return};
    if runtime.revision.load(Ordering::Acquire)==0 || event["status"]!="running" {return;}
    if let (Some(scene),Some(run),Some(text))=(event["sceneId"].as_str(),event["runId"].as_str(),event["text"].as_str()) {
        if let Ok(mut streams)=runtime.streams.lock() {streams.push(scene,run,text,Instant::now());};
    }
}

/// Called with the already-validated plugin snapshot, including the fail-closed
/// empty snapshot. Never reacquire plugins while a mutation owns that mutex.
pub(crate) fn reconcile(app: &AppHandle, snapshot: &crate::plugins::PluginSnapshot) {
    let Some(runtime) = app.try_state::<Runtime>() else { return; };
    let revision = snapshot.plugins.iter().find(|record|
        record.manifest.id == "mewu.cli" && record.source == crate::plugins::PluginSource::Official
        && record.state == crate::plugins::PluginState::Enabled && record.error.is_none()
        && record.manifest == crate::plugins::parse_manifest(include_bytes!("../../plugins/official-cli.json")).expect("official CLI manifest")
    ).map(|record| record.revision).unwrap_or(0);
    let previous = runtime.revision.swap(revision, Ordering::AcqRel);
    if previous != revision {
        if let Ok(mut streams) = runtime.streams.lock() { streams.clear(); }
        if let Ok(mut inner) = runtime.inner.lock() {
            for operation in inner.operations.values_mut().filter(|op| op.phase == Phase::Queued) {
                operation.phase = Phase::Complete;
                operation.result = Some(Err(error(ErrorCode::Disabled, "命令行插件已变更，操作未执行")));
            }
        }
    }
    // Listener management is independent of the plugin/Engine lock order.
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || ensure_listener(&app));
}
fn ensure_listener(app: &AppHandle) {
    let runtime = app.state::<Runtime>();
    let Ok(mut slot) = runtime.server.lock() else { return; };
    let enabled = !runtime.stopped.load(Ordering::Acquire) && runtime.revision.load(Ordering::Acquire) != 0;
    if !enabled {
        if let Some(server) = slot.as_mut() {
            server.revoke();
            if server.shutdown(Duration::from_secs(2)).is_err() { return; }
        }
        *slot = None;
        return;
    }
    if let Some(server) = slot.as_mut() {
        if server.is_enabled() {
            drop(slot);
            initialize_integration(app);
            return;
        }
        if server.shutdown(Duration::from_secs(2)).is_err() { return; }
        *slot = None;
    }
    let Ok(endpoint) = mewu_control::Endpoint::for_current_user() else { return; };
    let target = app.clone();
    let dispatcher = Arc::new(move |request: Request| dispatch(&target, request));
    match Server::start(endpoint, dispatcher, ServerOptions::default()) {
        Ok(server) => { *slot = Some(server); }
        Err(_) => { let _ = app.emit_to("settings", "host-error", "无法启动命令行服务"); }
    }
    drop(slot);
    initialize_integration(app);
}
fn initialize_integration(app: &AppHandle) {
    #[cfg(unix)]
    {
        let runtime = app.state::<Runtime>();
        if runtime.stopped.load(Ordering::Acquire) || current_revision(app).is_err()
            || runtime.integration_started.swap(true, Ordering::AcqRel) { return; }
        if crate::app_control_integration::initialize(app).is_err() {
            let _ = app.emit_to("settings", "host-error", "无法完成命令行初始化");
        }
    }
    #[cfg(not(unix))]
    let _ = app;
}
pub(crate) fn shutdown(app: &AppHandle) {
    if let Some(runtime) = app.try_state::<Runtime>() {
        runtime.stopped.store(true, Ordering::Release);
        runtime.revision.store(0, Ordering::Release);
        if let Ok(mut streams) = runtime.streams.lock() { streams.clear(); }
        renderer_invalidated(app);
        if let Ok(mut server) = runtime.server.lock() {
            if let Some(server) = server.as_mut() {
                server.revoke();
                if server.shutdown(Duration::from_secs(2)).is_err() {
                    // Final process exit owns the remaining threads. Preserve the
                    // diagnostic instead of claiming a timed-out drain succeeded.
                    let _ = app.emit_to("settings", "host-error", "命令行服务尚未完全退出");
                }
            }
        }
    }
}
pub(crate) fn renderer_invalidated(app: &AppHandle) {
    if let Some(runtime) = app.try_state::<Runtime>() {
        if let Ok(mut inner) = runtime.inner.lock() {
            inner.renderer_id = None;
            for operation in inner.operations.values_mut().filter(|op| op.phase != Phase::Complete) {
                let failure = if operation.phase == Phase::Queued { error(ErrorCode::Unavailable, "空间已重载，操作未执行") }
                    else { ControlError::unknown(ErrorCode::Unavailable, "空间已重载，请检查操作结果") };
                operation.phase = Phase::Complete; operation.result = Some(Err(failure));
            }
        }
    }
}
fn space(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "space" { Ok(()) } else { Err("此窗口不能接收控制请求".into()) }
}
#[tauri::command]
pub(crate) fn register_control_renderer(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    space(&window)?;
    renderer_invalidated(&app);
    let id = uuid::Uuid::new_v4().to_string();
    app.state::<Runtime>().inner.lock().map_err(|_| "控制状态不可用")?.renderer_id = Some(id.clone());
    Ok(json!({"rendererId":id}))
}
#[tauri::command]
pub(crate) fn claim_control_request(app: AppHandle, window: WebviewWindow, renderer_id: String, id: String, nonce: String) -> Result<(), String> {
    space(&window)?;
    let revision = current_revision(&app).map_err(|_| "命令行插件不可用")?;
    let runtime = app.state::<Runtime>();
    let mut inner = runtime.inner.lock().map_err(|_| "控制状态不可用")?;
    if inner.renderer_id.as_deref() != Some(&renderer_id) { return Err("空间已变化".into()); }
    let op = inner.operations.get_mut(&id).ok_or("控制请求已结束")?;
    if op.phase != Phase::Queued || op.nonce != nonce || op.renderer_id != renderer_id || op.plugin_revision != revision || op.since.elapsed() >= Duration::from_secs(30) {
        return Err("控制请求已过期或撤销".into());
    }
    op.phase = Phase::Claimed;
    op.since = Instant::now();
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionError { code: String, message: String, executed: Option<bool> }
#[tauri::command]
pub(crate) fn complete_control_request(app: AppHandle, window: WebviewWindow, renderer_id: String, id: String, nonce: String, result: Option<Value>, error: Option<CompletionError>) -> Result<(), String> {
    space(&window)?;
    let runtime = app.state::<Runtime>();
    let mut inner = runtime.inner.lock().map_err(|_| "控制状态不可用")?;
    if inner.renderer_id.as_deref() != Some(&renderer_id) { return Err("空间已变化".into()); }
    let op = inner.operations.get_mut(&id).ok_or("控制请求已结束")?;
    expire(op);
    if op.nonce != nonce || op.renderer_id != renderer_id || op.phase == Phase::Complete { return Err("控制请求已结束".into()); }
    if result.is_some() && error.is_some() { return Err("控制结果格式无效".into()); }
    let outcome = match error {
        Some(failure) => {
            let code = match failure.code.as_str() {
                "CONFLICT" => ErrorCode::Conflict, "BUSY" => ErrorCode::Busy,
                "INVALID_REQUEST" => ErrorCode::InvalidRequest, "UNSUPPORTED_METHOD" => ErrorCode::Unsupported,
                "REQUEST_EXPIRED" => ErrorCode::Timeout, "NOT_READY" => ErrorCode::Unavailable,
                _ => ErrorCode::Internal,
            };
            let execution = if op.phase == Phase::Queued && failure.executed == Some(false) { ExecutionOutcome::NotSent } else { ExecutionOutcome::Unknown };
            let message = if failure.message.chars().count() <= 256 { failure.message } else { "操作未完成".into() };
            Err(ControlError::new(code, message, execution))
        }
        None if op.phase == Phase::Claimed => {
            let value = result.unwrap_or(Value::Null);
            if serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > MAX_RESULT) {
                Err(ControlError::new(ErrorCode::ResultTooLarge, "操作已执行，结果过大", ExecutionOutcome::Executed))
            } else { Ok(value) }
        }
        None => return Err("请求尚未获准执行".into()),
    };
    op.result = Some(outcome); op.phase = Phase::Complete;
    Ok(())
}

fn dispatch(app: &AppHandle, request: Request) -> Response {
    let id = request.id.clone();
    let result = request.validate().and_then(|_| current_revision(app)).and_then(|revision| dispatch_inner(app, request, revision));
    match result { Ok(value) => Response::success(id, value), Err(error) => Response::error(id, error) }
}
fn dispatch_inner(app: &AppHandle, request: Request, revision: u64) -> Result<Value, ControlError> {
    match request.method.as_str() {
        "app.status" => {
            let _: Empty = params(request.params)?;
            let host = app.state::<crate::Host>();
            let engine = host.lock().map_err(|_| error(ErrorCode::Internal, "状态不可用"))?;
            output(json!({"protocolVersion":mewu_control::PROTOCOL_VERSION,"application":"MewuAI","version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"activeSceneId":engine.store.active_scene().ok().map(|scene|&scene.id),"revision":host.revision.load(Ordering::Acquire),"cliPluginRevision":revision}))
        }
        "app.schema" | "app.capabilities" => { let _: Empty = params(request.params)?; output(catalog()) }
        "scene.list" => {
            let page: Page = params(request.params)?; let limit = page_limit(page.limit)?;
            let host = app.state::<crate::Host>(); let engine = host.lock().map_err(|_| error(ErrorCode::Internal,"状态不可用"))?;
            let scenes = engine.store.scenes();
            let rows: Vec<_> = scenes.iter().skip(page.offset).take(limit).map(|scene| json!({"id":scene.id,"title":scene.title,"agentId":scene.agent_id,"frozen":scene.frozen,"closed":scene.closed,"regionCount":scene.regions.len(),"itemCount":scene.items.len(),"messageCount":scene.messages.len()})).collect();
            output(json!({"scenes":rows,"nextOffset":(page.offset.saturating_add(rows.len())<scenes.len()).then(||page.offset+rows.len())}))
        }
        "scene.get" | "run.get" => {
            let query: SceneQuery = params(request.params)?; let limit = page_limit(query.limit)?;
            let host = app.state::<crate::Host>(); let engine = host.lock().map_err(|_| error(ErrorCode::Internal,"状态不可用"))?;
            let scene = engine.store.scene(&query.scene_id).map_err(|_| error(ErrorCode::InvalidRequest,"会话不存在"))?;
            let messages = &scene.messages[scene.messages.len().saturating_sub(limit)..];
            if request.method == "run.get" {
                let stream = scene.run.as_ref().and_then(|run|app.state::<Runtime>().streams.lock().ok()
                    .and_then(|mut streams|streams.view(&scene.id,&run.id,run.status==mewu_core::RunStatus::Running,Instant::now())));
                return output(RunView { scene_id: &scene.id, run: &scene.run, messages, stream });
            }
            output(SceneView { scene_id: &scene.id, title: &scene.title, agent_id: &scene.agent_id, connection_id: &scene.connection_id,
                revision: host.revision.load(Ordering::Acquire), background: &scene.background,
                regions: &scene.regions, items: &scene.items, refs: &scene.refs, draft: &scene.draft,
                frozen: scene.frozen, closed: scene.closed, run: &scene.run, messages })
        }
        "agent.list" | "model.list" => {
            let _: Empty = params(request.params)?;
            let host = app.state::<crate::Host>(); let engine = host.lock().map_err(|_| error(ErrorCode::Internal,"状态不可用"))?;
            // Public profiles contain references/hasKey flags, never credentials.
            if request.method == "agent.list" { output(engine.store.agent_profiles().iter().filter(|agent| !agent.archived).collect::<Vec<_>>()) } else { output(engine.store.connection_profiles()) }
        }
        "plugin.list" => { let _: Empty = params(request.params)?; let host = app.state::<crate::Host>(); let value = host.plugins.lock().map_err(|_|error(ErrorCode::Internal,"插件状态不可用"))?.store.snapshot().map_err(|_|error(ErrorCode::Internal,"插件状态不可用"))?; output(value) }
        "recording.status" => {
            let query: RecordingStatusQuery = params(request.params)?;
            if let Some(id) = query.id {
                if uuid::Uuid::parse_str(&id).is_err() || id.len() != 36 {
                    return Err(error(ErrorCode::InvalidRequest,"录制 ID 必须是 UUID"));
                }
                output(crate::recording::get_recording_result(app.clone(), &id)
                    .map_err(|_|error(ErrorCode::Internal,"录屏结果不可用"))?)
            } else {
                output(crate::recording::get_recording_status(app.clone())
                    .map_err(|_|error(ErrorCode::Internal,"录屏状态不可用"))?)
            }
        }
        "operation.get" => {
            let query: Id = params(request.params)?;
            let runtime = app.state::<Runtime>(); let mut inner = runtime.inner.lock().map_err(|_|error(ErrorCode::Internal,"控制状态不可用"))?;
            let op = inner.operations.get_mut(&query.id).ok_or_else(||error(ErrorCode::InvalidRequest,"操作记录不存在或已过期"))?;
            expire(op);
            let mut result = json!({"operationId":query.id,"status":match op.phase {Phase::Queued=>"queued",Phase::Claimed=>"running",Phase::Complete=>"completed"}});
            match &op.result { Some(Ok(value))=>{result["result"]=value.clone();},Some(Err(failure))=>{result["status"]=json!("failed");result["error"]=serde_json::to_value(failure).map_err(|_|error(ErrorCode::Internal,"控制结果不可用"))?;},None=>{} }
            output(result)
        }
        method if METHODS.contains(&method) => enqueue(app, request, revision),
        _ => Err(error(ErrorCode::Unsupported,"不支持此命令，请运行 mewu schema")),
    }
}
fn expire(operation: &mut Operation) {
    if operation.phase == Phase::Queued && operation.since.elapsed() >= Duration::from_secs(30) {
        operation.phase = Phase::Complete; operation.result = Some(Err(error(ErrorCode::Timeout,"请求排队超时，未执行")));
    } else if operation.phase == Phase::Claimed && operation.since.elapsed() >= CLAIM_TIMEOUT {
        operation.phase = Phase::Complete;
        operation.result = Some(Err(ControlError::unknown(ErrorCode::Timeout,"操作未及时返回，结果未知，请检查当前状态")));
    }
}
fn enqueue(app: &AppHandle, mut request: Request, revision: u64) -> Result<Value, ControlError> {
    let fingerprint: [u8;32] = Sha256::digest(serde_json::to_vec(&request).map_err(|_|error(ErrorCode::InvalidRequest,"请求无效"))?).into();
    let context: Context = match request.params.as_object_mut().and_then(|params|params.remove("context")) { Some(context)=>params(context)?,None=>Context::default() };
    let capacity = MAX_PENDING + if urgent(&request.method, &request.params) { MAX_URGENT } else { 0 };
    let runtime = app.state::<Runtime>();
    let event = {
        let mut inner = runtime.inner.lock().map_err(|_|error(ErrorCode::Internal,"控制状态不可用"))?;
        for operation in inner.operations.values_mut() { expire(operation); }
        inner.operations.retain(|_,op| op.phase != Phase::Complete || op.since.elapsed()<TTL);
        if let Some(op) = inner.operations.get(&request.id) {
            if op.fingerprint != fingerprint { return Err(error(ErrorCode::Conflict,"请求 ID 已用于其他操作")); }
            return Ok(json!({"operationId":request.id,"status":"accepted","duplicate":true}));
        }
        if inner.operations.values().filter(|op|op.phase!=Phase::Complete).count()>=capacity { return Err(error(ErrorCode::Busy,"待执行操作过多")); }
        while inner.operations.len()>=MAX_RECEIPTS {
            let oldest = inner.operations.iter().filter(|(_,op)|op.phase==Phase::Complete).min_by_key(|(_,op)|op.since).map(|(id,_)|id.clone()).ok_or_else(||error(ErrorCode::Busy,"待执行操作过多"))?;
            inner.operations.remove(&oldest);
        }
        let renderer_id = inner.renderer_id.clone().ok_or_else(||error(ErrorCode::Unavailable,"Mewu 界面尚未就绪"))?;
        let nonce = uuid::Uuid::new_v4().to_string();
        inner.operations.insert(request.id.clone(),Operation{fingerprint,nonce:nonce.clone(),renderer_id,plugin_revision:revision,phase:Phase::Queued,since:Instant::now(),result:None});
        Event{id:request.id.clone(),nonce,method:request.method,params:request.params,scene_id:context.scene_id,revision:context.revision}
    };
    if app.emit_to("space","mewu-control-request",event).is_err() {
        if let Ok(mut inner)=runtime.inner.lock(){if let Some(op)=inner.operations.get_mut(&request.id){op.phase=Phase::Complete;op.result=Some(Err(error(ErrorCode::Unavailable,"空间未接收请求")));}}
        return Err(error(ErrorCode::Unavailable,"空间未接收请求"));
    }
    Ok(json!({"operationId":request.id,"status":"queued"}))
}
fn urgent(method: &str, params: &Value) -> bool {
    let Some(fields) = params.as_object() else { return false; };
    let valid_id = |key| fields.get(key).and_then(Value::as_str).is_some_and(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control));
    match method {
        "chat.cancel" => fields.len() == 1 && valid_id("expectedRunId"),
        "recording.control" | "scroll.control" => fields.len() == 2 && valid_id("id")
            && matches!(fields.get("action").and_then(Value::as_str), Some("stop" | "cancel")),
        _ => false,
    }
}
fn catalog() -> Value {
    serde_json::from_slice(include_bytes!("../../control/catalog.json"))
        .expect("validated official CLI catalog")
}

#[cfg(test)]
#[path = "app_control_catalog_tests.rs"]
mod catalog_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn result_budget_stops_serializing_before_walking_the_entire_collection() {
        struct Counted<'a>(&'a std::cell::Cell<usize>);
        impl Serialize for Counted<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.set(self.0.get() + 1);
                serializer.serialize_str(&"a".repeat(1024))
            }
        }
        let count = std::cell::Cell::new(0);
        let values: Vec<_> = (0..4096).map(|_| Counted(&count)).collect();
        let failure = output(&values).unwrap_err();
        assert_eq!(failure.code, ErrorCode::ResultTooLarge);
        assert!(count.get() < 300, "must not serialize all 4 MiB before rejecting");
        assert_eq!(output(json!({"text":"红黄蓝"})).unwrap(), json!({"text":"红黄蓝"}));
    }

    #[test]
    fn pending_timeout_is_not_sent_but_claimed_timeout_is_unknown_and_never_requeued() {
        let mut op = Operation { fingerprint: [0;32], nonce: "n".into(), renderer_id: "r".into(), plugin_revision: 1,
            phase: Phase::Queued, since: Instant::now() - Duration::from_secs(31), result: None };
        expire(&mut op);
        assert!(op.phase == Phase::Complete);
        assert_eq!(op.result.as_ref().unwrap().as_ref().unwrap_err().outcome, ExecutionOutcome::NotSent);
        op.phase = Phase::Claimed; op.since = Instant::now() - CLAIM_TIMEOUT; op.result = None;
        expire(&mut op);
        assert!(op.phase == Phase::Complete);
        assert_eq!(op.result.as_ref().unwrap().as_ref().unwrap_err().outcome, ExecutionOutcome::Unknown);
        expire(&mut op);
        assert_eq!(op.result.as_ref().unwrap().as_ref().unwrap_err().outcome, ExecutionOutcome::Unknown);
    }

    #[test]
    fn overloaded_queue_reserves_only_bounded_stop_and_cancel_capacity() {
        assert!(urgent("chat.cancel", &json!({"expectedRunId":"run"})));
        assert!(urgent("recording.control", &json!({"id":"recording","action":"stop"})));
        assert!(urgent("scroll.control", &json!({"id":"scroll","action":"cancel"})));
        assert!(!urgent("recording.control", &json!({"id":"recording","action":"resume"})));
        assert!(!urgent("recording.control", &json!({"id":"recording","action":"stop","arbitrary":true})));
        assert!(!urgent("chat.send", &json!({"action":"cancel"})));
        assert!(!urgent("chat.cancel", &json!({"expectedRunId":""})));
    }
}
