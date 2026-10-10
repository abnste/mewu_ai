// SPDX-License-Identifier: MPL-2.0
//! A native, cancelable presentation choice. It never modifies saved annotations.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportVariant {
    Annotated,
    Clean,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Image,
    Video,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Selected(ExportVariant),
    UserCanceled,
    RequestCanceled,
}

fn check(cancel: &AtomicBool, allowed: &dyn Fn() -> Result<(), String>) -> Result<bool, String> {
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    allowed()?;
    Ok(!cancel.load(Ordering::Acquire))
}

fn choose_with(
    has_annotations: bool,
    cancel: &AtomicBool,
    allowed: &dyn Fn() -> Result<(), String>,
    show: impl FnOnce() -> Result<Outcome, String>,
) -> Result<Outcome, String> {
    if !check(cancel, allowed)? {
        return Ok(Outcome::RequestCanceled);
    }
    if !has_annotations {
        return Ok(Outcome::Selected(ExportVariant::Clean));
    }
    let selected = show()?;
    // A valid button click is not publication. Revalidate the original request
    // after the native dialog, including a cancellation racing its last callback.
    if !check(cancel, allowed)? {
        return Ok(Outcome::RequestCanceled);
    }
    Ok(selected)
}

/// Called only from a host-owned blocking worker with its real operation lease.
pub fn pick_variant(
    parent_hwnd: isize,
    media: Media,
    has_annotations: bool,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<Outcome, String> {
    std::thread::Builder::new()
        .name("mewu-export-choice".into())
        .spawn(move || {
            choose_with(has_annotations, &cancel, &allowed, || {
                native::show(parent_hwnd, media, &cancel, &allowed)
            })
        })
        .map_err(|_| "无法打开导出选项")?
        .join()
        .map_err(|_| "导出选项已中断")?
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::cell::{Cell, RefCell};
    use windows::{
        core::{HRESULT, PCWSTR},
        Win32::{
            Foundation::{HWND, LPARAM, WPARAM},
            UI::{
                Controls::{
                    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON,
                    TASKDIALOG_NOTIFICATIONS, TDF_ALLOW_DIALOG_CANCELLATION, TDF_CALLBACK_TIMER,
                    TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT, TDM_CLICK_BUTTON,
                    TDN_BUTTON_CLICKED, TDN_CREATED, TDN_TIMER,
                },
                WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, PostMessageW, IDCANCEL},
            },
        },
    };
    const ANNOTATED: i32 = 1001;
    const CLEAN: i32 = 1002;
    struct State<'a> {
        cancel: &'a AtomicBool,
        allowed: &'a dyn Fn() -> Result<(), String>,
        failure: RefCell<Option<String>>,
        canceled: Cell<bool>,
    }
    impl State<'_> {
        fn current(&self) -> bool {
            if self.canceled.get() {
                return false;
            }
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check(self.cancel, self.allowed)
            })) {
                Ok(Ok(true)) => true,
                value => {
                    self.canceled.set(true);
                    let failure = match value {
                        Ok(Err(error)) => Some(error),
                        Err(_) => Some("无法核对导出来源".into()),
                        _ => None,
                    };
                    *self.failure.borrow_mut() = failure;
                    false
                }
            }
        }
    }
    unsafe extern "system" fn callback(
        hwnd: HWND,
        event: TASKDIALOG_NOTIFICATIONS,
        button: WPARAM,
        _: LPARAM,
        data: isize,
    ) -> HRESULT {
        // TaskDialogIndirect is synchronous; State and every UTF-16 buffer live
        // until it returns. Posting avoids a reentrant button callback while
        // the source/admission check holds its short-lived host locks.
        let state = &*(data as *const State<'_>);
        if event == TDN_CREATED || event == TDN_TIMER || event == TDN_BUTTON_CLICKED {
            if !state.current() {
                if event == TDN_BUTTON_CLICKED && button.0 == IDCANCEL.0 as usize {
                    return HRESULT(0);
                }
                let _ = PostMessageW(
                    Some(hwnd),
                    TDM_CLICK_BUTTON.0 as u32,
                    WPARAM(IDCANCEL.0 as usize),
                    LPARAM(0),
                );
                if event == TDN_BUTTON_CLICKED {
                    return HRESULT(1);
                }
            }
        }
        HRESULT(0)
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }
    #[cfg(test)]
    pub(super) fn test_gate() {
        let cancel = AtomicBool::new(false);
        let current = Cell::new(true);
        let allowed = || {
            if current.get() {
                Ok(())
            } else {
                Err("source changed".into())
            }
        };
        let state = State {
            cancel: &cancel,
            allowed: &allowed,
            failure: RefCell::new(None),
            canceled: Cell::new(false),
        };
        assert!(state.current());
        current.set(false);
        assert!(!state.current());
        current.set(true);
        assert!(!state.current());
        assert_eq!(state.failure.borrow().as_deref(), Some("source changed"));
        let state = State {
            cancel: &cancel,
            allowed: &allowed,
            failure: RefCell::new(None),
            canceled: Cell::new(false),
        };
        cancel.store(true, Ordering::Release);
        assert!(!state.current());
        cancel.store(false, Ordering::Release);
        assert!(!state.current());
        assert!(state.failure.borrow().is_none());
    }
    pub(super) fn show(
        parent_hwnd: isize,
        media: Media,
        cancel: &AtomicBool,
        allowed: &dyn Fn() -> Result<(), String>,
    ) -> Result<Outcome, String> {
        let owner = HWND(parent_hwnd as *mut _);
        let mut process = 0;
        if parent_hwnd == 0
            || !unsafe { IsWindow(Some(owner)) }.as_bool()
            || unsafe { GetWindowThreadProcessId(owner, Some(&mut process)) } == 0
            || process != std::process::id()
        {
            return Err("导出窗口已不可用".into());
        }
        let text = crate::ui_language::text;
        let title = wide(match media {
            Media::Image => text("保存图片", "Save image"),
            Media::Video => text("保存视频", "Save video"),
        });
        let instruction = wide(text("导出版本", "Export version"));
        let annotated = wide(text("带标注版本", "With annotations"));
        let clean = wide(text("干净原件", "Clean copy"));
        let cancel_label = wide(text("取消", "Cancel"));
        let buttons = [
            TASKDIALOG_BUTTON {
                nButtonID: ANNOTATED,
                pszButtonText: PCWSTR(annotated.as_ptr()),
            },
            TASKDIALOG_BUTTON {
                nButtonID: CLEAN,
                pszButtonText: PCWSTR(clean.as_ptr()),
            },
            TASKDIALOG_BUTTON {
                nButtonID: IDCANCEL.0,
                pszButtonText: PCWSTR(cancel_label.as_ptr()),
            },
        ];
        let state = State {
            cancel,
            allowed,
            failure: RefCell::new(None),
            canceled: Cell::new(false),
        };
        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: owner,
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
                | TDF_CALLBACK_TIMER
                | TDF_POSITION_RELATIVE_TO_WINDOW
                | TDF_SIZE_TO_CONTENT,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            pszMainInstruction: PCWSTR(instruction.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: ANNOTATED,
            pfCallback: Some(callback),
            lpCallbackData: &state as *const State<'_> as isize,
            ..Default::default()
        };
        let mut selected = IDCANCEL.0;
        unsafe { TaskDialogIndirect(&config, Some(&mut selected), None, None) }
            .map_err(|_| "无法打开导出选项")?;
        if let Some(error) = state.failure.into_inner() {
            return Err(error);
        }
        if state.canceled.get() {
            return Ok(Outcome::RequestCanceled);
        }
        Ok(match selected {
            ANNOTATED => Outcome::Selected(ExportVariant::Annotated),
            CLEAN => Outcome::Selected(ExportVariant::Clean),
            _ => Outcome::UserCanceled,
        })
    }
}

