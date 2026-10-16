// SPDX-License-Identifier: MPL-2.0
//! A capture lease for an owned HWND. Shared capture never enables WDA protection.
//! Mutations and restoration run on the HWND's message thread, before sampling and
//! after its worker has drained. Do not restore a replacement selected by label.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, RECT},
    Graphics::{
        Dwm::{DwmFlush, DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_CLOAKED},
        Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn, SetWindowRgn, HRGN,
            RGN_COPY, RGN_DIFF,
        },
    },
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        GetPropW, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsWindow, RemovePropW, SetPropW, SetWindowDisplayAffinity,
        SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
    },
};

struct Dpi(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
impl Dpi {
    fn physical() -> Result<Self, String> {
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let old =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if old.0.is_null() {
            Err("无法确认采集屏幕坐标".into())
        } else {
            Ok(Self(old))
        }
    }
}
impl Drop for Dpi {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
        }
    }
}
fn window_rect(hwnd: HWND) -> Result<RECT, String> {
    let _dpi = Dpi::physical()?;
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        Err("无法读取采集窗口位置".into())
    } else {
        Ok(rect)
    }
}

const INPUT_BITS: isize = (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) as isize;
const APP_CLOAK: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Rect {
    pub fn offset(
        origin_x: i32,
        origin_y: i32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let x =
            i32::try_from(i64::from(origin_x) + i64::from(x)).map_err(|_| "采集屏幕坐标无效")?;
        let y =
            i32::try_from(i64::from(origin_y) + i64::from(y)).map_err(|_| "采集屏幕坐标无效")?;
        Self::new(x, y, width, height)
    }
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, String> {
        let right = i64::from(x) + i64::from(width);
        let bottom = i64::from(y) + i64::from(height);
        if width == 0 || height == 0 || right > i64::from(i32::MAX) || bottom > i64::from(i32::MAX)
        {
            return Err("采集区域无效".into());
        }
        Ok(Self {
            left: x,
            top: y,
            right: right as i32,
            bottom: bottom as i32,
        })
    }
    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
    fn contains(self, other: Self) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }
}

/// Prefer the same top/side controls, but never clamp them back into the pixels.
/// All values are physical screen pixels; the HUD's OS shadow is disabled.
pub(crate) fn hud_position(work: Rect, roi: Rect, size: (u32, u32)) -> Option<(i32, i32)> {
    let w = i32::try_from(size.0).ok()?;
    let h = i32::try_from(size.1).ok()?;
    if w <= 0 || h <= 0 {
        return None;
    }
    let x = (i64::from(roi.left) + (i64::from(roi.right) - i64::from(roi.left) - i64::from(w)) / 2)
        .clamp(
            i64::from(work.left),
            i64::from(work.right)
                .checked_sub(i64::from(w))?
                .max(i64::from(work.left)),
        ) as i32;
    let y = roi.top.max(work.top).min(work.bottom.saturating_sub(h));
    for (left, top) in [
        (x, work.top.saturating_add(6)),
        (roi.right.saturating_add(6), y),
        (roi.left.saturating_sub(w).saturating_sub(6), y),
        (x, roi.bottom.saturating_add(6)),
        (x, roi.top.saturating_sub(h).saturating_sub(6)),
    ] {
        let candidate = Rect::new(left, top, size.0, size.1).ok()?;
        if work.contains(candidate) && !roi.intersects(candidate) {
            return Some((left, top));
        }
    }
    None
}

struct Region(usize);
impl Region {
    fn empty() -> Result<Self, String> {
        let handle = unsafe { CreateRectRgn(0, 0, 0, 0) };
        if handle.is_null() {
            Err("无法准备采集窗口区域".into())
        } else {
            Ok(Self(handle as usize))
        }
    }
    fn raw(&self) -> HRGN {
        self.0 as HRGN
    }
    fn copy(&self) -> Result<Self, String> {
        let copy = Self::empty()?;
        if unsafe { CombineRgn(copy.raw(), self.raw(), std::ptr::null_mut(), RGN_COPY) } == 0 {
            return Err("无法复制采集窗口区域".into());
        }
        Ok(copy)
    }
    fn assign(self, hwnd: HWND) -> Result<(), String> {
        if unsafe { SetWindowRgn(hwnd, self.raw(), 1) } == 0 {
            return Err("无法设置采集窗口区域".into());
        }
        // SetWindowRgn transfers ownership to Windows, including an empty region.
        std::mem::forget(self);
        Ok(())
    }
}
impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.raw());
        }
    }
}

