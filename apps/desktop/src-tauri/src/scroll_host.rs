// SPDX-License-Identifier: MPL-2.0
//! Explicit user-scrolled capture. No synthetic wheel/input or automatic scrolling.
use crate::{
    capture,
    scroll_capture::FrameSource,
    scroll_stitch::{ScrollStitcher, StitchLimits, StitchState, StitchStatus},
    Host,
};
use image::ImageEncoder;
use mewu_core::{Asset, AssetKind, OcrTarget};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub const LABEL: &str = "scroll-controls";
#[derive(Clone, Serialize)]
pub struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    id: String,
    scene_id: String,
    phase: &'static str,
    rect: Rect,
    width: u32,
    height: u32,
    status: StitchStatus,
    stop_hotkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}
struct Active {
    status: Status,
    action: Arc<AtomicU8>,
    cancel: Arc<AtomicBool>,
    plugin_id: String,
    revision: u64,
}
#[derive(Default)]
pub struct ScrollState(Mutex<Option<Active>>);
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Finish,
    Keep,
    Cancel,
}
pub fn busy(app: &AppHandle) -> bool {
    app.try_state::<ScrollState>()
        .is_some_and(|state| state.0.lock().map_or(true, |value| value.is_some()))
}
pub fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    if busy(app) {
        Err("请先完成或取消长截图".into())
    } else {
        Ok(())
    }
}
fn emit(app: &AppHandle, status: &Option<Status>) {
    let _ = app.emit_to("space", "scroll-status", status);
    let _ = app.emit_to(LABEL, "scroll-status", status);
}
fn act(app: &AppHandle, id: Option<&str>, action: Action) -> Result<(), String> {
    let state = app.state::<ScrollState>();
    let mut lock = state.0.lock().map_err(|_| "长截图状态不可用")?;
    let current = lock.as_mut().ok_or("长截图已结束")?;
    if id.is_some_and(|id| current.status.id != id) {
        return Err("长截图已变更".into());
    }
    match action {
        Action::Cancel => {
            current.cancel.store(true, Ordering::Release);
            current.action.store(2, Ordering::Release);
            current.status.phase = "finishing";
        }
        Action::Finish => {
            let _ = current
                .action
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        }
        Action::Keep => {
            // All producers serialize through Active; the worker only resets
            // to zero. Never turn a pending cancellation into a save request.
            if !current.cancel.load(Ordering::Acquire) {
                current.action.store(3, Ordering::Release);
            }
        }
    }
    emit(app, &Some(current.status.clone()));
    Ok(())
}
pub fn cancel(app: &AppHandle) {
    let _ = act(app, None, Action::Cancel);
}
pub fn revoke_plugins(app: &AppHandle, snapshot: &crate::plugins::PluginSnapshot) {
    let Some(state) = app.try_state::<ScrollState>() else {
        return;
    };
    if let Ok(active) = state.0.lock() {
        if let Some(active) = active.as_ref() {
            if !snapshot.plugins.iter().any(|p| {
                p.manifest.id == active.plugin_id
                    && p.revision == active.revision
                    && p.state == crate::plugins::PluginState::Enabled
                    && p.error.is_none()
            }) {
                active.cancel.store(true, Ordering::Release);
                active.action.store(2, Ordering::Release);
            }
        }
    };
}
#[tauri::command]
pub fn get_scroll_status(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<Status>, String> {
    control_owner(window.label())?;
    Ok(app
        .state::<ScrollState>()
        .0
        .lock()
        .map_err(|_| "长截图状态不可用")?
        .as_ref()
        .map(|a| a.status.clone()))
}
fn control_owner(label: &str) -> Result<(), String> {
    if label == "space" || label == LABEL {
        Ok(())
    } else {
        Err("此窗口不能控制长截图".into())
    }
}
#[tauri::command]
pub fn control_scroll_capture(
    app: AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    action: Action,
) -> Result<(), String> {
    control_owner(window.label())?;
    act(&app, Some(&id), action)
}

#[derive(Clone, Debug, PartialEq)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    origin_x: i32,
    origin_y: i32,
    monitor_width: u32,
    monitor_height: u32,
    scale: f64,
}
fn geometry(background: &Asset, target: &OcrTarget) -> Result<Geometry, String> {
    let (mw, mh) = (
        background.width.ok_or("截图尺寸缺失")?,
        background.height.ok_or("截图尺寸缺失")?,
    );
    let (ox, oy) = (
        background.origin_x.ok_or("截图位置缺失，请重新截图")?,
        background.origin_y.ok_or("截图位置缺失，请重新截图")?,
    );
    if ![target.x, target.y, target.width, target.height]
        .iter()
        .all(|v| v.is_finite())
        || target.x < 0.
        || target.y < 0.
        || target.width < 16.
        || target.height < 64.
        || target.x + target.width > mw as f64
        || target.y + target.height > mh as f64
    {
        return Err("长截图选区无效，至少需要 16×64 像素".into());
    }
    let (w, h) = (target.width.round() as u32, target.height.round() as u32);
    if w > 8192 || h > 8192 {
        return Err("长截图选区单边不能超过 8192 像素".into());
    }
    if w as u64 * h as u64 > 4 * 1024 * 1024 {
        return Err("长截图选区超过 419 万像素，请缩小选区".into());
    }
    let x = i32::try_from(ox as i64 + target.x.floor() as i64).map_err(|_| "截图位置无效")?;
    let y = i32::try_from(oy as i64 + target.y.floor() as i64).map_err(|_| "截图位置无效")?;
    let scale = background.scale_factor.unwrap_or(1.);
    if !scale.is_finite() || !(0.5..=8.).contains(&scale) {
        return Err("截图缩放无效".into());
    }
    Ok(Geometry {
        x,
        y,
        width: w,
        height: h,
        origin_x: ox,
        origin_y: oy,
        monitor_width: mw,
        monitor_height: mh,
        scale,
    })
}
fn start_geometry(
    store: &mewu_core::Store,
    target: &OcrTarget,
    visible: bool,
) -> Result<Geometry, String> {
    if !visible {
        return Err("请先打开截图选区".into());
    }
    let state = store.snapshot();
    let scene = state
        .scenes
        .iter()
        .find(|scene| {
            scene.id == target.scene_id && scene.id == state.active_scene_id && !scene.closed
        })
        .ok_or("请先打开此会话")?;
    if scene
        .run
        .as_ref()
        .is_some_and(|run| run.status == mewu_core::RunStatus::Running)
    {
        return Err("请等待当前回答完成".into());
    }
    store
        .region_for_ocr(target)
        .map_err(|error| error.to_string())?;
    let region = scene
        .regions
        .iter()
        .find(|region| region.id == target.region_id)
        .ok_or("选区不存在")?;
    if region.image_override.is_some() {
        return Err("请重新框选屏幕区域".into());
    }
    geometry(scene.background.as_ref().ok_or("请先截图")?, target)
}

