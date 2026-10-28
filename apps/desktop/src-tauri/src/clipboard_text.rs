// SPDX-License-Identifier: MPL-2.0
//! First-party, write-only text clipboard. Publication completes before the UI
//! may close. GTK retains its MIME owner; Cocoa runs on the application thread.
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::{oneshot, Semaphore};

const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
static REQUESTS: Semaphore = Semaphore::const_new(16);

fn validate(label: &str, text: &str) -> Result<(), String> {
    if label != "space" {
        return Err("此窗口不能复制文本".into());
    }
    if text.is_empty() || text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Err("复制文本为空、过长或包含空字符".into());
    }
    Ok(())
}

fn deliver(
    send: oneshot::Sender<Result<(), String>>,
    publish: impl FnOnce() -> Result<(), String>,
) {
    // A caller that left before its UI callback must not overwrite a later copy.
    if !send.is_closed() {
        let _ = send.send(publish());
    }
}

#[tauri::command]
pub async fn write_clipboard_text(
    app: AppHandle,
    window: WebviewWindow,
    text: String,
) -> Result<(), String> {
    validate(window.label(), &text)?;
    app.state::<crate::Host>().exit.ensure_running()?;
    let _request = REQUESTS.try_acquire().map_err(|_| "剪贴板正在使用")?;
    let (send, receive) = oneshot::channel();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            deliver(send, || {
                app.state::<crate::Host>().exit.ensure_running()?;
                if !target.is_visible().unwrap_or(false) {
                    return Err("截图窗口已关闭".into());
                }
                // Other exports can hold this lock while awaiting a GTK callback;
                // never block the UI thread waiting for that worker.
                let _clipboard = crate::table_host::CLIPBOARD
                    .try_lock()
                    .map_err(|_| "剪贴板正在使用")?;
                #[cfg(target_os = "linux")]
                {
                    use gtk::prelude::WidgetExt;
                    let widget = target.gtk_window().map_err(|_| "截图窗口已关闭")?;
                    crate::table_clipboard::linux::Data::text(text)?.publish_on(&widget.display())
                }
                #[cfg(not(target_os = "linux"))]
                {
                    arboard::Clipboard::new()
                        .map_err(|_| "无法打开剪贴板")?
                        .set_text(text)
                        .map_err(|_| "无法复制文字".into())
                }
            });
        })
        .map_err(|_| "无法复制文字")?;
    receive.await.map_err(|_| "复制操作中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_first_party_space_can_publish_bounded_unicode_text() {
        assert!(validate("space", "中文\nTEXT COPY").is_ok());
        for owner in [
            "settings",
            "pin-test",
            "extension-object-test",
            "space-other",
        ] {
            assert!(validate(owner, "text").is_err());
        }
        for value in [
            String::new(),
            "a\0b".into(),
            "中".repeat(MAX_TEXT_BYTES / 3 + 1),
        ] {
            assert!(validate("space", &value).is_err());
        }
        assert!(validate("space", &"a".repeat(MAX_TEXT_BYTES)).is_ok());
    }
    #[tokio::test]
    async fn caller_observes_the_actual_publication_result() {
        for outcome in [Ok(()), Err("clipboard locked".to_owned())] {
            let (send, receive) = oneshot::channel();
            deliver(send, || outcome.clone());
            assert_eq!(receive.await.unwrap(), outcome);
        }
    }
    #[test]
    fn abandoned_callback_does_not_overwrite_the_clipboard() {
        let (send, receive) = oneshot::channel();
        drop(receive);
        deliver(send, || panic!("abandoned copy must not publish"));
    }
}
