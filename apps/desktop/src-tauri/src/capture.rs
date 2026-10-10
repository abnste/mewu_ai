//! Local, single-monitor snapshots. Call from a blocking worker after hiding
//! Mewu windows. This module neither uploads pixels nor changes window state.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// PNG dimensions are always physical image pixels. Desktop geometry uses the
/// explicit coordinate_space below; in particular, a macOS origin is in points.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedScreen {
    /// Optional capture-time metadata; absent on unsupported/unstable desktops.
    /// It is registered by the host only after the PNG joins the active scene.
    #[serde(skip)]
    pub window_map: Option<super::window_snap::FrameSnapMap>,
    pub asset_id: String,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale_factor: f64,
    pub monitor_id: u32,
    pub display_width: u32,
    pub display_height: u32,
    pub coordinate_space: CoordinateSpace,
    pub pixels_per_display_unit_x: f64,
    pub pixels_per_display_unit_y: f64,
    pub backend: CaptureBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CoordinateSpace {
    PhysicalPixels,
    LogicalPoints,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureBackend {
    Gdi,
    CoreGraphics,
}

/// Monitor ids are valid for the current display configuration, not persistent
/// device identities. Re-enumerate after displays are added or removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub origin_x: i32,
    pub origin_y: i32,
    pub display_width: u32,
    pub display_height: u32,
    pub scale_factor: f64,
    pub rotation: f32,
    pub is_primary: bool,
    pub coordinate_space: CoordinateSpace,
}

/// Immutable host-selected choices for one actual capture. This adapter does
/// not read settings or change an in-flight capture when preferences change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureOptions {
    pub include_cursor: bool,
}

pub fn capture_desktop(asset_dir: &Path) -> Result<CapturedScreen, String> {
    capture_desktop_with_options(asset_dir, CaptureOptions::default())
}

pub fn capture_monitor(asset_dir: &Path, monitor_id: u32) -> Result<CapturedScreen, String> {
    capture_monitor_with_options(asset_dir, monitor_id, CaptureOptions::default())
}

pub fn capture_desktop_with_options(
    asset_dir: &Path,
    options: CaptureOptions,
) -> Result<CapturedScreen, String> {
    ensure_supported_options(options)?;
    implementation::capture(asset_dir, None, options)
}

pub fn capture_monitor_with_options(
    asset_dir: &Path,
    monitor_id: u32,
    options: CaptureOptions,
) -> Result<CapturedScreen, String> {
    ensure_supported_options(options)?;
    implementation::capture(asset_dir, Some(monitor_id), options)
}