#[derive(Debug, PartialEq, Eq)]
enum NextFrame {
    Capture { finish_after: bool },
    Wait,
    Save,
    Cancel,
}
fn next_frame(request: u8, holding: bool) -> NextFrame {
    match request {
        2 => NextFrame::Cancel,
        3 => NextFrame::Save,
        _ if holding => NextFrame::Wait,
        _ => NextFrame::Capture {
            finish_after: request == 1,
        },
    }
}
fn complete_frame(status: StitchStatus) -> bool {
    matches!(
        status,
        StitchStatus::Initial
            | StitchStatus::Extended
            | StitchStatus::Retraced
            | StitchStatus::Unchanged
    )
}
fn authorized(
    app: &AppHandle,
    target: &OcrTarget,
    plugin_id: &str,
    revision: u64,
    contribution: &str,
) -> Result<(), String> {
    let host = app.state::<Host>();
    let runtime = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    runtime.store.scroll(plugin_id, revision, contribution)?;
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    engine
        .store
        .region_for_ocr(target)
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn start_scroll_capture(
    app: AppHandle,
    window: tauri::WebviewWindow,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: OcrTarget,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("请在截图选区启动长截图".into());
    }
    crate::recording_storage::supported()?;
    let host = app.state::<Host>();
    let _transition = crate::begin_space_transition(&app, &host)?;
    crate::pixel_sampler::invalidate(&app);
    crate::pin_host::cancel_pending(&app, "space");
    crate::recording::ensure_idle(&app)?;
    let geometry = {
        let runtime = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        runtime
            .store
            .scroll(&plugin_id, revision, &contribution_id)?;
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        start_geometry(
            &engine.store,
            &target,
            window.is_visible().map_err(|_| "截图窗口不可用")?,
        )?
    };
    let verify = geometry.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !capture::list_monitors()?.iter().any(|m| {
            m.origin_x == verify.origin_x
                && m.origin_y == verify.origin_y
                && m.display_width == verify.monitor_width
                && m.display_height == verify.monitor_height
                && m.scale_factor == verify.scale
        }) {
            return Err("显示器已改变，请重新截图".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|_| "显示器检查中断")??;
    let id = uuid::Uuid::new_v4().to_string();
    let action = Arc::new(AtomicU8::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        // Registration and plugin revocation follow plugins -> Engine -> active.
        let runtime = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        runtime
            .store
            .scroll(&plugin_id, revision, &contribution_id)?;
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        let current = start_geometry(
            &engine.store,
            &target,
            window.is_visible().map_err(|_| "截图窗口不可用")?,
        )?;
        if current != geometry {
            return Err("截图位置已改变，请重新框选".into());
        }
        engine.ocr_jobs.cancel_owner("space");
        engine
            .translation_jobs
            .cancel_scene("space", &target.scene_id);
        let state = app.state::<ScrollState>();
        let mut slot = state.0.lock().map_err(|_| "长截图状态不可用")?;
        if slot.is_some() {
            return Err("已有长截图正在进行".into());
        }
        let status = Status {
            id: id.clone(),
            scene_id: target.scene_id.clone(),
            phase: "starting",
            rect: Rect {
                x: target.x / geometry.monitor_width as f64,
                y: target.y / geometry.monitor_height as f64,
                width: target.width / geometry.monitor_width as f64,
                height: target.height / geometry.monitor_height as f64,
            },
            width: geometry.width,
            height: geometry.height,
            status: StitchStatus::Initial,
            stop_hotkey: String::new(),
            preview: None,
        };
        emit(&app, &Some(status.clone()));
        *slot = Some(Active {
            status,
            action: action.clone(),
            cancel: cancel.clone(),
            plugin_id: plugin_id.clone(),
            revision,
        });
    }
    if let Err(error) = prepare_windows(&app, &geometry).await {
        finish(&app, &id, Err(error.clone())).await;
        return Err(error);
    }
    let f8 = app
        .global_shortcut()
        .on_shortcut("F8", |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                let _ = act(app, None, Action::Finish);
            }
        })
        .is_ok();
    let esc = app
        .global_shortcut()
        .on_shortcut("Escape", |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                self::cancel(app);
            }
        })
        .is_ok();
    update(&app, &id, |s| {
        s.stop_hotkey = match (f8, esc) {
            (true, true) => "F8 / Esc",
            (true, false) => "F8",
            (false, true) => "Esc",
            _ => "",
        }
        .into();
    });
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let work_app = worker_app.clone();
        let work_id = id.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            authorized(&work_app, &target, &plugin_id, revision, &contribution_id)?;
            // GDI resources, their DPI context and all calls stay on this thread.
            let mut source =
                FrameSource::new(geometry.x, geometry.y, geometry.width, geometry.height)?;
            let mut stitch = ScrollStitcher::new(
                source.capture()?,
                StitchLimits {
                    max_output_height: 16384,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            update(&work_app, &work_id, |s| s.phase = "capturing");
            publish_preview(&work_app, &work_id, &stitch, &cancel)?;
            let mut preview_at = Instant::now();
            let mut preview_dirty = false;
            let started = Instant::now();
            let mut holding = false;
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                if !work_app.state::<Host>().exit.is_running() {
                    return Ok(());
                }
                if preview_dirty && preview_at.elapsed() >= Duration::from_millis(200) {
                    publish_preview(&work_app, &work_id, &stitch, &cancel)?;
                    preview_at = Instant::now();
                    preview_dirty = false;
                }
                if !holding && started.elapsed() > Duration::from_secs(600) {
                    holding = true;
                    update(&work_app, &work_id, |s| {
                        s.status = StitchStatus::LimitReached
                    });
                }
                let finish_after = match next_frame(action.swap(0, Ordering::AcqRel), holding) {
                    NextFrame::Cancel => return Ok(()),
                    NextFrame::Save => break,
                    NextFrame::Wait => {
                        std::thread::sleep(Duration::from_millis(80));
                        continue;
                    }
                    NextFrame::Capture { finish_after } => finish_after,
                };
                let frame = match source.capture() {
                    Ok(frame) => frame,
                    Err(error) => {
                        // Keep the accepted strips. A failed desktop read is not
                        // permission to silently publish an incomplete result.
                        holding = true;
                        update(&work_app, &work_id, |s| {
                            s.status = StitchStatus::LostOverlap
                        });
                        let _ = work_app.emit_to("space", "host-error", error);
                        continue;
                    }
                };
                let state = match stitch.accept_with_cancel(frame, &cancel) {
                    Ok(s) => s,
                    Err(crate::scroll_stitch::StitchError::Canceled) => return Ok(()),
                    Err(e) => return Err(e.to_string()),
                };
                progress(&work_app, &work_id, state);
                preview_dirty |= matches!(
                    state.status,
                    StitchStatus::Extended | StitchStatus::Retraced
                );
                authorized(&work_app, &target, &plugin_id, revision, &contribution_id)?;
                if finish_after && complete_frame(state.status) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            update(&work_app, &work_id, |s| s.phase = "finishing");
            let image = stitch.finish().map_err(|e| e.to_string())?;
            persist(
                &work_app,
                &work_id,
                &target,
                &plugin_id,
                revision,
                &contribution_id,
                &cancel,
                image,
            )
        })
        .await
        .map_err(|_| "长截图任务中断".to_string())
        .and_then(|value| value);
        if f8 {
            let _ = worker_app.global_shortcut().unregister("F8");
        }
        if esc {
            let _ = worker_app.global_shortcut().unregister("Escape");
        }
        finish(&worker_app, &id, result).await;
    });
    Ok(())
}
fn progress(app: &AppHandle, id: &str, state: StitchState) {
    update(app, id, |s| {
        s.width = state.width;
        s.height = state.height;
        s.status = state.status;
    });
}
fn publish_preview(
    app: &AppHandle,
    id: &str,
    stitch: &ScrollStitcher,
    cancel: &AtomicBool,
) -> Result<(), String> {
    use base64::Engine;
    let image = stitch
        .preview(220, 1536, cancel)
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法生成长截图预览")?;
    let value = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    update(app, id, |s| s.preview = Some(value));
    Ok(())
}
fn update(app: &AppHandle, id: &str, change: impl FnOnce(&mut Status)) {
    if let Ok(mut slot) = app.state::<ScrollState>().0.lock() {
        if let Some(active) = slot.as_mut().filter(|a| a.status.id == id) {
            let was_finishing = active.status.phase == "finishing";
            change(&mut active.status);
            if was_finishing {
                active.status.phase = "finishing";
            }
            emit(app, &Some(active.status.clone()));
        }
    }
}
fn persist(
    app: &AppHandle,
    capture_id: &str,
    target: &OcrTarget,
    plugin: &str,
    revision: u64,
    contribution: &str,
    cancel: &AtomicBool,
    image: image::RgbaImage,
) -> Result<(), String> {
    let host = app.state::<Host>();
    let id = uuid::Uuid::new_v4().to_string();
    let path = host.assets.join(format!("{id}.png"));
    let asset = Asset {
        id,
        name: "长截图.png".into(),
        kind: AssetKind::Image,
        path: path.to_string_lossy().into_owned(),
        width: Some(image.width()),
        height: Some(image.height()),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "无法保存长截图")?;
    let encoded = image::DynamicImage::ImageRgba8(image)
        .write_to(&mut file, image::ImageFormat::Png)
        .map_err(|_| "无法编码长截图")
        .and_then(|_| {
            file.flush()
                .and_then(|_| file.sync_all())
                .map_err(|_| "无法保存长截图")
        });
    drop(file);
    if let Err(error) = encoded {
        let _ = std::fs::remove_file(&path);
        return Err(error.into());
    }
    let mut registration_attempted = false;
    let mut registration = (|| {
        // Cancel, revoke and final registration serialize on Active. Maintain
        // plugins -> Engine -> Active order; never call back into these locks.
        let runtime = host
            .plugins
            .lock()
            .map_err(|_| "插件状态不可用".to_string())?;
        let mut engine = host.lock()?;
        let state = app.state::<ScrollState>();
        let slot = state.0.lock().map_err(|_| "长截图状态不可用".to_string())?;
        if cancel.load(Ordering::Acquire)
            || !host.exit.is_running()
            || !slot
                .as_ref()
                .is_some_and(|active| active.status.id == capture_id)
        {
            return Ok(false);
        }
        runtime.store.scroll(plugin, revision, contribution)?;
        registration_attempted = true;
        let snapshot = engine
            .store
            .set_region_image(target, asset)
            .map_err(|error| error.to_string())?;
        crate::publish(app, &snapshot);
        Ok(true)
    })();
    if !registration_attempted && (cancel.load(Ordering::Acquire) || !host.exit.is_running()) {
        registration = Ok(false);
    }
    settle_file(&path, registration)
}

