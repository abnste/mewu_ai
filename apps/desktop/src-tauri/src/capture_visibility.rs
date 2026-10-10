// SPDX-License-Identifier: MPL-2.0
//! Screen sharing policy for presentation windows. Capture workers override it
//! while collecting live pixels, then restore this policy before showing space.
use std::sync::{Mutex, MutexGuard};
use tauri::Manager;

static AFFINITY: Mutex<()> = Mutex::new(());

fn presentation(label: &str) -> bool {
    label == "space" || label == crate::frozen::LABEL || crate::pin_host::is_pin_label(label)
}
pub fn validate(allow_screen_share: bool) -> Result<(), String> {
    if allow_screen_share {
        return Ok(());
    }
    #[cfg(windows)]
    {
        crate::recording_storage::supported()
            .map_err(|_| "隐藏屏幕共享中的框选和标注需要 Windows 10 2004 或更新版本".into())
    }
    #[cfg(not(windows))]
    {
        Err("此平台暂不支持隐藏屏幕共享中的框选和标注".into())
    }
}
pub fn apply(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    let hwnd = window.hwnd().map_err(|_| "无法设置屏幕共享可见性")?.0 as usize;
    let _guard = AFFINITY.lock().map_err(|_| "屏幕共享设置不可用")?;
    let value = window
        .app_handle()
        .state::<crate::Host>()
        .system_preferences
        .snapshot_for_operation()
        .map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        validate(value.allow_screen_share)?;
        let expected = affinity(value.allow_screen_share, false);
        write(hwnd, expected)
    }
    #[cfg(not(windows))]
    {
        if value.allow_screen_share {
            Ok(())
        } else {
            Err("此平台暂不支持隐藏屏幕共享中的框选和标注".into())
        }
    }
}

#[cfg(windows)]
fn read(hwnd: usize) -> Result<u32, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity;
    let mut value = u32::MAX;
    if unsafe { GetWindowDisplayAffinity(hwnd as _, &mut value) } == 0 {
        return Err("无法读取窗口的屏幕共享状态".into());
    }
    Ok(value)
}

#[cfg(windows)]
fn write(hwnd: usize, expected: u32) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity;
    if unsafe { SetWindowDisplayAffinity(hwnd as _, expected) } == 0 || read(hwnd)? != expected {
        return Err("无法应用屏幕共享设置".into());
    }
    Ok(())
}

#[cfg(windows)]
fn owns_cookie(hwnd: usize, cookie: &[u16]) -> bool {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetPropW(hwnd as _, cookie.as_ptr()) as usize
            == 1
    }
}
#[cfg(windows)]
fn remove_cookie(hwnd: usize, cookie: &[u16]) {
    if owns_cookie(hwnd, cookie) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::RemovePropW(hwnd as _, cookie.as_ptr());
        }
    }
}
#[cfg(windows)]
fn stamp(
    app: &tauri::AppHandle,
    cookie: &[u16],
) -> Result<Vec<(tauri::WebviewWindow, usize)>, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let target = app.clone();
    let cookie = cookie.to_vec();
    app.run_on_main_thread(move || {
        let mut stamped = Vec::new();
        let result = (|| {
            for window in target
                .webview_windows()
                .into_values()
                .filter(|w| presentation(w.label()))
            {
                // Identity is stamped on the native message thread, where window
                // destruction cannot interleave with obtaining this HWND.
                let hwnd = window.hwnd().map_err(|_| "无法读取窗口身份")?.0 as usize;
                if unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::SetPropW(
                        hwnd as _,
                        cookie.as_ptr(),
                        1usize as _,
                    )
                } == 0
                {
                    return Err("无法确认窗口身份".to_string());
                }
                stamped.push((window, hwnd));
            }
            Ok(())
        })();
        if let Err(error) = result {
            for (_, hwnd) in &stamped {
                remove_cookie(*hwnd, &cookie);
            }
            let _ = send.send(Err(error));
        } else if let Err(error) = send.send(Ok(stamped)) {
            if let Ok(stamped) = error.0 {
                for (_, hwnd) in stamped {
                    remove_cookie(hwnd, &cookie);
                }
            }
        }
    })
    .map_err(|_| "无法准备屏幕共享设置")?;
    receive.recv().map_err(|_| "屏幕共享设置准备中断")?
}

