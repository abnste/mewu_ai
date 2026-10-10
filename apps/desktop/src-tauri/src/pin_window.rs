// SPDX-License-Identifier: MPL-2.0
//! Physical-pixel window geometry and Windows z-order. Never acquire Engine;
//! callers may already own it when requesting a deferred, non-activating restack.
use crate::{
    pin_host::{self, PinAction, PinRegistry},
    Host,
};
use mewu_core::{Asset, Region};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub const PADDING: u32 = 12;
const MAX_SIDE: f64 = 8192.;
const MAX_SURFACE_PIXELS: f64 = 16. * 1024. * 1024.;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

fn fit_dimensions(width: f64, height: f64, ratio: f64) -> Result<(u32, u32), String> {
    if !width.is_finite()
        || !height.is_finite()
        || !ratio.is_finite()
        || width <= 0.
        || height <= 0.
        || ratio <= 0.
    {
        return Err("贴图尺寸无效".into());
    }
    let mut w = width.min(height * ratio);
    let mut h = w / ratio;
    let mut shrink = ((MAX_SIDE - 24.) / w).min((MAX_SIDE - 24.) / h).min(1.);
    if (w * shrink + 24.) * (h * shrink + 24.) > MAX_SURFACE_PIXELS {
        let (mut low, mut high) = (0., shrink);
        for _ in 0..48 {
            let mid = (low + high) / 2.;
            if (w * mid + 24.) * (h * mid + 24.) <= MAX_SURFACE_PIXELS {
                low = mid;
            } else {
                high = mid;
            }
        }
        shrink = low;
    }
    w *= shrink;
    h *= shrink;
    if w < 1. || h < 1. {
        return Err("贴图缩放过小".into());
    }
    // Rounding is unavoidable at the physical pixel boundary. The error is at
    // most one pixel; no independent width/height scaling is stored in content.
    let (w, h) = (w.floor().max(1.) as u32, h.floor().max(1.) as u32);
    if f64::from(w + 24) > MAX_SIDE
        || f64::from(h + 24) > MAX_SIDE
        || u64::from(w + 24) * u64::from(h + 24) > MAX_SURFACE_PIXELS as u64
    {
        return Err("贴图窗口超过显示预算".into());
    }
    Ok((w + 24, h + 24))
}
pub(crate) fn initial_rect(
    background: &Asset,
    region: &Region,
    image: &Asset,
    viewport: Rect,
) -> Result<Rect, String> {
    let background_width = f64::from(background.width.ok_or("截图宽度无效")?);
    let background_height = f64::from(background.height.ok_or("截图高度无效")?);
    let ratio = f64::from(image.width.ok_or("贴图宽度无效")?)
        / f64::from(image.height.ok_or("贴图高度无效")?);
    if background_width <= 0.
        || background_height <= 0.
        || viewport.width == 0
        || viewport.height == 0
        || !ratio.is_finite()
        || ratio <= 0.
        || ![region.x, region.y, region.width, region.height]
            .iter()
            .all(|v| v.is_finite())
        || region.width <= 0.
        || region.height <= 0.
    {
        return Err("贴图尺寸无效".into());
    }
    // Match SpaceCanvas's two contain projections in physical client pixels.
    // A restored screenshot may be displayed on a different monitor; its saved
    // capture origin is not the position or scale of the current visible image.
    let scale = (f64::from(viewport.width) / background_width)
        .min(f64::from(viewport.height) / background_height);
    let box_width = region.width * scale;
    let box_height = region.height * scale;
    let content_width = box_width.min(box_height * ratio);
    let content_height = content_width / ratio;
    let width = content_width.round().max(1.) as u32;
    let height = content_height.round().max(1.) as u32;
    let width = width.checked_add(PADDING * 2).ok_or("贴图尺寸无效")?;
    let height = height.checked_add(PADDING * 2).ok_or("贴图尺寸无效")?;
    // Never silently turn an original-size pin into a smaller preview.
    if f64::from(width) > MAX_SIDE
        || f64::from(height) > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_SURFACE_PIXELS as u64
    {
        return Err("贴图窗口超过显示预算".into());
    }
    let x = (f64::from(viewport.x)
        + (f64::from(viewport.width) - background_width * scale) / 2.
        + region.x * scale
        + (box_width - content_width) / 2.)
        .round()
        - f64::from(PADDING);
    let y = (f64::from(viewport.y)
        + (f64::from(viewport.height) - background_height * scale) / 2.
        + region.y * scale
        + (box_height - content_height) / 2.)
        .round()
        - f64::from(PADDING);
    if !(i32::MIN as f64..=i32::MAX as f64).contains(&x)
        || !(i32::MIN as f64..=i32::MAX as f64).contains(&y)
    {
        return Err("贴图位置无效".into());
    }
    Ok(Rect {
        x: x as i32,
        y: y as i32,
        width,
        height,
    })
}
fn transformed(rect: Rect, ratio: f64, scale: f64, swap: bool) -> Result<Rect, String> {
    let (w, h) = (
        f64::from(rect.width.saturating_sub(24)),
        f64::from(rect.height.saturating_sub(24)),
    );
    let (w, h) = if swap { (h, w) } else { (w * scale, h * scale) };
    let (width, height) = fit_dimensions(w, h, ratio)?;
    let x = i64::from(rect.x) + (i64::from(rect.width) - i64::from(width)) / 2;
    let y = i64::from(rect.y) + (i64::from(rect.height) - i64::from(height)) / 2;
    Ok(Rect {
        x: i32::try_from(x).map_err(|_| "贴图位置超出范围")?,
        y: i32::try_from(y).map_err(|_| "贴图位置超出范围")?,
        width,
        height,
    })
}
pub(crate) async fn build(app: &AppHandle, label: &str, original: Rect) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App("index.html?surface=pin".into()),
    )
    .title("Mewu · 贴图")
    .inner_size(80., 80.)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .background_color(tauri::window::Color(0, 0, 0, 0))
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .disable_drag_drop_handler()
    .zoom_hotkeys_enabled(false)
    .on_navigation(|url| {
        (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost"))
            || (cfg!(debug_assertions)
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420))
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()
    .map_err(|_| "无法创建贴图窗口")?;
    let initial_window = window.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(apply_rect(&initial_window, original));
        })
        .map_err(|_| "无法定位贴图")?;
    rx.await.map_err(|_| "贴图初始化中断")??;
    let handle = app.clone();
    let own_label = label.to_string();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::ScaleFactorChanged { .. }) {
            // The OS can resize a non-resizable window during a DPI transition.
            // Refit after its suggested physical rectangle has been applied.
            let app = handle.clone();
            let label = own_label.clone();
            tauri::async_runtime::spawn(async move {
                let queue = app.clone();
                let _ = queue.run_on_main_thread(move || {
                    if let Some(w) = app.get_webview_window(&label) {
                        if let Err(message) = constrain_after_dpi(&app, &w) {
                            let _ = w.emit("pin-error", PinError { message });
                        }
                        pin_host::emit_state(&app, &w);
                    }
                });
            });
        }
    });
    let handle = app.clone();
    let own_label = label.to_string();
    window.on_menu_event(move |_, event| {
        let prefix = format!("{own_label}:");
        let Some(action) = event.id().as_ref().strip_prefix(&prefix) else {
            return;
        };
        let action = action.to_owned();
        let handle = handle.clone();
        let label = own_label.clone();
        tauri::async_runtime::spawn(async move {
            let Some(w) = handle.get_webview_window(&label) else {
                return;
            };
            let result = match action.as_str() {
                "copy" => pin_host::export(&handle, &w, true).await,
                "save" => pin_host::export(&handle, &w, false).await,
                _ => match menu_action(&handle, &w, &action) {
                    Ok(action) => control(&handle, &w, action).await,
                    Err(error) => Err(error),
                },
            };
            if let Err(message) = result {
                let _ = w.emit("pin-error", PinError { message });
            }
        });
    });
    Ok(())
}
fn constrain_after_dpi(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let ratio = {
        let r = app.state::<PinRegistry>();
        let mut s = r.state.lock().map_err(|_| "贴图状态不可用")?;
        let e = s.entries.get_mut(window.label()).ok_or("贴图已关闭")?;
        e.revision = e.revision.saturating_add(1);
        let ratio = f64::from(e.source.asset().width.unwrap())
            / f64::from(e.source.asset().height.unwrap());
        if e.quarter_turns % 2 == 1 {
            1. / ratio
        } else {
            ratio
        }
    };
    let current = current_rect(window)?;
    let fitted = transformed(current, ratio, 1., false)?;
    if fitted != current {
        apply_rect(window, fitted)?;
    }
    Ok(())
}
#[derive(Clone, Serialize)]
struct PinError {
    message: String,
}
pub(crate) fn drag_threshold(scale: f64) -> (f64, f64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            (
                f64::from(GetSystemMetrics(SM_CXDRAG).max(1)) / scale,
                f64::from(GetSystemMetrics(SM_CYDRAG).max(1)) / scale,
            )
        }
    }
    #[cfg(not(windows))]
    {
        (4. / scale, 4. / scale)
    }
}
fn dragged_rect(rect: Rect, dx: i64, dy: i64) -> Result<Rect, String> {
    Ok(Rect {
        x: i32::try_from(i64::from(rect.x) + dx).map_err(|_| "贴图位置超出范围")?,
        y: i32::try_from(i64::from(rect.y) + dy).map_err(|_| "贴图位置超出范围")?,
        ..rect
    })
}
pub(crate) async fn start_drag(window: &WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
        static DRAG: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        let _slot = DRAG.try_acquire().map_err(|_| "贴图正在移动")?;
        if unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) } >= 0 {
            return Ok(());
        }
        let original = current_rect(window)?;
        let mut start = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut start) } == 0 {
            return Err("无法读取指针位置".into());
        }
        let app = window.app_handle();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        let mut last = original;
        loop {
            app.state::<Host>().exit.ensure_running()?;
            let mut point = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut point) } == 0 {
                return Err("无法读取指针位置".into());
            }
            let next = dragged_rect(
                original,
                i64::from(point.x) - i64::from(start.x),
                i64::from(point.y) - i64::from(start.y),
            )?;
            if next != last {
                apply_rect(window, next)?;
                last = next;
            }
            if unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) } >= 0 {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("贴图移动已结束".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
        pin_host::objects::changed(app);
        return Ok(());
    }
    #[cfg(not(windows))]
    window.start_dragging().map_err(|_| "无法拖动贴图".into())
}
pub(crate) fn current_rect(window: &WebviewWindow) -> Result<Rect, String> {
    let position = window.outer_position().map_err(|_| "无法读取贴图位置")?;
    let size = window.outer_size().map_err(|_| "无法读取贴图尺寸")?;
    Ok(Rect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}
pub(crate) fn apply_rect(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
        };
        let hwnd = window.hwnd().map_err(|_| "贴图窗口不可用")?.0;
        if unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.x,
                rect.y,
                rect.width as i32,
                rect.height as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        } == 0
        {
            return Err("无法调整贴图窗口".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        window
            .set_position(tauri::PhysicalPosition::new(rect.x, rect.y))
            .map_err(|_| "无法定位贴图")?;
        window
            .set_size(tauri::PhysicalSize::new(rect.width, rect.height))
            .map_err(|_| "无法调整贴图".into())
    }
}
pub(crate) async fn show_without_activation(
    window: &WebviewWindow,
    cancelled: Arc<AtomicBool>,
    source_viewport: Rect,
    original: Rect,
) -> Result<(), String> {
    let target = window.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                if cancelled.load(Ordering::Acquire) {
                    return Err("已取消贴图".into());
                }
                if space_rect(target.app_handle())? != source_viewport {
                    return Err("截图显示位置已改变，请重试".into());
                }
                crate::capture_visibility::apply(&target)?;
                apply_rect(&target, original)?;
                if current_rect(&target)? != original {
                    return Err("贴图尺寸未能保持，请重试".into());
                }
                #[cfg(windows)]
                {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                        ShowWindow, SW_SHOWNOACTIVATE,
                    };
                    let hwnd = target.hwnd().map_err(|_| "贴图窗口不可用")?.0;
                    unsafe {
                        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                }
                #[cfg(not(windows))]
                target.show().map_err(|_| "无法显示贴图")?;
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(|_| "无法显示贴图")?;
    rx.await.map_err(|_| "贴图显示中断")?
}

