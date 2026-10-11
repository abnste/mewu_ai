// SPDX-License-Identifier: MPL-2.0
//! X11 snapshots use RandR's actual pixel rectangles, not Xft DPI-rounded
//! geometry. Wayland needs a separate consent/portal adapter and is rejected.
//! X11 GetImage ZPixmap: image byte order, visual masks and per-row padding
//! come from the server setup. No shell commands or implicit display guessing.
//! https://www.x.org/docs/XProtocol/proto.pdf

use image::RgbaImage;

const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 33_554_432;

#[derive(Clone, Copy, Debug)]
struct PixelFormat {
    bits_per_pixel: u8,
    scanline_pad: u8,
    little_endian: bool,
    masks: [u32; 3],
}

fn checked_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("显示器分辨率无效或超过当前截图限制".into());
    }
    Ok(())
}

fn decode_zpixmap(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
) -> Result<RgbaImage, String> {
    checked_size(width, height)?;
    if !matches!(format.bits_per_pixel, 16 | 24 | 32) || !matches!(format.scanline_pad, 8 | 16 | 32)
    {
        return Err("此 X11 像素格式尚未支持".into());
    }
    let valid_bits = u32::MAX >> (32 - format.bits_per_pixel);
    let [red, green, blue] = format.masks;
    if red & green != 0
        || red & blue != 0
        || green & blue != 0
        || format.masks.iter().any(|mask| {
            if *mask == 0 || *mask & !valid_bits != 0 {
                return true;
            }
            let shifted = *mask >> mask.trailing_zeros();
            shifted & shifted.wrapping_add(1) != 0
        })
    {
        return Err("X11 颜色掩码无效".into());
    }
    let pad = usize::from(format.scanline_pad);
    let row_bits = width as usize * usize::from(format.bits_per_pixel);
    let stride = row_bits.div_ceil(pad) * pad / 8;
    let expected = stride * height as usize;
    // X11 reply payloads may contain up to three protocol-padding bytes.
    if bytes.len() != expected && bytes.len() != expected.div_ceil(4) * 4 {
        return Err("X11 截图像素长度不一致".into());
    }
    let pixel_bytes = usize::from(format.bits_per_pixel) / 8;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(width as usize * height as usize * 4)
        .map_err(|_| "没有足够内存读取截图")?;
    let channel = |pixel: u32, mask: u32| -> u8 {
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        let value = (pixel & mask) >> shift;
        ((u64::from(value) * 255 + u64::from(maximum) / 2) / u64::from(maximum)) as u8
    };
    for row in bytes[..expected].chunks_exact(stride) {
        for encoded in row[..width as usize * pixel_bytes].chunks_exact(pixel_bytes) {
            let pixel = if format.little_endian {
                encoded
                    .iter()
                    .enumerate()
                    .fold(0_u32, |v, (i, b)| v | u32::from(*b) << (i * 8))
            } else {
                encoded.iter().fold(0_u32, |v, b| (v << 8) | u32::from(*b))
            };
            rgba.extend_from_slice(&[
                channel(pixel, red),
                channel(pixel, green),
                channel(pixel, blue),
                255,
            ]);
        }
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "X11 截图尺寸不一致".into())
}

fn supported_session(
    display: Option<&str>,
    wayland: Option<&str>,
    session: Option<&str>,
) -> Result<(), String> {
    if wayland.is_some_and(|v| !v.is_empty())
        || session.is_some_and(|v| v.eq_ignore_ascii_case("wayland"))
    {
        return Err("当前 Wayland 桌面尚未接入屏幕共享授权，请使用 X11 会话".into());
    }
    if display.is_none_or(|v| v.trim().is_empty()) {
        return Err("没有可用的 X11 图形会话，请从桌面启动 Mewu".into());
    }
    Ok(())
}

