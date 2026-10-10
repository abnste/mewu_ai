// SPDX-License-Identifier: MPL-2.0
//! Explicit, source-fenced access to durable run records. Reads never start a
//! model, a tool process, or a memory-provider request.
use crate::{ai, recording, Host, HostSnapshot};
use mewu_core::{
    ContinuationDecision, ContinuationSelection, ContinueFromRecord, CoreError, JournalEntryDetail,
    JournalPage,
};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, WebviewWindow};

fn space(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能访问运行记录".into());
    }
    Ok(())
}

pub(crate) fn store_error(error: CoreError) -> String {
    match error {
        CoreError::Database(_) | CoreError::Json(_) => "无法读取或保存运行记录".into(),
        error => error.to_string(),
    }
}

#[tauri::command]
pub async fn get_run_journal(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    run_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<Option<JournalPage>, String> {
    space(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        engine
            .store
            .run_journal_page(&scene_id, &run_id, cursor.as_deref(), limit.unwrap_or(25))
            .map_err(store_error)
    })
    .await
    .map_err(|_| "运行记录读取中断")?
}

#[tauri::command]
pub async fn get_run_journal_event(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    run_id: String,
    event_id: String,
    expected_journal_revision: u64,
) -> Result<JournalEntryDetail, String> {
    space(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        engine
            .store
            .run_journal_entry(&scene_id, &run_id, &event_id, expected_journal_revision)
            .map_err(store_error)
    })
    .await
    .map_err(|_| "运行记录读取中断")?
}

#[tauri::command]
pub async fn preview_run_continuation(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    source_run_id: String,
    expected_journal_revision: u64,
    expected_checkpoint_seq: u64,
    selected_event_ids: Option<Vec<String>>,
) -> Result<ContinuationDecision, String> {
    space(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        engine
            .store
            .preview_run_continuation(&ContinuationSelection {
                scene_id,
                source_run_id,
                expected_journal_revision,
                expected_checkpoint_seq,
                selected_event_ids,
            })
            .map_err(store_error)
    })
    .await
    .map_err(|_| "运行记录读取中断")?
}

#[tauri::command]
pub async fn continue_run_from_journal(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    source_run_id: String,
    expected_journal_revision: u64,
    expected_checkpoint_seq: u64,
    expected_connection_id: String,
    expected_connection_revision: u64,
    selected_event_ids: Option<Vec<String>>,
) -> Result<HostSnapshot, String> {
    space(&window)?;
    recording::ensure_idle(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        recording::ensure_idle(&app)?;
        if host.capturing.load(Ordering::Acquire) {
            return Err("截图正在切换".into());
        }
        let preflight = crate::preflight_run(&host, &engine, &scene_id)?;
        if preflight.connection.id != expected_connection_id
            || preflight.connection.revision != expected_connection_revision
        {
            return Err("连接已更改，请重新操作".into());
        }
        let context = engine
            .store
            .begin_recorded_continuation(ContinueFromRecord {
                scene_id,
                source_run_id,
                expected_journal_revision,
                expected_checkpoint_seq,
                expected_connection_id,
                expected_connection_revision,
                selected_event_ids,
            })
            .map_err(store_error)?;
        // The new action attaches nothing. Its typed input projection resolves
        // the old messages' immutable attachments, never today's live selection.
        crate::start_run(
            &app,
            &mut engine,
            context,
            preflight,
            ai::PreparedAttachments::empty(),
        )
    })
    .await
    .map_err(|_| "启动续答中断")?
}
