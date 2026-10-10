// SPDX-License-Identifier: MPL-2.0
//! First-party connection management. Profiles never contain plaintext secrets.
//! Discovery/test are explicit, cancellable operations on one captured draft.
use crate::{credentials, provider_transport, Host, HostSnapshot};
use futures_util::StreamExt;
use mewu_core::{
    ConnectionAuthMode, ConnectionProfile, ConnectionProtocol, Snapshot, Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const TOMBSTONE_TIME: Duration = Duration::from_secs(300);
const MAX_PROBES: usize = 4;
const MAX_REQUEST_IDS: usize = 512;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_MODELS: usize = 5000;

#[tauri::command]
pub fn get_provider_presets(app: AppHandle) -> Result<Vec<crate::plugins::ModelConnectionPreset>, String> {
    let host = app.state::<Host>();
    let result = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?.store.model_connection_presets();
    result
}

fn current_profile(
    store: &Store,
    id: &str,
    expected: Option<u64>,
) -> Result<Option<ConnectionProfile>, String> {
    let current = store
        .snapshot()
        .connections
        .into_iter()
        .find(|p| p.id == id);
    if current.as_ref().map(|p| p.revision) != expected {
        return Err("连接已更改，请载入最新配置".into());
    }
    Ok(current)
}

fn normalize(profile: &mut ConnectionProfile) {
    profile.name = profile.name.trim().into();
    profile.base_url = profile.base_url.trim().trim_end_matches('/').into();
    profile.model = profile.model.trim().into();
    profile.advanced.request_path = profile
        .advanced
        .request_path
        .take()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(|v| {
            if v.starts_with('/') {
                v
            } else {
                format!("/{v}")
            }
        });
}

/// The frontend's hasKey/credentialId/revision are never authority. In
/// particular a new profile cannot borrow another profile's credential UUID.
pub(crate) fn save_profile(
    store: &mut Store,
    root: &Path,
    mut profile: ConnectionProfile,
    expected: Option<u64>,
    api_key: Option<Zeroizing<String>>,
) -> Result<Snapshot, String> {
    let current = current_profile(store, &profile.id, expected)?;
    profile.revision = expected
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("连接版本已达上限")?;
    profile.credential_id = current.as_ref().and_then(|p| p.credential_id.clone());
    profile.has_key = profile.credential_id.is_some();
    normalize(&mut profile);
    mewu_core::validate_connection_profile(&profile).map_err(|e| e.to_string())?;
    validate_request_options(&profile)?;
    if !profile.base_url.is_empty() {
        provider_transport::endpoint(&profile)?;
    }
    let api_key = api_key.map(|value| Zeroizing::new(value.trim().to_string()));
    if let Some(value) = &api_key {
        credentials::validate_key(value)?;
    }
    // A durable new blob precedes the DB commit. Keep old blobs immutable;
    // deleting a connection only removes its live reference.
    let created = match api_key.as_deref() {
        Some(value) if !value.is_empty() => Some(credentials::create(root, &profile.id, value)?),
        _ => None,
    };
    if api_key.is_some() {
        profile.credential_id = created.clone();
        profile.has_key = created.is_some();
    }
    let id = profile.id.clone();
    let result = store
        .save_connection_profile(profile, expected)
        .map_err(|e| e.to_string());
    if result.is_err() {
        if let Some(credential_id) = created {
            credentials::discard_uncommitted(root, &id, &credential_id);
        }
    }
    result
}

#[tauri::command]
pub async fn save_connection_profile(
    app: AppHandle,
    profile: ConnectionProfile,
    expected_revision: Option<u64>,
    api_key: Option<String>,
) -> Result<HostSnapshot, String> {
    let api_key = api_key.map(Zeroizing::new);
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        let id = profile.id.clone();
        let snapshot = save_profile(
            &mut engine.store,
            &host.root,
            profile,
            expected_revision,
            api_key,
        )?;
        crate::cancel_ended_runs(&mut engine, &snapshot);
        engine.translation_jobs.cancel_connection(&id);
        if let Ok(mut probes) = host.connection_probes.lock() {
            probes.cancel_connection(&id);
        }
        Ok(crate::publish(&app, &snapshot))
    })
    .await
    .map_err(|_| "连接保存中断")?
}