// The X11 client is pure Rust: keep its API and result assembly type-checked
// alongside decoder tests on other hosts, without opening an X11 connection.
#[cfg(any(target_os = "linux", test))]
mod native {
    use super::super::{
        CaptureBackend, CaptureOptions, CapturedScreen, CoordinateSpace, MonitorInfo,
    };
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::{BufWriter, Write},
        path::Path,
        sync::{Mutex, TryLockError},
    };
    use x11rb::{
        connection::Connection,
        protocol::{
            randr::ConnectionExt as _,
            xproto::{ConnectionExt as _, ImageFormat, ImageOrder, VisualClass},
        },
        rust_connection::RustConnection,
    };

    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    fn session_check() -> Result<(), String> {
        supported_session(
            std::env::var("DISPLAY").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        )
    }

    pub(crate) fn backend_name() -> &'static str {
        if session_check().is_ok() {
            "X11"
        } else {
            "Unsupported"
        }
    }

    fn connect() -> Result<(RustConnection, usize), String> {
        session_check()?;
        x11rb::connect(None).map_err(|_| "无法连接当前 X11 图形会话".into())
    }

    fn topology(connection: &RustConnection, screen: usize) -> Result<Vec<MonitorInfo>, String> {
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or("X11 屏幕不存在")?;
        let reply = connection
            .randr_get_monitors(root.root, true)
            .map_err(|_| "无法请求显示器配置")?
            .reply()
            .map_err(|_| "无法读取 RandR 显示器配置")?;
        if reply.monitors.is_empty() || reply.monitors.len() > 64 {
            return Err("显示器配置数量无效".into());
        }
        let mut monitors = Vec::with_capacity(reply.monitors.len());
        for monitor in reply.monitors {
            checked_size(u32::from(monitor.width), u32::from(monitor.height))?;
            if monitor.name == 0 {
                return Err("显示器标识无效".into());
            }
            let name = connection
                .get_atom_name(monitor.name)
                .map_err(|_| "无法读取显示器名称")?
                .reply()
                .map_err(|_| "无法读取显示器名称")?
                .name;
            if name.len() > 1024 {
                return Err("显示器名称无效".into());
            }
            monitors.push(MonitorInfo {
                id: monitor.name,
                name: String::from_utf8_lossy(&name).into_owned(),
                origin_x: i32::from(monitor.x),
                origin_y: i32::from(monitor.y),
                display_width: u32::from(monitor.width),
                display_height: u32::from(monitor.height),
                // RandR rectangles are root-window pixels. Font DPI does not
                // alter screen bounds or the PNG-to-desktop coordinate map.
                scale_factor: 1.0,
                rotation: 0.0,
                is_primary: monitor.primary,
                coordinate_space: CoordinateSpace::PhysicalPixels,
            });
        }
        monitors.sort_by_key(|monitor| monitor.id);
        if monitors.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err("显示器标识不唯一".into());
        }
        Ok(monitors)
    }

    pub(crate) fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
        let (connection, screen) = connect()?;
        topology(&connection, screen)
    }

    pub(crate) fn capture(
        asset_dir: &Path,
        selected_id: Option<u32>,
        options: CaptureOptions,
    ) -> Result<CapturedScreen, String> {
        if options.include_cursor {
            return Err("当前 Linux 截图尚不支持包含光标".into());
        }
        if !asset_dir.is_absolute() {
            return Err("截图素材目录必须是绝对路径".into());
        }
        let _guard = CAPTURE_LOCK.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => "正在获取屏幕，请稍后重试".to_owned(),
            TryLockError::Poisoned(_) => "截图服务需要重新启动".to_owned(),
        })?;
        let (connection, screen_index) = connect()?;
        let before = topology(&connection, screen_index)?;
        let root = connection
            .setup()
            .roots
            .get(screen_index)
            .ok_or("X11 屏幕不存在")?;
        let pointer = connection
            .query_pointer(root.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let monitor = if let Some(id) = selected_id {
            before
                .iter()
                .find(|monitor| monitor.id == id)
                .ok_or("所选显示器已不可用，请重新选择")?
        } else {
            pointer
                .as_ref()
                .filter(|pointer| pointer.same_screen)
                .and_then(|pointer| {
                    let x = i64::from(pointer.root_x);
                    let y = i64::from(pointer.root_y);
                    before.iter().find(|m| {
                        x >= i64::from(m.origin_x)
                            && y >= i64::from(m.origin_y)
                            && x < i64::from(m.origin_x) + i64::from(m.display_width)
                            && y < i64::from(m.origin_y) + i64::from(m.display_height)
                    })
                })
                .or_else(|| before.iter().find(|m| m.is_primary))
                .or_else(|| before.first())
                .ok_or("当前没有可截图的显示器")?
        };
        let geometry = connection
            .get_geometry(root.root)
            .map_err(|_| "无法检查桌面尺寸")?
            .reply()
            .map_err(|_| "无法检查桌面尺寸")?;
        if monitor.origin_x < 0
            || monitor.origin_y < 0
            || i64::from(monitor.origin_x) + i64::from(monitor.display_width)
                > i64::from(geometry.width)
            || i64::from(monitor.origin_y) + i64::from(monitor.display_height)
                > i64::from(geometry.height)
        {
            return Err("显示器区域不在当前 X11 桌面内".into());
        }
        let reply = connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                root.root,
                monitor.origin_x as i16,
                monitor.origin_y as i16,
                monitor.display_width as u16,
                monitor.display_height as u16,
                u32::MAX,
            )
            .map_err(|_| "无法请求 X11 截图")?
            .reply()
            .map_err(|_| "无法获取 X11 屏幕像素")?;
        let pixmap = connection
            .setup()
            .pixmap_formats
            .iter()
            .find(|format| format.depth == reply.depth)
            .ok_or("X11 像素格式不存在")?;
        let visual = root
            .allowed_depths
            .iter()
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == reply.visual)
            .ok_or("X11 截图颜色格式不存在")?;
        if visual.class != VisualClass::TRUE_COLOR {
            return Err("此 X11 调色板格式尚未支持".into());
        }
        let pixels = decode_zpixmap(
            &reply.data,
            monitor.display_width,
            monitor.display_height,
            PixelFormat {
                bits_per_pixel: pixmap.bits_per_pixel,
                scanline_pad: pixmap.scanline_pad,
                little_endian: connection.setup().image_byte_order == ImageOrder::LSB_FIRST,
                masks: [visual.red_mask, visual.green_mask, visual.blue_mask],
            },
        )?;
        if before != topology(&connection, screen_index)? {
            return Err("截图期间显示器设置已改变，请重新截图".into());
        }
        let asset_id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(asset_dir).map_err(|_| "无法创建素材目录")?;
        let path = asset_dir.join(format!("{asset_id}.png"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "无法创建截图素材")?;
        let encoded = (|| {
            use image::{
                codecs::png::{CompressionType, FilterType, PngEncoder},
                ExtendedColorType, ImageEncoder,
            };
            let mut writer = BufWriter::new(file);
            PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, FilterType::Adaptive)
                .write_image(
                    pixels.as_raw(),
                    pixels.width(),
                    pixels.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|_| "无法编码截图".to_owned())?;
            writer.flush().map_err(|_| "无法保存截图".to_owned())
        })();
        drop(pixels);
        if let Err(error) = encoded {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(CapturedScreen {
            window_map: None,
            asset_id,
            path,
            width: monitor.display_width,
            height: monitor.display_height,
            origin_x: monitor.origin_x,
            origin_y: monitor.origin_y,
            scale_factor: 1.0,
            monitor_id: monitor.id,
            display_width: monitor.display_width,
            display_height: monitor.display_height,
            coordinate_space: CoordinateSpace::PhysicalPixels,
            pixels_per_display_unit_x: 1.0,
            pixels_per_display_unit_y: 1.0,
            capture_coordinates: mewu_core::CaptureCoordinates {
                space: mewu_core::CaptureCoordinateSpace::PhysicalPixels,
                origin_x: f64::from(monitor.origin_x),
                origin_y: f64::from(monitor.origin_y),
                width: f64::from(monitor.display_width),
                height: f64::from(monitor.display_height),
            },
            backend: CaptureBackend::X11,
        })
    }
}

