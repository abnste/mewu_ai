// SPDX-License-Identifier: MPL-2.0
//! Shared typed Windows save dialog with cooperative cancellation on its owning STA.
//! windows = 0.62.2: Win32_Foundation, Win32_System_Com, Win32_UI_Shell,
//! Win32_UI_Shell_Common, Win32_UI_WindowsAndMessaging.
//! Callers own operation permits, source leases, cancellation and commit fences.
//! Only this module creates COM dialogs; format wrappers contain no native loop.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

fn default_filename(basename: &str, local_time: chrono::NaiveDateTime) -> String {
    let name = match basename {
        "截图" | "Mewu-贴图" => crate::ui_language::text("截图", "Screenshot"),
        "录屏" => crate::ui_language::text("录屏", "Recording"),
        other => other,
    };
    format!("{}_{}", name, local_time.format("%Y%m%d_%H%M%S"))
}

#[derive(Clone, Copy, Debug)]
pub struct FormatSpec<F: 'static> {
    pub format: F,
    pub label: &'static str,
    /// First extension is the default; every extension is accepted after Show.
    pub extensions: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub struct DialogSpec<F: 'static> {
    /// Host-defined, extension-free name. Never a renderer-provided path.
    pub basename: &'static str,
    pub formats: &'static [FormatSpec<F>],
    /// Same one-based convention as IFileDialog::GetFileTypeIndex.
    pub default_index: u32,
}

impl<F: Copy + Eq + 'static> DialogSpec<F> {
    pub fn validate(&self) -> Result<(), SaveDialogError> {
        if self.formats.is_empty()
            || self.formats.len() > 8
            || self.basename.is_empty()
            || self.basename.encode_utf16().count() > 128
            || self
                .basename
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            || self.basename.ends_with('.')
            || self.basename.ends_with(' ')
            || self.selected(self.default_index).is_err()
        {
            return Err(SaveDialogError::InvalidFormat);
        }
        let mut seen_extensions = std::collections::HashSet::new();
        for (index, value) in self.formats.iter().enumerate() {
            if value.label.is_empty()
                || value.label.encode_utf16().count() > 128
                || value.label.chars().any(char::is_control)
                || value.extensions.is_empty()
                || value.extensions.len() > 4
                || self.formats[..index]
                    .iter()
                    .any(|old| old.format == value.format)
            {
                return Err(SaveDialogError::InvalidFormat);
            }
            for extension in value.extensions {
                if extension.is_empty()
                    || extension.len() > 12
                    || !extension
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    || !seen_extensions.insert(*extension)
                {
                    return Err(SaveDialogError::InvalidFormat);
                }
            }
        }
        Ok(())
    }

    pub fn selected(&self, index: u32) -> Result<&FormatSpec<F>, SaveDialogError> {
        index
            .checked_sub(1)
            .and_then(|index| self.formats.get(index as usize))
            .ok_or(SaveDialogError::InvalidFormat)
    }

    /// Suffix validates an already selected type; it never selects an encoder.
    pub fn destination(&self, path: &Path, index: u32) -> Result<F, SaveDialogError> {
        let selected = self.selected(index)?;
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(SaveDialogError::InvalidPath);
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or(SaveDialogError::ExtensionConflict)?;
        if !selected
            .extensions
            .iter()
            .any(|expected| extension.eq_ignore_ascii_case(expected))
        {
            return Err(SaveDialogError::ExtensionConflict);
        }
        Ok(selected.format)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveDialogOutcome<F> {
    Selected { path: PathBuf, format: F },
    UserCanceled,
    RequestCanceled,
}

#[derive(Debug)]
pub enum SaveDialogError {
    Unsupported,
    InvalidOwner,
    InvalidFormat,
    InvalidPath,
    ExtensionConflict,
    ThreadUnavailable,
    ThreadInterrupted,
    Rejected(String),
    Native { stage: &'static str, hresult: i32 },
    CancelWatcherUnavailable,
}

impl std::fmt::Display for SaveDialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("此平台尚未接入文件保存"),
            Self::InvalidOwner => f.write_str("保存窗口已不可用"),
            Self::InvalidFormat => f.write_str("保存格式不可用"),
            Self::InvalidPath => f.write_str("保存位置无效"),
            Self::ExtensionConflict => f.write_str("文件扩展名与所选格式不一致"),
            Self::ThreadUnavailable | Self::ThreadInterrupted => f.write_str("保存对话框未完成"),
            Self::Rejected(message) => f.write_str(message),
            Self::Native { .. } => f.write_str("保存窗口无法完成操作"),
            Self::CancelWatcherUnavailable => f.write_str("无法准备保存取消操作"),
        }
    }
}
impl std::error::Error for SaveDialogError {}

