// SPDX-License-Identifier: MPL-2.0
//! Called only inside --application-snapshot-worker, on its dedicated MTA.
use crate::{
    application_snapshot::{ScrollPosition, Session, Viewport, WindowFrame},
    window_snap::SnapshotWindow,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::RECT,
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    UI::{
        Accessibility::*,
        HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
    },
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};
struct Runtime(DPI_AWARENESS_CONTEXT);
impl Runtime {
    fn new() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|_| "无法初始化应用快照")?;
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if previous.0.is_null() {
                CoUninitialize();
                return Err("应用窗口物理坐标不可用".into());
            }
            Ok(Self(previous))
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
            CoUninitialize();
        }
    }
}
fn rect(value: RECT) -> Result<Viewport, String> {
    let w = i64::from(value.right) - i64::from(value.left);
    let h = i64::from(value.bottom) - i64::from(value.top);
    if w <= 0 || h <= 0 || w > u32::MAX as i64 || h > u32::MAX as i64 {
        return Err("应用滚动区域无效".into());
    }
    Ok(Viewport {
        x: value.left,
        y: value.top,
        width: w as u32,
        height: h as u32,
    })
}
fn intersection(a: Viewport, b: Viewport) -> Option<Viewport> {
    let x = i64::from(a.x).max(i64::from(b.x));
    let y = i64::from(a.y).max(i64::from(b.y));
    let right = (i64::from(a.x) + a.width as i64).min(i64::from(b.x) + b.width as i64);
    let bottom = (i64::from(a.y) + a.height as i64).min(i64::from(b.y) + b.height as i64);
    (right > x && bottom > y).then_some(Viewport {
        x: x as i32,
        y: y as i32,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    })
}
struct Scroller {
    element: IUIAutomationElement,
    pattern: IUIAutomationScrollPattern,
    bounds: Viewport,
    viewport: Viewport,
}
fn discover(
    automation: &IUIAutomation,
    window: &SnapshotWindow,
    selection: Viewport,
) -> Result<Scroller, String> {
    unsafe {
        let root = automation
            .ElementFromHandle(windows::Win32::Foundation::HWND(window.handle as *mut _))
            .map_err(|_| "应用自动化不可用")?;
        if root.CurrentProcessId().map_err(|_| "应用窗口标识不可用")? != window.process as i32
        {
            return Err("应用窗口标识已改变".into());
        }
        let walker = automation.RawViewWalker().map_err(|_| "应用自动化不可用")?;
        let started = Instant::now();
        let mut visited = 0;
        let mut stack = vec![(root, 0usize)];
        let mut best: Option<Scroller> = None;
        while let Some((element, depth)) = stack.pop() {
            visited += 1;
            if visited > 10000 || depth > 48 || started.elapsed() > Duration::from_secs(7) {
                return Err("应用滚动区域过于复杂".into());
            }
            // Password providers are never queried for patterns or descendants.
            if element.CurrentIsPassword().map_or(true, |v| v.as_bool())
                || element.CurrentIsOffscreen().map_or(true, |v| v.as_bool())
            {
                continue;
            }
            let bounds = element
                .CurrentBoundingRectangle()
                .ok()
                .and_then(|r| rect(r).ok());
            if let (Some(bounds), Ok(pattern)) = (
                bounds,
                element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId),
            ) {
                if pattern
                    .CurrentVerticallyScrollable()
                    .is_ok_and(|v| v.as_bool())
                {
                    if let Some(viewport) = intersection(bounds, selection).filter(|r| {
                        r.width >= 16
                            && r.height >= 64
                            && r.width as u64 * r.height as u64 <= 4 * 1024 * 1024
                    }) {
                        let candidate = Scroller {
                            element: element.clone(),
                            pattern,
                            bounds,
                            viewport,
                        };
                        if best.as_ref().is_none_or(|old| {
                            old.viewport.width as u64 * (old.viewport.height as u64)
                                < viewport.width as u64 * viewport.height as u64
                        }) {
                            best = Some(candidate);
                        }
                        // Prefer the outer document viewport to embedded subpanels.
                        continue;
                    }
                }
            }
            if depth == 48 {
                continue;
            }
            if let Ok(mut child) = walker.GetFirstChildElement(&element) {
                loop {
                    if stack.len() + visited >= 10000 {
                        return Err("应用滚动区域过于复杂".into());
                    }
                    stack.push((child.clone(), depth + 1));
                    match walker.GetNextSiblingElement(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
        }
        best.ok_or("此应用不支持自动快照，请使用手动长截图".into())
    }
}
impl Scroller {
    fn state(&self) -> Result<ScrollPosition, String> {
        unsafe {
            if rect(
                self.element
                    .CurrentBoundingRectangle()
                    .map_err(|_| "应用滚动区域不可用")?,
            )? != self.bounds
            {
                return Err("应用滚动区域已改变".into());
            }
            let state = ScrollPosition {
                viewport: self.viewport,
                horizontal: if self
                    .pattern
                    .CurrentHorizontallyScrollable()
                    .map_err(|_| "应用滚动状态不可用")?
                    .as_bool()
                {
                    self.pattern
                        .CurrentHorizontalScrollPercent()
                        .map_err(|_| "应用滚动状态不可用")?
                } else {
                    -1.0
                },
                vertical: self
                    .pattern
                    .CurrentVerticalScrollPercent()
                    .map_err(|_| "应用滚动状态不可用")?,
                view_size: self
                    .pattern
                    .CurrentVerticalViewSize()
                    .map_err(|_| "应用滚动状态不可用")?,
            };
            state.validate()?;
            Ok(state)
        }
    }
    fn move_to(&self, horizontal: f64, vertical: f64) -> Result<ScrollPosition, String> {
        if !vertical.is_finite()
            || !(0.0..=100.0).contains(&vertical)
            || !horizontal.is_finite()
            || !(horizontal == -1.0 || (0.0..=100.0).contains(&horizontal))
        {
            return Err("应用滚动位置无效".into());
        }
        unsafe {
            self.pattern
                .SetScrollPercent(horizontal, vertical)
                .map_err(|_| "应用无法滚动到指定位置")?;
        }
        let started = Instant::now();
        loop {
            let state = self.state()?;
            if (state.vertical - vertical).abs() <= 0.05
                && (horizontal == -1.0 || (state.horizontal - horizontal).abs() <= 0.05)
            {
                return Ok(state);
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("应用没有滚动到指定位置".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
#[derive(Default)]
struct Latest {
    sequence: u64,
    timestamp: i64,
    changed_at: Option<Instant>,
    pixels: Option<Arc<[u8]>>,
    failed: bool,
}
struct Flags {
    window: SnapshotWindow,
    viewport: Viewport,
    latest: Arc<Mutex<Latest>>,
    requested: Arc<AtomicBool>,
}
struct Capture {
    flags: Flags,
}
impl GraphicsCaptureApiHandler for Capture {
    type Flags = Flags;
    type Error = String;
    fn new(context: Context<Flags>) -> Result<Self, String> {
        Ok(Self {
            flags: context.flags,
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> Result<(), String> {
        // No GPU mapping, pixel copy or window query while the parent is
        // stitching/previewing an already acquired frame.
        if !self.flags.requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = (|| {
            let w = &self.flags.window;
            let r = self.flags.viewport;
            let timestamp = frame
                .timestamp()
                .map_err(|_| "应用画面时间戳不可用")?
                .Duration;
            if frame.width() != w.width || frame.height() != w.height || !w.is_current() {
                return Err("应用窗口已移动或改变".into());
            }
            let x =
                u32::try_from(i64::from(r.x) - i64::from(w.x)).map_err(|_| "应用画面位置无效")?;
            let y =
                u32::try_from(i64::from(r.y) - i64::from(w.y)).map_err(|_| "应用画面位置无效")?;
            if u64::from(x) + r.width as u64 > w.width as u64
                || u64::from(y) + r.height as u64 > w.height as u64
            {
                return Err("应用画面越界".into());
            }
            let mut buffer = frame
                .buffer_crop(x, y, x + r.width, y + r.height)
                .map_err(|_| "无法读取应用画面")?;
            let pitch = buffer.row_pitch() as usize;
            let bytes = buffer.as_raw_buffer();
            let row = r.width as usize * 4;
            if pitch < row || bytes.len() < pitch * (r.height as usize - 1) + row {
                return Err("应用画面数据无效".into());
            }
            let mut pixels = vec![0; row * r.height as usize];
            for (source, destination) in bytes
                .chunks(pitch)
                .take(r.height as usize)
                .zip(pixels.chunks_mut(row))
            {
                for (bgra, rgba) in source[..row]
                    .chunks_exact(4)
                    .zip(destination.chunks_exact_mut(4))
                {
                    rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
                }
            }
            let mut latest = self.flags.latest.lock().map_err(|_| "应用采集状态不可用")?;
            latest.sequence = latest.sequence.checked_add(1).ok_or("应用采集序号溢出")?;
            latest.timestamp = timestamp;
            if latest
                .pixels
                .as_ref()
                .is_none_or(|old| old.as_ref() != pixels.as_slice())
            {
                latest.changed_at = Some(Instant::now());
                latest.pixels = Some(Arc::from(pixels));
            }
            Ok(())
        })();
        if result.is_err() {
            if let Ok(mut latest) = self.flags.latest.lock() {
                latest.failed = true;
            }
        }
        result
    }
    fn on_closed(&mut self) -> Result<(), String> {
        if let Ok(mut latest) = self.flags.latest.lock() {
            latest.failed = true;
        }
        Ok(())
    }
}
pub(crate) struct NativeSession {
    capture: Option<CaptureControl<Capture, String>>,
    latest: Arc<Mutex<Latest>>,
    requested: Arc<AtomicBool>,
    scroller: Scroller,
    window: SnapshotWindow,
    original: ScrollPosition,
    mutated: bool,
    _runtime: Runtime,
}
impl NativeSession {
    pub fn new(window: SnapshotWindow, selection: Viewport) -> Result<Self, String> {
        let runtime = Runtime::new()?;
        window.validate()?;
        if !window.is_current() {
            return Err("应用窗口已改变".into());
        }
        let full = Viewport {
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
        };
        if intersection(full, selection) != Some(selection) {
            return Err("应用快照选区越界".into());
        }
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| "应用自动化不可用")?;
        let scroller = discover(&automation, &window, selection)?;
        let original = scroller.state()?;
        if !window.is_current() {
            return Err("应用窗口已改变".into());
        }
        let latest = Arc::new(Mutex::new(Latest::default()));
        let requested = Arc::new(AtomicBool::new(true));
        let settings = Settings::new(
            Window::from_raw_hwnd(window.handle as *mut _),
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            Flags {
                window: window.clone(),
                viewport: scroller.viewport,
                latest: latest.clone(),
                requested: requested.clone(),
            },
        );
        let capture = Capture::start_free_threaded(settings).map_err(|_| "无法采集此应用窗口")?;
        Ok(Self {
            capture: Some(capture),
            latest,
            requested,
            scroller,
            window,
            original,
            mutated: false,
            _runtime: runtime,
        })
    }
    fn pixels(
        &self,
        before: u64,
        after_time: i64,
        changed: bool,
        started: Instant,
        cancel: &AtomicBool,
    ) -> Result<image::RgbaImage, String> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("应用快照已取消".into());
            }
            {
                let latest = self.latest.lock().map_err(|_| "应用采集状态不可用")?;
                if latest.failed {
                    return Err("应用窗口采集已停止".into());
                }
                if started.elapsed() >= Duration::from_millis(240)
                    && latest
                        .changed_at
                        .is_some_and(|time| time.elapsed() >= Duration::from_millis(120))
                    && (!changed || latest.sequence > before && latest.timestamp >= after_time)
                {
                    if let Some(bytes) = &latest.pixels {
                        return image::RgbaImage::from_raw(
                            self.original.viewport.width,
                            self.original.viewport.height,
                            bytes.to_vec(),
                        )
                        .ok_or("应用画面无效".into());
                    }
                }
            }
            if started.elapsed() > Duration::from_secs(5)
                || self.capture.as_ref().is_none_or(|c| c.is_finished())
            {
                return Err("未收到更新后的应用画面".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Session for NativeSession {
    fn original(&self) -> &ScrollPosition {
        &self.original
    }
    fn frame_at(&mut self, position: f64, cancel: &AtomicBool) -> Result<WindowFrame, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("应用快照已取消".into());
        }
        self.requested.store(true, Ordering::Release);
        let _request = FrameRequest(self.requested.clone());
        if !self.window.is_current() {
            return Err("应用窗口已改变".into());
        }
        let previous = self.scroller.state()?;
        let sequence = self
            .latest
            .lock()
            .map_err(|_| "应用采集状态不可用")?
            .sequence;
        self.mutated = true;
        let started = Instant::now();
        let after_time = compositor_time()?;
        let state = self.scroller.move_to(-1.0, position)?;
        let image = self.pixels(
            sequence,
            after_time,
            (previous.vertical - position).abs() > 0.05,
            started,
            cancel,
        )?;
        if !self.window.is_current() || self.scroller.state()? != state {
            return Err("应用画面在采集中发生变化".into());
        }
        Ok(WindowFrame { state, image })
    }
    fn restore(&mut self) -> Result<ScrollPosition, String> {
        if !self.window.is_same_lifetime() {
            return Err("应用窗口已关闭或更换".into());
        }
        let state = if self.mutated {
            self.scroller
                .move_to(self.original.horizontal, self.original.vertical)?
        } else {
            self.scroller.state()?
        };
        if !self.original.restored(&state) {
            return Err("应用没有恢复原滚动位置".into());
        }
        self.mutated = false;
        Ok(state)
    }
}
fn compositor_time() -> Result<i64, String> {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let mut count = 0i64;
    let mut frequency = 0i64;
    unsafe {
        QueryPerformanceCounter(&mut count).map_err(|_| "应用画面时钟不可用")?;
        QueryPerformanceFrequency(&mut frequency).map_err(|_| "应用画面时钟不可用")?;
    }
    if count < 0 || frequency <= 0 {
        return Err("应用画面时钟无效".into());
    }
    i64::try_from(i128::from(count) * 10_000_000 / i128::from(frequency))
        .map_err(|_| "应用画面时钟溢出".into())
}
struct FrameRequest(Arc<AtomicBool>);
impl Drop for FrameRequest {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Drop for NativeSession {
    fn drop(&mut self) {
        if self.mutated {
            let _ = self.restore();
        }
        if let Some(capture) = self.capture.take() {
            let _ = capture.stop();
        }
    }
}