#[tauri::command]
pub async fn delete_connection_profile(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<HostSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        let snapshot = engine
            .store
            .remove_connection_profile(&id, expected_revision)
            .map_err(|e| e.to_string())?;
        crate::cancel_ended_runs(&mut engine, &snapshot);
        engine.translation_jobs.cancel_connection(&id);
        if let Ok(mut probes) = host.connection_probes.lock() {
            probes.cancel_connection(&id);
        }
        Ok(crate::publish(&app, &snapshot))
    })
    .await
    .map_err(|_| "连接删除中断")?
}

struct ActiveProbe {
    connection_id: String,
    cancel: oneshot::Sender<()>,
}
struct ProbeEntry {
    active: Option<ActiveProbe>,
    touched: Instant,
}
#[derive(Default)]
pub struct ProbeRegistry {
    entries: HashMap<(String, String), ProbeEntry>,
}
impl ProbeRegistry {
    fn prune(&mut self) {
        self.entries
            .retain(|_, v| v.active.is_some() || v.touched.elapsed() < TOMBSTONE_TIME);
    }
    fn start(
        &mut self,
        owner: &str,
        request_id: &str,
        connection_id: &str,
    ) -> Result<oneshot::Receiver<()>, String> {
        self.prune();
        if uuid::Uuid::parse_str(request_id)
            .map(|id| id.to_string() != request_id)
            .unwrap_or(true)
        {
            return Err("连接测试标识无效".into());
        }
        let identity = (owner.to_string(), request_id.to_string());
        if self.entries.contains_key(&identity) {
            return Err("请求已取消或结束".into());
        }
        if self.entries.len() >= MAX_REQUEST_IDS
            || self.entries.values().filter(|v| v.active.is_some()).count() >= MAX_PROBES
        {
            return Err("连接测试过于频繁，请稍后再试".into());
        }
        // One operation per settings surface; starting another cancels the
        // previous future even if the frontend has not sent its cancel yet.
        self.cancel_owner(owner);
        let (cancel, receiver) = oneshot::channel();
        self.entries.insert(
            identity,
            ProbeEntry {
                active: Some(ActiveProbe {
                    connection_id: connection_id.into(),
                    cancel,
                }),
                touched: Instant::now(),
            },
        );
        Ok(receiver)
    }
    fn finish(&mut self, owner: &str, request_id: &str) {
        if let Some(entry) = self.entries.get_mut(&(owner.into(), request_id.into())) {
            entry.active.take();
            entry.touched = Instant::now();
        }
    }
    fn cancel(&mut self, owner: &str, request_id: &str) -> Result<(), String> {
        if uuid::Uuid::parse_str(request_id)
            .map(|id| id.to_string() != request_id)
            .unwrap_or(true)
        {
            return Err("连接测试标识无效".into());
        }
        self.prune();
        let identity = (owner.into(), request_id.into());
        if !self.entries.contains_key(&identity) && self.entries.len() >= MAX_REQUEST_IDS {
            return Err("连接测试过于频繁，请稍后再试".into());
        }
        let entry = self.entries.entry(identity).or_insert(ProbeEntry {
            active: None,
            touched: Instant::now(),
        });
        if let Some(active) = entry.active.take() {
            let _ = active.cancel.send(());
        }
        entry.touched = Instant::now();
        Ok(())
    }
    pub fn cancel_owner(&mut self, owner: &str) {
        for ((candidate, _), entry) in &mut self.entries {
            if candidate == owner {
                if let Some(active) = entry.active.take() {
                    let _ = active.cancel.send(());
                    entry.touched = Instant::now();
                }
            }
        }
    }
    pub fn cancel_all(&mut self) {
        for entry in self.entries.values_mut() {
            if let Some(active) = entry.active.take() {
                let _ = active.cancel.send(());
                entry.touched = Instant::now();
            }
        }
    }
    fn cancel_connection(&mut self, connection_id: &str) {
        for entry in self.entries.values_mut() {
            if entry
                .active
                .as_ref()
                .is_some_and(|p| p.connection_id == connection_id)
            {
                if let Some(active) = entry.active.take() {
                    let _ = active.cancel.send(());
                }
                entry.touched = Instant::now();
            }
        }
    }
}

