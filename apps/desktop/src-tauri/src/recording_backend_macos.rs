// SPDX-License-Identifier: MPL-2.0
//! ScreenCaptureKit and AVAssetWriter live solely in the owned recording child.
//! The parent still owns startup/finalize deadlines and actual process/IO drain.
//! https://developer.apple.com/documentation/screencapturekit/scstream
//! https://developer.apple.com/documentation/avfoundation/avassetwriter
use super::{AudioMode, Config, Control, Progress, Report, FPS, MAX_ACTIVE};
use crate::recording_audio_timeline::{PacketPart, Timeline, TICKS_PER_SECOND};
use block2::RcBlock;
use core_foundation::{
    array::{CFArrayGetCount, CFArrayGetValueAtIndex},
    base::{CFGetTypeID, CFTypeRef},
    dictionary::{CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef},
    number::{kCFNumberSInt64Type, CFNumberGetTypeID, CFNumberGetValue},
};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{
    define_class, msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyObject, ProtocolObject},
    AnyThread, DefinedClass,
};
use objc2_av_foundation::*;
use objc2_avf_audio::{AVEncoderBitRateKey, AVFormatIDKey, AVNumberOfChannelsKey, AVSampleRateKey};
use objc2_core_audio_types::{kAudioFormatFlagIsNonInterleaved, AudioBuffer, AudioBufferList};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGDisplayBounds, CGDisplayPixelsHigh, CGDisplayPixelsWide};
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMClock, CMSampleBuffer, CMSampleTimingInfo,
    CMTime, CMTimeFlags,
};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_screen_capture_kit::*;
use std::{
    collections::VecDeque,
    fs::File,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::NonNull,
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const MAX_AUDIO_FRAMES: usize = RATE as usize * 2;
const POLL: Duration = Duration::from_millis(15);
const MAX_BACKPRESSURE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Backpressure {
    since: Option<Instant>,
}
impl Backpressure {
    fn waiting(&mut self, now: Instant) -> Result<(), String> {
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= MAX_BACKPRESSURE {
            return Err("录屏编码积压超过两秒".into());
        }
        Ok(())
    }
    fn accepted(&mut self) {
        self.since = None;
    }
}

// SCK delivers immutable, ready samples. We retain them, never invalidate or
// modify them, and only create separately owned timing/range copies for writing.
// The mutex transfers ownership to the single writer; no mutable buffer access.
struct Sample(Retained<CMSampleBuffer>);
unsafe impl Send for Sample {}
impl Sample {
    fn duplicate(&self) -> Self {
        Self(self.0.clone())
    }
}
struct Content(Retained<SCShareableContent>);
// Immutable SCK enumeration is transferred once after its completion callback.
unsafe impl Send for Content {}

#[derive(Default)]
struct Queue {
    latest: Option<Sample>,
    audio: VecDeque<Sample>,
    audio_frames: usize,
    error: Option<String>,
    #[cfg(debug_assertions)]
    audio_metadata_logged: bool,
}
struct HandlerState {
    queue: Arc<Mutex<Queue>>,
    width: u32,
    height: u32,
}
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = AnyThread]
    #[ivars = HandlerState]
    struct Handler;
    unsafe impl NSObjectProtocol for Handler {}
    unsafe impl SCStreamOutput for Handler {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn output(&self, _: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            let result = catch_unwind(AssertUnwindSafe(|| {
                autoreleasepool(|_| self.accept(sample, kind))
            }));
            let error = match result {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e,
                Err(_) => "原生录屏回调失败".into(),
            };
            if let Ok(mut queue) = self.ivars().queue.lock() {
                queue.error.get_or_insert(error);
            }
        }
    }
    unsafe impl SCStreamDelegate for Handler {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn stopped(&self, _: &SCStream, error: &NSError) {
            if let Ok(mut queue) = self.ivars().queue.lock() {
                queue
                    .error
                    .get_or_insert_with(|| failure("屏幕或电脑声音采集已结束", error));
            }
        }
    }
);
impl Handler {
    fn new(queue: Arc<Mutex<Queue>>, config: &Config) -> Retained<Self> {
        let this = Self::alloc().set_ivars(HandlerState {
            queue,
            width: config.width,
            height: config.height,
        });
        unsafe { msg_send![super(this), init] }
    }
    fn accept(&self, sample: &CMSampleBuffer, kind: SCStreamOutputType) -> Result<(), String> {
        if !unsafe { sample.data_is_ready() } {
            return Ok(());
        }
        let mut queue = self.ivars().queue.lock().map_err(|_| "录屏帧状态不可用")?;
        if queue.error.is_some() {
            return Ok(());
        }
        if kind == SCStreamOutputType::Screen {
            if !complete_frame(sample)? {
                return Ok(());
            }
            let image = unsafe { sample.image_buffer() }.ok_or("原生录屏帧缺少像素")?;
            let pixel =
                unsafe { &*((&*image as *const _) as *const objc2_core_video::CVPixelBuffer) };
            if unsafe { CVPixelBufferGetWidth(pixel) } != self.ivars().width as usize
                || unsafe { CVPixelBufferGetHeight(pixel) } != self.ivars().height as usize
                || unsafe { CVPixelBufferGetPixelFormatType(pixel) } != kCVPixelFormatType_32BGRA
                || unsafe { sample.num_samples() } != 1
            {
                return Err("原生录屏帧尺寸或像素格式已改变".into());
            }
            host_ticks(unsafe { sample.presentation_time_stamp() })?;
            queue.latest = Some(Sample(
                unsafe { Retained::retain(sample as *const _ as *mut _) }
                    .ok_or("无法持有录屏帧")?,
            ));
        } else if kind == SCStreamOutputType::Audio {
            let count =
                usize::try_from(unsafe { sample.num_samples() }).map_err(|_| "电脑声音帧数无效")?;
            if count == 0 {
                return Ok(());
            }
            let format = unsafe { sample.format_description() }.ok_or("电脑声音格式缺失")?;
            let asbd = unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format) };
            let asbd = unsafe { asbd.as_ref() }.ok_or("电脑声音 PCM 格式缺失")?;
            if asbd.mFormatID != u32::from_be_bytes(*b"lpcm")
                || asbd.mSampleRate != f64::from(RATE)
                || asbd.mChannelsPerFrame != 2
                || count > RATE as usize / 2
            {
                return Err("电脑声音格式已改变，请重新录制".into());
            }
            host_ticks(unsafe { sample.presentation_time_stamp() })?;
            if queue
                .audio_frames
                .checked_add(count)
                .is_none_or(|n| n > MAX_AUDIO_FRAMES)
            {
                return Err("电脑声音采集队列已满，录制已停止".into());
            }
            queue.audio_frames += count;
            #[cfg(debug_assertions)]
            if !queue.audio_metadata_logged {
                audio_metadata_diagnostic(asbd);
                queue.audio_metadata_logged = true;
            }
            queue.audio.push_back(Sample(
                unsafe { Retained::retain(sample as *const _ as *mut _) }
                    .ok_or("无法持有电脑声音")?,
            ));
        }
        Ok(())
    }
}
// Public Foundation ABI, also declared in the installed Apple SDK's
// NSObjCRuntime.h. Only bounded numeric ASBD metadata reaches the system log;
// worker stdout/stderr and the recording protocol remain unchanged.
// https://developer.apple.com/documentation/Foundation/NSLog
#[cfg(debug_assertions)]
fn audio_metadata_diagnostic(asbd: &objc2_core_audio_types::AudioStreamBasicDescription) {
    if std::env::var_os("MEWU_MAC_RECORDING_DIAGNOSTICS").as_deref()
        != Some(std::ffi::OsStr::new("1"))
        || asbd.mSampleRate != f64::from(RATE)
        || asbd.mChannelsPerFrame != 2
        || !(1..=16).contains(&asbd.mBytesPerFrame)
        || !(1..=64).contains(&asbd.mBitsPerChannel)
    {
        return;
    }
    let planar = u8::from(asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0);
    let message = NSString::from_str(&format!(
        "mewu_mac_recording_audio planar={planar} rate={RATE} channels=2 bits={} bpf={} flags={}",
        asbd.mBitsPerChannel, asbd.mBytesPerFrame, asbd.mFormatFlags
    ));
    unsafe extern "C-unwind" {
        fn NSLog(format: *const NSString, ...);
    }
    // This format contains only our literals and decimal integers, never '%'.
    unsafe { NSLog(&*message) };
}
fn complete_frame(sample: &CMSampleBuffer) -> Result<bool, String> {
    let attachments = unsafe { sample.sample_attachments_array(false) }.ok_or("屏幕帧状态缺失")?;
    unsafe {
        let array = &*attachments as *const _ as core_foundation::array::CFArrayRef;
        if CFArrayGetCount(array) != 1 {
            return Err("屏幕帧状态数量无效".into());
        }
        let value = CFArrayGetValueAtIndex(array, 0);
        if value.is_null() || CFGetTypeID(value as CFTypeRef) != CFDictionaryGetTypeID() {
            return Err("屏幕帧状态类型无效".into());
        }
        let key = SCStreamFrameInfoStatus;
        let status = CFDictionaryGetValue(value as CFDictionaryRef, key as *const _ as *const _);
        if status.is_null() || CFGetTypeID(status as CFTypeRef) != CFNumberGetTypeID() {
            return Err("屏幕帧状态无效".into());
        }
        let mut number = -1i64;
        if !CFNumberGetValue(status as _, kCFNumberSInt64Type, &mut number as *mut _ as _) {
            return Err("屏幕帧状态无法读取".into());
        }
        Ok(number == 0) // SCFrameStatusComplete; idle/stopped frames are not pixels.
    }
}
fn failure(stage: &str, error: &NSError) -> String {
    format!("{stage}（{}:{}）", error.domain(), error.code())
}
fn time(ticks: u64) -> CMTime {
    CMTime {
        value: ticks as i64,
        timescale: TICKS_PER_SECOND as i32,
        flags: CMTimeFlags::Valid,
        epoch: 0,
    }
}
fn host_ticks(value: CMTime) -> Result<u64, String> {
    if !value.flags.contains(CMTimeFlags::Valid)
        || value.flags.intersects(
            CMTimeFlags::PositiveInfinity | CMTimeFlags::NegativeInfinity | CMTimeFlags::Indefinite,
        )
        || value.value < 0
        || value.timescale <= 0
        || value.epoch != 0
    {
        return Err("录屏时间戳无效".into());
    }
    u64::try_from(
        i128::from(value.value) * i128::from(TICKS_PER_SECOND) / i128::from(value.timescale),
    )
    .map_err(|_| "录屏时间戳溢出".into())
}
fn now() -> Result<u64, String> {
    host_ticks(unsafe { CMClock::host_time_clock().time() })
}