fn ensure_supported_options(options: CaptureOptions) -> Result<(), String> {
    if options.include_cursor && !cfg!(windows) {
        return Err("当前平台尚不支持截图包含光标".into());
    }
    Ok(())
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
    implementation::list_monitors()
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod implementation {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::{BufWriter, Write},
        sync::{Mutex, TryLockError},
    };
    use uuid::Uuid;
    use xcap::{
        image::{
            codecs::png::{CompressionType, FilterType, PngEncoder},
            ExtendedColorType, ImageEncoder,
        },
        Monitor,
    };

    // This bounds one returned RGBA allocation, not the process working set:
    // upstream capture also owns OS surfaces and temporary buffers. 8K fits.
    const MAX_SIDE: u32 = 16_384;
    const MAX_PIXELS: u64 = 33_554_432;
    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    pub(super) fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
        let _dpi_context = platform::enter_dpi_context()?;
        Monitor::all()
            .map_err(|error| format!("无法读取显示器：{error}"))?
            .iter()
            .map(monitor_info)
            .collect()
    }

    pub(super) fn capture(
        asset_dir: &Path,
        selected_id: Option<u32>,
        options: CaptureOptions,
    ) -> Result<CapturedScreen, String> {
        #[cfg(not(windows))]
        let _ = options;
        let _capture_guard = CAPTURE_LOCK.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => "正在获取屏幕，请稍后重试".to_owned(),
            TryLockError::Poisoned(_) => "截图服务需要重新启动".to_owned(),
        })?;
        let _dpi_context = platform::enter_dpi_context()?;
        if !asset_dir.is_absolute() {
            return Err("截图素材目录必须是绝对路径".to_owned());
        }

        let monitors = Monitor::all().map_err(|error| format!("无法读取显示器：{error}"))?;
        let monitor = select_monitor(&monitors, selected_id)?;
        let before = monitor_info(&monitor)?;
        validate_expected_size(&before)?;

        let asset_id = Uuid::new_v4().to_string();
        let window_probe = super::super::window_snap::begin_capture(
            before.origin_x,
            before.origin_y,
            before.display_width,
            before.display_height,
        );

        let pixels = monitor
            // Fix the requested bounds before allocation, so a Windows mode
            // change cannot silently expand the upstream GDI pixel buffer.
            .capture_region(0, 0, before.display_width, before.display_height)
            .map_err(|error| format!("无法获取屏幕，请检查屏幕录制权限：{error}"))?;
        // Sample before slower window-map metadata/encoding. OS pixel capture
        // and cursor sampling are separate calls, not an atomic desktop frame.
        // Disabled captures never read the global cursor resource here.
        #[cfg(windows)]
        let cursor = if options.include_cursor {
            crate::capture_cursor::CursorSnapshot::sample()?
        } else {
            None
        };
        let window_map = super::super::window_snap::finish_capture(window_probe, &asset_id);
        let (width, height) = pixels.dimensions();
        validate_pixel_size(width, height)?;
        if before != monitor_info(&monitor)? {
            return Err("截图期间显示器设置已改变，请重新截图".to_owned());
        }
        if before.coordinate_space == CoordinateSpace::PhysicalPixels
            && (width != before.display_width || height != before.display_height)
        {
            return Err("截图尺寸与显示器不一致，请重新截图".to_owned());
        }
        #[cfg(windows)]
        let pixels = {
            let mut pixels = pixels;
            // Upstream GDI uses 32bpp BI_RGB: its fourth byte is reserved,
            // unlike a transparent image asset's meaningful alpha channel.
            crate::capture_cursor::opaque_gdi_screenshot_alpha(pixels.as_mut())?;
            if let Some(cursor) = cursor {
                cursor.compose_into(
                    pixels.as_mut(),
                    width,
                    height,
                    (before.origin_x, before.origin_y),
                )?;
            }
            pixels
        };

        // The caller supplies a trusted app-owned directory. No caller-provided
        // filename is joined into it. Only successfully encoded files are handed
        // to the asset catalog; failed writes are removed before returning.
        fs::create_dir_all(asset_dir).map_err(|error| format!("无法创建素材目录：{error}"))?;
        let path = asset_dir.join(format!("{asset_id}.png"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("无法创建截图素材：{error}"))?;
        let encoded = {
            let mut writer = BufWriter::new(file);
            PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, FilterType::Adaptive)
                .write_image(pixels.as_raw(), width, height, ExtendedColorType::Rgba8)
                .map_err(|error| format!("无法编码截图：{error}"))
                .and_then(|()| {
                    writer
                        .flush()
                        .map_err(|error| format!("无法保存截图：{error}"))
                })
        };
        // Drop the large RGBA allocation before producing metadata or returning
        // an encoding error. The renderer loads the file through the asset path.
        drop(pixels);
        if let Err(error) = encoded {
            let _ = fs::remove_file(&path);
            return Err(error);
        }

        Ok(CapturedScreen {
            window_map,
            asset_id,
            path,
            width,
            height,
            origin_x: before.origin_x,
            origin_y: before.origin_y,
            scale_factor: before.scale_factor,
            monitor_id: before.id,
            display_width: before.display_width,
            display_height: before.display_height,
            coordinate_space: before.coordinate_space,
            pixels_per_display_unit_x: f64::from(width) / f64::from(before.display_width),
            pixels_per_display_unit_y: f64::from(height) / f64::from(before.display_height),
            backend: platform::BACKEND,
        })
    }

    fn select_monitor(monitors: &[Monitor], selected_id: Option<u32>) -> Result<Monitor, String> {
        if let Some(id) = selected_id {
            return monitors
                .iter()
                .find(|monitor| monitor.id().ok() == Some(id))
                .cloned()
                .ok_or_else(|| "所选显示器已不可用，请重新选择".to_owned());
        }
        if let Some((x, y)) = platform::cursor_position() {
            if let Ok(monitor) = Monitor::from_point(x, y) {
                return Ok(monitor);
            }
        }
        monitors
            .iter()
            .find(|monitor| monitor.is_primary().unwrap_or(false))
            .or_else(|| monitors.first())
            .cloned()
            .ok_or_else(|| "当前没有可截图的显示器".to_owned())
    }

    fn monitor_info(monitor: &Monitor) -> Result<MonitorInfo, String> {
        let read = || -> xcap::XCapResult<MonitorInfo> {
            Ok(MonitorInfo {
                id: monitor.id()?,
                // friendly_name calls NSScreen with an unchecked main-thread
                // marker on macOS in xcap 0.9.8. Use the CoreGraphics name here.
                name: monitor.name()?,
                origin_x: monitor.x()?,
                origin_y: monitor.y()?,
                display_width: monitor.width()?,
                display_height: monitor.height()?,
                scale_factor: f64::from(monitor.scale_factor()?),
                rotation: monitor.rotation()?,
                is_primary: monitor.is_primary()?,
                coordinate_space: platform::COORDINATE_SPACE,
            })
        };
        read().map_err(|error| format!("无法读取显示器属性：{error}"))
    }

    fn validate_expected_size(info: &MonitorInfo) -> Result<(), String> {
        if !info.scale_factor.is_finite() || info.scale_factor <= 0.0 || info.scale_factor > 8.0 {
            return Err("显示器缩放比例无效".to_owned());
        }
        // xcap's macOS geometry uses CGDisplayBounds points; Windows uses
        // DEVMODE physical pixels. Never scale a desktop origin by local DPI.
        let factor = match info.coordinate_space {
            CoordinateSpace::PhysicalPixels => 1.0,
            CoordinateSpace::LogicalPoints => info.scale_factor,
        };
        let width = (f64::from(info.display_width) * factor).ceil();
        let height = (f64::from(info.display_height) * factor).ceil();
        if width > f64::from(MAX_SIDE) || height > f64::from(MAX_SIDE) {
            return Err("显示器分辨率超过当前截图限制".to_owned());
        }
        validate_pixel_size(width as u32, height as u32)
    }

    fn validate_pixel_size(width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("显示器尺寸无效".to_owned());
        }
        if width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return Err("显示器分辨率超过当前截图限制（最多 3355 万像素）".to_owned());
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    mod platform {
        use super::{CaptureBackend, CoordinateSpace};
        use std::ffi::c_void;

        pub(super) const BACKEND: CaptureBackend = CaptureBackend::Gdi;
        pub(super) const COORDINATE_SPACE: CoordinateSpace = CoordinateSpace::PhysicalPixels;

        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }

        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetPhysicalCursorPos(point: *mut Point) -> i32;
            fn SetThreadDpiAwarenessContext(context: *mut c_void) -> *mut c_void;
        }

        // A thread-local scope, never held across an await or moved to a worker.
        // xcap must observe physical monitor coordinates even on a worker whose
        // inherited DPI context differs from the visible Tauri window's context.
        pub(super) struct DpiScope(*mut c_void);

        impl Drop for DpiScope {
            fn drop(&mut self) {
                unsafe { SetThreadDpiAwarenessContext(self.0) };
            }
        }

        pub(super) fn enter_dpi_context() -> Result<DpiScope, String> {
            // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 is the documented -4
            // pseudo-handle. Restore the exact previous thread context on drop.
            let previous = unsafe { SetThreadDpiAwarenessContext(-4_isize as *mut c_void) };
            if previous.is_null() {
                return Err(format!(
                    "无法设置截图坐标模式：{}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(DpiScope(previous))
        }

        pub(super) fn cursor_position() -> Option<(i32, i32)> {
            let mut point = Point { x: 0, y: 0 };
            (unsafe { GetPhysicalCursorPos(&mut point) } != 0).then_some((point.x, point.y))
        }
    }

    #[cfg(target_os = "macos")]
    mod platform {
        use super::{CaptureBackend, CoordinateSpace};
        use std::ffi::c_void;

        pub(super) const BACKEND: CaptureBackend = CaptureBackend::CoreGraphics;
        pub(super) const COORDINATE_SPACE: CoordinateSpace = CoordinateSpace::LogicalPoints;

        #[repr(C)]
        struct Point {
            x: f64,
            y: f64,
        }

        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGEventCreate(source: *const c_void) -> *const c_void;
            fn CGEventGetLocation(event: *const c_void) -> Point;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRelease(object: *const c_void);
        }

        pub(super) fn enter_dpi_context() -> Result<(), String> {
            Ok(())
        }

        pub(super) fn cursor_position() -> Option<(i32, i32)> {
            let event = unsafe { CGEventCreate(std::ptr::null()) };
            if event.is_null() {
                return None;
            }
            let point = unsafe { CGEventGetLocation(event) };
            unsafe { CFRelease(event) };
            let valid = |value: f64| {
                value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX)
            };
            if !valid(point.x) || !valid(point.y) {
                return None;
            }
            Some((point.x.floor() as i32, point.y.floor() as i32))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn monitor(space: CoordinateSpace, width: u32, height: u32, scale: f64) -> MonitorInfo {
            MonitorInfo {
                id: 1,
                name: "Test display".to_owned(),
                origin_x: -3840,
                origin_y: -1080,
                display_width: width,
                display_height: height,
                scale_factor: scale,
                rotation: 0.0,
                is_primary: false,
                coordinate_space: space,
            }
        }

        #[test]
        fn bounds_allow_8k_but_reject_zero_and_large_allocations() {
            assert!(validate_pixel_size(7680, 4320).is_ok());
            assert!(validate_pixel_size(0, 1080).is_err());
            assert!(validate_pixel_size(16_384, 16_384).is_err());
            assert!(validate_pixel_size(u32::MAX, u32::MAX).is_err());
        }

        #[test]
        fn preflight_distinguishes_pixels_from_retina_points() {
            let physical = monitor(CoordinateSpace::PhysicalPixels, 7680, 4320, 2.0);
            assert!(validate_expected_size(&physical).is_ok());
            assert!(validate_expected_size(&monitor(
                CoordinateSpace::LogicalPoints,
                7680,
                4320,
                2.0
            ))
            .is_err());
            assert!(validate_expected_size(&monitor(
                CoordinateSpace::LogicalPoints,
                3840,
                2160,
                2.0
            ))
            .is_ok());
            assert_eq!((physical.origin_x, physical.origin_y), (-3840, -1080));
        }

        #[test]
        fn invalid_scale_cannot_bypass_preflight() {
            for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 9.0] {
                assert!(validate_expected_size(&monitor(
                    CoordinateSpace::LogicalPoints,
                    1920,
                    1080,
                    scale
                ))
                .is_err());
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod implementation {
    use super::*;

    pub(super) fn capture(
        _: &Path,
        _: Option<u32>,
        _: CaptureOptions,
    ) -> Result<CapturedScreen, String> {
        Err("当前平台的原生截图适配尚未完成".to_owned())
    }

    pub(super) fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
        Err("当前平台的显示器适配尚未完成".to_owned())
    }
}