#[cfg(target_os = "linux")]
pub(super) use native::{backend_name, capture, list_monitors};

#[cfg(test)]
mod tests {
    use super::*;
    fn rgb(bits: u8, little_endian: bool) -> PixelFormat {
        PixelFormat {
            bits_per_pixel: bits,
            scanline_pad: 32,
            little_endian,
            masks: [0xff0000, 0xff00, 0xff],
        }
    }
    #[test]
    fn padded_24_bit_rows_and_protocol_padding_do_not_shift_the_second_row() {
        let pixels = decode_zpixmap(&[0, 0, 255, 7, 255, 0, 0, 9], 1, 2, rgb(24, true)).unwrap();
        assert_eq!(pixels.into_raw(), [255, 0, 0, 255, 0, 0, 255, 255]);
        let mut format = rgb(24, true);
        format.scanline_pad = 8;
        assert_eq!(
            decode_zpixmap(&[0, 255, 0, 1], 1, 1, format)
                .unwrap()
                .into_raw(),
            [0, 255, 0, 255]
        );
    }
    #[test]
    fn image_byte_order_and_visual_masks_determine_colors_and_ignore_device_alpha() {
        assert_eq!(
            decode_zpixmap(&[0, 255, 0, 0], 1, 1, rgb(32, false))
                .unwrap()
                .into_raw(),
            [255, 0, 0, 255]
        );
        let format = PixelFormat {
            bits_per_pixel: 16,
            scanline_pad: 16,
            little_endian: true,
            masks: [0xf800, 0x07e0, 0x001f],
        };
        assert_eq!(
            decode_zpixmap(&[0xe0, 0x07], 1, 1, format)
                .unwrap()
                .into_raw(),
            [0, 255, 0, 255]
        );
        let format = PixelFormat {
            masks: [0x0000ff, 0x00ff00, 0xff0000],
            ..rgb(32, true)
        };
        assert_eq!(
            decode_zpixmap(&[255, 0, 0, 0], 1, 1, format)
                .unwrap()
                .into_raw(),
            [255, 0, 0, 255]
        );
    }
    #[test]
    fn invalid_layouts_and_masks_fail_before_reading_or_allocating() {
        assert!(decode_zpixmap(&[], 1, 1, rgb(32, true)).is_err());
        assert!(decode_zpixmap(&[0; 4], 0, 1, rgb(32, true)).is_err());
        assert!(decode_zpixmap(&[], 16_384, 16_384, rgb(32, true)).is_err());
        for masks in [
            [0, 0xff00, 0xff],
            [0xff, 0xff, 0xff0000],
            [0xff0000, 0xff00, 0x55],
        ] {
            assert!(decode_zpixmap(
                &[0; 4],
                1,
                1,
                PixelFormat {
                    masks,
                    ..rgb(32, true)
                }
            )
            .is_err());
        }
    }
    #[test]
    fn wayland_and_missing_session_are_not_misreported_as_an_x11_desktop() {
        assert!(supported_session(Some(":0"), None, Some("x11")).is_ok());
        assert!(supported_session(None, None, None).is_err());
        assert!(supported_session(Some(":0"), Some("wayland-0"), Some("x11")).is_err());
        assert!(supported_session(Some(":0"), None, Some("Wayland")).is_err());
    }
}
