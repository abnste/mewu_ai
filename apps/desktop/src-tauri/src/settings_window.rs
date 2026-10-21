// SPDX-License-Identifier: MPL-2.0
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

const LABEL: &str = "settings";
static OPENING: Mutex<()> = Mutex::new(());

/// Only a window created by this call belongs to failed-open cleanup. Reusing
/// an existing settings window must never destroy it if presentation fails.
struct PendingSettings<T> {
    window: T,
    cleanup: Option<fn(&T)>,
}
impl<T> PendingSettings<T> {
    fn new(window: T, cleanup: fn(&T)) -> Self {
        Self {
            window,
            cleanup: Some(cleanup),
        }
    }
    fn existing(window: T) -> Self {
        Self {
            window,
            cleanup: None,
        }
    }
    fn presented(&mut self) {
        self.cleanup.take();
    }
}
impl<T> Drop for PendingSettings<T> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup(&self.window);
        }
    }
}

fn destroy_unopened(window: &tauri::WebviewWindow) {
    // Keep the exact runtime identity; never fetch another window by LABEL.
    if window.destroy().is_err() {
        let _ = window.app_handle().emit_to(
            "space",
            "host-error",
            "无法关闭未打开的设置窗口，请退出后重试",
        );
    }
}

/// Call from an async command or worker, never a synchronous WebView event handler.
/// Closing this window uses Tauri's normal destruction path; it is not kept alive.
pub fn show(app: &AppHandle) -> Result<(), String> {
    let _opening = OPENING.lock().map_err(|_| "无法打开设置")?;
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    let mut pending = if let Some(window) = app.get_webview_window(LABEL) {
        PendingSettings::existing(window)
    } else {
        let navigation_app = app.clone();
        let window = tauri::WebviewWindowBuilder::new(
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
        .on_navigation(move |url| {
            let local_app = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                || (matches!(url.scheme(), "http" | "https")
                    && url.host_str() == Some("tauri.localhost"));
            let local_dev = cfg!(debug_assertions)
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420);
            local_app || local_dev || navigation_app
                .state::<crate::extension_host::ExtensionHost>()
                .surfaces
                .allows_window_document(url, &navigation_app.state::<crate::Host>().content_origin, LABEL)
        })
        .build()
        .map_err(|error| {
            crate::window_creation_error::describe(
                crate::window_creation_error::Surface::Settings,
                &error,
            )
        })?;
        PendingSettings::new(window, destroy_unopened)
    };
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    let window = &pending.window;
    window.unminimize().map_err(|_| "无法恢复设置窗口")?;
    window.show().map_err(|_| "无法显示设置窗口")?;
    // A visible window is useful even if Windows refuses foreground focus.
    // Do not destroy it for a focus-only failure after successful presentation.
    pending.presented();
    pending.window.set_focus().map_err(|_| "无法激活设置窗口")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Window {
        id: u32,
        closed: Arc<Mutex<Vec<u32>>>,
    }

    fn close(window: &Window) {
        window.closed.lock().unwrap().push(window.id);
    }

    #[test]
    fn settings_window_pending_cleans_only_exact_new_window_on_failed_open() {
        let closed = Arc::new(Mutex::new(vec![]));
        let created = PendingSettings::new(
            Window {
                id: 1,
                closed: closed.clone(),
            },
            close,
        );
        let existing = PendingSettings::existing(Window {
            id: 2,
            closed: closed.clone(),
        });
        drop(existing);
        assert!(closed.lock().unwrap().is_empty());
        drop(created);
        assert_eq!(*closed.lock().unwrap(), vec![1]);
    }

    #[test]
    fn settings_window_pending_retains_new_window_after_successful_presentation() {
        let closed = Arc::new(Mutex::new(vec![]));
        let mut created = PendingSettings::new(
            Window {
                id: 1,
                closed: closed.clone(),
            },
            close,
        );
        created.presented();
        created.presented();
        drop(created);
        assert!(closed.lock().unwrap().is_empty());
    }
}
