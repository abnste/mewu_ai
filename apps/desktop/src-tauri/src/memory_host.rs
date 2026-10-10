// SPDX-License-Identifier: MPL-2.0
//! Host-owned routes and credentials for replaceable memory providers.
use crate::{credentials, hindsight, memory_runtime, plugins, Host};
use mewu_core::{AgentMemoryStatus, MemoryAuthorityGrant, MemoryBindingView, MemoryPolicy, Store};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

fn settings(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("请从 Agent 设置管理记忆服务".into())
    }
}

fn domain(error: mewu_core::CoreError) -> String {
    crate::agent_tools::domain_error(error)
}

fn binding(store: &Store, agent: &str, id: &str) -> Result<MemoryBindingView, String> {
    let value = store.memory_binding(id).map_err(domain)?;
    if value.agent_id != agent {
        return Err("记忆连接不属于当前 Agent".into());
    }
    Ok(value)
}

pub(crate) fn grant(value: &MemoryBindingView) -> MemoryAuthorityGrant {
    MemoryAuthorityGrant {
        binding_id: value.id.clone(),
        binding_revision: value.revision,
        plugin_id: value.plugin_id.clone(),
        contribution_id: value.contribution_id.clone(),
        plugin_revision: value.plugin_revision,
    }
}

pub(crate) fn origin(value: &MemoryBindingView) -> memory_runtime::Origin {
    memory_runtime::Origin {
        binding_id: value.id.clone(),
        plugin_id: value.plugin_id.clone(),
        plugin_revision: value.plugin_revision,
    }
}

pub(crate) fn authorize(
    plugins: &plugins::PluginStore,
    grant: &MemoryAuthorityGrant,
) -> Result<(), String> {
    plugins.memory_provider(
        &grant.plugin_id,
        grant.plugin_revision,
        &grant.contribution_id,
    )?;
    Ok(())
}

pub(crate) fn grant_for_scene(
    store: &Store,
    plugins: &plugins::PluginStore,
    scene_id: &str,
) -> Result<Option<MemoryAuthorityGrant>, String> {
    let snapshot = store.snapshot();
    let scene = snapshot
        .scenes
        .iter()
        .find(|scene| scene.id == scene_id)
        .ok_or("会话不存在")?;
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == scene.agent_id)
        .ok_or("Agent 不存在")?;
    if !agent.memory_enabled {
        return Ok(None);
    }
    let Some(id) = agent.memory_provider.binding_id.as_deref() else {
        return Ok(None);
    };
    let value = binding(store, &agent.id, id)?;
    if !value.enabled || value.status != mewu_core::BindingStatus::Ready {
        return Ok(None);
    }
    let grant = grant(&value);
    Ok(authorize(plugins, &grant).is_ok().then_some(grant))
}

fn current_plugin_revision(
    store: &plugins::PluginStore,
    binding: &MemoryBindingView,
) -> Result<u64, String> {
    let plugin = store
        .snapshot()?
        .plugins
        .into_iter()
        .find(|entry| entry.manifest.id == binding.plugin_id)
        .ok_or("记忆插件已移除")?;
    store.memory_provider(
        &binding.plugin_id,
        plugin.revision,
        &binding.contribution_id,
    )?;
    Ok(plugin.revision)
}

pub(crate) fn changed(app: &AppHandle, agent_id: &str) {
    let _ = app.emit_to(
        "settings",
        "memory-state",
        serde_json::json!({"agentId":agent_id}),
    );
    app.state::<Host>().memory.wake.notify_one();
}

fn state(store: &Store, agent_id: &str) -> Result<AgentMemoryStatus, String> {
    store
        .memory_provider_status(agent_id, None, 100)
        .map_err(domain)
}

#[tauri::command]
pub fn get_memory_provider_state(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    agent_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    host.lock()?
        .store
        .memory_provider_status(&agent_id, cursor.as_deref(), limit.unwrap_or(100))
        .map_err(domain)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    version: String,
    latency_ms: u64,
}

