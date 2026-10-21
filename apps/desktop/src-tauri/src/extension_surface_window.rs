// SPDX-License-Identifier: MPL-2.0
//! First-party shell for any plugin surface; no plugin IDs or business layout.
use crate::{extension_host::ExtensionHost, extension_surfaces::Owner, Host};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
const PREFIX: &str = "extension-surface-";
fn navigation_allowed(url: &tauri::Url, document_url: &tauri::Url) -> bool {
    // WebKit calls the navigation delegate for subframes as well. Permit only
    // this surface's exact document, not the whole loopback content server.
    url == document_url
        || (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost"))
        || (cfg!(debug_assertions)
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(1420))
}
pub(crate) fn id(label: &str) -> Option<&str> {
    let id = label.strip_prefix(PREFIX)?;
    uuid::Uuid::parse_str(id)
        .ok()
        .filter(|value| value.to_string() == id)
        .map(|_| id)
}
pub(crate) fn bound(app: &AppHandle, window: &WebviewWindow) -> Result<(String, Owner), String> {
    let id = id(window.label()).ok_or("此窗口不是插件对象")?;
    let owner = app
        .state::<ExtensionHost>()
        .surfaces
        .window_owner(id)
        .ok_or("插件对象已关闭")?;
    Ok((id.into(), owner))
}
pub(crate) fn close(app: &AppHandle, id: &str) {
    let host = app.state::<ExtensionHost>();
    if let Some(owner) = host.surfaces.window_owner(id) {
        let _ = host.close_surface(id, &owner);
    }
    if let Some(window) = app.get_webview_window(&format!("{PREFIX}{id}")) {
        let _ = window.destroy();
    }
    let _ = app.emit_to("space", "extension-surface-closed", id);
}
pub(crate) fn destroyed(app: &AppHandle, id: &str) {
    // A webview can disappear without a client acknowledgement. Do not drain
    // an in-flight plugin transaction on the platform event-loop thread.
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<ExtensionHost>();
        if let Some(owner) = host.surfaces.window_owner(&id) {
            let _ = host.close_surface(&id, &owner);
        }
        let _ = app.emit_to("space", "extension-surface-closed", &id);
    });
}
pub(crate) fn reconcile(app: &AppHandle) {
    let active = app.state::<Host>().lock().ok().and_then(|engine| {
        engine
            .store
            .active_scene()
            .ok()
            .map(|s| (s.id.clone(), s.closed))
    });
    reconcile_active(
        app,
        active.as_ref().map(|(id, closed)| (id.as_str(), *closed)),
    );
}
pub(crate) fn reconcile_snapshot(app: &AppHandle, snapshot: &mewu_core::Snapshot) {
    let scene = snapshot
        .scenes
        .iter()
        .find(|s| s.id == snapshot.active_scene_id);
    reconcile_active(app, scene.map(|s| (s.id.as_str(), s.closed)));
}
fn reconcile_active(app: &AppHandle, active: Option<(&str, bool)>) {
    let host = app.state::<ExtensionHost>();
    for (label, _) in app.webview_windows() {
        let Some(id) = id(&label) else { continue };
        let owner = host.surfaces.window_owner(id);
        if owner.is_none() {
            close(app, id);
        } else if !host.surfaces.pinned(id)
            && owner.as_ref().is_some_and(|owner| {
                active.is_none_or(|(scene, closed)| closed || scene != owner.scene)
            })
        {
            request_close(app, id);
        }
    }
}
pub(crate) fn request_close(app: &AppHandle, id: &str) {
    let _ = app.emit_to(
        format!("{PREFIX}{id}"),
        "extension-surface-close-request",
        (),
    );
}
#[tauri::command]
pub async fn open_extension_surface_window(
    app: AppHandle,
    window: WebviewWindow,
    surface_id: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    if window.label() != "space"
        || ![x, y]
            .iter()
            .all(|n| n.is_finite() && (-32768.0..=32768.0).contains(n))
    {
        return Err("无法打开插件对象".into());
    }
    app.state::<Host>().exit.ensure_new_operation()?;
    let state = app.state::<ExtensionHost>();
    if let Some(existing) = app.get_webview_window(&format!("{PREFIX}{surface_id}")) {
        existing.show().map_err(|_| "无法显示插件对象")?;
        return existing.set_focus().map_err(|_| "无法聚焦插件对象".into());
    }
    let owner = state.surfaces.attach_window(&surface_id)?;
    let check: Result<String, String> = (|| {
        let registry = state.store()?;
        let record = registry.get(&owner.plugin)?;
        if record.state != "enabled" || record.revision != owner.revision {
            return Err("插件已撤权".into());
        }
        record.ensure_supported()?;
        registry.validate_dependencies(&record.manifest)?;
        let engine = app.state::<Host>();
        let engine = engine.lock()?;
        let scene = engine.store.active_scene().map_err(|e| e.to_string())?;
        if scene.id != owner.scene || scene.closed || scene.frozen {
            return Err("会话已变化".into());
        }
        Ok(record.manifest.name.clone())
    })();
    let build_window=window.clone();
    let build_surface_id=surface_id.clone();
    let result: Result<(WebviewWindow, crate::pin_window::Rect), String> = crate::pin_window::build_floating(&app, move |app| {
        let state=app.state::<ExtensionHost>();
        let window=&build_window;
        let surface_id=&build_surface_id;
        let title = check?;
        let view = state
            .surfaces
            .window_state(&surface_id, &app.state::<Host>().content_origin)
            .ok_or("插件界面已关闭")?;
        let document_url = view["url"]
            .as_str()
            .and_then(|value| tauri::Url::parse(value).ok())
            .ok_or("插件界面地址无效")?;
        let scale = window.scale_factor().map_err(|_| "无法读取屏幕比例")?;
        let origin = window
            .inner_position()
            .map_err(|_| "无法读取屏幕位置")?
            .to_logical::<f64>(scale);
        let label = format!("{PREFIX}{surface_id}");
        let builder = tauri::WebviewWindowBuilder::new(
            app,
            &label,
            tauri::WebviewUrl::App("index.html?surface=extension-object".into()),
        )
        .title(format!("Mewu · {title}"))
        .inner_size(
            view["width"].as_f64().unwrap() + 24.,
            view["height"].as_f64().unwrap() + 24.,
        )
        .min_inner_size(180., 140.)
        .max_inner_size(1624., 1224.)
        .position(origin.x + x - 12., origin.y + y - 12.)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .disable_drag_drop_handler()
        .zoom_hotkeys_enabled(false)
        .on_navigation(move |url| navigation_allowed(url, &document_url))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
        #[cfg(target_os = "macos")]
        let builder = builder.accept_first_mouse(true);
        let created = builder
            .build()
            .map_err(|e| format!("无法打开插件对象：{e}"))?;
        let rect = crate::pin_window::Rect {
            x: ((origin.x + x - 12.) * scale).round() as i32,
            y: ((origin.y + y - 12.) * scale).round() as i32,
            width: ((view["width"].as_f64().unwrap() + 24.) * scale).round() as u32,
            height: ((view["height"].as_f64().unwrap() + 24.) * scale).round() as u32,
        };
        Ok((created, rect))
    }).await;
    let result: Result<(), String> = async {
        let (created, rect) = result?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let target = created.clone();
        created
            .run_on_main_thread(move || {
                let result = crate::pin_window::prepare_floating(&target)
                    .and_then(|_| crate::pin_window::apply_rect(&target, rect));
                let _ = tx.send(result);
            })
            .map_err(|_| "无法初始化插件对象")?;
        rx.await.map_err(|_| "插件对象初始化中断")??;
        state.surfaces.mark_window_prepared(&surface_id)?;
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(10);
        while !created.is_visible().map_err(|_|"插件对象已关闭")? {
            if std::time::Instant::now()>=deadline||state.surfaces.window_owner(&surface_id).is_none(){
                return Err("插件对象界面加载失败".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        close(&app, &surface_id);
    }
    result
}
#[tauri::command]
pub fn get_extension_surface_window(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Value, String> {
    let (id, owner) = bound(&app, &window)?;
    let registry = app.state::<ExtensionHost>();
    let registry = registry.store()?;
    let record = registry.get(&owner.plugin)?;
    if record.state != "enabled" || record.revision != owner.revision {
        return Err("插件已撤权".into());
    }
    app.state::<ExtensionHost>()
        .surfaces
        .window_state(&id, &app.state::<Host>().content_origin)
        .ok_or("插件对象已关闭".into())
}
#[tauri::command]
pub async fn control_extension_surface_window(
    app: AppHandle,
    window: WebviewWindow,
    action: String,
) -> Result<(), String> {
    let (id, _) = bound(&app, &window)?;
    match action.as_str() {
        "ready" => {
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
            while !app.state::<ExtensionHost>().surfaces.window_prepared(&id){
                if std::time::Instant::now()>=deadline||app.state::<ExtensionHost>().surfaces.window_owner(&id).is_none(){
                    return Err("插件对象初始化未完成".into());
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            window.show().map_err(|_| "无法显示插件对象")?;
            window.set_focus().map_err(|_| "无法聚焦插件对象")?;
        }
        "drag" => crate::pin_window::start_drag(&window).await?,
        "pin" => app
            .state::<ExtensionHost>()
            .surfaces
            .pin_window(&id, true)?,
        "unpin" => {
            app.state::<ExtensionHost>()
                .surfaces
                .pin_window(&id, false)?;
            reconcile(&app);
        }
        "close" => close(&app, &id),
        _ => return Err("未知对象操作".into()),
    }
    Ok(())
}
#[tauri::command]
pub async fn release_extension_surface_window(
    app: AppHandle,
    window: WebviewWindow,
    surface_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能释放插件对象".into());
    }
    if !app.state::<ExtensionHost>().surfaces.pinned(&surface_id) {
        request_close(&app, &surface_id);
    }
    Ok(())
}
#[tauri::command]
pub async fn request_close_extension_surface_window(
    app: AppHandle,
    window: WebviewWindow,
    surface_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能关闭插件对象".into());
    }
    request_close(&app, &surface_id);
    Ok(())
}
#[tauri::command]
pub fn extension_surface_viewport_point(
    app: AppHandle,
    window: WebviewWindow,
    x: f64,
    y: f64,
) -> Result<Value, String> {
    let (_, owner) = bound(&app, &window)?;
    if ![x, y].iter().all(|n| n.is_finite() && n.abs() <= 32768.) {
        return Err("无效坐标".into());
    }
    {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        let scene = engine.store.active_scene().map_err(|e| e.to_string())?;
        if scene.id != owner.scene || scene.closed || scene.frozen {
            return Err("请回到原截图空间取出对象".into());
        }
    }
    let space = app.get_webview_window("space").ok_or("截图空间未打开")?;
    let origin = window.inner_position().map_err(|_| "无法读取对象位置")?;
    let scale = window.scale_factor().map_err(|_| "无法读取屏幕比例")?;
    let target = space.inner_position().map_err(|_| "无法读取空间位置")?;
    let size = space.inner_size().map_err(|_| "无法读取空间大小")?;
    if size.width == 0 || size.height == 0 {
        return Err("截图空间不可见".into());
    }
    Ok(
        serde_json::json!({"x":((origin.x as f64+x*scale-target.x as f64)/size.width as f64).clamp(0.,0.95),"y":((origin.y as f64+y*scale-target.y as f64)/size.height as f64).clamp(0.,0.95)}),
    )
}
/// Only the trusted shell can register hit targets. The plugin supplies bounded
/// client rectangles; the host supplies actual window position and DPI mapping.
#[tauri::command]
pub fn publish_extension_surface_targets(
    app: AppHandle,
    window: WebviewWindow,
    targets: Value,
) -> Result<(), String> {
    let (id, owner) = bound(&app, &window)?;
    let targets = targets.as_array().ok_or("无效拖放区域")?;
    if targets.len() > 64 {
        return Err("拖放区域过多".into());
    }
    for target in targets {
        if target
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(|s| s.is_empty() || s.len() > 128)
            || ["x", "y", "width", "height"].iter().any(|key| {
                target
                    .get(key)
                    .and_then(Value::as_f64)
                    .is_none_or(|v| !v.is_finite() || v.abs() > 32768.)
            })
        {
            return Err("无效拖放区域".into());
        }
    }
    let space = app.get_webview_window("space").ok_or("截图空间未打开")?;
    let origin = window.inner_position().map_err(|_| "无法读取对象位置")?;
    let source_scale = window.scale_factor().map_err(|_| "无法读取屏幕比例")?;
    let target = space.inner_position().map_err(|_| "无法读取空间位置")?;
    let target_scale = space.scale_factor().map_err(|_| "无法读取空间比例")?;
    let mapped:Vec<Value>=targets.iter().map(|r|serde_json::json!({
        "id":r["id"],
        "x":(origin.x as f64+r["x"].as_f64().unwrap()*source_scale-target.x as f64)/target_scale,
        "y":(origin.y as f64+r["y"].as_f64().unwrap()*source_scale-target.y as f64)/target_scale,
        "width":r["width"].as_f64().unwrap()*source_scale/target_scale,
        "height":r["height"].as_f64().unwrap()*source_scale/target_scale
    })).collect();
    app.emit_to(
        "space",
        "extension-surface-targets",
        serde_json::json!({"surfaceId":id,"owner":owner,"targets":mapped}),
    )
    .map_err(|_| "无法更新拖放区域".into())
}
#[tauri::command]
pub fn forward_extension_surface_event(
    app: AppHandle,
    window: WebviewWindow,
    surface_id: String,
    name: String,
    value: Value,
) -> Result<(), String> {
    if window.label() != "space"
        || !matches!(name.as_str(), "drag" | "change")
        || value.to_string().len() > 1024 * 1024
    {
        return Err("无效插件事件".into());
    }
    if app
        .state::<ExtensionHost>()
        .surfaces
        .window_owner(&surface_id)
        .is_none()
    {
        return Err("插件对象已关闭".into());
    }
    app.emit_to(
        format!("{PREFIX}{surface_id}"),
        "extension-surface-event",
        serde_json::json!({"name":name,"value":value}),
    )
    .map_err(|_| "无法发送插件事件".into())
}

#[cfg(test)]
mod tests {
    use super::navigation_allowed;

    #[test]
    fn surface_navigation_allows_its_document_without_opening_other_content() {
        let document = tauri::Url::parse(
            "http://127.0.0.1:43123/2352a796-4f50-4bd0-b796-4826f3f85de9",
        )
        .unwrap();
        assert!(navigation_allowed(&document, &document));
        for shell in [
            "tauri://localhost/index.html?surface=extension-object",
            "http://tauri.localhost/index.html?surface=extension-object",
            "https://tauri.localhost/index.html?surface=extension-object",
        ] {
            assert!(navigation_allowed(&tauri::Url::parse(shell).unwrap(), &document));
        }
        for denied in [
            "http://127.0.0.1:43123/2352a796-4f50-4bd0-b796-4826f3f85de8",
            "http://127.0.0.1:43124/2352a796-4f50-4bd0-b796-4826f3f85de9",
            "http://127.0.0.1:43123/2352a796-4f50-4bd0-b796-4826f3f85de9?other=1",
            "http://user@127.0.0.1:43123/2352a796-4f50-4bd0-b796-4826f3f85de9",
            "https://example.com/",
            "file:///tmp/other.html",
            "javascript:alert(1)",
        ] {
            assert!(!navigation_allowed(&tauri::Url::parse(denied).unwrap(), &document));
        }
    }
}
