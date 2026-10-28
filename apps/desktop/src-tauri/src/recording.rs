// SPDX-License-Identifier: MPL-2.0
//! One transient recorder. Driver and encoder calls live in a supervised child,
//! with kill-on-parent-exit ownership and bounded startup/finalization deadlines.
use crate::{assets, capture, recording_backend as backend, Host};
use mewu_core::{Asset, AssetKind, Region};
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const LABEL: &str = "recording-controls";
const CONTROLS_WIDTH: f64 = 176.;
const CONTROLS_HEIGHT: f64 = 44.;
#[cfg(windows)]
#[path = "recording_controls_windows.rs"]
mod controls_windows;
#[cfg(target_os = "macos")]
fn mac_hud_position(
    display: &mewu_core::CaptureCoordinates,
    work: (f64, f64, f64, f64),
    hud: (f64, f64),
) -> Result<tauri::LogicalPosition<f64>, String> {
    let (x, y, width, height) = work;
    if [x, y, width, height, hud.0, hud.1]
        .iter()
        .any(|v| !v.is_finite())
        || hud.0 <= 0.
        || hud.1 <= 0.
        || width < hud.0
        || height < hud.1
    {
        return Err("录屏控制条显示范围无效".into());
    }
    Ok(tauri::LogicalPosition::new(
        (display.origin_x + (display.width - hud.0) / 2.).clamp(x, x + width - hud.0),
        (display.origin_y + 10.)
            .max(y + 10.)
            .min(y + height - hud.1),
    ))
}
#[derive(Clone, Serialize)]
pub struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}
#[cfg(windows)]
#[derive(Clone, Serialize)]
pub(crate) struct Hud {
    pub(crate) rect: Option<Rect>,
    pub(crate) stage: &'static str,
    #[serde(skip)]
    ready: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub(crate) id: String,
    pub(crate) scene_id: String,
    pub(crate) phase: &'static str,
    pub(crate) countdown: u8,
    pub(crate) elapsed_ms: u64,
    pub(crate) rect: Rect,
    pub(crate) stop_hotkey: String,
    pub(crate) audio: backend::AudioMode,
    #[cfg(windows)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hud: Option<Hud>,
    #[cfg(windows)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retain_main: Option<bool>,
}
struct Active {
    #[cfg(windows)]
    capture_session: Option<crate::shared_capture_windows::Session>,
    status: Status,
    commands: Arc<crate::recording_worker::ControlBus>,
    grant: crate::plugins::RecordingGrant,
    audio_grant: Option<crate::recording_audio_host::RecordingAudioGrant>,
    plugin_revoked: bool,
    suppress_reveal: bool,
}
impl Active {
    fn revoke_plugins(&mut self, plugins: &crate::plugins::PluginSnapshot) {
        if !crate::plugins::recording_grant_is_current(&self.grant, plugins)
            || self
                .audio_grant
                .as_ref()
                .is_some_and(|grant| !crate::recording_audio_host::grant_is_current(grant, plugins))
        {
            self.plugin_revoked = true;
            let _ = self.commands.send(backend::Control::Stop);
        }
    }
}
#[derive(Default)]
pub struct RecordingState(
    Mutex<Option<Active>>,
    Mutex<Option<(String, Result<(), String>)>>,
);
impl RecordingState {
    fn begin_exit<T>(
        &self,
        begin: impl FnOnce() -> Option<T>,
    ) -> Result<Option<(T, Option<String>)>, String> {
        let mut active = self.0.lock().map_err(|_| "录屏状态不可用，已取消退出")?;
        // finish cannot publish a completion and clear Active between the exit
        // phase transition and binding its target. No Engine lock is involved.
        let Some(attempt) = begin() else {
            return Ok(None);
        };
        let target = active.as_mut().map(|active| {
            active.suppress_reveal = true;
            active.status.id.clone()
        });
        Ok(Some((attempt, target)))
    }
    fn completed(&self, id: &str) -> Result<Option<Result<(), String>>, String> {
        Ok(self
            .1
            .lock()
            .map_err(|_| "录屏状态不可用，已取消退出")?
            .as_ref()
            .filter(|(completed, _)| completed == id)
            .map(|(_, result)| result.clone()))
    }
}
pub fn busy(app: &AppHandle) -> bool {
    #[cfg(windows)]
    if crate::shared_capture_windows::pending() { return true; }
    (crate::recording_worker::busy() && {
        #[cfg(target_os="linux")] { !crate::recording_wayland_host::selecting() }
        #[cfg(not(target_os="linux"))] { true }
    })
        || app
            .try_state::<RecordingState>()
            .is_some_and(|s| s.0.lock().map(|v| v.is_some()).unwrap_or(true))
}
/// Unlike admission's busy(), this excludes pure presentation cleanup debt.
/// It includes the actual child/reaper permit even without an Active target.
pub(crate) fn workers_busy(app: &AppHandle) -> bool {
    crate::recording_worker::busy() || app.try_state::<RecordingState>()
        .is_some_and(|state| state.0.lock().map_or(true, |slot| slot.is_some()))
}
pub fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    crate::scroll_host::ensure_idle(app)?;
    if busy(app) {
        Err("请先停止录屏".into())
    } else {
        Ok(())
    }
}
#[tauri::command]
pub fn get_recording_status(app: AppHandle) -> Result<Option<Status>, String> {
    #[cfg(target_os="linux")]
    if let Some(status)=crate::recording_wayland_host::status()?{return Ok(Some(status));}
    Ok(app
        .state::<RecordingState>()
        .0
        .lock()
        .map_err(|_| "录屏状态不可用")?
        .as_ref()
        .map(|a| a.status.clone()))
}