fn space_rect(app: &AppHandle) -> Result<Rect, String> {
    let window = app.get_webview_window("space").ok_or("空间已关闭")?;
    let position = window.inner_position().map_err(|_| "无法读取空间位置")?;
    let size = window.inner_size().map_err(|_| "无法读取空间尺寸")?;
    Ok(Rect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

pub(crate) async fn current_space_rect(app: &AppHandle) -> Result<Rect, String> {
    let target = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(space_rect(&target));
    })
    .map_err(|_| "无法读取空间位置")?;
    rx.await.map_err(|_| "空间位置读取中断")?
}

pub(crate) async fn control(
    app: &AppHandle,
    window: &WebviewWindow,
    action: PinAction,
) -> Result<(), String> {
    pin_host::view_state(app, window)?;
    // Closing remains possible while shutdown is preparing; other mutations
    // stay behind the same exit gate as first-party windows.
    if matches!(action, PinAction::Close {}) {
        pin_host::close_entry(app, window.label(), false);
        return Ok(());
    }
    app.state::<Host>().exit.ensure_running()?;
    if matches!(action, PinAction::Escape {}) {
        if let Some(space) = app.get_webview_window("space") {
            if space.is_visible().unwrap_or(false) {
                space.emit("pin-escape", ()).map_err(|_| "无法返回空间")?;
            }
        }
        return Ok(());
    }
    // All geometry mutations execute as one event-loop closure. Rapid wheel,
    // menu and DPI events cannot apply competing snapshots out of order.
    let target = window.clone();
    let handle = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = apply_control(&handle, &target, action);
            if result.is_ok() {
                pin_host::emit_state(&handle, &target);
                arrange_for_space(&handle);
            }
            let _ = tx.send(result);
        })
        .map_err(|_| "无法操作贴图")?;
    rx.await.map_err(|_| "贴图操作中断")?
}
fn apply_control(app: &AppHandle, window: &WebviewWindow, action: PinAction) -> Result<(), String> {
    app.state::<Host>().exit.ensure_running()?;
    let (turns, topmost, opacity, original, ratio) = {
        let r = app.state::<PinRegistry>();
        let s = r.state.lock().map_err(|_| "贴图状态不可用")?;
        let e = s
            .entries
            .get(window.label())
            .filter(|e| e.live)
            .ok_or("贴图尚未就绪或已关闭")?;
        (
            e.quarter_turns,
            e.topmost,
            e.opacity,
            e.original,
            f64::from(e.source.asset().width.unwrap())
                / f64::from(e.source.asset().height.unwrap()),
        )
    };
    let mut new_turns = turns;
    let mut new_topmost = topmost;
    let mut new_opacity = opacity;
    match action {
        PinAction::Zoom { direction } if direction == 1 || direction == -1 => {
            let ratio = if turns % 2 == 1 { 1. / ratio } else { ratio };
            let rect = transformed(
                current_rect(window)?,
                ratio,
                if direction == 1 { 1.08 } else { 1. / 1.08 },
                false,
            )?;
            apply_rect(window, rect)?;
        }
        PinAction::Rotate { quarter_turns } if quarter_turns == 1 || quarter_turns == -1 => {
            new_turns = ((i16::from(turns) + i16::from(quarter_turns) + 4) % 4) as u8;
            let ratio = if new_turns % 2 == 1 {
                1. / ratio
            } else {
                ratio
            };
            apply_rect(window, transformed(current_rect(window)?, ratio, 1., true)?)?;
        }
        PinAction::Restore {} => {
            new_turns = 0;
            apply_rect(window, visible_restore(window, original)?)?;
        }
        PinAction::SetTopmost { enabled } => {
            window
                .set_always_on_top(enabled)
                .map_err(|_| "无法更改贴图置顶")?;
            new_topmost = enabled;
        }
        PinAction::SetOpacity { opacity } if opacity == 0.8 || opacity == 1. => {
            new_opacity = opacity
        }
        _ => return Err("贴图操作参数无效".into()),
    }
    let r = app.state::<PinRegistry>();
    let mut s = r.state.lock().map_err(|_| "贴图状态不可用")?;
    let e = s.entries.get_mut(window.label()).ok_or("贴图已关闭")?;
    e.quarter_turns = new_turns;
    e.topmost = new_topmost;
    e.opacity = new_opacity;
    e.revision = e.revision.saturating_add(1);
    Ok(())
}
fn visible_restore(window: &WebviewWindow, original: Rect) -> Result<Rect, String> {
    let monitors = window.available_monitors().map_err(|_| "无法定位显示器")?;
    if monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        i64::from(original.x) < i64::from(p.x) + i64::from(s.width)
            && i64::from(original.y) < i64::from(p.y) + i64::from(s.height)
            && i64::from(original.x) + i64::from(original.width) > i64::from(p.x)
            && i64::from(original.y) + i64::from(original.height) > i64::from(p.y)
    }) {
        return Ok(original);
    }
    let monitor = window
        .current_monitor()
        .map_err(|_| "无法定位显示器")?
        .or(window.primary_monitor().map_err(|_| "无法定位显示器")?)
        .ok_or("没有可用显示器")?;
    let mut rect = original;
    rect.x = monitor.position().x + 32;
    rect.y = monitor.position().y + 32;
    Ok(rect)
}
fn menu_action(app: &AppHandle, window: &WebviewWindow, action: &str) -> Result<PinAction, String> {
    let state = pin_host::view_state(app, window)?;
    match action {
        "left" => Ok(PinAction::Rotate { quarter_turns: -1 }),
        "right" => Ok(PinAction::Rotate { quarter_turns: 1 }),
        "restore" => Ok(PinAction::Restore {}),
        "topmost" => Ok(PinAction::SetTopmost {
            enabled: !state.topmost,
        }),
        "opacity" => Ok(PinAction::SetOpacity {
            opacity: if state.opacity == 1. { 0.8 } else { 1. },
        }),
        "close" => Ok(PinAction::Close {}),
        _ => Err("贴图菜单无效".into()),
    }
}
pub(crate) fn menu(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    let state = pin_host::view_state(app, window)?;
    app.state::<Host>().exit.ensure_running()?;
    let id = |suffix: &str| format!("{}:{suffix}", window.label());
    let item = |suffix, title| {
        MenuItem::with_id(app, id(suffix), title, true, None::<&str>)
            .map_err(|_| "无法创建贴图菜单")
    };
    let copy = item("copy", "复制图片")?;
    let save = item("save", "保存 PNG…")?;
    let left = item("left", "向左旋转")?;
    let right = item("right", "向右旋转")?;
    let restore = item("restore", "还原大小与方向")?;
    let close = item("close", "关闭贴图")?;
    let topmost = CheckMenuItem::with_id(
        app,
        id("topmost"),
        "置顶",
        true,
        state.topmost,
        None::<&str>,
    )
    .map_err(|_| "无法创建贴图菜单")?;
    let opacity = CheckMenuItem::with_id(
        app,
        id("opacity"),
        "80% 不透明度",
        true,
        state.opacity == 0.8,
        None::<&str>,
    )
    .map_err(|_| "无法创建贴图菜单")?;
    let separator = PredefinedMenuItem::separator(app).map_err(|_| "无法创建贴图菜单")?;
    let menu = Menu::with_items(
        app,
        &[
            &copy, &save, &separator, &left, &right, &restore, &topmost, &opacity, &close,
        ],
    )
    .map_err(|_| "无法创建贴图菜单")?;
    window
        .popup_menu(&menu)
        .map_err(|_| "无法打开贴图菜单".into())
}

