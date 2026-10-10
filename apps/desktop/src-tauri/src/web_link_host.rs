// SPDX-License-Identifier: MPL-2.0
use crate::{code_open, Host};
use tauri::{AppHandle, Manager, WebviewWindow};

#[tauri::command]
pub async fn open_web_link(
    app: AppHandle,
    window: WebviewWindow,
    url: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能打开回复链接".into());
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