/// Held on the settings blocking worker under SpaceTransition. Preference CAS
/// commits only after every existing presentation window accepts the OS policy.
/// The mutex also prevents a late first-show callback from applying an old value.
pub struct Change {
    guard: Option<MutexGuard<'static, ()>>,
    #[cfg(windows)]
    entries: Vec<(tauri::WebviewWindow, usize, Option<u32>, u32)>,
    #[cfg(windows)]
    cookie: Vec<u16>,
    finished: bool,
}
impl Change {
    pub fn apply_current(app: &tauri::AppHandle, allow: bool) -> Result<Self, String> {
        validate(allow)?;
        #[cfg(windows)]
        let cookie: Vec<_> = format!("Mewu-Affinity-{}", uuid::Uuid::new_v4())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        #[cfg(windows)]
        let stamped = stamp(app, &cookie)?;
        let guard = match AFFINITY.lock() {
            Ok(guard) => guard,
            Err(_) => {
                #[cfg(windows)]
                for (_, hwnd) in &stamped {
                    remove_cookie(*hwnd, &cookie);
                }
                return Err("屏幕共享设置不可用".into());
            }
        };
        let mut change = Self {
            guard: Some(guard),
            #[cfg(windows)]
            entries: stamped
                .into_iter()
                .map(|(window, hwnd)| (window, hwnd, None, affinity(allow, false)))
                .collect(),
            #[cfg(windows)]
            cookie,
            finished: false,
        };
        #[cfg(windows)]
        {
            let result: Result<(), String> = (|| {
                for (_, hwnd, before, expected) in &mut change.entries {
                    // Capture/recording/scroll admission is excluded by the
                    // caller's transition; their sampling override stays intact.
                    if !owns_cookie(*hwnd, &change.cookie) {
                        continue;
                    }
                    let original = read(*hwnd)?;
                    if original != *expected {
                        *before = Some(original);
                        write(*hwnd, *expected)?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                return match change.rollback() {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(format!("{error}；{rollback}")),
                };
            }
        }
        #[cfg(not(windows))]
        let _ = app;
        Ok(change)
    }
    pub fn commit(mut self) {
        #[cfg(windows)]
        for (_, hwnd, _, _) in &self.entries {
            remove_cookie(*hwnd, &self.cookie);
        }
        self.guard.take();
        self.finished = true;
    }
    fn restore(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            let mut failed = Vec::new();
            for (window, hwnd, before, after) in self.entries.iter().rev() {
                // Never select a replacement by label or overwrite another
                // affinity. Destroyed windows no longer expose any pixels.
                if !owns_cookie(*hwnd, &self.cookie) {
                    continue;
                }
                if let Some(before) = before {
                    let restored = read(*hwnd)
                        .is_ok_and(|actual| actual == *after || actual == *before)
                        && write(*hwnd, *before).is_ok();
                    if !restored {
                        failed.push(window.clone());
                    }
                }
                remove_cookie(*hwnd, &self.cookie);
            }
            // Never call a Tauri/UI dispatcher while holding AFFINITY: the
            // message thread may itself be waiting to apply a display policy.
            self.guard.take();
            for window in &failed {
                let _ = window.hide();
            }
            if !failed.is_empty() {
                return Err("部分窗口未能恢复屏幕共享状态，已隐藏，请重新显示后重试".into());
            }
        }
        self.guard.take();
        Ok(())
    }
    pub fn rollback(mut self) -> Result<(), String> {
        let result = self.restore();
        self.finished = true;
        result
    }
}
impl Drop for Change {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.restore();
        }
    }
}
fn affinity(allow_screen_share: bool, collecting_pixels: bool) -> u32 {
    if collecting_pixels || !allow_screen_share {
        0x11
    } else {
        0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_and_scrolling_always_exclude_the_capture_overlay() {
        assert_eq!(affinity(true, false), 0);
        assert_eq!(affinity(false, false), 0x11);
        assert_eq!(affinity(true, true), 0x11);
        assert_eq!(affinity(false, true), 0x11);
    }
    #[test]
    fn policy_targets_only_actual_presentation_surfaces() {
        assert!(presentation("space"));
        assert!(presentation(crate::frozen::LABEL));
        assert!(presentation("pin-04f2cc75-dd56-4891-8d8c-430d285773de"));
        for label in [
            "settings",
            "recording-controls",
            "scroll-controls",
            "pin-evil",
            "plugin-space",
        ] {
            assert!(!presentation(label));
        }
    }
}
