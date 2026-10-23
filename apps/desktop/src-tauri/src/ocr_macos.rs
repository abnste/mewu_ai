// SPDX-License-Identifier: MPL-2.0
//! Local Apple Vision OCR for an owned, composed crop. No screen/file/network
//! access occurs here. This is a child of `ocr`, sharing its pixel limits and
//! centered resize transform. Vision cancellation is cooperative: a deadline
//! never proves native completion, and all inputs and permits remain owned
//! until the synchronous handler actually returns.
//!
//! https://developer.apple.com/documentation/vision/vnrecognizetextrequest
//! https://developer.apple.com/documentation/vision/vndetectedobjectobservation/boundingbox
//! https://developer.apple.com/documentation/vision/vnrecognizedtext/boundingbox(for:)
use super::*;
use block2::RcBlock;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mewu_core::{validate_ocr_document, OcrLine, OcrWord};
use objc2::{
    rc::{autoreleasepool, Retained},
    AnyThread, ClassType,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSIndexSet, NSProcessInfo, NSRange, NSString,
};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestProgressProviding,
    VNRequestTextRecognitionLevel,
};
use std::{ptr::NonNull, sync::atomic::AtomicUsize};
use unicode_segmentation::UnicodeSegmentation;
use zeroize::Zeroizing;

const MAX_LINES: usize = 512;
const MAX_WORDS: usize = 4096;
const MAX_TEXT: usize = 128 * 1024;
const MAX_LINE_CHARS: usize = 8192;
const MAX_WORD_CHARS: usize = 512;
const MAX_PNG_BYTES: usize = MAX_RECOGNITION_PIXELS as usize * 4 + 1024 * 1024;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
static COMPAT_MODEL_READY: AtomicBool = AtomicBool::new(false);

fn native_timeout(compatibility: bool, model_ready: bool) -> Duration {
    if compatibility && !model_ready {
        // The macOS 27 revision 2 model can spend about 19 seconds loading
        // both scripts on its first request. This is a bounded cold-start
        // allowance, not a retry or a longer deadline for ordinary OCR.
        Duration::from_secs(30)
    } else {
        TIMEOUT
    }
}