fn affinity(hwnd: HWND) -> Result<u32, String> {
    let mut value = u32::MAX;
    if unsafe { GetWindowDisplayAffinity(hwnd, &mut value) } == 0 {
        return Err("无法读取采集窗口可见性".into());
    }
    Ok(value)
}
fn cloak(hwnd: HWND) -> Result<u32, String> {
    let mut value = 0u32;
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut value as *mut u32).cast(),
            4,
        )
    } < 0
    {
        return Err("无法读取采集窗口显示状态".into());
    }
    Ok(value & APP_CLOAK)
}
fn set_cloak(hwnd: HWND, value: u32) -> Result<(), String> {
    if unsafe { DwmSetWindowAttribute(hwnd, DWMWA_CLOAK as u32, (&value as *const u32).cast(), 4) }
        < 0
    {
        return Err("无法调整采集窗口显示状态".into());
    }
    Ok(())
}
fn flush() -> Result<(), String> {
    if unsafe { DwmFlush() } < 0 {
        Err("采集窗口画面尚未就绪".into())
    } else {
        Ok(())
    }
}
fn input_style(hwnd: HWND) -> Result<isize, String> {
    unsafe {
        SetLastError(0);
    }
    let value = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    if value == 0 && unsafe { GetLastError() } != 0 {
        return Err("无法读取采集窗口输入状态".into());
    }
    Ok(value)
}
fn set_input(hwnd: HWND, bits: isize) -> Result<(), String> {
    let current = input_style(hwnd)?;
    let next = (current & !INPUT_BITS) | (bits & INPUT_BITS);
    if next == current {
        return Ok(());
    }
    unsafe {
        SetLastError(0);
    }
    if unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next) } == 0 && unsafe { GetLastError() } != 0
    {
        return Err("无法调整采集窗口输入状态".into());
    }
    if unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    } == 0
        || input_style(hwnd)? & INPUT_BITS != bits & INPUT_BITS
    {
        return Err("无法确认采集窗口输入状态".into());
    }
    Ok(())
}

/// GetWindowRgn's ERROR is ambiguous (no region OR failure). We never turn it
/// into an assumed rectangular region: leave that HRGN untouched and cloak the
/// whole overlay instead. LastError is an additional rejection, not proof of None.
fn region(hwnd: HWND) -> Result<Option<Region>, String> {
    let region = Region::empty()?;
    unsafe {
        SetLastError(0);
    }
    if unsafe { GetWindowRgn(hwnd, region.raw()) } != 0 {
        return Ok(Some(region));
    }
    if unsafe { GetLastError() } != 0 {
        return Err("无法读取采集窗口区域".into());
    }
    Ok(None)
}

