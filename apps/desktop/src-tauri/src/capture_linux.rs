// SPDX-License-Identifier: MPL-2.0
//! X11 snapshots use RandR's actual pixel rectangles, not Xft DPI-rounded
//! geometry. Wayland needs a separate consent/portal adapter and is rejected.
//! X11 GetImage ZPixmap: image byte order, visual masks and per-row padding
//! come from the server setup. No shell commands or implicit display guessing.
//! https://www.x.org/docs/XProtocol/proto.pdf

use image::RgbaImage;

#[cfg(target_os = "linux")]
#[path = "capture_linux_stream.rs"]
mod stream;

#[cfg(any(target_os = "linux", test))]
#[path = "capture_linux_visibility.rs"]
mod visibility;

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
        CaptureBackend, CaptureCheck, CaptureFile, CaptureOptions, CapturedScreen, CoordinateSpace,
        MonitorInfo, PendingCapture,
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
    };

    #[cfg(target_os = "linux")]
    type NativeConnection = super::stream::Connection;
    #[cfg(not(target_os = "linux"))]
    type NativeConnection = x11rb::rust_connection::RustConnection;

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

    fn connect() -> Result<(NativeConnection, usize), String> {
        connect_display(None)
    }

    fn connect_display(display_name: Option<&str>) -> Result<(NativeConnection, usize), String> {
        session_check()?;
        #[cfg(target_os = "linux")]
        let (connection, screen) = super::stream::connect_display(display_name)?;
        #[cfg(not(target_os = "linux"))]
        let (connection, screen) =
            x11rb::connect(display_name).map_err(|_| "无法连接当前 X11 图形会话".to_owned())?;
        let version = connection
            .randr_query_version(1, 5)
            .map_err(|_| "无法检查 RandR 截图支持")?
            .reply()
            .map_err(|_| "无法检查 RandR 截图支持")?;
        if version.major_version < 1 || (version.major_version == 1 && version.minor_version < 5) {
            return Err("当前 X11 桌面需要 RandR 1.5 显示器支持".into());
        }
        Ok((connection, screen))
    }

    #[derive(PartialEq)]
    struct Topology {
        monitors: Vec<MonitorInfo>,
        monitor_timestamp: u32,
        resource_timestamp: u32,
        config_timestamp: u32,
    }

    fn topology(connection: &NativeConnection, screen: usize) -> Result<Topology, String> {
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or("X11 屏幕不存在")?;
        // A rotation/transform can retain identical monitor rectangles. Keep
        // the server's configuration revisions as well as the visible bounds.
        let resources = connection
            .randr_get_screen_resources_current(root.root)
            .map_err(|_| "无法请求 RandR 配置版本")?
            .reply()
            .map_err(|_| "无法读取 RandR 配置版本")?;
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
        Ok(Topology {
            monitors,
            monitor_timestamp: reply.timestamp,
            resource_timestamp: resources.timestamp,
            config_timestamp: resources.config_timestamp,
        })
    }

    pub(crate) fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
        let (connection, screen) = connect()?;
        Ok(topology(&connection, screen)?.monitors)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) struct RecordingCapture {
        connection: NativeConnection,
        screen: usize,
        initial: Topology,
        hidden: super::visibility::HiddenReceipt,
        region: crate::recording_backend::Config,
    }

    #[cfg(any(target_os = "linux", test))]
    impl RecordingCapture {
        // The host supplies native identities from CaptureWitness. This method
        // receives no frontend window ids and never maps/unmaps a window.
        pub(crate) fn new(
            region: crate::recording_backend::Config,
            display: &str,
            windows: &[u32; 2],
            expires: std::time::Instant,
            allowed: &dyn Fn() -> Result<(), String>,
        ) -> Result<Self, String> {
            session_check()?;
            if display.is_empty() || display.len() > 128 || display.contains('\0') {
                return Err("录屏图形会话无效".into());
            }
            if region.width < 2
                || region.height < 2
                || region.width % 2 != 0
                || region.height % 2 != 0
                || u64::from(region.width) * u64::from(region.height) > 4_000_000
                || region
                    .x
                    .checked_add(region.width)
                    .is_none_or(|end| end > region.monitor_width)
                || region
                    .y
                    .checked_add(region.height)
                    .is_none_or(|end| end > region.monitor_height)
            {
                return Err("录屏选区无效".into());
            }
            allowed()?;
            #[cfg(target_os = "linux")]
            let (connection, screen) =
                super::stream::connect_display_until(Some(display), expires)?;
            #[cfg(not(target_os = "linux"))]
            let (connection, screen) = {
                let _ = expires;
                connect_display(Some(display))?
            };
            let mut visibility = super::visibility::HiddenWindows::subscribe(&connection, windows)?;
            visibility.wait_hidden(allowed)?;
            let hidden = visibility.receipt()?;
            drop(visibility);
            let initial = topology(&connection, screen)?;
            if !initial.monitors.iter().any(|monitor| {
                monitor.origin_x == region.origin_x
                    && monitor.origin_y == region.origin_y
                    && monitor.display_width == region.monitor_width
                    && monitor.display_height == region.monitor_height
            }) {
                return Err("录屏显示器已改变，请重新选择".into());
            }
            allowed()?;
            let capture = Self {
                connection,
                screen,
                initial,
                hidden,
                region,
            };
            capture.check(allowed)?;
            Ok(capture)
        }

        pub(crate) fn check(&self, allowed: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
            #[cfg(target_os = "linux")]
            self.connection
                .stream()
                .begin_request(std::time::Duration::from_secs(1))
                .map_err(|_| "录屏图形连接预算已结束")?;
            self.hidden.check_on(&self.connection, allowed)?;
            if self.initial != topology(&self.connection, self.screen)? {
                return Err("录屏期间显示器设置已改变，请重新选择".into());
            }
            self.hidden.check_on(&self.connection, allowed)
        }

        pub(crate) fn frame(
            &self,
            allowed: &dyn Fn() -> Result<(), String>,
        ) -> Result<RgbaImage, String> {
            self.check(allowed)?;
            let root = self
                .connection
                .setup()
                .roots
                .get(self.screen)
                .ok_or("X11 屏幕不存在")?;
            let geometry = self
                .connection
                .get_geometry(root.root)
                .map_err(|_| "无法检查录屏桌面尺寸")?
                .reply()
                .map_err(|_| "无法检查录屏桌面尺寸")?;
            let x = i64::from(self.region.origin_x) + i64::from(self.region.x);
            let y = i64::from(self.region.origin_y) + i64::from(self.region.y);
            if x < 0
                || y < 0
                || x > i64::from(i16::MAX)
                || y > i64::from(i16::MAX)
                || x + i64::from(self.region.width) > i64::from(geometry.width)
                || y + i64::from(self.region.height) > i64::from(geometry.height)
                || self.region.width > u32::from(u16::MAX)
                || self.region.height > u32::from(u16::MAX)
            {
                return Err("录屏选区已超出当前 X11 桌面".into());
            }
            self.hidden.check_on(&self.connection, allowed)?;
            // Request only the exact region. GetImage never falls back to a
            // full desktop when coordinates are stale or invalid.
            let reply = self
                .connection
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    root.root,
                    x as i16,
                    y as i16,
                    self.region.width as u16,
                    self.region.height as u16,
                    u32::MAX,
                )
                .map_err(|_| "无法请求 X11 录屏像素")?
                .reply()
                .map_err(|_| "无法读取 X11 录屏像素")?;
            self.hidden.check_on(&self.connection, allowed)?;
            let pixmap = self
                .connection
                .setup()
                .pixmap_formats
                .iter()
                .find(|format| format.depth == reply.depth)
                .ok_or("X11 录屏像素格式不存在")?;
            let visual = root
                .allowed_depths
                .iter()
                .flat_map(|depth| &depth.visuals)
                .find(|visual| visual.visual_id == reply.visual)
                .ok_or("X11 录屏颜色格式不存在")?;
            if visual.class != VisualClass::TRUE_COLOR {
                return Err("此 X11 调色板格式尚未支持".into());
            }
            let pixels = decode_zpixmap(
                &reply.data,
                self.region.width,
                self.region.height,
                PixelFormat {
                    bits_per_pixel: pixmap.bits_per_pixel,
                    scanline_pad: pixmap.scanline_pad,
                    little_endian: self.connection.setup().image_byte_order
                        == ImageOrder::LSB_FIRST,
                    masks: [visual.red_mask, visual.green_mask, visual.blue_mask],
                },
            )?;
            // Do not renew the request budget between the before/after proof.
            if self.initial != topology(&self.connection, self.screen)? {
                return Err("录屏期间显示器设置已改变，请重新选择".into());
            }
            self.hidden.check_on(&self.connection, allowed)?;
            Ok(pixels)
        }
    }

    #[cfg(all(test, target_os = "linux", feature = "linux-x11-tests"))]
    mod recording_tests {
        use super::*;
        use x11rb::protocol::xproto::{CreateWindowAux, WindowClass};

        #[test]
        #[ignore = "isolated real Xvfb recording pixel source; no audio/encoder/GUI acceptance"]
        fn x11_visibility_native_recording_exact_region_and_transient_remap() {
            let display = std::env::var("DISPLAY").unwrap();
            let (producer, screen) = connect_display(Some(&display)).unwrap();
            let root = &producer.setup().roots[screen];
            let monitor = topology(&producer, screen).unwrap().monitors.remove(0);
            let windows = [
                producer.generate_id().unwrap(),
                producer.generate_id().unwrap(),
            ];
            let painted = producer.generate_id().unwrap();
            struct Owners<'a> {
                connection: &'a NativeConnection,
                windows: [u32; 3],
            }
            impl Drop for Owners<'_> {
                fn drop(&mut self) {
                    for window in self.windows {
                        let _ = self.connection.destroy_window(window);
                    }
                    let _ = self.connection.flush();
                }
            }
            let _owners = Owners {
                connection: &producer,
                windows: [windows[0], windows[1], painted],
            };
            for &window in &windows {
                producer
                    .create_window(
                        x11rb::COPY_DEPTH_FROM_PARENT,
                        window,
                        root.root,
                        1,
                        1,
                        10,
                        10,
                        0,
                        WindowClass::INPUT_OUTPUT,
                        0,
                        &CreateWindowAux::new(),
                    )
                    .unwrap()
                    .check()
                    .unwrap();
            }
            let left = monitor.origin_x + 20;
            let top = monitor.origin_y + 30;
            producer
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    painted,
                    root.root,
                    left as i16,
                    top as i16,
                    64,
                    48,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new().background_pixel(root.white_pixel),
                )
                .unwrap()
                .check()
                .unwrap();
            producer.map_window(painted).unwrap().check().unwrap();
            let config = crate::recording_backend::Config {
                output: std::path::PathBuf::from("/tmp/not-created-native-recording-test.mp4"),
                origin_x: monitor.origin_x,
                origin_y: monitor.origin_y,
                monitor_width: monitor.display_width,
                monitor_height: monitor.display_height,
                x: 20,
                y: 30,
                width: 64,
                height: 48,
                audio: crate::recording_audio::AudioMode::Mute,
            };
            let capture = RecordingCapture::new(
                config,
                &display,
                &windows,
                std::time::Instant::now() + std::time::Duration::from_secs(10),
                &|| Ok(()),
            )
            .unwrap();
            let frame = capture.frame(&|| Ok(())).unwrap();
            assert_eq!(frame.dimensions(), (64, 48));
            assert!(frame.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
            producer.map_window(windows[0]).unwrap().check().unwrap();
            producer.unmap_window(windows[0]).unwrap().check().unwrap();
            assert!(
                capture.frame(&|| Ok(())).is_err(),
                "same subscribed connection must remember even a transient remap"
            );
        }
    }

    pub(crate) fn capture(
        asset_dir: &Path,
        selected_id: Option<u32>,
        options: CaptureOptions,
    ) -> Result<CapturedScreen, String> {
        capture_inner(
            asset_dir,
            selected_id,
            options,
            None,
            &[],
            std::sync::Arc::new(|| Ok(())),
        )
        .map(PendingCapture::into_retained)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn capture_with_witnesses(
        asset_dir: &Path,
        options: CaptureOptions,
        witnesses: &[crate::window_witness_linux::CaptureWitness],
        allowed: CaptureCheck,
    ) -> Result<PendingCapture, String> {
        if witnesses.len() != 2 || witnesses[0].display() != witnesses[1].display() {
            return Err("截图窗口不属于同一个完整桌面会话".into());
        }
        for witness in witnesses {
            crate::window_witness_linux::current(witness)?;
        }
        let windows: Vec<_> = witnesses
            .iter()
            .map(|witness| witness.native_id())
            .collect();
        capture_inner(
            asset_dir,
            None,
            options,
            Some(witnesses[0].display()),
            &windows,
            allowed,
        )
    }

    fn capture_inner(
        asset_dir: &Path,
        selected_id: Option<u32>,
        options: CaptureOptions,
        display_name: Option<&str>,
        windows: &[u32],
        allowed: CaptureCheck,
    ) -> Result<PendingCapture, String> {
        allowed()?;
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
        let (connection, screen_index) = connect_display(display_name)?;
        let mut visibility = if windows.is_empty() {
            None
        } else {
            Some(super::visibility::HiddenWindows::subscribe(
                &connection,
                windows,
            )?)
        };
        if let Some(visibility) = &mut visibility {
            visibility.wait_hidden(allowed.as_ref())?;
        }
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
                .monitors
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
                    before.monitors.iter().find(|m| {
                        x >= i64::from(m.origin_x)
                            && y >= i64::from(m.origin_y)
                            && x < i64::from(m.origin_x) + i64::from(m.display_width)
                            && y < i64::from(m.origin_y) + i64::from(m.display_height)
                    })
                })
                .or_else(|| before.monitors.iter().find(|m| m.is_primary))
                .or_else(|| before.monitors.first())
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
        allowed()?;
        if let Some(visibility) = &visibility {
            visibility.check(allowed.as_ref())?;
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
        if let Some(visibility) = &visibility {
            visibility.check(allowed.as_ref())?;
        }
        allowed()?;
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
        if let Some(visibility) = &visibility {
            visibility.check(allowed.as_ref())?;
        }
        allowed()?;
        let asset_id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(asset_dir).map_err(|_| "无法创建素材目录")?;
        let path = asset_dir.join(format!("{asset_id}.png"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "无法创建截图素材")?;
        let owned_file = CaptureFile::created(path.clone());
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
        encoded?;
        if let Some(visibility) = &visibility {
            visibility.check(allowed.as_ref())?;
        }
        allowed()?;
        let receipt = visibility
            .as_ref()
            .map(|visibility| visibility.receipt())
            .transpose()?;
        drop(visibility);
        // Preserve this exact connection and its subscribed event queue across
        // the async UI layout. Reconnecting would lose transient map events.
        let native_check = receipt.map(|receipt| {
            std::sync::Arc::new(move || receipt.check_on(&connection, allowed.as_ref()))
                as CaptureCheck
        });
        Ok(PendingCapture {
            file: owned_file,
            native_check,
            screen: CapturedScreen {
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
            },
        })
    }
}

#[cfg(target_os = "linux")]
pub(super) use native::{backend_name, capture, list_monitors};

#[cfg(target_os = "linux")]
pub(super) use native::capture_with_witnesses;

#[cfg(target_os = "linux")]
pub(super) use native::RecordingCapture;

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