// Only fixed stage/reason labels and process-local counters are recorded. Do
// not add NSError descriptions, image geometry, text, paths or caller IDs.
#[derive(Clone, Copy)]
struct OcrDiagnostics {
    #[cfg(debug_assertions)]
    id: Option<usize>,
    #[cfg(debug_assertions)]
    started: Instant,
}
impl OcrDiagnostics {
    fn new(started: Instant) -> Self {
        #[cfg(debug_assertions)]
        {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
            let enabled =
                std::env::var_os("MEWU_MAC_OCR_DIAGNOSTICS").is_some_and(|value| value == "1");
            Self {
                id: enabled.then(|| NEXT_ID.fetch_add(1, Ordering::Relaxed)),
                started,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = started;
            Self {}
        }
    }
    fn enabled(self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.id.is_some()
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }
    fn event(
        self,
        stage: &'static str,
        reason: &'static str,
        native_code: Option<isize>,
        count: Option<usize>,
    ) {
        #[cfg(debug_assertions)]
        if let Some(id) = self.id {
            eprintln!(
                "[mewu.mac.ocr] id={id} stage={stage} elapsed_ms={} thread={:?} reason={reason} native_code={native_code:?} count={count:?} active_workers={}",
                self.started.elapsed().as_millis(),
                std::thread::current().id(),
                ACTIVE_WORKERS.load(Ordering::Acquire),
            );
        }
        #[cfg(not(debug_assertions))]
        let _ = (self, stage, reason, native_code, count);
    }
}

fn check_interest_diagnosed(
    cancel: &AtomicBool,
    started: Instant,
    timeout: Duration,
    diagnostics: OcrDiagnostics,
    stage: &'static str,
) -> Result<(), String> {
    let interest = if cancel.load(Ordering::Acquire) {
        Err(CANCEL_ERROR.to_string())
    } else if started.elapsed() >= timeout {
        Err(TIMEOUT_ERROR.to_string())
    } else {
        Ok(())
    };
    interest.inspect_err(|error| {
        let reason = if error == CANCEL_ERROR {
            "cancel_flag"
        } else {
            "deadline"
        };
        diagnostics.event(stage, reason, None, None);
    })
}

// Independent protection also covers translation callers and future host paths.
// A stuck synchronous Vision call keeps its permit; no abandoned native object
// or artificial timeout permits a replacement worker to exceed this bound.
struct WorkerPermit(OcrDiagnostics);
impl WorkerPermit {
    fn acquire(diagnostics: OcrDiagnostics) -> Result<Self, String> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map(|_| Self(diagnostics))
            .map_err(|_| "已有文字识别正在进行，请稍后重试".to_string())
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        self.0.event("permit.released", "none", None, None);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PixelRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl PixelRect {
    fn matches(self, other: Self) -> bool {
        // Equal Vision word boxes can differ by floating point roundoff. Do
        // not merge merely overlapping/nearby words or infer character boxes.
        [
            self.x - other.x,
            self.y - other.y,
            self.width - other.width,
            self.height - other.height,
        ]
        .into_iter()
        .all(|difference| difference.abs() <= 1e-6)
    }
}

fn original_rect(
    geometry: Geometry,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<PixelRect, String> {
    if ![x, y, width, height].into_iter().all(f64::is_finite) || width <= 0. || height <= 0. {
        return Err("本机文字识别返回了无效文字位置".into());
    }
    // Vision normalized coordinates have a lower-left origin. The composed
    // crop is upright with a top-left origin. Reverse Y once, before the
    // existing centered uniform resize inverse; Retina and screen positions
    // are irrelevant to these owned image pixels.
    let (x, y, width, height) = geometry.rect(
        x * f64::from(geometry.width),
        (1. - y - height) * f64::from(geometry.height),
        width * f64::from(geometry.width),
        height * f64::from(geometry.height),
    );
    if ![x, y, width, height].into_iter().all(f64::is_finite)
        || x < -1.
        || y < -1.
        || x + width > f64::from(geometry.source_width) + 1.
        || y + height > f64::from(geometry.source_height) + 1.
    {
        return Err("本机文字识别返回了图像之外的文字位置".into());
    }
    Ok(PixelRect {
        x,
        y,
        width,
        height,
    })
}

#[derive(Debug)]
struct TextSpan {
    start: usize,
    end: usize,
    range: NSRange,
}
fn text_spans(text: &str) -> Vec<TextSpan> {
    let mut utf16_start = 0;
    let mut spans = Vec::new();
    for (start, part) in text.split_word_bound_indices() {
        let length = part.encode_utf16().count();
        if !part.trim().is_empty() {
            spans.push(TextSpan {
                start,
                end: start + part.len(),
                range: NSRange::new(utf16_start, length),
            });
        }
        // Count each string segment only once, including skipped whitespace.
        // Foundation NSRange uses UTF-16, never UTF-8 byte or scalar offsets.
        utf16_start += length;
    }
    spans
}

struct LocatedSpan {
    start: usize,
    end: usize,
    rect: PixelRect,
}
fn words_from_spans(text: &str, spans: Vec<LocatedSpan>) -> Result<Vec<OcrWord>, String> {
    let mut grouped: Vec<LocatedSpan> = Vec::new();
    for span in spans {
        if let Some(previous) = grouped.last_mut() {
            if previous.end == span.start && previous.rect.matches(span.rect) {
                previous.end = span.end;
                continue;
            }
        }
        grouped.push(span);
    }
    grouped
        .into_iter()
        .map(|span| {
            let value = text
                .get(span.start..span.end)
                .ok_or_else(|| "本机文字识别返回了无效文字范围".to_string())?;
            if value.chars().count() > MAX_WORD_CHARS {
                return Err("识别词长度超过限制，请缩小选区".into());
            }
            Ok(OcrWord {
                text: value.into(),
                x: span.rect.x,
                y: span.rect.y,
                width: span.rect.width,
                height: span.rect.height,
            })
        })
        .collect()
}

fn native_error(action: &str, error: &NSError) -> String {
    // Arbitrary localized diagnostics can include file paths or input text.
    format!("{action}失败（系统错误 {}）", error.code())
}
fn checked_text(value: &NSString, maximum_chars: usize) -> Result<String, String> {
    if value.len_utf16() > maximum_chars.saturating_mul(2)
        || value.len() > maximum_chars.saturating_mul(4)
    {
        return Err("识别文字超过长度限制，请缩小选区".into());
    }
    let text = value.to_string();
    if text.chars().count() > maximum_chars
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
    {
        return Err("识别文字内容或长度无效，请缩小选区".into());
    }
    Ok(text)
}

fn recognition_revision(major: isize, supported: impl Fn(usize) -> bool) -> Option<usize> {
    // macOS 27's revision 3 accurate recognizer can stall inside Vision on
    // mixed Chinese/Latin text, even with correction disabled or CPU-only.
    // Revision 2 remains an accurate recognizer with both Chinese scripts.
    // Use it only on the affected OS and only when Vision advertises it;
    // don't widen our deadline or downgrade other OS versions to fast OCR.
    if major == 27 && supported(2) {
        Some(2)
    } else if major >= 13 && supported(3) {
        Some(3)
    } else if supported(2) {
        Some(2)
    } else if supported(1) {
        Some(1)
    } else {
        None
    }
}

fn configure_languages(
    request: &VNRecognizeTextRequest,
    diagnostics: OcrDiagnostics,
) -> Result<String, String> {
    diagnostics.event("languages.begin", "none", None, None);
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    // Revision 3 and automatic language detection require macOS 13.0. The
    // bundle can still support 12.x using revision 2 and explicit languages.
    // This class method is declared on VNRequest; Rust associated functions
    // cannot inherit it through Deref. Query the actual recognizer class, not
    // VNRequest's base-class implementation. Its declared return is nonnull.
    let revisions: Retained<NSIndexSet> =
        unsafe { objc2::msg_send![VNRecognizeTextRequest::class(), supportedRevisions] };
    diagnostics.event("languages.revisions", "none", None, None);
    let major = NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion;
    let revision = recognition_revision(major, |revision| revisions.containsIndex(revision))
        .ok_or_else(|| "本机系统没有可用的文字识别版本".to_string())?;
    unsafe { request.setRevision(revision) };
    let supported =
        unsafe { request.supportedRecognitionLanguagesAndReturnError() }.map_err(|error| {
            diagnostics.event("languages.error", "native", Some(error.code()), None);
            native_error("读取文字识别语言", &error)
        })?;
    diagnostics.event("languages.supported", "none", None, Some(supported.len()));
    if supported.is_empty() || supported.len() > 128 {
        return Err("本机系统没有有效的文字识别语言".into());
    }
    let mut available = Vec::with_capacity(supported.len());
    for index in 0..supported.len() {
        let language = checked_text(&supported.objectAtIndex(index), 64)?;
        if language.trim().is_empty() || language.chars().any(char::is_control) {
            return Err("本机文字识别语言无效".into());
        }
        available.push(language);
    }
    let mut languages = ["zh-Hans", "zh-Hant", "en-US"]
        .into_iter()
        .filter(|wanted| available.iter().any(|language| language == wanted))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if languages.is_empty() {
        languages.push(available.remove(0));
    }
    let labels = languages
        .iter()
        .map(|language| NSString::from_str(language))
        .collect::<Vec<_>>();
    request.setRecognitionLanguages(&NSArray::from_retained_slice(&labels));
    request.setUsesLanguageCorrection(true);
    if objc2::available!(macos = 13.0) && revision >= 3 {
        request.setAutomaticallyDetectsLanguage(true);
    }
    diagnostics.event("languages.configured", "none", None, Some(languages.len()));
    // This records configured supported hints, not a fictitious per-line
    // detected language; Vision's recognized text does not expose that value.
    Ok(languages.join(","))
}

pub(super) fn recognize_impl(
    bitmap: PixelBuffer,
    cancel: Arc<AtomicBool>,
) -> Result<OcrDocument, String> {
    recognize_with_progress(bitmap, cancel, || {})
}

fn recognize_with_progress(
    bitmap: PixelBuffer,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn() + Send + Sync + 'static,
) -> Result<OcrDocument, String> {
    let started = Instant::now();
    let diagnostics = OcrDiagnostics::new(started);
    diagnostics.event("module.enter", "blocking_thread", None, None);
    let result = recognize_diagnosed(bitmap, cancel, on_progress, started, diagnostics);
    // Empty/cancelled/failed requests don't prove the recognizer was loaded.
    if result
        .as_ref()
        .is_ok_and(|document| !document.lines.is_empty())
    {
        COMPAT_MODEL_READY.store(true, Ordering::Release);
    }
    let reason = match &result {
        Ok(_) => "success",
        Err(error) if error == CANCEL_ERROR => "cancel_flag",
        Err(error) if error == TIMEOUT_ERROR => "deadline",
        Err(_) => "error",
    };
    diagnostics.event("module.return", reason, None, None);
    result
}

fn recognize_diagnosed(
    bitmap: PixelBuffer,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn() + Send + Sync + 'static,
    started: Instant,
    diagnostics: OcrDiagnostics,
) -> Result<OcrDocument, String> {
    check_interest_diagnosed(&cancel, started, TIMEOUT, diagnostics, "interest.entry")?;
    validate_pixels(&bitmap)?;
    diagnostics.event("permit.admission", "none", None, None);
    let _permit = WorkerPermit::acquire(diagnostics).inspect_err(|_| {
        diagnostics.event("permit.rejected", "capacity", None, None);
    })?;
    diagnostics.event("permit.acquired", "none", None, None);
    let geometry = Geometry::new(bitmap.width(), bitmap.height(), MAX_SIDE)?;
    diagnostics.event("prepare.begin", "none", None, None);
    let mut rgba = Zeroizing::new(prepare(bitmap, geometry, &cancel, started)?);
    diagnostics.event("prepare.return", "none", None, None);
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        if index % 16_384 == 0 {
            check_interest_diagnosed(&cancel, started, TIMEOUT, diagnostics, "interest.rgba")?;
        }
        // Shared preparation returns Windows BGRA. PNG requires RGBA.
        pixel.swap(0, 2);
    }
    let mut png = Zeroizing::new(Vec::new());
    diagnostics.event("png.begin", "none", None, None);
    PngEncoder::new(&mut *png)
        .write_image(
            &rgba,
            geometry.width,
            geometry.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法准备文字识别图片".to_string())?;
    diagnostics.event("png.return", "none", None, None);
    drop(rgba);
    if png.len() > MAX_PNG_BYTES {
        return Err("文字识别图片缓冲区超过限制".into());
    }
    check_interest_diagnosed(
        &cancel,
        started,
        TIMEOUT,
        diagnostics,
        "interest.pre_native",
    )?;
    autoreleasepool(move |_| {
        diagnostics.event("native.data.begin", "none", None, None);
        let data = NSData::with_bytes(&png);
        drop(png);
        diagnostics.event("native.request.begin", "none", None, None);
        let request = VNRecognizeTextRequest::new();
        diagnostics.event("native.request.created", "none", None, None);
        let language = configure_languages(&request, diagnostics)?;
        let timeout = native_timeout(
            NSProcessInfo::processInfo()
                .operatingSystemVersion()
                .majorVersion
                == 27,
            COMPAT_MODEL_READY.load(Ordering::Acquire),
        );
        let stop = cancel.clone();
        let cancel_sent = Arc::new(AtomicBool::new(false));
        let sent = cancel_sent.clone();
        let progress_count = Arc::new(AtomicUsize::new(0));
        let progress_observed = progress_count.clone();
        // Vision can call progress on a different queue. Only thread-safe Rust
        // state is captured: no Retained native object crosses a Rust thread.
        // The callback uses the valid request pointer supplied by Vision, never
        // a saved/raw pointer shared with an unrelated watchdog.
        let progress = RcBlock::new(
            move |native: NonNull<VNRequest>, _fraction: f64, error: *mut NSError| {
                on_progress();
                if diagnostics.enabled() {
                    let count = progress_observed.fetch_add(1, Ordering::Relaxed) + 1;
                    // Vision owns this optional error for the callback's
                    // duration. Read only its numeric code on this queue.
                    let code = unsafe { error.as_ref() }.map(NSError::code);
                    if count <= 64 {
                        diagnostics.event("perform.progress", "none", code, Some(count));
                    } else if count == 65 {
                        diagnostics.event("perform.progress", "log_limit", code, Some(count));
                    }
                }
                let externally_canceled = stop.load(Ordering::Acquire);
                let deadline_expired = started.elapsed() >= timeout;
                if (externally_canceled || deadline_expired) && !sent.swap(true, Ordering::AcqRel) {
                    let reason = match (externally_canceled, deadline_expired) {
                        (true, true) => "cancel_flag_and_deadline",
                        (true, false) => "cancel_flag",
                        _ => "deadline",
                    };
                    diagnostics.event("perform.cancel.request", reason, None, None);
                    // The request is alive for this Vision-owned invocation.
                    // Cancellation is only a request; it is not proof of exit.
                    unsafe { native.as_ref().cancel() };
                    diagnostics.event("perform.cancel.return", reason, None, None);
                }
            },
        );
        unsafe { request.setProgressHandler(RcBlock::as_ptr(&progress)) };
        let native_request: &VNRequest = &request;
        let requests = NSArray::from_slice(&[native_request]);
        diagnostics.event("handler.begin", "none", None, None);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        diagnostics.event("handler.created", "none", None, None);
        check_interest_diagnosed(
            &cancel,
            started,
            timeout,
            diagnostics,
            "interest.pre_perform",
        )?;
        // This synchronous call owns all request/input/block allocations until
        // it returns. Do not race it with an async timeout that drops this
        // stack. A stuck call keeps the native and caller-owned host permits.
        diagnostics.event("perform.begin", "none", None, None);
        let performed = handler.performRequests_error(&requests);
        if diagnostics.enabled() {
            // The synchronous call has actually returned. Counts may describe
            // partial results; they never bypass the interest/result checks.
            diagnostics.event(
                "perform.return",
                if performed.is_ok() {
                    "success"
                } else {
                    "native"
                },
                performed.as_ref().err().map(|error| error.code()),
                request.results().map(|results| results.len()),
            );
            diagnostics.event(
                "perform.progress.total",
                if cancel_sent.load(Ordering::Acquire) {
                    "cancel_sent"
                } else {
                    "none"
                },
                None,
                Some(progress_count.load(Ordering::Relaxed)),
            );
        }
        check_interest_diagnosed(
            &cancel,
            started,
            timeout,
            diagnostics,
            "interest.post_perform",
        )?;
        performed.map_err(|error| native_error("文字识别", &error))?;
        let observations = request.results().ok_or_else(|| {
            diagnostics.event("results.nil", "native", None, None);
            "本机文字识别没有返回有效结果".to_string()
        })?;
        diagnostics.event("results.begin", "none", None, Some(observations.len()));
        if observations.len() > MAX_LINES {
            return Err("识别行数超过限制，请缩小选区".into());
        }
        let mut lines = Vec::with_capacity(observations.len());
        let mut word_count = 0;
        let mut text_bytes = 0usize;
        for index in 0..observations.len() {
            check_interest_diagnosed(&cancel, started, timeout, diagnostics, "interest.results")?;
            let observation = observations.objectAtIndex(index);
            let candidates = observation.topCandidates(1);
            if candidates.is_empty() {
                continue;
            }
            let candidate = candidates.objectAtIndex(0);
            let text = checked_text(&candidate.string(), MAX_LINE_CHARS)?;
            if text.trim().is_empty() {
                continue;
            }
            text_bytes = text_bytes.saturating_add(text.len());
            if text_bytes > MAX_TEXT {
                return Err("识别文字超过长度限制，请缩小选区".into());
            }
            let mut located = Vec::new();
            for span in text_spans(&text) {
                check_interest_diagnosed(
                    &cancel,
                    started,
                    timeout,
                    diagnostics,
                    "interest.word_boxes",
                )?;
                let rectangle = unsafe { candidate.boundingBoxForRange_error(span.range) }
                    .map_err(|error| {
                        diagnostics.event("word_boxes.error", "native", Some(error.code()), None);
                        native_error("读取文字位置", &error)
                    })?;
                let rect = unsafe { rectangle.boundingBox() };
                located.push(LocatedSpan {
                    start: span.start,
                    end: span.end,
                    rect: original_rect(
                        geometry,
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    )?,
                });
            }
            // Accurate recognition supplies word boxes; repeated subranges of
            // a word can have the same box. Keep one contiguous word span. The
            // UI's existing equal-width grapheme layout remains an approximation.
            let words = words_from_spans(&text, located)?;
            word_count += words.len();
            text_bytes = text_bytes.saturating_add(words.iter().map(|word| word.text.len()).sum());
            if word_count > MAX_WORDS || text_bytes > MAX_TEXT {
                return Err("识别词数或文字长度超过限制，请缩小选区".into());
            }
            lines.push(OcrLine { text, words });
        }
        // Preserve the recognizer's observation order; no geometric heuristic
        // invents a column/paragraph order. Multiline fixtures must verify it.
        let document = OcrDocument {
            engine: "apple.vision".into(),
            language,
            width: geometry.source_width,
            height: geometry.source_height,
            text_angle: None,
            lines,
        };
        diagnostics.event("results.parsed", "none", None, Some(document.lines.len()));
        diagnostics.event("results.words", "none", None, Some(word_count));
        diagnostics.event("validation.begin", "none", None, None);
        validate_ocr_document(&document)
            .map_err(|_| "本机识别结果格式或大小无效，请缩小选区后重试".to_string())?;
        diagnostics.event("validation.return", "success", None, None);
        check_interest_diagnosed(&cancel, started, timeout, diagnostics, "interest.return")?;
        Ok(document)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba};
    use mewu_core::{Drawing, DrawingKind, DrawingPoint, Region};

    #[test]
    fn only_compatibility_model_startup_gets_extra_time_and_cancel_still_wins() {
        assert_eq!(native_timeout(false, false), TIMEOUT);
        assert_eq!(native_timeout(false, true), TIMEOUT);
        assert_eq!(native_timeout(true, true), TIMEOUT);
        assert_eq!(native_timeout(true, false), Duration::from_secs(30));
        let started = Instant::now() - Duration::from_secs(20);
        let diagnostics = OcrDiagnostics::new(started);
        let cancel = AtomicBool::new(false);
        assert!(check_interest_diagnosed(
            &cancel,
            started,
            native_timeout(true, false),
            diagnostics,
            "test.cold"
        )
        .is_ok());
        assert_eq!(
            check_interest_diagnosed(&cancel, started, TIMEOUT, diagnostics, "test.warm")
                .unwrap_err(),
            TIMEOUT_ERROR
        );
        cancel.store(true, Ordering::Release);
        assert_eq!(
            check_interest_diagnosed(
                &cancel,
                started,
                native_timeout(true, false),
                diagnostics,
                "test.cancel"
            )
            .unwrap_err(),
            CANCEL_ERROR
        );
    }

    #[test]
    fn revision_compatibility_is_os_scoped_and_requires_native_support() {
        let all = |revision| [1, 2, 3].contains(&revision);
        assert_eq!(recognition_revision(12, all), Some(2));
        assert_eq!(recognition_revision(13, all), Some(3));
        assert_eq!(recognition_revision(26, all), Some(3));
        assert_eq!(recognition_revision(27, all), Some(2));
        assert_eq!(recognition_revision(28, all), Some(3));
        assert_eq!(recognition_revision(27, |revision| revision == 3), Some(3));
        assert_eq!(recognition_revision(27, |revision| revision == 1), Some(1));
        assert_eq!(recognition_revision(27, |_| false), None);
    }

    #[test]
    fn normalized_lower_left_boxes_reverse_y_once_in_original_pixels() {
        let geometry = Geometry::new(1000, 500, MAX_SIDE).unwrap();
        let upper = original_rect(geometry, 0.1, 0.7, 0.2, 0.1).unwrap();
        assert!((upper.x - 100.).abs() < 1e-9);
        assert!((upper.y - 100.).abs() < 1e-9);
        assert_eq!((upper.width, upper.height), (200., 50.));
        let lower = original_rect(geometry, 0.1, 0.1, 0.2, 0.1).unwrap();
        assert!((lower.y - 400.).abs() < 1e-9);
        for rect in [
            (f64::NAN, 0., 0.1, 0.1),
            (0., 0., 0., 0.1),
            (0., 0., 0.1, -0.1),
            (0.9, 0., 0.2, 0.1),
            (0., 1.1, 0.1, 0.1),
        ] {
            assert!(original_rect(geometry, rect.0, rect.1, rect.2, rect.3).is_err());
        }
    }

    #[test]
    fn resized_box_uses_uniform_center_inverse_even_when_integer_sides_differ() {
        let geometry = Geometry::new(1001, 333, 500).unwrap();
        let scale = geometry.scale();
        let tx = (f64::from(geometry.width) - 1001. * scale) / 2.;
        let ty = (f64::from(geometry.height) - 333. * scale) / 2.;
        let left = 101. * scale + tx;
        let top = 73. * scale + ty;
        let width = 150. * scale;
        let height = 31. * scale;
        let rect = original_rect(
            geometry,
            left / f64::from(geometry.width),
            1. - (top + height) / f64::from(geometry.height),
            width / f64::from(geometry.width),
            height / f64::from(geometry.height),
        )
        .unwrap();
        for difference in [
            rect.x - 101.,
            rect.y - 73.,
            rect.width - 150.,
            rect.height - 31.,
        ] {
            assert!(difference.abs() < 1e-9);
        }
    }

    #[test]
    fn word_ranges_count_utf16_including_spaces_surrogates_and_combining_marks() {
        let text = "中😀A e\u{301}\t中文测试!";
        let spans = text_spans(text);
        assert!(!spans.is_empty());
        for span in &spans {
            assert_eq!(
                span.range.location,
                text[..span.start].encode_utf16().count()
            );
            assert_eq!(
                span.range.length,
                text[span.start..span.end].encode_utf16().count()
            );
            assert!(!text[span.start..span.end].trim().is_empty());
        }
        let emoji = spans
            .iter()
            .find(|span| &text[span.start..span.end] == "😀")
            .unwrap();
        assert_eq!((emoji.range.location, emoji.range.length), (1, 2));
        let combining = spans
            .iter()
            .find(|span| &text[span.start..span.end] == "e\u{301}")
            .unwrap();
        assert_eq!(combining.range.length, 2);
    }

    #[test]
    fn same_word_boxes_merge_only_adjacent_text_and_respect_word_limits() {
        let rect = PixelRect {
            x: 10.,
            y: 20.,
            width: 80.,
            height: 24.,
        };
        let other = PixelRect { x: 95., ..rect };
        let text = "中文 A";
        let words = words_from_spans(
            text,
            vec![
                LocatedSpan {
                    start: 0,
                    end: 3,
                    rect,
                },
                LocatedSpan {
                    start: 3,
                    end: 6,
                    rect,
                },
                LocatedSpan {
                    start: 7,
                    end: 8,
                    rect: other,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>(),
            ["中文", "A"]
        );
        assert_eq!(words[0].width, 80.);
        let separated = words_from_spans(
            "a b",
            vec![
                LocatedSpan {
                    start: 0,
                    end: 1,
                    rect,
                },
                LocatedSpan {
                    start: 2,
                    end: 3,
                    rect,
                },
            ],
        )
        .unwrap();
        assert_eq!(separated.len(), 2);
        let oversized = "字".repeat(MAX_WORD_CHARS + 1);
        assert!(words_from_spans(
            &oversized,
            vec![LocatedSpan {
                start: 0,
                end: oversized.len(),
                rect
            }]
        )
        .is_err());
    }

    fn render_text_fixture(base: PixelBuffer, items: &[(&str, f64, f64, f64)]) -> PixelBuffer {
        let width = base.width();
        let height = base.height();
        let image = DynamicImage::ImageRgba8(base);
        let drawings = items
            .iter()
            .map(|&(text, x, y, font_size)| Drawing {
                id: uuid::Uuid::new_v4().to_string(),
                kind: DrawingKind::Text,
                color: "#000000".into(),
                stroke_width: 1.,
                points: vec![DrawingPoint { x, y }],
                text: Some(text.into()),
                font_size: Some(font_size),
                text_style: None,
                origin: None,
                rich: None,
            })
            .collect();
        let rendered = crate::drawing_render::crop_region(
            &image,
            &Region {
                id: "mac-ocr-synthetic".into(),
                x: 0.,
                y: 0.,
                width: f64::from(width),
                height: f64::from(height),
                drawings,
                ..Default::default()
            },
        )
        .unwrap()
        .into_rgba8();
        for &(text, x, y, font_size) in items {
            let ink = rendered
                .enumerate_pixels()
                .filter(|(px, py, pixel)| {
                    f64::from(*px) >= x
                        && f64::from(*py) >= y
                        && f64::from(*py) < y + font_size * 1.5
                        && pixel[3] > 128
                        && pixel[0] < 128
                })
                .count();
            assert!(
                ink > 100,
                "synthetic OCR text was not rendered: {text:?}, visible ink={ink}"
            );
        }
        rendered
    }

    fn synthetic_text() -> PixelBuffer {
        render_text_fixture(
            RgbaImage::from_pixel(1400, 360, Rgba([255, 255, 255, 255])),
            &[
                ("Hello OCR test 12345", 48., 40., 64.),
                ("中文测试", 48., 200., 64.),
            ],
        )
    }

    fn compact_result(document: &OcrDocument) -> String {
        document
            .lines
            .iter()
            .flat_map(|line| line.text.chars())
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    #[ignore = "real Apple Vision on synthetic URL/contact/punctuation text; no links are opened"]
    fn real_vision_recognizes_synthetic_links_punctuation_and_contact_text() {
        let image = render_text_fixture(
            RgbaImage::from_pixel(1600, 600, Rgba([255, 255, 255, 255])),
            &[
                ("https://example.com/path?q=42&v=1", 48., 32., 56.),
                ("user@example.com", 48., 164., 64.),
                ("Phone +86 138 0000 1234", 48., 296., 64.),
                ("Punctuation: (test), [42]; A/B!", 48., 428., 56.),
            ],
        );
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        let text = compact_result(&document);
        eprintln!("synthetic URL/contact/punctuation Vision result: {text}");
        for expected in [
            "https://example.com/path?q=42&v=1",
            "user@example.com",
            "Phone+8613800001234",
        ] {
            assert!(
                text.contains(expected),
                "missing {expected:?}; actual: {text}"
            );
        }
        // Chinese-capable Vision revisions can emit full-width punctuation.
        // Keep URL/contact assertions exact above; here require every mark,
        // in order, in one of the two observed width forms. Production text
        // is never normalized, since width can be meaningful to the user.
        assert!(
            [
                "Punctuation:(test),[42];A/B!",
                "Punctuation：（test），［42］；A/B！"
            ]
            .iter()
            .any(|punctuation| text.contains(punctuation)),
            "punctuation missing or changed: {text}"
        );
        assert!(document.lines.iter().all(|line| !line.words.is_empty()));
        validate_ocr_document(&document).unwrap();
    }

    #[test]
    #[ignore = "real Apple Vision on synthetic transparent pixels including invisible RGB text"]
    fn real_vision_ignores_hidden_rgb_and_recognizes_transparent_text() {
        let mut base = render_text_fixture(
            RgbaImage::from_pixel(1400, 300, Rgba([255, 255, 255, 255])),
            &[("HIDDEN RGB", 48., 160., 64.)],
        );
        for pixel in base.pixels_mut() {
            pixel[3] = 0;
        }
        assert!(base.pixels().any(|pixel| pixel[0] < 128 && pixel[3] == 0));
        let image = render_text_fixture(base, &[("Transparent text 24680", 48., 24., 64.)]);
        assert!(image.pixels().any(|pixel| pixel[3] == 0));
        assert!(image.pixels().filter(|pixel| pixel[3] > 128).count() > 1000);
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        let text = compact_result(&document);
        eprintln!("synthetic transparent Vision result: {text}");
        assert!(text.contains("Transparenttext24680"), "actual: {text}");
        assert!(
            !text.contains("HIDDEN") && !text.contains("RGB"),
            "actual: {text}"
        );
        validate_ocr_document(&document).unwrap();
    }

    #[test]
    #[ignore = "real Apple Vision on an 8MP synthetic image reduced by the shared 4MP preparation"]
    fn real_vision_returns_original_coordinates_after_recognition_resize() {
        let image = render_text_fixture(
            RgbaImage::from_pixel(4000, 2000, Rgba([255, 255, 255, 255])),
            &[("Scaling check 67890", 640., 640., 128.)],
        );
        let geometry = Geometry::new(image.width(), image.height(), MAX_SIDE).unwrap();
        assert!(geometry.width < image.width() && geometry.height < image.height());
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        let text = compact_result(&document);
        eprintln!("synthetic scaled Vision result: {text}");
        assert!(text.contains("Scalingcheck67890"), "actual: {text}");
        assert_eq!((document.width, document.height), (4000, 2000));
        let words: Vec<_> = document.lines.iter().flat_map(|line| &line.words).collect();
        assert!(!words.is_empty());
        // Broad bounds cover font ascenders/descenders and OCR tolerance while
        // rejecting recognition-sized coordinates or a second Y inversion.
        assert!(words.iter().all(|word| word.x >= 600. && word.y >= 600.));
        assert!(words.iter().all(|word| word.y + word.height <= 840.));
        validate_ocr_document(&document).unwrap();
    }

    fn non_bmp_fixture_options(symbol: char) -> (resvg::usvg::Options<'static>, String) {
        use resvg::usvg;
        let mut options = usvg::Options::default();
        options.fontdb = crate::drawing_render::fonts().unwrap();
        // Load one explicit local outline font, without traversing font folders.
        // The resolver checks its real cmap before any fixture is rendered.
        // Core Text on the actual test Mac reports Apple Symbols lacks this
        // scalar; the installed STIX Two Math outline font supplies it.
        let font_path = "/System/Library/Fonts/Supplemental/STIXTwoMath.otf";
        let metadata = std::fs::metadata(font_path).expect("installed synthetic STIX font");
        use std::io::Read;
        assert!(metadata.len() <= 32 * 1024 * 1024);
        let mut bytes = Vec::new();
        std::fs::File::open(font_path)
            .unwrap()
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= 32 * 1024 * 1024);
        options.fontdb_mut().load_font_data(bytes);
        let base = options
            .fontdb
            .faces()
            .find(|face| face.post_script_name == "Helvetica")
            .expect("loaded exact Helvetica base without the non-BMP scalar")
            .id;
        let selector = usvg::FontResolver::default_fallback_selector();
        // This API excludes its base face: a single arbitrary base can itself
        // be the only face with the character and thus produce a false miss.
        // The known Helvetica base cannot support the fixture. usvg checks
        // actual face bytes through fontdb.with_face_data and skrifa's cmap,
        // whose map explicitly rejects GlyphId::NOTDEF (zero).
        let supporting = selector(symbol, &[base], &mut options.fontdb)
            .expect("a loaded outline font must actually support the non-BMP fixture");
        let supporting_face = options.fontdb.face(supporting).unwrap();
        assert_eq!(supporting_face.post_script_name, "STIXTwoMath-Regular");
        let family = supporting_face.families[0]
            .0
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        options.image_href_resolver = usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        };
        (options, family)
    }

    #[test]
    #[ignore = "real Apple Vision on an outline-font-supported non-BMP symbol; reports normalization"]
    fn real_vision_handles_supported_non_bmp_symbol_without_fabricating_boxes() {
        use resvg::{tiny_skia, usvg};
        let symbol = '\u{1d7d8}'; // MATHEMATICAL DOUBLE-STRUCK DIGIT ZERO.
        assert!(symbol as u32 > 0xffff);
        let (options, family) = non_bmp_fixture_options(symbol);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="240"><rect width="1400" height="240" fill="white"/><text x="48" y="140" fill="black" font-size="80" font-family="Helvetica">Before</text><text x="450" y="140" fill="black" font-size="80" font-family="{family}">{symbol}</text><text x="600" y="140" fill="black" font-size="80" font-family="Helvetica">After 34567</text></svg>"#
        );
        let tree = usvg::Tree::from_str(&svg, &options).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(1400, 240).unwrap();
        resvg::render(
            &tree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        let image = RgbaImage::from_raw(1400, 240, pixmap.data().to_vec()).unwrap();
        // The explicit white background makes every pixel opaque; only here
        // are tiny-skia's premultiplied bytes also valid straight RGBA input.
        assert!(image.pixels().all(|pixel| pixel[3] == 255));
        assert!(
            image
                .enumerate_pixels()
                .filter(|(x, _, pixel)| { (450..600).contains(x) && pixel[0] < 128 })
                .count()
                > 100,
            "the supported non-BMP glyph must produce visible ink"
        );
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        let text = compact_result(&document);
        eprintln!("synthetic U+1D7D8 Vision result: {text}");
        assert!(
            text.contains("Before") && text.contains("After34567"),
            "actual: {text}"
        );
        assert!(
            document
                .lines
                .iter()
                .flat_map(|line| &line.words)
                .any(|word| {
                    !word.text.is_empty()
                        && word.x < 550.
                        && word.x + word.width > 450.
                }),
            "the observed token for the non-BMP ink must keep its real position: {text}"
        );
        // This tests the adapter's text/range/geometry handling, not a promise
        // of perfect recognition by Apple's model. macOS 27 revision 2 can
        // classify this isolated double-struck zero as a circled one. Preserve
        // that actual result; never fabricate a zero or claim a surrogate was
        // preserved. The separate numeric-context test still requires zero.
        // If Vision does return non-BMP text, its UTF-16 ranges stay exact.
        for line in &document.lines {
            for span in text_spans(&line.text) {
                assert_eq!(
                    span.range.location,
                    line.text[..span.start].encode_utf16().count()
                );
                assert_eq!(
                    span.range.length,
                    line.text[span.start..span.end].encode_utf16().count()
                );
            }
        }
        validate_ocr_document(&document).unwrap();
    }

    #[test]
    #[ignore = "real Apple Vision on a supported non-BMP digit in a complete numeric token"]
    fn real_vision_recognizes_supported_non_bmp_digit_in_numeric_context() {
        use resvg::{tiny_skia, usvg};
        let symbol = '\u{1d7d8}';
        let (options, family) = non_bmp_fixture_options(symbol);
        let render = |symbol_fill: &str| {
            // Adjacent spans retain ordinary numeric context without replacing
            // the genuine mathematical glyph with an ASCII lookalike.
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="240"><rect width="1400" height="240" fill="white"/><text x="48" y="140" fill="black" font-size="80" font-family="Helvetica">Serial 12<tspan fill="{symbol_fill}" font-family="{family}">{symbol}</tspan><tspan>34567</tspan></text></svg>"#
            );
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let mut pixmap = tiny_skia::Pixmap::new(1400, 240).unwrap();
            resvg::render(
                &tree,
                tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            let image = RgbaImage::from_raw(1400, 240, pixmap.data().to_vec()).unwrap();
            assert!(image.pixels().all(|pixel| pixel[3] == 255));
            image
        };
        let image = render("black");
        // Changing only glyph fill preserves advance widths and the positions
        // of every neighbouring character. Differences identify actual U+1D7D8
        // ink, rather than counting unrelated text inside a guessed rectangle.
        let control = render("white");
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (1400, 240, 0, 0);
        let mut ink = 0;
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel[0] < 128 && control.get_pixel(x, y)[0] >= 128 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                ink += 1;
            }
        }
        assert!(ink > 100, "the contextual non-BMP glyph must produce ink");
        assert!(min_x > 48 && max_x < 1000 && min_y > 40 && max_y < 160);
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        let text = compact_result(&document);
        eprintln!(
            "synthetic contextual U+1D7D8 Vision result: {text}; ink={ink} bounds=({min_x},{min_y})..({max_x},{max_y})"
        );
        // Exact complete-token recognition permits only styled
        // digit normalization; dropping the symbol cannot satisfy this check.
        assert_eq!(text.replace(symbol, "0"), "Serial12034567");
        assert!(
            document
                .lines
                .iter()
                .flat_map(|line| &line.words)
                .any(|word| {
                    word.text.replace(symbol, "0").contains("12034567")
                        && word.x < f64::from(max_x)
                        && word.x + word.width > f64::from(min_x)
                        && word.y < f64::from(max_y)
                        && word.y + word.height > f64::from(min_y)
                }),
            "the recognized numeric token must overlap the real non-BMP ink: {text}"
        );
        validate_ocr_document(&document).unwrap();
    }

    #[test]
    #[ignore = "explicit Apple Vision OCR on synthetic text only; captures no desktop or model"]
    fn real_vision_recognizes_mixed_text_blank_and_active_cancellation() {
        let image = synthetic_text();
        assert!(image.pixels().filter(|pixel| pixel[0] < 128).count() > 1000);
        let result = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!((result.width, result.height), (1400, 360));
        assert_eq!(result.engine, "apple.vision");
        assert_eq!(result.text_angle, None);
        let text: String = result
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            text.contains("HelloOCRtest12345"),
            "actual Vision result: {text}"
        );
        assert!(text.contains("中文测试"), "actual Vision result: {text}");
        let english = result
            .lines
            .iter()
            .find(|line| line.text.contains("Hello"))
            .unwrap();
        let chinese = result
            .lines
            .iter()
            .find(|line| line.text.contains("中文"))
            .unwrap();
        assert!(english.words.iter().all(|word| word.y < 150.));
        assert!(chinese.words.iter().all(|word| word.y > 150.));
        validate_ocr_document(&result).unwrap();
        let blank = recognize_impl(
            RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255])),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(blank.lines.is_empty());

        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let observed = Arc::new(AtomicBool::new(false));
        let running = observed.clone();
        let canceled = recognize_with_progress(synthetic_text(), cancel, move || {
            running.store(
                ACTIVE_WORKERS.load(Ordering::Acquire) > 0,
                Ordering::Release,
            );
            stop.store(true, Ordering::Release);
        });
        assert!(
            observed.load(Ordering::Acquire),
            "Vision must enter a real progress callback"
        );
        assert_eq!(canceled.unwrap_err(), CANCEL_ERROR);
        // A subsequent actual request verifies ordinary cleanup after the
        // canceled perform call returned; no native result is mocked.
        assert!(recognize_impl(
            RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255])),
            Arc::new(AtomicBool::new(false))
        )
        .unwrap()
        .lines
        .is_empty());
    }
}
