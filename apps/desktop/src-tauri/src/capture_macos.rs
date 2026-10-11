//! Native, single-display screenshots. Call only from a blocking worker while
//! the parent's capture/scene transition permit remains held until this returns.
//! This module never requests permission, changes windows, or writes a file.

#![cfg(target_os = "macos")]

use crate::capture::CaptureCheck;
use block2::RcBlock;
use image::RgbaImage;
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::AnyClass,
    AnyThread, MainThreadMarker,
};
use objc2_core_graphics::{
    CGBitmapInfo, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_foundation::{NSArray, NSError, NSOperatingSystemVersion, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    SCWindow,
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc, Condvar, Mutex,
    },
};

const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 33_554_432;
const MAX_DISPLAYS: usize = 64;
const MAX_WINDOWS: usize = 4096;
const MAX_EXCLUDED_WINDOWS: usize = 2;
const MAX_TITLE_BYTES: usize = 4096;
const MAX_ROW_PADDING: usize = 4096;
const MAX_NATIVE_BYTES: usize = MAX_PIXELS as usize * 4 + MAX_SIDE as usize * MAX_ROW_PADDING;
const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// A native window identity obtained by the host's generation-aware witness.
/// This is an internal host contract, never caller-supplied IPC or a fixed ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExcludedWindow {
    pub(crate) id: u32,
    pub(crate) title: String,
}

/// The single-frame manager is available on macOS 14+. ScreenCaptureKit itself
/// exists on 12.3, so linking it does not justify sending the newer selector.
/// https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager
pub(crate) fn available() -> bool {
    autoreleasepool(|_| {
        NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
            majorVersion: 14,
            minorVersion: 0,
            patchVersion: 0,
        }) && AnyClass::get(c"SCScreenshotManager").is_some()
    })
}

/// Captures the host-selected display at an explicitly bounded output size.
/// The requested dimensions are image pixels; this API takes no desktop rect
/// or origin, and never scales a global origin by a display's Retina factor.
pub(crate) fn capture(
    display_id: u32,
    width_pixels: u32,
    height_pixels: u32,
    exclusions: &[ExcludedWindow],
    allowed: CaptureCheck,
) -> Result<RgbaImage, String> {
    if MainThreadMarker::new().is_some() {
        return Err("Mac 原生截图必须由后台截图任务调用".to_owned());
    }
    validate_dimensions(width_pixels, height_pixels)?;
    validate_exclusions(exclusions)?;
    if display_id == 0 {
        return Err("所选显示器标识无效，请重新选择".to_owned());
    }
    if !available() {
        return Err("当前 Mac 系统不支持原生单帧截图，需要 macOS 14 或更新版本".to_owned());
    }
    crate::capture_permission_macos::preflight()?;

    let request = Request {
        display_id,
        width: width_pixels,
        height: height_pixels,
        pid: i32::try_from(std::process::id()).map_err(|_| "当前应用进程标识无效")?,
        exclusions: exclusions.to_vec(),
        allowed,
    };
    let admission = request.allowed.clone();
    let (sender, receiver) = mpsc::channel();
    let native_lifetime = Arc::new(NativeLifetime::default());
    let outer_lease = NativeBlockLease::retain(&native_lifetime)?;
    let callback_lifetime = native_lifetime.clone();
    let admitted = Arc::new(AtomicBool::new(false));
    let completion = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let _native_block_lease = &outer_lease;
            if admitted.swap(true, Ordering::AcqRel) {
                return;
            }
            let result = catch_unwind(AssertUnwindSafe(|| {
                autoreleasepool(|_| {
                    (request.allowed)()?;
                    // Apple provides these borrowed pointers only for this callback.
                    // A successful object is retained before issuing the next call.
                    if let Some(error) = unsafe { error.as_ref() } {
                        return Err(native_error("无法读取可截图显示器", error));
                    }
                    let content = unsafe { Retained::retain(content) }
                        .ok_or_else(|| "系统未返回可截图显示器".to_owned())?;
                    begin_screenshot(content, request.clone(), sender.clone(), &callback_lifetime)
                })
            }))
            .unwrap_or_else(|_| Err("读取原生截图内容失败".to_owned()));
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        },
    );

    autoreleasepool(|_| -> Result<(), String> {
        // Include off-screen windows so the host's exact presentation-window
        // exclusions can still be found after its acknowledged hide operation.
        // https://developer.apple.com/documentation/screencapturekit/scshareablecontent/getexcludingdesktopwindows(_:onscreenwindowsonly:completionhandler:)
        admission()?;
        unsafe {
            SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
                false,
                false,
                &completion,
            );
        }
        Ok(())
    })?;
    // The system has copied the escaping heap block. Release our local copy so
    // the lease observes the final native release rather than our own receiver.
    drop(completion);

    // No timeout: a timed-out Rust task would release the parent's permit while
    // native work remains alive. The UI stays responsive because this caller is
    // a blocking worker. Never wait inside the shareable-content callback: the
    // image completion may use the same system callback queue.
    let result = receiver
        .recv()
        .unwrap_or_else(|_| Err("原生截图回调已终止，请重试".to_owned()));
    native_lifetime.wait_released();
    let pixels = result?;
    admission()?;
    Ok(pixels)
}