struct ProbeLease {
    registry: Arc<Mutex<ProbeRegistry>>,
    owner: String,
    id: String,
}
impl Drop for ProbeLease {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.finish(&self.owner, &self.id);
        }
    }
}
fn begin_probe(
    app: &AppHandle,
    window: &WebviewWindow,
    id: &str,
    connection_id: &str,
) -> Result<(ProbeLease, oneshot::Receiver<()>), String> {
    if !matches!(window.label(), "space" | "settings") {
        return Err("此窗口无法测试连接".into());
    }
    let host = app.state::<Host>();
    let registry = host.connection_probes.clone();
    let receiver = {
        let mut probes = registry.lock().map_err(|_| "连接测试不可用")?;
        host.exit.ensure_running()?;
        probes.start(window.label(), id, connection_id)?
    };
    Ok((
        ProbeLease {
            registry,
            owner: window.label().into(),
            id: id.into(),
        },
        receiver,
    ))
}

#[tauri::command]
pub fn cancel_connection_probe(
    host: tauri::State<'_, Host>,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    host.connection_probes
        .lock()
        .map_err(|_| "连接测试不可用")?
        .cancel(window.label(), &request_id)
}

fn prepare_probe(
    host: &Host,
    mut profile: ConnectionProfile,
    expected: Option<u64>,
    api_key: Option<Zeroizing<String>>,
) -> Result<(ConnectionProfile, Zeroizing<String>), String> {
    let engine = host.lock()?;
    let current = current_profile(&engine.store, &profile.id, expected)?;
    profile.credential_id = current.as_ref().and_then(|p| p.credential_id.clone());
    profile.has_key = profile.credential_id.is_some();
    profile.revision = expected.unwrap_or(1);
    normalize(&mut profile);
    mewu_core::validate_connection_profile(&profile).map_err(|e| e.to_string())?;
    validate_request_options(&profile)?;
    provider_transport::endpoint(&profile)?;
    let key = if profile.advanced.auth_mode == ConnectionAuthMode::None {
        Zeroizing::new(String::new())
    } else if let Some(key) = api_key {
        Zeroizing::new(key.trim().to_string())
    } else if let Some(current) = current {
        credentials::read_profile(&host.root, &current)?
    } else {
        Zeroizing::new(String::new())
    };
    credentials::validate_key(&key)?;
    Ok((profile, key))
}

fn recheck_probe(app: &AppHandle, id: &str, expected: Option<u64>) -> Result<(), String> {
    current_profile(&app.state::<Host>().lock()?.store, id, expected).map(|_| ())
}

#[derive(Serialize)]
pub struct ModelCatalog {
    models: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    latency_ms: u64,
    model: String,
}

#[tauri::command]
pub async fn discover_connection_models(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    profile: ConnectionProfile,
    expected_revision: Option<u64>,
    api_key: Option<String>,
) -> Result<ModelCatalog, String> {
    let (_lease, mut canceled) = begin_probe(&app, &window, &request_id, &profile.id)?;
    let (profile, key) = prepare_probe(
        &app.state::<Host>(),
        profile,
        expected_revision,
        api_key.map(Zeroizing::new),
    )?;
    let work = async {
        let models = discover_models(&profile, &key).await?;
        recheck_probe(&app, &profile.id, expected_revision)?;
        Ok(ModelCatalog { models })
    };
    tokio::select! { biased; _ = &mut canceled => Err("请求已取消".into()), result = tokio::time::timeout(PROBE_TIMEOUT, work) => result.map_err(|_| "获取模型列表超时".to_string())? }
}

#[tauri::command]
pub async fn test_connection(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    profile: ConnectionProfile,
    expected_revision: Option<u64>,
    api_key: Option<String>,
) -> Result<ConnectionTest, String> {
    let (_lease, mut canceled) = begin_probe(&app, &window, &request_id, &profile.id)?;
    let (profile, key) = prepare_probe(
        &app.state::<Host>(),
        profile,
        expected_revision,
        api_key.map(Zeroizing::new),
    )?;
    if profile.model.is_empty() {
        return Err("请输入模型名称".into());
    }
    let work = async {
        let started = Instant::now();
        let transport = transport_for(&profile)?;
        let mut body = json!({"model":profile.model,"stream":true,"messages":[{"role":"user","content":"Reply with exactly: MEWU_OK"}]});
        apply_parameters(&mut body, &profile)?;
        let answer = crate::agent_loop::run_with_transport(
            transport,
            &key,
            body,
            vec![],
            |_| {},
            |_| async { Err("测试不允许执行工具".to_string()) },
        )
        .await?;
        if answer.trim() != "MEWU_OK" {
            return Err("模型未返回测试标记，请检查模型或协议".into());
        }
        recheck_probe(&app, &profile.id, expected_revision)?;
        Ok(ConnectionTest {
            latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            model: profile.model.clone(),
        })
    };
    tokio::select! { biased; _ = &mut canceled => Err("请求已取消".into()), result = tokio::time::timeout(Duration::from_secs(25), work) => result.map_err(|_| "连接测试超时".to_string())? }
}