#[tauri::command]
pub async fn probe_memory_provider(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    agent_id: String,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    endpoint: String,
    api_key: String,
) -> Result<ProbeResult, String> {
    settings(&window)?;
    let key = Zeroizing::new(api_key);
    credentials::validate_key(&key)?;
    let adapter = hindsight::HindsightHttp::new(&endpoint).map_err(adapter_error)?;
    let host = app.state::<Host>();
    let (permit, mut receive) = host.memory.register_request(
        memory_runtime::Origin {
            binding_id: String::new(),
            plugin_id: plugin_id.clone(),
            plugin_revision,
        },
        window.label(),
        &request_id,
    )?;
    {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins
            .store
            .memory_provider(&plugin_id, plugin_revision, &contribution_id)?;
        host.exit.ensure_running()?;
        state(&host.lock()?.store, &agent_id)?;
    }
    let began = tokio::time::Instant::now();
    let mut control = hindsight::Control {
        deadline: began + Duration::from_secs(12),
        cancel: &mut receive,
    };
    let version = adapter
        .probe(
            if key.is_empty() {
                None
            } else {
                Some(key.as_str())
            },
            &mut control,
        )
        .await
        .map_err(adapter_error)?;
    {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins
            .store
            .memory_provider(&plugin_id, plugin_revision, &contribution_id)?;
        host.exit.ensure_running()?;
    }
    drop(permit);
    Ok(ProbeResult {
        version: version.api_version,
        latency_ms: began.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

#[tauri::command]
pub fn cancel_memory_request(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    request_id: String,
) -> Result<(), String> {
    settings(&window)?;
    host.memory.cancel_request(window.label(), &request_id);
    Ok(())
}

#[tauri::command]
pub async fn create_memory_binding(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    agent_id: String,
    expected_provider_revision: u64,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    endpoint: String,
    api_key: String,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    let key = Zeroizing::new(api_key);
    credentials::validate_key(&key)?;
    let endpoint = hindsight::HindsightHttp::new(&endpoint)
        .map_err(adapter_error)?
        .normalized_endpoint();
    let host = app.state::<Host>();
    let (permit, mut canceled) = host.memory.register_request(
        memory_runtime::Origin {
            binding_id: String::new(),
            plugin_id: plugin_id.clone(),
            plugin_revision,
        },
        window.label(),
        &request_id,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins
            .store
            .memory_provider(&plugin_id, plugin_revision, &contribution_id)?;
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        if !matches!(
            canceled.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ) {
            return Err("记忆请求已取消".into());
        }
        let before = state(&engine.store, &agent_id)?;
        if before.selection.revision != expected_provider_revision {
            return Err("记忆来源已更改，请重新读取".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let credential_id = if key.is_empty() {
            None
        } else {
            Some(credentials::create_memory(&host.root, &id, &key)?)
        };
        let saved = engine.store.reserve_memory_binding(
            expected_provider_revision,
            mewu_core::HostNewMemoryBinding {
                id: id.clone(),
                agent_id: agent_id.clone(),
                endpoint,
                credential_id: credential_id.clone(),
                policy: MemoryPolicy {
                    recall: true,
                    explicit_retain: true,
                    completed_turn_sync: false,
                },
                plugin_id,
                contribution_id,
                plugin_revision,
            },
        );
        if let Err(error) = saved {
            if let Some(credential) = credential_id {
                credentials::discard_uncommitted_memory(&host.root, &id, &credential);
            }
            return Err(domain(error));
        }
        let result = state(&engine.store, &agent_id)?;
        drop(engine);
        drop(plugins);
        changed(&app, &agent_id);
        Ok(result)
    })
    .await
    .map_err(|_| "保存记忆连接中断")?
}

#[tauri::command]
pub fn select_memory_provider(
    app: AppHandle,
    window: tauri::WebviewWindow,
    agent_id: String,
    expected_provider_revision: u64,
    binding_id: Option<String>,
    expected_binding_revision: Option<u64>,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    let previous = state(&engine.store, &agent_id)?.selection;
    if previous.revision != expected_provider_revision {
        return Err("记忆来源已更改，请重新读取".into());
    }
    let mut revision = expected_binding_revision;
    let mut regranted = false;
    // Re-enabling a plugin does not revive an old request. Explicitly choosing
    // this connection renews its grant for future work under the new revision.
    if let Some(id) = &binding_id {
        let value = binding(&engine.store, &agent_id, id)?;
        if revision != Some(value.revision) {
            return Err("记忆连接已更改，请重新读取".into());
        }
        if !matches!(
            value.status,
            mewu_core::BindingStatus::Ready | mewu_core::BindingStatus::Suspended
        ) {
            return Err("记忆库尚未就绪或已经停用".into());
        }
        let current = current_plugin_revision(&plugins.store, &value)?;
        if current != value.plugin_revision || !value.enabled {
            let refreshed = engine
                .store
                .set_memory_binding_policy(id, value.revision, current, true, value.policy)
                .map_err(domain)?;
            revision = Some(refreshed.binding.revision);
            regranted = true;
        }
    }
    let selection = engine
        .store
        .select_memory_provider(
            &agent_id,
            expected_provider_revision,
            binding_id.as_deref(),
            revision,
        )
        .map_err(domain);
    // Regrant and selection are separate durable changes. Even if the second
    // write fails, settle runs canceled by the first before returning its error.
    if selection.is_err() && !regranted {
        return Err(selection.expect_err("checked"));
    }
    host.memory.cancel_where(|task| {
        previous
            .binding_id
            .as_ref()
            .is_some_and(|id| task.binding_id == *id)
            || binding_id.as_ref().is_some_and(|id| task.binding_id == *id)
    });
    let snapshot = engine.store.snapshot();
    crate::cancel_ended_runs(&mut engine, &snapshot);
    let result = state(&engine.store, &agent_id)?;
    crate::publish(&app, &snapshot);
    drop(engine);
    drop(plugins);
    changed(&app, &agent_id);
    selection?;
    Ok(result)
}

#[tauri::command]
pub fn set_memory_sync(
    app: AppHandle,
    window: tauri::WebviewWindow,
    agent_id: String,
    binding_id: String,
    expected_binding_revision: u64,
    completed_turn_sync: bool,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    let binding = binding(&engine.store, &agent_id, &binding_id)?;
    let plugin_revision = if completed_turn_sync {
        current_plugin_revision(&plugins.store, &binding)?
    } else {
        binding.plugin_revision
    };
    let policy = MemoryPolicy {
        completed_turn_sync,
        ..binding.policy
    };
    engine
        .store
        .set_memory_binding_policy(
            &binding_id,
            expected_binding_revision,
            plugin_revision,
            binding.enabled,
            policy,
        )
        .map_err(domain)?;
    host.memory
        .cancel_where(|task| task.binding_id == binding_id);
    let snapshot = engine.store.snapshot();
    crate::cancel_ended_runs(&mut engine, &snapshot);
    let result = state(&engine.store, &agent_id)?;
    crate::publish(&app, &snapshot);
    drop(engine);
    drop(plugins);
    changed(&app, &agent_id);
    Ok(result)
}

#[tauri::command]
pub fn retire_memory_binding(
    app: AppHandle,
    window: tauri::WebviewWindow,
    agent_id: String,
    binding_id: String,
    expected_binding_revision: u64,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    let host = app.state::<Host>();
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    binding(&engine.store, &agent_id, &binding_id)?;
    engine
        .store
        .retire_memory_binding(&binding_id, expected_binding_revision)
        .map_err(domain)?;
    host.memory
        .cancel_where(|task| task.binding_id == binding_id);
    let snapshot = engine.store.snapshot();
    crate::cancel_ended_runs(&mut engine, &snapshot);
    let result = state(&engine.store, &agent_id)?;
    crate::publish(&app, &snapshot);
    drop(engine);
    changed(&app, &agent_id);
    Ok(result)
}

#[tauri::command]
pub fn external_memory_page(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    agent_id: String,
    binding_id: String,
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<mewu_core::MemoryEvidencePage, String> {
    settings(&window)?;
    host.lock()?
        .store
        .external_memory_page(
            &agent_id,
            &binding_id,
            query.as_deref().unwrap_or(""),
            cursor.as_deref(),
            limit.unwrap_or(25),
        )
        .map_err(domain)
}

#[tauri::command]
pub fn external_memory_entry(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    agent_id: String,
    binding_id: String,
    evidence_id: String,
) -> Result<Option<mewu_core::MemoryEvidenceDetail>, String> {
    settings(&window)?;
    host.lock()?
        .store
        .external_memory_entry(&agent_id, &binding_id, &evidence_id)
        .map_err(domain)
}

#[tauri::command]
pub fn queue_external_memory(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    agent_id: String,
    binding_id: String,
    expected_binding_revision: u64,
    text: String,
) -> Result<mewu_core::WriteReceipt, String> {
    settings(&window)?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    authorize(
        &plugins.store,
        &grant(&binding(&engine.store, &agent_id, &binding_id)?),
    )?;
    let result = engine
        .store
        .queue_manual_memory_evidence(
            &agent_id,
            &binding_id,
            expected_binding_revision,
            &request_id,
            &text,
        )
        .map_err(domain)?;
    drop(engine);
    drop(plugins);
    changed(&app, &agent_id);
    Ok(result)
}

#[tauri::command]
pub fn forget_external_memory(
    app: AppHandle,
    window: tauri::WebviewWindow,
    agent_id: String,
    binding_id: String,
    evidence_id: String,
    expected_revision: u64,
) -> Result<AgentMemoryStatus, String> {
    settings(&window)?;
    let host = app.state::<Host>();
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    engine
        .store
        .forget_memory_evidence(&agent_id, &binding_id, &evidence_id, expected_revision)
        .map_err(domain)?;
    host.memory
        .cancel_where(|task| task.binding_id == binding_id);
    let result = state(&engine.store, &agent_id)?;
    let snapshot = engine.store.snapshot();
    crate::cancel_ended_runs(&mut engine, &snapshot);
    crate::publish(&app, &snapshot);
    drop(engine);
    changed(&app, &agent_id);
    Ok(result)
}

fn adapter_error(error: hindsight::AdapterError) -> String {
    use hindsight::ErrorCode;
    match error.code {
        ErrorCode::InvalidInput => "记忆服务地址或参数无效",
        ErrorCode::Cancelled => "记忆请求已取消",
        ErrorCode::Deadline | ErrorCode::Transport => "无法连接记忆服务",
        ErrorCode::Http(401 | 403) => "记忆服务未授权此连接",
        ErrorCode::UnsupportedVersion => "此版本的记忆服务暂不支持",
        ErrorCode::NotReady => "记忆服务尚未就绪",
        _ => "记忆服务响应无效",
    }
    .into()
}