#[derive(Default)]
struct NativeLifetime {
    blocks: Mutex<usize>,
    released: Condvar,
}

impl NativeLifetime {
    fn wait_released(&self) {
        let mut blocks = self
            .blocks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *blocks != 0 {
            blocks = self
                .released
                .wait(blocks)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

struct NativeBlockLease {
    lifetime: Arc<NativeLifetime>,
}

impl NativeBlockLease {
    fn retain(lifetime: &Arc<NativeLifetime>) -> Result<Self, String> {
        let mut blocks = lifetime
            .blocks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *blocks = blocks
            .checked_add(1)
            .ok_or_else(|| "原生截图回调数量超出限制".to_owned())?;
        Ok(Self {
            lifetime: lifetime.clone(),
        })
    }
}

impl Drop for NativeBlockLease {
    fn drop(&mut self) {
        let mut blocks = self
            .lifetime
            .blocks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Each lease is created once, is not Clone, and is dropped exactly once
        // by the final release of its RcBlock closure. No completion flag can
        // substitute for the native block's actual destruction.
        *blocks -= 1;
        self.lifetime.released.notify_all();
    }
}

#[derive(Clone)]
struct Request {
    display_id: u32,
    width: u32,
    height: u32,
    pid: i32,
    exclusions: Vec<ExcludedWindow>,
    allowed: CaptureCheck,
}

fn begin_screenshot(
    content: Retained<SCShareableContent>,
    request: Request,
    sender: Sender<Result<RgbaImage, String>>,
    native_lifetime: &Arc<NativeLifetime>,
) -> Result<(), String> {
    crate::capture_permission_macos::preflight()?;
    let displays = unsafe { content.displays() };
    if displays.count() == 0 || displays.count() > MAX_DISPLAYS {
        return Err("系统返回的显示器数量无效".to_owned());
    }
    let mut selected: Option<Retained<SCDisplay>> = None;
    for index in 0..displays.count() {
        let display = displays.objectAtIndex(index);
        if unsafe { display.displayID() } == request.display_id {
            if selected.is_some() {
                return Err("所选显示器标识不唯一，请重新选择".to_owned());
            }
            validate_display_size(
                unsafe { display.width() },
                unsafe { display.height() },
                request.width,
                request.height,
            )?;
            selected = Some(display);
        }
    }
    let display = selected.ok_or_else(|| "所选显示器已不可用，请重新选择".to_owned())?;

    let windows = unsafe { content.windows() };
    if windows.count() > MAX_WINDOWS {
        return Err("系统返回的窗口数量超出截图限制".to_owned());
    }
    let mut selected_windows: Vec<Retained<SCWindow>> = Vec::new();
    let mut found_ids = [false; MAX_EXCLUDED_WINDOWS];
    for index in 0..windows.count() {
        let window = windows.objectAtIndex(index);
        let id = unsafe { window.windowID() };
        let Some(position) = request
            .exclusions
            .iter()
            .position(|requested| requested.id == id)
        else {
            continue;
        };
        if found_ids[position] {
            return Err("需排除的截图窗口标识不唯一，请重试".to_owned());
        }
        let owner = unsafe { window.owningApplication() }
            .ok_or_else(|| "无法确认需排除的截图窗口归属".to_owned())?;
        let title =
            unsafe { window.title() }.ok_or_else(|| "无法确认需排除的截图窗口标题".to_owned())?;
        if title.length() > MAX_TITLE_BYTES {
            return Err("需排除的截图窗口标题超出限制".to_owned());
        }
        validate_matching_window(
            &request.exclusions[position],
            unsafe { owner.processID() },
            &title.to_string(),
            request.pid,
        )?;
        found_ids[position] = true;
        selected_windows.push(window);
    }
    // Hidden windows may be absent from SCShareableContent; those IDs need no
    // native exclusion. The host must already have proved the corresponding
    // window generations hidden while holding its scene-transition lease. An
    // ID alone is not a creation fingerprint and cannot authorize replacement.
    let excluded = NSArray::from_retained_slice(&selected_windows);
    // Only the host-witnessed space/frozen presentation windows are excluded.
    // Independently visible Mewu settings remain part of a normal desktop shot.
    // https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(display:excludingwindows:)
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &excluded,
        )
    };
    let configuration = unsafe { SCStreamConfiguration::new() };
    unsafe {
        configuration.setWidth(request.width as usize);
        configuration.setHeight(request.height as usize);
        configuration.setPixelFormat(BGRA);
        configuration.setShowsCursor(false);
    }

    // These immutable ScreenCaptureKit inputs are made and used within Apple's
    // callback chain. The copied completion block holds strong ARC references
    // until native completion; no native objects/pointers cross a Rust channel
    // and no unsafe Send implementation or AppKit UI dispatch is involved.
    let keepalive = (
        content,
        display,
        excluded,
        filter.clone(),
        configuration.clone(),
        // Rust drops tuple fields in declaration order: release every native
        // input first, then signal block destruction with this final lease.
        NativeBlockLease::retain(native_lifetime)?,
    );
    let admission = request.allowed.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let completion = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let _native_inputs = &keepalive;
        if completed.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            autoreleasepool(|_| {
                (request.allowed)()?;
                if let Some(error) = unsafe { error.as_ref() } {
                    return Err(native_error("无法获取屏幕图像", error));
                }
                let image =
                    unsafe { image.as_ref() }.ok_or_else(|| "系统未返回屏幕图像".to_owned())?;
                // The borrowed native image is fully copied before this callback
                // returns. The parent receives only owned and validated pixels.
                let pixels = copy_image(image, request.width, request.height)?;
                (request.allowed)()?;
                Ok(pixels)
            })
        }))
        .unwrap_or_else(|_| Err("读取原生截图像素失败".to_owned()));
        let _ = sender.send(result);
    });
    // The documented API delivers an asynchronous completion block. RcBlock is
    // heap-owned; the native API copies it, retaining its input keepalive. Dropping
    // our local RcBlock after this call leaves the native copy responsible for
    // completion and release. It is never a stack block or a detached Rust job.
    // https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager/captureimage(contentfilter:configuration:completionhandler:)
    admission()?;
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &configuration,
            Some(&completion),
        );
    }
    drop(completion);
    Ok(())
}

