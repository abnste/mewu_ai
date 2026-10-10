// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0

//! Fixed physical-pixel region frames for manual scrolling. This module does not
//! scroll, inject input, exclude/hide windows, upload, or persist pixels. The host
//! must establish capture exclusion before constructing the source and keep it
//! valid throughout capture. Create/use/drop FrameSource on ONE blocking thread.
//!
//! Reuses a desktop DC, memory DC and top-down 32-bit DIB. BitBlt uses
//! SRCCOPY|CAPTUREBLT (as the original ScreenCaptureService does), then GdiFlush
//! before CPU access. Device alpha is ignored and output is always opaque RGBA.
//! Microsoft Learn: /windows/win32/api/{wingdi/nf-wingdi-createdibsection,
//! wingdi/nf-wingdi-bitblt,winuser/nf-winuser-getdc,
//! winuser/nf-winuser-setthreaddpiawarenesscontext}.

use image::RgbaImage;

#[cfg(windows)]
pub use implementation::FrameSource;

#[cfg(not(windows))]
pub struct FrameSource;

#[cfg(not(windows))]
impl FrameSource {
    pub fn new(_: i32, _: i32, _: u32, _: u32) -> Result<Self, String> {
        Err("区域滚动采集目前仅支持 Windows".into())
    }

    pub fn capture(&mut self) -> Result<RgbaImage, String> {
        Err("区域滚动采集目前仅支持 Windows".into())
    }
}