/// The caller created this exact UUID file exclusively. Cancellation discards
/// it; failed registration preserves the completed image for manual recovery.
fn settle_file(path: &std::path::Path, registration: Result<bool, String>) -> Result<(), String> {
    match registration {
        Ok(true) => Ok(()),
        Ok(false) => std::fs::remove_file(path)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|_| "长截图已取消，但临时图片未能清理".into()),
        Err(_) => Err("长截图未能登记，图片已保留在素材目录".into()),
    }
}
async fn prepare_windows(app: &AppHandle, g: &Geometry) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = app.clone();
    let g = g.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            target.state::<Host>().exit.ensure_running()?;
            let main = target.get_webview_window("space").ok_or("截图窗口不存在")?;
            crate::recording::exclude(&main, true)?;
            main.set_ignore_cursor_events(true)
                .map_err(|_| "无法穿透截图覆盖层")?;
            main.set_focusable(false).map_err(|_| "无法释放桌面输入")?;
            let toolbar = tauri::WebviewWindowBuilder::new(
                &target,
                LABEL,
                tauri::WebviewUrl::App("index.html?surface=scroll".into()),
            )
            .title("Mewu · 长截图")
            .inner_size(preview_size(&g).0, preview_size(&g).1)
            .decorations(false)
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .shadow(false)
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
            })
            .build()
            .map_err(|_| "无法打开长截图控制条")?;
            crate::recording::exclude(&toolbar, true)?;
            toolbar
                .set_position(tauri::PhysicalPosition::new(
                    preview_placement(&g).0,
                    preview_placement(&g).1,
                ))
                .map_err(|_| "无法放置长截图控制条")?;
            toolbar.show().map_err(|_| "无法显示长截图控制条")?;
            Ok(())
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "长截图窗口准备失败")?;
    receive.await.map_err(|_| "长截图窗口准备中断")?
}
fn preview_size(g: &Geometry) -> (f64, f64) {
    let available_width = (g.monitor_width as f64 / g.scale - 12.).max(1.);
    let available_height = (g.monitor_height as f64 / g.scale - 12.).max(1.);
    (
        available_width.min(260.),
        (g.monitor_height as f64 / g.scale * 0.68 + 76.)
            .min(640.)
            .min(available_height),
    )
}
fn preview_placement(g: &Geometry) -> (i32, i32) {
    let (width, height) = preview_size(g);
    let (width, height) = (width * g.scale, height * g.scale);
    let margin = 6. * g.scale;
    let right = g.x as f64 + g.width as f64 + 10. * g.scale;
    let left = if right + width <= g.origin_x as f64 + g.monitor_width as f64 - margin {
        right
    } else {
        g.x as f64 - width - 10. * g.scale
    };
    (
        left.max(g.origin_x as f64 + margin)
            .min(g.origin_x as f64 + g.monitor_width as f64 - width - margin)
            .round() as i32,
        (g.y as f64 + g.height as f64 - height)
            .max(g.origin_y as f64 + margin)
            .min(g.origin_y as f64 + g.monitor_height as f64 - height - margin)
            .round() as i32,
    )
}
async fn finish(app: &AppHandle, id: &str, result: Result<(), String>) {
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = app.clone();
    if app
        .run_on_main_thread(move || {
            if let Some(toolbar) = target.get_webview_window(LABEL) {
                let _ = toolbar.destroy();
            }
            if let Some(main) = target.get_webview_window("space") {
                let _ = main.set_ignore_cursor_events(false);
                let _ = main.set_focusable(true);
                let _ = crate::capture_visibility::apply(&main);
            }
            let _ = send.send(());
        })
        .is_ok()
    {
        let _ = receive.await;
    }
    if let Ok(mut state) = app.state::<ScrollState>().0.lock() {
        if state.as_ref().is_some_and(|a| a.status.id == id) {
            *state = None;
            emit(app, &None);
        }
    }
    if app.state::<Host>().exit.is_running() {
        crate::show(app);
    }
    if let Err(error) = result {
        let _ = app.emit_to("space", "host-error", error);
    }
}
pub async fn cancel_for_shutdown(app: &AppHandle) -> Result<(), String> {
    cancel(app);
    let started = Instant::now();
    while busy(app) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("停止长截图超时，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_preview_uses_original_negative_monitor_coordinates_and_fits_small_scaled_monitors() {
        let mut g = Geometry {
            x: -2400,
            y: -100,
            width: 640,
            height: 480,
            origin_x: -2560,
            origin_y: -200,
            monitor_width: 2560,
            monitor_height: 1400,
            scale: 1.5,
        };
        assert_eq!(preview_placement(&g), (-1745, -191));
        g.x = -200;
        g.width = 180;
        assert_eq!(preview_placement(&g).0, -605);
        g = Geometry {
            x: -300,
            y: 0,
            width: 200,
            height: 200,
            origin_x: -320,
            origin_y: 0,
            monitor_width: 320,
            monitor_height: 360,
            scale: 1.5,
        };
        let (x, y) = preview_placement(&g);
        let (w, h) = preview_size(&g);
        assert!(x >= g.origin_x + 9 && x as f64 + w * g.scale <= -9.);
        assert!(y >= 9 && y as f64 + h * g.scale <= 351.);
    }

    #[test]
    fn finish_never_implies_keep_on_uncertain_last_frame_or_a_stopped_source() {
        for status in [
            StitchStatus::LostOverlap,
            StitchStatus::Ambiguous,
            StitchStatus::LowInformation,
            StitchStatus::LimitReached,
        ] {
            assert!(!complete_frame(status));
        }
        for status in [
            StitchStatus::Extended,
            StitchStatus::Retraced,
            StitchStatus::Unchanged,
        ] {
            assert!(complete_frame(status));
        }
        assert_eq!(
            next_frame(1, false),
            NextFrame::Capture { finish_after: true }
        );
        assert_eq!(
            next_frame(0, false),
            NextFrame::Capture {
                finish_after: false
            }
        );
        assert_eq!(next_frame(1, true), NextFrame::Wait);
        assert_eq!(next_frame(0, true), NextFrame::Wait);
        assert_eq!(next_frame(3, true), NextFrame::Save);
        assert_eq!(next_frame(3, false), NextFrame::Save);
        assert_eq!(next_frame(2, true), NextFrame::Cancel);
        assert_eq!(
            serde_json::from_str::<Action>("\"keep\"")
                .map(|a| matches!(a, Action::Keep))
                .unwrap(),
            true
        );
    }

    #[test]
    fn registration_rechecks_active_visibility_run_and_original_source() {
        use mewu_core::{Region, SceneCommand, Store};
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let background = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "screen.png".into(),
            kind: AssetKind::Image,
            path: "fixture.png".into(),
            width: Some(1920),
            height: Some(1080),
            origin_x: Some(-1920),
            origin_y: Some(0),
            scale_factor: Some(1.),
        };
        store.set_background(&scene_id, background.clone()).unwrap();
        let target = OcrTarget {
            scene_id: scene_id.clone(),
            region_id: uuid::Uuid::new_v4().to_string(),
            background_id: background.id.clone(),
            drawing_revision: 0,
            x: 100.,
            y: 100.,
            width: 640.,
            height: 480.,
        };
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene_id.clone(),
                region: Region {
                    id: target.region_id.clone(),
                    x: target.x,
                    y: target.y,
                    width: target.width,
                    height: target.height,
                    ..Default::default()
                },
            })
            .unwrap();
        let expected = start_geometry(&store, &target, true).unwrap();
        assert!(start_geometry(&store, &target, false).is_err());
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "fixture".into(),
            })
            .unwrap();
        store.begin_run(&scene_id).unwrap();
        assert!(start_geometry(&store, &target, true).is_err());
        store.cancel_run(&scene_id).unwrap();
        assert_eq!(start_geometry(&store, &target, true).unwrap(), expected);
        store.apply(SceneCommand::NewScene).unwrap();
        assert!(start_geometry(&store, &target, true).is_err());
        store
            .apply(SceneCommand::ActivateScene {
                scene_id: scene_id.clone(),
            })
            .unwrap();
        let replacement = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            origin_x: None,
            origin_y: None,
            ..background
        };
        store.set_region_image(&target, replacement).unwrap();
        let state = store.snapshot();
        let region = &state
            .scenes
            .iter()
            .find(|scene| scene.id == scene_id)
            .unwrap()
            .regions[0];
        let current = OcrTarget {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
            drawing_revision: region.drawing_revision,
            ..target
        };
        assert!(store.region_for_ocr(&current).is_ok());
        assert!(start_geometry(&store, &current, true).is_err());
    }

    #[test]
    fn completed_file_is_discarded_only_for_cancel_and_retained_on_database_failure() {
        let folder =
            std::env::temp_dir().join(format!("mewu-scroll-file-{}", uuid::Uuid::new_v4()));
        assert!(!folder
            .to_string_lossy()
            .to_lowercase()
            .starts_with("c:\\hermes"));
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join(format!("{}.png", uuid::Uuid::new_v4()));
        let other = folder.join("unrelated.png");
        std::fs::write(&other, b"unrelated").unwrap();
        std::fs::write(&file, b"completed synthetic image").unwrap();
        assert!(settle_file(&file, Err("injected transaction failure".into())).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"completed synthetic image");
        settle_file(&file, Ok(true)).unwrap();
        assert!(file.is_file());
        settle_file(&file, Ok(false)).unwrap();
        assert!(!file.exists());
        settle_file(&file, Ok(false)).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"unrelated");
        std::fs::remove_file(other).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
    #[test]
    fn physical_region_geometry_keeps_negative_monitor_origin_and_rejects_invalid_bounds() {
        let background = Asset {
            id: "b".into(),
            name: "screen".into(),
            kind: AssetKind::Image,
            path: "test".into(),
            width: Some(1920),
            height: Some(1080),
            origin_x: Some(-1920),
            origin_y: Some(-400),
            scale_factor: Some(1.5),
        };
        let mut target = OcrTarget {
            scene_id: "s".into(),
            region_id: "r".into(),
            background_id: "b".into(),
            drawing_revision: 0,
            x: 100.25,
            y: 90.75,
            width: 600.25,
            height: 700.25,
        };
        let rect = geometry(&background, &target).unwrap();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (-1820, -310, 600, 700)
        );
        for (x, width) in [(-1., 100.), (f64::NAN, 100.), (0., 15.), (0., 3000.)] {
            target.x = x;
            target.width = width;
            assert!(geometry(&background, &target).is_err());
        }
        assert!(control_owner("space").is_ok());
        assert!(control_owner(LABEL).is_ok());
        assert!(control_owner("settings").is_err());
    }
}