// An escaping block's final native release is distinct from its callback firing.
#[derive(Default)]
struct Flights {
    count: Mutex<usize>,
    cv: Condvar,
}
struct Lease(Arc<Flights>);
impl Lease {
    fn new(flights: &Arc<Flights>) -> Self {
        *flights.count.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        Self(flights.clone())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut n = self.0.count.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        self.0.cv.notify_all();
    }
}
impl Flights {
    fn drain(&self) {
        let mut n = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *n != 0 {
            n = self.cv.wait(n).unwrap_or_else(|e| e.into_inner());
        }
    }
}
fn completion(
    f: impl FnOnce(&RcBlock<dyn Fn(*mut NSError)>),
    stage: &'static str,
) -> Result<(), String> {
    let flights = Arc::new(Flights::default());
    let lease = Lease::new(&flights);
    let (tx, rx) = mpsc::sync_channel(1);
    let block = RcBlock::new(move |error: *mut NSError| {
        let _ = &lease;
        let _ = tx.try_send(unsafe { error.as_ref() }.map_or(Ok(()), |e| Err(failure(stage, e))));
    });
    f(&block);
    drop(block);
    let result = rx.recv().map_err(|_| "原生录屏回调已中断")?;
    flights.drain();
    result
}
fn shareable() -> Result<Retained<SCShareableContent>, String> {
    let flights = Arc::new(Flights::default());
    let lease = Lease::new(&flights);
    let (tx, rx) = mpsc::sync_channel(1);
    let block = RcBlock::new(move |value: *mut SCShareableContent, error: *mut NSError| {
        let _ = &lease;
        let result = if let Some(e) = unsafe { error.as_ref() } {
            Err(failure("无法获取录屏显示器", e))
        } else {
            unsafe { Retained::retain(value) }
                .map(Content)
                .ok_or_else(|| "录屏显示器列表缺失".to_string())
        };
        let _ = tx.try_send(result);
    });
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(false,false,&block);
    }
    drop(block);
    let result = rx.recv().map_err(|_| "录屏显示器回调中断")?;
    flights.drain();
    result.map(|v| v.0)
}
struct Capture {
    stream: Retained<SCStream>,
    audio_stream: Option<Retained<SCStream>>,
    handler: Retained<Handler>,
    dispatch: DispatchRetained<DispatchQueue>,
    active: bool,
    display: u32,
    coordinates: mewu_core::CaptureCoordinates,
    pixels: (u32, u32),
}
impl Capture {
    fn new(config: &Config, queue: Arc<Mutex<Queue>>) -> Result<Self, String> {
        crate::capture_permission_macos::preflight()?;
        if !config
            .capture_coordinates
            .valid_for(config.monitor_width, config.monitor_height)
            || config.capture_coordinates.space != mewu_core::CaptureCoordinateSpace::QuartzPoints
        {
            return Err("Mac 录屏屏幕坐标无效".into());
        }
        let content = shareable()?;
        let displays = unsafe { content.displays() };
        let applications = unsafe { content.applications() };
        if displays.is_empty() || displays.len() > 64 || applications.len() > 4096 {
            return Err("录屏显示器或应用列表超出限制".into());
        }
        let wanted = &config.capture_coordinates;
        let mut matches = Vec::new();
        for display in displays.iter() {
            let frame = unsafe { display.frame() };
            if frame.origin.x == wanted.origin_x
                && frame.origin.y == wanted.origin_y
                && frame.size.width == wanted.width
                && frame.size.height == wanted.height
            {
                matches.push(display);
            }
        }
        if matches.len() != 1 {
            return Err("显示器已改变，请重新截图".into());
        }
        let display = &matches[0];
        let parent = unsafe { libc::getppid() };
        if parent <= 1 {
            return Err("录屏父进程已结束".into());
        }
        let own: Vec<_> = applications
            .iter()
            .filter(|a| unsafe { a.processID() } == parent)
            .collect();
        if own.len() != 1 {
            return Err("无法排除当前应用的录屏覆盖层".into());
        }
        let excluded = NSArray::from_retained_slice(&own);
        let windows = NSArray::<SCWindow>::new();
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                display,
                &excluded,
                &windows,
            )
        };
        let sc = unsafe { SCStreamConfiguration::new() };
        unsafe {
            sc.setWidth(config.width as usize);
            sc.setHeight(config.height as usize);
            sc.setPixelFormat(kCVPixelFormatType_32BGRA);
            sc.setMinimumFrameInterval(CMTime {
                value: 1,
                timescale: FPS as i32,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            });
            sc.setQueueDepth(3);
            sc.setShowsCursor(true);
            sc.setSourceRect(CGRect {
                origin: CGPoint {
                    x: f64::from(config.x) / f64::from(config.monitor_width) * wanted.width,
                    y: f64::from(config.y) / f64::from(config.monitor_height) * wanted.height,
                },
                size: CGSize {
                    width: f64::from(config.width) / f64::from(config.monitor_width) * wanted.width,
                    height: f64::from(config.height) / f64::from(config.monitor_height)
                        * wanted.height,
                },
            });
            sc.setCapturesAudio(false);
            sc.setSampleRate(RATE as isize);
            sc.setChannelCount(2);
        }
        let handler = Handler::new(queue, config);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &sc,
                Some(ProtocolObject::from_ref(&*handler)),
            )
        };
        let dispatch = DispatchQueue::new("dev.mewu.recording.samples", DispatchQueueAttr::SERIAL);
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*handler),
                    SCStreamOutputType::Screen,
                    Some(&dispatch),
                )
                .map_err(|e| failure("无法接入屏幕帧", &e))?;
        }
        // Application exclusion must not filter computer sound. A separate
        // audio-only output uses the same display/host clock without excluding
        // apps. It delivers real PCM only; no microphone or synthetic silence.
        let audio_stream = if config.audio == AudioMode::System {
            let audio_filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    display,
                    &windows,
                )
            };
            let audio_config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                audio_config.setWidth(2);
                audio_config.setHeight(2);
                audio_config.setMinimumFrameInterval(CMTime {
                    value: 1,
                    timescale: 1,
                    flags: CMTimeFlags::Valid,
                    epoch: 0,
                });
                audio_config.setQueueDepth(3);
                audio_config.setCapturesAudio(true);
                audio_config.setSampleRate(RATE as isize);
                audio_config.setChannelCount(2);
                audio_config.setExcludesCurrentProcessAudio(false);
            }
            let audio_stream = unsafe {
                SCStream::initWithFilter_configuration_delegate(
                    SCStream::alloc(),
                    &audio_filter,
                    &audio_config,
                    Some(ProtocolObject::from_ref(&*handler)),
                )
            };
            unsafe {
                audio_stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        ProtocolObject::from_ref(&*handler),
                        SCStreamOutputType::Audio,
                        Some(&dispatch),
                    )
                    .map_err(|e| failure("无法接入电脑声音", &e))?;
            }
            Some(audio_stream)
        } else {
            None
        };
        let mut capture = Self {
            stream,
            audio_stream,
            handler,
            dispatch,
            active: true,
            display: unsafe { display.displayID() },
            coordinates: *wanted,
            pixels: (config.monitor_width, config.monitor_height),
        };
        let result = completion(
            |block| unsafe {
                capture
                    .stream
                    .startCaptureWithCompletionHandler(Some(block))
            },
            "无法启动屏幕录制",
        );
        if let Err(error) = result {
            let _ = capture.stop();
            return Err(error);
        }
        if let Some(audio_stream) = &capture.audio_stream {
            let result = completion(
                |block| unsafe { audio_stream.startCaptureWithCompletionHandler(Some(block)) },
                "无法启动电脑声音采集",
            );
            if let Err(error) = result {
                let _ = capture.stop();
                return Err(error);
            }
        }
        capture.check_display()?;
        Ok(capture)
    }
    fn check_display(&self) -> Result<(), String> {
        let b = CGDisplayBounds(self.display);
        let c = &self.coordinates;
        if b.origin.x != c.origin_x
            || b.origin.y != c.origin_y
            || b.size.width != c.width
            || b.size.height != c.height
            || CGDisplayPixelsWide(self.display) != self.pixels.0 as usize
            || CGDisplayPixelsHigh(self.display) != self.pixels.1 as usize
        {
            Err("显示器已改变，录制已停止".into())
        } else {
            Ok(())
        }
    }
    fn stop(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let result = completion(
            |block| unsafe { self.stream.stopCaptureWithCompletionHandler(Some(block)) },
            "停止屏幕采集失败",
        );
        let screen = unsafe {
            self.stream.removeStreamOutput_type_error(
                ProtocolObject::from_ref(&*self.handler),
                SCStreamOutputType::Screen,
            )
        }
        .map_err(|e| failure("无法移除屏幕回调", &e));
        let audio = if let Some(audio_stream) = &self.audio_stream {
            let result = completion(
                |block| unsafe { audio_stream.stopCaptureWithCompletionHandler(Some(block)) },
                "停止电脑声音采集失败",
            );
            let remove = unsafe {
                audio_stream.removeStreamOutput_type_error(
                    ProtocolObject::from_ref(&*self.handler),
                    SCStreamOutputType::Audio,
                )
            }
            .map_err(|e| failure("无法移除声音回调", &e));
            result.and(remove)
        } else {
            Ok(())
        };
        // Completion alone does not drain previously queued sample callbacks.
        self.dispatch.exec_sync(|| {});
        self.active = false;
        result.and(screen).and(audio)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn copied_timing(
    source: &CMSampleBuffer,
    start: u64,
    duration: CMTime,
) -> Result<Retained<CMSampleBuffer>, String> {
    let timing = CMSampleTimingInfo {
        duration,
        presentationTimeStamp: time(start),
        decodeTimeStamp: CMTime {
            value: 0,
            timescale: 0,
            flags: CMTimeFlags(0),
            epoch: 0,
        },
    };
    let mut result = std::ptr::null_mut();
    if unsafe {
        CMSampleBuffer::create_copy_with_new_timing(
            None,
            source,
            1,
            &timing,
            NonNull::from(&mut result),
        )
    } != 0
    {
        return Err("无法重设录屏样本时间".into());
    }
    unsafe { Retained::from_raw(result) }.ok_or_else(|| "录屏样本时间副本缺失".into())
}
// The host's private recording-staging/UUID.mp4 remains the actual file.
// Parent size monitoring, crash recovery and fully-exited cleanup keep their
// original target; no untracked hidden staging directory is introduced.
struct Output {
    path: std::path::PathBuf,
    parent_path: std::path::PathBuf,
    parent: File,
    file: Option<File>,
    complete: bool,
}
fn file_identity(file: &File) -> Result<(u64, u64), String> {
    let m = file.metadata().map_err(|_| "无法检查录屏文件句柄")?;
    Ok((m.dev(), m.ino()))
}
impl Output {
    fn new(path: &std::path::Path) -> Result<Self, String> {
        let parent_path = path.parent().ok_or("录屏输出目录缺失")?;
        if parent_path
            .canonicalize()
            .map_err(|_| "录屏输出目录不可用")?
            != parent_path
        {
            return Err("录屏输出目录不能经过链接".into());
        }
        let parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent_path)
            .map_err(|_| "无法持有录屏输出目录")?;
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err("录屏输出文件已存在，不能覆盖".into()),
        }
        let result = Self {
            path: path.to_owned(),
            parent_path: parent_path.to_owned(),
            parent,
            file: None,
            complete: false,
        };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<(), String> {
        let parent =
            std::fs::symlink_metadata(&self.parent_path).map_err(|_| "录屏输出目录不可用")?;
        if !parent.is_dir()
            || parent.file_type().is_symlink()
            || (parent.dev(), parent.ino()) != file_identity(&self.parent)?
        {
            return Err("录屏输出目录已改变".into());
        }
        if let Some(file) = &self.file {
            let m = std::fs::symlink_metadata(&self.path).map_err(|_| "录屏文件已移走")?;
            if !m.is_file()
                || m.file_type().is_symlink()
                || (m.dev(), m.ino()) != file_identity(file)?
            {
                return Err("录屏文件已改变".into());
            }
        }
        Ok(())
    }
    fn hold(&mut self) -> Result<(), String> {
        self.check()?;
        if self.file.is_none() {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&self.path)
                .map_err(|_| "无法持有实际录屏文件")?;
            if !file.metadata().map_err(|_| "无法检查录屏文件")?.is_file() {
                return Err("录屏输出不是普通文件".into());
            }
            self.file = Some(file);
            self.check()?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.check()?;
        self.file
            .as_ref()
            .ok_or("实际录屏文件缺失")?
            .sync_all()
            .map_err(|_| "无法同步录屏文件")?;
        self.complete = true;
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        // Writer drops first; cancelWriting blocks until its actual work ends.
        // Keep the old FD alive and never unlink a replacement file.
        if !self.complete && self.file.is_some() && self.check().is_ok() {
            if let Some(name) = self.path.file_name() {
                if let Ok(name) = std::ffi::CString::new(name.as_bytes()) {
                    unsafe {
                        libc::unlinkat(self.parent.as_raw_fd(), name.as_ptr(), 0);
                    }
                }
            }
        }
    }
}
struct Writer {
    writer: Retained<AVAssetWriter>,
    video: Retained<AVAssetWriterInput>,
    audio: Option<Retained<AVAssetWriterInput>>,
    finished: bool,
    video_finished: bool,
    backpressure: Mutex<[Backpressure; 2]>,
}
impl Writer {
    fn new(
        path: &std::path::Path,
        config: &Config,
        first_audio: Option<&CMSampleBuffer>,
    ) -> Result<Self, String> {
        let path = path.to_str().ok_or("录屏输出路径无效")?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let writer = unsafe {
            AVAssetWriter::initWithURL_fileType_error(
                AVAssetWriter::alloc(),
                &url,
                AVFileTypeMPEG4.ok_or("MP4 编码不可用")?,
            )
        }
        .map_err(|e| failure("无法创建 MP4 编码器", &e))?;
        let width = NSNumber::numberWithUnsignedInt(config.width);
        let height = NSNumber::numberWithUnsignedInt(config.height);
        let fps = NSNumber::numberWithUnsignedInt(FPS);
        let settings = unsafe {
            NSDictionary::from_slices(
                &[
                    AVVideoCodecKey.ok_or("H264 配置不可用")?,
                    AVVideoWidthKey.ok_or("H264 配置不可用")?,
                    AVVideoHeightKey.ok_or("H264 配置不可用")?,
                    AVVideoCompressionPropertiesKey.ok_or("H264 配置不可用")?,
                ],
                &[
                    AVVideoCodecTypeH264.ok_or("H264 不可用")? as &AnyObject,
                    &*width,
                    &*height,
                    &*NSDictionary::from_slices(
                        &[AVVideoExpectedSourceFrameRateKey.ok_or("H264 帧率不可用")?],
                        &[&*fps as &AnyObject],
                    ),
                ],
            )
        };
        let video = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                AVMediaTypeVideo.ok_or("视频编码不可用")?,
                Some(&settings),
            )
        };
        unsafe {
            video.setExpectsMediaDataInRealTime(true);
            video.setMediaTimeScale(TICKS_PER_SECOND as i32);
        }
        if !unsafe { writer.canAddInput(&video) } {
            return Err("无法配置 H264 编码器".into());
        }
        unsafe {
            writer.addInput(&video);
        }
        let audio = if config.audio == AudioMode::System {
            let first = first_audio.ok_or("电脑声音尚未就绪")?;
            let hint = unsafe { first.format_description() }.ok_or("电脑声音格式缺失")?;
            let format = NSNumber::numberWithUnsignedInt(u32::from_be_bytes(*b"aac "));
            let rate = NSNumber::numberWithUnsignedInt(RATE);
            let channels = NSNumber::numberWithUnsignedInt(2);
            let bitrate = NSNumber::numberWithUnsignedInt(192_000);
            let settings = unsafe {
                NSDictionary::from_slices(
                    &[
                        AVFormatIDKey.ok_or("AAC 不可用")?,
                        AVSampleRateKey.ok_or("AAC 不可用")?,
                        AVNumberOfChannelsKey.ok_or("AAC 不可用")?,
                        AVEncoderBitRateKey.ok_or("AAC 不可用")?,
                    ],
                    &[&*format as &AnyObject, &*rate, &*channels, &*bitrate],
                )
            };
            let input = unsafe {
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                    AVMediaTypeAudio.ok_or("声音编码不可用")?,
                    Some(&settings),
                    Some(&hint),
                )
            };
            unsafe {
                input.setExpectsMediaDataInRealTime(true);
            }
            if !unsafe { writer.canAddInput(&input) } {
                return Err("无法配置 AAC 编码器".into());
            }
            unsafe {
                writer.addInput(&input);
            }
            Some(input)
        } else {
            None
        };
        let mut this = Self {
            writer,
            video,
            audio,
            finished: false,
            video_finished: false,
            backpressure: Mutex::new([Backpressure::default(), Backpressure::default()]),
        };
        if !unsafe { this.writer.startWriting() } {
            return Err(this.error("无法启动 MP4 编码"));
        }
        unsafe {
            this.writer.startSessionAtSourceTime(time(0));
        }
        this.finished = false;
        Ok(this)
    }
    fn error(&self, stage: &str) -> String {
        unsafe { self.writer.error() }.map_or_else(|| stage.into(), |e| failure(stage, &e))
    }
    fn append(&self, input: &AVAssetWriterInput, sample: &CMSampleBuffer) -> Result<bool, String> {
        if unsafe { self.writer.status() } != AVAssetWriterStatus::Writing {
            return Err(self.error("录屏编码器已停止接收样本"));
        }
        let channel = usize::from(!std::ptr::eq(input, &*self.video));
        let mut gates = self.backpressure.lock().map_err(|_| "录屏编码状态不可用")?;
        if !unsafe { input.isReadyForMoreMediaData() } {
            gates[channel].waiting(Instant::now())?;
            return Ok(false);
        }
        if !unsafe { input.appendSampleBuffer(sample) } {
            if unsafe { self.writer.status() } == AVAssetWriterStatus::Writing
                && !unsafe { input.isReadyForMoreMediaData() }
            {
                gates[channel].waiting(Instant::now())?;
                return Ok(false);
            }
            return Err(self.error("无法编码录屏样本"));
        }
        gates[channel].accepted();
        Ok(true)
    }
    fn finish_video(&mut self) {
        if !self.video_finished {
            unsafe {
                self.video.markAsFinished();
            }
            self.video_finished = true;
        }
    }
    fn finish(&mut self, end: u64) -> Result<(), String> {
        self.finish_video();
        unsafe {
            self.writer.endSessionAtSourceTime(time(end));
            if let Some(audio) = &self.audio {
                audio.markAsFinished();
            }
        }
        let flights = Arc::new(Flights::default());
        let lease = Lease::new(&flights);
        let (tx, rx) = mpsc::sync_channel(1);
        let block = RcBlock::new(move || {
            let _ = &lease;
            let _ = tx.try_send(());
        });
        unsafe {
            self.writer.finishWritingWithCompletionHandler(&block);
        }
        drop(block);
        rx.recv().map_err(|_| "录屏编码结束回调已中断")?;
        flights.drain();
        if unsafe { self.writer.status() } != AVAssetWriterStatus::Completed {
            return Err(self.error("录屏文件未完成"));
        }
        self.finished = true;
        Ok(())
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if !self.finished {
            unsafe {
                self.writer.cancelWriting();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AudioClock {
    end: u64,
    segment: Option<u64>,
    bias: u64,
}
#[derive(Clone, Copy)]
struct AudioPlacement {
    start: u64,
    after: AudioClock,
}
impl AudioClock {
    fn place(&self, part: &PacketPart) -> Result<AudioPlacement, String> {
        let invalid = || "电脑声音时间不连续，录制已停止".to_string();
        let frames = part
            .end
            .checked_sub(part.first)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let mut bias = self.bias;
        let mut start = part.active_ticks.checked_add(bias).ok_or_else(invalid)?;
        if let Some(previous) = self.segment {
            if part.segment < previous {
                return Err(invalid());
            }
            if part.segment > previous && start < self.end {
                // Timeline keeps every sample starting before a pause cutoff.
                // Its final sample may extend strictly less than 1/48,000 s
                // beyond that cutoff. Join a new active segment to that sample
                // end, then keep ONE constant bias for the entire new segment.
                // Never clamp each packet: that would hide real overlapping PCM.
                let overlap = self.end - start;
                if u128::from(overlap) * u128::from(RATE) >= u128::from(TICKS_PER_SECOND) {
                    return Err(invalid());
                }
                bias = bias.checked_add(overlap).ok_or_else(invalid)?;
                start = self.end;
            }
        }
        if start < self.end || start - self.end > TICKS_PER_SECOND / 10 {
            return Err(invalid());
        }
        let duration =
            u64::try_from(frames as u128 * u128::from(TICKS_PER_SECOND) / u128::from(RATE))
                .map_err(|_| invalid())?;
        let end = start.checked_add(duration).ok_or_else(invalid)?;
        // Every start is still projected from the source's absolute PTS plus
        // the fixed segment bias, not accumulated from packet durations. The
        // 48 kHz rational duration is rounded only at this packet's endpoint.
        Ok(AudioPlacement {
            start,
            after: Self {
                end,
                segment: Some(part.segment),
                bias,
            },
        })
    }
}

struct PendingAudio {
    source: Sample,
    frames: usize,
    parts: VecDeque<PacketPart>,
    copied: Option<(Retained<CMSampleBuffer>, AudioPlacement)>,
}

// A packet remains charged against the existing two-second queue budget until
// every accepted part is written. One copied part is retained across readiness
// retries; a rejected append never advances PCM position or drops source data.
fn audio(
    queue: &Arc<Mutex<Queue>>,
    clock: &Timeline,
    position: &mut AudioClock,
    pending: &mut Option<PendingAudio>,
    mut append: impl FnMut(&CMSampleBuffer) -> Result<bool, String>,
) -> Result<bool, String> {
    if pending.is_none() {
        let sample = {
            let mut queue = queue.lock().map_err(|_| "电脑声音队列不可用")?;
            queue.audio.pop_front()
        };
        let Some(sample) = sample else {
            return Ok(true);
        };
        let pts = host_ticks(unsafe { sample.0.presentation_time_stamp() })?;
        let count = unsafe { sample.0.num_samples() } as usize;
        *pending = Some(PendingAudio {
            source: sample,
            frames: count,
            parts: clock
                .parts(pts, count, RATE)
                .map_err(|_| "电脑声音时间无效")?
                .into(),
            copied: None,
        });
    }
    let batch = pending.as_mut().unwrap();
    if let Some(part) = batch.parts.front() {
        if batch.copied.is_none() {
            let placement = position.place(part)?;
            batch.copied = Some((
                pcm_part(&batch.source.0, part.first, part.end, placement.start)?,
                placement,
            ));
        }
        let (sample, placement) = batch.copied.as_ref().unwrap();
        if !append(sample)? {
            return Ok(false);
        }
        *position = placement.after;
        batch.parts.pop_front();
        batch.copied = None;
    }
    if batch.parts.is_empty() {
        queue.lock().map_err(|_| "电脑声音队列不可用")?.audio_frames -= batch.frames;
        *pending = None;
    }
    Ok(pending.is_none()
        && queue
            .lock()
            .map_err(|_| "电脑声音队列不可用")?
            .audio
            .is_empty())
}

fn video(
    writer: &Writer,
    last: &Sample,
    frames: &mut u64,
    end: &mut u64,
    target: u64,
    pending: &mut Option<(Retained<CMSampleBuffer>, u64)>,
    mut accepted: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if target.saturating_sub(*frames) > u64::from(FPS) * 2 {
        return Err("屏幕编码积压超出限制".into());
    }
    while *frames < target {
        if pending.is_none() {
            let next = (*frames + 1) * TICKS_PER_SECOND / u64::from(FPS);
            *pending = Some((copied_timing(&last.0, *end, time(next - *end))?, next));
        }
        let (sample, next) = pending.as_ref().unwrap();
        if !writer.append(&writer.video, sample)? {
            return Ok(());
        }
        accepted()?;
        *end = *next;
        *frames += 1;
        *pending = None;
    }
    Ok(())
}

// AudioBufferList has a C flexible tail. Use the framework's AudioBuffer and
// enough aligned storage for stereo's two planes; the advertised count is one
// for interleaved PCM and two for planar PCM.
#[repr(C)]
struct StereoBufferList {
    count: u32,
    buffers: [AudioBuffer; 2],
}
impl StereoBufferList {
    fn pointer(&mut self) -> NonNull<AudioBufferList> {
        const {
            assert!(
                std::mem::align_of::<StereoBufferList>() == std::mem::align_of::<AudioBufferList>()
            );
            assert!(
                std::mem::offset_of!(StereoBufferList, buffers)
                    == std::mem::offset_of!(AudioBufferList, mBuffers)
            );
        }
        NonNull::from(self).cast()
    }
}

fn pcm_part(
    source: &CMSampleBuffer,
    first: usize,
    end: usize,
    active_ticks: u64,
) -> Result<Retained<CMSampleBuffer>, String> {
    let count = usize::try_from(unsafe { source.num_samples() }).map_err(|_| "电脑声音帧数无效")?;
    if first >= end || end > count || count > RATE as usize / 2 {
        return Err("电脑声音帧区间无效".into());
    }
    let format = unsafe { source.format_description() }.ok_or("电脑声音格式缺失")?;
    let asbd = unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() }
        .ok_or("电脑声音 PCM 格式缺失")?;
    let bytes_per_frame = asbd.mBytesPerFrame as usize;
    if asbd.mFormatID != u32::from_be_bytes(*b"lpcm")
        || asbd.mSampleRate != f64::from(RATE)
        || asbd.mChannelsPerFrame != 2
        || bytes_per_frame == 0
        || bytes_per_frame > 16
    {
        return Err("电脑声音 PCM 布局不支持".into());
    }
    let planar = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;
    let frames = end - first;
    let bytes = frames * bytes_per_frame; // bounded to 24k frames × 16 bytes
    let mut left = vec![0u8; bytes];
    let mut right = if planar { vec![0u8; bytes] } else { Vec::new() };
    let mut list = StereoBufferList {
        count: if planar { 2 } else { 1 },
        buffers: [
            AudioBuffer {
                mNumberChannels: if planar { 1 } else { 2 },
                mDataByteSize: bytes as u32,
                mData: left.as_mut_ptr().cast(),
            },
            AudioBuffer {
                mNumberChannels: 1,
                mDataByteSize: if planar { bytes as u32 } else { 0 },
                mData: if planar {
                    right.as_mut_ptr().cast()
                } else {
                    std::ptr::null_mut()
                },
            },
        ],
    };
    // CopySampleBufferForRange explicitly does not support planar audio.
    // This API copies only the requested real PCM frames into matching planes.
    if unsafe {
        source.copy_pcm_data_into_audio_buffer_list(first as i32, frames as i32, list.pointer())
    } != 0
    {
        return Err("电脑声音 PCM 区间拷贝失败".into());
    }
    let timing = CMSampleTimingInfo {
        duration: CMTime {
            value: 1,
            timescale: RATE as i32,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        },
        presentationTimeStamp: time(active_ticks),
        decodeTimeStamp: CMTime {
            value: 0,
            timescale: 0,
            flags: CMTimeFlags(0),
            epoch: 0,
        },
    };
    let mut output = std::ptr::null_mut();
    if unsafe {
        CMSampleBuffer::create(
            None,
            None,
            false,
            None,
            std::ptr::null_mut(),
            Some(&format),
            frames as isize,
            1,
            &timing,
            if planar { 0 } else { 1 },
            if planar {
                std::ptr::null()
            } else {
                &bytes_per_frame
            },
            NonNull::from(&mut output),
        )
    } != 0
    {
        return Err("电脑声音 PCM 样本创建失败".into());
    }
    let output = unsafe { Retained::from_raw(output) }.ok_or("电脑声音 PCM 样本缺失")?;
    // CoreMedia owns a copy before these local buffers are dropped. Preserve
    // the original format description, plane order, and 1/48k frame timing.
    if unsafe { output.set_data_buffer_from_audio_buffer_list(None, None, 0, list.pointer()) } != 0
        || unsafe { output.set_data_ready() } != 0
    {
        return Err("电脑声音 PCM 样本写入失败".into());
    }
    Ok(output)
}
fn queue_check(queue: &Arc<Mutex<Queue>>) -> Result<(), String> {
    let state = queue.lock().map_err(|_| "录屏帧状态不可用")?;
    match &state.error {
        Some(e) => Err(e.clone()),
        None => Ok(()),
    }
}
fn startup(commands: &Receiver<Control>, paused: &mut bool) -> Result<bool, String> {
    loop {
        match commands.try_recv() {
            Ok(Control::Stop | Control::Cancel) => return Ok(false),
            Ok(Control::Pause) => *paused = true,
            Ok(Control::Resume) => *paused = false,
            Err(TryRecvError::Empty) => return Ok(true),
            Err(TryRecvError::Disconnected) => return Err("录制控制连接已断开".into()),
        }
    }
}

pub(super) fn run(
    config: Config,
    commands: Receiver<Control>,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Option<Report>, String> {
    autoreleasepool(|_| {
        if matches!(config.audio, AudioMode::Microphone | AudioMode::Both) {
            return Err("当前 Mac 录屏支持电脑声音或静音，麦克风录制尚未实现".into());
        }
        let mut paused = false;
        if !startup(&commands, &mut paused)? {
            return Ok(None);
        }
        let queue = Arc::new(Mutex::new(Queue::default()));
        let mut capture = Capture::new(&config, queue.clone())?;
        let result = (|| {
            let wait = Instant::now();
            let (first, first_audio) = loop {
                if !startup(&commands, &mut paused)? {
                    return Ok(None);
                }
                queue_check(&queue)?;
                {
                    let q = queue.lock().map_err(|_| "录屏帧状态不可用")?;
                    if q.latest.is_some()
                        && (config.audio == AudioMode::Mute || !q.audio.is_empty())
                    {
                        break (
                            q.latest.as_ref().unwrap().duplicate(),
                            q.audio.front().map(Sample::duplicate),
                        );
                    }
                }
                if wait.elapsed() >= Duration::from_secs(5) {
                    return Err("等待屏幕画面或电脑声音超时".into());
                }
                std::thread::sleep(POLL);
            };
            if !startup(&commands, &mut paused)? {
                return Ok(None);
            }
            let mut destination = Output::new(&config.output)?;
            let mut writer = Writer::new(
                &destination.path,
                &config,
                first_audio.as_ref().map(|s| &*s.0),
            )?;
            let epoch = now()?;
            let mut clock = Timeline::new(epoch, paused);
            let mut frames = 0u64;
            let mut end = 0u64;
            let mut audio_end = AudioClock::default();
            let mut next_progress = Instant::now();
            let mut last = first;
            let mut pending_video = None;
            let mut pending_audio = None;
            loop {
                destination.check()?;
                queue_check(&queue)?;
                capture.check_display()?;
                let stamp = now()?;
                let mut stopped = false;
                loop {
                    match commands.try_recv() {
                        Ok(Control::Cancel) => return Err("录制已取消".into()),
                        Ok(Control::Stop) => {
                            clock
                                .set_paused(true, stamp)
                                .map_err(|_| "录制结束时钟无效")?;
                            stopped = true;
                            break;
                        }
                        Ok(Control::Pause) => clock
                            .set_paused(true, stamp)
                            .map_err(|_| "录制暂停时钟无效")?,
                        Ok(Control::Resume) => clock
                            .set_paused(false, stamp)
                            .map_err(|_| "录制恢复时钟无效")?,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return Err("录制控制连接已断开".into()),
                    }
                }
                let elapsed = clock.elapsed(stamp).map_err(|_| "录制时钟无效")?;
                if stopped {
                    break;
                }
                if elapsed >= MAX_ACTIVE.as_secs() * TICKS_PER_SECOND {
                    clock
                        .set_paused(true, stamp)
                        .map_err(|_| "录制结束时钟无效")?;
                    break;
                }
                if let Some(frame) = queue
                    .lock()
                    .map_err(|_| "录屏帧状态不可用")?
                    .latest
                    .as_ref()
                {
                    last = frame.duplicate();
                }
                if !clock.paused() || pending_video.is_some() {
                    let target = if clock.paused() {
                        frames + 1
                    } else {
                        elapsed * u64::from(FPS) / TICKS_PER_SECOND + 1
                    };
                    video(
                        &writer,
                        &last,
                        &mut frames,
                        &mut end,
                        target,
                        &mut pending_video,
                        || destination.hold(),
                    )?;
                }
                if writer.audio.is_some() {
                    audio(
                        &queue,
                        &clock,
                        &mut audio_end,
                        &mut pending_audio,
                        |sample| {
                            let accepted = writer.append(writer.audio.as_ref().unwrap(), sample)?;
                            if accepted {
                                destination.hold()?;
                            }
                            Ok(accepted)
                        },
                    )?;
                }
                clock.prune_before(stamp.saturating_sub(TICKS_PER_SECOND * 3));
                if Instant::now() >= next_progress {
                    progress(Progress {
                        elapsed_ms: elapsed / 10_000,
                        paused: clock.paused(),
                    });
                    next_progress = Instant::now() + Duration::from_millis(200);
                }
                match commands.recv_timeout(POLL) {
                    Ok(Control::Cancel) => return Err("录制已取消".into()),
                    Ok(Control::Stop) => {
                        clock
                            .set_paused(true, now()?)
                            .map_err(|_| "录制结束时钟无效")?;
                        break;
                    }
                    Ok(Control::Pause) => clock
                        .set_paused(true, now()?)
                        .map_err(|_| "录制暂停时钟无效")?,
                    Ok(Control::Resume) => clock
                        .set_paused(false, now()?)
                        .map_err(|_| "录制恢复时钟无效")?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("录制控制连接已断开".into())
                    }
                }
            }
            capture.stop()?;
            let target = clock.elapsed(now()?).map_err(|_| "录制结束时钟无效")? * u64::from(FPS)
                / TICKS_PER_SECOND
                + 1;
            let drain = Instant::now();
            loop {
                destination.check()?;
                queue_check(&queue)?;
                match commands.try_recv() {
                    Ok(Control::Cancel) => return Err("录制已取消".into()),
                    Err(TryRecvError::Disconnected) => return Err("录制控制连接已断开".into()),
                    _ => {}
                }
                video(
                    &writer,
                    &last,
                    &mut frames,
                    &mut end,
                    target,
                    &mut pending_video,
                    || destination.hold(),
                )?;
                if frames == target {
                    writer.finish_video();
                }
                let audio_done = if let Some(input) = &writer.audio {
                    audio(
                        &queue,
                        &clock,
                        &mut audio_end,
                        &mut pending_audio,
                        |sample| {
                            let accepted = writer.append(input, sample)?;
                            if accepted {
                                destination.hold()?;
                            }
                            Ok(accepted)
                        },
                    )?
                } else {
                    true
                };
                if frames == target && audio_done {
                    break;
                }
                if drain.elapsed() >= MAX_BACKPRESSURE {
                    return Err("录屏编码结束积压超过两秒".into());
                }
                std::thread::sleep(POLL);
            }
            if frames == 0 {
                return Err("录制尚未形成可保存的画面".into());
            }
            if writer.audio.is_some() {
                if end.abs_diff(audio_end.end) > TICKS_PER_SECOND / 10 {
                    return Err("电脑声音未覆盖录屏时间，文件未发布".into());
                }
            }
            writer.finish(end)?;
            drop(writer);
            destination.check()?;
            let checked = crate::video_clip_backend::execute(
                &crate::video_clip_contract::ClipJob::Inspect {
                    source: destination.path.clone(),
                },
                &std::sync::atomic::AtomicBool::new(false),
                &|_| {},
            )
            .map_err(|e| e.to_string())?;
            let crate::video_clip_contract::ClipJobResult::Inspected(metadata) = checked else {
                return Err("录屏文件核验未返回视频信息".into());
            };
            if metadata.width != config.width
                || metadata.height != config.height
                || metadata.has_audio != (config.audio == AudioMode::System)
                || metadata.duration_ticks.abs_diff(end) > TICKS_PER_SECOND / 10
            {
                return Err("录屏文件尺寸、声音或时长核验失败".into());
            }
            destination.finish()?;
            Ok(Some(Report {
                width: config.width,
                height: config.height,
                duration_ms: end / 10_000,
                frames,
                audio: config.audio,
                stop_reason: None,
            }))
        })();
        let stop = capture.stop();
        match result {
            Ok(value) => {
                stop?;
                Ok(value)
            }
            Err(error) => {
                let _ = stop;
                Err(error)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_audio_types::{
        kAudioFormatFlagIsFloat, kAudioFormatFlagIsPacked, AudioStreamBasicDescription,
    };
    use objc2_core_foundation::CFRange;
    use objc2_core_media::CMAudioFormatDescriptionCreate;

    #[test]
    fn backpressure_keeps_a_bounded_deadline_until_a_sample_is_accepted() {
        let mut gate = Backpressure::default();
        let start = Instant::now();
        gate.waiting(start).unwrap();
        gate.waiting(start + Duration::from_millis(1900)).unwrap();
        assert!(gate.waiting(start + MAX_BACKPRESSURE).is_err());
        gate.accepted();
        gate.waiting(start + Duration::from_secs(10)).unwrap();
        gate.waiting(start + Duration::from_millis(11999)).unwrap();
        assert!(gate.waiting(start + Duration::from_secs(12)).is_err());
    }

    #[test]
    fn backpressure_preserves_real_pcm_pause_parts_and_queue_budget_until_acceptance() {
        autoreleasepool(|_| {
            let epoch = TICKS_PER_SECOND * 5;
            let (source, expected) = stereo_fixture(true, epoch - 10_000);
            let mut clock = Timeline::new(epoch, false);
            clock.set_paused(true, epoch + 10_000).unwrap();
            clock.set_paused(false, epoch + 40_000).unwrap();
            clock.set_paused(true, epoch + 60_000).unwrap();
            let queue = Arc::new(Mutex::new(Queue::default()));
            {
                let mut q = queue.lock().unwrap();
                q.audio.push_back(Sample(source.clone()));
                q.audio_frames = 2048;
            }
            let mut pending = None;
            let mut end = AudioClock::default();
            let mut address = None;
            for _ in 0..3 {
                assert!(!audio(&queue, &clock, &mut end, &mut pending, |part| {
                    assert_eq!(read_pcm(part, true), expected[48..96]);
                    let pointer = part as *const CMSampleBuffer;
                    if let Some(previous) = address {
                        assert_eq!(pointer, previous);
                    } else {
                        address = Some(pointer);
                    }
                    Ok(false)
                })
                .unwrap());
                assert_eq!(end, AudioClock::default());
                assert_eq!(queue.lock().unwrap().audio_frames, 2048);
            }
            assert!(!audio(&queue, &clock, &mut end, &mut pending, |part| {
                assert_eq!(read_pcm(part, true), expected[48..96]);
                Ok(true)
            })
            .unwrap());
            assert_eq!(end.end, 10_000);
            assert_eq!(queue.lock().unwrap().audio_frames, 2048);
            assert!(audio(&queue, &clock, &mut end, &mut pending, |part| {
                assert_eq!(read_pcm(part, true), expected[240..336]);
                Ok(true)
            })
            .unwrap());
            assert_eq!(end.end, 30_000);
            assert_eq!(queue.lock().unwrap().audio_frames, 0);
            assert!(pending.is_none());
            assert_eq!(read_pcm(&source, true), expected);
        });
    }

    fn off_grid_pause_audio(planar: bool) {
        autoreleasepool(|_| {
            let (source, original) = stereo_fixture(planar, 0);
            let queue = Arc::new(Mutex::new(Queue::default()));
            for (stamp, count) in [(0, 5), (10_000, 480), (110_000, 480)] {
                let sample = pcm_part(&source, 0, count, stamp).unwrap();
                let mut q = queue.lock().unwrap();
                q.audio.push_back(Sample(sample));
                q.audio_frames += count;
            }
            // These three source ranges never overlap. The five real samples
            // before Pause end at 5/48000 s, just beyond the 1000-tick cutoff.
            let mut clock = Timeline::new(0, false);
            clock.set_paused(true, 1000).unwrap();
            clock.set_paused(false, 10_000).unwrap();
            let mut position = AudioClock::default();
            let mut pending = None;
            let mut accepted = Vec::new();
            assert!(!audio(&queue, &clock, &mut position, &mut pending, |part| {
                assert_eq!(read_pcm(part, planar), original[..5]);
                assert_eq!(host_ticks(unsafe { part.presentation_time_stamp() })?, 0);
                accepted.extend(read_pcm(part, planar));
                Ok(true)
            })
            .unwrap());
            assert_eq!(position.end, 1041);
            // The old guard rejected this legitimate new segment: 1000<1041.
            assert_eq!(
                clock.parts(10_000, 480, RATE).unwrap()[0].active_ticks,
                1000
            );
            let before = position;
            let mut address = None;
            for _ in 0..3 {
                assert!(!audio(&queue, &clock, &mut position, &mut pending, |part| {
                    assert_eq!(read_pcm(part, planar), original[..480]);
                    assert_eq!(host_ticks(unsafe { part.presentation_time_stamp() })?, 1041);
                    let pointer = part as *const CMSampleBuffer;
                    if let Some(previous) = address {
                        assert_eq!(pointer, previous);
                    } else {
                        address = Some(pointer);
                    }
                    Ok(false)
                })
                .unwrap());
                assert_eq!(
                    position, before,
                    "rejected append must not commit timing or bias"
                );
                assert_eq!(queue.lock().unwrap().audio_frames, 960);
            }
            assert!(!audio(&queue, &clock, &mut position, &mut pending, |part| {
                assert_eq!(host_ticks(unsafe { part.presentation_time_stamp() })?, 1041);
                accepted.extend(read_pcm(part, planar));
                Ok(true)
            })
            .unwrap());
            assert_eq!((position.end, position.bias), (101_041, 41));
            assert!(audio(&queue, &clock, &mut position, &mut pending, |part| {
                // The next packet must keep that same 41-tick segment mapping.
                assert_eq!(
                    host_ticks(unsafe { part.presentation_time_stamp() })?,
                    101_041
                );
                assert_eq!(read_pcm(part, planar), original[..480]);
                accepted.extend(read_pcm(part, planar));
                Ok(true)
            })
            .unwrap());
            assert_eq!((position.end, position.bias), (201_041, 41));
            assert_eq!(
                accepted,
                [&original[..5], &original[..480], &original[..480]].concat()
            );
            assert_eq!(
                accepted.len(),
                965,
                "every selected real PCM frame is retained exactly once"
            );
            assert_eq!(queue.lock().unwrap().audio_frames, 0);
            assert!(pending.is_none());
            assert_eq!(read_pcm(&source, planar), original);
            assert_eq!(
                host_ticks(unsafe { source.presentation_time_stamp() }).unwrap(),
                0
            );
        });
    }
    #[test]
    fn off_grid_pause_resume_preserves_planar_pcm_and_constant_segment_timing() {
        off_grid_pause_audio(true);
    }
    #[test]
    fn off_grid_pause_resume_preserves_interleaved_pcm_and_constant_segment_timing() {
        off_grid_pause_audio(false);
    }
    #[test]
    fn audio_clock_rejects_real_overlap_large_gap_and_invalid_segment_without_mutation() {
        let position = AudioClock {
            end: 100_000,
            segment: Some(10_000),
            bias: 41,
        };
        let part = |active_ticks, segment| PacketPart {
            first: 0,
            end: 480,
            active_ticks,
            segment,
        };
        // Same-segment overlap, even one 100ns tick, is never clamped.
        assert!(position.place(&part(99_958, 10_000)).is_err());
        // A real gap beyond the original 100ms allowance is still rejected.
        assert!(position.place(&part(1_099_960, 10_000)).is_err());
        assert!(position.place(&part(99_959, 9999)).is_err());
        // 208 ticks is strictly under one 48kHz frame; 209 is not.
        assert!(position.place(&part(99_751, 20_000)).is_ok());
        assert!(position.place(&part(99_750, 20_000)).is_err());
        assert_eq!(
            position,
            AudioClock {
                end: 100_000,
                segment: Some(10_000),
                bias: 41
            }
        );
    }
    #[test]
    fn fixed_segment_mapping_uses_absolute_pts_without_per_packet_rounding_drift() {
        let first = PacketPart {
            first: 0,
            end: 5,
            active_ticks: 0,
            segment: 0,
        };
        let mut position = AudioClock::default().place(&first).unwrap().after;
        let packets = 20_000u64;
        for index in 0..packets {
            let absolute = 1000 + index * 1024 * TICKS_PER_SECOND / u64::from(RATE);
            let part = PacketPart {
                first: 0,
                end: 1024,
                active_ticks: absolute,
                segment: 10_000,
            };
            position = position.place(&part).unwrap().after;
            assert_eq!(position.bias, 41);
        }
        let rational_end = 1041 + packets * 1024 * TICKS_PER_SECOND / u64::from(RATE);
        assert!(
            position.end.abs_diff(rational_end) <= 1,
            "packet floors must not accumulate over seven minutes"
        );
    }

    fn owned_video_sample() -> Sample {
        use objc2_core_video::*;
        struct OwnedCf(core_foundation::base::CFTypeRef);
        impl Drop for OwnedCf {
            fn drop(&mut self) {
                unsafe {
                    core_foundation::base::CFRelease(self.0);
                }
            }
        }
        let mut pixel = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                CVPixelBufferCreate(
                    None,
                    64,
                    48,
                    kCVPixelFormatType_32BGRA,
                    None,
                    NonNull::from(&mut pixel),
                )
            },
            0
        );
        let _pixel = OwnedCf(pixel.cast());
        let pixel = unsafe { &*pixel };
        assert_eq!(
            unsafe { CVPixelBufferLockBaseAddress(pixel, CVPixelBufferLockFlags(0)) },
            0
        );
        let stride = CVPixelBufferGetBytesPerRow(pixel);
        let base = CVPixelBufferGetBaseAddress(pixel).cast::<u8>();
        assert!(!base.is_null());
        for row in 0..48 {
            for col in 0..64 {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        [64, 128, 200, 255].as_ptr(),
                        base.add(row * stride + col * 4),
                        4,
                    );
                }
            }
        }
        assert_eq!(
            unsafe { CVPixelBufferUnlockBaseAddress(pixel, CVPixelBufferLockFlags(0)) },
            0
        );
        let mut format = std::ptr::null();
        assert_eq!(
            unsafe {
                objc2_core_media::CMVideoFormatDescriptionCreateForImageBuffer(
                    None,
                    pixel,
                    NonNull::from(&mut format),
                )
            },
            0
        );
        let _format = OwnedCf(format.cast());
        let mut timing = CMSampleTimingInfo {
            duration: time(TICKS_PER_SECOND / u64::from(FPS)),
            presentationTimeStamp: time(0),
            decodeTimeStamp: time(0),
        };
        let mut sample = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                CMSampleBuffer::create_ready_with_image_buffer(
                    None,
                    pixel,
                    &*format,
                    NonNull::from(&mut timing),
                    NonNull::from(&mut sample),
                )
            },
            0
        );
        Sample(unsafe { Retained::from_raw(sample) }.unwrap())
    }

    #[test]
    fn backpressure_native_writer_keeps_real_audio_and_video_under_short_cpu_pressure() {
        autoreleasepool(|_| {
            let root =
                std::env::temp_dir().join(format!("mewu-writer-pressure-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let path = root.join("owned.mp4");
            let config = Config {
                output: path.clone(),
                origin_x: 0,
                origin_y: 0,
                monitor_width: 64,
                monitor_height: 48,
                x: 0,
                y: 0,
                width: 64,
                height: 48,
                audio: AudioMode::System,
                capture_coordinates: mewu_core::CaptureCoordinates {
                    space: mewu_core::CaptureCoordinateSpace::QuartzPoints,
                    origin_x: 0.,
                    origin_y: 0.,
                    width: 64.,
                    height: 48.,
                },
            };
            let (source, _) = stereo_fixture(false, 0);
            let queue = Arc::new(Mutex::new(Queue::default()));
            // Two seconds of real PCM, separated by a two-second host pause.
            // Pause removal occurs through Timeline, not invented output PCM.
            for epoch in [0, 3 * TICKS_PER_SECOND] {
                let mut frame = 0usize;
                while frame < RATE as usize {
                    let count = 2048.min(RATE as usize - frame);
                    let sample = pcm_part(
                        &source,
                        0,
                        count,
                        epoch + frame as u64 * TICKS_PER_SECOND / u64::from(RATE),
                    )
                    .unwrap();
                    let mut q = queue.lock().unwrap();
                    q.audio.push_back(Sample(sample));
                    q.audio_frames += count;
                    frame += count;
                }
            }
            assert_eq!(queue.lock().unwrap().audio_frames, MAX_AUDIO_FRAMES);
            let mut clock = Timeline::new(0, false);
            clock.set_paused(true, TICKS_PER_SECOND).unwrap();
            clock.set_paused(false, 3 * TICKS_PER_SECOND).unwrap();
            clock.set_paused(true, 4 * TICKS_PER_SECOND).unwrap();
            let mut writer = Writer::new(&path, &config, Some(&source)).unwrap();
            let last = owned_video_sample();
            let mut pending_video = None;
            let mut pending_audio = None;
            let mut frames = 0;
            let mut end = 0;
            let mut audio_end = AudioClock::default();
            let pressure = std::thread::spawn(|| {
                let until = Instant::now() + Duration::from_millis(400);
                let mut sum = 1u64;
                while Instant::now() < until {
                    sum = sum.wrapping_mul(6364136223846793005).wrapping_add(1);
                    std::hint::black_box(sum);
                }
            });
            let started = Instant::now();
            let mut retries = 0;
            let mut encoded_pcm_frames = 0usize;
            loop {
                assert!(started.elapsed() < Duration::from_secs(5));
                let before = frames;
                let target = (frames + 1).min(30);
                video(
                    &writer,
                    &last,
                    &mut frames,
                    &mut end,
                    target,
                    &mut pending_video,
                    || Ok(()),
                )
                .unwrap();
                if frames == before && frames < 30 {
                    retries += 1;
                }
                if frames == 30 {
                    writer.finish_video();
                }
                let audio_done = audio(
                    &queue,
                    &clock,
                    &mut audio_end,
                    &mut pending_audio,
                    |sample| {
                        let accepted = writer.append(writer.audio.as_ref().unwrap(), sample)?;
                        if accepted {
                            encoded_pcm_frames += unsafe { sample.num_samples() } as usize;
                        }
                        Ok(accepted)
                    },
                )
                .unwrap();
                if frames == 30 && audio_done {
                    break;
                }
                std::thread::sleep(POLL);
            }
            pressure.join().unwrap();
            let last_start = RATE as usize / 2048 * 2048;
            let expected_audio_end = TICKS_PER_SECOND
                + last_start as u64 * TICKS_PER_SECOND / u64::from(RATE)
                + (RATE as usize - last_start) as u64 * TICKS_PER_SECOND / u64::from(RATE);
            assert_eq!(encoded_pcm_frames, MAX_AUDIO_FRAMES);
            assert_eq!(audio_end.end, expected_audio_end);
            assert_eq!(end, 2 * TICKS_PER_SECOND);
            writer.finish(end).unwrap();
            drop(writer);
            let result = crate::video_clip_backend::execute(
                &crate::video_clip_contract::ClipJob::Inspect {
                    source: path.clone(),
                },
                &std::sync::atomic::AtomicBool::new(false),
                &|_| {},
            )
            .unwrap();
            let crate::video_clip_contract::ClipJobResult::Inspected(metadata) = result else {
                panic!("Missing real MP4 metadata");
            };
            assert_eq!((metadata.width, metadata.height), (64, 48));
            assert_eq!(
                (
                    metadata.frame_rate_numerator,
                    metadata.frame_rate_denominator
                ),
                (15, 1)
            );
            assert!(metadata.has_audio);
            assert!(metadata.duration_ticks.abs_diff(end) <= TICKS_PER_SECOND / 10);
            println!("actual_writer_frames={frames} readiness_retries={retries} pcm_frames={} duration_ticks={}", encoded_pcm_frames, metadata.duration_ticks);
            std::fs::remove_file(path).unwrap();
            std::fs::remove_dir(root).unwrap();
        });
    }

    fn read_pcm(source: &CMSampleBuffer, planar: bool) -> Vec<[u32; 2]> {
        let frames = unsafe { source.num_samples() } as usize;
        let mut first = vec![0f32; frames * if planar { 1 } else { 2 }];
        let mut second = if planar {
            vec![0f32; frames]
        } else {
            Vec::new()
        };
        let mut list = StereoBufferList {
            count: if planar { 2 } else { 1 },
            buffers: [
                AudioBuffer {
                    mNumberChannels: if planar { 1 } else { 2 },
                    mDataByteSize: (first.len() * 4) as u32,
                    mData: first.as_mut_ptr().cast(),
                },
                AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: (second.len() * 4) as u32,
                    mData: if planar {
                        second.as_mut_ptr().cast()
                    } else {
                        std::ptr::null_mut()
                    },
                },
            ],
        };
        assert_eq!(
            unsafe {
                source.copy_pcm_data_into_audio_buffer_list(0, frames as i32, list.pointer())
            },
            0
        );
        (0..frames)
            .map(|n| {
                if planar {
                    [first[n].to_bits(), second[n].to_bits()]
                } else {
                    [first[n * 2].to_bits(), first[n * 2 + 1].to_bits()]
                }
            })
            .collect()
    }

    fn stereo_fixture(planar: bool, pts: u64) -> (Retained<CMSampleBuffer>, Vec<[u32; 2]>) {
        let frames = 2048usize;
        let tones: Vec<[f32; 2]> = (0..frames)
            .map(|n| {
                [440.0, 880.0].map(|hz| {
                    (0.08 * (std::f64::consts::TAU * hz * n as f64 / f64::from(RATE)).sin()) as f32
                })
            })
            .collect();
        let expected = tones.iter().map(|pair| pair.map(f32::to_bits)).collect();
        let mut first: Vec<f32> = if planar {
            tones.iter().map(|p| p[0]).collect()
        } else {
            tones.iter().flat_map(|p| *p).collect()
        };
        let mut second: Vec<f32> = if planar {
            tones.iter().map(|p| p[1]).collect()
        } else {
            Vec::new()
        };
        let mut list = StereoBufferList {
            count: if planar { 2 } else { 1 },
            buffers: [
                AudioBuffer {
                    mNumberChannels: if planar { 1 } else { 2 },
                    mDataByteSize: (first.len() * 4) as u32,
                    mData: first.as_mut_ptr().cast(),
                },
                AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: (second.len() * 4) as u32,
                    mData: if planar {
                        second.as_mut_ptr().cast()
                    } else {
                        std::ptr::null_mut()
                    },
                },
            ],
        };
        let stride = if planar { 4 } else { 8 };
        let asbd = AudioStreamBasicDescription {
            mSampleRate: f64::from(RATE),
            mFormatID: u32::from_be_bytes(*b"lpcm"),
            mFormatFlags: kAudioFormatFlagIsFloat
                | kAudioFormatFlagIsPacked
                | if planar {
                    kAudioFormatFlagIsNonInterleaved
                } else {
                    0
                },
            mBytesPerPacket: stride,
            mFramesPerPacket: 1,
            mBytesPerFrame: stride,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        let mut format = std::ptr::null();
        assert_eq!(
            unsafe {
                CMAudioFormatDescriptionCreate(
                    None,
                    NonNull::from(&asbd),
                    0,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    None,
                    NonNull::from(&mut format),
                )
            },
            0
        );
        let format = unsafe { Retained::from_raw(format.cast_mut()) }.unwrap();
        let timing = CMSampleTimingInfo {
            duration: CMTime {
                value: 1,
                timescale: RATE as i32,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            },
            presentationTimeStamp: time(pts),
            decodeTimeStamp: CMTime {
                value: 0,
                timescale: 0,
                flags: CMTimeFlags(0),
                epoch: 0,
            },
        };
        let mut source = std::ptr::null_mut();
        let sample_bytes = stride as usize;
        assert_eq!(
            unsafe {
                CMSampleBuffer::create(
                    None,
                    None,
                    false,
                    None,
                    std::ptr::null_mut(),
                    Some(&format),
                    frames as isize,
                    1,
                    &timing,
                    if planar { 0 } else { 1 },
                    if planar {
                        std::ptr::null()
                    } else {
                        &sample_bytes
                    },
                    NonNull::from(&mut source),
                )
            },
            0
        );
        let source = unsafe { Retained::from_raw(source) }.unwrap();
        assert_eq!(
            unsafe { source.set_data_buffer_from_audio_buffer_list(None, None, 0, list.pointer()) },
            0
        );
        assert_eq!(unsafe { source.set_data_ready() }, 0);
        (source, expected)
    }

    fn check_pcm_ranges(planar: bool) {
        autoreleasepool(|_| {
            let epoch = TICKS_PER_SECOND * 5;
            let (source, expected) = stereo_fixture(planar, epoch - 10_000);
            assert_eq!(read_pcm(&source, planar), expected);
            let original_format = unsafe { source.format_description() }.unwrap();
            let original_asbd = *unsafe {
                CMAudioFormatDescriptionGetStreamBasicDescription(&original_format).as_ref()
            }
            .unwrap();
            let mut legacy = std::ptr::null_mut();
            let legacy_status = unsafe {
                CMSampleBuffer::copy_sample_buffer_for_range(
                    None,
                    &source,
                    CFRange {
                        location: 1,
                        length: 7,
                    },
                    NonNull::from(&mut legacy),
                )
            };
            drop(unsafe { Retained::from_raw(legacy) });
            if planar {
                assert_ne!(
                    legacy_status, 0,
                    "Official legacy API rejects planar subranges"
                );
            } else {
                assert_eq!(legacy_status, 0);
            }
            for (first, end) in [(0, 2048), (1, 37), (2039, 2048)] {
                let part = pcm_part(&source, first, end, 1_234_567).unwrap();
                assert_eq!(read_pcm(&part, planar), expected[first..end]);
                assert_eq!(unsafe { part.num_samples() } as usize, end - first);
                assert!(unsafe { part.data_is_ready() });
                assert_eq!(
                    host_ticks(unsafe { part.presentation_time_stamp() }).unwrap(),
                    1_234_567
                );
                assert_eq!(
                    host_ticks(unsafe { part.duration() }).unwrap(),
                    (end - first) as u64 * TICKS_PER_SECOND / u64::from(RATE)
                );
                let format = unsafe { part.format_description() }.unwrap();
                let asbd =
                    unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() }
                        .unwrap();
                assert_eq!(*asbd, original_asbd);
            }
            assert!(pcm_part(&source, 1, 1, 0).is_err());
            assert!(pcm_part(&source, 0, 2049, 0).is_err());
            // 1 ms before the start and the 3 ms pause are excluded; the stop
            // boundary truncates the real packet at 6 ms, with no made-up PCM.
            let mut clock = Timeline::new(epoch, false);
            clock.set_paused(true, epoch + 10_000).unwrap();
            clock.set_paused(false, epoch + 40_000).unwrap();
            clock.set_paused(true, epoch + 60_000).unwrap();
            let parts = clock.parts(epoch - 10_000, 2048, RATE).unwrap();
            assert_eq!(
                parts
                    .iter()
                    .map(|p| (p.first, p.end, p.active_ticks))
                    .collect::<Vec<_>>(),
                vec![(48, 96, 0), (240, 336, 10_000)]
            );
            for part in parts {
                let copied = pcm_part(&source, part.first, part.end, part.active_ticks).unwrap();
                assert_eq!(read_pcm(&copied, planar), expected[part.first..part.end]);
                assert_eq!(
                    host_ticks(unsafe { copied.presentation_time_stamp() }).unwrap(),
                    part.active_ticks
                );
            }
            assert_eq!(read_pcm(&source, planar), expected);
            assert_eq!(
                host_ticks(unsafe { source.presentation_time_stamp() }).unwrap(),
                epoch - 10_000
            );
        });
    }
    #[test]
    fn planar_pcm_subranges_preserve_tones_timing_pause_boundaries_and_source() {
        check_pcm_ranges(true);
    }
    #[test]
    fn interleaved_pcm_subranges_preserve_tones_timing_pause_boundaries_and_source() {
        check_pcm_ranges(false);
    }
    #[test]
    fn output_refuses_existing_files_and_never_deletes_replacements() {
        let root =
            std::env::temp_dir().join(format!("mewu-recording-output-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("actual.mp4");
        std::fs::write(&path, b"existing").unwrap();
        assert!(Output::new(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        std::fs::remove_file(&path).unwrap();
        let mut output = Output::new(&path).unwrap();
        std::fs::write(&path, b"original").unwrap();
        output.hold().unwrap();
        std::fs::rename(&path, root.join("retained.mp4")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert!(output.check().is_err());
        drop(output);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(root.join("retained.mp4")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn actual_partial_is_removed_and_finalized_file_stays_at_parent_observed_path() {
        let root =
            std::env::temp_dir().join(format!("mewu-recording-output-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("actual.mp4");
        let mut output = Output::new(&path).unwrap();
        std::fs::write(&path, b"partial").unwrap();
        output.hold().unwrap();
        drop(output);
        assert!(!path.exists());
        let mut output = Output::new(&path).unwrap();
        std::fs::write(&path, b"finalized").unwrap();
        output.hold().unwrap();
        output.finish().unwrap();
        drop(output);
        assert_eq!(std::fs::read(&path).unwrap(), b"finalized");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