/// A terminal receipt is not inferred from the absence of an active recorder.
/// Only the most recent completed receipt survives, and only until process exit.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum RecordingResult {
    Active { id: String, recording: Status },
    Finished { id: String },
    Failed { id: String, error: RecordingFailure },
    Unknown { id: String },
}
#[derive(Serialize)]
pub(crate) struct RecordingFailure { code: &'static str, message: String }

fn completed_result(id: &str, receipt: Option<Result<(), String>>) -> RecordingResult {
    match receipt {
        Some(Ok(())) => RecordingResult::Finished { id: id.into() },
        Some(Err(message)) => RecordingResult::Failed { id: id.into(), error: RecordingFailure {
            code: "RECORDING_FAILED", message: message.chars().take(256).collect(),
        } },
        None => RecordingResult::Unknown { id: id.into() },
    }
}

pub(crate) fn get_recording_result(app: AppHandle, id: &str) -> Result<RecordingResult, String> {
    if let Some(recording) = get_recording_status(app.clone())?.filter(|s| s.id == id) {
        return Ok(RecordingResult::Active { id: id.into(), recording });
    }
    #[cfg(target_os = "linux")]
    if let Some(receipt) = crate::recording_wayland_host::completed_result(id)? {
        return Ok(completed_result(id, Some(receipt)));
    }
    Ok(completed_result(id, app.state::<RecordingState>().completed(id)?))
}
fn emit(app: &AppHandle, status: &Option<Status>) {
    let _ = app.emit_to("space", "recording-status", status);
    let _ = app.emit_to(LABEL, "recording-status", status);
}
fn update(app: &AppHandle, id: &str, change: impl FnOnce(&mut Status)) {
    if let Ok(mut state) = app.state::<RecordingState>().0.lock() {
        if let Some(active) = state.as_mut().filter(|a| a.status.id == id) {
            change(&mut active.status);
            emit(app, &Some(active.status.clone()));
        }
    }
}
#[tauri::command]
pub fn control_recording(app: AppHandle, id: String, action: String) -> Result<(), String> {
    if !matches!(action.as_str(), "stop" | "cancel") {
        app.state::<Host>().exit.ensure_new_operation()?;
    }
    #[cfg(target_os="linux")]
    if crate::recording_wayland_host::control(&id,&action)?{return Ok(());}
    let control = match action.as_str() {
        "pause" => backend::Control::Pause,
        "resume" => backend::Control::Resume,
        "stop" => backend::Control::Stop,
        "cancel" => backend::Control::Cancel,
        _ => return Err("无效录屏操作".into()),
    };
    let state = app.state::<RecordingState>();
    let state = state.0.lock().map_err(|_| "录屏状态不可用")?;
    // Exit reserves under this same mutex; a queued pause/resume must recheck
    // after acquiring it, while stop/cancel remain usable during shutdown.
    if !matches!(action.as_str(), "stop" | "cancel") {
        app.state::<Host>().exit.ensure_new_operation()?;
    }
    let active = state
        .as_ref()
        .filter(|a| a.status.id == id)
        .ok_or("录屏已结束")?;
    active
        .commands
        .send(control)
        .map_err(|_| "录屏正在结束".into())
}
pub fn request_stop(app: &AppHandle) {
    #[cfg(target_os="linux")]
    crate::recording_wayland_host::request_stop();
    if let Some(state) = app.try_state::<RecordingState>() {
        if let Ok(state) = state.0.lock() {
            if let Some(active) = state.as_ref() {
                let _ = active.commands.send(backend::Control::Stop);
            }
        }
    }
}
pub fn revoke_plugins(app: &AppHandle, plugins: &crate::plugins::PluginSnapshot) {
    #[cfg(target_os="linux")]
    crate::recording_wayland_host::revoke_plugins(plugins);
    if let Some(state) = app.try_state::<RecordingState>() {
        if let Ok(mut state) = state.0.lock() {
            if let Some(active) = state.as_mut() {
                active.revoke_plugins(plugins);
            }
        }
    }
}
pub fn begin_shutdown<T>(
    app: &AppHandle,
    begin: impl FnOnce() -> Option<T>,
) -> Result<Option<(T, Option<String>)>, String> {
    #[cfg(target_os="linux")]
    if crate::recording_wayland_host::has_lease(){return crate::recording_wayland_host::begin_exit(begin);}
    app.try_state::<RecordingState>()
        .ok_or("录屏状态尚未就绪")?
        .begin_exit(begin)
}
pub async fn stop_for_shutdown(app: &AppHandle, target: Option<String>) -> Result<(), String> {
    let Some(id) = target else {
        let deadline = Instant::now() + Duration::from_secs(45);
        while workers_busy(app) {
            request_stop(app);
            if Instant::now() >= deadline { return Err("录屏进程尚未结束，已取消退出".into()); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        return Ok(());
    };
    #[cfg(target_os="linux")]
    if crate::recording_wayland_host::stop_for_exit(&id).await?{return Ok(());}
    {
        let state = app.state::<RecordingState>();
        let active = state.0.lock().map_err(|_| "录屏状态不可用，已取消退出")?;
        if let Some(active) = active.as_ref().filter(|active| active.status.id == id) {
            let _ = active.commands.send(backend::Control::Stop);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let completed = app.state::<RecordingState>().completed(&id)?;
        if let Some(result) = completed {
            // A saved/completed marker cannot overtake an owned reaper or an
            // active operation. Keep all original recording/DB errors fatal.
            result.map_err(|_| "录屏未能保存，已取消退出".to_string())?;
            if workers_busy(app) {
                if Instant::now() >= deadline { return Err("录屏进程尚未结束，已取消退出".into()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("录屏尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn config_for(
    background: &Asset,
    region: &Region,
    output: std::path::PathBuf,
) -> Result<backend::Config, String> {
    let (mw, mh) = (
        background.width.ok_or("截图尺寸缺失")?,
        background.height.ok_or("截图尺寸缺失")?,
    );
    if ![region.x, region.y, region.width, region.height]
        .iter()
        .all(|v| v.is_finite())
        || region.x < 0.
        || region.y < 0.
        || region.width < 2.
        || region.height < 2.
        || region.x + region.width > mw as f64
        || region.y + region.height > mh as f64
    {
        return Err("请重新选择录制区域".into());
    }
    let width = (region.width.floor() as u32) & !1;
    let height = (region.height.floor() as u32) & !1;
    if u64::from(width) * u64::from(height) > 4_000_000 {
        return Err("录制区域不能超过 400 万像素".into());
    }
    Ok(backend::Config {
        output,
        origin_x: background.origin_x.ok_or("请重新截图后录制")?,
        origin_y: background.origin_y.ok_or("请重新截图后录制")?,
        monitor_width: mw,
        monitor_height: mh,
        x: region.x.floor() as u32,
        y: region.y.floor() as u32,
        width,
        height,
        audio: backend::AudioMode::Mute,
        #[cfg(target_os = "macos")]
        capture_coordinates: background
            .capture_coordinates
            .clone()
            .filter(|c| {
                c.space == mewu_core::CaptureCoordinateSpace::QuartzPoints && c.valid_for(mw, mh)
            })
            .ok_or("截图的 Mac 屏幕坐标缺失，请重新截图后录制")?,
    })
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    region_id: String,
    audio: crate::recording_audio_host::RecordingAudioSelection,
    grant: crate::plugins::RecordingGrant,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能开始录屏".into());
    }
    app.state::<Host>().exit.ensure_new_operation()?;
    #[cfg(windows)]
    crate::shared_capture_windows::recover(&app).await?;
    #[cfg(target_os="linux")]
    if crate::recording_wayland_host::start(app.clone(),window.clone(),scene_id.clone(),region_id.clone(),audio.clone(),grant.clone()).await?{return Ok(());}
    #[cfg(windows)]
    let window_handle = window.hwnd().map_err(|_| "空间窗口不可用")?.0 as usize;
    #[cfg(target_os = "macos")]
    let window_handle = crate::window_witness_macos::window_handle(&window)?;
    crate::recording_storage::supported()?;
    let host = app.state::<Host>();
    let _transition = crate::begin_space_transition(&app, &host)?;
    crate::pixel_sampler::invalidate(&app);
    crate::pin_host::cancel_pending(&app, "space");
    ensure_idle(&app)?;
    if !crate::speech_worker::wait_until_idle(Duration::from_secs(5)).await {
        return Err("请等待语音输入停止".into());
    }
    let scene = {
        let snapshot = host.lock()?.store.snapshot();
        if snapshot.active_scene_id != scene_id {
            return Err("请先打开要录制的会话".into());
        }
        snapshot
            .scenes
            .into_iter()
            .find(|s| s.id == scene_id)
            .ok_or("会话不存在")?
    };
    if scene.closed || scene.frozen {
        return Err("请先恢复要录制的会话".into());
    }
    if scene
        .run
        .as_ref()
        .is_some_and(|r| r.status == mewu_core::RunStatus::Running)
    {
        return Err("请等待当前回答完成".into());
    }
    let background = scene.background.ok_or("请先截图并选择区域")?;
    let region = scene
        .regions
        .into_iter()
        .find(|r| r.id == region_id)
        .ok_or("请选择录制区域")?;
    if region.image_override.is_some() {
        return Err("请重新框选屏幕录制区域".into());
    }
    let asset_id = uuid::Uuid::new_v4().to_string();
    let mut config = config_for(
        &background,
        &region,
        host.root
            .join("recording-staging")
            .join(format!("{asset_id}.mp4")),
    )?;
    let monitors = tauri::async_runtime::spawn_blocking(capture::list_monitors)
        .await
        .map_err(|_| "无法检查录制屏幕")??;
    let monitor = monitors
        .into_iter()
        .find(|m| {
            #[cfg(target_os = "macos")]
            {
                let bounds = objc2_core_graphics::CGDisplayBounds(m.id);
                return bounds.origin.x == config.capture_coordinates.origin_x
                    && bounds.origin.y == config.capture_coordinates.origin_y
                    && bounds.size.width == config.capture_coordinates.width
                    && bounds.size.height == config.capture_coordinates.height;
            }
            #[cfg(not(target_os = "macos"))]
            {
                m.origin_x == config.origin_x
                    && m.origin_y == config.origin_y
                    && m.display_width == config.monitor_width
                    && m.display_height == config.monitor_height
            }
        })
        .ok_or("显示器已改变，请重新截图")?;
    let controls = Arc::new(crate::recording_worker::ControlBus::default());
    let id = uuid::Uuid::new_v4().to_string();
    let status = Status {
        id: id.clone(),
        scene_id: scene_id.clone(),
        phase: if cfg!(windows) { "starting" } else { "countdown" },
        countdown: if cfg!(windows) { 0 } else { 3 },
        elapsed_ms: 0,
        rect: Rect {
            x: config.x as f64 / config.monitor_width as f64,
            y: config.y as f64 / config.monitor_height as f64,
            width: config.width as f64 / config.monitor_width as f64,
            height: config.height as f64 / config.monitor_height as f64,
        },
        stop_hotkey: String::new(),
        audio: audio.mode,
        #[cfg(windows)]
        hud: None,
        #[cfg(windows)]
        retain_main: Some(false),
    };
    {
        let plugins = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        let plugin_snapshot = plugins.store.snapshot()?;
        validate_recording_grants(&grant, &audio, &plugin_snapshot)?;
        let engine = host.lock()?;
        let latest = engine.store.snapshot();
        let current = latest
            .scenes
            .iter()
            .find(|scene| scene.id == scene_id)
            .ok_or("会话不存在")?;
        if latest.active_scene_id != scene_id
            || current.closed
            || current.frozen
            || current.background.as_ref() != Some(&background)
            || current.regions.iter().find(|r| r.id == region_id) != Some(&region)
            || current
                .run
                .as_ref()
                .is_some_and(|r| r.status == mewu_core::RunStatus::Running)
        {
            return Err("录制区域已改变，请重试".into());
        }
        let recording_state = app.state::<RecordingState>();
        let mut active = recording_state.0.lock().map_err(|_| "录屏状态不可用")?;
        host.exit.ensure_new_operation()?;
        #[cfg(target_os = "macos")]
        if !crate::window_witness_macos::current_for(window_handle, "space") {
            return Err("空间已收起".into());
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
            if !unsafe {
                IsWindow(window_handle as _) != 0 && IsWindowVisible(window_handle as _) != 0
            } {
                return Err("空间已收起".into());
            }
        }
        if active.is_some() || crate::recording_worker::busy() {
            return Err("请先停止录屏".into());
        }
        config.audio =
            crate::recording_audio_host::validate_selection(&app, &audio, &plugin_snapshot)?;
        engine.ocr_jobs.cancel_owner("space");
        engine.translation_jobs.cancel_scene("space", &scene_id);
        *active = Some(Active {
            #[cfg(windows)]
            capture_session: None,
            status: status.clone(),
            commands: controls.clone(),
            grant,
            audio_grant: audio.grant.clone(),
            plugin_revoked: false,
            suppress_reveal: false,
        });
    }
    #[cfg(all(target_os = "macos", debug_assertions))]
    crate::recording_worker::mac_diagnostic(
        crate::recording_worker::MacStage::HostAdmitted,
        crate::recording_worker::MacOutcome::Ok,
    );
    emit(&app, &Some(status));
    let f8 = app
        .global_shortcut()
        .on_shortcut("F8", |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                request_stop(app);
            }
        })
        .is_ok();
    #[cfg(target_os = "macos")]
    let esc = app
        .global_shortcut()
        .on_shortcut("Escape", |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                request_stop(app);
            }
        })
        .is_ok();
    update(&app, &id, |s| {
        #[cfg(not(target_os = "macos"))]
        {
            s.stop_hotkey = if f8 { "F8 / Esc" } else { "Esc" }.into();
        }
        #[cfg(target_os = "macos")]
        {
            s.stop_hotkey = match (f8, esc) {
                (true, true) => "F8 / Esc",
                (true, false) => "F8",
                (false, true) => "Esc",
                (false, false) => "停止按钮",
            }
            .into();
        }
    });
    let prepared = prepare_windows(&app, &id, &config, monitor.scale_factor, f8).await;
    if !matches!(prepared, Ok(true)) {
        #[cfg(all(target_os = "macos", debug_assertions))]
        crate::recording_worker::mac_diagnostic(
            crate::recording_worker::MacStage::OverlayPrepared,
            crate::recording_worker::MacOutcome::Failed,
        );
        if f8 { let _ = app.global_shortcut().unregister("F8"); }
        #[cfg(target_os = "macos")]
        if esc { let _ = app.global_shortcut().unregister("Escape"); }
        // A confirmed Stop/Cancel during window preparation has no unfinished
        // media. Keep it successful for normal exit, and never start the worker.
        // Actual construction/native errors stay failures, even if Stop follows.
        let result = prepared.map(|_| ());
        finish(&app, &id, result.clone()).await;
        return result;
    }
    #[cfg(all(target_os = "macos", debug_assertions))]
    crate::recording_worker::mac_diagnostic(
        crate::recording_worker::MacStage::OverlayPrepared,
        crate::recording_worker::MacOutcome::Ok,
    );
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let run_app = app2.clone();
        let run_id = id.clone();
        let path = config.output.clone();
        let finalized = Arc::new(AtomicBool::new(false));
        let received_finalized = finalized.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            supervise(&run_app, &run_id, config, controls, received_finalized)
        })
        .await
        .map_err(|_| "录屏任务中断".to_string())
        .and_then(|v| v);
        // An error may have handed cleanup to the process reaper. Keep the
        // recording active and its files untouched until the actual tree and
        // pipe threads have exited; shutdown may time out and retain the app.
        while crate::recording_worker::busy() {
            update(&app2, &id, |status| status.phase = "stopping");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if result.is_err() && finalized.load(Ordering::Acquire) {
            // A valid Finalize receipt remains recoverable even if process/IO
            // teardown subsequently failed. Never treat it as crash debris.
            let _ = std::fs::rename(&path, path.with_extension("completed.mp4"));
        } else if result.is_err() || matches!(&result, Ok(None)) {
            let _ = std::fs::remove_file(&path);
        }
        let had_output = matches!(&result, Ok(Some(_)));
        let warning = result
            .as_ref()
            .ok()
            .and_then(|r| r.as_ref())
            .and_then(|r| r.stop_reason);
        let result = result.and_then(|report| {
            let Some(report) = report else {
                return Ok(());
            };
            let host = app2.state::<Host>();
            let final_path = host.assets.join(format!("{asset_id}.mp4"));
            let asset = Asset {
                capture_coordinates: None,
                id: asset_id,
                name: "录屏.mp4".into(),
                kind: AssetKind::Video,
                path: final_path.to_string_lossy().into_owned(),
                width: Some(report.width),
                height: Some(report.height),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            };
            let snapshot = retain_completed_output(&path, &final_path, || {
                assets::open_video(&asset, &host.assets)?;
                host.lock()?
                    .store
                    .finish_recording(&scene_id, &background.id, &region, asset)
                    .map_err(|e| e.to_string())
            })?;
            crate::publish(&app2, &snapshot);
            Ok(())
        });
        if f8 {
            let _ = app2.global_shortcut().unregister("F8");
        }
        #[cfg(target_os = "macos")]
        if esc {
            let _ = app2.global_shortcut().unregister("Escape");
        }
        let saved = had_output && result.is_ok();
        let revoked = app2
            .state::<RecordingState>()
            .0
            .lock()
            .ok()
            .and_then(|active| {
                active
                    .as_ref()
                    .filter(|active| active.status.id == id)
                    .map(|active| active.plugin_revoked)
            })
            .unwrap_or(false);
        finish(&app2, &id, result).await;
        if saved && (warning.is_some() || revoked) {
            let message = if revoked {
                "屏幕录制已停用，已保存录下的片段"
            } else {
                "声音中断，已保存录下的片段"
            };
            let _ = app2.emit_to("space", "host-error", message);
        }
    });
    Ok(())
}

pub(crate) fn validate_recording_grants(
    grant: &crate::plugins::RecordingGrant,
    audio: &crate::recording_audio_host::RecordingAudioSelection,
    plugins: &crate::plugins::PluginSnapshot,
) -> Result<(), String> {
    if !crate::plugins::recording_grant_is_current(grant, plugins) {
        return Err("屏幕录制已停用或改变".into());
    }
    if audio.mode != backend::AudioMode::Mute
        && !audio.grant.as_ref().is_some_and(|audio_grant| {
            audio_grant.plugin_id == grant.plugin_id
                && audio_grant.revision == grant.revision
                && crate::recording_audio_host::grant_is_current(audio_grant, plugins)
        })
    {
        return Err("请重新选择录屏声源".into());
    }
    Ok(())
}

pub(crate) fn retain_completed_output<T>(
    staging: &std::path::Path,
    destination: &std::path::Path,
    register: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    // Startup removes UUID.mp4 partials only. Mark a finalized output before
    // trying its destination so a failed move is not mistaken for crash debris.
    let completed = staging.with_extension("completed.mp4");
    std::fs::rename(staging, &completed).map_err(|_| "无法整理已完成的录屏文件")?;
    std::fs::rename(&completed, destination).map_err(|_| "无法保存录屏素材，成品仍在录制目录")?;
    // A database failure leaves the exact finalized UUID file in assets. No
    // automatic re-import is attempted without its original scene transaction.
    register()
}

#[cfg(windows)]
fn preparation_result(result: Result<(), controls_windows::PrepareFailure>) -> Result<bool, String> {
    match result {
        Ok(()) => Ok(true),
        Err(controls_windows::PrepareFailure::Canceled) => Ok(false),
        Err(controls_windows::PrepareFailure::Failed(error)) => Err(error),
    }
}

#[cfg(windows)]
async fn prepare_windows(
    app: &AppHandle,
    id: &str,
    config: &backend::Config,
    scale: f64,
    f8: bool,
) -> Result<bool, String> {
    preparation_result(controls_windows::prepare(app, id, config, scale, f8).await)
}

#[cfg(not(windows))]
async fn prepare_windows(
    app: &AppHandle,
    id: &str,
    config: &backend::Config,
    scale: f64,
    f8: bool,
) -> Result<bool, String> {
    #[cfg(not(windows))]
    let _ = (id, f8);
    #[cfg(windows)]
    let id = id.to_owned();
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = app.clone();
    let config = config.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            target.state::<Host>().exit.ensure_new_operation()?;
            let main = target.get_webview_window("space").ok_or("窗口不存在")?;
            #[cfg(windows)]
            let shared = target.state::<Host>().system_preferences.snapshot_for_operation()
                .map_err(|error| error.to_string())?.allow_screen_share;
            #[cfg(windows)]
            let roi = crate::shared_capture_windows::Rect::offset(config.origin_x, config.origin_y,
                config.x, config.y, config.width, config.height)?;
            #[cfg(windows)]
            {
                let session = crate::shared_capture_windows::Session::capture(&main, shared)?;
                {
                    let state = target.state::<RecordingState>();
                    let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
                    let active = slot.as_mut().filter(|active| active.status.id == id).ok_or("录屏已结束")?;
                    if active.capture_session.is_some() { return Err("录屏窗口已准备".into()); }
                    active.capture_session = Some(session);
                }
                edit_capture_session(&target.state::<RecordingState>(), &id, |session| session.main.prepare(roi, false))?;
            }
            #[cfg(not(windows))]
            {
                exclude(&main, true)?;
                main.set_ignore_cursor_events(true).map_err(|_| "无法穿透录屏覆盖层")?;
                main.set_focusable(false).map_err(|_| "无法释放桌面输入")?;
            }
            #[cfg(windows)]
            let shadow = !shared;
            #[cfg(not(windows))]
            let shadow = true;
            let builder = tauri::WebviewWindowBuilder::new(
                &target,
                LABEL,
                tauri::WebviewUrl::App("index.html?surface=recording".into()),
            )
            .title("Mewu · 录屏")
            .inner_size(CONTROLS_WIDTH, CONTROLS_HEIGHT)
            .decorations(false)
            // GetWindowDisplayAffinity requires a layered window on Windows.
            // CSS/background paint remain opaque; other platforms keep their HUD.
            .transparent(cfg!(windows))
            .background_color(tauri::window::Color(252, 253, 255, 255))
            .shadow(shadow)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .disable_drag_drop_handler()
            .on_navigation(|url| {
                (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                    || (matches!(url.scheme(), "http" | "https")
                        && url.host_str() == Some("tauri.localhost"))
                    || (cfg!(debug_assertions)
                        && url.scheme() == "http"
                        && url.host_str() == Some("127.0.0.1")
                        && url.port() == Some(1420))
            });
            #[cfg(target_os = "macos")]
            let builder = builder.accept_first_mouse(true);
            let toolbar = builder.build()
            .map_err(|_| "无法打开录屏控制条")?;
            #[cfg(windows)]
            {
                edit_capture_session(&target.state::<RecordingState>(), &id, |session| session.attach_hud(&toolbar, false))?;
                if !shared { exclude(&toolbar, true)?; }
            }
            #[cfg(not(windows))]
            exclude(&toolbar, true)?;
            #[cfg(target_os = "macos")]
            {
                // Quartz origins and display sizes are points, not screenshot
                // pixels. Tauri converts a PhysicalPosition using the window's
                // current screen scale, which may still be the primary screen.
                let display = &config.capture_coordinates;
                let monitor = toolbar
                    .available_monitors()
                    .map_err(|_| "无法定位录屏显示器")?
                    .into_iter()
                    .find(|monitor| {
                        let factor = monitor.scale_factor();
                        let position = monitor.position().to_logical::<f64>(factor);
                        let size = monitor.size().to_logical::<f64>(factor);
                        (position.x - display.origin_x).abs() < 1.
                            && (position.y - display.origin_y).abs() < 1.
                            && (size.width - display.width).abs() < 1.
                            && (size.height - display.height).abs() < 1.
                    })
                    .ok_or("显示器已改变，请重新截图")?;
                let factor = monitor.scale_factor();
                let work = monitor.work_area();
                let origin = work.position.to_logical::<f64>(factor);
                let size = work.size.to_logical::<f64>(factor);
                let hud = toolbar
                    .inner_size()
                    .map_err(|_| "无法读取录屏控制条大小")?
                    .to_logical::<f64>(
                        toolbar
                            .scale_factor()
                            .map_err(|_| "无法读取录屏控制条缩放")?,
                    );
                toolbar
                    .set_position(mac_hud_position(
                        display,
                        (origin.x, origin.y, size.width, size.height),
                        (hud.width, hud.height),
                    )?)
                    .map_err(|_| "无法放置录屏控制条")?;
                let _ = scale;
            }
            #[cfg(not(target_os = "macos"))]
            {
                let x = config.origin_x as f64
                    + (config.monitor_width as f64 - CONTROLS_WIDTH * scale) / 2.;
                let y = config.origin_y as f64 + 10. * scale;
                toolbar
                    .set_position(tauri::PhysicalPosition::new(x as i32, y as i32))
                    .map_err(|_| "无法放置录屏控制条")?;
            }
            #[cfg(windows)]
            let show_hud = if shared {
                let display = crate::shared_capture_windows::Rect::new(config.origin_x, config.origin_y,
                    config.monitor_width, config.monitor_height)?;
                let work = crate::shared_capture_windows::work_area(&toolbar, display)?;
                crate::shared_capture_windows::place_hud(&toolbar, roi, work, f8)?
            } else { true };
            #[cfg(not(windows))]
            let show_hud = true;
            if show_hud { toolbar.show().map_err(|_| "无法显示录屏控制条")?; }
            Ok(())
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "无法准备录屏窗口")?;
    receive.await.map_err(|_| "录屏窗口准备中断")?.map(|_| true)
}
async fn finish(app: &AppHandle, id: &str, result: Result<(), String>) {
    #[cfg(all(target_os = "macos", debug_assertions))]
    crate::recording_worker::mac_diagnostic(
        crate::recording_worker::MacStage::HostFinished,
        if result.is_ok() {
            crate::recording_worker::MacOutcome::Ok
        } else {
            crate::recording_worker::MacOutcome::Failed
        },
    );
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = app.clone();
    #[cfg(windows)]
    let restore_id = id.to_owned();
    let dispatched = app.run_on_main_thread(move || {
        #[cfg(windows)]
        let restored = (|| {
            let session = {
                let state = target.state::<RecordingState>();
                let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
                slot.as_mut().filter(|active| active.status.id == restore_id)
                    .and_then(|active| active.capture_session.take())
            };
            session.map_or(Ok(()), crate::shared_capture_windows::finish)
        })();
        #[cfg(not(windows))]
        let restored: Result<(), String> = (|| {
            if let Some(toolbar) = target.get_webview_window(LABEL) { let _ = toolbar.destroy(); }
            if let Some(main) = target.get_webview_window("space") {
                let _ = main.set_ignore_cursor_events(false);
                let _ = main.set_focusable(true);
                crate::capture_visibility::apply(&main)?;
            }
            Ok(())
        })();
        let _ = send.send(restored);
    });
    let restored = match dispatched {
        Ok(()) => receive.await.unwrap_or_else(|_| Err("录屏窗口恢复中断".into())),
        Err(_) => Err("无法恢复录屏窗口".into()),
    };
    #[cfg(windows)]
    let restored = {
        // A rejected/discarded dispatch can leave the lease inside Active.
        // Take that exact operation's remainder before clearing Active, and
        // only transfer it to debt here; restoration belongs to the HWND thread.
        let residual = (|| -> Result<_, String> {
            let state = app.state::<RecordingState>();
            let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
            Ok(slot.as_mut().filter(|active| active.status.id == id)
                .and_then(|active| active.capture_session.take()))
        })();
        let deferred = residual.and_then(|session| session.map_or(Ok(()), crate::shared_capture_windows::defer));
        restored.and(deferred)
    };
    let mut reveal = false;
    if let Ok(mut state) = app.state::<RecordingState>().0.lock() {
        if state.as_ref().is_some_and(|a| a.status.id == id) {
            reveal = state.as_ref().is_some_and(|active| !active.suppress_reveal);
            if let Ok(mut completed) = app.state::<RecordingState>().1.lock() {
                *completed = Some((id.to_owned(), result.clone()));
            }
            *state = None;
            emit(app, &None);
        }
    }
    if reveal && restored.is_ok() {
        crate::show(app);
    }
    // The original worker result remains available for shutdown. Recovery debt
    // blocks new work; after READY normal OS exit may discard display leases.
    if let Err(error) = restored.and(result) {
        let _ = app.emit_to("space", "host-error", error);
    }
}
#[cfg(windows)]
pub(crate) fn exclude(window: &tauri::WebviewWindow, exclude: bool) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|_| "录屏窗口不可用")?.0;
    if unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity(
            hwnd,
            if exclude { 0x11 } else { 0 },
        )
    } == 0
    {
        return Err("此系统无法排除录屏覆盖层".into());
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub(crate) fn exclude(_: &tauri::WebviewWindow, _: bool) -> Result<(), String> {
    // SCK has no WDA flag. The owned recorder must resolve and exclude its
    // actual parent application before starting; this covers new HUD windows.
    // Failure to find that parent stops capture rather than capturing overlays.
    Ok(())
}
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn exclude(_: &tauri::WebviewWindow, _: bool) -> Result<(), String> {
    Err("此平台暂不支持录屏".into())
}

fn escape_down() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x1b) < 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
#[cfg(windows)]
fn edit_capture_session(state: &RecordingState, id: &str,
    edit: impl FnOnce(&mut crate::shared_capture_windows::Session) -> Result<(), String>) -> Result<(), String> {
    let mut session = {
        let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
        slot.as_mut().filter(|active| active.status.id == id)
            .and_then(|active| active.capture_session.take()).ok_or("录屏已结束")?
    };
    // Native mutations can synchronously reenter window callbacks. Active's
    // mutex must be free until the exact same operation receives its lease back.
    let result = edit(&mut session);
    let mut remaining = Some(session);
    let stored = match state.0.lock() {
        Ok(mut slot) => match slot.as_mut().filter(|active| active.status.id == id) {
            Some(active) if active.capture_session.is_none() => {
                active.capture_session = remaining.take(); true
            }
            _ => false,
        },
        Err(_) => false,
    };
    if !stored {
        if let Some(session) = remaining { crate::shared_capture_windows::defer(session)?; }
        return Err("录屏已改变，窗口恢复待完成".into());
    }
    result
}
#[cfg(windows)]
fn verify_capture(app: &AppHandle, id: &str) -> Result<(), String> {
    let target = app.clone();
    let id = id.to_owned();
    crate::shared_capture_windows::check(app, move || {
        let state = target.state::<RecordingState>();
        let active = state.0.lock().map_err(|_| "录屏状态不可用")?;
        active.as_ref().filter(|active| active.status.id == id)
            .and_then(|active| active.capture_session.as_ref()).ok_or("录屏已结束")?.verify()
    })
}
/// A presentation ACK is admitted only by the current owned recording HWND.
/// macOS/Linux retain their existing recording surfaces and do not use this gate.
pub async fn surface_ready(
    app: &AppHandle,
    window: tauri::WebviewWindow,
    id: &str,
    stage: &str,
) -> Result<bool, String> {
    #[cfg(windows)]
    { controls_windows::surface_ready(app, window, id, stage).await }
    #[cfg(not(windows))]
    { let _ = (app, window, id, stage); Ok(false) }
}
fn recording_countdown(
    app: &AppHandle, id: &str, commands: &crate::recording_worker::ControlBus,
    escape: &std::cell::Cell<bool>,
) -> Result<bool, String> {
    #[cfg(windows)]
    update(app, id, |s| { s.phase = "countdown"; s.countdown = 3; });
    for number in (1..=3).rev() {
        update(app, id, |s| s.countdown = number);
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            if commands.take()?.is_some_and(|c| matches!(c, backend::Control::Stop | backend::Control::Cancel)) {
                return Ok(false);
            }
            let down = escape_down();
            if down && !escape.replace(down) { return Ok(false); }
            escape.set(down);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    update(app, id, |s| {
        #[cfg(windows)]
        if let Some(hud) = &mut s.hud {
            s.phase = "starting"; s.countdown = 0;
            hud.stage = "live";
        }
        // Compact capture retains the last numeral until the real native
        // aperture and retainMain proof are published together. An unproven
        // intermediate `starting` would briefly reveal the screenshot toolbar.
        #[cfg(not(windows))]
        { s.phase = "starting"; s.countdown = 0; }
    });
    #[cfg(all(target_os = "macos", debug_assertions))]
    crate::recording_worker::mac_diagnostic(
        crate::recording_worker::MacStage::CountdownComplete,
        crate::recording_worker::MacOutcome::Ok,
    );
    Ok(true)
}

fn supervise(
    app: &AppHandle,
    id: &str,
    config: backend::Config,
    commands: Arc<crate::recording_worker::ControlBus>,
    finalized: Arc<AtomicBool>,
) -> Result<Option<backend::Report>, String> {
    let escape = std::cell::Cell::new(escape_down());
    #[cfg(windows)]
    let capture_live = std::cell::Cell::new(false);
    #[cfg(windows)]
    let last_verified = std::cell::Cell::new(Instant::now());
    let poll = || {
        #[cfg(windows)]
        if capture_live.get() && last_verified.get().elapsed() >= Duration::from_millis(100) {
            verify_capture(app, id)?;
            last_verified.set(Instant::now());
        }
        let down = escape_down();
        let esc = down && !escape.replace(down);
        escape.set(down);
        if esc { Ok(Some(backend::Control::Stop)) } else { commands.take() }
    };
    let progress = |progress: backend::Progress| {
        update(app, id, |status| {
            status.phase = if progress.paused { "paused" } else { "recording" };
            status.elapsed_ms = progress.elapsed_ms;
        })
    };
    let stopping = || update(app, id, |status| status.phase = "stopping");
    let finalized = || finalized.store(true, Ordering::Release);
    #[cfg(windows)]
    {
        let prepare_config = config.clone();
        crate::recording_worker::run_prepared(config, poll, || {
            // Release the frozen capture and its input ownership BEFORE the
            // countdown. The prepared encoder still cannot record until the
            // countdown completes and the compositor epoch is captured below.
            if !controls_windows::prepare_live_desktop(app, id, &prepare_config)? { return Ok(None); }
            if !recording_countdown(app, id, &commands, &escape)? { return Ok(None); }
            if commands.terminal_pending()? { return Ok(None); }
            verify_capture(app, id)?;
            // main.prepare has flushed native composition. The child must use
            // a compositor timestamp at/after this point, not its warm-up frame.
            let epoch = crate::recording_audio::qpc_now()?;
            last_verified.set(Instant::now());
            capture_live.set(true);
            Ok(Some(epoch))
        }, progress, stopping, finalized)
    }
    #[cfg(not(windows))]
    {
        if !recording_countdown(app, id, &commands, &escape)? { return Ok(None); }
        crate::recording_worker::run(config, poll, progress, stopping, finalized)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn recording_cli_result_matches_only_the_exact_terminal_id_and_never_infers_success() {
        let state = super::RecordingState::default();
        let encode = |id: &str| serde_json::to_value(super::completed_result(id, state.completed(id).unwrap())).unwrap();
        assert_eq!(encode("first")["state"], "unknown");
        *state.1.lock().unwrap() = Some(("first".into(), Err("原生编码未完成".into())));
        let failed = encode("first");
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error"]["code"], "RECORDING_FAILED");
        assert_eq!(failed["error"]["message"], "原生编码未完成");
        assert_eq!(encode("other")["state"], "unknown");
        *state.1.lock().unwrap() = Some(("next".into(), Ok(())));
        assert_eq!(encode("first")["state"], "unknown");
        assert_eq!(encode("next")["state"], "finished");
        *state.1.lock().unwrap() = Some(("next".into(), Err("错🙂".repeat(300))));
        assert_eq!(encode("next")["error"]["message"].as_str().unwrap().chars().count(), 256);
    }
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_hud_uses_quartz_points_real_size_and_visible_work_area() {
        let display = mewu_core::CaptureCoordinates {
            space: mewu_core::CaptureCoordinateSpace::QuartzPoints,
            origin_x: -1920.,
            origin_y: -200.,
            width: 1920.,
            height: 1080.,
        };
        let point = mac_hud_position(&display, (-1920., -176., 1836., 1056.), (220., 44.)).unwrap();
        assert_eq!((point.x, point.y), (-1070., -166.));
        // No screenshot pixel width or primary-screen Retina multiplier enters
        // these coordinates; a wider actual HUD stays centered on this screen.
        assert!(mac_hud_position(&display, (-1920., -176., 100., 20.), (220., 44.)).is_err());
    }
    fn recording_plugins() -> crate::plugins::PluginSnapshot {
        crate::plugins::PluginSnapshot {
            revision: 1,
            plugins: vec![crate::plugins::PluginRecord {
                manifest: crate::plugins::parse_manifest(include_bytes!(
                    "../../plugins/migrations/official-recording-1.0.0.json"
                ))
                .unwrap(),
                revision: 1,
                state: crate::plugins::PluginState::Enabled,
                source: crate::plugins::PluginSource::Official,
                has_rollback: false,
                error: None,
            }],
        }
    }
    #[test]
    fn silent_recording_requires_current_bundle_and_audio_cannot_cross_packages() {
        let grant = crate::plugins::RecordingGrant {
            plugin_id: "mewu.recording".into(),
            revision: 1,
            contribution_id: "record".into(),
        };
        let mut snapshot = recording_plugins();
        let mut audio = crate::recording_audio_host::RecordingAudioSelection {
            revision: 0,
            mode: backend::AudioMode::Mute,
            grant: None,
        };
        assert!(validate_recording_grants(&grant, &audio, &snapshot).is_ok());
        audio.mode = backend::AudioMode::Both;
        audio.grant = Some(crate::recording_audio_host::RecordingAudioGrant {
            plugin_id: grant.plugin_id.clone(),
            revision: 1,
            contribution_id: "audio".into(),
        });
        assert!(validate_recording_grants(&grant, &audio, &snapshot).is_ok());
        audio.grant.as_mut().unwrap().plugin_id = "mewu.recording-audio".into();
        assert!(validate_recording_grants(&grant, &audio, &snapshot).is_err());
        audio.mode = backend::AudioMode::Mute;
        audio.grant = None;
        snapshot.plugins[0].state = crate::plugins::PluginState::Disabled;
        assert!(validate_recording_grants(&grant, &audio, &snapshot).is_err());
        snapshot.plugins[0].state = crate::plugins::PluginState::Enabled;
        snapshot.plugins[0].revision += 1;
        assert!(validate_recording_grants(&grant, &audio, &snapshot).is_err());
    }
    #[test]
    fn core_recording_and_audio_are_exact_and_do_not_revoke_when_legacy_packages_change() {
        let grant = crate::plugins::RecordingGrant {
            plugin_id: crate::plugins::CORE_RECORDING.into(),
            revision: 1,
            contribution_id: "record".into(),
        };
        let audio_grant = crate::recording_audio_host::RecordingAudioGrant {
            plugin_id: crate::plugins::CORE_RECORDING.into(),
            revision: 1,
            contribution_id: "audio".into(),
        };
        let audio = crate::recording_audio_host::RecordingAudioSelection {
            revision: 0,
            mode: backend::AudioMode::System,
            grant: Some(audio_grant.clone()),
        };
        let empty = crate::plugins::PluginSnapshot {
            revision: 0,
            plugins: vec![],
        };
        let mut legacy = recording_plugins();
        legacy.plugins[0].state = crate::plugins::PluginState::Disabled;
        legacy.plugins[0].revision = 2;
        for plugins in [&empty, &legacy] {
            assert!(validate_recording_grants(&grant, &audio, plugins).is_ok());
        }
        for revision in [0, 2, u64::MAX] {
            assert!(validate_recording_grants(
                &crate::plugins::RecordingGrant {
                    revision,
                    ..grant.clone()
                },
                &audio,
                &empty
            )
            .is_err());
        }
        let crossed = crate::recording_audio_host::RecordingAudioSelection {
            grant: Some(crate::recording_audio_host::RecordingAudioGrant {
                contribution_id: "gif".into(),
                ..audio_grant
            }),
            ..audio
        };
        assert!(validate_recording_grants(&grant, &crossed, &empty).is_err());
    }
    #[test]
    fn disabling_recording_stops_an_active_silent_capture() {
        let commands = Arc::new(crate::recording_worker::ControlBus::default());
        let mut active = Active {
            #[cfg(windows)]
            capture_session: None,
            status: Status {
                id: "synthetic".into(),
                scene_id: "scene".into(),
                phase: "recording",
                countdown: 0,
                elapsed_ms: 0,
                rect: Rect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                stop_hotkey: String::new(),
                audio: backend::AudioMode::Mute,
                #[cfg(windows)]
                hud: None,
                #[cfg(windows)]
                retain_main: None,
            },
            commands: commands.clone(),
            grant: crate::plugins::RecordingGrant {
                plugin_id: "mewu.recording".into(),
                revision: 1,
                contribution_id: "record".into(),
            },
            audio_grant: None,
            plugin_revoked: false,
            suppress_reveal: false,
        };
        let mut plugins = recording_plugins();
        active.revoke_plugins(&plugins);
        assert!(!active.plugin_revoked);
        assert!(commands.take().unwrap().is_none());
        plugins.plugins[0].state = crate::plugins::PluginState::Disabled;
        active.revoke_plugins(&plugins);
        assert!(active.plugin_revoked);
        assert!(matches!(
            commands.take().unwrap(),
            Some(backend::Control::Stop)
        ));
    }
    #[test]
    fn exit_transition_and_recording_identity_are_bound_under_the_same_active_lock() {
        let state = RecordingState::default();
        let send = Arc::new(crate::recording_worker::ControlBus::default());
        *state.0.lock().unwrap() = Some(Active {
            #[cfg(windows)]
            capture_session: None,
            status: Status {
                id: "current".into(),
                scene_id: "scene".into(),
                phase: "recording",
                countdown: 0,
                elapsed_ms: 0,
                rect: Rect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                stop_hotkey: String::new(),
                audio: backend::AudioMode::Mute,
                #[cfg(windows)]
                hud: None,
                #[cfg(windows)]
                retain_main: None,
            },
            commands: send,
            grant: crate::plugins::RecordingGrant {
                plugin_id: "mewu.recording".into(),
                revision: 1,
                contribution_id: "record".into(),
            },
            audio_grant: None,
            plugin_revoked: false,
            suppress_reveal: false,
        });
        let (attempt, target) = state
            .begin_exit(|| {
                assert!(matches!(
                    state.0.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                Some("nonce")
            })
            .unwrap()
            .unwrap();
        assert_eq!(attempt, "nonce");
        assert_eq!(target.as_deref(), Some("current"));
        assert!(state.0.lock().unwrap().as_ref().unwrap().suppress_reveal);
        *state.1.lock().unwrap() = Some(("current".into(), Err("fixture late failure".into())));
        *state.0.lock().unwrap() = None;
        assert!(state
            .completed(target.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .is_err());
    }
    #[test]
    fn exit_observes_its_recording_failure_after_active_slot_is_cleared() {
        let state = RecordingState::default();
        *state.1.lock().unwrap() = Some((
            "exit-recording".into(),
            Err("synthetic save failure".into()),
        ));
        assert!(state.0.lock().unwrap().is_none());
        assert!(state.completed("exit-recording").unwrap().unwrap().is_err());
        assert!(state.completed("another-recording").unwrap().is_none());
    }
    #[test]
    fn finalized_output_survives_destination_failure_and_startup_cleanup() {
        let root = std::env::temp_dir().join(format!("mewu-completed-{}", uuid::Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        let staging = crate::recording_storage::prepare(&root).unwrap();
        let source = staging.join(format!("{}.mp4", uuid::Uuid::new_v4()));
        std::fs::write(&source, b"synthetic-completed-video").unwrap();
        let completed = source.with_extension("completed.mp4");
        let destination = root.join("missing").join("video.mp4");
        let result = retain_completed_output(&source, &destination, || -> Result<(), String> {
            panic!("must not register missing destination")
        });
        assert!(result.is_err());
        crate::recording_storage::prepare(&root).unwrap();
        assert_eq!(
            std::fs::read(&completed).unwrap(),
            b"synthetic-completed-video"
        );
        std::fs::remove_file(completed).unwrap();
        std::fs::remove_dir(staging).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn finalized_output_survives_database_registration_failure() {
        let root = std::env::temp_dir().join(format!("mewu-registration-{}", uuid::Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&root).unwrap();
        let source = root.join(format!("{}.mp4", uuid::Uuid::new_v4()));
        let destination = root.join("retained.mp4");
        std::fs::write(&source, b"synthetic-completed-video").unwrap();
        assert!(
            retain_completed_output(&source, &destination, || -> Result<(), String> {
                Err("fixture database failure".into())
            })
            .is_err()
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"synthetic-completed-video"
        );
        assert!(!source.exists());
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn recording_geometry_keeps_physical_origin_and_even_crop() {
        let bg = Asset {
            capture_coordinates: {
                #[cfg(target_os = "macos")]
                {
                    Some(mewu_core::CaptureCoordinates {
                        space: mewu_core::CaptureCoordinateSpace::QuartzPoints,
                        origin_x: -1920.,
                        origin_y: 0.,
                        width: 1920.,
                        height: 1080.,
                    })
                }
                #[cfg(not(target_os = "macos"))]
                {
                    None
                }
            },
            id: "bg".into(),
            name: "screen".into(),
            kind: AssetKind::Image,
            path: "none".into(),
            width: Some(1920),
            height: Some(1080),
            origin_x: Some(-1920),
            origin_y: Some(0),
            scale_factor: Some(1.5),
        };
        let region = Region {
            id: "region".into(),
            x: 100.5,
            y: 90.4,
            width: 321.5,
            height: 221.5,
            ..Default::default()
        };
        let c = config_for(&bg, &region, std::env::temp_dir().join("test.mp4")).unwrap();
        assert_eq!((c.origin_x, c.x, c.width, c.height), (-1920, 100, 320, 220));
        let mut invalid = region;
        invalid.x = 1900.;
        assert!(config_for(&bg, &invalid, c.output).is_err());
    }
}