const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_SIDE: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FixedRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl FixedRect {
    fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
            || x.checked_add(width as i32).is_none()
            || y.checked_add(height as i32).is_none()
        {
            return Err("滚动截图区域无效或超过 4 Mi 像素限制".into());
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    fn inside(self, x: i32, y: i32, width: u32, height: u32) -> bool {
        self.x >= x
            && self.y >= y
            && i64::from(self.x) + i64::from(self.width) <= i64::from(x) + i64::from(width)
            && i64::from(self.y) + i64::from(self.height) <= i64::from(y) + i64::from(height)
    }

    fn bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

fn opaque_rgba(bgra: &[u8], rectangle: FixedRect) -> Result<RgbaImage, String> {
    let length = rectangle.bytes();
    if bgra.len() != length {
        return Err("区域采集像素长度不一致".into());
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(length)
        .map_err(|_| "没有足够内存读取滚动截图")?;
    rgba.resize(length, 0);
    for (source, destination) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
        destination.copy_from_slice(&[source[2], source[1], source[0], 255]);
    }
    RgbaImage::from_raw(rectangle.width, rectangle.height, rgba)
        .ok_or_else(|| "区域采集图片尺寸不一致".into())
}

#[cfg(windows)]
mod implementation {
    use super::*;
    use crate::capture::{CoordinateSpace, MonitorInfo};
    use std::{
        ffi::c_void,
        marker::PhantomData,
        ptr::{null_mut, NonNull},
        rc::Rc,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
        HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    };

    #[link(name = "user32")]
    extern "system" {
        fn SetThreadDpiAwarenessContext(context: *mut c_void) -> *mut c_void;
    }

    struct DpiScope {
        previous: *mut c_void,
        _thread: PhantomData<Rc<()>>,
    }

    impl DpiScope {
        fn enter() -> Result<Self, String> {
            let previous = unsafe { SetThreadDpiAwarenessContext(-4isize as *mut c_void) };
            if previous.is_null() {
                return Err("无法设置滚动截图的物理像素坐标模式".into());
            }
            Ok(Self {
                previous,
                _thread: PhantomData,
            })
        }
    }

    impl Drop for DpiScope {
        fn drop(&mut self) {
            unsafe { SetThreadDpiAwarenessContext(self.previous) };
        }
    }

    struct DesktopDc(HDC);

    impl DesktopDc {
        fn new() -> Result<Self, String> {
            let dc = unsafe { GetDC(null_mut()) };
            if dc.is_null() {
                Err("无法打开桌面采集设备".into())
            } else {
                Ok(Self(dc))
            }
        }
    }

    impl Drop for DesktopDc {
        fn drop(&mut self) {
            // Official GetDC/ReleaseDC ownership is thread-affine.
            unsafe { ReleaseDC(null_mut(), self.0) };
        }
    }

    struct Surface {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        bits: Option<NonNull<u8>>,
        rectangle: FixedRect,
    }

    impl Surface {
        fn new(source: HDC, rectangle: FixedRect) -> Result<Self, String> {
            let dc = unsafe { CreateCompatibleDC(source) };
            if dc.is_null() {
                return Err("无法创建区域采集设备".into());
            }
            // Initialize RAII before the next fallible allocation.
            let mut surface = Self {
                dc,
                bitmap: null_mut(),
                previous: null_mut(),
                bits: None,
                rectangle,
            };
            let information = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: rectangle.width as i32,
                    biHeight: -(rectangle.height as i32), // top-down, four bytes per pixel
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: rectangle.bytes() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            surface.bitmap = unsafe {
                CreateDIBSection(
                    source,
                    &information,
                    DIB_RGB_COLORS,
                    &mut bits,
                    null_mut(),
                    0,
                )
            };
            if surface.bitmap.is_null() || bits.is_null() {
                return Err("无法分配区域采集位图".into());
            }
            surface.bits = NonNull::new(bits.cast());
            surface.previous = unsafe { SelectObject(surface.dc, surface.bitmap) };
            if surface.previous.is_null() || surface.previous as isize == -1 {
                surface.previous = null_mut();
                return Err("无法选择区域采集位图".into());
            }
            Ok(surface)
        }

        fn transfer(&mut self, source: HDC, x: i32, y: i32, operation: u32) -> Result<(), String> {
            let rectangle = self.rectangle;
            if unsafe {
                BitBlt(
                    self.dc,
                    0,
                    0,
                    rectangle.width as i32,
                    rectangle.height as i32,
                    source,
                    x,
                    y,
                    operation,
                )
            } == 0
            {
                return Err("无法获取滚动截图帧".into());
            }
            // Required for any CPU access to CreateDIBSection memory after GDI.
            if unsafe { GdiFlush() } == 0 {
                return Err("无法完成滚动截图帧读取".into());
            }
            Ok(())
        }

        fn pixels(&self) -> Result<RgbaImage, String> {
            let pointer = self.bits.ok_or("区域采集位图不可用")?;
            // Allocated by CreateDIBSection with exactly this validated size; no
            // GDI commands can execute concurrently because FrameSource is !Send.
            let bytes =
                unsafe { std::slice::from_raw_parts(pointer.as_ptr(), self.rectangle.bytes()) };
            opaque_rgba(bytes, self.rectangle)
        }
    }

    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                if !self.previous.is_null() {
                    SelectObject(self.dc, self.previous);
                }
                // Deleting the memory DC also deselects the bitmap if restoration
                // unexpectedly failed. Never DeleteObject a still-selected bitmap.
                if !self.dc.is_null() {
                    DeleteDC(self.dc);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
            }
        }
    }

    fn normalize_topology(mut monitors: Vec<MonitorInfo>) -> Result<Vec<MonitorInfo>, String> {
        if monitors.is_empty() || monitors.len() > 32 {
            return Err("当前显示器配置不可用".into());
        }
        for monitor in &monitors {
            if monitor.coordinate_space != CoordinateSpace::PhysicalPixels
                || monitor.display_width == 0
                || monitor.display_height == 0
                || !monitor.scale_factor.is_finite()
                || monitor.scale_factor <= 0.0
                || monitor.scale_factor > 8.0
                || !monitor.rotation.is_finite()
                || i64::from(monitor.origin_x) + i64::from(monitor.display_width)
                    > i64::from(i32::MAX)
                || i64::from(monitor.origin_y) + i64::from(monitor.display_height)
                    > i64::from(i32::MAX)
            {
                return Err("显示器物理坐标或缩放无效".into());
            }
        }
        monitors.sort_by_key(|monitor| monitor.id);
        if monitors.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err("显示器标识不唯一，请重新开始滚动截图".into());
        }
        Ok(monitors)
    }

    fn topology() -> Result<Vec<MonitorInfo>, String> {
        // Shared native enumeration already includes physical bounds, DPI scale,
        // rotation, device name and primary selection. Sorting avoids order noise.
        normalize_topology(crate::capture::list_monitors()?)
    }

    fn region_on_one_monitor(rectangle: FixedRect, monitors: &[MonitorInfo]) -> bool {
        monitors.iter().any(|monitor| {
            rectangle.inside(
                monitor.origin_x,
                monitor.origin_y,
                monitor.display_width,
                monitor.display_height,
            )
        })
    }

    fn unchanged(expected: &[MonitorInfo], current: &[MonitorInfo]) -> Result<(), String> {
        if expected != current {
            Err("采集期间显示器设置已改变，请重新开始滚动截图".into())
        } else {
            Ok(())
        }
    }

    /// Thread-affine; construct, capture repeatedly and drop inside the same
    /// blocking worker. An error permanently invalidates this capture source.
    pub struct FrameSource {
        surface: Surface,
        desktop: DesktopDc,
        rectangle: FixedRect,
        monitors: Vec<MonitorInfo>,
        valid: bool,
        _thread: PhantomData<Rc<()>>,
    }

    impl FrameSource {
        pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, String> {
            let rectangle = FixedRect::new(x, y, width, height)?;
            let _dpi = DpiScope::enter()?;
            let monitors = topology()?;
            if !region_on_one_monitor(rectangle, &monitors) {
                return Err("滚动截图区域必须完整位于同一显示器内".into());
            }
            let desktop = DesktopDc::new()?;
            let surface = Surface::new(desktop.0, rectangle)?;
            unchanged(&monitors, &topology()?)?;
            Ok(Self {
                surface,
                desktop,
                rectangle,
                monitors,
                valid: true,
                _thread: PhantomData,
            })
        }

        pub fn capture(&mut self) -> Result<RgbaImage, String> {
            if !self.valid {
                return Err("区域采集已停止，请重新开始滚动截图".into());
            }
            // Remain invalid on EVERY early return, including a DPI or allocation
            // error. The prior DIB contents must never be returned after failure.
            self.valid = false;
            let _dpi = DpiScope::enter()?;
            unchanged(&self.monitors, &topology()?)?;
            self.surface.transfer(
                self.desktop.0,
                self.rectangle.x,
                self.rectangle.y,
                SRCCOPY | CAPTUREBLT,
            )?;
            unchanged(&self.monitors, &topology()?)?;
            let image = self.surface.pixels()?;
            self.valid = true;
            Ok(image)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn monitor(id: u32, x: i32, y: i32) -> MonitorInfo {
            MonitorInfo {
                id,
                name: format!("fixture-{id}"),
                origin_x: x,
                origin_y: y,
                display_width: 1920,
                display_height: 1080,
                scale_factor: 1.5,
                rotation: 0.0,
                is_primary: id == 1,
                coordinate_space: CoordinateSpace::PhysicalPixels,
            }
        }

        #[test]
        fn topology_changes_are_rejected_but_enumeration_order_is_irrelevant() {
            let expected =
                normalize_topology(vec![monitor(1, 0, 0), monitor(2, -1920, -100)]).unwrap();
            let reordered =
                normalize_topology(vec![monitor(2, -1920, -100), monitor(1, 0, 0)]).unwrap();
            unchanged(&expected, &reordered).unwrap();
            for field in 0..6 {
                let mut changed = expected.clone();
                match field {
                    0 => changed[0].scale_factor = 1.25,
                    1 => changed[0].rotation = 90.0,
                    2 => changed[0].display_width = 1600,
                    3 => changed[0].origin_x = 10,
                    4 => changed[0].name = "different device".into(),
                    _ => changed[0].is_primary = false,
                }
                assert!(unchanged(&expected, &changed).is_err());
            }
            assert!(unchanged(&expected, &expected[..1]).is_err());
            assert!(normalize_topology(vec![monitor(1, 0, 0), monitor(1, 0, 0)]).is_err());
            assert!(normalize_topology(Vec::new()).is_err());
        }

        #[test]
        fn region_must_fit_one_monitor_not_just_virtual_desktop_aabb() {
            let monitors = vec![monitor(1, 0, 0), monitor(2, -1920, -100)];
            assert!(region_on_one_monitor(
                FixedRect::new(-1920, -100, 1920, 1080).unwrap(),
                &monitors
            ));
            assert!(!region_on_one_monitor(
                FixedRect::new(-10, 100, 20, 20).unwrap(),
                &monitors
            ));
            assert!(!region_on_one_monitor(
                FixedRect::new(0, -100, 100, 100).unwrap(),
                &monitors
            ));
        }

        #[test]
        #[ignore = "Explicit local GDI validation using only synthetic memory surfaces; no desktop pixels"]
        fn synthetic_gdi_bitblt_reuses_dib_and_has_top_down_rgba() {
            // Null-compatible memory DC allocates resources but never calls GetDC
            // or reads the desktop. All source pixels are generated in this test.
            let rectangle = FixedRect::new(0, 0, 2, 2).unwrap();
            let source = Surface::new(null_mut(), rectangle).unwrap();
            let mut destination = Surface::new(null_mut(), rectangle).unwrap();
            let first = [0, 0, 255, 0, 0, 255, 0, 8, 255, 0, 0, 1, 10, 20, 30, 99];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    first.as_ptr(),
                    source.bits.unwrap().as_ptr(),
                    first.len(),
                );
            }
            destination.transfer(source.dc, 0, 0, SRCCOPY).unwrap();
            let image = destination.pixels().unwrap();
            assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
            assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
            assert_eq!(image.get_pixel(1, 1).0, [30, 20, 10, 255]);
            let second = [17u8; 16];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    second.as_ptr(),
                    source.bits.unwrap().as_ptr(),
                    second.len(),
                );
            }
            destination.transfer(source.dc, 0, 0, SRCCOPY).unwrap();
            assert!(destination
                .pixels()
                .unwrap()
                .pixels()
                .all(|pixel| pixel.0 == [17, 17, 17, 255]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_physical_rectangle_has_checked_bounds_and_four_mipixel_budget() {
        assert_eq!(
            FixedRect::new(-2000, -500, 2048, 2048).unwrap().bytes(),
            16 * 1024 * 1024
        );
        assert!(FixedRect::new(0, 0, 0, 20).is_err());
        assert!(FixedRect::new(0, 0, 2048, 2049).is_err());
        assert!(FixedRect::new(0, 0, MAX_SIDE + 1, 1).is_err());
        assert!(FixedRect::new(i32::MAX, 0, 1, 1).is_err());
        assert!(FixedRect::new(0, i32::MAX, 1, 1).is_err());
        assert!(FixedRect::new(i32::MIN, i32::MIN, 1, 1).is_ok());
        assert!(FixedRect::new(0, 0, u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn negative_origin_and_edges_do_not_apply_dpi_twice() {
        let rectangle = FixedRect::new(-3000, -900, 500, 600).unwrap();
        assert!(rectangle.inside(-3840, -1080, 3840, 2160));
        assert!(!rectangle.inside(0, 0, 1920, 1080));
        assert!(!rectangle.inside(-2999, -900, 500, 600));
        assert!(FixedRect::new(-3840, -1080, 100, 100)
            .unwrap()
            .inside(-3840, -1080, 3840, 2160));
    }

    #[test]
    fn pixel_conversion_ignores_gdi_alpha_and_rejects_wrong_length() {
        let rectangle = FixedRect::new(0, 0, 1, 2).unwrap();
        let image = opaque_rgba(&[10, 20, 30, 0, 90, 80, 70, 17], rectangle).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [70, 80, 90, 255]);
        assert!(opaque_rgba(&[0; 4], rectangle).is_err());
        assert!(opaque_rgba(&[0; 12], rectangle).is_err());
    }
}