fn native_error(context: &str, error: &NSError) -> String {
    let detail: String = error
        .localizedDescription()
        .to_string()
        .chars()
        .take(256)
        .collect();
    format!("{context}：{detail}（{}）", error.code())
}

fn validate_dimensions(width: u32, height: u32) -> Result<usize, String> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("显示器分辨率超出当前截图限制".to_owned());
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "原生截图像素数量无效".to_owned())
}

pub(crate) fn validate_exclusions(exclusions: &[ExcludedWindow]) -> Result<(), String> {
    if exclusions.len() > MAX_EXCLUDED_WINDOWS {
        return Err("截图窗口排除列表无效".to_owned());
    }
    for (index, window) in exclusions.iter().enumerate() {
        if window.id == 0
            || window.title.is_empty()
            || window.title.len() > MAX_TITLE_BYTES
            || window.title.chars().any(char::is_control)
        {
            return Err("截图窗口排除列表包含无效标识或标题".to_owned());
        }
        if exclusions[..index]
            .iter()
            .any(|previous| previous.id == window.id)
        {
            return Err("截图窗口排除列表包含重复标识".to_owned());
        }
    }
    Ok(())
}

fn validate_matching_window(
    expected: &ExcludedWindow,
    actual_pid: i32,
    actual_title: &str,
    own_pid: i32,
) -> Result<(), String> {
    if actual_pid != own_pid {
        return Err("需排除的窗口已改变归属，请重试截图".to_owned());
    }
    if actual_title != expected.title {
        return Err("需排除的窗口标题已改变，请重试截图".to_owned());
    }
    Ok(())
}

