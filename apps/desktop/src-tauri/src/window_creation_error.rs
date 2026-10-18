// SPDX-License-Identifier: MPL-2.0
//! Safe diagnostics shared by auxiliary first-party windows. Error text may
//! contain profile paths or callbacks, so only inspect known typed variants.

#[derive(Clone, Copy)]
pub(crate) enum Surface {
    Settings,
    #[cfg(windows)]
    RecordingControls,
}

impl Surface {
    fn message(self) -> &'static str {
        match self {
            Self::Settings => "无法创建设置窗口",
            #[cfg(windows)]
            Self::RecordingControls => "无法打开录屏控制条",
        }
    }
}

#[cfg(windows)]
fn webview_error(error: &wry::Error) -> (&'static str, Option<u32>) {
    match error {
        wry::Error::WebView2Error(webview2_com::Error::WindowsError(error)) => {
            ("WebView2", Some(error.code().0 as u32))
        }
        wry::Error::WebView2Error(_) => ("WebView2", None),
        _ => ("WebView", None),
    }
}

/// These wrappers do not expose their boxed causes through Error::source().
/// Match the actual locked types instead of parsing Display strings or sampling
/// GetLastError after the fact. No paths, labels, callbacks or OS text escape.
pub(crate) fn describe(surface: Surface, error: &tauri::Error) -> String {
    #[cfg(windows)]
    {
        let (stage, code) = match error {
            tauri::Error::WindowLabelAlreadyExists(_)
            | tauri::Error::WebviewLabelAlreadyExists(_) => ("窗口已存在", None),
            tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(error)) => error
                .downcast_ref::<wry::Error>()
                .map(webview_error)
                .unwrap_or(("WebView", None)),
            tauri::Error::Runtime(tauri_runtime::Error::CreateWindow) => ("创建窗口", None),
            tauri::Error::Runtime(tauri_runtime::Error::WebviewRuntimeNotInstalled) => {
                ("缺少 WebView2", None)
            }
            tauri::Error::Runtime(tauri_runtime::Error::EventLoopClosed) => {
                ("窗口循环已结束", None)
            }
            tauri::Error::Runtime(_) => ("窗口运行时", None),
            _ => ("创建窗口", None),
        };
        let message = surface.message();
        match code {
            Some(code) => format!("{message}（{stage}，0x{code:08X}）"),
            None => format!("{message}（{stage}）"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        surface.message().into()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn window_creation_diagnostics_keep_typed_hresult_for_settings_and_recording() {
        for code in [0x80070005u32, 0x8007139F, 0x80004002] {
            let error = tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(Box::new(
                wry::Error::WebView2Error(webview2_com::Error::WindowsError(
                    windows::core::Error::new(
                        windows::core::HRESULT(code as i32),
                        r"C:\Users\fixture\private-profile token=private-token",
                    ),
                )),
            )));
            for surface in [Surface::Settings, Surface::RecordingControls] {
                assert_eq!(
                    describe(surface, &error),
                    format!("{}（WebView2，0x{code:08X}）", surface.message())
                );
            }
        }
    }

    #[test]
    fn window_creation_diagnostics_never_parse_sensitive_callback_or_unknown_cause() {
        let callback = tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(Box::new(
            wry::Error::WebView2Error(webview2_com::Error::CallbackError(
                r"C:\Users\fixture\private-profile token=private-token 0xDEADBEEF".into(),
            )),
        )));
        let unknown = tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(Box::new(
            std::io::Error::other(
                r"C:\Users\fixture\private-profile token=private-token 0xDEADBEEF",
            ),
        )));
        for surface in [Surface::Settings, Surface::RecordingControls] {
            assert_eq!(
                describe(surface, &callback),
                format!("{}（WebView2）", surface.message())
            );
            assert_eq!(
                describe(surface, &unknown),
                format!("{}（WebView）", surface.message())
            );
        }
    }

    #[test]
    fn window_creation_diagnostics_keep_only_fixed_categories_without_labels() {
        for surface in [Surface::Settings, Surface::RecordingControls] {
            for error in [
                tauri::Error::WindowLabelAlreadyExists("private-label-0xDEADBEEF".into()),
                tauri::Error::WebviewLabelAlreadyExists("private-label-0xDEADBEEF".into()),
            ] {
                assert_eq!(
                    describe(surface, &error),
                    format!("{}（窗口已存在）", surface.message())
                );
            }
            for (error, stage) in [
                (tauri_runtime::Error::CreateWindow, "创建窗口"),
                (
                    tauri_runtime::Error::WebviewRuntimeNotInstalled,
                    "缺少 WebView2",
                ),
                (tauri_runtime::Error::EventLoopClosed, "窗口循环已结束"),
            ] {
                assert_eq!(
                    describe(surface, &tauri::Error::Runtime(error)),
                    format!("{}（{stage}）", surface.message())
                );
            }
        }
    }
}
