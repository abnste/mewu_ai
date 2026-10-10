// SPDX-License-Identifier: MPL-2.0
//! Local Windows OCR worker. The host owns concurrency and scene/plugin fences.
//! Keep the worker permit until this blocking function actually returns, even
//! after the async caller loses interest. WinRT cancellation is cooperative.
//! https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine
//! https://learn.microsoft.com/en-us/windows/win32/api/roapi/nf-roapi-roinitialize
use image::RgbaImage;
use mewu_core::OcrDocument;
use resvg::tiny_skia;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub type PixelBuffer = RgbaImage;
const MAX_SIDE: u32 = 16_384;
const MAX_SOURCE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_RECOGNITION_PIXELS: u64 = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);
const CANCEL_ERROR: &str = "文字识别已取消";
const TIMEOUT_ERROR: &str = "文字识别超时，请缩小选区后重试";

/// Consumes an owned, already-cropped/composited RGBA image. No file, display,
/// network, or model access occurs here. Returns coordinates in original crop
/// pixels in Windows' OCR coordinate frame; apply text_angle once about center.
pub fn recognize(bitmap: PixelBuffer, cancel: Arc<AtomicBool>) -> Result<OcrDocument, String> {
    #[cfg(windows)]
    {
        native::recognize_impl(bitmap, cancel, || {})
    }
    #[cfg(not(windows))]
    {
        let _ = (bitmap, cancel);
        Err("当前系统尚未支持本机文字识别；此插件需要 Windows 10 或更新版本".into())
    }
}

#[derive(Debug, Clone, Copy)]
struct Geometry {
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
}
impl Geometry {
    fn new(width: u32, height: u32, maximum: u32) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
        {
            return Err("文字识别图片为空或超过 3200 万像素".into());
        }
        if maximum == 0 {
            return Err("本机文字识别返回了无效尺寸限制".into());
        }
        let scale = 1.0_f64
            .min(f64::from(maximum) / f64::from(width.max(height)))
            .min((MAX_RECOGNITION_PIXELS as f64 / (f64::from(width) * f64::from(height))).sqrt());
        let resized_width = (f64::from(width) * scale).floor().max(1.0) as u32;
        let resized_height = (f64::from(height) * scale).floor().max(1.0) as u32;
        Ok(Self {
            source_width: width,
            source_height: height,
            width: resized_width,
            height: resized_height,
        })
    }
    fn scale(self) -> f64 {
        (f64::from(self.width) / f64::from(self.source_width))
            .min(f64::from(self.height) / f64::from(self.source_height))
    }
    fn rect(self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64, f64, f64) {
        let scale = self.scale();
        // One uniform scale about image centers commutes with TextAngle's
        // center rotation. Independent integer width/height scaling does not.
        // No clamping: rotated OCR coordinate frames can extend past the image.
        (
            (x - f64::from(self.width) / 2.0) / scale + f64::from(self.source_width) / 2.0,
            (y - f64::from(self.height) / 2.0) / scale + f64::from(self.source_height) / 2.0,
            width / scale,
            height / scale,
        )
    }
}
fn check_interest(cancel: &AtomicBool, started: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCEL_ERROR.into())
    } else if started.elapsed() >= TIMEOUT {
        Err(TIMEOUT_ERROR.into())
    } else {
        Ok(())
    }
}
fn validate_pixels(bitmap: &PixelBuffer) -> Result<(), String> {
    Geometry::new(bitmap.width(), bitmap.height(), MAX_SIDE)?;
    if bitmap.as_raw().len() as u64 != u64::from(bitmap.width()) * u64::from(bitmap.height()) * 4 {
        return Err("文字识别图片缓冲区大小无效".into());
    }
    Ok(())
}
fn prepare(
    mut bitmap: PixelBuffer,
    geometry: Geometry,
    cancel: &AtomicBool,
    started: Instant,
) -> Result<Vec<u8>, String> {
    check_interest(cancel, started)?;
    validate_pixels(&bitmap)?;
    // Composite transparent source pixels on white before resampling. This
    // avoids interpreting transparent RGB as text or producing alpha halos.
    for (index, pixel) in bitmap.pixels_mut().enumerate() {
        if index % 16_384 == 0 {
            check_interest(cancel, started)?;
        }
        let a = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
    let mut bytes = if bitmap.dimensions() == (geometry.width, geometry.height) {
        bitmap.into_raw()
    } else {
        // Reuse the owned, now-opaque RGBA allocation. tiny-skia requires
        // premultiplied RGBA, which is identical for these opaque pixels.
        let size = tiny_skia::IntSize::from_wh(geometry.source_width, geometry.source_height)
            .ok_or_else(|| "文字识别图片尺寸无效".to_string())?;
        let source = tiny_skia::Pixmap::from_vec(bitmap.into_raw(), size)
            .ok_or_else(|| "文字识别图片缓冲区大小无效".to_string())?;
        let mut target = tiny_skia::Pixmap::new(geometry.width, geometry.height)
            .ok_or_else(|| "无法准备文字识别图片".to_string())?;
        target.fill(tiny_skia::Color::WHITE);
        let scale = geometry.scale();
        let tx = (f64::from(geometry.width) - f64::from(geometry.source_width) * scale) / 2.0;
        let ty = (f64::from(geometry.height) - f64::from(geometry.source_height) * scale) / 2.0;
        target.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Bilinear,
                ..Default::default()
            },
            tiny_skia::Transform::from_row(
                scale as f32,
                0.0,
                0.0,
                scale as f32,
                tx as f32,
                ty as f32,
            ),
            None,
        );
        target.take()
    };
    check_interest(cancel, started)?;
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(bytes)
}

