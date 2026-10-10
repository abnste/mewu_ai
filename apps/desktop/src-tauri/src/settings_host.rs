// SPDX-License-Identifier: MPL-2.0
use crate::{capture_preferences as prefs, settings_info, Host};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPreferencesStatus {
    #[serde(flatten)]
    value: crate::system_preferences::SystemPreferences,
    startup_registered: bool,
    startup_warning: Option<String>,
}

#[tauri::command]
pub fn get_system_preferences(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<SystemPreferencesStatus, String> {
    owner(&window)?;
    let value = app
        .state::<Host>()
        .system_preferences
        .view()
        .map_err(|error| error.to_string())?;
    let (startup_registered, startup_warning) =
        crate::startup_windows::status(value.startup_owner.as_deref());
    Ok(SystemPreferencesStatus {
        value,
        startup_registered,
        startup_warning,
    })
}

#[tauri::command]
pub async fn set_system_preferences(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    network_proxy_mode: crate::system_preferences::ProxyMode,
    network_proxy_url: String,
    launch_at_startup: bool,
    allow_screen_share: bool,
    auto_generate_title: bool,
) -> Result<SystemPreferencesStatus, String> {
    owner(&window)?;
    if !save_allowed(&app) {
        return Err("当前无法修改系统设置".into());
    }
    let previous = app
        .state::<Host>()
        .system_preferences
        .view()
        .map_err(|error| error.to_string())?;
    let startup_owner = previous
        .startup_owner
        .or_else(|| launch_at_startup.then(|| uuid::Uuid::new_v4().to_string()));
    let task = app
        .state::<Host>()
        .system_preferences
        .begin_save(crate::system_preferences::SaveRequest {
            expected_revision,
            network_proxy_mode,
            network_proxy_url,
            launch_at_startup,
            startup_owner: startup_owner.clone(),
            allow_screen_share,
            auto_generate_title,
        })
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        if !save_allowed(&app) {
            return Err("当前无法修改系统设置".to_string());
        }
        // Own the same gate as capture/recording/scroll while OS side effects
        // and preference CAS settle. It is held through rollback on failure.
        let host = app.state::<Host>();
        let _transition =
            crate::SpaceTransition::try_begin(&host.capturing).ok_or("当前无法修改系统设置")?;
        let allowed = || {
            host.exit.accepts_new_operations()
                && !crate::scroll_host::busy(&app)
                && crate::recording::ensure_idle(&app).is_ok()
        };
        if !allowed() {
            return Err("当前无法修改系统设置".into());
        }
        crate::capture_visibility::validate(allow_screen_share)?;
        let visibility =
            crate::capture_visibility::Change::apply_current(&app, allow_screen_share)?;
        let startup = match crate::startup_windows::Change::apply(
            launch_at_startup,
            startup_owner.as_deref(),
        ) {
            Ok(change) => change,
            Err(error) => {
                return match visibility.rollback() {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(format!("{error}；{rollback}")),
                }
            }
        };
        let receipt = match task.commit(allowed) {
            Ok(receipt) => {
                startup.commit();
                visibility.commit();
                receipt
            }
            Err(error) => {
                let mut errors = vec![error.to_string()];
                if let Err(rollback) = startup.rollback() {
                    errors.push(rollback);
                }
                if let Err(rollback) = visibility.rollback() {
                    errors.push(rollback);
                }
                return Err(errors.join("；"));
            }
        };
        if let Err(error) = crate::network_policy::configure(&receipt.value) {
            let _ = app.emit_to("settings", "host-error", error);
        }
        if !receipt.value.auto_generate_title {
            host.session_titles.cancel_all();
        }
        let (startup_registered, startup_warning) =
            crate::startup_windows::status(receipt.value.startup_owner.as_deref());
        let value = SystemPreferencesStatus {
            value: receipt.value,
            startup_registered,
            startup_warning,
        };
        let _ = app.emit_to("settings", "system-preferences-changed", &value);
        if receipt.durability_warning {
            let _ = app.emit_to(
                "settings",
                "host-error",
                "系统设置已保存，持久化状态需要检查",
            );
        }
        Ok(value)
    })
    .await
    .map_err(|_| "系统设置保存中断".to_string())?
}

fn owner(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("此窗口不能访问设置".into())
    }
}
fn save_allowed(app: &AppHandle) -> bool {
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    host.exit.accepts_new_operations()
        && !host.capturing.load(Ordering::Acquire)
        && !crate::scroll_host::busy(app)
        && crate::recording::ensure_idle(app).is_ok()
}
#[tauri::command]
pub fn get_capture_preferences(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<prefs::CapturePreferences, String> {
    owner(&window)?;
    app.state::<Host>()
        .capture_preferences
        .view()
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn set_capture_preferences(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    capture_delay_seconds: prefs::CaptureDelay,
    include_cursor: bool,
    default_image_format: prefs::DefaultImageFormat,
) -> Result<prefs::CapturePreferences, String> {
    owner(&window)?;
    if !save_allowed(&app) {
        return Err("当前无法修改截图设置".into());
    }
    let task = app
        .state::<Host>()
        .capture_preferences
        .begin_save(prefs::SaveRequest {
            expected_revision,
            capture_delay_seconds,
            include_cursor,
            default_image_format,
        })
        .map_err(|e| e.to_string())?;
    // The task and event live in the actual worker, even if the invoke is abandoned.
    tauri::async_runtime::spawn_blocking(move || {
        let receipt = task
            .commit(|| save_allowed(&app))
            .map_err(|e| e.to_string())?;
        if receipt.changed {
            let _ = app.emit_to("settings", "capture-preferences-changed", &receipt.value);
        }
        if receipt.durability_warning {
            let _ = app.emit_to(
                "settings",
                "host-error",
                "截图设置已保存，持久化状态需要检查",
            );
        }
        Ok(receipt.value)
    })
    .await
    .map_err(|_| "截图设置保存中断".to_string())?
}
#[tauri::command]
pub fn settings_info(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<settings_info::SettingsInfo, String> {
    owner(&window)?;
    app.state::<Host>()
        .settings_resources
        .as_ref()
        .map_err(|e| e.to_string())?
        .info()
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn read_license_document(
    window: WebviewWindow,
    kind: settings_info::LicenseDocumentKind,
) -> Result<settings_info::LicenseDocument, String> {
    owner(&window)?;
    settings_info::read_license_document(kind).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn open_data_directory(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    owner(&window)?;
    app.state::<Host>().exit.ensure_running()?;
    let task = app
        .state::<Host>()
        .settings_resources
        .as_ref()
        .map_err(|e| e.to_string())?
        .begin_open_data_directory()
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        task.open(move || app.state::<Host>().exit.is_running())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "数据目录操作中断".to_string())?
}
