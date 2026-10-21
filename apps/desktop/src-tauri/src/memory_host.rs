// SPDX-License-Identifier: MPL-2.0
//! Read-only historical memory data and explicit local retirement/deletion.
//! Network providers execute exclusively inside executable extensions. Old
//! binding IDs and evidence remain in the database for export/migration.
use crate::{plugins, Host};
use mewu_core::{AgentMemoryStatus, MemoryAuthorityGrant, MemoryBindingView, Store};
use tauri::{AppHandle, Emitter, Manager};

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

pub(crate) fn authorize(_: &plugins::PluginStore, _: &MemoryAuthorityGrant) -> Result<(), String> {
    Err("旧记忆连接已停用，请在独立记忆插件中迁移连接".into())
}

pub(crate) fn changed(app: &AppHandle, agent_id: &str) {
    let _ = app.emit_to(
        "settings",
        "memory-state",
        serde_json::json!({"agentId":agent_id}),
    );
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
