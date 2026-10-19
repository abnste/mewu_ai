// SPDX-License-Identifier: MPL-2.0
//! A capture lease for an owned HWND. Shared capture never enables WDA protection.
//! Mutations and restoration run on the HWND's message thread, before sampling and
//! after its worker has drained. Do not restore a replacement selected by label.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    LazyLock, Mutex,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, RECT},
    Graphics::{
        Dwm::{DwmFlush, DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_CLOAKED,
            DWMWA_TRANSITIONS_FORCEDISABLED},
        Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn, SetWindowRgn, HRGN,
            RGN_COPY, RGN_DIFF, RGN_OR,
        },
    },
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        GetPropW, GetWindow, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsWindow, IsWindowVisible, RemovePropW, SetPropW, SetWindowDisplayAffinity,
        SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, WS_CHILD, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
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

// Match Tao's ignore-cursor-events policy. TRANSPARENT alone only affects paint
// ordering; on a layered window it also passes mouse events to windows beneath.
// All three original bits are captured and restored without changing opacity.
const INPUT_BITS: isize = (WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) as isize;
const APP_CLOAK: u32 = 1;
// One exact native owner per HWND. This also prevents a geometry publication
// from replacing an active session's live aperture or outstanding restore debt.
static LEASE_OWNER_KEY: LazyLock<Vec<u16>> = LazyLock::new(||
    "Mewu-Live-Capture-Owner".encode_utf16().chain(Some(0)).collect());

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
        (x, roi.top.saturating_sub(h).saturating_sub(6)),
        (roi.right.saturating_add(6), y),
        (roi.left.saturating_sub(w).saturating_sub(6), y),
        (x, roi.bottom.saturating_add(6)),
        (x, work.top.saturating_add(6)),
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

struct ScrollClip { bounds: Rect, roi: Rect, expected: Region }

