// SPDX-License-Identifier: MPL-2.0
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

const LABEL: &str = "settings";
static OPENING: Mutex<()> = Mutex::new(());

/// Call from an async command or worker, never a synchronous WebView event handler.
/// Closing this window uses Tauri's normal destruction path; it is not kept alive.
pub fn show(app: &AppHandle) -> Result<(), String> {
    let _opening = OPENING.lock().map_err(|_| "无法打开设置")?;
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    let window = if let Some(window) = app.get_webview_window(LABEL) {
        window
    } else {
        tauri::WebviewWindowBuilder::new(
            app,
            LABEL,
            tauri::WebviewUrl::App("index.html?surface=settings".into()),
        )
        .title("Mewu 设置")
        // Fill the client area: Windows ignores the HWND background alpha,
        // so a transparent CSS surround would expose a solid black border.
        // Use the native shadow/corners rather than an inset CSS shadow.
        .inner_size(820., 650.)
        .min_inner_size(660., 460.)
        .resizable(true)
        .maximizable(true)
        .minimizable(true)
        .decorations(false)
        .transparent(false)
        .background_color(tauri::window::Color(245, 247, 251, 255))
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(false)
        .center()
        .visible(false)
        .disable_drag_drop_handler()
        .on_navigation(|url| {
            let local_app = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                || (matches!(url.scheme(), "http" | "https")
                    && url.host_str() == Some("tauri.localhost"));
            let local_dev = cfg!(debug_assertions)
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420);
            local_app || local_dev
        })
        .build()
        .map_err(|_| "无法创建设置窗口")?
    };
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    window.unminimize().map_err(|_| "无法恢复设置窗口")?;
    window.show().map_err(|_| "无法显示设置窗口")?;
    window.set_focus().map_err(|_| "无法激活设置窗口")?;
    Ok(())
}