/// Blocking adapter: call from the existing host spawn_blocking boundary, never
/// the Tauri event/UI thread. Only plain owned values cross into the STA.
/// `parent_hwnd` comes from the trusted host window, never renderer arguments.
pub fn pick_destination<F: Copy + Eq + Send + Sync + 'static>(
    parent_hwnd: isize,
    spec: DialogSpec<F>,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome<F>, SaveDialogError> {
    spec.validate()?;
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("mewu-save-dialog".into())
            .spawn(move || native::show(parent_hwnd, spec, cancel, allowed))
            .map_err(|_| SaveDialogError::ThreadUnavailable)?
            .join()
            .map_err(|_| SaveDialogError::ThreadInterrupted)?
    }
    #[cfg(not(windows))]
    {
        let _ = (parent_hwnd, spec, cancel, allowed);
        Err(SaveDialogError::Unsupported)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{cell::RefCell, ffi::OsString, os::windows::ffi::OsStringExt};
    use windows::{
        core::{HRESULT, PCWSTR, PWSTR},
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            UI::{
                Shell::{
                    Common::COMDLG_FILTERSPEC, FileSaveDialog, IFileSaveDialog,
                    FOS_DONTADDTORECENT, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT,
                    FOS_PATHMUSTEXIST, FOS_STRICTFILETYPES, SIGDN_FILESYSPATH,
                },
                WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, KillTimer, SetTimer},
            },
        },
    };

    fn error(stage: &'static str, error: windows::core::Error) -> SaveDialogError {
        SaveDialogError::Native {
            stage,
            hresult: error.code().0,
        }
    }
    fn canceled_hresult() -> HRESULT {
        HRESULT::from_win32(ERROR_CANCELLED.0)
    }
    struct Apartment;
    impl Apartment {
        fn enter() -> Result<Self, SaveDialogError> {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                .ok()
                .map_err(|e| error("initialize", e))?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    struct TaskString(PWSTR);
    impl Drop for TaskString {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0 .0.cast())) }
        }
    }

    struct CancelState {
        dialog: IFileSaveDialog,
        cancel: Arc<AtomicBool>,
    }
    thread_local! { static CANCEL_STATE: RefCell<Option<CancelState>> = const { RefCell::new(None) }; }
    struct CancelWatch(usize);
    impl CancelWatch {
        fn attach(
            dialog: &IFileSaveDialog,
            cancel: Arc<AtomicBool>,
        ) -> Result<Self, SaveDialogError> {
            // Dedicated thread has exactly one dialog, one bounded timer. The
            // modal shell loop dispatches this TIMERPROC on the same STA.
            CANCEL_STATE.with(|slot| {
                *slot.borrow_mut() = Some(CancelState {
                    dialog: dialog.clone(),
                    cancel,
                })
            });
            let id = unsafe { SetTimer(None, 0, 50, Some(cancel_tick)) };
            if id == 0 {
                CANCEL_STATE.with(|slot| {
                    slot.borrow_mut().take();
                });
                return Err(SaveDialogError::CancelWatcherUnavailable);
            }
            Ok(Self(id))
        }
    }
    impl Drop for CancelWatch {
        fn drop(&mut self) {
            unsafe {
                let _ = KillTimer(None, self.0);
            }
            CANCEL_STATE.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    unsafe extern "system" fn cancel_tick(_: HWND, _: u32, _: usize, _: u32) {
        // Release RefCell borrow before Close: COM may pump/reenter messages.
        let close = CANCEL_STATE.with(|slot| {
            slot.try_borrow().ok().and_then(|value| {
                value
                    .as_ref()
                    .filter(|v| v.cancel.load(Ordering::Acquire))
                    .map(|v| v.dialog.clone())
            })
        });
        if let Some(dialog) = close {
            // Failure cannot release the host permit: it remains held until
            // Show actually returns. Cancellation is attempted again next tick.
            let _ = unsafe { dialog.Close(canceled_hresult()) };
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn create_dialog<F: Copy + Eq + 'static>(
        spec: &DialogSpec<F>,
    ) -> Result<IFileSaveDialog, SaveDialogError> {
        spec.validate()?;
        let dialog: IFileSaveDialog =
            unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| error("create", e))?;
        // Owning UTF-16 buffers outlive every synchronous Shell configuration
        // call; no borrowed filter/name pointer crosses the STA thread boundary.
        let names: Vec<_> = spec
            .formats
            .iter()
            .map(|value| wide(&crate::ui_language::translated(value.label)))
            .collect();
        let patterns: Vec<_> = spec
            .formats
            .iter()
            .map(|value| {
                wide(
                    &value
                        .extensions
                        .iter()
                        .map(|extension| format!("*.{extension}"))
                        .collect::<Vec<_>>()
                        .join(";"),
                )
            })
            .collect();
        let filters: Vec<_> = names
            .iter()
            .zip(&patterns)
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        let extension = wide(spec.selected(spec.default_index)?.extensions[0]);
        let basename = wide(&default_filename(
            spec.basename,
            chrono::Local::now().naive_local(),
        ));
        unsafe {
            dialog
                .SetFileTypes(&filters)
                .map_err(|e| error("types", e))?;
            dialog
                .SetFileTypeIndex(spec.default_index)
                .map_err(|e| error("type", e))?;
            // No leading dot. Called BEFORE Show so Shell changes the default
            // extension when the user changes the chosen file type.
            dialog
                .SetDefaultExtension(PCWSTR(extension.as_ptr()))
                .map_err(|e| error("extension", e))?;
            // Extension-free name avoids baking the initial format into names.
            dialog
                .SetFileName(PCWSTR(basename.as_ptr()))
                .map_err(|e| error("name", e))?;
            let flags = dialog.GetOptions().map_err(|e| error("options", e))?;
            dialog
                .SetOptions(
                    flags
                        | FOS_OVERWRITEPROMPT
                        | FOS_FORCEFILESYSTEM
                        | FOS_PATHMUSTEXIST
                        | FOS_STRICTFILETYPES
                        | FOS_NOCHANGEDIR
                        | FOS_DONTADDTORECENT,
                )
                .map_err(|e| error("options", e))?;
        }
        Ok(dialog)
    }

    pub(super) fn show<F: Copy + Eq + 'static>(
        parent_hwnd: isize,
        spec: DialogSpec<F>,
        cancel: Arc<AtomicBool>,
        allowed: impl Fn() -> Result<(), String>,
    ) -> Result<SaveDialogOutcome<F>, SaveDialogError> {
        let _apartment = Apartment::enter()?; // dropped after every COM object
        let owner = HWND(parent_hwnd as *mut std::ffi::c_void);
        let mut process_id = 0;
        if parent_hwnd == 0
            || !unsafe { IsWindow(Some(owner)) }.as_bool()
            || unsafe { GetWindowThreadProcessId(owner, Some(&mut process_id)) } == 0
            || process_id != std::process::id()
        {
            return Err(SaveDialogError::InvalidOwner);
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(SaveDialogOutcome::RequestCanceled);
        }
        allowed().map_err(SaveDialogError::Rejected)?;
        let dialog = create_dialog(&spec)?;
        let watch = CancelWatch::attach(&dialog, cancel.clone())?;
        if cancel.load(Ordering::Acquire) {
            return Ok(SaveDialogOutcome::RequestCanceled);
        }
        allowed().map_err(SaveDialogError::Rejected)?;
        let shown = unsafe { dialog.Show(Some(owner)) };
        drop(watch); // no cancellation callback can touch result extraction
        if let Err(failure) = shown {
            if failure.code() == canceled_hresult() {
                return Ok(if cancel.load(Ordering::Acquire) {
                    SaveDialogOutcome::RequestCanceled
                } else {
                    SaveDialogOutcome::UserCanceled
                });
            }
            return Err(error("show", failure)); // actual error is never cancel
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(SaveDialogOutcome::RequestCanceled);
        }
        allowed().map_err(SaveDialogError::Rejected)?;
        let index = unsafe { dialog.GetFileTypeIndex() }.map_err(|e| error("selected-type", e))?;
        // Reject an unexpected Shell index before touching a result path.
        spec.selected(index)?;
        let item = unsafe { dialog.GetResult() }.map_err(|e| error("result", e))?;
        let raw = TaskString(
            unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(|e| error("path", e))?,
        );
        if raw.0.is_null() {
            return Err(SaveDialogError::InvalidPath);
        }
        let wide = unsafe { raw.0.as_wide() };
        if wide.len() > 32_767 {
            return Err(SaveDialogError::InvalidPath);
        }
        // Preserve non-Unicode Windows filenames; never UTF-8 lossy conversion.
        let path = PathBuf::from(OsString::from_wide(wide));
        let format = spec.destination(&path, index)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(SaveDialogOutcome::RequestCanceled);
        }
        allowed().map_err(SaveDialogError::Rejected)?;
        Ok(SaveDialogOutcome::Selected { path, format })
    }

    /// Optional ignored probe for parent to compile/run later. NEVER calls Show,
    /// GetResult, a picker, an encoder, an audio device or a user-file operation.
    #[cfg(test)]
    #[test]
    #[ignore = "explicit COM configuration probe; never displays a dialog"]
    fn configure_formats_without_showing_a_dialog() {
        std::thread::spawn(|| {
            let _apartment = Apartment::enter().unwrap();
            // This exercises the real shared COM configuration. Wrappers own
            // format tables; no test constructs a second native implementation.
            for spec in [
                crate::video_save_dialog::spec(false),
                crate::video_save_dialog::spec(true),
            ] {
                let dialog = create_dialog(&spec).unwrap();
                assert_eq!(unsafe { dialog.GetFileTypeIndex() }.unwrap(), 1);
                if spec.formats.len() == 2 {
                    unsafe { dialog.SetFileTypeIndex(2) }.unwrap();
                    assert_eq!(unsafe { dialog.GetFileTypeIndex() }.unwrap(), 2);
                }
            }
            for basename in crate::image_save_dialog::ImageSaveName::ALL {
                let dialog = create_dialog(&crate::image_save_dialog::spec(basename)).unwrap();
                assert_eq!(unsafe { dialog.GetFileTypeIndex() }.unwrap(), 1);
                unsafe { dialog.SetFileTypeIndex(2) }.unwrap();
                assert_eq!(unsafe { dialog.GetFileTypeIndex() }.unwrap(), 2);
            }
        })
        .join()
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FORMATS: &[FormatSpec<u8>] = &[
        FormatSpec {
            format: 1,
            label: "PNG 图片",
            extensions: &["png"],
        },
        FormatSpec {
            format: 2,
            label: "JPEG 图片",
            extensions: &["jpg", "jpeg"],
        },
    ];
    fn spec() -> DialogSpec<u8> {
        DialogSpec {
            basename: "截图",
            formats: FORMATS,
            default_index: 1,
        }
    }

    #[test]
    fn format_table_is_bounded_unique_and_one_based() {
        spec().validate().unwrap();
        assert_eq!(spec().selected(1).unwrap().format, 1);
        assert_eq!(spec().selected(2).unwrap().format, 2);
        for index in [0, 3, u32::MAX] {
            assert!(spec().selected(index).is_err());
        }
        assert!(DialogSpec {
            formats: &[],
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            default_index: 3,
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            basename: "../escape",
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            basename: "a\0b",
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            formats: &[FormatSpec {
                format: 1,
                label: "x",
                extensions: &["png", "png"]
            }],
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            formats: &[FormatSpec {
                format: 1,
                label: "x",
                extensions: &["p*ng"]
            }],
            ..spec()
        }
        .validate()
        .is_err());
        assert!(DialogSpec {
            formats: &[
                FormatSpec {
                    format: 1,
                    label: "x",
                    extensions: &["png"]
                },
                FormatSpec {
                    format: 1,
                    label: "y",
                    extensions: &["jpg"]
                }
            ],
            ..spec()
        }
        .validate()
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn extension_is_only_validation_never_format_inference() {
        for file in [
            r"D:\test\截图.jpg",
            r"D:\test\截图.JPEG",
            r"D:\test\screenshot.png.jpg",
        ] {
            assert_eq!(spec().destination(Path::new(file), 2).unwrap(), 2);
            assert!(matches!(
                spec().destination(Path::new(file), 1),
                Err(SaveDialogError::ExtensionConflict)
            ));
        }
        for file in [
            r"D:\test\截图",
            r"D:\test\clip.png",
            r"D:\test\clip.gif",
            r"relative.jpg",
        ] {
            assert!(spec().destination(Path::new(file), 2).is_err());
        }
    }
}