#[cfg(not(windows))]
mod native {
    use super::*;
    pub(super) fn show(
        _: isize,
        media: Media,
        _: &AtomicBool,
        _: &dyn Fn() -> Result<(), String>,
    ) -> Result<Outcome, String> {
        let text = crate::ui_language::text;
        let annotated = text("带标注版本", "With annotations");
        let clean = text("干净原件", "Clean copy");
        let result = rfd::MessageDialog::new()
            .set_title(match media {
                Media::Image => text("保存图片", "Save image"),
                Media::Video => text("保存视频", "Save video"),
            })
            .set_description(text("导出版本", "Export version"))
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                annotated.into(),
                clean.into(),
                text("取消", "Cancel").into(),
            ))
            .show();
        Ok(match result {
            rfd::MessageDialogResult::Custom(value) if value == annotated => {
                Outcome::Selected(ExportVariant::Annotated)
            }
            rfd::MessageDialogResult::Custom(value) if value == clean => {
                Outcome::Selected(ExportVariant::Clean)
            }
            _ => Outcome::UserCanceled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn clean_source_bypasses_dialog_and_canceled_admission_never_opens_it() {
        let cancel = AtomicBool::new(false);
        let checks = Cell::new(0);
        assert_eq!(
            choose_with(
                false,
                &cancel,
                &|| {
                    checks.set(checks.get() + 1);
                    Ok(())
                },
                || panic!("no annotations")
            ),
            Ok(Outcome::Selected(ExportVariant::Clean))
        );
        assert_eq!(checks.get(), 1);
        cancel.store(true, Ordering::Release);
        assert_eq!(
            choose_with(true, &cancel, &|| panic!("canceled admission"), || panic!(
                "must not show"
            )),
            Ok(Outcome::RequestCanceled)
        );
    }
    #[test]
    fn original_request_is_rechecked_after_selection_and_neither_cancel_can_publish() {
        for selected in [
            Outcome::Selected(ExportVariant::Annotated),
            Outcome::Selected(ExportVariant::Clean),
            Outcome::UserCanceled,
        ] {
            let cancel = AtomicBool::new(false);
            assert_eq!(
                choose_with(true, &cancel, &|| Ok(()), || Ok(selected)),
                Ok(selected)
            );
            assert_eq!(
                choose_with(true, &cancel, &|| Ok(()), || {
                    cancel.store(true, Ordering::Release);
                    Ok(selected)
                }),
                Ok(Outcome::RequestCanceled)
            );
            let stale = Cell::new(false);
            cancel.store(false, Ordering::Release);
            assert_eq!(
                choose_with(
                    true,
                    &cancel,
                    &|| if stale.get() {
                        Err("source changed".into())
                    } else {
                        Ok(())
                    },
                    || {
                        stale.set(true);
                        Ok(selected)
                    }
                ),
                Err("source changed".into())
            );
        }
    }
    #[cfg(windows)]
    #[test]
    fn native_timer_gate_keeps_request_cancel_and_source_failure_sticky() {
        native::test_gate();
    }
}