fn validate_display_size(
    width_points: isize,
    height_points: isize,
    width_pixels: u32,
    height_pixels: u32,
) -> Result<(), String> {
    if width_points <= 0 || height_points <= 0 {
        return Err("所选显示器的原生尺寸无效".to_owned());
    }
    // SCDisplay geometry is points; SCStreamConfiguration dimensions are pixels.
    // Validate only this display's aspect ratio, allowing rounding by one pixel.
    // No origin or mixed-display desktop scale enters the calculation.
    let expected_height = f64::from(width_pixels) * height_points as f64 / width_points as f64;
    if !expected_height.is_finite() || (f64::from(height_pixels) - expected_height).abs() > 1.0 {
        return Err("显示器尺寸已改变，请重新选择并截图".to_owned());
    }
    Ok(())
}

fn copy_image(image: &CGImage, width: u32, height: u32) -> Result<RgbaImage, String> {
    let source = Some(image);
    if CGImage::width(source) != width as usize || CGImage::height(source) != height as usize {
        return Err(format!(
            "原生截图像素尺寸与请求不一致：{}×{}，请求{width}×{height}",
            CGImage::width(source),
            CGImage::height(source),
        ));
    }
    validate_dimensions(width, height)?;
    if CGImage::bits_per_component(source) != 8
        || CGImage::bits_per_pixel(source) != 32
        || CGImage::bitmap_info(source).contains(CGBitmapInfo::FloatComponents)
    {
        return Err(format!(
            "系统返回了不支持的原生截图像素格式：component={}，pixel={}，bitmap={:#x}",
            CGImage::bits_per_component(source),
            CGImage::bits_per_pixel(source),
            CGImage::bitmap_info(source).bits(),
        ));
    }
    let layout = PixelLayout::from_metadata(
        CGImage::alpha_info(source),
        CGImage::byte_order_info(source),
    )?;
    let stride = CGImage::bytes_per_row(source);
    let required = validate_stride(width, height, stride)?;
    let provider =
        CGImage::data_provider(source).ok_or_else(|| "原生截图没有像素数据源".to_owned())?;
    let data =
        CGDataProvider::data(Some(&provider)).ok_or_else(|| "无法复制原生截图像素".to_owned())?;
    let length = usize::try_from(data.length()).map_err(|_| "原生截图数据长度无效")?;
    if length < required || length > MAX_NATIVE_BYTES || data.byte_ptr().is_null() {
        return Err("原生截图数据长度超出限制".to_owned());
    }
    // CGDataProviderCopyData gives us a private retained, immutable CFData copy.
    // Its bytes are valid during conversion, and no provider buffer escapes.
    let bytes = unsafe { std::slice::from_raw_parts(data.byte_ptr(), length) };
    convert_pixels(bytes, width, height, stride, layout)
}

#[derive(Clone, Copy)]
struct PixelLayout {
    red: usize,
    green: usize,
    blue: usize,
    alpha: usize,
    premultiplied: bool,
    opaque: bool,
}

