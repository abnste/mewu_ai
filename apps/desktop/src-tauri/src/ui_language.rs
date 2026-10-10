// SPDX-License-Identifier: MPL-2.0
use serde::Deserialize;
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::{menu::MenuItem, AppHandle, Manager, WebviewWindow};
static LANGUAGE: AtomicU8 = AtomicU8::new(0);
#[derive(Clone, Copy, Deserialize)]
pub enum DisplayLanguage {
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en-US")]
    English,
}
pub struct TrayLabels {
    pub settings: MenuItem<tauri::Wry>,
    pub quit: MenuItem<tauri::Wry>,
}
pub fn text<'a>(chinese: &'a str, english: &'a str) -> &'a str {
    let language = LANGUAGE.load(Ordering::Relaxed);
    #[cfg(windows)]
    let system_chinese =
        unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() & 0x3ff == 4 };
    #[cfg(not(windows))]
    let system_chinese = false;
    if language == 1 || language == 0 && system_chinese {
        chinese
    } else {
        english
    }
}
pub fn translated(input: &str) -> String {
    if text("zh", "en") == "zh" {
        return input.to_owned();
    }
    static ENGLISH: std::sync::OnceLock<std::collections::HashMap<String, String>> =
        std::sync::OnceLock::new();
    ENGLISH
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../src/locales/en-US.json"))
                .expect("validated UI dictionary")
        })
        .get(input)
        .cloned()
        .unwrap_or_else(|| input.to_owned())
}
#[tauri::command]
pub fn set_ui_language(
    app: AppHandle,
    window: WebviewWindow,
    locale: DisplayLanguage,
) -> Result<(), String> {
    if !matches!(window.label(), "space" | "settings") {
        return Err("此窗口不能修改界面语言".into());
    }
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    LANGUAGE.store(
        match locale {
            DisplayLanguage::Chinese => 1,
            DisplayLanguage::English => 2,
        },
        Ordering::Relaxed,
    );
    if let Some(labels) = app.try_state::<TrayLabels>() {
        labels
            .settings
            .set_text(text("设置", "Settings"))
            .map_err(|_| "无法更新托盘语言")?;
        labels
            .quit
            .set_text(text("退出", "Quit"))
            .map_err(|_| "无法更新托盘语言")?;
    }
    Ok(())
}