#[cfg(windows)]
mod native {
    use super::*;
    use mewu_core::{validate_ocr_document, OcrLine, OcrWord};
    use std::{cell::Cell, marker::PhantomData, rc::Rc, sync::atomic::AtomicUsize, thread};
    use windows::{
        core::HSTRING,
        Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::{OcrEngine, OcrResult},
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    use zeroize::Zeroizing;
    const CANCEL_GRACE: Duration = Duration::from_secs(2);
    const MAX_TEXT: usize = 128 * 1024;
    static QUARANTINED: AtomicBool = AtomicBool::new(false);
    static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
    const QUARANTINE_ERROR: &str = "本机文字识别未能停止，请重启应用后重试";

    // Independently bound exceptional resource retention, even if a future
    // caller forgets the host's worker limit. Quarantine precedes permit release.
    struct WorkerPermit;
    impl WorkerPermit {
        fn acquire() -> Result<Self, String> {
            if QUARANTINED.load(Ordering::Acquire) {
                return Err(QUARANTINE_ERROR.into());
            }
            ACTIVE_WORKERS
                .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < 2).then_some(count + 1)
                })
                .map_err(|_| "已有文字识别正在进行，请稍后重试".to_string())?;
            let permit = Self;
            if QUARANTINED.load(Ordering::Acquire) {
                return Err(QUARANTINE_ERROR.into());
            }
            Ok(permit)
        }
    }
    impl Drop for WorkerPermit {
        fn drop(&mut self) {
            ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }

    struct Apartment(PhantomData<Rc<()>>);
    impl Apartment {
        fn new() -> Result<Self, String> {
            // S_FALSE is also success and still requires same-thread balancing.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|e| native_error("初始化本机文字识别", e))?;
            Ok(Self(PhantomData))
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    struct Cleanup<F: FnMut()>(F);
    impl<F: FnMut()> Drop for Cleanup<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    struct NativeBitmap(SoftwareBitmap, Cell<bool>);
    impl Drop for NativeBitmap {
        fn drop(&mut self) {
            if self.1.get() {
                let _ = self.0.Close();
            }
        }
    }

    fn terminal(status: Option<i32>) -> bool {
        matches!(status, Some(1..=3))
    }
    fn cleanup_operation(
        mut status: impl FnMut() -> Option<i32>,
        cancel: impl FnOnce(),
        close: impl FnOnce(),
        retain: impl FnOnce(),
    ) {
        if !terminal(status()) {
            cancel();
            if !terminal(status()) {
                retain();
                return;
            }
        }
        close();
    }

    fn native_error(action: &str, error: windows::core::Error) -> String {
        // Never expose arbitrary COM diagnostics or paths to the UI.
        format!(
            "{action}失败（0x{:08X}），请确认此 Windows 环境支持本机文字识别",
            error.code().0 as u32
        )
    }
    fn supported_os() -> Result<(), String> {
        #[repr(C)]
        struct OsVersion {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            service_pack: [u16; 128],
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetVersion(version: *mut OsVersion) -> i32;
        }
        let mut version = OsVersion {
            size: std::mem::size_of::<OsVersion>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            service_pack: [0; 128],
        };
        if unsafe { RtlGetVersion(&mut version) } < 0 {
            return Err("无法确认系统的本机文字识别能力".into());
        }
        if version.major < 10 || (version.major == 10 && version.build < 10_240) {
            return Err("本机文字识别需要 Windows 10 或更新版本".into());
        }
        Ok(())
    }
    fn engine() -> Result<OcrEngine, String> {
        let languages = OcrEngine::AvailableRecognizerLanguages()
            .map_err(|e| native_error("读取文字识别语言", e))?;
        let count = languages
            .Size()
            .map_err(|e| native_error("读取文字识别语言", e))?;
        if count == 0 {
            return Err(
                "Windows 未安装可用的文字识别语言，请在系统语言设置中添加 OCR 语言功能".into(),
            );
        }
        if count > 128 {
            return Err("本机文字识别语言列表超过限制".into());
        }
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(engine);
        }
        // A user's preferred language may have no OCR FOD. Only fall back to
        // explicitly enumerated installed languages, never download a model.
        for i in 0..count {
            let language = languages
                .GetAt(i)
                .map_err(|e| native_error("读取文字识别语言", e))?;
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                return Ok(engine);
            }
        }
        Err("本机文字识别无法创建可用语言引擎，请检查 Windows 语言功能".into())
    }
    fn text(value: HSTRING, maximum_chars: usize, budget: &mut usize) -> Result<String, String> {
        if value.len() > maximum_chars.saturating_mul(2) {
            return Err("识别文字超过长度限制，请缩小选区".into());
        }
        let text = value.to_string();
        if text.chars().count() > maximum_chars || text.len() > *budget {
            return Err("识别文字超过长度限制，请缩小选区".into());
        }
        *budget -= text.len();
        Ok(text)
    }
    fn nullable_angle(
        value: windows::core::Result<windows::Foundation::IReference<f64>>,
    ) -> Result<Option<f64>, String> {
        match value {
            Ok(reference) => reference
                .Value()
                .map(Some)
                .map_err(|e| native_error("读取文字角度", e)),
            // windows-core 0.62.2 Type::from_abi maps a null WinRT reference to
            // Error::empty() (code 0). A failing HRESULT must not be swallowed.
            Err(error) if error.code().0 == 0 => Ok(None),
            Err(error) => Err(native_error("读取文字角度", error)),
        }
    }
    fn document(
        result: OcrResult,
        geometry: Geometry,
        language: String,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<OcrDocument, String> {
        check_interest(cancel, started)?;
        let text_angle = nullable_angle(result.TextAngle())?;
        let lines = result.Lines().map_err(|e| native_error("读取识别行", e))?;
        let line_count = lines.Size().map_err(|e| native_error("读取识别行", e))?;
        if line_count > 512 {
            return Err("识别行数超过限制，请缩小选区".into());
        }
        let mut output = Vec::with_capacity(line_count as usize);
        let mut word_count = 0;
        let mut budget = MAX_TEXT;
        for i in 0..line_count {
            check_interest(cancel, started)?;
            let line = lines.GetAt(i).map_err(|e| native_error("读取识别行", e))?;
            let line_text = text(
                line.Text().map_err(|e| native_error("读取识别文字", e))?,
                8192,
                &mut budget,
            )?;
            let words = line.Words().map_err(|e| native_error("读取文字位置", e))?;
            let count = words.Size().map_err(|e| native_error("读取文字位置", e))?;
            word_count += u64::from(count);
            if word_count > 4096 {
                return Err("识别词数超过限制，请缩小选区".into());
            }
            if count == 0 && line_text.trim().is_empty() {
                continue;
            }
            let mut output_words = Vec::with_capacity(count as usize);
            for j in 0..count {
                check_interest(cancel, started)?;
                let word = words
                    .GetAt(j)
                    .map_err(|e| native_error("读取文字位置", e))?;
                let word_text = text(
                    word.Text().map_err(|e| native_error("读取识别文字", e))?,
                    512,
                    &mut budget,
                )?;
                let rect = word
                    .BoundingRect()
                    .map_err(|e| native_error("读取文字位置", e))?;
                let (x, y, width, height) = geometry.rect(
                    f64::from(rect.X),
                    f64::from(rect.Y),
                    f64::from(rect.Width),
                    f64::from(rect.Height),
                );
                output_words.push(OcrWord {
                    text: word_text,
                    x,
                    y,
                    width,
                    height,
                });
            }
            output.push(OcrLine {
                text: line_text,
                words: output_words,
            });
        }
        let document = OcrDocument {
            engine: "windows.media.ocr".into(),
            language,
            width: geometry.source_width,
            height: geometry.source_height,
            text_angle,
            lines: output,
        };
        validate_ocr_document(&document)
            .map_err(|_| "本机识别结果格式或大小无效，请缩小选区后重试".to_string())?;
        check_interest(cancel, started)?;
        Ok(document)
    }

    pub(super) fn recognize_impl(
        bitmap: PixelBuffer,
        cancel: Arc<AtomicBool>,
        on_started: impl FnOnce(),
    ) -> Result<OcrDocument, String> {
        let started = Instant::now();
        check_interest(&cancel, started)?;
        let _permit = WorkerPermit::acquire()?;
        supported_os()?;
        // Check caller-owned allocation before any WinRT work or extra buffer.
        validate_pixels(&bitmap)?;
        let _apartment = Apartment::new()?;
        let engine = engine()?;
        let maximum =
            OcrEngine::MaxImageDimension().map_err(|e| native_error("读取识别尺寸限制", e))?;
        let geometry = Geometry::new(bitmap.width(), bitmap.height(), maximum)?;
        let language = engine
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map_err(|e| native_error("读取识别语言", e))?
            .to_string();
        let bytes = Zeroizing::new(prepare(bitmap, geometry, &cancel, started)?);
        let writer = DataWriter::new().map_err(|e| native_error("准备识别图片", e))?;
        let _writer_cleanup = Cleanup(|| {
            let _ = writer.Close();
        });
        writer
            .WriteBytes(&bytes)
            .map_err(|e| native_error("准备识别图片", e))?;
        let buffer = writer
            .DetachBuffer()
            .map_err(|e| native_error("准备识别图片", e))?;
        let bitmap = NativeBitmap(
            SoftwareBitmap::CreateWithAlpha(
                BitmapPixelFormat::Bgra8,
                geometry.width as i32,
                geometry.height as i32,
                BitmapAlphaMode::Premultiplied,
            )
            .map_err(|e| native_error("准备识别图片", e))?,
            Cell::new(true),
        );
        bitmap
            .0
            .CopyFromBuffer(&buffer)
            .map_err(|e| native_error("准备识别图片", e))?;
        drop(bytes);
        drop(buffer);
        check_interest(&cancel, started)?;
        let operation = engine
            .RecognizeAsync(&bitmap.0)
            .map_err(|e| native_error("启动文字识别", e))?;
        let cancel_sent = Cell::new(false);
        let _operation_cleanup = Cleanup(|| {
            cleanup_operation(
                || operation.Status().ok().map(|status| status.0),
                || {
                    if !cancel_sent.replace(true) {
                        let _ = operation.Cancel();
                    }
                },
                || {
                    let _ = operation.Close();
                },
                || {
                    // A live/unknown operation may still read its bitmap. Do
                    // not Close that input or release the last strong refs.
                    // These agile COM objects are retained until process exit.
                    // At most two sets can survive: admission is capped at two
                    // and quarantine permanently rejects replacement workers.
                    QUARANTINED.store(true, Ordering::Release);
                    bitmap.1.set(false);
                    std::mem::forget((operation.clone(), bitmap.0.clone(), engine.clone()));
                },
            );
        });
        on_started();
        let mut stopping: Option<(String, Instant)> = None;
        loop {
            if stopping.is_none() {
                if let Err(reason) = check_interest(&cancel, started) {
                    cancel_sent.set(true);
                    // Fence completion even when Cancel loses a race to Completed.
                    let _ = operation.Cancel();
                    stopping = Some((reason, Instant::now()));
                }
            }
            let status = operation
                .Status()
                .map_err(|e| native_error("检查文字识别状态", e))?;
            if let Some((reason, requested)) = &stopping {
                if terminal(Some(status.0)) {
                    return Err(reason.clone());
                }
                if status.0 != 0 {
                    return Err("本机文字识别返回了未知状态".into());
                }
                if requested.elapsed() >= CANCEL_GRACE {
                    // Do not spawn unbounded replacement operations after a
                    // native cancellation failure. Host permits remain needed.
                    QUARANTINED.store(true, Ordering::Release);
                    return Err(QUARANTINE_ERROR.into());
                }
            } else {
                match status.0 {
                    0 => {}
                    1 => {
                        check_interest(&cancel, started)?;
                        let result = operation
                            .GetResults()
                            .map_err(|e| native_error("读取文字识别结果", e))?;
                        return document(result, geometry, language, &cancel, started);
                    }
                    2 => return Err(CANCEL_ERROR.into()),
                    3 => {
                        let hresult = operation
                            .ErrorCode()
                            .map_err(|e| native_error("读取文字识别状态", e))?;
                        return Err(format!("本机文字识别失败（0x{:08X}）", hresult.0 as u32));
                    }
                    _ => return Err("本机文字识别返回了未知状态".into()),
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cleanup_retains_live_unknown_or_unreadable_operations_and_their_inputs() {
            for value in [Some(0), Some(4), None] {
                let canceled = Cell::new(false);
                let closed = Cell::new(false);
                let retained = Cell::new(false);
                cleanup_operation(
                    || value,
                    || canceled.set(true),
                    || closed.set(true),
                    || retained.set(true),
                );
                assert!(canceled.get());
                assert!(retained.get());
                assert!(
                    !closed.get(),
                    "an unconfirmed live operation must not be closed"
                );
            }
            // Cancellation may synchronously reach a terminal state. In that
            // case ordinary RAII can safely release the input and operation.
            let mut states = [Some(0), Some(2)].into_iter();
            let closed = Cell::new(false);
            cleanup_operation(
                || states.next().flatten(),
                || {},
                || closed.set(true),
                || panic!("confirmed termination must not retain resources"),
            );
            assert!(closed.get());
            for state in 1..=3 {
                cleanup_operation(
                    || Some(state),
                    || panic!("terminal operation need not be canceled"),
                    || {},
                    || panic!("terminal operation must release resources"),
                );
            }
        }
        #[test]
        fn null_angle_is_absent_but_a_real_hresult_is_not_swallowed() {
            assert_eq!(
                nullable_angle(Err(windows::core::Error::empty())).unwrap(),
                None
            );
            assert!(nullable_angle(Err(windows::core::Error::from_hresult(
                windows::core::HRESULT(0x80004003_u32 as i32)
            )))
            .is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba};
    #[test]
    fn input_and_resampling_bounds_are_finite_and_never_enlarge() {
        assert!(Geometry::new(0, 1, 10000).is_err());
        assert!(Geometry::new(16385, 1, 10000).is_err());
        assert!(Geometry::new(8192, 8192, 10000).is_err());
        assert!(Geometry::new(1, 1, 0).is_err());
        for (w, h, limit) in [
            (7680, 4320, 10000),
            (16000, 1000, 2000),
            (3, 9000, 100),
            (20, 10, 10000),
        ] {
            let g = Geometry::new(w, h, limit).unwrap();
            assert!(g.width <= w && g.height <= h && g.width <= limit && g.height <= limit);
            assert!(u64::from(g.width) * u64::from(g.height) <= MAX_RECOGNITION_PIXELS);
        }
    }
    #[test]
    fn transparent_pixels_are_white_and_opaque_pixels_become_bgra() {
        // ImageBuffer accepts a backing Vec larger than its pixel dimensions.
        let excess = RgbaImage::from_raw(1, 1, vec![0; 8]).unwrap();
        assert!(validate_pixels(&excess).is_err());
        let image =
            RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let bytes = prepare(
            image,
            Geometry::new(3, 1, 100).unwrap(),
            &AtomicBool::new(false),
            Instant::now(),
        )
        .unwrap();
        assert_eq!(
            bytes,
            vec![255, 255, 255, 255, 0, 0, 255, 255, 255, 127, 127, 255]
        );
    }
    #[test]
    fn cancel_and_deadline_win_before_pixel_processing() {
        assert_eq!(
            check_interest(&AtomicBool::new(true), Instant::now()).unwrap_err(),
            CANCEL_ERROR
        );
        assert_eq!(
            check_interest(&AtomicBool::new(false), Instant::now() - TIMEOUT).unwrap_err(),
            TIMEOUT_ERROR
        );
        let image = RgbaImage::new(10, 10);
        assert_eq!(
            prepare(
                image,
                Geometry::new(10, 10, 100).unwrap(),
                &AtomicBool::new(true),
                Instant::now()
            )
            .unwrap_err(),
            CANCEL_ERROR
        );
    }
    #[test]
    fn geometry_maps_back_to_source_without_clamping_rotated_frame() {
        let geometry = Geometry {
            source_width: 200,
            source_height: 100,
            width: 100,
            height: 50,
        };
        assert_eq!(geometry.rect(-2., 4., 10., 8.), (-4., 8., 20., 16.));
    }
    #[test]
    fn uniform_center_mapping_commutes_with_rotation_even_for_extreme_aspect_ratios() {
        fn rotate(point: (f64, f64), center: (f64, f64), degrees: f64) -> (f64, f64) {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let x = point.0 - center.0;
            let y = point.1 - center.1;
            (center.0 + cos * x - sin * y, center.1 + sin * x + cos * y)
        }
        for (w, h, limit) in [(16000, 999, 10000), (3, 9000, 100), (9000, 3, 100)] {
            let g = Geometry::new(w, h, limit).unwrap();
            let inverse = |p: (f64, f64)| {
                let (x, y, _, _) = g.rect(p.0, p.1, 1.0, 1.0);
                (x, y)
            };
            let source_center = (f64::from(w) / 2.0, f64::from(h) / 2.0);
            let target_center = (f64::from(g.width) / 2.0, f64::from(g.height) / 2.0);
            for angle in [-20.0, 0.5, 20.0, 45.0, 90.0, 179.0] {
                for point in [
                    (0.0, 0.0),
                    target_center,
                    (f64::from(g.width), f64::from(g.height)),
                ] {
                    let a = rotate(inverse(point), source_center, angle);
                    let b = inverse(rotate(point, target_center, angle));
                    assert!((a.0 - b.0).abs() < 1e-8 && (a.1 - b.1).abs() < 1e-8);
                }
            }
            let (_, _, mapped_w, mapped_h) = g.rect(0.0, 0.0, 1.0, 1.0);
            assert_eq!(mapped_w, mapped_h);
            assert_eq!(inverse(target_center), source_center);
        }
    }
    #[test]
    fn resampling_centers_content_on_white_without_stretching() {
        // A deliberately roomy canvas exposes whole padding pixels. Real
        // Geometry::new usually needs only fractional padding after rounding.
        let g = Geometry {
            source_width: 2,
            source_height: 4,
            width: 4,
            height: 4,
        };
        let bytes = prepare(
            RgbaImage::from_pixel(2, 4, Rgba([255, 0, 0, 255])),
            g,
            &AtomicBool::new(false),
            Instant::now(),
        )
        .unwrap();
        for row in bytes.chunks_exact(16) {
            assert_eq!(&row[..4], &[255, 255, 255, 255]);
            assert_eq!(&row[4..12], &[0, 0, 255, 255, 0, 0, 255, 255]);
            assert_eq!(&row[12..], &[255, 255, 255, 255]);
        }
        let extreme = Geometry::new(3, 9000, 100).unwrap();
        let bytes = prepare(
            RgbaImage::from_pixel(3, 9000, Rgba([255, 255, 255, 255])),
            extreme,
            &AtomicBool::new(false),
            Instant::now(),
        )
        .unwrap();
        assert_eq!((extreme.width, extreme.height), (1, 100));
        assert_eq!(bytes.len(), 400);
        assert!(bytes.iter().all(|&byte| byte == 255));
    }
    #[cfg(windows)]
    fn synthetic_text() -> PixelBuffer {
        use mewu_core::{Drawing, DrawingKind, DrawingPoint, Region};
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(1400, 300, Rgba([255, 255, 255, 255])));
        let drawings = [
            ("Mewu local OCR test 12345", 40.),
            ("这是本机文字识别测试", 160.),
        ]
        .into_iter()
        .map(|(text, y)| Drawing {
            id: uuid::Uuid::new_v4().to_string(),
            kind: DrawingKind::Text,
            color: "#000000".into(),
            stroke_width: 1.,
            points: vec![DrawingPoint { x: 48., y }],
            text: Some(text.into()),
            font_size: Some(64.),
            origin: None,
            rich: None,
        })
        .collect();
        crate::drawing_render::crop_region(
            &image,
            &Region {
                id: "ocr-synthetic".into(),
                x: 0.,
                y: 0.,
                width: 1400.,
                height: 300.,
                drawings,
                ..Default::default()
            },
        )
        .unwrap()
        .into_rgba8()
    }
    #[cfg(windows)]
    #[test]
    fn synthetic_fixture_uses_existing_local_renderer_without_screenshots() {
        let image = synthetic_text();
        assert_eq!(image.dimensions(), (1400, 300));
        assert!(image.pixels().filter(|p| p[0] < 128).count() > 1000);
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "explicit Windows OCR/language-pack verification with synthetic text only; captures no desktop"]
    fn real_windows_ocr_recognizes_synthetic_text_and_handles_blank_and_active_cancel() {
        let result = recognize(synthetic_text(), Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!((result.width, result.height), (1400, 300));
        assert_eq!(result.engine, "windows.media.ocr");
        let text: String = result
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(text.contains("MewulocalOCRtest12345"));
        // Known zh-Hans-CN engine splits 别 into 另 刂 on this fixture; do not
        // assert PP-OCR accuracy equivalence or silently correct OCR output.
        let blank = recognize(
            RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255])),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(blank.lines.is_empty());
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let result = native::recognize_impl(synthetic_text(), cancel, move || {
            stop.store(true, Ordering::Release)
        });
        assert_eq!(result.unwrap_err(), CANCEL_ERROR);
    }
}
