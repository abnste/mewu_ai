// SPDX-License-Identifier: MPL-2.0
//! Exit is a reversible preparation followed by durable cancellation and cleanup.
//! Never share this gate with the short-lived capture/window transition guard.
use crate::{Host, SceneCommand};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Mutex,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

const RUNNING: u8 = 0;
const PREPARING: u8 = 1;
const DRAINING: u8 = 2;
const READY: u8 = 3;
const EXIT_BUSY: &str = "正在退出，请稍候";

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchAction {
    Capture,
    Settings,
    Quit,
    Ignore,
}
pub fn launch_action(args: &[String]) -> LaunchAction {
    match args.get(1..) {
        Some([]) => LaunchAction::Capture,
        Some([arg]) if arg == "--quit" => LaunchAction::Quit,
        Some([arg]) if arg == "--settings" => LaunchAction::Settings,
        _ => LaunchAction::Ignore,
    }
}

/// A normal Linux launch has no usable AppIndicator click callback. Dispatch
/// its capture intent once, after Tauri has completed native initialization.
#[cfg(target_os = "linux")]
pub(crate) struct LinuxStartupCapture {
    pending: bool,
    settings: bool,
}
#[cfg(target_os = "linux")]
impl LinuxStartupCapture {
    pub(crate) fn new(startup_args: &[String]) -> Self {
        Self {
            pending: launch_action(startup_args) == LaunchAction::Capture,
            settings: launch_action(startup_args) == LaunchAction::Settings,
        }
    }
    pub(crate) fn take_ready(&mut self, event: &tauri::RunEvent) -> bool {
        matches!(event, tauri::RunEvent::Ready) && std::mem::take(&mut self.pending)
    }
    pub(crate) fn restore_at_ready(&self) -> bool {
        // Opening settings owns SpaceTransition until its native window has
        // been shown. Its completion, rather than Ready, dispatches restore.
        !self.settings
    }
}