/// A long-capture HUD may span the source, but its real HWND region must not.
/// The sole WebView paints the outside dimming, outline, preview and controls. Removing
/// the acquired rectangle at the native boundary also removes its input target;
/// CSS transparency is deliberately not used as proof of clean capture pixels.
fn scroll_region(bounds: Rect, roi: Rect, shapes: &[Rect]) -> Result<Region, String> {
    if !bounds.contains(roi) || shapes.len() > 3 { return Err("长截图显示区域无效".into()); }
    let visible = Region::empty()?;
    for shape in shapes {
        let left = shape.left.max(bounds.left); let top = shape.top.max(bounds.top);
        let right = shape.right.min(bounds.right); let bottom = shape.bottom.min(bounds.bottom);
        if right <= left || bottom <= top { continue; }
        let local = |value: i32, origin: i32| i32::try_from(i64::from(value)-i64::from(origin)).map_err(|_| "长截图显示坐标无效");
        let piece = Region(unsafe { CreateRectRgn(local(left,bounds.left)?,local(top,bounds.top)?,local(right,bounds.left)?,local(bottom,bounds.top)?) } as usize);
        if piece.0 == 0 || unsafe { CombineRgn(visible.raw(),visible.raw(),piece.raw(),RGN_OR) } == 0 {
            return Err("无法准备长截图显示区域".into());
        }
    }
    let local = |value: i32, origin: i32| i32::try_from(i64::from(value)-i64::from(origin)).map_err(|_| "长截图采集坐标无效");
    let hole = Region(unsafe { CreateRectRgn(local(roi.left,bounds.left)?,local(roi.top,bounds.top)?,local(roi.right,bounds.left)?,local(roi.bottom,bounds.top)?) } as usize);
    if hole.0 == 0 || unsafe { CombineRgn(visible.raw(),visible.raw(),hole.raw(),RGN_DIFF) } == 0 {
        return Err("无法留出长截图采集区域".into());
    }
    Ok(visible)
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
    // Only disposable compound HUDs use this explicit native aperture.
    // The main capture lease retains its original region/cloak fallback.
    scroll_clip: Option<ScrollClip>,
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
        if !unsafe { GetPropW(raw, LEASE_OWNER_KEY.as_ptr()) }.is_null() {
            return Err("采集窗口仍由上一项操作持有".into());
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
        if unsafe { SetPropW(raw, LEASE_OWNER_KEY.as_ptr(), cookie.as_ptr() as _) } == 0 {
            unsafe { RemovePropW(raw, cookie.as_ptr()); }
            return Err("无法绑定采集窗口所有者".into());
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
            scroll_clip: None,
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
                && GetPropW(self.hwnd as HWND, LEASE_OWNER_KEY.as_ptr()) as usize == self.cookie.as_ptr() as usize
        }
    }
    /// Only a newly attached, still-hidden owned HUD may disable its DWM show
    /// transition. A composed show animation can move the otherwise safe native
    /// aperture through the sampled pixels; DwmFlush is not an animation fence.
    fn disable_hud_transitions_before_show(&self) -> Result<(), String> {
        let hwnd = self.hwnd as HWND;
        if !self.current() || self.finished || self.prepared || self.scroll_clip.is_some()
            || self.input_changed || self.affinity_changed || self.cloak_changed
            || unsafe { IsWindowVisible(hwnd) } != 0 {
            return Err("采集控件身份或显示阶段已改变".into());
        }
        let disabled = 1i32;
        let result = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            (&disabled as *const i32).cast(), std::mem::size_of::<i32>() as u32) };
        if result < 0 {
            return Err("无法关闭采集控件的窗口动画".into());
        }
        // This attribute is documented for Set, not a promised Get readback.
        if !self.current() || unsafe { IsWindowVisible(hwnd) } != 0 {
            return Err("采集控件显示阶段已改变".into());
        }
        Ok(())
    }
    /// Pin the chosen top-level relationship. Retained overlays own their HUD;
    /// cloaked/protected fallbacks keep it unowned to avoid inherited cloaking.
    fn verify_hud_owner(&self, hud: &Lease, retained_owner: bool) -> Result<(), String> {
        if self.finished || hud.finished || !self.current() || !hud.current() {
            return Err("采集窗口或控制条身份已改变".into());
        }
        let hwnd = hud.hwnd as HWND;
        let expected = if retained_owner { self.hwnd as HWND } else { std::ptr::null_mut() };
        if unsafe { GetWindow(hwnd, GW_OWNER) } != expected
            || unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } & WS_CHILD as isize != 0 {
            return Err("采集控制条的所属窗口已改变".into());
        }
        if retained_owner && cloak(self.hwnd as HWND)? != 0 {
            return Err("采集控制条不能继承已隐藏窗口".into());
        }
        Ok(())
    }
    /// Author a known space geometry, never infer one from GetWindowRgn ERROR.
    /// This temporary owner is deliberately not a prepared sampling lease.
    fn author_full_region(&self, expected_bounds: Rect) -> Result<bool, String> {
        if !self.current() || self.finished || self.prepared || self.scroll_clip.is_some() {
            return Err("空间窗口身份或阶段已改变".into());
        }
        let hwnd = self.hwnd as HWND;
        let actual = window_rect(hwnd)?;
        if expected_bounds != (Rect { left: actual.left, top: actual.top, right: actual.right, bottom: actual.bottom }) {
            return Err("空间窗口位置尚未提交".into());
        }
        let width = i32::try_from(i64::from(expected_bounds.right) - i64::from(expected_bounds.left)).map_err(|_| "空间窗口宽度无效")?;
        let height = i32::try_from(i64::from(expected_bounds.bottom) - i64::from(expected_bounds.top)).map_err(|_| "空间窗口高度无效")?;
        if width <= 0 || height <= 0 { return Err("空间窗口尺寸无效".into()); }
        let full = Region(unsafe { CreateRectRgn(0, 0, width, height) } as usize);
        if full.0 == 0 { return Err("无法建立空间窗口区域".into()); }
        let unchanged = region(hwnd)?.is_some_and(|actual| unsafe { EqualRgn(actual.raw(), full.raw()) } != 0);
        if !unchanged { full.copy()?.assign(hwnd)?; }
        let actual_bounds = window_rect(hwnd)?;
        let actual = region(hwnd)?.ok_or("空间窗口区域未能确认")?;
        if !self.current() || expected_bounds != (Rect { left: actual_bounds.left, top: actual_bounds.top, right: actual_bounds.right, bottom: actual_bounds.bottom })
            || unsafe { EqualRgn(actual.raw(), full.raw()) } == 0 {
            return Err("空间窗口区域已改变".into());
        }
        Ok(!unchanged)
    }
    fn release_author_identity(&mut self) -> Result<(), String> {
        if !self.current() || self.prepared || self.applied_region.is_some() || self.scroll_clip.is_some()
            || self.input_changed || self.affinity_changed || self.cloak_changed {
            return Err("空间窗口所有权已改变".into());
        }
        unsafe {
            RemovePropW(self.hwnd as HWND, self.cookie.as_ptr());
            RemovePropW(self.hwnd as HWND, LEASE_OWNER_KEY.as_ptr());
        }
        self.finished = true;
        Ok(())
    }
    fn verify_staged(&self, roi: Rect) -> Result<(), String> {
        if !self.current() || self.finished || self.prepared || !self.before_bounds.contains(roi) {
            return Err("采集窗口身份或阶段已改变".into());
        }
        let native = window_rect(self.hwnd as HWND)?;
        if self.before_bounds != (Rect { left: native.left, top: native.top, right: native.right, bottom: native.bottom }) {
            return Err("采集窗口位置已改变".into());
        }
        if affinity(self.hwnd as HWND)? != self.before_affinity {
            return Err("采集窗口可见性已改变".into());
        }
        Ok(())
    }
    fn set_scroll_clip(&mut self, roi: Rect, shapes: &[Rect]) -> Result<(), String> {
        if !self.current() || self.finished || self.prepared {
            return Err("长截图控件身份已改变".into());
        }
        let hwnd = self.hwnd as HWND;
        let native = window_rect(hwnd)?;
        let bounds = Rect { left:native.left,top:native.top,right:native.right,bottom:native.bottom };
        if bounds != self.before_bounds { return Err("长截图控件位置已改变".into()); }
        if let Some(clip) = &self.scroll_clip {
            let actual = region(hwnd)?.ok_or("长截图显示区域已改变")?;
            if clip.roi != roi || unsafe { EqualRgn(actual.raw(),clip.expected.raw()) } == 0 {
                return Err("长截图显示区域已改变".into());
            }
        }
        let expected = scroll_region(bounds,roi,shapes)?;
        if self.scroll_clip.as_ref().is_some_and(|clip| unsafe { EqualRgn(clip.expected.raw(),expected.raw()) } != 0) {
            // Once the preview reaches its cap, new image pixels do not require
            // another SetWindowRgn/DwmFlush or native window repaint.
            return self.verify_scroll_clip(roi);
        }
        expected.copy()?.assign(hwnd)?;
        // Remember successful native mutation before any fallible readback.
        self.scroll_clip = Some(ScrollClip { bounds,roi,expected });
        flush()?;
        self.verify_scroll_clip(roi)
    }
    fn verify_scroll_clip(&self, roi: Rect) -> Result<(), String> {
        if !self.current() || self.finished { return Err("长截图控件身份已改变".into()); }
        let clip = self.scroll_clip.as_ref().ok_or("长截图显示区域未就绪")?;
        let hwnd = self.hwnd as HWND;
        let native = window_rect(hwnd)?;
        let bounds = Rect { left:native.left,top:native.top,right:native.right,bottom:native.bottom };
        let actual = region(hwnd)?.ok_or("长截图显示区域已改变")?;
        if clip.roi != roi || clip.bounds != bounds || unsafe { EqualRgn(actual.raw(),clip.expected.raw()) } == 0
            || affinity(hwnd)? != if self.shared { 0 } else { 0x11 } {
            return Err("长截图控件无法避开采集区域".into());
        }
        Ok(())
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
            RemovePropW(hwnd, LEASE_OWNER_KEY.as_ptr());
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

fn author_known_space_region(hwnd: usize, expected: Rect) -> Result<bool, String> {
    // Use the same process/thread/native-cookie ownership as live sessions, but
    // deliberately commit the authored geometry rather than restoring ERROR/None.
    let mut owner = Lease::capture(hwnd, false)?;
    let result = owner.author_full_region(expected);
    let released = owner.release_author_identity();
    match result { Err(error) => Err(error), Ok(changed) => released.map(|()| changed) }
}

/// Call only after the owned Space's explicit physical geometry commit. No
/// generic focus/show path may reset a region while capture owns its HWND.
pub(crate) fn author_space_region(window: &tauri::WebviewWindow, expected: Rect) -> Result<bool, String> {
    if window.label() != "space" { return Err("只有空间窗口可以提交截图区域".into()); }
    author_known_space_region(window.hwnd().map_err(|_| "空间窗口不存在")?.0 as usize, expected)
}

/// Both the overlay and its one HUD are bound while on their message thread.
pub(crate) struct Session {
    pub main: Lease,
    hud: Option<(tauri::WebviewWindow, Lease, bool)>,
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
    /// Prediction only: the original explicit native region can preserve the
    /// scene outside a future ROI. A caller must still prepare and verify it.
    pub fn can_retain_main_outside(&self) -> Result<bool, String> {
        self.main.verify_staged(self.main.before_bounds)?;
        if !self.main.shared || self.main.before_cloak != 0 { return Ok(false); }
        let Some(original) = &self.main.before_region else { return Ok(false); };
        let hwnd = self.main.hwnd as HWND;
        if input_style(hwnd)? & INPUT_BITS != self.main.before_input || cloak(hwnd)? != self.main.before_cloak {
            return Err("采集窗口显示状态已改变".into());
        }
        let actual = region(hwnd)?.ok_or("采集窗口原区域已改变")?;
        if unsafe { EqualRgn(original.raw(), actual.raw()) } == 0 { return Err("采集窗口原区域已改变".into()); }
        Ok(true)
    }
    /// Actual proof, not the earlier prediction or a published UI flag.
    pub fn retains_main_outside(&self) -> Result<bool, String> {
        if !self.main.shared || !self.main.prepared || !self.main.ready || self.main.finished
            || self.main.applied_region.is_none() || self.main.cloak_changed || self.main.before_cloak != 0 {
            return Ok(false);
        }
        let Some(roi) = self.main.roi else { return Ok(false); };
        self.main.verify(roi)?;
        Ok(cloak(self.main.hwnd as HWND)? == 0)
    }
    pub fn attach_hud(&mut self, window: &tauri::WebviewWindow, retained_owner: bool) -> Result<(), String> {
        if self.hud.is_some() {
            return Err("采集控制条已存在".into());
        }
        match window
            .hwnd()
            .map_err(|_| "采集控制条不存在".to_string())
            .and_then(|hwnd| {
                let lease = Lease::capture(hwnd.0 as usize, self.main.shared)?;
                self.main.verify_hud_owner(&lease, retained_owner)?;
                if retained_owner && !self.can_retain_main_outside()? {
                    return Err("采集主窗口不能保留控制条层级".into());
                }
                lease.disable_hud_transitions_before_show()?;
                Ok(lease)
            })
        {
            Ok(lease) => {
                self.hud = Some((window.clone(), lease, retained_owner));
                Ok(())
            }
            Err(error) => {
                let _ = window.destroy();
                Err(error)
            }
        }
    }
    /// Presentation ACKs must name this exact owned HWND on its message thread.
    /// A replacement with the same Tauri label must never acknowledge this lease.
    pub fn owns_hud(&self, window: &tauri::WebviewWindow) -> bool {
        !self.main.finished && self.main.current() && self.hud.as_ref().is_some_and(|(_, lease, retained_owner)| {
            self.main.verify_hud_owner(lease, *retained_owner).is_ok()
                && window.hwnd().is_ok_and(|hwnd| hwnd.0 as usize == lease.hwnd)
        })
    }
    pub fn verify_hud_owner(&self) -> Result<(), String> {
        let (_, lease, retained_owner) = self.hud.as_ref().ok_or("采集控件不存在")?;
        self.main.verify_hud_owner(lease, *retained_owner)?;
        if *retained_owner && self.main.prepared && !self.retains_main_outside()? {
            return Err("采集主窗口未保留控制条层级".into());
        }
        Ok(())
    }
    /// Stage only the disposable HUD aperture. The original main window remains
    /// visible and untouched until its frontend has presented the countdown.
    pub fn stage_scroll_hud(&mut self, roi: Rect, shapes: &[Rect]) -> Result<(), String> {
        self.verify_hud_owner()?;
        self.main.verify_staged(roi)?;
        self.hud.as_mut().ok_or("采集控件不存在")?.1.set_scroll_clip(roi, shapes)
    }
    /// Update only an already owned compound HUD after the main is safe to sample.
    pub fn configure_scroll_hud(&mut self, roi: Rect, shapes: &[Rect]) -> Result<(), String> {
        self.verify_hud_owner()?;
        self.main.verify(roi)?;
        self.hud.as_mut().ok_or("长截图控件不存在")?.1.set_scroll_clip(roi,shapes)
    }
    fn restore(&mut self) -> Result<(), String> {
        let hud = (|| {
            if let Some((window, lease, _)) = &mut self.hud {
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
        if let Some((_, lease, _)) = &self.hud {
            self.verify_hud_owner()?;
            if lease.scroll_clip.is_some() {
                lease.verify_scroll_clip(roi)?;
            } else if self.main.shared
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
        if let Some((_, lease, _)) = &mut session.hud {
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
        CreateWindowExW, DestroyWindow, GWLP_HWNDPARENT, WS_EX_NOREDIRECTIONBITMAP,
        WS_EX_TOOLWINDOW, WS_POPUP,
    };
    // Only tests that mutate the global recovery queue share this lock. Each
    // fixture's HWND must be recovered by its own test/message thread.
    static RECOVERY_TEST: Mutex<()> = Mutex::new(());
    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            Self::with_owner(std::ptr::null_mut())
        }
        fn with_owner(owner: HWND) -> Self {
            Self::with_ex_style(owner, WS_EX_LAYERED)
        }
        fn with_ex_style(owner: HWND, style: u32) -> Self {
            let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
            let hwnd = unsafe {
                CreateWindowExW(
                    style,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_POPUP,
                    -2000,
                    -100,
                    800,
                    600,
                    owner,
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
    fn physical_bounds(window: &Window) -> Rect {
        let value = window_rect(window.0).unwrap();
        Rect { left: value.left, top: value.top, right: value.right, bottom: value.bottom }
    }
    fn middle_roi(bounds: Rect) -> Rect {
        let width = u32::try_from(i64::from(bounds.right) - i64::from(bounds.left)).unwrap();
        let height = u32::try_from(i64::from(bounds.bottom) - i64::from(bounds.top)).unwrap();
        Rect::offset(bounds.left, bounds.top, width / 4, height / 4, width / 2, height / 2).unwrap()
    }
    #[test]
    fn hidden_hud_owner_is_the_exact_top_level_main_and_survives_capture_restore() {
        for shared in [false, true] {
            let main = Window::new();
            let hud = Window::with_owner(main.0);
            if shared { author_known_space_region(main.0 as usize, physical_bounds(&main)).unwrap(); }
            let mut main_lease = Lease::capture(main.0 as usize, shared).unwrap();
            let mut hud_lease = Lease::capture(hud.0 as usize, shared).unwrap();
            assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, main.0);
            assert_eq!(unsafe { GetWindowLongPtrW(hud.0, GWL_STYLE) } & WS_CHILD as isize, 0);
            assert_eq!(unsafe { IsWindowVisible(hud.0) }, 0);
            main_lease.verify_hud_owner(&hud_lease, true).unwrap();
            let before = (physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap());
            main_lease.prepare(middle_roi(physical_bounds(&main)), false).unwrap();
            main_lease.verify_hud_owner(&hud_lease, true).unwrap();
            main_lease.restore().unwrap();
            hud_lease.restore().unwrap();
            assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, main.0);
            assert_eq!((physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap()), before);
        }
    }
    #[test]
    fn hidden_hud_owner_rejects_missing_or_wrong_main_without_reparenting() {
        let main = Window::new();
        let other = Window::new();
        let mut main_lease = Lease::capture(main.0 as usize, true).unwrap();
        for owner in [std::ptr::null_mut(), other.0] {
            let hud = Window::with_owner(owner);
            let mut hud_lease = Lease::capture(hud.0 as usize, true).unwrap();
            let before = (physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap());
            assert!(main_lease.verify_hud_owner(&hud_lease, true).is_err());
            assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, owner);
            assert_eq!((physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap()), before);
            hud_lease.restore().unwrap();
        }
        main_lease.restore().unwrap();
    }
    #[test]
    fn hidden_hud_owner_loss_or_old_main_cookie_cannot_authorize_the_hud() {
        let main = Window::new();
        let hud = Window::with_owner(main.0);
        let mut main_lease = Lease::capture(main.0 as usize, true).unwrap();
        let mut hud_lease = Lease::capture(hud.0 as usize, true).unwrap();
        main_lease.verify_hud_owner(&hud_lease, true).unwrap();
        assert_eq!(unsafe { SetWindowLongPtrW(hud.0, GWLP_HWNDPARENT, 0) }, main.0 as isize);
        assert!(main_lease.verify_hud_owner(&hud_lease, true).is_err());
        assert!(unsafe { GetWindow(hud.0, GW_OWNER) }.is_null(), "verification must not reparent a changed HUD");
        unsafe { SetLastError(0); }
        let previous = unsafe { SetWindowLongPtrW(hud.0, GWLP_HWNDPARENT, main.0 as isize) };
        assert!(previous != 0 || unsafe { GetLastError() } == 0);
        main_lease.verify_hud_owner(&hud_lease, true).unwrap();
        unsafe { RemovePropW(main.0, main_lease.cookie.as_ptr()); }
        assert!(main_lease.verify_hud_owner(&hud_lease, true).is_err());
        assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, main.0);
        assert_ne!(unsafe { SetPropW(main.0, main_lease.cookie.as_ptr(), 1usize as _) }, 0);
        hud_lease.restore().unwrap();
        main_lease.restore().unwrap();
    }
    #[test]
    fn destroying_main_removes_owned_hud_and_old_leases_retire_without_touching_a_successor() {
        let main = Window::new();
        let hud = Window::with_owner(main.0);
        let mut main_lease = Lease::capture(main.0 as usize, false).unwrap();
        let mut hud_lease = Lease::capture(hud.0 as usize, false).unwrap();
        main_lease.verify_hud_owner(&hud_lease, true).unwrap();
        assert_ne!(unsafe { DestroyWindow(main.0) }, 0);
        assert_eq!(unsafe { IsWindow(hud.0) }, 0);
        assert!(main_lease.verify_hud_owner(&hud_lease, true).is_err());
        hud_lease.restore().unwrap();
        main_lease.restore().unwrap();
        assert!(hud_lease.finished && main_lease.finished);
        let successor = Window::new();
        let before = (physical_bounds(&successor), input_style(successor.0).unwrap(), affinity(successor.0).unwrap());
        hud_lease.restore().unwrap(); main_lease.restore().unwrap();
        assert_eq!((physical_bounds(&successor), input_style(successor.0).unwrap(), affinity(successor.0).unwrap()), before);
    }
    #[test]
    fn unowned_hud_survives_main_cloak_fallback_and_rejects_an_unknown_owner() {
        for owned in [false, true] {
            let main = Window::new();
            let hud = Window::with_owner(if owned { main.0 } else { std::ptr::null_mut() });
            let mut session = Session { main: Lease::capture(main.0 as usize, true).unwrap(), hud: None };
            let mut hud_lease = Lease::capture(hud.0 as usize, true).unwrap();
            assert!(!session.can_retain_main_outside().unwrap(), "no explicit main region must retain the cloak fallback");
            assert_eq!(session.main.verify_hud_owner(&hud_lease, false).is_ok(), !owned);
            let before = (physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap());
            session.main.prepare(middle_roi(physical_bounds(&main)), false).unwrap();
            assert_eq!(cloak(main.0).unwrap(), APP_CLOAK);
            assert!(!session.retains_main_outside().unwrap());
            if owned {
                assert!(session.main.verify_hud_owner(&hud_lease, true).is_err(), "a cloaked main cannot authorize the owned mode");
                assert!(session.main.verify_hud_owner(&hud_lease, false).is_err(), "fallback cannot silently accept an unknown owner");
            } else {
                session.main.verify_hud_owner(&hud_lease, false).unwrap();
                assert_eq!(cloak(hud.0).unwrap(), 0, "the unowned HUD does not inherit the main cloak");
            }
            assert_eq!((physical_bounds(&hud), input_style(hud.0).unwrap(), affinity(hud.0).unwrap()), before);
            session.restore().unwrap(); hud_lease.restore().unwrap();
            assert_eq!(cloak(main.0).unwrap(), 0);
            assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, if owned { main.0 } else { std::ptr::null_mut() });
        }
    }
    #[test]
    fn owned_hud_is_rejected_if_an_explicit_main_region_takes_whole_window_cloak_fallback() {
        let main = Window::new(); let bounds = physical_bounds(&main);
        author_known_space_region(main.0 as usize, bounds).unwrap();
        let hud = Window::with_owner(main.0);
        let mut session = Session { main: Lease::capture(main.0 as usize, true).unwrap(), hud: None };
        let mut hud_lease = Lease::capture(hud.0 as usize, true).unwrap();
        assert!(session.can_retain_main_outside().unwrap());
        session.main.verify_hud_owner(&hud_lease, true).unwrap();
        session.main.prepare(middle_roi(bounds), true).unwrap();
        assert_eq!(cloak(main.0).unwrap(), APP_CLOAK);
        assert!(!session.retains_main_outside().unwrap());
        assert!(session.main.verify_hud_owner(&hud_lease, true).is_err());
        session.restore().unwrap(); hud_lease.restore().unwrap();
        assert_eq!(unsafe { GetWindow(hud.0, GW_OWNER) }, main.0);
    }
    #[test]
    fn nonlayered_composition_capture_adds_all_input_bits_and_restores_exact_original_state() {
        for initial in [0, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_EX_NOACTIVATE | WS_EX_TRANSPARENT] {
            for shared in [false, true] {
                let window = Window::with_ex_style(std::ptr::null_mut(), WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | initial);
                let bounds = physical_bounds(&window); let roi = middle_roi(bounds);
                author_known_space_region(window.0 as usize, bounds).unwrap();
                let original = region(window.0).unwrap().unwrap();
                let before = (input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap());
                assert_eq!(before.0 & WS_EX_LAYERED as isize, 0);
                assert_ne!(before.0 & WS_EX_NOREDIRECTIONBITMAP as isize, 0);
                let mut lease = Lease::capture(window.0 as usize, shared).unwrap();
                lease.prepare(roi, false).unwrap(); lease.verify(roi).unwrap();
                let during = input_style(window.0).unwrap();
                assert_eq!(during & INPUT_BITS, INPUT_BITS);
                assert_eq!(during & !INPUT_BITS, before.0 & !INPUT_BITS);
                assert_eq!(physical_bounds(&window), bounds);
                assert_eq!(cloak(window.0).unwrap(), before.2);
                assert_eq!(affinity(window.0).unwrap(), if shared { before.1 } else { 0x11 });
                lease.restore().unwrap();
                assert_eq!((input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap()), before);
                assert_eq!(physical_bounds(&window), bounds);
                assert_ne!(unsafe { EqualRgn(original.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
            }
        }
    }
    #[test]
    fn already_layered_capture_preserves_each_original_input_combination_after_restore() {
        for initial in [0, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_EX_NOACTIVATE | WS_EX_TRANSPARENT] {
            let window = Window::with_ex_style(std::ptr::null_mut(), WS_EX_LAYERED | WS_EX_TOOLWINDOW | initial);
            let bounds = physical_bounds(&window); let roi = middle_roi(bounds);
            author_known_space_region(window.0 as usize, bounds).unwrap();
            let original = region(window.0).unwrap().unwrap();
            let before = (input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap());
            let mut lease = Lease::capture(window.0 as usize, true).unwrap();
            lease.prepare(roi, false).unwrap(); lease.verify(roi).unwrap();
            assert_eq!(input_style(window.0).unwrap() & INPUT_BITS, INPUT_BITS);
            assert_eq!(input_style(window.0).unwrap() & !INPUT_BITS, before.0 & !INPUT_BITS);
            lease.restore().unwrap();
            assert_eq!((input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap()), before);
            assert_eq!(physical_bounds(&window), bounds);
            assert_ne!(unsafe { EqualRgn(original.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        }
    }
    #[test]
    fn capture_verify_rejects_layered_bit_loss_without_rewriting_unrelated_state() {
        let window = Window::with_ex_style(std::ptr::null_mut(), WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW);
        let bounds = physical_bounds(&window); let roi = middle_roi(bounds);
        author_known_space_region(window.0 as usize, bounds).unwrap();
        let original = region(window.0).unwrap().unwrap();
        let before = (input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap());
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        lease.prepare(roi, false).unwrap();
        let removed = input_style(window.0).unwrap() & !(WS_EX_LAYERED as isize);
        assert_ne!(unsafe { SetWindowLongPtrW(window.0, GWL_EXSTYLE, removed) }, 0);
        let hole = region(window.0).unwrap().unwrap();
        assert_eq!(removed & (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) as isize, (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT) as isize);
        assert!(lease.verify(roi).is_err(), "the other two flags cannot authorize a nonlayered live overlay");
        assert_eq!(input_style(window.0).unwrap(), removed, "verification must not repair another writer's style");
        assert_eq!((physical_bounds(&window), affinity(window.0).unwrap(), cloak(window.0).unwrap()), (bounds, before.1, before.2));
        assert_ne!(unsafe { EqualRgn(hole.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        lease.restore().unwrap();
        assert_eq!((input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap()), before);
        assert_ne!(unsafe { EqualRgn(original.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
    }
    #[test]
    fn authored_space_region_is_explicit_idempotent_and_keeps_actual_negative_physical_geometry() {
        use windows_sys::Win32::Graphics::Gdi::PtInRegion;
        let window = Window::new();
        let bounds = physical_bounds(&window);
        assert!(bounds.left < 0 && bounds.top < 0);
        let before = (input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap());
        assert!(region(window.0).unwrap().is_none());
        assert!(author_known_space_region(window.0 as usize, bounds).unwrap());
        let explicit = region(window.0).unwrap().unwrap();
        let width = bounds.right - bounds.left; let height = bounds.bottom - bounds.top;
        for (x, y) in [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)] {
            assert_ne!(unsafe { PtInRegion(explicit.raw(), x, y) }, 0);
        }
        assert_eq!(unsafe { PtInRegion(explicit.raw(), width, height / 2) }, 0);
        assert!(!author_known_space_region(window.0 as usize, bounds).unwrap(), "same authored shape must skip SetWindowRgn");
        assert_eq!(physical_bounds(&window), bounds);
        assert_eq!((input_style(window.0).unwrap(), affinity(window.0).unwrap(), cloak(window.0).unwrap()), before);
        assert!(unsafe { GetPropW(window.0, LEASE_OWNER_KEY.as_ptr()) }.is_null());
    }
    #[test]
    fn authored_space_region_rejects_wrong_geometry_and_active_native_lease_without_resetting_hole() {
        let window = Window::new(); let bounds = physical_bounds(&window);
        let wrong = Rect { left: bounds.left + 1, ..bounds };
        assert!(author_known_space_region(window.0 as usize, wrong).is_err());
        assert!(region(window.0).unwrap().is_none());
        author_known_space_region(window.0 as usize, bounds).unwrap();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        assert!(Lease::capture(window.0 as usize, true).is_err());
        lease.prepare(middle_roi(bounds), false).unwrap();
        let aperture = region(window.0).unwrap().unwrap();
        assert!(author_known_space_region(window.0 as usize, bounds).is_err());
        assert_ne!(unsafe { EqualRgn(aperture.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        lease.verify(middle_roi(bounds)).unwrap(); lease.restore().unwrap();
    }
    #[test]
    fn authored_space_retention_is_predicted_then_proven_and_restores_full_region_while_none_still_cloaks() {
        let window = Window::new(); let bounds = physical_bounds(&window); let roi = middle_roi(bounds);
        author_known_space_region(window.0 as usize, bounds).unwrap();
        let full = region(window.0).unwrap().unwrap();
        let mut session = Session { main: Lease::capture(window.0 as usize, true).unwrap(), hud: None };
        assert!(session.can_retain_main_outside().unwrap());
        assert!(!session.retains_main_outside().unwrap());
        session.main.prepare(roi, false).unwrap();
        assert!(session.retains_main_outside().unwrap());
        assert_eq!(cloak(window.0).unwrap(), 0);
        session.restore().unwrap();
        assert_ne!(unsafe { EqualRgn(full.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        let fallback = Window::new(); let fallback_roi = middle_roi(physical_bounds(&fallback));
        let mut session = Session { main: Lease::capture(fallback.0 as usize, true).unwrap(), hud: None };
        assert!(!session.can_retain_main_outside().unwrap());
        session.main.prepare(fallback_roi, false).unwrap();
        assert!(!session.retains_main_outside().unwrap());
        assert_eq!(cloak(fallback.0).unwrap(), APP_CLOAK);
        session.restore().unwrap(); assert!(region(fallback.0).unwrap().is_none());
    }
    #[test]
    fn authored_space_cookie_loss_and_foreign_thread_cannot_change_current_region() {
        let window = Window::new(); let bounds = physical_bounds(&window);
        let hwnd = window.0 as usize;
        assert!(std::thread::spawn(move || author_known_space_region(hwnd, bounds)).join().unwrap().is_err());
        assert!(region(window.0).unwrap().is_none());
        let mut owner = Lease::capture(window.0 as usize, false).unwrap();
        let distinct = Region(unsafe { CreateRectRgn(0, 0, 30, 40) } as usize);
        distinct.copy().unwrap().assign(window.0).unwrap();
        unsafe { RemovePropW(window.0, owner.cookie.as_ptr()); }
        assert!(owner.author_full_region(bounds).is_err());
        assert!(author_known_space_region(hwnd, bounds).is_err());
        assert_ne!(unsafe { EqualRgn(distinct.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        assert!(owner.restore().is_err());
    }
    #[test]
    fn hidden_hud_disables_transitions_without_changing_capture_region_or_native_state() {
        for shared in [false, true] {
            let window = Window::new();
            let original = Region(unsafe { CreateRectRgn(0, 0, 83, 97) } as usize);
            assert_ne!(original.0, 0);
            original.copy().unwrap().assign(window.0).unwrap();
            let mut lease = Lease::capture(window.0 as usize, shared).unwrap();
            let before = (physical_bounds(&window), affinity(window.0).unwrap(),
                input_style(window.0).unwrap(), cloak(window.0).unwrap());
            assert_eq!(unsafe { IsWindowVisible(window.0) }, 0);
            lease.disable_hud_transitions_before_show().unwrap();
            assert_eq!((physical_bounds(&window), affinity(window.0).unwrap(),
                input_style(window.0).unwrap(), cloak(window.0).unwrap()), before);
            assert_ne!(unsafe { EqualRgn(original.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
            assert!(lease.current());
            assert!(!lease.prepared && !lease.ready && lease.scroll_clip.is_none());
            lease.restore().unwrap();
        }
    }
    #[test]
    fn hidden_hud_transition_change_rejects_stale_cookie_and_foreign_thread() {
        let window = Window::new();
        let mut lease = Lease::capture(window.0 as usize, false).unwrap();
        let before = (physical_bounds(&window), affinity(window.0).unwrap(),
            input_style(window.0).unwrap(), cloak(window.0).unwrap());
        std::thread::scope(|scope| {
            assert!(scope.spawn(|| lease.disable_hud_transitions_before_show()).join().unwrap().is_err());
        });
        unsafe { RemovePropW(window.0, lease.cookie.as_ptr()); }
        assert!(lease.disable_hud_transitions_before_show().is_err());
        assert_eq!((physical_bounds(&window), affinity(window.0).unwrap(),
            input_style(window.0).unwrap(), cloak(window.0).unwrap()), before);
        assert!(region(window.0).unwrap().is_none());
        assert_ne!(unsafe { SetPropW(window.0, lease.cookie.as_ptr(), 1usize as _) }, 0);
        lease.restore().unwrap();
    }
    #[test]
    fn hidden_hud_transition_change_rejects_prepared_lease_without_resetting_aperture() {
        let window = Window::new();
        let bounds = physical_bounds(&window);
        author_known_space_region(window.0 as usize, bounds).unwrap();
        let mut lease = Lease::capture(window.0 as usize, true).unwrap();
        let roi = middle_roi(bounds);
        lease.prepare(roi, false).unwrap();
        let aperture = region(window.0).unwrap().unwrap();
        let before = (physical_bounds(&window), affinity(window.0).unwrap(),
            input_style(window.0).unwrap(), cloak(window.0).unwrap());
        assert!(lease.disable_hud_transitions_before_show().is_err());
        assert_eq!((physical_bounds(&window), affinity(window.0).unwrap(),
            input_style(window.0).unwrap(), cloak(window.0).unwrap()), before);
        assert_ne!(unsafe { EqualRgn(aperture.raw(), region(window.0).unwrap().unwrap().raw()) }, 0);
        lease.verify(roi).unwrap();
        lease.restore().unwrap();
    }
    #[test]
    fn shared_hud_stays_outside_roi_and_fullscreen_requires_stop_hotkey() {
        let work = Rect::new(-2000, -100, 800, 600).unwrap();
        let point = hud_position(work, roi(), (176, 44)).unwrap();
        assert_eq!(point.1, roi().top - 44 - 6, "controls prefer directly above the ROI, not the screen edge");
        assert!(!roi().intersects(Rect::new(point.0, point.1, 176, 44).unwrap()));
        assert_eq!(hud_position(work, work, (176, 44)), None);
        assert_eq!(hud_position(work, roi(), (900, 44)), None);
        assert!(Rect::new(i32::MAX, 0, 1, 1).is_err());
    }
    #[test]
    fn scroll_native_aperture_always_subtracts_every_acquired_pixel_even_from_overlapping_shapes() {
        use windows_sys::Win32::Graphics::Gdi::{PtInRegion, RGN_AND};
        let bounds = Rect::new(-2000,-100,800,600).unwrap();
        let clip = scroll_region(bounds,roi(),&[bounds]).unwrap();
        let hole = Region(unsafe { CreateRectRgn(100,100,400,300) } as usize);
        let intersection = Region::empty().unwrap();
        assert_ne!(unsafe { CombineRgn(intersection.raw(),clip.raw(),hole.raw(),RGN_AND) },0);
        assert_ne!(unsafe { EqualRgn(intersection.raw(),Region::empty().unwrap().raw()) },0);
        for (x,y) in [(100,100),(399,100),(100,299),(399,299),(250,200)] {
            assert_eq!(unsafe { PtInRegion(clip.raw(),x,y) },0);
        }
        for (x,y) in [(99,100),(400,100),(100,99),(100,300)] {
            assert_ne!(unsafe { PtInRegion(clip.raw(),x,y) },0);
        }
        assert!(scroll_region(bounds,Rect::new(0,0,10,10).unwrap(),&[]).is_err());
        assert!(scroll_region(bounds,roi(),&[bounds;4]).is_err());
    }
    #[test]
    fn scroll_native_shared_hud_grows_while_main_window_stays_safely_cloaked() {
        use windows_sys::Win32::Graphics::Gdi::PtInRegion;
        let main = Window::new();
        let mut main_lease = Lease::capture(main.0 as usize,true).unwrap();
        // CreateWindowEx uses the test thread's awareness context; the native
        // lease deliberately reads PMv2 physical bounds. Never infer local
        // region pixels from the fixture's requested logical coordinates.
        let bounds = main_lease.before_bounds;
        let width = u32::try_from(bounds.right-bounds.left).unwrap();
        let height = u32::try_from(bounds.bottom-bounds.top).unwrap();
        let selected = Rect::new(bounds.left+(width/4) as i32,bounds.top+(height/4) as i32,width/4,height/3).unwrap();
        main_lease.prepare(selected,false).unwrap();
        assert_eq!(cloak(main.0).unwrap(),APP_CLOAK);
        let hud = Window::new();
        let mut lease = Lease::capture(hud.0 as usize,true).unwrap();
        assert_eq!(lease.before_bounds,bounds);
        let outline = Rect::new(selected.left-2,selected.top-2,width/4+4,height/3+4).unwrap();
        let left = selected.right+16;
        let preview_width = u32::try_from(bounds.right-left-16).unwrap().min(150);
        let short_height = u32::try_from(bounds.bottom-selected.top-16).unwrap()/3;
        let short = Rect::new(left,selected.top,preview_width,short_height).unwrap();
        let tall = Rect::new(left,selected.top,preview_width,short_height*2).unwrap();
        assert!(bounds.contains(tall)); assert!(!selected.intersects(tall));
        let added_point = (tall.left+(preview_width/2) as i32-bounds.left,
            short.bottom+(short_height/2) as i32-bounds.top);
        let hole_point = (selected.left+(width/8) as i32-bounds.left,
            selected.top+(height/6) as i32-bounds.top);
        lease.set_scroll_clip(selected,&[outline,short]).unwrap();
        let first = region(hud.0).unwrap().unwrap();
        assert_eq!(unsafe { PtInRegion(first.raw(),added_point.0,added_point.1) },0);
        lease.set_scroll_clip(selected,&[outline,tall]).unwrap();
        let grown = region(hud.0).unwrap().unwrap();
        assert_ne!(unsafe { PtInRegion(grown.raw(),added_point.0,added_point.1) },0);
        assert_eq!(unsafe { PtInRegion(grown.raw(),hole_point.0,hole_point.1) },0);
        lease.set_scroll_clip(selected,&[outline,tall]).unwrap();
        lease.verify_scroll_clip(selected).unwrap();
        main_lease.verify(selected).unwrap();
        assert_eq!(cloak(main.0).unwrap(),APP_CLOAK);
        assert_eq!(affinity(hud.0).unwrap(),0);
        // Session cleanup destroys only its disposable HUD before restoring it.
        drop(hud);
        lease.restore().unwrap();
        main_lease.restore().unwrap();
        assert!(region(main.0).unwrap().is_none());
    }
    #[test]
    fn scroll_native_dimmed_monitor_keeps_every_roi_pixel_and_input_target_outside_the_hud() {
        use windows_sys::Win32::Graphics::Gdi::PtInRegion;
        let hud = Window::new();
        let mut lease = Lease::capture(hud.0 as usize,true).unwrap();
        let bounds = lease.before_bounds;
        let width = bounds.right-bounds.left; let height = bounds.bottom-bounds.top;
        let selected = Rect::new(bounds.left+width/4,bounds.top+height/4,(width/2) as u32,(height/2) as u32).unwrap();
        lease.set_scroll_clip(selected,&[bounds]).unwrap();
        let original = region(hud.0).unwrap().unwrap();
        for (x,y) in [(0,0),(width-1,0),(0,height-1),(width-1,height-1)] {
            assert_ne!(unsafe { PtInRegion(original.raw(),x,y) },0);
        }
        let left=selected.left-bounds.left; let top=selected.top-bounds.top;
        let right=selected.right-bounds.left; let bottom=selected.bottom-bounds.top;
        for (x,y) in [(left,top),(right-1,top),(left,bottom-1),(right-1,bottom-1),((left+right)/2,(top+bottom)/2)] {
            assert_eq!(unsafe { PtInRegion(original.raw(),x,y) },0);
        }
        // Repainted/growing preview pixels must not resize the native aperture.
        lease.set_scroll_clip(selected,&[bounds]).unwrap();
        lease.verify_scroll_clip(selected).unwrap();
        assert_ne!(unsafe { EqualRgn(original.raw(),region(hud.0).unwrap().unwrap().raw()) },0);
        drop(hud); lease.restore().unwrap();
    }
    #[test]
    fn scroll_native_hud_rejects_foreign_region_or_identity_without_overwriting_it() {
        let hud = Window::new();
        let mut lease = Lease::capture(hud.0 as usize,true).unwrap();
        let shape = Rect::new(-1550,0,150,100).unwrap();
        lease.set_scroll_clip(roi(),&[shape]).unwrap();
        let empty = Region::empty().unwrap();
        empty.copy().unwrap().assign(hud.0).unwrap();
        assert!(lease.verify_scroll_clip(roi()).is_err());
        assert!(lease.set_scroll_clip(roi(),&[shape]).is_err());
        assert_ne!(unsafe { EqualRgn(region(hud.0).unwrap().unwrap().raw(),empty.raw()) },0);
        unsafe { RemovePropW(hud.0,lease.cookie.as_ptr()); }
        assert!(lease.set_scroll_clip(roi(),&[shape]).is_err());
        assert_ne!(unsafe { EqualRgn(region(hud.0).unwrap().unwrap().raw(),empty.raw()) },0);
        drop(hud);
        lease.restore().unwrap();
    }
    #[test]
    fn scroll_native_protected_hud_retains_original_capture_exclusion_policy() {
        let hud = Window::new();
        let mut lease = Lease::capture(hud.0 as usize,false).unwrap();
        assert_ne!(unsafe { SetWindowDisplayAffinity(hud.0,0x11) },0);
        lease.set_scroll_clip(roi(),&[Rect::new(-1550,0,150,100).unwrap()]).unwrap();
        lease.verify_scroll_clip(roi()).unwrap();
        assert_eq!(affinity(hud.0).unwrap(),0x11);
        assert_eq!(cloak(hud.0).unwrap(),0);
        drop(hud);
        lease.restore().unwrap();
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
    fn staged_native_hud_keeps_main_unchanged_until_capture_and_rejects_late_staging() {
        let main_window = Window::new();
        let hud_window = Window::new();
        let mut main = Lease::capture(main_window.0 as usize, true).unwrap();
        let mut hud = Lease::capture(hud_window.0 as usize, true).unwrap();
        let bounds = main.before_bounds;
        let width = (bounds.right - bounds.left) as u32;
        let height = (bounds.bottom - bounds.top) as u32;
        let roi = Rect::new(bounds.left + (width / 4) as i32, bounds.top + (height / 4) as i32, width / 2, height / 2).unwrap();
        let before = (input_style(main_window.0).unwrap(), affinity(main_window.0).unwrap(), cloak(main_window.0).unwrap());
        main.verify_staged(roi).unwrap();
        assert!(main.verify_staged(Rect::new(bounds.right, bounds.bottom, 1, 1).unwrap()).is_err());
        let hud_bounds = hud.before_bounds;
        hud.set_scroll_clip(roi, &[hud_bounds]).unwrap();
        main.verify_staged(roi).unwrap();
        assert!(region(main_window.0).unwrap().is_none());
        assert_eq!((input_style(main_window.0).unwrap(), affinity(main_window.0).unwrap(), cloak(main_window.0).unwrap()), before);
        hud.verify_scroll_clip(roi).unwrap();
        main.prepare(roi, false).unwrap();
        assert!(main.verify_staged(roi).is_err());
        main.verify(roi).unwrap();
        hud.verify_scroll_clip(roi).unwrap();
        main.restore().unwrap();
        assert!(region(main_window.0).unwrap().is_none());
        assert_eq!((input_style(main_window.0).unwrap(), affinity(main_window.0).unwrap(), cloak(main_window.0).unwrap()), before);
        drop(hud_window);
        hud.restore().unwrap();
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