impl PixelLayout {
    fn from_metadata(alpha: CGImageAlphaInfo, order: CGImageByteOrderInfo) -> Result<Self, String> {
        let (first, premultiplied, opaque) = match alpha {
            CGImageAlphaInfo::PremultipliedFirst => (true, true, false),
            CGImageAlphaInfo::First => (true, false, false),
            CGImageAlphaInfo::NoneSkipFirst => (true, false, true),
            CGImageAlphaInfo::PremultipliedLast => (false, true, false),
            CGImageAlphaInfo::Last => (false, false, false),
            CGImageAlphaInfo::NoneSkipLast => (false, false, true),
            _ => return Err(format!("原生截图的 alpha 格式不受支持：{}", alpha.0)),
        };
        let (red, green, blue, alpha) = match (order, first) {
            (CGImageByteOrderInfo::Order32Little, true) => (2, 1, 0, 3),
            (CGImageByteOrderInfo::Order32Little, false) => (3, 2, 1, 0),
            (CGImageByteOrderInfo::Order32Big, true) => (1, 2, 3, 0),
            (CGImageByteOrderInfo::Order32Big, false) => (0, 1, 2, 3),
            // Do not guess host byte order for an unspecified native format.
            _ => return Err(format!("原生截图的字节顺序不受支持：{}", order.0)),
        };
        Ok(Self {
            red,
            green,
            blue,
            alpha,
            premultiplied,
            opaque,
        })
    }
}

fn validate_stride(width: u32, height: u32, stride: usize) -> Result<usize, String> {
    validate_dimensions(width, height)?;
    let row = width as usize * 4;
    if stride < row || stride > row + MAX_ROW_PADDING {
        return Err("原生截图的行宽无效".to_owned());
    }
    stride
        .checked_mul(height as usize)
        .filter(|length| *length <= MAX_NATIVE_BYTES)
        .ok_or_else(|| "原生截图的像素数据超出限制".to_owned())
}

