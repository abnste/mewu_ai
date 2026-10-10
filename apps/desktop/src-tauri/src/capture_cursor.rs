// SPDX-License-Identifier: MPL-2.0
//! Screenshot cursor compositor. Tests use owned synthetic bitmaps and cursors.
//! Windows caller keeps its existing PMv2 DPI scope on this same blocking thread.
//! Paint native cursor onto a bounded patch of actual opaque screenshot pixels.
//! DrawIconEx handles alpha and legacy AND/XOR; do not flatten cursor on transparent RGBA.

const MAX_CURSOR_SIDE: u32 = 1024;

/// GDI BI_RGB's fourth byte is reserved, unlike meaningful RGBA alpha.
/// Call only for Windows full-desktop GDI output, before cursor composition.
/// Do not apply this to arbitrary image assets, transparent PNGs, or macOS frames.
pub(crate) fn opaque_gdi_screenshot_alpha(rgba: &mut [u8]) -> Result<(), String> {
    if rgba.len() % 4 != 0 {
        return Err("截图像素长度无效".into());
    }
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorShape {
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
}
impl CursorShape {
    fn validate(self) -> Result<Self, String> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_CURSOR_SIDE
            || self.height > MAX_CURSOR_SIDE
            || self.hotspot_x >= self.width
            || self.hotspot_y >= self.height
        {
            return Err("光标尺寸无效".into());
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorPatch {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    draw_x: i32,
    draw_y: i32,
}

/// All coordinates and dimensions are physical pixels, independent of monitor DPI.
/// A hotspot outside this image may still have a visible clipped cursor footprint.
fn patch_bounds(
    image_width: u32,
    image_height: u32,
    origin: (i32, i32),
    position: (i32, i32),
    shape: CursorShape,
) -> Result<Option<CursorPatch>, String> {
    let shape = shape.validate()?;
    if image_width == 0 || image_height == 0 {
        return Err("截图尺寸无效".into());
    }
    let left = i64::from(position.0) - i64::from(origin.0) - i64::from(shape.hotspot_x);
    let top = i64::from(position.1) - i64::from(origin.1) - i64::from(shape.hotspot_y);
    let right = left + i64::from(shape.width);
    let bottom = top + i64::from(shape.height);
    let x = left.max(0);
    let y = top.max(0);
    let end_x = right.min(i64::from(image_width));
    let end_y = bottom.min(i64::from(image_height));
    if x >= end_x || y >= end_y {
        return Ok(None);
    }
    Ok(Some(CursorPatch {
        x: u32::try_from(x).map_err(|_| "光标位置无效")?,
        y: u32::try_from(y).map_err(|_| "光标位置无效")?,
        width: u32::try_from(end_x - x).map_err(|_| "光标位置无效")?,
        height: u32::try_from(end_y - y).map_err(|_| "光标位置无效")?,
        draw_x: i32::try_from(left - x).map_err(|_| "光标位置无效")?,
        draw_y: i32::try_from(top - y).map_err(|_| "光标位置无效")?,
    }))
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ffi::c_void, mem::size_of, ptr};
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetObjectW,
            SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
            HDC, HGDIOBJ,
        },
        UI::WindowsAndMessaging::{
            CopyIcon, DestroyIcon, DrawIconEx, GetCursorInfo, GetIconInfo, GetPhysicalCursorPos,
            CURSORINFO, CURSOR_SHOWING, CURSOR_SUPPRESSED, DI_NOMIRROR, DI_NORMAL, HICON, ICONINFO,
        },
    };

    fn failed(stage: &str) -> String {
        format!(
            "无法合成截图光标（{stage}）：{}",
            std::io::Error::last_os_error()
        )
    }

    struct OwnedIcon(HICON);
    impl Drop for OwnedIcon {
        fn drop(&mut self) {
            unsafe { DestroyIcon(self.0) };
        }
    }

    struct IconBitmaps(ICONINFO);
    impl Drop for IconBitmaps {
        fn drop(&mut self) {
            for bitmap in [self.0.hbmMask, self.0.hbmColor] {
                if !bitmap.is_null() {
                    unsafe { DeleteObject(bitmap) };
                }
            }
        }
    }

    fn bitmap_size(bitmap: HBITMAP) -> Result<(u32, u32), String> {
        if bitmap.is_null() {
            return Err("光标位图不可用".into());
        }
        let mut value = BITMAP::default();
        if unsafe {
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                (&mut value as *mut BITMAP).cast(),
            )
        } != size_of::<BITMAP>() as i32
        {
            return Err(failed("读取尺寸"));
        }
        if value.bmWidth <= 0 || value.bmHeight <= 0 {
            return Err("光标位图尺寸无效".into());
        }
        Ok((value.bmWidth as u32, value.bmHeight as u32))
    }

    /// Owned icon cannot cross a blocking-worker/thread/await boundary.
    /// This snapshot is sampled immediately after the GDI image call, before
    /// slower window-map metadata or encoding. OS image/cursor sampling is not atomic.
    pub(crate) struct CursorSnapshot {
        icon: OwnedIcon,
        position: (i32, i32),
        shape: CursorShape,
    }
    impl CursorSnapshot {
        /// Call only when include_cursor=true, within the capture worker's PMv2 scope.
        pub(crate) fn sample() -> Result<Option<Self>, String> {
            let mut info = CURSORINFO {
                cbSize: size_of::<CURSORINFO>() as u32,
                ..CURSORINFO::default()
            };
            if unsafe { GetCursorInfo(&mut info) } == 0 {
                return Err(failed("读取状态"));
            }
            if info.flags & CURSOR_SHOWING == 0 || info.flags & CURSOR_SUPPRESSED != 0 {
                return Ok(None);
            }
            if info.hCursor.is_null() {
                return Err("截图光标不可用".into());
            }
            // Read explicitly physical position instead of assuming the DPI
            // interpretation of CURSORINFO::ptScreenPos on another calling thread.
            let mut position = POINT::default();
            if unsafe { GetPhysicalCursorPos(&mut position) } == 0 {
                return Err(failed("读取位置"));
            }
            // Borrowed global cursor is never destroyed. CopyIcon gives this worker
            // an owned resource; GetIconInfo bitmaps are individually released below.
            let copy = unsafe { CopyIcon(info.hCursor) };
            if copy.is_null() {
                return Err(failed("复制光标"));
            }
            Self::from_owned(OwnedIcon(copy), (position.x, position.y)).map(Some)
        }

        fn from_owned(icon: OwnedIcon, position: (i32, i32)) -> Result<Self, String> {
            let mut info = ICONINFO::default();
            if unsafe { GetIconInfo(icon.0, &mut info) } == 0 {
                return Err(failed("读取热点"));
            }
            let info = IconBitmaps(info);
            if info.0.fIcon != 0 {
                return Err("截图光标类型无效".into());
            }
            let mask = bitmap_size(info.0.hbmMask)?;
            let (width, height) = if info.0.hbmColor.is_null() {
                if mask.1 % 2 != 0 {
                    return Err("黑白光标位图无效".into());
                }
                (mask.0, mask.1 / 2)
            } else {
                let color = bitmap_size(info.0.hbmColor)?;
                if color != mask {
                    return Err("彩色光标位图无效".into());
                }
                color
            };
            let shape = CursorShape {
                width,
                height,
                hotspot_x: info.0.xHotspot,
                hotspot_y: info.0.yHotspot,
            }
            .validate()?;
            Ok(Self {
                icon,
                position,
                shape,
            })
        }

        pub(crate) fn compose_into(
            &self,
            rgba: &mut [u8],
            width: u32,
            height: u32,
            origin: (i32, i32),
        ) -> Result<(), String> {
            let length = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|value| value.checked_mul(4))
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("截图像素长度无效")?;
            if rgba.len() != length {
                return Err("截图像素长度不一致".into());
            }
            let Some(bounds) = patch_bounds(width, height, origin, self.position, self.shape)?
            else {
                return Ok(());
            };
            let mut patch = NativePatch::new(bounds.width, bounds.height)?;
            // Load actual captured background as opaque BGRA. Native AND/XOR
            // needs the destination RGB; drawing to transparent black is incorrect.
            for y in 0..bounds.height as usize {
                for x in 0..bounds.width as usize {
                    let source =
                        ((bounds.y as usize + y) * width as usize + bounds.x as usize + x) * 4;
                    let target = (y * bounds.width as usize + x) * 4;
                    patch.bytes_mut()[target..target + 4].copy_from_slice(&[
                        rgba[source + 2],
                        rgba[source + 1],
                        rgba[source],
                        255,
                    ]);
                }
            }
            if unsafe {
                DrawIconEx(
                    patch.dc,
                    bounds.draw_x,
                    bounds.draw_y,
                    self.icon.0,
                    self.shape.width as i32,
                    self.shape.height as i32,
                    0,
                    ptr::null_mut(),
                    DI_NORMAL | DI_NOMIRROR,
                )
            } == 0
            {
                return Err(failed("绘制光标"));
            }
            // DIB direct memory access must follow the GDI completion boundary.
            if unsafe { GdiFlush() } == 0 {
                return Err(failed("完成绘制"));
            }
            for y in 0..bounds.height as usize {
                for x in 0..bounds.width as usize {
                    let target =
                        ((bounds.y as usize + y) * width as usize + bounds.x as usize + x) * 4;
                    let source = (y * bounds.width as usize + x) * 4;
                    let bgra = &patch.bytes()[source..source + 4];
                    rgba[target..target + 3].copy_from_slice(&[bgra[2], bgra[1], bgra[0]]);
                    // Keep original screenshot alpha. GDI raster operations do not
                    // guarantee a meaningful alpha channel in the destination DIB.
                }
            }
            Ok(())
        }
    }

    struct NativePatch {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        pixels: *mut u8,
        length: usize,
    }
    impl NativePatch {
        fn new(width: u32, height: u32) -> Result<Self, String> {
            if width == 0 || height == 0 || width > MAX_CURSOR_SIDE || height > MAX_CURSOR_SIDE {
                return Err("光标合成区域无效".into());
            }
            let mut result = Self {
                dc: unsafe { CreateCompatibleDC(ptr::null_mut()) },
                bitmap: ptr::null_mut(),
                previous: ptr::null_mut(),
                pixels: ptr::null_mut(),
                length: width as usize * height as usize * 4,
            };
            if result.dc.is_null() {
                return Err(failed("创建画布"));
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: result.length as u32,
                    ..BITMAPINFOHEADER::default()
                },
                ..BITMAPINFO::default()
            };
            let mut bits: *mut c_void = ptr::null_mut();
            result.bitmap = unsafe {
                CreateDIBSection(
                    result.dc,
                    &info,
                    DIB_RGB_COLORS,
                    &mut bits,
                    ptr::null_mut(),
                    0,
                )
            };
            if result.bitmap.is_null() || bits.is_null() {
                return Err(failed("创建像素画布"));
            }
            result.pixels = bits.cast();
            let previous = unsafe { SelectObject(result.dc, result.bitmap) };
            if previous.is_null() || previous as isize == -1 {
                return Err(failed("选择画布"));
            }
            result.previous = previous;
            Ok(result)
        }
        fn bytes(&self) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.pixels, self.length) }
        }
        fn bytes_mut(&mut self) -> &mut [u8] {
            unsafe { std::slice::from_raw_parts_mut(self.pixels, self.length) }
        }
    }
    impl Drop for NativePatch {
        fn drop(&mut self) {
            if !self.dc.is_null() {
                unsafe { GdiFlush() };
                if !self.previous.is_null() {
                    unsafe { SelectObject(self.dc, self.previous) };
                }
            }
            if !self.dc.is_null() {
                unsafe { DeleteDC(self.dc) };
            }
            // Destroy the DC first so even a failed restore cannot leave our
            // bitmap selected and make DeleteObject silently leak the patch.
            if !self.bitmap.is_null() {
                unsafe { DeleteObject(self.bitmap) };
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::{
            Graphics::Gdi::CreateBitmap, UI::WindowsAndMessaging::CreateIconIndirect,
        };

        fn synthetic_cursor(mask: HBITMAP, color: HBITMAP, hotspot: (u32, u32)) -> OwnedIcon {
            assert!(!mask.is_null());
            let info = IconBitmaps(ICONINFO {
                fIcon: 0,
                xHotspot: hotspot.0,
                yHotspot: hotspot.1,
                hbmMask: mask,
                hbmColor: color,
            });
            let icon = unsafe { CreateIconIndirect(&info.0) };
            assert!(!icon.is_null());
            OwnedIcon(icon)
        }

        #[test]
        fn synthetic_monochrome_and_xor_draw_on_real_background() {
            // Four distinct classic cursor operations in top two columns:
            // AND/XOR (1,0) keep, (0,0) black, (0,1) white, (1,1) invert.
            // WORD-aligned monochrome rows; first half AND, second half XOR.
            let bits = [0x80_u8, 0, 0x40, 0, 0, 0, 0xc0, 0];
            let mask = unsafe { CreateBitmap(2, 4, 1, 1, bits.as_ptr().cast()) };
            let icon = synthetic_cursor(mask, ptr::null_mut(), (1, 1));
            let cursor = CursorSnapshot::from_owned(icon, (-119, -79)).unwrap();
            assert_eq!(cursor.shape.width, 2);
            assert_eq!(cursor.shape.height, 2);
            let mut pixels = [20_u8, 40, 80, 255].repeat(16);
            cursor.compose_into(&mut pixels, 4, 4, (-120, -80)).unwrap();
            assert_eq!(&pixels[0..4], &[20, 40, 80, 255]);
            assert_eq!(&pixels[4..8], &[0, 0, 0, 255]);
            assert_eq!(&pixels[16..20], &[255, 255, 255, 255]);
            assert_eq!(&pixels[20..24], &[235, 215, 175, 255]);
            assert_eq!(&pixels[60..64], &[20, 40, 80, 255]);
        }

        #[test]
        fn synthetic_alpha_cursor_keeps_black_edges_and_clips_hotspot() {
            let mut color = NativePatch::new(2, 2).unwrap();
            // CreateIconIndirect fixture input uses straight BGRA. DrawIconEx
            // applies its alpha during rendering; pre-multiplying this input
            // would darken the half-alpha red a second time (R74 instead of R138).
            color
                .bytes_mut()
                .copy_from_slice(&[0, 0, 0, 255, 0, 0, 255, 128, 0, 255, 0, 255, 0, 0, 0, 0]);
            unsafe { SelectObject(color.dc, color.previous) };
            color.previous = ptr::null_mut();
            let mask_bits = [0_u8; 4];
            let mask = unsafe { CreateBitmap(2, 2, 1, 1, mask_bits.as_ptr().cast()) };
            // CreateIconIndirect copies the unselected bitmap; color remains ours.
            let icon = synthetic_cursor(mask, color.bitmap, (1, 1));
            // synthetic_cursor released both original bitmaps after the copy.
            color.bitmap = ptr::null_mut();
            let mut cursor = CursorSnapshot::from_owned(icon, (-99, -59)).unwrap();
            let mut pixels = [20_u8, 40, 80, 255].repeat(16);
            cursor.compose_into(&mut pixels, 4, 4, (-100, -60)).unwrap();
            assert_eq!(&pixels[0..4], &[0, 0, 0, 255]);
            let blended = &pixels[4..8];
            assert_eq!(blended, &[138, 20, 40, 255]);
            assert_eq!(&pixels[16..20], &[0, 255, 0, 255]);
            assert_eq!(&pixels[20..24], &[20, 40, 80, 255]);
            assert_eq!(&pixels[60..64], &[20, 40, 80, 255]);
            // Move the hotspot onto the left edge: column zero is clipped,
            // so the half-alpha red pixel is now the only changed pixel.
            cursor.position = (-100, -59);
            let mut clipped = [20_u8, 40, 80, 255].repeat(16);
            cursor
                .compose_into(&mut clipped, 4, 4, (-100, -60))
                .unwrap();
            assert_eq!(&clipped[0..4], blended);
            assert!(clipped[4..].chunks_exact(4).all(|p| p == [20, 40, 80, 255]));
            let mut malformed = [1, 2, 3];
            assert!(cursor
                .compose_into(&mut malformed, 4, 4, (-100, -60))
                .is_err());
            assert_eq!(malformed, [1, 2, 3]);
        }
    }
}

#[cfg(windows)]
pub(crate) use native::CursorSnapshot;

#[cfg(test)]
mod geometry_tests {
    use super::*;
    fn shape() -> CursorShape {
        CursorShape {
            width: 32,
            height: 48,
            hotspot_x: 7,
            hotspot_y: 11,
        }
    }
    #[test]
    fn gdi_reserved_byte_becomes_opaque_without_changing_rgb() {
        let mut bytes = [0, 20, 40, 0, 128, 64, 32, 7, 255, 250, 245, 255];
        opaque_gdi_screenshot_alpha(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            [0, 20, 40, 255, 128, 64, 32, 255, 255, 250, 245, 255]
        );
        let mut malformed = [1, 2, 3];
        assert!(opaque_gdi_screenshot_alpha(&mut malformed).is_err());
        assert_eq!(malformed, [1, 2, 3]);
    }
    #[test]
    fn physical_negative_origin_never_uses_scale_factor() {
        let bounds = patch_bounds(3840, 2160, (-3840, -1080), (-3800, -1000), shape())
            .unwrap()
            .unwrap();
        assert_eq!(
            bounds,
            CursorPatch {
                x: 33,
                y: 69,
                width: 32,
                height: 48,
                draw_x: 0,
                draw_y: 0
            }
        );
        // A 150% or 200% monitor still has these exact physical pixel coordinates.
    }
    #[test]
    fn hotspot_outside_image_still_composes_intersecting_footprint() {
        let bounds = patch_bounds(100, 100, (-100, -100), (2, -70), shape())
            .unwrap()
            .unwrap();
        assert_eq!(bounds.x, 95);
        assert_eq!(bounds.width, 5);
        assert_eq!(bounds.y, 19);
        assert_eq!(bounds.draw_x, 0);
        let bounds = patch_bounds(100, 100, (-100, -100), (-102, -102), shape())
            .unwrap()
            .unwrap();
        assert_eq!(
            (bounds.x, bounds.y, bounds.draw_x, bounds.draw_y),
            (0, 0, -9, -13)
        );
    }
    #[test]
    fn disjoint_and_extreme_desktop_positions_are_safe() {
        assert!(patch_bounds(
            100,
            100,
            (i32::MIN, i32::MIN),
            (i32::MAX, i32::MAX),
            shape()
        )
        .unwrap()
        .is_none());
        assert!(patch_bounds(100, 100, (0, 0), (-100, -100), shape())
            .unwrap()
            .is_none());
        for invalid in [
            CursorShape {
                width: 0,
                ..shape()
            },
            CursorShape {
                width: 1025,
                ..shape()
            },
            CursorShape {
                hotspot_x: 32,
                ..shape()
            },
        ] {
            assert!(patch_bounds(100, 100, (0, 0), (0, 0), invalid).is_err());
        }
    }
}
