// SPDX-License-Identifier: MPL-2.0
use crate::{code_open, Host};
use tauri::{AppHandle, Manager, WebviewWindow};

fn source_allowed(label: &str) -> bool {
    matches!(label, "space" | "settings")
}

#[tauri::command]
pub async fn open_web_link(
    app: AppHandle,
    window: WebviewWindow,
    url: String,
) -> Result<(), String> {
    if !source_allowed(window.label()) {
        return Err("此窗口不能打开网页链接".into());
    }
    app.state::<Host>().exit.ensure_new_operation()?;
    let address = code_open::http_url(&url).ok_or("链接不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        code_open::open(address, move || {
            app.state::<Host>().exit.ensure_new_operation()
        })
    })
    .await
    .map_err(|_| "打开链接的任务已中断".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_settings_and_space_can_open_web_links_without_widening_other_windows() {
        for label in ["space", "settings"] {
            assert!(source_allowed(label));
        }
        for label in [
            "",
            "Settings",
            "settings-other",
            "pin",
            "frozen-widget",
            "recording-controls",
            "scroll-controls",
            "extension:example",
        ] {
            assert!(!source_allowed(label), "{label}");
        }
        let settings: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/settings.json")).unwrap();
        assert_eq!(settings["windows"], serde_json::json!(["settings"]));
        assert!(settings["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "allow-open-web-link"));
    }

    #[test]
    fn plugin_project_and_release_links_keep_existing_explicit_http_validation() {
        for address in [
            "https://github.com/abnste/mewu-plugin-region-agent/tree/bc96f00e0be1293ed1870b9c66aadb64c83c0e60",
            "https://github.com/abnste/mewu-plugins/releases/tag/v0.1.2",
        ] {
            assert_eq!(code_open::http_url(address).unwrap().as_str(), address);
        }
        for address in [
            "javascript:alert(1)",
            "file:///tmp/mewu",
            "https://user:secret@github.com/own/repo",
            "https://github.com/own/repo\n",
        ] {
            assert!(code_open::http_url(address).is_none());
        }
    }
}