struct Attempt {
    id: String,
    reply: Option<oneshot::Sender<Result<(), String>>>,
    completion: Option<oneshot::Sender<Result<(), String>>>,
    mcp: Option<crate::mcp_runtime::QuiesceTicket>,
}
#[derive(Default)]
pub struct ExitState {
    phase: AtomicU8,
    // Reserve input while still RUNNING so the renderer can finish existing
    // manual authoring. This is not general RUNNING admission.
    reserved: AtomicBool,
    attempt: Mutex<Option<Attempt>>,
}
impl ExitState {
    pub fn is_running(&self) -> bool {
        self.phase.load(Ordering::Acquire) == RUNNING
    }
    pub fn has_attempt(&self) -> bool {
        self.reserved.load(Ordering::Acquire)
    }
    pub fn accepts_new_operations(&self) -> bool {
        self.is_running() && !self.has_attempt()
    }
    /// Native/tray/hotkey/queued new-work entry points use this; existing
    /// authoring commits must retain ensure_running until renderer predrain ends.
    pub fn ensure_new_operation(&self) -> Result<(), String> {
        if self.accepts_new_operations() {
            Ok(())
        } else {
            Err(EXIT_BUSY.into())
        }
    }
    /// A synchronous native restore must not overtake an exit reservation.
    /// The caller owns SpaceTransition and has released Engine before entering;
    /// this closure may only show existing windows, with no IO, await or IPC.
    pub(crate) fn with_window_restore(&self, restore: impl FnOnce()) -> bool {
        let Ok(attempt) = self.attempt.lock() else {
            return false;
        };
        if !self.is_running() || attempt.is_some() {
            return false;
        }
        restore();
        true
    }
    pub fn is_ready(&self) -> bool {
        self.phase.load(Ordering::Acquire) == READY
    }
    pub fn ensure_running(&self) -> Result<(), String> {
        if self.is_running() {
            Ok(())
        } else {
            Err(EXIT_BUSY.into())
        }
    }
    pub fn ensure_background(&self) -> Result<(), String> {
        if self.phase.load(Ordering::Acquire) <= PREPARING {
            Ok(())
        } else {
            Err(EXIT_BUSY.into())
        }
    }
    pub fn ensure_scene_command(&self, command: &SceneCommand) -> Result<(), String> {
        let phase = self.phase.load(Ordering::Acquire);
        if (phase == RUNNING && (!self.has_attempt() || is_manual_drawing_history(command)))
            || (phase <= PREPARING && is_exit_flush_command(command))
        {
            Ok(())
        } else {
            Err(EXIT_BUSY.into())
        }
    }
    /// Existing card placement may still be pending when exit reserves input.
    /// The caller holds Engine through this proof and the subsequent Store write.
    /// Keep generic UpdateItem outside the exit-flush command whitelist.
    pub(crate) fn ensure_scene_command_with_store(
        &self,
        owner: &str,
        command: &SceneCommand,
        store: &mewu_core::Store,
    ) -> Result<(), String> {
        if self.ensure_scene_command(command).is_ok() {
            return Ok(());
        }
        if self.phase.load(Ordering::Acquire) > PREPARING
            || !is_existing_item_geometry_flush(owner, command, store)
        {
            return Err(EXIT_BUSY.into());
        }
        self.ensure_background()
    }
    pub fn allows_command(&self, command: &str) -> bool {
        self.accepts_new_operations()
            || (self.is_running()
                && self.has_attempt()
                && matches!(
                    command,
                    "apply_video_drawing"
                        | "get_video_annotation_vector"
                        | "get_video_annotation_text"
                        | "get_video_annotation_plan"
                        | "get_video_annotation_preview"
                        | "get_video_info"
                ))
            || matches!(
                command,
                "begin_exit_preparation"
                    | "finish_exit_preparation"
                    | "cancel_plugin_pin"
                    | "cancel_plugin_dictation"
                    | "cancel_plugin_code_scan"
                    | "cancel_video_request"
                    | "apply_video_edit"
                    | "apply_video_annotation_document"
                    | "edit_video_annotation_text"
                    | "end_capture_shortcut_edit"
                    | "cancel_capture_shortcut_portal"
                    | "get_capture_shortcut"
                    | "get_capture_preferences"
                    | "get_system_preferences"
                    | "settings_info"
                    | "read_license_document"
                    | "apply_scene_command"
                    | "apply_plugin_drawing"
                    | "apply_drawing_document"
                    | "save_blackboard_text"
                    | "get_drawing_layout_preview"
                    | "get_snapshot"
                    | "memory_page"
                    | "memory_entry"
                    | "get_run_journal"
                    | "get_run_journal_event"
                    | "preview_run_continuation"
                    | "get_memory_provider_state"
                    | "external_memory_page"
                    | "external_memory_entry"
                    | "cancel_memory_request"
                    | "read_artifact"
                    | "get_provider_presets"
                    | "get_plugins"
                    | "runtime_info"
                    | "get_frozen_scenes"
                    | "get_recording_status"
                    | "get_recording_audio"
                    | "cancel_run"
                    | "cancel_plugin_ocr"
                    | "cancel_connection_probe"
                    | "control_recording"
            )
    }
    fn begin(&self) -> Option<(String, oneshot::Receiver<Result<(), String>>)> {
        self.begin_with_completion(None)
    }
    fn begin_with_completion(
        &self,
        completion: Option<oneshot::Sender<Result<(), String>>>,
    ) -> Option<(String, oneshot::Receiver<Result<(), String>>)> {
        let mut attempt = self.attempt.lock().ok()?;
        if !self.is_running() || attempt.is_some() {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (send, receive) = oneshot::channel();
        *attempt = Some(Attempt {
            id: id.clone(),
            reply: Some(send),
            completion,
            mcp: None,
        });
        self.reserved.store(true, Ordering::Release);
        Some((id, receive))
    }
    fn prepare_with(&self, owner: &str, id: &str, cancel: impl FnOnce()) -> Result<(), String> {
        if owner != "space" {
            return Err("退出确认来源无效".into());
        }
        let attempts = self.attempt.lock().map_err(|_| "退出状态不可用")?;
        let attempt = attempts
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or("退出请求已结束")?;
        match self.phase.load(Ordering::Acquire) {
            PREPARING => return Ok(()), // exact duplicate does not cancel again
            RUNNING if attempt.reply.is_some() => {}
            _ => return Err("退出请求已结束".into()),
        }
        self.phase.store(PREPARING, Ordering::Release);
        // Only short registry/atomic revocations here: no Engine/plugin lock,
        // filesystem, window getter/destroy or waiting. Serialize against abort
        // so a late cancellation cannot close a gate after abort restored it.
        cancel();
        Ok(())
    }
    fn set_mcp(&self, id: &str, ticket: crate::mcp_runtime::QuiesceTicket) -> bool {
        let Ok(mut attempt) = self.attempt.lock() else {
            return false;
        };
        let Some(attempt) = attempt.as_mut().filter(|a| a.id == id) else {
            return false;
        };
        attempt.mcp = Some(ticket);
        true
    }
    fn complete(&self, id: &str) {
        if let Ok(mut attempt) = self.attempt.lock() {
            if let Some(attempt) = attempt.as_mut().filter(|a| a.id == id && self.is_ready()) {
                if let Some(completion) = attempt.completion.take() {
                    let _ = completion.send(Ok(()));
                }
            }
        }
    }
    /// Report a failed restart without reopening admission or canceling an
    /// unproven durable intent. The renderer must not wait forever for a receipt.
    fn fail_completion(&self, id: &str, error: &str) {
        if let Ok(mut attempt) = self.attempt.lock() {
            if let Some(attempt) = attempt.as_mut().filter(|a| a.id == id) {
                if let Some(completion) = attempt.completion.take() {
                    let _ = completion.send(Err(error.into()));
                }
            }
        }
    }
    fn acknowledge(&self, owner: &str, id: &str, success: bool) -> Result<(), String> {
        if owner != "space" {
            return Err("退出确认来源无效".into());
        }
        let mut attempt = self.attempt.lock().map_err(|_| "退出状态不可用")?;
        let attempt = attempt
            .as_mut()
            .filter(|attempt| attempt.id == id)
            .filter(|_| {
                let phase = self.phase.load(Ordering::Acquire);
                phase == PREPARING || (!success && phase == RUNNING)
            })
            .ok_or("退出请求已结束")?;
        // Duplicate acknowledgements for the same nonce are harmless.
        if let Some(reply) = attempt.reply.take() {
            let result = if success {
                Ok(())
            } else {
                Err("未能保存会话，已取消退出".into())
            };
            let _ = reply.send(result);
        }
        Ok(())
    }
    fn advance(&self, id: &str, phase: u8) -> bool {
        let Ok(attempt) = self.attempt.lock() else {
            return false;
        };
        if !attempt.as_ref().is_some_and(|attempt| attempt.id == id) {
            return false;
        }
        let current = self.phase.load(Ordering::Acquire);
        if (current == PREPARING && phase == DRAINING) || (current == DRAINING && phase == READY) {
            self.phase.store(phase, Ordering::Release);
            true
        } else {
            false
        }
    }
    #[cfg(test)]
    fn abort(&self, id: &str) -> bool {
        self.abort_with_restore(id, || {})
    }
    fn abort_with_restore(&self, id: &str, restore: impl FnOnce()) -> bool {
        self.abort_with_error(id, "已取消退出", restore)
    }
    fn abort_with_error(&self, id: &str, error: &str, restore: impl FnOnce()) -> bool {
        let Ok(mut attempt) = self.attempt.lock() else {
            return false;
        };
        if self.is_ready() || !attempt.as_ref().is_some_and(|attempt| attempt.id == id) {
            return false;
        }
        if let Some(ticket) = attempt.as_ref().and_then(|a| a.mcp.as_ref()) {
            if crate::mcp_runtime::resume_after_failed_quiesce(ticket).is_err() {
                if let Some(completion) = attempt.as_mut().and_then(|a| a.completion.take()) {
                    let _ = completion.send(Err("MCP 停止状态无法恢复，请重启应用".into()));
                }
                return false;
            }
        }
        if let Some(completion) = attempt.as_mut().and_then(|a| a.completion.take()) {
            let _ = completion.send(Err(error.into()));
        }
        *attempt = None;
        self.phase.store(RUNNING, Ordering::Release);
        // begin() takes the same mutex. A newer exit cannot close these gates
        // and then have this older attempt reopen them after releasing it.
        restore();
        self.reserved.store(false, Ordering::Release);
        true
    }
}

/// Only draft/ref are data writes allowed during capture. Finishing a transient
/// geometry group is classified separately; it never grants a replay exception.
pub fn is_draft_command(command: &SceneCommand) -> bool {
    matches!(
        command,
        SceneCommand::SetDraft { .. } | SceneCommand::SetRefs { .. }
    )
}

pub fn is_geometry_finish_command(command: &SceneCommand) -> bool {
    matches!(command, SceneCommand::FinishRegionGeometryEdit { .. })
}

// Accepted Undo/Redo intents may still be waiting behind a durable drawing
// write when exit reserves input. The renderer drains these while RUNNING,
// before begin_exit_preparation; they are never PREPARING flush commands.
fn is_manual_drawing_history(command: &SceneCommand) -> bool {
    matches!(
        command,
        SceneCommand::UndoDrawing { .. }
            | SceneCommand::RedoDrawing { .. }
            | SceneCommand::EditImageAnnotations {
                action: mewu_core::ImageAnnotationEdit::UndoDrawing
                    | mewu_core::ImageAnnotationEdit::RedoDrawing,
                ..
            }
    )
}

pub fn is_exit_flush_command(command: &SceneCommand) -> bool {
    is_draft_command(command)
        || is_geometry_finish_command(command)
        || matches!(command, SceneCommand::SetRegionGeometry { .. })
        || matches!(
            command,
            SceneCommand::EditImageAnnotations {
                action: mewu_core::ImageAnnotationEdit::AddDrawing { .. }
                    | mewu_core::ImageAnnotationEdit::UpdateDrawing { .. }
                    | mewu_core::ImageAnnotationEdit::UpdateDrawings { .. }
                    | mewu_core::ImageAnnotationEdit::EraseDrawingStrokes { .. },
                ..
            } | SceneCommand::AddDrawing { .. }
                | SceneCommand::UpdateDrawing { .. }
                | SceneCommand::UpdateDrawings { .. }
                | SceneCommand::EraseDrawingStrokes { .. }
        )
}

fn is_existing_item_geometry_flush(
    owner: &str,
    command: &SceneCommand,
    store: &mewu_core::Store,
) -> bool {
    let SceneCommand::UpdateItem { scene_id, item } = command else {
        return false;
    };
    if owner != "space" {
        return false;
    }
    let Ok(scene) = store.active_scene() else {
        return false;
    };
    if scene.id != *scene_id || scene.frozen || scene.closed {
        return false;
    }
    // No `..`: a future field must be reviewed before exit can admit it.
    let mewu_core::SpaceItem {
        id,
        asset,
        x,
        y,
        width,
        height,
        state,
        video_edit,
        video_annotations,
        image_annotations,
    } = item;
    let Some(current) = scene.items.iter().find(|current| current.id == *id) else {
        return false;
    };
    [*x, *y, *width, *height].iter().all(|v| v.is_finite())
        && x.abs() <= 16.
        && y.abs() <= 16.
        && *width > 0.
        && *height > 0.
        && *width <= 8.
        && *height <= 8.
        && current.asset == *asset
        && current.state == *state
        && current.video_edit == *video_edit
        && current.video_annotations == *video_annotations
        && current.image_annotations == *image_annotations
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitEvent<'a> {
    request_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[tauri::command]
pub fn begin_exit_preparation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.prepare_with(window.label(), &request_id, || {
        if let Some(extensions) = app.try_state::<crate::extension_host::ExtensionHost>() {
            extensions.cancel_all();
        }
        crate::video_host::cancel_all(&app);
        crate::image_host::cancel_all(&app);
        crate::raster_edit_host::cancel_all();
        crate::formula_layout::cancel_all();
        crate::capture_shortcut_host::invalidate_all(&app);
        crate::recording_audio_host::invalidate_all(&app);
        host.capture_jobs.cancel();
        host.capture_preferences.invalidate();
        host.system_preferences.invalidate();
        host.session_titles.cancel_all();
        if let Ok(resources) = &host.settings_resources {
            resources.invalidate();
        }
    })
}

#[tauri::command]
pub fn finish_exit_preparation(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    request_id: String,
    success: bool,
    error: Option<String>,
) -> Result<(), String> {
    // Renderer errors may contain draft text or secrets. Never relay arbitrary
    // renderer details into another surface or a diagnostic log.
    let _ = error;
    host.exit.acknowledge(window.label(), &request_id, success)
}

fn abort(app: &AppHandle, id: &str, error: &str) {
    let host = app.state::<Host>();
    if host.exit.abort_with_error(id, error, || {
        crate::hooks::resume(app);
        crate::formula_layout::resume_after_failed_exit();
        host.memory.resume();
    }) {
        let _ = app.emit_to(
            "space",
            "exit-preparation-canceled",
            ExitEvent {
                request_id: id,
                error: Some(error),
            },
        );
        let _ = app.emit_to("settings", "host-error", error);
    }
}

pub fn request_shutdown(app: &AppHandle) {
    if let Err(error) = start_shutdown(app, None, None, None) {
        let _ = app.emit_to("space", "host-error", &error);
        let _ = app.emit_to("settings", "host-error", &error);
    }
}

pub async fn request_storage_restart(
    app: &AppHandle,
    migration: crate::storage_host::MigrationTask,
) -> Result<(), String> {
    let (send, receive) = oneshot::channel();
    start_shutdown(app, Some(std::sync::Arc::new(migration)), Some(send), None)?;
    receive.await.map_err(|_| "迁移重启未完成".to_string())?
}

pub async fn request_update_restart(
    app: &AppHandle,
    update: crate::app_update::PreparedInstall,
) -> Result<(), String> {
    let (send, receive) = oneshot::channel();
    start_shutdown(app, None, Some(send), Some(update))?;
    receive
        .await
        .map_err(|_| "update_save_failed".to_string())?
}

fn start_shutdown(
    app: &AppHandle,
    migration: Option<std::sync::Arc<crate::storage_host::MigrationTask>>,
    completion: Option<oneshot::Sender<Result<(), String>>>,
    update: Option<crate::app_update::PreparedInstall>,
) -> Result<(), String> {
    static HOOK_PREFLIGHT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if !crate::hooks::enabled(app) {
        return start_shutdown_after_hooks(app, migration, completion, update);
    }
    if !app.state::<Host>().exit.accepts_new_operations()
        || HOOK_PREFLIGHT.swap(true, Ordering::AcqRel)
    {
        return Err(EXIT_BUSY.into());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::hooks::before_quit(&app).await;
        if let Err(error) = start_shutdown_after_hooks(&app, migration, completion, update) {
            crate::hooks::resume(&app);
            let _ = app.emit_to("settings", "host-error", &error);
            let _ = app.emit_to("space", "host-error", &error);
        }
        HOOK_PREFLIGHT.store(false, Ordering::Release);
    });
    Ok(())
}
fn start_shutdown_after_hooks(
    app: &AppHandle,
    migration: Option<std::sync::Arc<crate::storage_host::MigrationTask>>,
    completion: Option<oneshot::Sender<Result<(), String>>>,
    update: Option<crate::app_update::PreparedInstall>,
) -> Result<(), String> {
    let Some(host) = app.try_state::<Host>() else {
        return Err("应用尚未启动".into());
    };
    let begun =
        match crate::recording::begin_shutdown(app, || host.exit.begin_with_completion(completion))
        {
            Ok(begun) => begun,
            Err(error) => {
                return Err(error);
            }
        };
    let Some(((id, receive), recording)) = begun else {
        return Err(EXIT_BUSY.into());
    };
    crate::hooks::pause(app);
    let ticket = match crate::mcp_runtime::begin_quiesce() {
        Ok(ticket) => ticket,
        Err(error) => {
            abort(app, &id, &error);
            return Err(error);
        }
    };
    if !host.exit.set_mcp(&id, ticket.clone()) {
        let _ = crate::mcp_runtime::resume_after_failed_quiesce(&ticket);
        abort(app, &id, "退出状态不可用");
        return Err("退出状态不可用".into());
    }
    // Revoke listening before the renderer collects its final draft state.
    crate::speech_host::cancel_all(app);
    crate::quick_capture::detach(app);
    if let Some(extensions) = app.try_state::<crate::extension_host::ExtensionHost>() {
        extensions.cancel_all();
    }
    crate::code_host::cancel_all(app);
    host.session_titles.cancel_all();
    // The renderer first locks fresh input and drains already registered manual
    // authoring while RUNNING. It then calls begin_exit_preparation with this
    // exact nonce before the ordinary PREPARING flush/acknowledgement.
    // Pending first-show requests must settle before renderer preparation
    // awaits its operations. Already shown pins survive an aborted exit.
    crate::pin_host::cancel_pending(app, "space");
    if app
        .emit_to(
            "space",
            "prepare-exit",
            ExitEvent {
                request_id: &id,
                error: None,
            },
        )
        .is_err()
    {
        abort(app, &id, "无法保存会话，已取消退出");
        return Err("无法保存会话，已取消退出".into());
    }
    // Single-instance callbacks run on the native message thread. Return now;
    // neither renderer acknowledgement nor recording finalization may block it.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let prepared = match tokio::time::timeout(Duration::from_secs(10), receive).await {
            Ok(Ok(result)) => result,
            _ => Err("保存会话超时，已取消退出".into()),
        };
        if let Err(error) = prepared {
            abort(&app, &id, &error);
            return;
        }
        if !app.state::<Host>().exit.advance(&id, DRAINING) {
            return;
        }
        let target = app.clone();
        let settled = tauri::async_runtime::spawn_blocking(move || settle(&target))
            .await
            .map_err(|_| "停止后台任务失败，已取消退出".to_string())
            .and_then(|result| result);
        if let Err(error) = settled {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::scroll_host::cancel_for_shutdown(&app).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::recording::stop_for_shutdown(&app, recording).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::pin_host::drain_pending(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::asset_host::drain(&app).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::hooks::drain(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = app
            .state::<Host>()
            .memory
            .drain(Duration::from_secs(5))
            .await
        {
            abort(&app, &id, &error);
            return;
        }
        // Closing this registry is irreversible. Do it only after all operations
        // which can fail and require retaining the app have succeeded.
        if !crate::speech_worker::wait_until_idle(Duration::from_secs(5)).await {
            abort(&app, &id, "语音输入尚未停止，已取消退出");
            return;
        }
        if !crate::quick_capture::wait_until_idle(Duration::from_secs(16)).await {
            abort(&app, &id, "截图分析尚未停止，已取消退出");
            return;
        }
        let formulas_idle = tauri::async_runtime::spawn_blocking(|| {
            crate::formula_layout::wait_until_idle(Duration::from_secs(5))
        })
        .await
        .unwrap_or(false);
        if !formulas_idle {
            abort(&app, &id, "公式绘制尚未停止，已取消退出");
            return;
        }
        let tables_idle = tokio::time::timeout(Duration::from_secs(5), async {
            while !crate::visual_annotation_host::table_workers_are_idle() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        if tables_idle.is_err() {
            abort(&app, &id, "表格绘制尚未停止，已取消退出");
            return;
        }
        if let Err(error) = crate::capture_shortcut_host::drain(&app, Duration::from_secs(5)).await
        {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::recording_audio_host::drain(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::code_host::drain(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::video_host::drain(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::image_host::drain(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::raster_edit_host::drain(Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        let host = app.state::<Host>();
        if let Err(error) = host.session_titles.drain(Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = host.capture_jobs.drain(Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = host.system_preferences.drain(Duration::from_secs(5)).await {
            abort(&app, &id, &error.to_string());
            return;
        }
        if let Err(error) = host.capture_preferences.drain(Duration::from_secs(5)).await {
            abort(&app, &id, &error.to_string());
            return;
        }
        if let Ok(resources) = &host.settings_resources {
            if let Err(error) = resources.drain(Duration::from_secs(5)).await {
                abort(&app, &id, &error.to_string());
                return;
            }
        }
        if let Err(error) = crate::storage_host::drain_picker(&app, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        if let Err(error) = crate::mcp_runtime::drain(&ticket, Duration::from_secs(5)).await {
            abort(&app, &id, &error);
            return;
        }
        // Durable relocation intent is created only after every reversible exit
        if let Some(extensions) = app.try_state::<crate::extension_host::ExtensionHost>() {
            if let Err(error) = extensions.drain(Duration::from_secs(5)).await {
                abort(&app, &id, &error);
                return;
            }
        }
        // Durable relocation intent is created only after every reversible exit
        // step has succeeded. The new process copies after the old lifetime lock
        // is released by the OS, before opening any business database.
        let pending = if let Some(migration) = &migration {
            // Recheck after admitted configuration writes have settled; the
            // earlier button check cannot authorize a newly changed MCP path.
            if let Err(error) = crate::storage_host::migration_preflight(&app) {
                abort(&app, &id, &error);
                return;
            }
            let task = std::sync::Arc::clone(migration);
            match tauri::async_runtime::spawn_blocking(move || task.prepare()).await {
                Ok(Ok(id)) => Some(id),
                Ok(Err(error)) => {
                    abort(&app, &id, &error);
                    return;
                }
                Err(_) => {
                    abort(&app, &id, "无法准备数据迁移，已取消退出");
                    return;
                }
            }
        } else {
            None
        };
        // The installed bundle may have been replaced while downloading or
        // while drafts/native panels drained. Refuse while exit is reversible.
        if let Some(prepared) = update.as_ref() {
            if let Err(error) = prepared.preflight() {
                abort(&app, &id, &error);
                return;
            }
        }
        if let Err(error) = crate::mcp_runtime::final_shutdown(&ticket) {
            if let (Some(migration), Some(pending)) = (&migration, &pending) {
                if migration.cancel_requested(pending).is_err() {
                    app.state::<Host>()
                        .exit
                        .fail_completion(&id, "迁移请求尚未撤销，请重启应用");
                    let _ = app.emit_to("settings", "host-error", "迁移请求尚未撤销，请重启应用");
                    return;
                }
            }
            abort(&app, &id, &error);
            return;
        }
        if app.state::<Host>().exit.advance(&id, READY) {
            // Stop only after reversible exit checks have succeeded. Request
            // under Engine, then wait outside its lock and off the UI thread.
            // FULL commits are already durable in WAL; stalled maintenance must
            // not strand an irreversible exit. Process exit releases any final
            // in-flight I/O before the next owner obtains the storage lock.
            let maintenance = app
                .state::<Host>()
                .lock()
                .ok()
                .map(|mut engine| engine.store.pause_checkpoint_maintenance());
            if let Some(maintenance) = maintenance {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    tauri::async_runtime::spawn_blocking(move || maintenance.wait()),
                )
                .await;
            }
            // No reversible exit work remains. Release the screenshot key
            // before installer/restart handoff; process exit is the final OS
            // fallback if native hook teardown itself reports a failure.
            if let Err(error) = crate::system_capture_shortcut::shutdown(&app) {
                let _ = app.emit_to("settings", "host-error", error);
            }
            // Workers and durable writes have drained. Presentation recovery
            // must not strand an irreversible exit; the OS destroys these HWNDs.
            #[cfg(windows)]
            let _ =
                crate::shared_capture_windows::abandon_for_process_exit(&app.state::<Host>().exit);
            crate::pin_host::shutdown(&app);
            crate::app_control_host::shutdown(&app);
            crate::hooks::shutdown(&app);
            if let Some(update) = update {
                // Windows installer launch exits the process. Do this only once
                // drafts, media, native workers and durable cancellations settled.
                if let Err(error) = update.install(&app.state::<Host>().exit) {
                    app.state::<Host>().exit.fail_completion(&id, &error);
                    // MCP final shutdown cannot be rolled back. Restore service
                    // using the current executable, never leave a half-closed app.
                    app.request_restart();
                } else {
                    // The Mac/Linux updater returns after replacing its package;
                    // Windows's installer exits the process inside install().
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    {
                        app.state::<Host>().exit.complete(&id);
                        app.request_restart();
                    }
                }
            } else if migration.is_some() {
                app.state::<Host>().exit.complete(&id);
                app.request_restart();
            } else {
                app.state::<Host>().exit.complete(&id);
                app.exit(0);
            }
        }
    });
    Ok(())
}

fn settle(app: &AppHandle) -> Result<(), String> {
    crate::pixel_sampler::invalidate(app);
    crate::pin_host::cancel_pending(app, "space");
    let host = app.state::<Host>();
    // Keep the same lock order as plugin mutations. A mutation admitted before
    // preparation must finish before this cancellation snapshot is taken.
    let _plugins = host
        .plugins
        .lock()
        .map_err(|_| "插件状态不可用，已取消退出")?;
    let mut engine = host.lock()?;
    engine.ocr_jobs.cancel_owner("space");
    engine.translation_jobs.cancel_owner("space");
    host.connection_probes
        .lock()
        .map_err(|_| "连接状态不可用，已取消退出")?
        .cancel_all();
    settle_runs(&mut engine, |snapshot| {
        crate::publish(app, snapshot);
    })
}

fn settle_runs(
    engine: &mut crate::Engine,
    mut publish: impl FnMut(&mewu_core::Snapshot),
) -> Result<(), String> {
    let scenes: Vec<_> = engine
        .store
        .snapshot()
        .scenes
        .into_iter()
        .filter(|scene| {
            scene
                .run
                .as_ref()
                .is_some_and(|run| run.status == mewu_core::RunStatus::Running)
        })
        .map(|scene| scene.id)
        .collect();
    for scene_id in scenes {
        // Never discard the database failure and then exit. Earlier successful
        // cancellations remain durable; uncancelled runs retain their data.
        let snapshot = engine
            .store
            .cancel_run(&scene_id)
            .map_err(|_| "无法保存后台任务状态，已取消退出")?;
        crate::cancel_ended_runs(engine, &snapshot);
        publish(&snapshot);
    }
    for (_, sender) in engine.runs.drain() {
        let _ = sender.send(());
    }
    engine.plugin_runs.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn explicit_close_or_freeze_does_not_wait_after_exit_reserved() {
        let state = super::ExitState::default();
        let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _capture = crate::SpaceTransition::try_begin(&active).unwrap();
        let (_, _reply) = state.begin().unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            crate::SpaceTransition::acquire_explicit(
                &active,
                &state,
                std::time::Duration::from_secs(2),
            ),
        )
        .await
        .expect("reserved exit rejects without waiting for the busy transition");
        assert!(result.is_err_and(|error| error == super::EXIT_BUSY));
        assert!(active.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn native_restore_never_runs_after_exit_reserved_or_preparation_began() {
        let state = super::ExitState::default();
        assert!(state.with_window_restore(|| {}));
        let (id, _) = state.begin().unwrap();
        assert!(!state.with_window_restore(|| panic!("exit already owns admission")));
        state.prepare_with("space", &id, || {}).unwrap();
        assert!(!state.with_window_restore(|| panic!("preparing exit must not show windows")));
    }

    #[test]
    fn exit_reservation_cannot_overtake_an_admitted_synchronous_window_restore() {
        use std::sync::{mpsc, Arc, Mutex};
        let state = Arc::new(super::ExitState::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let restoring = state.clone();
        let restore_events = events.clone();
        let restore = std::thread::spawn(move || {
            restoring.with_window_restore(|| {
                started.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                restore_events.lock().unwrap().push("show");
            })
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let quitting = state.clone();
        let exit_events = events.clone();
        let quit = std::thread::spawn(move || {
            let result = quitting.begin();
            exit_events.lock().unwrap().push("exit-reserved");
            result.is_some()
        });
        release.send(()).unwrap();
        assert!(restore.join().unwrap());
        assert!(quit.join().unwrap());
        assert_eq!(*events.lock().unwrap(), ["show", "exit-reserved"]);
        assert!(!state.with_window_restore(|| panic!("late restore after exit")));
    }
    use super::*;

    fn item_geometry_store() -> (mewu_core::Store, String) {
        use mewu_core::*;
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.active_scene().unwrap().id.clone();
        let asset = |kind: AssetKind, name: &str| Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind,
            path: format!("synthetic/{name}"),
            width: Some(160),
            height: Some(90),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
            capture_coordinates: None,
        };
        let video = asset(AssetKind::Video, "video.mp4");
        store
            .import_assets(
                &scene_id,
                vec![asset(AssetKind::Image, "image.png"), video.clone()],
            )
            .unwrap();
        let item_id = store.active_scene().unwrap().items[1].id.clone();
        let verified = VerifiedVideoSource {
            asset: video.clone(),
            duration_ticks: 50_000_000,
            width: 160,
            height: 90,
        };
        store
            .set_video_range(
                &VideoTarget {
                    scene_id: scene_id.clone(),
                    item_id: item_id.clone(),
                    source_id: video.id.clone(),
                    expected_revision: 0,
                },
                &verified,
                None,
                Some(VideoRange {
                    start_ticks: 10_000_000,
                    end_ticks: 40_000_000,
                }),
            )
            .unwrap();
        store
            .append_video_annotations(
                &VideoAnnotationTarget {
                    scene_id: scene_id.clone(),
                    item_id,
                    source_id: video.id,
                    expected_range_revision: 1,
                    expected_annotation_revision: 0,
                },
                &VerifiedVideoAnnotationSource::new(verified, "ab".repeat(32)).unwrap(),
                vec![VideoAnnotationDraft {
                    interval: VideoRange {
                        start_ticks: 20_000_000,
                        end_ticks: 30_000_000,
                    },
                    primitive: VideoAnnotationPrimitive::Rect {
                        bounds: VideoPixelRect {
                            x: 10.,
                            y: 20.,
                            width: 40.,
                            height: 30.,
                        },
                        style: VideoRectStyle {
                            color: "#FF0000".into(),
                            stroke_width: 2.,
                            opacity: 1.,
                        },
                    },
                }],
                vec![],
            )
            .unwrap();
        let mut board = asset(AssetKind::Image, "黑板.png");
        board.path = format!("synthetic/{}.board.png", board.id);
        store.create_blackboard_document(&scene_id, board).unwrap();
        let expected = store.active_scene().unwrap().clone();
        let mut preview = asset(AssetKind::Image, "黑板.png");
        preview.path = format!("synthetic/{}.board.png", preview.id);
        store
            .finish_blackboard_document(&expected, preview)
            .unwrap();
        (store, scene_id)
    }

    fn item_geometry_command(scene_id: &str, mut item: mewu_core::SpaceItem) -> SceneCommand {
        item.x = -0.5;
        item.y = 0.1;
        item.width = 1.75;
        item.height = 1.25;
        SceneCommand::UpdateItem {
            scene_id: scene_id.into(),
            item,
        }
    }

    #[test]
    fn item_geometry_exit_flush_persists_media_without_rewriting_content() {
        let (mut store, scene_id) = item_geometry_store();
        let items = store.active_scene().unwrap().items.clone();
        assert_eq!(items.len(), 3); // imported image, trimmed/annotated video, returned board
        assert!(items[1].video_edit.is_some() && items[1].video_annotations.is_some());
        assert!(items[2]
            .state
            .as_ref()
            .unwrap()
            .contains_key("blackboardSceneId"));
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        for (index, original) in items.into_iter().enumerate() {
            if index == 1 {
                state.prepare_with("space", &id, || {}).unwrap();
            }
            let command = item_geometry_command(&scene_id, original.clone());
            assert!(!is_exit_flush_command(&command));
            assert!(state.ensure_scene_command(&command).is_err());
            state
                .ensure_scene_command_with_store("space", &command, &store)
                .unwrap();
            store.apply(command).unwrap();
            let saved = store
                .active_scene()
                .unwrap()
                .items
                .iter()
                .find(|item| item.id == original.id)
                .unwrap();
            assert_eq!(
                (saved.x, saved.y, saved.width, saved.height),
                (-0.5, 0.1, 1.75, 1.25)
            );
            let mut expected = original;
            expected.x = saved.x;
            expected.y = saved.y;
            expected.width = saved.width;
            expected.height = saved.height;
            assert_eq!(*saved, expected);
        }
        assert!(state.advance(&id, DRAINING));
    }

    #[test]
    fn item_geometry_exit_flush_rejects_content_source_and_scene_changes() {
        let (mut store, scene_id) = item_geometry_store();
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        let before = store.snapshot();
        let video = store.active_scene().unwrap().items[1].clone();
        let board = store.active_scene().unwrap().items[2].clone();
        let mut changed = vec![];
        let mut item = video.clone();
        item.id = uuid::Uuid::new_v4().to_string();
        changed.push(item);
        let mut item = video.clone();
        item.asset.id = uuid::Uuid::new_v4().to_string();
        changed.push(item);
        let mut item = video.clone();
        item.asset.path.push_str(".replaced");
        changed.push(item);
        let mut item = video.clone();
        item.asset.width = Some(320);
        changed.push(item);
        let mut item = video.clone();
        item.state = Some(std::collections::BTreeMap::from([(
            "nested".into(),
            serde_json::json!({"value":1}),
        )]));
        changed.push(item);
        let mut item = video.clone();
        item.video_edit.as_mut().unwrap().range = None;
        changed.push(item);
        let mut item = video.clone();
        item.video_edit.as_mut().unwrap().undo.clear();
        changed.push(item);
        let mut item = video.clone();
        item.video_annotations.as_mut().unwrap().objects[0]
            .interval
            .start_ticks += 1;
        changed.push(item);
        let mut item = video.clone();
        item.video_annotations.as_mut().unwrap().undo.clear();
        changed.push(item);
        let mut item = video.clone();
        item.video_annotations = None;
        changed.push(item);
        let mut item = board.clone();
        item.state.as_mut().unwrap().insert(
            "blackboardSceneId".into(),
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        );
        changed.push(item);
        for item in changed {
            let command = item_geometry_command(&scene_id, item);
            assert!(state
                .ensure_scene_command_with_store("space", &command, &store)
                .is_err());
            assert_eq!(store.snapshot(), before);
        }
        let command = item_geometry_command(&scene_id, video.clone());
        for owner in ["settings", "pin-object", ""] {
            assert!(state
                .ensure_scene_command_with_store(owner, &command, &store)
                .is_err());
        }
        let wrong_scene = item_geometry_command(&uuid::Uuid::new_v4().to_string(), video.clone());
        assert!(state
            .ensure_scene_command_with_store("space", &wrong_scene, &store)
            .is_err());
        store
            .apply(SceneCommand::RemoveItem {
                scene_id: scene_id.clone(),
                item_id: video.id,
            })
            .unwrap();
        assert!(state
            .ensure_scene_command_with_store("space", &command, &store)
            .is_err());
        let stale_board = item_geometry_command(&scene_id, board);
        store.apply(SceneCommand::NewScene).unwrap();
        assert!(store.scene(&scene_id).unwrap().frozen);
        assert!(state
            .ensure_scene_command_with_store("space", &stale_board, &store)
            .is_err());
        assert!(state
            .ensure_scene_command_with_store("space", &SceneCommand::NewScene, &store)
            .is_err());
    }

    #[test]
    fn item_geometry_exit_flush_rechecks_lifecycle_and_geometry_bounds() {
        let (store, scene_id) = item_geometry_store();
        let original = store.active_scene().unwrap().items[0].clone();
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        let command = item_geometry_command(&scene_id, original.clone());
        state
            .ensure_scene_command_with_store("space", &command, &store)
            .unwrap();
        for (field, value) in [
            (0, f64::NAN),
            (1, f64::INFINITY),
            (2, f64::NEG_INFINITY),
            (3, f64::NAN),
            (0, 16.001),
            (1, -16.001),
            (2, 0.),
            (3, -0.1),
            (2, 8.001),
            (3, 8.001),
        ] {
            let mut item = original.clone();
            match field {
                0 => item.x = value,
                1 => item.y = value,
                2 => item.width = value,
                _ => item.height = value,
            }
            assert!(state
                .ensure_scene_command_with_store(
                    "space",
                    &SceneCommand::UpdateItem {
                        scene_id: scene_id.clone(),
                        item
                    },
                    &store
                )
                .is_err());
        }
        let mut boundary = original;
        boundary.x = -16.;
        boundary.y = 16.;
        boundary.width = 8.;
        boundary.height = 8.;
        state
            .ensure_scene_command_with_store(
                "space",
                &SceneCommand::UpdateItem {
                    scene_id,
                    item: boundary,
                },
                &store,
            )
            .unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        state
            .ensure_scene_command_with_store("space", &command, &store)
            .unwrap();
        assert!(state.advance(&id, DRAINING));
        assert!(state
            .ensure_scene_command_with_store("space", &command, &store)
            .is_err());
        assert!(state.advance(&id, READY));
        assert!(state
            .ensure_scene_command_with_store("space", &command, &store)
            .is_err());
    }

    #[test]
    fn migration_receipt_waits_for_ready_and_stale_attempt_cannot_complete_or_abort_it() {
        let state = ExitState::default();
        let (send, mut receive) = oneshot::channel();
        let (id, _) = state.begin_with_completion(Some(send)).unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        state.complete(&id);
        assert_eq!(receive.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        assert!(state.abort_with_error(&id, "草稿未保存", || {}));
        assert_eq!(receive.try_recv().unwrap(), Err("草稿未保存".into()));
        let (send, mut receive) = oneshot::channel();
        let (next, _) = state.begin_with_completion(Some(send)).unwrap();
        state.prepare_with("space", &next, || {}).unwrap();
        state.complete(&id);
        assert!(!state.abort_with_error(&id, "旧请求", || panic!("stale request")));
        assert!(state.advance(&next, DRAINING));
        state.complete(&next);
        assert_eq!(receive.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        assert!(state.advance(&next, READY));
        state.complete(&next);
        assert_eq!(receive.try_recv().unwrap(), Ok(()));
        state.complete(&next);
        assert!(!state.abort(&next));
    }
    #[test]
    fn failed_closed_migration_settles_receipt_without_reopening_admission() {
        let state = ExitState::default();
        let (send, mut receive) = oneshot::channel();
        let (id, _) = state.begin_with_completion(Some(send)).unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        assert!(state.advance(&id, DRAINING));
        state.fail_completion("stale", "不得影响新请求");
        assert_eq!(receive.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        state.fail_completion(&id, "请求尚未撤销");
        assert_eq!(receive.try_recv().unwrap(), Err("请求尚未撤销".into()));
        assert!(!state.is_running());
        assert!(!state.is_ready());
        assert!(state.begin().is_none());
    }
    #[test]
    fn abort_restores_worker_admission_before_another_exit_can_begin() {
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        let restored = std::sync::atomic::AtomicBool::new(false);
        assert!(state.abort_with_restore(&id, || {
            assert!(state.is_running());
            assert!(state.attempt.try_lock().is_err());
            restored.store(true, Ordering::Release);
        }));
        let (next, _) = state.begin().unwrap();
        state.prepare_with("space", &next, || {}).unwrap();
        assert!(restored.load(Ordering::Acquire));
        assert_ne!(next, id);
        assert!(!state.abort_with_restore(&id, || panic!("stale exit cannot reopen workers")));
        assert!(!state.is_running());
    }
    #[test]
    fn sqlite_cancellation_failure_keeps_data_and_transport_then_retry_settles_durably() {
        let path = std::env::temp_dir().join(format!("mewu-exit-{}.db", uuid::Uuid::new_v4()));
        assert!(!path
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        let mut store = mewu_core::Store::open(&path).unwrap();
        let scene_id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "合成退出测试".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene_id).unwrap();
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "下一条未发送草稿".into(),
            })
            .unwrap();
        let (sender, mut receiver) = oneshot::channel();
        let mut engine = crate::Engine {
            store,
            optional: crate::optional_capabilities::Authorizations::test_defaults(),
            agent_runs: Default::default(),
            runs: std::collections::HashMap::from([(run.run_id.clone(), sender)]),
            plugin_runs: Default::default(),
            visual_runs: Default::default(),
            ocr_jobs: Default::default(),
            translation_jobs: Default::default(),
        };
        let before = engine.store.snapshot();
        let db = rusqlite::Connection::open(&path).unwrap();
        let payload: String = db
            .query_row("SELECT payload FROM app_state WHERE id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        db.execute_batch("CREATE TRIGGER fail_exit BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        assert!(state.advance(&id, DRAINING));
        assert!(settle_runs(&mut engine, |_| panic!(
            "must not publish a failed transaction"
        ))
        .is_err());
        assert_eq!(engine.store.snapshot(), before);
        assert!(engine.store.run_is_active(&scene_id, &run.run_id));
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(
            db.query_row("SELECT payload FROM app_state WHERE id=1", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            payload
        );
        assert!(state.abort(&id));
        assert!(state.is_running());
        db.execute_batch("DROP TRIGGER fail_exit;").unwrap();
        settle_runs(&mut engine, |_| {}).unwrap();
        assert!(receiver.try_recv().is_ok());
        let after = engine.store.snapshot();
        assert_eq!(after.active_scene_id, scene_id);
        assert_eq!(
            after
                .scenes
                .iter()
                .find(|scene| scene.id == scene_id)
                .unwrap()
                .draft,
            "下一条未发送草稿"
        );
        assert!(!engine.store.run_is_active(&scene_id, &run.run_id));
        drop(db);
        drop(engine);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
    #[test]
    fn only_exact_quit_is_routed_as_exit_and_never_capture() {
        for args in [
            vec!["mewu", "--quit", "extra"],
            vec!["mewu", "--quitt"],
            vec!["mewu", "--settings", "extra"],
            vec!["mewu", "--setting"],
            vec!["mewu", "file"],
        ] {
            assert_eq!(
                launch_action(&args.into_iter().map(String::from).collect::<Vec<_>>()),
                LaunchAction::Ignore
            );
        }
        assert_eq!(
            launch_action(&["mewu".into(), "--quit".into()]),
            LaunchAction::Quit
        );
        assert_eq!(launch_action(&["mewu".into()]), LaunchAction::Capture);
        assert_eq!(
            launch_action(&["mewu".into(), "--settings".into()]),
            LaunchAction::Settings
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_startup_capture_waits_for_ready_and_dispatches_once() {
        let mut startup = LinuxStartupCapture::new(&["mewu".into()]);
        assert!(!startup.take_ready(&tauri::RunEvent::MainEventsCleared));
        assert!(!startup.take_ready(&tauri::RunEvent::Resumed));
        assert!(startup.take_ready(&tauri::RunEvent::Ready));
        assert!(!startup.take_ready(&tauri::RunEvent::Ready));
        assert!(!startup.take_ready(&tauri::RunEvent::MainEventsCleared));
        assert!(!startup.take_ready(&tauri::RunEvent::Resumed));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_startup_capture_preserves_background_settings_and_exact_arguments() {
        for args in [
            vec!["mewu", "--background"],
            vec!["mewu", "--background", "--startup-owner=test"],
            vec!["mewu", "--settings"],
            vec!["mewu", "--quit"],
            vec!["mewu", "--settings", "extra"],
            vec!["mewu", "--verify-update", "/tmp/update"],
            vec!["mewu", "--mewu-linux-worker"],
            vec!["mewu", "file"],
            vec![],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            let mut startup = LinuxStartupCapture::new(&args);
            assert!(!startup.take_ready(&tauri::RunEvent::Ready), "{args:?}");
            assert!(!startup.take_ready(&tauri::RunEvent::Ready), "{args:?}");
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_shortcut_restore_defers_exact_settings_launch_until_window_transition_ends() {
        let settings = LinuxStartupCapture::new(&["mewu".into(), "--settings".into()]);
        assert!(!settings.restore_at_ready());
        for args in [
            vec!["mewu", "--background"],
            vec!["mewu"],
            vec!["mewu", "--settings", "extra"],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(LinuxStartupCapture::new(&args).restore_at_ready());
        }
    }
    #[tokio::test]
    async fn acknowledgement_is_nonce_and_owner_bound_and_deduplicated() {
        let state = ExitState::default();
        let (id, reply) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        assert!(state.begin().is_none());
        assert!(state.acknowledge("settings", &id, true).is_err());
        assert!(state.acknowledge("space", "old", true).is_err());
        state.acknowledge("space", &id, true).unwrap();
        state.acknowledge("space", &id, false).unwrap();
        assert!(reply.await.unwrap().is_ok());
        assert!(state.advance(&id, DRAINING));
        assert!(state.acknowledge("space", &id, true).is_err());
        assert!(state.advance(&id, READY));
        assert!(!state.abort(&id));
        assert!(state.is_ready());
    }
    #[tokio::test]
    async fn failed_or_expired_attempt_unlocks_and_late_ack_cannot_finish_next_exit() {
        let state = ExitState::default();
        let (old, reply) = state.begin().unwrap();
        state.prepare_with("space", &old, || {}).unwrap();
        state.acknowledge("space", &old, false).unwrap();
        assert!(reply.await.unwrap().is_err());
        assert!(state.abort(&old));
        assert!(state.is_running());
        let (next, _) = state.begin().unwrap();
        state.prepare_with("space", &next, || {}).unwrap();
        assert!(!state.abort(&old));
        assert!(!state.advance(&old, DRAINING));
        assert!(state.acknowledge("space", &old, true).is_err());
        assert!(state.abort(&next));
        assert!(state.is_running());
    }
    #[test]
    fn drawing_draft_handoff_only_allows_upserts_until_draining() {
        let drawing = mewu_core::Drawing {
            id: "text".into(),
            kind: mewu_core::DrawingKind::Text,
            color: "#ffffff".into(),
            stroke_width: 4.,
            points: vec![mewu_core::DrawingPoint { x: 10., y: 10. }],
            text: Some("已修改文字".into()),
            font_size: Some(20.),
            text_style: None,
            origin: None,
            rich: None,
         text_runs: None, };
        let add = SceneCommand::AddDrawing {
            scene_id: "s".into(),
            background_id: "b".into(),
            region_id: "r".into(),
            expected_revision: 1,
            drawing: drawing.clone(),
        };
        let update = SceneCommand::UpdateDrawing {
            scene_id: "s".into(),
            background_id: "b".into(),
            region_id: "r".into(),
            expected_revision: 1,
            drawing,
        };
        let remove = SceneCommand::RemoveDrawing {
            scene_id: "s".into(),
            background_id: "b".into(),
            region_id: "r".into(),
            expected_revision: 1,
            drawing_id: "text".into(),
        };
        let remove_batch = SceneCommand::RemoveDrawings {
            scene_id: "s".into(),
            background_id: "b".into(),
            region_id: "r".into(),
            expected_revision: 1,
            drawing_ids: vec!["text".into(), "other".into()],
        };
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        assert!(state.allows_command("apply_plugin_drawing"));
        assert!(state.ensure_scene_command(&add).is_ok());
        assert!(state.ensure_scene_command(&update).is_ok());
        assert!(state.ensure_scene_command(&remove).is_err());
        assert!(!is_exit_flush_command(&remove_batch));
        assert!(state.ensure_scene_command(&remove_batch).is_err());
        assert!(state.advance(&id, DRAINING));
        for command in [&add, &update, &remove, &remove_batch] {
            assert!(state.ensure_scene_command(command).is_err());
        }
        assert!(state.advance(&id, READY));
        assert!(state.ensure_scene_command(&update).is_err());
    }

    #[test]
    fn image_drawing_exit_flush_is_action_bounded_and_geometry_cannot_change_its_document() {
        use mewu_core::{Drawing, DrawingKind, DrawingPoint, ImageAnnotationEdit};
        let (mut store, scene_id) = item_geometry_store();
        let image = store.active_scene().unwrap().items[0].clone();
        let drawing = Drawing {
            id: uuid::Uuid::new_v4().to_string(),
            kind: DrawingKind::Line,
            color: "#ff0000".into(),
            stroke_width: 2.,
            points: vec![
                DrawingPoint { x: 10., y: 10. },
                DrawingPoint { x: 30., y: 30. },
            ],
            text: None,
            font_size: None,
            text_style: None,
            origin: None,
            rich: None,
         text_runs: None, };
        let command = |action, revision| SceneCommand::EditImageAnnotations {
            scene_id: scene_id.clone(),
            item_id: image.id.clone(),
            source_asset_id: image.asset.id.clone(),
            expected_revision: revision,
            action,
        };
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        for action in [
            ImageAnnotationEdit::UndoDrawing,
            ImageAnnotationEdit::RedoDrawing,
            ImageAnnotationEdit::RemoveDrawing {
                drawing_id: drawing.id.clone(),
            },
            ImageAnnotationEdit::RemoveDrawings {
                drawing_ids: vec![drawing.id.clone()],
            },
        ] {
            let replay = command(action, 0);
            assert!(!is_exit_flush_command(&replay));
            assert!(state
                .ensure_scene_command_with_store("space", &replay, &store)
                .is_err());
        }
        let add = command(
            ImageAnnotationEdit::AddDrawing {
                drawing: drawing.clone(),
            },
            0,
        );
        assert!(is_exit_flush_command(&add));
        state
            .ensure_scene_command_with_store("space", &add, &store)
            .unwrap();
        store.apply(add).unwrap();
        let saved = store.active_scene().unwrap().items[0].clone();
        let before = store.snapshot();
        assert!(store
            .apply(command(ImageAnnotationEdit::AddDrawing { drawing }, 0))
            .is_err());
        assert_eq!(store.snapshot(), before);
        let mut wrong = command(ImageAnnotationEdit::UndoDrawing, 1);
        if let SceneCommand::EditImageAnnotations {
            source_asset_id, ..
        } = &mut wrong
        {
            *source_asset_id = uuid::Uuid::new_v4().to_string();
        }
        assert!(store.apply(wrong).is_err());
        assert_eq!(store.snapshot(), before);
        let geometry = item_geometry_command(&scene_id, saved.clone());
        state
            .ensure_scene_command_with_store("space", &geometry, &store)
            .unwrap();
        for modify in [0, 1, 2] {
            let mut forged = saved.clone();
            let doc = forged.image_annotations.as_mut().unwrap();
            match modify {
                0 => doc.revision += 1,
                1 => doc.drawings.clear(),
                _ => doc.source_asset_id = uuid::Uuid::new_v4().to_string(),
            };
            assert!(state
                .ensure_scene_command_with_store(
                    "space",
                    &item_geometry_command(&scene_id, forged),
                    &store
                )
                .is_err());
        }
        assert!(state.advance(&id, DRAINING));
        assert!(state
            .ensure_scene_command_with_store("space", &geometry, &store)
            .is_err());
    }

    #[test]
    fn reserved_manual_history_finishes_before_preparation_and_never_afterwards() {
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        assert!(state.is_running() && state.has_attempt());
        assert!(!state.accepts_new_operations());
        let commands = [
            SceneCommand::UndoDrawing {
                scene_id: "s".into(),
                background_id: "b".into(),
                region_id: "r".into(),
                expected_revision: 2,
            },
            SceneCommand::RedoDrawing {
                scene_id: "s".into(),
                background_id: "b".into(),
                region_id: "r".into(),
                expected_revision: 3,
            },
            SceneCommand::EditImageAnnotations {
                scene_id: "s".into(),
                item_id: "i".into(),
                source_asset_id: "b".into(),
                expected_revision: 2,
                action: mewu_core::ImageAnnotationEdit::UndoDrawing,
            },
            SceneCommand::EditImageAnnotations {
                scene_id: "s".into(),
                item_id: "i".into(),
                source_asset_id: "b".into(),
                expected_revision: 3,
                action: mewu_core::ImageAnnotationEdit::RedoDrawing,
            },
        ];
        for command in &commands {
            assert!(!is_exit_flush_command(command));
            state.ensure_scene_command(command).unwrap();
        }
        assert!(state.ensure_scene_command(&SceneCommand::NewScene).is_err());
        assert!(state
            .ensure_scene_command(&SceneCommand::RemoveDrawing {
                scene_id: "s".into(),
                background_id: "b".into(),
                region_id: "r".into(),
                expected_revision: 2,
                drawing_id: "d".into()
            })
            .is_err());
        state.prepare_with("space", &id, || {}).unwrap();
        for command in &commands {
            assert!(state.ensure_scene_command(command).is_err());
        }
        assert!(state.advance(&id, DRAINING));
        for command in &commands {
            assert!(state.ensure_scene_command(command).is_err());
        }
        assert!(state.advance(&id, READY));
        for command in &commands {
            assert!(state.ensure_scene_command(command).is_err());
        }
    }

    #[test]
    fn preparation_allows_bounded_exit_flush_and_draining_disallows_even_flush() {
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        let draft = SceneCommand::SetDraft {
            scene_id: "s".into(),
            draft: "draft".into(),
        };
        let refs = SceneCommand::SetRefs {
            scene_id: "s".into(),
            refs: vec![],
        };
        let geometry = SceneCommand::SetRegionGeometry {
            scene_id: "s".into(),
            region_id: "r".into(),
            background_id: "b".into(),
            source_id: "b".into(),
            expected_revision: 0,
            edit_id: None,
            from: mewu_core::RegionGeometry {
                x: 0.,
                y: 0.,
                width: 30.,
                height: 40.,
            },
            to: mewu_core::RegionGeometry {
                x: 1.,
                y: 0.,
                width: 30.,
                height: 40.,
            },
        };
        let legacy = SceneCommand::UpdateRegion {
            scene_id: "s".into(),
            region: mewu_core::Region::default(),
        };
        let finish = SceneCommand::FinishRegionGeometryEdit {
            scene_id: "s".into(),
            edit_id: uuid::Uuid::new_v4().to_string(),
        };
        let from = mewu_core::RegionGeometry {
            x: 1.,
            y: 0.,
            width: 30.,
            height: 40.,
        };
        let undo = SceneCommand::UndoRegionGeometry {
            scene_id: "s".into(),
            expected_history_revision: 1,
            expected_operation_id: uuid::Uuid::new_v4().to_string(),
            region_id: "r".into(),
            background_id: "b".into(),
            source_id: "b".into(),
            expected_revision: 1,
            from: from.clone(),
        };
        let redo = SceneCommand::RedoRegionGeometry {
            scene_id: "s".into(),
            expected_history_revision: 1,
            expected_operation_id: uuid::Uuid::new_v4().to_string(),
            region_id: "r".into(),
            background_id: "b".into(),
            source_id: "b".into(),
            expected_revision: 1,
            from,
        };
        assert!(state.ensure_scene_command(&draft).is_ok());
        assert!(state.ensure_scene_command(&refs).is_ok());
        assert!(state.ensure_scene_command(&geometry).is_ok());
        assert!(state.ensure_scene_command(&finish).is_ok());
        assert!(!is_draft_command(&finish));
        assert!(is_geometry_finish_command(&finish));
        assert!(is_exit_flush_command(&finish));
        for replay in [&undo, &redo] {
            assert!(!is_exit_flush_command(replay));
            assert!(state.ensure_scene_command(replay).is_err());
        }
        assert!(!is_draft_command(&geometry));
        assert!(is_exit_flush_command(&geometry));
        assert!(!is_exit_flush_command(&legacy));
        assert!(state.ensure_scene_command(&legacy).is_err());
        assert!(state.ensure_scene_command(&SceneCommand::NewScene).is_err());
        assert!(!state.allows_command("send_message"));
        assert!(!state.allows_command("save_connection_profile"));
        assert!(!state.allows_command("run_plugin_ocr"));
        assert!(!state.allows_command("scan_plugin_codes"));
        assert!(!state.allows_command("act_on_code"));
        assert!(state.allows_command("cancel_plugin_code_scan"));
        assert!(!state.allows_command("set_capture_shortcut"));
        assert!(!state.allows_command("begin_capture_shortcut_edit"));
        assert!(state.allows_command("end_capture_shortcut_edit"));
        assert!(state.allows_command("get_capture_shortcut"));
        assert!(state.allows_command("finish_exit_preparation"));
        assert!(state.advance(&id, DRAINING));
        assert!(!state.allows_command("scan_plugin_codes"));
        assert!(!state.allows_command("act_on_code"));
        assert!(state.allows_command("cancel_plugin_code_scan"));
        assert!(!state.allows_command("set_capture_shortcut"));
        assert!(!state.allows_command("begin_capture_shortcut_edit"));
        assert!(state.allows_command("end_capture_shortcut_edit"));
        for command in [
            &draft,
            &refs,
            &geometry,
            &finish,
            &undo,
            &redo,
            &legacy,
            &SceneCommand::NewScene,
        ] {
            assert!(state.ensure_scene_command(command).is_err());
        }
        assert!(state.abort(&id));
        assert!(state.ensure_running().is_ok());
        assert!(state.ensure_scene_command(&geometry).is_ok());
        assert!(state.ensure_scene_command(&finish).is_ok());
        assert!(state.ensure_scene_command(&undo).is_ok());
        assert!(state.ensure_scene_command(&redo).is_ok());
    }

    #[test]
    fn ready_rejects_transient_finish_as_well_as_persistent_mutation() {
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        state.prepare_with("space", &id, || {}).unwrap();
        let finish = SceneCommand::FinishRegionGeometryEdit {
            scene_id: "already-closed-scene".into(),
            edit_id: uuid::Uuid::new_v4().to_string(),
        };
        assert!(state.ensure_scene_command(&finish).is_ok());
        assert!(state.advance(&id, DRAINING));
        assert!(state.ensure_scene_command(&finish).is_err());
        assert!(state.advance(&id, READY));
        assert!(state.ensure_scene_command(&finish).is_err());
        assert!(state.ensure_scene_command(&SceneCommand::NewScene).is_err());
    }

    #[test]
    fn reserve_keeps_authoring_running_but_rejects_new_commands_and_duplicate_attempts() {
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        assert!(state.is_running());
        assert!(state.has_attempt());
        assert!(state.ensure_running().is_ok());
        assert!(state.ensure_new_operation().is_err());
        assert!(state.begin().is_none());
        for command in [
            "apply_video_drawing",
            "get_video_annotation_vector",
            "get_video_annotation_text",
            "apply_scene_command",
            "begin_exit_preparation",
            "cancel_video_request",
        ] {
            assert!(state.allows_command(command), "{command}");
        }
        for command in [
            "capture_screen",
            "start_recording",
            "send_message",
            "save_connection_profile",
            "set_recording_audio",
            "migrate_data_directory",
        ] {
            assert!(!state.allows_command(command), "{command}");
        }
        assert!(state.ensure_scene_command(&SceneCommand::NewScene).is_err());
        assert!(state
            .ensure_scene_command(&SceneCommand::SetDraft {
                scene_id: "s".into(),
                draft: "pending".into()
            })
            .is_ok());
        assert!(!state.advance(&id, DRAINING));
        assert!(state.abort(&id));
        assert!(!state.has_attempt());
        assert!(state.ensure_new_operation().is_ok());
    }

    #[tokio::test]
    async fn reserve_cannot_skip_prepare_and_exact_prepare_cancels_only_once() {
        let state = ExitState::default();
        let (id, reply) = state.begin().unwrap();
        assert!(state.acknowledge("space", &id, true).is_err());
        assert!(state
            .prepare_with("settings", &id, || panic!("wrong owner"))
            .is_err());
        assert!(state
            .prepare_with("space", "stale", || panic!("stale nonce"))
            .is_err());
        let count = std::sync::atomic::AtomicUsize::new(0);
        state
            .prepare_with("space", &id, || {
                count.fetch_add(1, Ordering::AcqRel);
            })
            .unwrap();
        state
            .prepare_with("space", &id, || panic!("duplicate must not recancel flush"))
            .unwrap();
        assert_eq!(count.load(Ordering::Acquire), 1);
        assert!(!state.is_running());
        assert!(!state.allows_command("apply_video_drawing"));
        assert!(state.allows_command("edit_video_annotation_text"));
        assert!(state.allows_command("apply_video_annotation_document"));
        state.acknowledge("space", &id, true).unwrap();
        assert_eq!(reply.await.unwrap(), Ok(()));
        assert!(state.advance(&id, DRAINING));
        assert!(state
            .prepare_with("space", &id, || panic!("draining"))
            .is_err());
        assert!(state.advance(&id, READY));
        assert!(state.has_attempt());
        assert!(state.begin().is_none());
    }

    #[tokio::test]
    async fn predrain_failure_or_timeout_restores_and_old_nonce_cannot_reenter() {
        let state = ExitState::default();
        let (completion, mut finished) = oneshot::channel();
        let (id, reply) = state.begin_with_completion(Some(completion)).unwrap();
        state.acknowledge("space", &id, false).unwrap();
        assert!(reply.await.unwrap().is_err());
        assert!(state
            .prepare_with("space", &id, || panic!("failed predrain"))
            .is_err());
        assert!(state.abort_with_error(&id, "pre-drain failed", || {}));
        assert_eq!(finished.try_recv().unwrap(), Err("pre-drain failed".into()));
        let (next, _) = state.begin().unwrap();
        assert!(state
            .prepare_with("space", &id, || panic!("old timeout"))
            .is_err());
        assert!(!state.abort(&id));
        let restored = AtomicBool::new(false);
        assert!(state.abort_with_error(&next, "timeout", || {
            assert!(state.is_running());
            assert!(state.has_attempt()); // fresh admission stays closed until restoration ends
            restored.store(true, Ordering::Release);
        }));
        assert!(restored.load(Ordering::Acquire));
        assert!(state.ensure_new_operation().is_ok());
        assert!(state.begin().is_some());
    }

    #[test]
    fn prepare_revocation_finishes_before_timeout_can_restore_admission() {
        use std::sync::{mpsc, Arc};
        let state = Arc::new(ExitState::default());
        let (id, _) = state.begin().unwrap();
        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let (aborting, aborting_rx) = mpsc::channel();
        let (restored, restored_rx) = mpsc::channel();
        let first = state.clone();
        let first_id = id.clone();
        let prepare = std::thread::spawn(move || {
            first.prepare_with("space", &first_id, || {
                entered.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        entered_rx.recv().unwrap();
        assert!(state.attempt.try_lock().is_err());
        let second = state.clone();
        let abort = std::thread::spawn(move || {
            aborting.send(()).unwrap();
            second.abort_with_error(&id, "timeout", || {
                restored.send(()).unwrap();
            })
        });
        aborting_rx.recv().unwrap();
        assert!(matches!(
            restored_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        prepare.join().unwrap().unwrap();
        assert!(abort.join().unwrap());
        restored_rx.recv().unwrap();
        assert!(state.ensure_new_operation().is_ok());
    }

    #[tokio::test]
    async fn reserved_exit_rejects_capture_delay_without_releasing_real_work_or_transition() {
        let jobs = std::sync::Arc::new(crate::capture_delay_work::CaptureJobs::default());
        let capturing = std::sync::Arc::new(AtomicBool::new(false));
        let transition = crate::SpaceTransition::try_begin(&capturing).unwrap();
        let work = jobs.begin().unwrap().unwrap();
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        // Same callback used by capture_screen, not a canceled-future substitute.
        assert!(work
            .wait_delay(0, || state.ensure_new_operation())
            .await
            .is_err());
        assert!(work.check().is_ok()); // no fake completed worker or freed slot
        assert!(jobs.begin().unwrap().is_none());
        assert!(crate::SpaceTransition::try_begin(&capturing).is_none());
        assert!(state.ensure_running().is_ok()); // accepted manual drawing may drain
        drop(work);
        drop(transition);
        assert!(!capturing.load(Ordering::Acquire));
        assert!(state.abort(&id));
        let next = jobs.begin().unwrap().unwrap();
        next.wait_delay(0, || state.ensure_new_operation())
            .await
            .unwrap();
    }

    #[test]
    fn reserved_native_freeze_layout_failure_compensates_without_erasing_old_scene() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "synthetic retained draft".into(),
            })
            .unwrap();
        let before = store.snapshot();
        // The freeze transaction was accepted before reserve, but the actual
        // native main-thread layout callback is delayed until after reserve.
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: scene.clone(),
            })
            .unwrap();
        let state = ExitState::default();
        let (id, _) = state.begin().unwrap();
        assert!(state.ensure_new_operation().is_err());
        // Existing compensation is deliberately background-allowed, not fresh work.
        state.ensure_background().unwrap();
        store.compensate_failed_minimize(&scene, false).unwrap();
        let after = store.snapshot();
        assert_eq!(after.active_scene_id, scene);
        for original in &before.scenes {
            assert_eq!(
                after.scenes.iter().find(|s| s.id == original.id),
                Some(original)
            );
        }
        assert_eq!(after.scenes.len(), before.scenes.len() + 1);
        let leftover = after
            .scenes
            .iter()
            .find(|s| !before.scenes.iter().any(|old| old.id == s.id))
            .unwrap();
        assert!(
            leftover.frozen
                && !leftover.closed
                && leftover.messages.is_empty()
                && leftover.items.is_empty()
                && leftover.draft.is_empty()
        );
        assert!(!state.accepts_new_operations()); // no show/focus during predrain
        assert!(state.abort(&id));
        assert!(state.accepts_new_operations());
    }

    #[tokio::test]
    async fn renderer_not_ready_timeout_preserves_draft_and_rejects_delayed_prepare() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene,
                draft: "synthetic unsent draft".into(),
            })
            .unwrap();
        let before = store.snapshot();
        let state = ExitState::default();
        let (send, mut completed) = oneshot::channel();
        let (id, reply) = state.begin_with_completion(Some(send)).unwrap();
        // Same timeout->abort branch as shutdown; no renderer receipt is invented.
        assert!(tokio::time::timeout(Duration::from_millis(1), reply)
            .await
            .is_err());
        assert!(!state.advance(&id, DRAINING));
        assert!(state.abort_with_error(&id, "保存会话超时，已取消退出", || {}));
        assert!(completed.try_recv().unwrap().is_err());
        assert!(state
            .prepare_with("space", &id, || panic!(
                "late event must not cancel fresh work"
            ))
            .is_err());
        assert!(state.acknowledge("space", &id, true).is_err());
        assert_eq!(store.snapshot(), before);
        assert!(state.ensure_new_operation().is_ok());
    }
}
