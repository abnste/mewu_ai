// SPDX-License-Identifier: MPL-2.0
//! ScreenCaptureKit and AVAssetWriter live solely in the owned recording child.
//! The parent still owns startup/finalize deadlines and actual process/IO drain.
//! https://developer.apple.com/documentation/screencapturekit/scstream
//! https://developer.apple.com/documentation/avfoundation/avassetwriter
use super::{AudioMode, Config, Control, Progress, Report, FPS, MAX_ACTIVE};
use crate::recording_audio_timeline::{Timeline, TICKS_PER_SECOND};
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
use objc2_core_foundation::{CFRange, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGDisplayBounds;
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
            let result = catch_unwind(AssertUnwindSafe(|| self.accept(sample, kind)));
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
            queue.audio.push_back(Sample(
                unsafe { Retained::retain(sample as *const _ as *mut _) }
                    .ok_or("无法持有电脑声音")?,
            ));
        }
        Ok(())
    }
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
    handler: Retained<Handler>,
    dispatch: DispatchRetained<DispatchQueue>,
    audio: bool,
    active: bool,
    display: u32,
    coordinates: mewu_core::CaptureCoordinates,
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
            sc.setCapturesAudio(config.audio == AudioMode::System);
            sc.setSampleRate(RATE as isize);
            sc.setChannelCount(2);
            // The user asked for computer sound, including this app's playback.
            sc.setExcludesCurrentProcessAudio(false);
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
        if config.audio == AudioMode::System {
            unsafe {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        ProtocolObject::from_ref(&*handler),
                        SCStreamOutputType::Audio,
                        Some(&dispatch),
                    )
                    .map_err(|e| failure("无法接入电脑声音", &e))?;
            }
        }
        let mut capture = Self {
            stream,
            handler,
            dispatch,
            audio: config.audio == AudioMode::System,
            active: true,
            display: unsafe { display.displayID() },
            coordinates: *wanted,
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
        let audio = if self.audio {
            unsafe {
                self.stream.removeStreamOutput_type_error(
                    ProtocolObject::from_ref(&*self.handler),
                    SCStreamOutputType::Audio,
                )
            }
            .map_err(|e| failure("无法移除声音回调", &e))
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
    fn append(&self, input: &AVAssetWriterInput, sample: &CMSampleBuffer) -> Result<(), String> {
        if !unsafe { input.isReadyForMoreMediaData() } {
            return Err(self.error("录屏编码队列无法及时接收样本"));
        }
        if !unsafe { input.appendSampleBuffer(sample) } {
            return Err(self.error("无法编码录屏样本"));
        }
        Ok(())
    }
    fn video(&self, sample: &CMSampleBuffer, start: u64, end: u64) -> Result<(), String> {
        let copy = copied_timing(sample, start, time(end - start))?;
        self.append(&self.video, &copy)
    }
    fn finish(&mut self, end: u64) -> Result<(), String> {
        unsafe {
            self.writer.endSessionAtSourceTime(time(end));
            self.video.markAsFinished();
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

fn audio(
    queue: &Arc<Mutex<Queue>>,
    writer: &Writer,
    clock: &Timeline,
    last_end: &mut u64,
) -> Result<(), String> {
    loop {
        let sample = {
            let mut queue = queue.lock().map_err(|_| "电脑声音队列不可用")?;
            let sample = queue.audio.pop_front();
            if let Some(sample) = &sample {
                queue.audio_frames -= unsafe { sample.0.num_samples() } as usize;
            }
            sample
        };
        let Some(sample) = sample else {
            return Ok(());
        };
        let pts = host_ticks(unsafe { sample.0.presentation_time_stamp() })?;
        let count = unsafe { sample.0.num_samples() } as usize;
        for part in clock
            .parts(pts, count, RATE)
            .map_err(|_| "电脑声音时间无效")?
        {
            if part.active_ticks < *last_end
                || part.active_ticks.saturating_sub(*last_end) > TICKS_PER_SECOND / 10
            {
                return Err("电脑声音时间不连续，录制已停止".into());
            }
            let mut sub = std::ptr::null_mut();
            if unsafe {
                CMSampleBuffer::copy_sample_buffer_for_range(
                    None,
                    &sample.0,
                    CFRange {
                        location: part.first as isize,
                        length: (part.end - part.first) as isize,
                    },
                    NonNull::from(&mut sub),
                )
            } != 0
            {
                return Err("无法裁切电脑声音样本".into());
            }
            let sub = unsafe { Retained::from_raw(sub) }.ok_or("电脑声音裁切样本缺失")?;
            let sample = copied_timing(
                &sub,
                part.active_ticks,
                CMTime {
                    value: 1,
                    timescale: RATE as i32,
                    flags: CMTimeFlags::Valid,
                    epoch: 0,
                },
            )?;
            writer.append(writer.audio.as_ref().ok_or("电脑声音编码器缺失")?, &sample)?;
            *last_end = part.active_ticks
                + (part.end - part.first) as u64 * TICKS_PER_SECOND / u64::from(RATE);
        }
    }
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
            let mut audio_end = 0u64;
            let mut next_progress = Instant::now();
            let mut last = first;
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
                if !clock.paused() {
                    let target = elapsed * u64::from(FPS) / TICKS_PER_SECOND + 1;
                    if target.saturating_sub(frames) > u64::from(FPS) * 2 {
                        return Err("屏幕编码积压超出限制".into());
                    }
                    while frames < target {
                        let next = (frames + 1) * TICKS_PER_SECOND / u64::from(FPS);
                        writer.video(&last.0, end, next)?;
                        destination.hold()?;
                        end = next;
                        frames += 1;
                    }
                }
                if writer.audio.is_some() {
                    audio(&queue, &writer, &clock, &mut audio_end)?;
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
            if frames == 0 {
                return Err("录制尚未形成可保存的画面".into());
            }
            if writer.audio.is_some() {
                audio(&queue, &writer, &clock, &mut audio_end)?;
                if end.abs_diff(audio_end) > TICKS_PER_SECOND / 10 {
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