pub fn transport_for(profile: &ConnectionProfile) -> Result<provider_transport::Transport, String> {
    Ok(provider_transport::Transport {
        url: provider_transport::endpoint(profile)?,
        protocol: profile.advanced.protocol.clone(),
        auth_mode: profile.advanced.auth_mode.clone(),
    })
}
pub fn apply_parameters(body: &mut Value, profile: &ConnectionProfile) -> Result<(), String> {
    let parameters =
        serde_json::to_value(&profile.advanced.request_parameters).map_err(|_| "请求参数无效")?;
    let object = body.as_object_mut().ok_or("请求格式无效")?;
    for (name, value) in parameters.as_object().ok_or("请求参数无效")? {
        if !value.is_null() {
            object.insert(name.clone(), value.clone());
        }
    }
    Ok(())
}

pub fn validate_request_options(profile: &ConnectionProfile) -> Result<(), String> {
    let mut body = json!({"model":profile.model,"messages":[],"stream":true});
    apply_parameters(&mut body, profile)?;
    provider_transport::request_body(profile.advanced.protocol.clone(), body).map(|_| ())
}

fn client(url: &url::Url) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    builder = crate::network_policy::apply(builder, Some(url))?;
    builder.build().map_err(|_| "无法创建连接".into())
}

async fn discover_models(profile: &ConnectionProfile, key: &str) -> Result<Vec<String>, String> {
    let mut url = provider_transport::models_endpoint(profile)?;
    let client = client(&url)?;
    let transport = transport_for(profile)?;
    if profile.advanced.protocol == ConnectionProtocol::AnthropicMessages {
        url.query_pairs_mut().append_pair("limit", "1000");
    }
    let mut models = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut bytes_read = 0usize;
    for _ in 0..20 {
        let response = provider_transport::authenticate(client.get(url.clone()), &transport, key)
            .send()
            .await
            .map_err(|_| "获取模型列表失败，请检查地址与网络")?;
        if !response.status().is_success() {
            return Err(format!(
                "获取模型列表失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > (MAX_CATALOG_BYTES - bytes_read) as u64)
        {
            return Err("模型列表过大，请手动填写模型名称".into());
        }
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| "读取模型列表失败")?;
            bytes_read = bytes_read.saturating_add(chunk.len());
            if bytes_read > MAX_CATALOG_BYTES {
                return Err("模型列表过大，请手动填写模型名称".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let page: Value =
            serde_json::from_slice(&bytes).map_err(|_| "服务返回的模型列表格式无效")?;
        let cursor = parse_models_page(&page, &mut models)?;
        match cursor {
            None => return Ok(models.into_iter().collect()),
            Some(cursor) if cursors.insert(cursor.clone()) => {
                // Cursor is data, never a URL supplied by a remote server.
                url.set_query(None);
                url.query_pairs_mut()
                    .append_pair("limit", "1000")
                    .append_pair("after_id", &cursor);
            }
            Some(_) => return Err("模型列表分页重复，请手动填写模型名称".into()),
        }
    }
    Err("模型列表分页过多，请手动填写模型名称".into())
}

fn parse_models_page(
    page: &Value,
    models: &mut BTreeSet<String>,
) -> Result<Option<String>, String> {
    // Together returns a top-level array; OpenAI/Anthropic return data[].
    let rows = page
        .as_array()
        .or_else(|| page.get("data").and_then(Value::as_array))
        .ok_or("服务返回的模型列表格式无效")?;
    if rows.len() > MAX_MODELS {
        return Err("模型列表过大，请手动填写模型名称".into());
    }
    for row in rows {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .ok_or("模型列表缺少标识")?;
        if id.is_empty()
            || id.chars().count() > 256
            || id.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("模型列表包含无效名称".into());
        }
        models.insert(id.into());
        if models.len() > MAX_MODELS {
            return Err("模型列表过大，请手动填写模型名称".into());
        }
    }
    if page
        .get("has_more")
        .is_some_and(|v| v.as_bool() == Some(true))
    {
        let cursor = page
            .get("last_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control))
            .ok_or("模型列表缺少分页标识")?;
        if rows.is_empty() {
            return Err("模型列表分页无效".into());
        }
        Ok(Some(cursor.into()))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "connection_host_tests.rs"]
mod tests;