pub(crate) struct Lease {
    hwnd: usize,
    thread: u32,
    cookie: Vec<u16>,
    shared: bool,
    before_input: isize,
    before_affinity: u32,
    before_cloak: u32,
    before_bounds: Rect,
    before_region: Option<Region>,
    applied_region: Option<Region>,
    region_restoring: bool,
    input_changed: bool,
    affinity_changed: bool,
    cloak_changed: bool,
    prepared: bool,
    ready: bool,
    roi: Option<Rect>,
    finished: bool,
}
impl Lease {
    /// Must be called on the actual window's message thread.
    pub fn capture(hwnd: usize, shared: bool) -> Result<Self, String> {
        let raw = hwnd as HWND;
        let mut owner = 0;
        let thread = unsafe { GetWindowThreadProcessId(raw, &mut owner) };
        if unsafe { IsWindow(raw) } == 0
            || owner != unsafe { GetCurrentProcessId() }
            || thread != unsafe { GetCurrentThreadId() }
        {
            return Err("采集窗口身份已改变".into());
        }
        let before_affinity = affinity(raw)?;
        if shared && before_affinity != 0 {
            return Err("共享采集窗口仍受保护，请重新截图".into());
        }
        let before_input = input_style(raw)? & INPUT_BITS;
        let before_cloak = cloak(raw)?;
        let bounds = window_rect(raw)?;
        let before_bounds = Rect {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        let before_region = if shared { region(raw)? } else { None };
        let cookie: Vec<_> = format!("Mewu-Live-Capture-{}", uuid::Uuid::new_v4())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        if unsafe { SetPropW(raw, cookie.as_ptr(), 1usize as _) } == 0 {
            return Err("无法绑定采集窗口".into());
        }
        Ok(Self {
            hwnd,
            thread,
            cookie,
            shared,
            before_input,
            before_affinity,
            before_cloak,
            before_bounds,
            before_region,
            applied_region: None,
            region_restoring: false,
            input_changed: false,
            affinity_changed: false,
            cloak_changed: false,
            prepared: false,
            ready: false,
            roi: None,
            finished: false,
        })
    }
    fn current(&self) -> bool {
        unsafe {
            GetCurrentThreadId() == self.thread
                && IsWindow(self.hwnd as HWND) != 0
                && GetPropW(self.hwnd as HWND, self.cookie.as_ptr()) as usize == 1
        }
    }
    pub fn prepare(&mut self, roi: Rect, whole_window: bool) -> Result<(), String> {
        if !self.current() || self.finished || self.prepared {
            return Err("采集窗口身份或阶段已改变".into());
        }
        self.prepared = true;
        if !self.before_bounds.contains(roi) {
            return Err("采集区域与窗口位置不一致".into());
        }
        let hwnd = self.hwnd as HWND;
        let bounds = window_rect(hwnd)?;
        if self.before_bounds
            != (Rect {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            })
        {
            return Err("采集窗口位置已改变".into());
        }
        if self.shared {
            if affinity(hwnd)? != 0 {
                return Err("共享采集窗口状态已改变".into());
            }
            if !whole_window {
                if let Some(original) = &self.before_region {
                    let bounds = window_rect(hwnd)?;
                    let local = Rect::new(
                        i32::try_from(i64::from(roi.left) - i64::from(bounds.left))
                            .map_err(|_| "采集区域超出窗口")?,
                        i32::try_from(i64::from(roi.top) - i64::from(bounds.top))
                            .map_err(|_| "采集区域超出窗口")?,
                        (i64::from(roi.right) - i64::from(roi.left)) as u32,
                        (i64::from(roi.bottom) - i64::from(roi.top)) as u32,
                    )?;
                    let hole = Region(unsafe {
                        CreateRectRgn(local.left, local.top, local.right, local.bottom)
                    } as usize);
                    if hole.0 == 0 {
                        return Err("无法准备实时采集区域".into());
                    }
                    let visible = Region::empty()?;
                    if unsafe { CombineRgn(visible.raw(), original.raw(), hole.raw(), RGN_DIFF) }
                        == 0
                    {
                        return Err("无法准备实时采集区域".into());
                    }
                    let expected = visible.copy()?;
                    visible.assign(hwnd)?;
                    self.applied_region = Some(expected);
                }
            }
            if whole_window || self.applied_region.is_none() {
                // No explicit HRGN is modified on this fallback path.
                self.cloak_changed = true;
                set_cloak(hwnd, APP_CLOAK)?;
            }
        } else if self.before_affinity != 0x11 {
            self.affinity_changed = true;
            if unsafe { SetWindowDisplayAffinity(hwnd, 0x11) } == 0 || affinity(hwnd)? != 0x11 {
                return Err("无法排除采集覆盖层".into());
            }
        }
        self.input_changed = true;
        set_input(hwnd, INPUT_BITS)?;
        flush()?;
        self.verify_native(roi)?;
        self.ready = true;
        self.roi = Some(roi);
        Ok(())
    }
    pub fn verify(&self, roi: Rect) -> Result<(), String> {
        if !self.ready || self.finished || self.roi != Some(roi) {
            return Err("采集窗口尚未准备完成".into());
        }
        self.verify_native(roi)
    }
    fn verify_native(&self, roi: Rect) -> Result<(), String> {
        if !self.current() {
            return Err("采集窗口身份已改变".into());
        }
        let hwnd = self.hwnd as HWND;
        let bounds = window_rect(hwnd)?;
        if self.before_bounds
            != (Rect {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            })
        {
            return Err("采集窗口位置已改变".into());
        }
        if input_style(hwnd)? & INPUT_BITS != INPUT_BITS {
            return Err("采集窗口输入状态已改变".into());
        }
        if self.shared {
            if affinity(hwnd)? != 0 {
                return Err("共享采集窗口状态已改变".into());
            }
            if let Some(expected) = &self.applied_region {
                let actual = region(hwnd)?.ok_or("实时采集窗口区域已改变")?;
                if unsafe { EqualRgn(actual.raw(), expected.raw()) } == 0 {
                    return Err("实时采集窗口区域已改变".into());
                }
                let bounds = window_rect(hwnd)?;
                let relative = |value: i32, origin: i32| {
                    i32::try_from(i64::from(value) - i64::from(origin))
                        .map_err(|_| "采集区域超出窗口")
                };
                let hole = Region(unsafe {
                    CreateRectRgn(
                        relative(roi.left, bounds.left)?,
                        relative(roi.top, bounds.top)?,
                        relative(roi.right, bounds.left)?,
                        relative(roi.bottom, bounds.top)?,
                    )
                } as usize);
                let intersection = Region::empty()?;
                if hole.0 == 0
                    || unsafe { CombineRgn(intersection.raw(), actual.raw(), hole.raw(), 1) } != 1
                {
                    return Err("实时采集区域仍被覆盖".into());
                }
            } else if cloak(hwnd)? != APP_CLOAK {
                return Err("实时采集窗口尚未隐藏".into());
            }
        } else if affinity(hwnd)? != 0x11 {
            return Err("采集窗口防捕获已改变".into());
        }
        Ok(())
    }
    pub fn restore(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        if unsafe { GetCurrentThreadId() } != self.thread {
            return Err("采集窗口恢复线程无效".into());
        }
        if !self.current() {
            // Destroyed/replaced HWNDs no longer contain our overlay. Never
            // mutate the new window, even if the label or numeric handle matches.
            if unsafe { IsWindow(self.hwnd as HWND) } == 0 {
                self.finished = true;
                return Ok(());
            }
            return Err("采集窗口身份已改变，未修改替代窗口".into());
        }
        let hwnd = self.hwnd as HWND;
        self.restore_region(region)?;
        if self.cloak_changed {
            set_cloak(hwnd, self.before_cloak)?;
            flush()?;
            if cloak(hwnd)? != self.before_cloak {
                return Err("采集窗口未能恢复显示".into());
            }
            self.cloak_changed = false;
        }
        if self.input_changed {
            set_input(hwnd, self.before_input)?;
            self.input_changed = false;
        }
        if self.affinity_changed {
            if unsafe { SetWindowDisplayAffinity(hwnd, self.before_affinity) } == 0
                || affinity(hwnd)? != self.before_affinity
            {
                return Err("采集窗口未能恢复可见性".into());
            }
            self.affinity_changed = false;
        }
        flush()?;
        unsafe {
            RemovePropW(hwnd, self.cookie.as_ptr());
        }
        self.finished = true;
        Ok(())
    }
    fn restore_region(
        &mut self,
        readback: impl FnOnce(HWND) -> Result<Option<Region>, String>,
    ) -> Result<(), String> {
        if !self.current() {
            return Err("采集窗口身份已改变，无法恢复区域".into());
        }
        if let Some(expected) = &self.applied_region {
            let hwnd = self.hwnd as HWND;
            let actual = region(hwnd)?.ok_or("采集窗口区域已改变，无法恢复")?;
            let original = self.before_region.as_ref().ok_or("原采集窗口区域缺失")?;
            let was_applied = unsafe { EqualRgn(actual.raw(), expected.raw()) } != 0;
            let already_restored =
                self.region_restoring && unsafe { EqualRgn(actual.raw(), original.raw()) } != 0;
            if !was_applied && !already_restored {
                return Err("采集窗口区域已改变，无法恢复".into());
            }
            if !already_restored {
                original.copy()?.assign(hwnd)?;
                // Successful native mutation is remembered BEFORE fallible readback.
                self.region_restoring = true;
            }
            let restored = readback(hwnd)?.ok_or("采集窗口原区域未恢复")?;
            if unsafe { EqualRgn(original.raw(), restored.raw()) } == 0 {
                return Err("采集窗口原区域未恢复".into());
            }
            self.applied_region = None;
            self.region_restoring = false;
        }
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Position once, then read back the actual native rectangle. No invisible
/// clamp, drag or shadow may put this owned HUD over a shared acquisition ROI.
pub(crate) fn place_hud(
    window: &tauri::WebviewWindow,
    roi: Rect,
    work: Rect,
    f8: bool,
) -> Result<bool, String> {
    let size = window.outer_size().map_err(|_| "无法读取采集控制条大小")?;
    if let Some((x, y)) = hud_position(work, roi, (size.width, size.height)) {
        window
            .set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|_| "无法放置采集控制条")?;
        let hwnd = window.hwnd().map_err(|_| "采集控制条身份已改变")?.0;
        let actual = window_rect(hwnd)?;
        let actual = Rect {
            left: actual.left,
            top: actual.top,
            right: actual.right,
            bottom: actual.bottom,
        };
        if !work.contains(actual) || roi.intersects(actual) || affinity(hwnd)? != 0 {
            return Err("采集控制条无法避开选区".into());
        }
        Ok(true)
    } else if f8 {
        Ok(false)
    } else {
        Err("F8 被占用，无法开始全屏采集".into())
    }
}
pub(crate) fn work_area(window: &tauri::WebviewWindow, display: Rect) -> Result<Rect, String> {
    let monitor = window
        .available_monitors()
        .map_err(|_| "无法读取采集显示器")?
        .into_iter()
        .find(|monitor| {
            monitor.position().x == display.left
                && monitor.position().y == display.top
                && i64::from(monitor.size().width)
                    == i64::from(display.right) - i64::from(display.left)
                && i64::from(monitor.size().height)
                    == i64::from(display.bottom) - i64::from(display.top)
        })
        .ok_or("显示器已改变，请重新截图")?;
    let work = monitor.work_area();
    Rect::new(
        work.position.x,
        work.position.y,
        work.size.width,
        work.size.height,
    )
}

/// Both the overlay and its one HUD are bound while on their message thread.
pub(crate) struct Session {
    pub main: Lease,
    hud: Option<(tauri::WebviewWindow, Lease)>,
}
impl Session {
    pub fn capture(window: &tauri::WebviewWindow, shared: bool) -> Result<Self, String> {
        Ok(Self {
            main: Lease::capture(
                window.hwnd().map_err(|_| "采集窗口不存在")?.0 as usize,
                shared,
            )?,
            hud: None,
        })
    }
    pub fn attach_hud(&mut self, window: &tauri::WebviewWindow) -> Result<(), String> {
        if self.hud.is_some() {
            return Err("采集控制条已存在".into());
        }
        match window
            .hwnd()
            .map_err(|_| "采集控制条不存在".to_string())
            .and_then(|hwnd| Lease::capture(hwnd.0 as usize, self.main.shared))
        {
            Ok(lease) => {
                self.hud = Some((window.clone(), lease));
                Ok(())
            }
            Err(error) => {
                let _ = window.destroy();
                Err(error)
            }
        }
    }
    fn restore(&mut self) -> Result<(), String> {
        let hud = (|| {
            if let Some((window, lease)) = &mut self.hud {
                if lease.current() {
                    window.destroy().map_err(|_| "无法关闭采集控制条")?;
                } else if unsafe { IsWindow(lease.hwnd as HWND) } != 0 {
                    return Err("采集控制条身份已改变，未修改替代窗口".into());
                }
                lease.restore()?;
                self.hud = None;
            }
            Ok(())
        })();
        let main = self.main.restore();
        hud.and(main)
    }
    pub fn verify(&self) -> Result<(), String> {
        let roi = self.main.roi.ok_or("采集窗口尚未准备完成")?;
        self.main.verify(roi)?;
        if let Some((_, lease)) = &self.hud {
            if !lease.current() {
                return Err("采集控制条身份已改变".into());
            }
            if self.main.shared
                && unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(lease.hwnd as HWND)
                } != 0
            {
                let bounds = window_rect(lease.hwnd as HWND)?;
                let bounds = Rect {
                    left: bounds.left,
                    top: bounds.top,
                    right: bounds.right,
                    bottom: bounds.bottom,
                };
                if roi.intersects(bounds) || affinity(lease.hwnd as HWND)? != 0 {
                    return Err("采集控制条遮挡选区".into());
                }
            }
        }
        Ok(())
    }
}
/// The blocking capture worker asks the original HWND's message thread for a
/// fresh read-only check. Recording uses this at startup and at 100ms supervisor
/// intervals; this is not an atomic per-frame proof. A failure terminates the
/// owned capture worker through its existing supervisor. Late callbacks only read.
pub(crate) fn check(
    app: &tauri::AppHandle,
    read: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = send.send(read());
    })
    .map_err(|_| "采集窗口校验中断")?;
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "采集窗口校验超时")?
}
static RECOVERY: Mutex<Vec<Session>> = Mutex::new(Vec::new());
static RECOVERING: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn pending() -> bool {
    RECOVERY.lock().map_or(true, |value| {
        !value.is_empty() || RECOVERING.load(Ordering::Acquire) != 0
    })
}
pub(crate) fn finish(mut session: Session) -> Result<(), String> {
    match session.restore() {
        Ok(()) => Ok(()),
        Err(error) => {
            defer(session)?;
            Err(error)
        }
    }
}
/// Queue transfer only: a failed/discarded UI dispatch must not drop its lease
/// on an async worker and lose the native cleanup obligation.
pub(crate) fn defer(session: Session) -> Result<(), String> {
    RECOVERY
        .lock()
        .map_err(|_| "采集窗口恢复状态不可用")?
        .push(session);
    Ok(())
}
/// Explicit restoration debt prevents a failed cleanup from being reported as
/// idle for new work. Retry uses the original HWND/cookie, never a label.
/// Pure display debt does not replace the worker/data gates on final exit.
pub(crate) fn recover_current(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    // A queued restore may arrive after irreversible exit preparation. It must
    // not mutate the old HWNDs once normal OS exit/restart owns their cleanup.
    if app.state::<crate::Host>().exit.is_ready() {
        return Err("应用正在退出".into());
    }
    recover_with(|| {})
}
fn recover_with(after_take: impl FnOnce()) -> Result<(), String> {
    struct Recovering;
    impl Drop for Recovering {
        fn drop(&mut self) {
            RECOVERING.fetch_sub(1, Ordering::Release);
        }
    }
    let sessions = {
        let mut queue = RECOVERY.lock().map_err(|_| "采集窗口恢复状态不可用")?;
        if RECOVERING.load(Ordering::Acquire) != 0 {
            return Err("采集窗口正在恢复".into());
        }
        // Publish in-flight state BEFORE the queue becomes empty. The mutex is
        // released for all native calls; synchronous callbacks still see busy.
        RECOVERING.fetch_add(1, Ordering::AcqRel);
        std::mem::take(&mut *queue)
    };
    let _recovering = Recovering;
    after_take();
    let mut error = None;
    for session in sessions {
        if let Err(cause) = finish(session) {
            error = Some(cause);
        }
    }
    error.map_or(Ok(()), Err)
}
/// Only after lifecycle READY, where workers, durable saves and all reversible
/// steps have settled. Normal process exit/restart destroys these HWNDs in the
/// OS; this function changes only our in-memory ownership, never a window.
pub(crate) fn abandon_for_process_exit(exit: &crate::lifecycle::ExitState) -> Result<(), String> {
    if !exit.is_ready() {
        return Err("尚未完成退出准备".into());
    }
    let mut queue = RECOVERY.lock().map_err(|_| "采集窗口恢复状态不可用")?;
    if RECOVERING.load(Ordering::Acquire) != 0 {
        return Err("采集窗口恢复尚未结束".into());
    }
    for mut session in queue.drain(..) {
        session.main.finished = true;
        if let Some((_, lease)) = &mut session.hud {
            lease.finished = true;
        }
    }
    Ok(())
}
pub(crate) async fn recover(app: &tauri::AppHandle) -> Result<(), String> {
    if !pending() {
        return Ok(());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = app.clone();
    app.run_on_main_thread(move || {
        let _ = send.send(recover_current(&target));
    })
    .map_err(|_| "无法恢复采集窗口")?;
    receive.await.map_err(|_| "采集窗口恢复中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_EX_LAYERED, WS_POPUP,
    };
    // Only tests that mutate the global recovery queue share this lock. Each
    // fixture's HWND must be recovered by its own test/message thread.
    static RECOVERY_TEST: Mutex<()> = Mutex::new(());
    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_POPUP,
                    -2000,
                    -100,
                    800,
                    600,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null());
            Self(hwnd)
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    fn roi() -> Rect {
        Rect::new(-1900, 0, 300, 200).unwrap()
    }
    #[test]
    fn shared_hud_stays_outside_roi_and_fullscreen_requires_stop_hotkey() {
        let work = Rect::new(-2000, -100, 800, 600).unwrap();
        let point = hud_position(work, roi(), (176, 44)).unwrap();
        assert!(!roi().intersects(Rect::new(point.0, point.1, 176, 44).unwrap()));
        assert_eq!(hud_position(work, work, (176, 44)), None);
        assert_eq!(hud_position(work, roi(), (900, 44)), None);
        assert!(Rect::new(i32::MAX, 0, 1, 1).is_err());
    }
    #[test]
    fn shared_native_no_explicit_region_cloaks_without_rewriting_region_or_affinity() {
        let window = Window::new();
        assert!(region(window.0).unwrap().is_none());
        let before = input_style(window.0).unwrap();
        let old_cloak = cloak(window.0).unwrap();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        lease.prepare(roi(), false).unwrap();
        assert!(region(window.0).unwrap().is_none());
        assert_eq!(affinity(window.0).unwrap(), 0);
        lease.restore().unwrap();
        assert!(region(window.0).unwrap().is_none());
        assert_eq!(cloak(window.0).unwrap(), old_cloak);
        assert_eq!(input_style(window.0).unwrap(), before);
    }
    #[test]
    fn shared_native_empty_and_complex_regions_restore_exactly_after_callback() {
        for complex in [false, true] {
            let window = Window::new();
            let original = Region::empty().unwrap();
            if complex {
                let a = Region(unsafe { CreateRectRgn(0, 0, 800, 500) } as usize);
                let b = Region(unsafe { CreateRectRgn(250, 250, 300, 500) } as usize);
                assert_ne!(
                    unsafe { CombineRgn(original.raw(), a.raw(), b.raw(), RGN_DIFF) },
                    0
                );
            }
            original.copy().unwrap().assign(window.0).unwrap();
            let mut lease = Lease::capture(window.0 as usize, true).unwrap();
            lease.prepare(roi(), false).unwrap();
            // This is the capture callback admission: native clear proof first.
            lease.verify(roi()).unwrap();
            assert_eq!(affinity(window.0).unwrap(), 0);
            lease.restore().unwrap();
            let restored = region(window.0).unwrap().unwrap();
            assert_ne!(unsafe { EqualRgn(original.raw(), restored.raw()) }, 0);
            lease.restore().unwrap();
        }
    }
    #[test]
    fn shared_native_identity_loss_never_restores_a_replacement_window() {
        let window = Window::new();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        unsafe {
            RemovePropW(window.0, lease.cookie.as_ptr());
        }
        assert!(lease.prepare(roi(), false).is_err());
        let before = input_style(window.0).unwrap();
        assert!(lease.restore().is_err());
        assert_eq!(input_style(window.0).unwrap(), before);
        assert!(Lease::capture(0, true).is_err());
    }
    #[test]
    fn shared_native_capture_callback_requires_preparation_and_stops_after_restore() {
        let window = Window::new();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        assert!(lease.verify(roi()).is_err());
        lease.prepare(roi(), false).unwrap();
        lease.verify(roi()).unwrap();
        assert!(lease.prepare(roi(), false).is_err());
        lease.restore().unwrap();
        assert!(lease.verify(roi()).is_err());
    }
    #[test]
    fn shared_native_foreign_region_change_is_rejected_without_overwriting_it() {
        let window = Window::new();
        Region(unsafe { CreateRectRgn(0, 0, 800, 600) } as usize)
            .assign(window.0)
            .unwrap();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        lease.prepare(roi(), false).unwrap();
        Region::empty().unwrap().assign(window.0).unwrap();
        assert!(lease.verify(roi()).is_err());
        assert!(lease.restore().is_err());
        let actual = region(window.0).unwrap().unwrap();
        let empty = Region::empty().unwrap();
        assert_ne!(unsafe { EqualRgn(actual.raw(), empty.raw()) }, 0);
    }
    #[test]
    fn shared_native_failed_region_bounds_restore_and_protected_mode_keeps_its_policy() {
        let window = Window::new();
        let original_style = input_style(window.0).unwrap();
        let mut failed = Lease::capture(window.0 as usize, true).unwrap();
        assert!(failed
            .prepare(Rect::new(5000, 5000, 20, 20).unwrap(), false)
            .is_err());
        failed.restore().unwrap();
        assert_eq!(input_style(window.0).unwrap(), original_style);
        let mut protected = Lease::capture(window.0 as usize, false).unwrap();
        protected.prepare(roi(), false).unwrap();
        assert_eq!(affinity(window.0).unwrap(), 0x11);
        protected.restore().unwrap();
        assert_eq!(affinity(window.0).unwrap(), 0);
        assert_eq!(input_style(window.0).unwrap(), original_style);
    }
    #[test]
    fn discarded_ui_dispatch_defers_lease_without_worker_drop_and_recovers_on_owner_thread() {
        let _serial = RECOVERY_TEST.lock().unwrap();
        let window = Window::new();
        let before_input = input_style(window.0).unwrap();
        let before_cloak = cloak(window.0).unwrap();
        let mut session = Session {
            main: Lease::capture(window.0 as usize, true).unwrap(),
            hud: None,
        };
        session.main.prepare(roi(), false).unwrap();
        // Model the exact residual transfer after a rejected/discarded UI
        // dispatch. The async worker can enqueue, but cannot restore this HWND.
        std::thread::spawn(move || defer(session))
            .join()
            .unwrap()
            .unwrap();
        assert!(pending());
        assert_eq!(cloak(window.0).unwrap(), APP_CLOAK);
        assert_eq!(input_style(window.0).unwrap() & INPUT_BITS, INPUT_BITS);
        recover_with(|| {
            // Model synchronous window callback/reentrant admission precisely in
            // the interval when the queue is empty and native restore is pending.
            assert!(pending());
            assert!(recover_with(|| {}).is_err());
            assert!(pending());
        })
        .unwrap();
        assert!(!pending());
        assert_eq!(input_style(window.0).unwrap(), before_input);
        assert_eq!(cloak(window.0).unwrap(), before_cloak);
        assert!(region(window.0).unwrap().is_none());
    }
    #[test]
    fn original_region_assign_then_failed_readback_retries_exact_original_only() {
        let window = Window::new();
        let original = Region(unsafe { CreateRectRgn(0, 0, 800, 600) } as usize);
        original.copy().unwrap().assign(window.0).unwrap();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        lease.prepare(roi(), false).unwrap();
        assert!(lease
            .restore_region(|_| Err("synthetic readback failure".into()))
            .is_err());
        assert!(lease.region_restoring);
        assert_ne!(
            unsafe { EqualRgn(region(window.0).unwrap().unwrap().raw(), original.raw()) },
            0
        );
        // Even in this phase a foreign region is still rejected and preserved.
        Region::empty().unwrap().assign(window.0).unwrap();
        assert!(lease.restore().is_err());
        let empty = Region::empty().unwrap();
        assert_ne!(
            unsafe { EqualRgn(region(window.0).unwrap().unwrap().raw(), empty.raw()) },
            0
        );
        // Restore the exact expected original in this isolated fixture; retry
        // may then finish the remaining input/affinity/cookie cleanup.
        original.copy().unwrap().assign(window.0).unwrap();
        lease.restore().unwrap();
        assert!(!lease.region_restoring);
        assert_ne!(
            unsafe { EqualRgn(region(window.0).unwrap().unwrap().raw(), original.raw()) },
            0
        );
    }
    #[test]
    fn presentation_debt_cannot_be_abandoned_before_ready_and_remains_recoverable() {
        let _serial = RECOVERY_TEST.lock().unwrap();
        let window = Window::new();
        let before_input = input_style(window.0).unwrap();
        let before_cloak = cloak(window.0).unwrap();
        let mut session = Session {
            main: Lease::capture(window.0 as usize, true).unwrap(),
            hud: None,
        };
        session.main.prepare(roi(), false).unwrap();
        defer(session).unwrap();
        let exit = crate::lifecycle::ExitState::default();
        assert!(abandon_for_process_exit(&exit).is_err());
        assert!(pending());
        assert_eq!(cloak(window.0).unwrap(), APP_CLOAK);
        recover_with(|| {}).unwrap();
        assert!(!pending());
        assert_eq!(input_style(window.0).unwrap(), before_input);
        assert_eq!(cloak(window.0).unwrap(), before_cloak);
    }
}