fn convert_pixels(
    source: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    layout: PixelLayout,
) -> Result<RgbaImage, String> {
    let required = validate_stride(width, height, stride)?;
    if source.len() < required || source.len() > MAX_NATIVE_BYTES {
        return Err("原生截图的像素数据不完整".to_owned());
    }
    let output_len = validate_dimensions(width, height)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .map_err(|_| "截图像素内存不足")?;
    for row in source[..required].chunks_exact(stride) {
        for pixel in row[..width as usize * 4].chunks_exact(4) {
            let alpha = if layout.opaque {
                255
            } else {
                pixel[layout.alpha]
            };
            let component = |index| {
                let value = pixel[index];
                if layout.premultiplied {
                    if alpha == 0 {
                        0
                    } else {
                        ((u32::from(value) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                            .min(255) as u8
                    }
                } else {
                    value
                }
            };
            output.extend_from_slice(&[
                component(layout.red),
                component(layout.green),
                component(layout.blue),
                alpha,
            ]);
        }
    }
    RgbaImage::from_raw(width, height, output).ok_or_else(|| "原生截图像素尺寸无效".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_padding_and_premultiplied_alpha_are_converted_without_crossing_rows() {
        let layout = PixelLayout::from_metadata(
            CGImageAlphaInfo::PremultipliedFirst,
            CGImageByteOrderInfo::Order32Little,
        )
        .unwrap();
        let image = convert_pixels(
            &[32, 64, 128, 128, 9, 9, 9, 9, 30, 20, 10, 255, 8, 8, 8, 8],
            1,
            2,
            8,
            layout,
        )
        .unwrap();
        assert_eq!(image.as_raw(), &[255, 128, 64, 128, 10, 20, 30, 255]);
    }

    #[test]
    fn explicit_endian_layouts_and_reserved_alpha_are_observed() {
        for (alpha, order, bytes) in [
            (
                CGImageAlphaInfo::First,
                CGImageByteOrderInfo::Order32Big,
                [255, 10, 20, 30],
            ),
            (
                CGImageAlphaInfo::Last,
                CGImageByteOrderInfo::Order32Big,
                [10, 20, 30, 255],
            ),
            (
                CGImageAlphaInfo::Last,
                CGImageByteOrderInfo::Order32Little,
                [255, 30, 20, 10],
            ),
            (
                CGImageAlphaInfo::NoneSkipFirst,
                CGImageByteOrderInfo::Order32Little,
                [30, 20, 10, 0],
            ),
        ] {
            let layout = PixelLayout::from_metadata(alpha, order).unwrap();
            assert_eq!(
                convert_pixels(&bytes, 1, 1, 4, layout).unwrap().as_raw(),
                &[10, 20, 30, 255]
            );
        }
        assert!(PixelLayout::from_metadata(
            CGImageAlphaInfo::First,
            CGImageByteOrderInfo::OrderDefault
        )
        .is_err());
    }

    #[test]
    fn zero_alpha_cannot_reveal_uninitialized_premultiplied_rgb() {
        let layout = PixelLayout::from_metadata(
            CGImageAlphaInfo::PremultipliedFirst,
            CGImageByteOrderInfo::Order32Little,
        )
        .unwrap();
        assert_eq!(
            convert_pixels(&[90, 80, 70, 0], 1, 1, 4, layout)
                .unwrap()
                .as_raw(),
            &[0, 0, 0, 0]
        );
    }

    #[test]
    fn invalid_geometry_and_short_or_excessively_padded_rows_fail_before_conversion() {
        assert!(validate_dimensions(0, 1).is_err());
        assert!(validate_dimensions(MAX_SIDE, MAX_SIDE).is_err());
        assert!(validate_display_size(1280, 720, 2560, 1440).is_ok());
        assert!(validate_display_size(1280, 720, 2560, 1000).is_err());
        assert!(validate_stride(1, 1, 3).is_err());
        assert!(validate_stride(1, 1, MAX_ROW_PADDING + 5).is_err());
        let layout =
            PixelLayout::from_metadata(CGImageAlphaInfo::First, CGImageByteOrderInfo::Order32Big)
                .unwrap();
        assert!(convert_pixels(&[255, 1, 2, 3], 1, 2, 4, layout).is_err());
    }

    #[test]
    fn exclusions_accept_only_bounded_distinct_native_ids_and_titles() {
        let first = ExcludedWindow {
            id: 17,
            title: "Mewu Space".into(),
        };
        let second = ExcludedWindow {
            id: 29,
            title: "Mewu Conversation".into(),
        };
        assert!(validate_exclusions(&[]).is_ok());
        assert!(validate_exclusions(&[first.clone(), second.clone()]).is_ok());
        assert!(validate_exclusions(&[ExcludedWindow {
            id: 0,
            ..first.clone()
        }])
        .is_err());
        assert!(validate_exclusions(&[ExcludedWindow {
            title: "".into(),
            ..first.clone()
        }])
        .is_err());
        assert!(validate_exclusions(&[first.clone(), first.clone()]).is_err());
        assert!(validate_exclusions(&[first.clone(), second, first]).is_err());
    }

    #[test]
    fn reused_native_id_cannot_exclude_settings_or_another_process() {
        let expected = ExcludedWindow {
            id: 17,
            title: "Mewu Space".into(),
        };
        assert!(validate_matching_window(&expected, 42, "Mewu Space", 42).is_ok());
        assert!(validate_matching_window(&expected, 42, "Mewu Settings", 42).is_err());
        assert!(validate_matching_window(&expected, 43, "Mewu Space", 42).is_err());
    }

    #[test]
    fn callback_result_does_not_release_native_lease_until_last_block_copy_drops() {
        let lifetime = Arc::new(NativeLifetime::default());
        let lease = NativeBlockLease::retain(&lifetime).unwrap();
        let (sender, receiver) = mpsc::channel();
        let block = RcBlock::new(move || {
            let _lease = &lease;
            let _ = sender.send(());
        });
        let native_copy = block.clone();
        drop(block);
        native_copy.call(());
        receiver.recv().unwrap();
        assert_eq!(*lifetime.blocks.lock().unwrap(), 1);
        drop(native_copy);
        lifetime.wait_released();
        assert_eq!(*lifetime.blocks.lock().unwrap(), 0);
    }
}