pub fn capture_begin(app: &AppHandle) {
    if let Some(r) = app.try_state::<PinRegistry>() {
        if let Ok(mut s) = r.state.lock() {
            s.epoch = s.epoch.saturating_add(1);
        }
    }
}
pub fn arrange_for_space(app: &AppHandle) {
    let Some(registry) = app.try_state::<PinRegistry>() else {
        return;
    };
    if registry.arranging.swap(true, Ordering::AcqRel) {
        return;
    }
    let handle = app.clone();
    if app
        .run_on_main_thread(move || {
            restack(&handle);
            if let Some(r) = handle.try_state::<PinRegistry>() {
                r.arranging.store(false, Ordering::Release);
            }
        })
        .is_err()
    {
        registry.arranging.store(false, Ordering::Release);
    }
}
fn restack(app: &AppHandle) {
    // A visible space also hosts transparent recording/scroll selection modes.
    // Raising it above their controls would hide the user's stop button.
    if crate::recording::busy(app) || crate::scroll_host::busy(app) {
        return;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let Some(space) = app
            .get_webview_window("space")
            .filter(|w| w.is_visible().unwrap_or(false))
        else {
            return;
        };
        let Ok(space_hwnd) = space.hwnd() else {
            return;
        };
        let Some(r) = app.try_state::<PinRegistry>() else {
            return;
        };
        let pins = if let Ok(s) = r.state.lock() {
            s.entries
                .iter()
                .filter(|(_, e)| e.live && e.topmost)
                .map(|(label, e)| (label.clone(), e.epoch == s.epoch))
                .collect::<Vec<_>>()
        } else {
            return;
        };
        let mut windows = Vec::new();
        for (label, new) in pins {
            if let Some(w) = app
                .get_webview_window(&label)
                .filter(|w| w.is_visible().unwrap_or(false))
            {
                if let Ok(hwnd) = w.hwnd() {
                    windows.push((hwnd.0, new));
                }
            }
        }
        // Rank existing windows by actual z-order, not creation time; a user
        // may have raised an older image. There are at most eight owned pins.
        let mut order = Vec::new();
        let mut cursor = unsafe { GetWindow(space_hwnd.0, GW_HWNDFIRST) };
        for _ in 0..2048 {
            if cursor.is_null() {
                break;
            }
            order.push(cursor);
            cursor = unsafe { GetWindow(cursor, GW_HWNDNEXT) };
        }
        windows.sort_by_key(|(h, _)| {
            std::cmp::Reverse(order.iter().position(|v| v == h).unwrap_or(2048))
        });
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        unsafe {
            for &(hwnd, new) in &windows {
                if !new {
                    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
                }
            }
            SetWindowPos(space_hwnd.0, HWND_TOPMOST, 0, 0, 0, 0, flags);
            // The space renders these same independent objects. Keep native
            // surfaces underneath it to avoid duplicate pictures/input targets.
            for &(hwnd, _) in &windows {
                SetWindowPos(hwnd, space_hwnd.0, 0, 0, 0, 0, flags);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pin_drag_crosses_top_and_negative_monitor_origins_without_clamping() {
        let rect = Rect {
            x: 100,
            y: 20,
            width: 300,
            height: 200,
        };
        assert_eq!(
            dragged_rect(rect, -400, -100).unwrap(),
            Rect {
                x: -300,
                y: -80,
                ..rect
            }
        );
        assert!(dragged_rect(
            Rect {
                x: i32::MAX,
                ..rect
            },
            1,
            0
        )
        .is_err());
    }
    fn projected(
        background_size: (u32, u32),
        region: (f64, f64, f64, f64),
        image_size: (u32, u32),
        viewport: Rect,
    ) -> Result<Rect, String> {
        let background = Asset {
            id: "background".into(),
            name: "screen".into(),
            kind: mewu_core::AssetKind::Image,
            path: "synthetic".into(),
            width: Some(background_size.0),
            height: Some(background_size.1),
            origin_x: Some(7000),
            origin_y: Some(-4000),
            scale_factor: Some(1.75),
        };
        let image = Asset {
            width: Some(image_size.0),
            height: Some(image_size.1),
            ..background.clone()
        };
        let region = Region {
            x: region.0,
            y: region.1,
            width: region.2,
            height: region.3,
            ..Default::default()
        };
        initial_rect(&background, &region, &image, viewport)
    }
    #[test]
    fn original_pin_uses_visible_restored_screenshot_and_letterbox() {
        assert_eq!(
            projected(
                (3840, 2160),
                (1000., 600., 800., 400.),
                (800, 400),
                Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080
                }
            )
            .unwrap(),
            Rect {
                x: 488,
                y: 288,
                width: 424,
                height: 224
            }
        );
        assert_eq!(
            projected(
                (3840, 2160),
                (1000., 600., 800., 400.),
                (800, 400),
                Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1200
                }
            )
            .unwrap(),
            Rect {
                x: 488,
                y: 348,
                width: 424,
                height: 224
            }
        );
        assert_eq!(
            projected(
                (1920, 1080),
                (150., 75., 300., 300.),
                (300, 300),
                Rect {
                    x: -2560,
                    y: -200,
                    width: 2560,
                    height: 1440
                }
            )
            .unwrap(),
            Rect {
                x: -2372,
                y: -112,
                width: 424,
                height: 424
            }
        );
    }
    #[test]
    fn override_pin_preserves_centered_visible_content() {
        let viewport = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            projected(
                (1920, 1080),
                (100., 100., 600., 400.),
                (600, 1200),
                viewport
            )
            .unwrap(),
            Rect {
                x: 288,
                y: 88,
                width: 224,
                height: 424
            }
        );
        assert_eq!(
            projected(
                (1920, 1080),
                (100., 100., 600., 400.),
                (1200, 300),
                viewport
            )
            .unwrap(),
            Rect {
                x: 88,
                y: 213,
                width: 624,
                height: 174
            }
        );
    }
    #[test]
    fn original_pin_never_silently_shrinks_over_budget_or_accepts_invalid_viewport() {
        assert!(projected(
            (8192, 8192),
            (0., 0., 8192., 8192.),
            (8192, 8192),
            Rect {
                x: 0,
                y: 0,
                width: 8192,
                height: 8192
            }
        )
        .is_err());
        assert!(projected(
            (1920, 1080),
            (0., 0., 100., 100.),
            (100, 100),
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 1080
            }
        )
        .is_err());
        assert!(projected(
            (1920, 1080),
            (f64::NAN, 0., 100., 100.),
            (100, 100),
            Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        )
        .is_err());
    }
    #[test]
    fn physical_geometry_keeps_aspect_padding_and_budgets() {
        assert_eq!(fit_dimensions(200., 100., 2.).unwrap(), (224, 124));
        assert!(fit_dimensions(f64::NAN, 100., 1.).is_err());
        let (w, h) = fit_dimensions(16000., 16000., 1.).unwrap();
        assert!(w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 16 * 1024 * 1024);
        let original = Rect {
            x: -120,
            y: 10,
            width: 224,
            height: 124,
        };
        let right = transformed(original, 0.5, 1., true).unwrap();
        assert_eq!((right.width, right.height), (124, 224));
        assert_eq!(transformed(right, 2., 1., true).unwrap(), original);
        let zoom = transformed(original, 2., 1.08, false).unwrap();
        assert!((f64::from(zoom.width - 24) / f64::from(zoom.height - 24) - 2.).abs() < 0.02);
    }
}
