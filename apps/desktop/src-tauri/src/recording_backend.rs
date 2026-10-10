// SPDX-License-Identifier: MPL-2.0
//! Windows-only recording worker. Invoke inside the supervised recording process,
//! never on the Tauri/UI thread: capture shutdown, WriteSample and Finalize can
//! block in a driver. The parent owns deadlines, process termination and assets.
//!
//! WGC supplies a single replaceable latest frame. Media Foundation's synchronous
//! sink writer keeps its default throttling; there is no application frame queue.
//! https://learn.microsoft.com/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsinkwriter-writesample

pub use crate::recording_audio::{AudioMode, RecordingStopReason};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::mpsc::Receiver, time::Duration};

const FPS: u32 = 15;
const MAX_PIXELS: u64 = 4_000_000;
const MAX_ACTIVE: Duration = Duration::from_secs(10 * 60);
const TICKS_PER_SECOND: u64 = 10_000_000;
const FRAME_TICKS: i64 = (TICKS_PER_SECOND / FPS as u64) as i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub output: PathBuf,
    pub origin_x: i32,
    pub origin_y: i32,
    pub monitor_width: u32,
    pub monitor_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub audio: AudioMode,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Stop,
    Pause,
    Resume,
    Cancel,
}

impl<'de> Deserialize<'de> for Control {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Stop {},
            Pause {},
            Resume {},
            Cancel {},
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Stop {} => Self::Stop,
            Wire::Pause {} => Self::Pause,
            Wire::Resume {} => Self::Resume,
            Wire::Cancel {} => Self::Cancel,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Progress {
    pub elapsed_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub frames: u64,
    pub audio: AudioMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<RecordingStopReason>,
}

fn validate(config: &Config) -> Result<(), String> {
    if !config.output.is_absolute() || config.output.extension().is_none_or(|ext| ext != "mp4") {
        return Err("录制文件必须是应用生成的绝对 MP4 路径".into());
    }
    if config.width < 2 || config.height < 2 || config.width % 2 != 0 || config.height % 2 != 0 {
        return Err("录制区域宽高必须是大于零的偶数".into());
    }
    if u64::from(config.width) * u64::from(config.height) > MAX_PIXELS {
        return Err("录制区域不能超过 400 万像素".into());
    }
    if config.monitor_width == 0
        || config.monitor_height == 0
        || config.monitor_width > 16_384
        || config.monitor_height > 16_384
        || u64::from(config.monitor_width) * u64::from(config.monitor_height) > 33_554_432
        || config
            .x
            .checked_add(config.width)
            .is_none_or(|end| end > config.monitor_width)
        || config
            .y
            .checked_add(config.height)
            .is_none_or(|end| end > config.monitor_height)
    {
        return Err("录制区域已超出屏幕，请重新选择".into());
    }
    Ok(())
}

fn ticks(duration: Duration) -> i64 {
    (duration.as_nanos() / 100).min(i64::MAX as u128) as i64
}

/// Copy only pixel bytes from a top-down D3D buffer. Media Foundation's input
/// type explicitly declares a positive (top-down) stride; do not flip this path.
fn packed_bgra(source: &[u8], pitch: usize, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let row = (width as usize).checked_mul(4).ok_or("录制像素尺寸无效")?;
    let len = row.checked_mul(height as usize).ok_or("录制像素尺寸无效")?;
    let required = pitch
        .checked_mul(height as usize)
        .ok_or("录制像素尺寸无效")?;
    if width == 0
        || height == 0
        || pitch < row
        || source.len() < required
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("录制帧的行间距或长度无效".into());
    }
    let mut packed = vec![0; len];
    for y in 0..height as usize {
        packed[y * row..(y + 1) * row].copy_from_slice(&source[y * pitch..y * pitch + row]);
    }
    Ok(packed)
}

pub fn run(
    config: Config,
    commands: Receiver<Control>,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Option<Report>, String> {
    validate(&config)?;
    #[cfg(windows)]
    {
        native::run(config, commands, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (commands, progress);
        Err("当前平台尚未实现原生录屏".into())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::recording_audio::{qpc_now, AudioEngine, StartError};
    use crate::recording_audio_timeline::{Timeline, JITTER_TICKS, TICKS_PER_SECOND as QPC_TICKS};
    use std::{
        fs::{self, OpenOptions},
        sync::{
            mpsc::{RecvTimeoutError, TryRecvError},
            Arc, Mutex,
        },
        time::Instant,
    };
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::{
            Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
            Media::MediaFoundation::*,
            System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
            UI::HiDpi::{
                SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
        },
    };
    use windows_capture::{
        capture::{Context, GraphicsCaptureApiHandler},
        frame::Frame,
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        },
    };

    const FRAME_PERIOD: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);
    const POLL: Duration = Duration::from_millis(25);
    const HEARTBEAT: Duration = Duration::from_millis(250);

    fn failure(context: &str, error: impl std::fmt::Display) -> String {
        format!("{context}：{error}")
    }

    struct Runtime;
    impl Runtime {
        fn new() -> Result<Self, String> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(|e| failure("无法初始化录制线程", e))?;
                if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                    CoUninitialize();
                    return Err(failure("Windows 媒体组件不可用", error));
                }
            }
            Ok(Self)
        }
    }
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
        }
    }

    struct DpiGuard(DPI_AWARENESS_CONTEXT);
    impl DpiGuard {
        fn new() -> Result<Self, String> {
            let previous =
                unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            if previous.0.is_null() {
                return Err("无法确定屏幕的物理像素坐标".into());
            }
            Ok(Self(previous))
        }
    }
    impl Drop for DpiGuard {
        fn drop(&mut self) {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }

    fn monitor_matches(handle: usize, config: &Config) -> Result<bool, String> {
        let _dpi = DpiGuard::new()?;
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(HMONITOR(handle as *mut _), &mut info) }.as_bool() {
            return Err("录制屏幕已断开".into());
        }
        let rect = info.rcMonitor;
        Ok(rect.left == config.origin_x
            && rect.top == config.origin_y
            && i64::from(rect.right) - i64::from(rect.left) == i64::from(config.monitor_width)
            && i64::from(rect.bottom) - i64::from(rect.top) == i64::from(config.monitor_height))
    }

    #[derive(Default)]
    struct Latest {
        pixels: Option<Arc<[u8]>>,
        error: Option<String>,
    }

    struct CaptureFlags {
        config: Config,
        handle: usize,
        latest: Arc<Mutex<Latest>>,
    }
    struct Capture {
        flags: CaptureFlags,
        last_copy: Option<Instant>,
    }

    impl Capture {
        fn copy_frame(&mut self, frame: &mut Frame) -> Result<(), String> {
            let config = &self.flags.config;
            // Check both the frame dimensions and the monitor's current location,
            // including while paused. Never clamp a stale ROI to a different screen.
            if frame.width() != config.monitor_width
                || frame.height() != config.monitor_height
                || !monitor_matches(self.flags.handle, config)?
            {
                return Err("屏幕尺寸或位置已改变，录制已停止".into());
            }
            if self
                .last_copy
                .is_some_and(|time| time.elapsed() < FRAME_PERIOD)
            {
                return Ok(());
            }
            let mut cropped = frame
                .buffer_crop(
                    config.x,
                    config.y,
                    config.x + config.width,
                    config.y + config.height,
                )
                .map_err(|e| failure("读取录制区域失败", e))?;
            let pitch = cropped.row_pitch() as usize;
            let pixels = packed_bgra(cropped.as_raw_buffer(), pitch, config.width, config.height)?;
            self.flags
                .latest
                .lock()
                .map_err(|_| "录制状态不可用")?
                .pixels = Some(Arc::from(pixels));
            self.last_copy = Some(Instant::now());
            Ok(())
        }
    }

    impl GraphicsCaptureApiHandler for Capture {
        type Flags = CaptureFlags;
        type Error = String;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self {
                flags: ctx.flags,
                last_copy: None,
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _: InternalCaptureControl,
        ) -> Result<(), String> {
            let result = self.copy_frame(frame);
            if let Err(error) = &result {
                if let Ok(mut latest) = self.flags.latest.lock() {
                    latest.error = Some(error.clone());
                }
            }
            result
        }

        fn on_closed(&mut self) -> Result<(), String> {
            if let Ok(mut latest) = self.flags.latest.lock() {
                latest.error = Some("录制屏幕已关闭或断开".into());
            }
            Ok(())
        }
    }

    struct Writer {
        sink: IMFSinkWriter,
        stream: u32,
        audio_stream: Option<u32>,
        bytes_per_frame: usize,
    }

    impl Writer {
        fn new(config: &Config) -> Result<Self, String> {
            let create = || -> windows::core::Result<Self> {
                unsafe {
                    let mut attributes = None;
                    MFCreateAttributes(&mut attributes, 3)?;
                    let attributes = attributes.ok_or_else(windows::core::Error::from_thread)?;
                    attributes
                        .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
                    attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
                    // Keep the default synchronous throttling. Do not set an async
                    // callback or MF_SINK_WRITER_DISABLE_THROTTLING.
                    let path = HSTRING::from(config.output.as_os_str());
                    let sink = MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None, &attributes)?;
                    let output = MFCreateMediaType()?;
                    output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                    output.SetUINT32(&MF_MT_AVG_BITRATE, 8_000_000)?;
                    set_geometry(&output, config)?;
                    let stream = sink.AddStream(&output)?;
                    let input = MFCreateMediaType()?;
                    input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    set_geometry(&input, config)?;
                    input.SetUINT32(&MF_MT_DEFAULT_STRIDE, config.width * 4)?;
                    input.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
                    input.SetUINT32(&MF_MT_SAMPLE_SIZE, config.width * config.height * 4)?;
                    sink.SetInputMediaType(stream, &input, None)?;
                    let audio_stream = if config.audio.enabled() {
                        let output = MFCreateMediaType()?;
                        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
                        output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)?;
                        output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24_000)?;
                        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                        output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
                        let audio_stream = sink.AddStream(&output)?;
                        let format = crate::recording_audio::platform::Format {
                            rate: 48_000,
                            channels: 2,
                            bits: 16,
                            valid_bits: 16,
                            mask: 3,
                            float: false,
                        };
                        let input = crate::recording_audio::platform::audio_type(&format)?;
                        sink.SetInputMediaType(audio_stream, &input, None)?;
                        Some(audio_stream)
                    } else {
                        None
                    };
                    sink.BeginWriting()?;
                    Ok(Self {
                        sink,
                        stream,
                        audio_stream,
                        bytes_per_frame: config.width as usize * config.height as usize * 4,
                    })
                }
            };
            create().map_err(|e| failure("无法创建 Windows H.264 编码器", e))
        }

        fn write(&self, pixels: &[u8], timestamp: i64, duration: i64) -> Result<(), String> {
            if pixels.len() != self.bytes_per_frame || timestamp < 0 || duration <= 0 {
                return Err("编码帧的尺寸或时间戳无效".into());
            }
            let write = || -> windows::core::Result<()> {
                unsafe {
                    let buffer = MFCreateMemoryBuffer(pixels.len() as u32)?;
                    let mut destination = std::ptr::null_mut();
                    buffer.Lock(&mut destination, None, None)?;
                    if destination.is_null() {
                        let _ = buffer.Unlock();
                        return Err(windows::core::Error::from_thread());
                    }
                    // The buffer was allocated at exactly pixels.len(); it stays
                    // locked for this copy, with no fallible operation in between.
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), destination, pixels.len());
                    buffer.Unlock()?;
                    buffer.SetCurrentLength(pixels.len() as u32)?;
                    let sample = MFCreateSample()?;
                    sample.AddBuffer(&buffer)?;
                    sample.SetSampleTime(timestamp)?;
                    sample.SetSampleDuration(duration)?;
                    self.sink.WriteSample(self.stream, &sample)
                }
            };
            write().map_err(|e| failure("写入录制视频失败", e))
        }

        fn finish(self) -> Result<(), String> {
            unsafe { self.sink.Finalize() }.map_err(|e| failure("完成 MP4 文件失败", e))
        }

        fn audio(&self, samples: &[i16], timestamp: u64, duration: u64) -> Result<(), String> {
            let stream = self.audio_stream.ok_or("当前录制没有声音流")?;
            if samples.is_empty()
                || samples.len() > 960
                || samples.len() % 2 != 0
                || timestamp > i64::MAX as u64
                || duration == 0
                || duration > i64::MAX as u64
            {
                return Err("声音编码样本长度或时间无效".into());
            }
            let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
            let sample = unsafe {
                crate::recording_audio::platform::byte_sample(
                    &bytes,
                    timestamp as i64,
                    duration as i64,
                )
            }
            .map_err(|_| "无法创建声音编码样本")?;
            unsafe { self.sink.WriteSample(stream, &sample) }.map_err(|_| "写入录制声音失败".into())
        }
    }

    unsafe fn set_geometry(
        media_type: &IMFMediaType,
        config: &Config,
    ) -> windows::core::Result<()> {
        unsafe {
            media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            media_type.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(config.width) << 32) | u64::from(config.height),
            )?;
            media_type.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(FPS) << 32) | 1)?;
            media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)
        }
    }

    fn latest_frame(latest: &Mutex<Latest>) -> Result<Option<Arc<[u8]>>, String> {
        let latest = latest.lock().map_err(|_| "录制状态不可用")?;
        if let Some(error) = &latest.error {
            return Err(error.clone());
        }
        Ok(latest.pixels.clone())
    }

    fn receive_control(command: Control, clock: &mut Timeline, now: u64) -> Result<bool, String> {
        match command {
            Control::Stop => Ok(true),
            Control::Cancel => Err("录制已取消".into()),
            Control::Pause => {
                clock
                    .set_paused(true, now)
                    .map_err(|_| "录制暂停时钟超限或无效")?;
                Ok(false)
            }
            Control::Resume => {
                clock
                    .set_paused(false, now)
                    .map_err(|_| "录制恢复时钟超限或无效")?;
                Ok(false)
            }
        }
    }

    pub(super) fn run(
        config: Config,
        commands: Receiver<Control>,
        progress: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Result<Option<Report>, String> {
        let _runtime = Runtime::new()?;
        let monitor = Monitor::enumerate()
            .map_err(|e| failure("读取屏幕列表失败", e))?
            .into_iter()
            .find(|monitor| {
                monitor_matches(monitor.as_raw_hmonitor() as usize, &config).unwrap_or(false)
            })
            .ok_or("原录制屏幕已改变或断开，请重新截图选择区域")?;
        // Reserve a new parent-generated file. A stale/repeated worker command
        // must never truncate an existing recording.
        drop(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&config.output)
                .map_err(|e| failure("无法创建录制文件", e))?,
        );
        let result = record(&config, monitor, commands, progress);
        if result.is_err() || matches!(&result, Ok(None)) {
            let _ = fs::remove_file(&config.output);
        }
        result
    }

    #[cfg(test)]
    mod codec_tests {
        use super::*;

        #[test]
        #[ignore = "requires the Windows Media Foundation H.264 codec; captures no desktop"]
        fn synthetic_frames_finalize_as_silent_h264_mp4() {
            let _runtime = Runtime::new().unwrap();
            let output =
                std::env::temp_dir().join(format!("mewu-codec-test-{}.mp4", uuid::Uuid::new_v4()));
            struct RemoveFile(PathBuf);
            impl Drop for RemoveFile {
                fn drop(&mut self) {
                    let _ = fs::remove_file(&self.0);
                }
            }
            let _cleanup = RemoveFile(output.clone());
            let config = Config {
                output,
                origin_x: 0,
                origin_y: 0,
                monitor_width: 128,
                monitor_height: 96,
                x: 0,
                y: 0,
                width: 128,
                height: 96,
                audio: AudioMode::Mute,
            };
            let writer = Writer::new(&config).unwrap();
            let mut pixels = [0u8, 0, 255, 255].repeat(128 * 48);
            pixels.extend_from_slice(&[255u8, 0, 0, 255].repeat(128 * 48));
            for index in 0..4 {
                writer
                    .write(&pixels, index * FRAME_TICKS, FRAME_TICKS)
                    .unwrap();
            }
            writer.finish().unwrap();
            unsafe {
                let path = HSTRING::from(config.output.as_os_str());
                let mut attributes = None;
                MFCreateAttributes(&mut attributes, 1).unwrap();
                let attributes = attributes.unwrap();
                attributes
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                    .unwrap();
                let reader =
                    MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), &attributes).unwrap();
                let media_type = reader
                    .GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, 0)
                    .unwrap();
                assert_eq!(
                    media_type.GetGUID(&MF_MT_SUBTYPE).unwrap(),
                    MFVideoFormat_H264
                );
                assert_eq!(
                    media_type.GetUINT64(&MF_MT_FRAME_SIZE).unwrap(),
                    (128u64 << 32) | 96
                );
                assert!(reader
                    .GetNativeMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, 0)
                    .is_err());
                let decoded_type = MFCreateMediaType().unwrap();
                decoded_type
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .unwrap();
                decoded_type
                    .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                    .unwrap();
                reader
                    .SetCurrentMediaType(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        None,
                        &decoded_type,
                    )
                    .unwrap();
                let mut sample = None;
                let mut stream_index = 0;
                let mut stream_flags = 0;
                let mut timestamp = 0;
                reader
                    .ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        0,
                        Some(&mut stream_index),
                        Some(&mut stream_flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                    .unwrap();
                assert!(
                    sample.is_some(),
                    "finalized MP4 must contain a readable sample"
                );
                let decoded_type = reader
                    .GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
                    .unwrap();
                let stride = decoded_type.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap() as i32;
                let buffer = sample.unwrap().ConvertToContiguousBuffer().unwrap();
                let mut raw = std::ptr::null_mut();
                let mut length = 0;
                buffer.Lock(&mut raw, None, Some(&mut length)).unwrap();
                let decoded = std::slice::from_raw_parts(raw, length as usize).to_vec();
                buffer.Unlock().unwrap();
                let row = |y: usize| {
                    if stride < 0 {
                        (95 - y) * stride.unsigned_abs() as usize
                    } else {
                        y * stride as usize
                    }
                };
                let top = &decoded[row(12) + 64 * 4..row(12) + 64 * 4 + 3];
                let bottom = &decoded[row(84) + 64 * 4..row(84) + 64 * 4 + 3];
                assert!(
                    top[2] > 180 && top[0] < 60,
                    "top half must stay red: {top:?}"
                );
                assert!(
                    bottom[0] > 180 && bottom[2] < 60,
                    "bottom half must stay blue: {bottom:?}"
                );
            }
        }
    }

    #[cfg(test)]
    mod audio_probe_tests {
        use super::*;
        use crate::recording_audio::platform::Format;
        use crate::recording_audio_timeline::{frames_to_ticks, SAMPLE_RATE};
        use crate::video_clip_contract::{ClipJob, ClipJobResult, VideoRange};
        use sha2::{Digest, Sha256};

        fn tone(rate: u32, channels: u16, start: u64, count: usize, frequency: f64) -> Vec<u8> {
            let mut bytes = Vec::with_capacity(count * channels as usize * 4);
            for frame in 0..count {
                let value = if frequency == 0.0 {
                    0.0
                } else {
                    (0.35
                        * (std::f64::consts::TAU * frequency * (start + frame as u64) as f64
                            / rate as f64)
                            .sin()) as f32
                };
                for _ in 0..channels {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            bytes
        }

        unsafe fn decode_audio(output: &std::path::Path) -> Vec<[i16; 2]> {
            unsafe {
                let path = HSTRING::from(output.as_os_str());
                let reader = MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), None).unwrap();
                let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
                let native = reader.GetNativeMediaType(stream, 0).unwrap();
                assert_eq!(native.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFAudioFormat_AAC);
                assert_eq!(native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap(), 2);
                reader
                    .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                    .unwrap();
                reader.SetStreamSelection(stream, true).unwrap();
                let format = Format {
                    rate: 48_000,
                    channels: 2,
                    bits: 16,
                    valid_bits: 16,
                    mask: 3,
                    float: false,
                };
                reader
                    .SetCurrentMediaType(
                        stream,
                        None,
                        &crate::recording_audio::platform::audio_type(&format).unwrap(),
                    )
                    .unwrap();
                let mut decoded = Vec::new();
                let mut previous = -1;
                let mut ended = false;
                for _ in 0..2000 {
                    let (mut flags, mut timestamp, mut sample) = (0, 0, None);
                    reader
                        .ReadSample(
                            stream,
                            0,
                            None,
                            Some(&mut flags),
                            Some(&mut timestamp),
                            Some(&mut sample),
                        )
                        .unwrap();
                    if let Some(sample) = sample {
                        assert!(timestamp >= previous);
                        previous = timestamp;
                        let buffer = sample.ConvertToContiguousBuffer().unwrap();
                        let (mut ptr, mut length) = (std::ptr::null_mut(), 0);
                        buffer.Lock(&mut ptr, None, Some(&mut length)).unwrap();
                        assert!(!ptr.is_null() && length % 4 == 0);
                        let bytes = std::slice::from_raw_parts(ptr, length as usize).to_vec();
                        buffer.Unlock().unwrap();
                        decoded.extend(bytes.chunks_exact(4).map(|v| {
                            [
                                i16::from_le_bytes(v[..2].try_into().unwrap()),
                                i16::from_le_bytes(v[2..].try_into().unwrap()),
                            ]
                        }));
                    }
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        ended = true;
                        break;
                    }
                }
                assert!(ended, "bounded decode must reach EOF");
                decoded
            }
        }

        unsafe fn check_video_timecodes(output: &std::path::Path) {
            unsafe {
                let path = HSTRING::from(output.as_os_str());
                let mut attributes = None;
                MFCreateAttributes(&mut attributes, 1).unwrap();
                let attributes = attributes.unwrap();
                attributes
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                    .unwrap();
                let reader =
                    MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), &attributes).unwrap();
                let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
                reader
                    .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                    .unwrap();
                reader.SetStreamSelection(stream, true).unwrap();
                let native = reader.GetNativeMediaType(stream, 0).unwrap();
                assert_eq!(native.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFVideoFormat_H264);
                let media = MFCreateMediaType().unwrap();
                media
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .unwrap();
                media.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).unwrap();
                reader.SetCurrentMediaType(stream, None, &media).unwrap();
                let mut previous = -1;
                let mut end = 0;
                let mut marks = [false; 3];
                let mut eof = false;
                for _ in 0..500 {
                    let (mut flags, mut timestamp, mut sample) = (0, 0, None);
                    reader
                        .ReadSample(
                            stream,
                            0,
                            None,
                            Some(&mut flags),
                            Some(&mut timestamp),
                            Some(&mut sample),
                        )
                        .unwrap();
                    if let Some(sample) = sample {
                        assert!(timestamp >= previous);
                        previous = timestamp;
                        end = timestamp + sample.GetSampleDuration().unwrap();
                        for (index, marker) in
                            [5_000_000, 50_000_000, 80_000_000].into_iter().enumerate()
                        {
                            if timestamp >= marker && timestamp < marker + FRAME_TICKS * 2 {
                                let buffer = sample.ConvertToContiguousBuffer().unwrap();
                                let (mut pointer, mut size) = (std::ptr::null_mut(), 0);
                                buffer.Lock(&mut pointer, None, Some(&mut size)).unwrap();
                                assert!(!pointer.is_null() && size >= 4);
                                let pixel = std::slice::from_raw_parts(pointer, 4).to_vec();
                                buffer.Unlock().unwrap();
                                let channel = [2usize, 0, 1][index];
                                assert!(pixel[channel] > 180, "video timecode {index}: {pixel:?}");
                                marks[index] = true;
                            }
                        }
                    }
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        eof = true;
                        break;
                    }
                }
                assert!(eof && marks.into_iter().all(|v| v));
                assert!(
                    (end - 110_000_000).abs() <= 1_000_000,
                    "video duration must exclude paused second: {end}"
                );
            }
        }

        fn rms(audio: &[[i16; 2]], begin: f64, end: f64, channel: usize) -> f64 {
            let slice = &audio[(begin * 48_000.0) as usize..(end * 48_000.0) as usize];
            (slice
                .iter()
                .map(|v| (v[channel] as f64).powi(2))
                .sum::<f64>()
                / slice.len() as f64)
                .sqrt()
        }
        fn energy(audio: &[[i16; 2]], frequency: f64) -> f64 {
            energy_window(audio, frequency, 5.0, 6.0)
        }
        fn energy_window(audio: &[[i16; 2]], frequency: f64, begin: f64, end: f64) -> f64 {
            let slice = &audio[(begin * 48_000.0) as usize..(end * 48_000.0) as usize];
            let (mut real, mut imag) = (0.0, 0.0);
            for (i, value) in slice.iter().enumerate() {
                let phase = std::f64::consts::TAU * frequency * i as f64 / 48_000.0;
                real += value[0] as f64 * phase.cos();
                imag += value[0] as f64 * phase.sin();
            }
            (real * real + imag * imag).sqrt() / slice.len() as f64
        }

        /// Uses only generated PCM and RGB data. Does not call AudioEngine::start,
        /// IMMDeviceEnumerator, WGC, microphone or any playback API.
        #[test]
        #[ignore = "Windows MF synthetic A/V + resampler probe; no devices or screen"]
        fn synthetic_audio_video_pause_mix_finalizes() {
            let _runtime = Runtime::new().unwrap();
            let directory =
                std::env::temp_dir().join(format!("mewu-audio-probe-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&directory).unwrap();
            let output = directory.join("synthetic.mp4");
            let config = Config {
                output: output.clone(),
                origin_x: 0,
                origin_y: 0,
                monitor_width: 128,
                monitor_height: 96,
                x: 0,
                y: 0,
                width: 128,
                height: 96,
                audio: AudioMode::Both,
            };
            let mut audio = AudioEngine::synthetic(vec![
                Format {
                    rate: 44_100,
                    channels: 1,
                    bits: 32,
                    valid_bits: 32,
                    mask: 4,
                    float: true,
                },
                Format {
                    rate: 48_000,
                    channels: 2,
                    bits: 32,
                    valid_bits: 32,
                    mask: 3,
                    float: true,
                },
            ])
            .unwrap();
            let writer = Writer::new(&config).unwrap();
            let epoch = 10_000_000;
            let mut timeline = Timeline::new(epoch, false);
            let mut video_index = 0u64;
            for step in 0..1200u64 {
                let qpc = epoch + step * 100_000;
                if step == 700 {
                    timeline.set_paused(true, qpc).unwrap();
                }
                if step == 800 {
                    timeline.set_paused(false, qpc).unwrap();
                }
                let frequency = if (400..700).contains(&step) {
                    440.0
                } else if (700..800).contains(&step) {
                    2000.0
                } else {
                    0.0
                };
                // The loopback source deliberately supplies no events during
                // long silence. It resumes with real packets at the same QPC.
                if (400..800).contains(&step) || step % 200 == 0 {
                    audio
                        .synthetic_packet(
                            0,
                            qpc,
                            tone(44_100, 1, step * 441, 441, frequency),
                            &timeline,
                        )
                        .unwrap();
                }
                audio
                    .synthetic_packet(
                        1,
                        qpc,
                        tone(48_000, 2, step * 480, 480, frequency * 2.0),
                        &timeline,
                    )
                    .unwrap();
                audio.settle_idle(&timeline, qpc + 100_000).unwrap();
                let elapsed = timeline.elapsed(qpc + 100_000).unwrap();
                let available = elapsed.saturating_sub(JITTER_TICKS);
                audio
                    .write_until(available, |data, time, duration| {
                        writer.audio(data, time, duration)
                    })
                    .unwrap();
                while frames_to_ticks(video_index, FPS) + FRAME_TICKS as u64 <= available {
                    let time = frames_to_ticks(video_index, FPS);
                    let pixel = if time < 40_000_000 {
                        [0u8, 0, 255, 255]
                    } else if time < 70_000_000 {
                        [255u8, 0, 0, 255]
                    } else {
                        [0u8, 255, 0, 255]
                    };
                    writer
                        .write(&pixel.repeat(128 * 96), time as i64, FRAME_TICKS)
                        .unwrap();
                    video_index += 1;
                }
                timeline.prune_before(qpc.saturating_sub(QPC_TICKS));
            }
            let duration = timeline.elapsed(epoch + 120_000_000).unwrap();
            assert_eq!(duration, 110_000_000);
            audio.finish_processing(duration).unwrap();
            audio
                .write_until(duration, |data, time, span| writer.audio(data, time, span))
                .unwrap();
            while frames_to_ticks(video_index, FPS) < duration {
                let start = frames_to_ticks(video_index, FPS);
                let end = frames_to_ticks(video_index + 1, FPS).min(duration);
                writer
                    .write(
                        &[0u8, 255, 0, 255].repeat(128 * 96),
                        start as i64,
                        (end - start) as i64,
                    )
                    .unwrap();
                video_index += 1;
            }
            writer.finish().unwrap();
            let decoded = unsafe { decode_audio(&output) };
            unsafe {
                check_video_timecodes(&output);
            }
            assert!(
                decoded.len().abs_diff(11 * SAMPLE_RATE as usize) <= 4096,
                "AAC pad bound: {}",
                decoded.len()
            );
            assert!(rms(&decoded, 0.5, 3.5, 0) < 20.0, "silence before tone");
            assert!(rms(&decoded, 4.5, 6.5, 0) > 3000.0, "both sources audible");
            assert!(
                rms(&decoded, 8.0, 10.0, 0) < 20.0,
                "paused tone cannot leak after resume"
            );
            assert!(
                (rms(&decoded, 4.5, 6.5, 0) - rms(&decoded, 4.5, 6.5, 1)).abs() < 150.0,
                "mono is copied to both channels"
            );
            assert!(energy(&decoded, 440.0) > 1500.0 && energy(&decoded, 880.0) > 1500.0);
            assert!(energy(&decoded, 2000.0) < 100.0 && energy(&decoded, 4000.0) < 100.0);
            // Exercise the actual exported-recording path too: inspect the new
            // AAC/H.264 file, trim it through production MediaComposition, then
            // independently decode the retained audio rather than trusting its
            // metadata or a successful render status alone.
            let source_sha = Sha256::digest(fs::read(&output).unwrap());
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let ClipJobResult::Inspected(original) = crate::video_clip_backend::execute(
                &ClipJob::Inspect {
                    source: output.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("inspect synthetic recording");
            };
            assert!(original.has_audio);
            let clipped_path = directory.join("trimmed-audio.mp4");
            let range = VideoRange {
                start_ticks: 42_500_000,
                end_ticks: 62_500_000,
            };
            let ClipJobResult::Rendered(clipped) = crate::video_clip_backend::execute(
                &ClipJob::Render {
                    source: output.clone(),
                    output: clipped_path.clone(),
                    expected: original.clone(),
                    range: range.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("render synthetic recording");
            };
            crate::video_clip_contract::validate_rendered(&original, &range, &clipped).unwrap();
            assert!(clipped.has_audio);
            let clipped_audio = unsafe { decode_audio(&clipped_path) };
            assert!(
                clipped_audio.len().abs_diff(2 * SAMPLE_RATE as usize) <= 4096,
                "trimmed AAC pad bound: {}",
                clipped_audio.len()
            );
            assert!(rms(&clipped_audio, 0.25, 1.75, 0) > 3000.0);
            for frequency in [440.0, 880.0] {
                assert!(energy_window(&clipped_audio, frequency, 0.5, 1.5) > 1500.0);
            }
            for frequency in [2000.0, 4000.0] {
                assert!(energy_window(&clipped_audio, frequency, 0.5, 1.5) < 100.0);
            }
            assert_eq!(source_sha, Sha256::digest(fs::read(&output).unwrap()));
            println!(
                "synthetic trimmed MP4: {} ticks, {} PCM frames, source SHA unchanged",
                clipped.duration_ticks,
                clipped_audio.len()
            );
            // Leave only opt-in evidence in a caller-controlled temporary root.
            println!(
                "synthetic MP4: {} ({} PCM frames; no devices)",
                output.display(),
                decoded.len()
            );
            drop(audio);
            fs::remove_file(&clipped_path).unwrap();
            fs::remove_file(&output).unwrap();
            fs::remove_dir(&directory).unwrap();
        }

        #[test]
        fn startup_cancel_is_observed_before_any_audio_factory() {
            for command in [Control::Stop, Control::Cancel] {
                let (send, receive) = std::sync::mpsc::channel();
                send.send(Control::Pause).unwrap();
                send.send(command).unwrap();
                let mut paused = false;
                assert!(matches!(
                    AudioEngine::start(AudioMode::Both, || startup_control(&receive, &mut paused)),
                    Err(StartError::Stopped)
                ));
                assert!(paused);
            }
            assert!(
                matches!(AudioEngine::start(AudioMode::Both, || Err("fixture failure".into())), Err(StartError::Failed(value)) if value == "fixture failure")
            );
            assert!(matches!(
                AudioEngine::start(AudioMode::Mute, || Ok(true)),
                Ok(None)
            ));
        }
    }

    fn startup_control(commands: &Receiver<Control>, paused: &mut bool) -> Result<bool, String> {
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

    fn record(
        config: &Config,
        monitor: Monitor,
        commands: Receiver<Control>,
        progress: impl Fn(Progress),
    ) -> Result<Option<Report>, String> {
        let mut initially_paused = false;
        if !startup_control(&commands, &mut initially_paused)? {
            return Ok(None);
        }
        let mut audio = match AudioEngine::start(config.audio, || {
            startup_control(&commands, &mut initially_paused)
        }) {
            Ok(audio) => audio,
            Err(StartError::Stopped) => return Ok(None),
            Err(StartError::Failed(error)) => return Err(error),
        };
        if !startup_control(&commands, &mut initially_paused)? {
            return Ok(None);
        }
        let writer = Writer::new(config)?;
        if !startup_control(&commands, &mut initially_paused)? {
            return Ok(None);
        }
        let latest = Arc::new(Mutex::new(Latest::default()));
        let handle = monitor.as_raw_hmonitor() as usize;
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            CaptureFlags {
                config: config.clone(),
                handle,
                latest: latest.clone(),
            },
        );
        let capture = Capture::start_free_threaded(settings)
            .map_err(|e| failure("无法启动 Windows 屏幕录制", e))?;
        let result = (|| {
            let started_waiting = Instant::now();
            let first = loop {
                match commands.recv_timeout(POLL) {
                    Ok(Control::Cancel | Control::Stop) => return Ok(None),
                    Ok(Control::Pause) => initially_paused = true,
                    Ok(Control::Resume) => initially_paused = false,
                    Err(RecvTimeoutError::Disconnected) => return Err("录制控制连接已断开".into()),
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if let Some(frame) = latest_frame(&latest)? {
                    break frame;
                }
                if capture.is_finished() {
                    return Err("屏幕采集已意外结束".into());
                }
                if started_waiting.elapsed() >= Duration::from_secs(5) {
                    return Err("等待屏幕画面超时".into());
                }
            };
            if !startup_control(&commands, &mut initially_paused)? {
                return Ok(None);
            }
            let epoch = qpc_now()?;
            let mut clock = Timeline::new(epoch, initially_paused);
            if let Some(audio) = &mut audio {
                audio.arm()?;
            }
            writer.write(&first, 0, FRAME_TICKS)?;
            let mut frames = 1;
            let mut last_end = FRAME_TICKS;
            let mut last_pixels = first;
            let mut next_frame = Instant::now() + FRAME_PERIOD;
            let mut next_progress = Instant::now() + HEARTBEAT;
            let mut stop_reason = None;
            progress(Progress {
                elapsed_ms: 0,
                paused: clock.paused(),
            });

            'recording: loop {
                let now = qpc_now()?;
                clock.prune_before(now.saturating_sub(QPC_TICKS));
                if !monitor_matches(handle, config)? {
                    return Err("屏幕尺寸或位置已改变，录制已停止".into());
                }
                if let Some(frame) = latest_frame(&latest)? {
                    last_pixels = frame;
                }
                if capture.is_finished() {
                    return Err("屏幕采集已意外结束".into());
                }
                if clock.elapsed(now).map_err(|_| "录制时钟无效")? >= ticks(MAX_ACTIVE) as u64
                {
                    break;
                }
                loop {
                    match commands.try_recv() {
                        Ok(command) => {
                            if receive_control(command, &mut clock, qpc_now()?)? {
                                break 'recording;
                            }
                            progress(Progress {
                                elapsed_ms: clock
                                    .elapsed(qpc_now()?)
                                    .map_err(|_| "录制时钟无效")?
                                    / 10_000,
                                paused: clock.paused(),
                            });
                        }
                        Err(TryRecvError::Disconnected) => return Err("录制控制连接已断开".into()),
                        Err(TryRecvError::Empty) => break,
                    }
                }
                let active_ticks = clock.elapsed(qpc_now()?).map_err(|_| "录制时钟无效")?;
                if let Some(audio) = &mut audio {
                    if let Err(reason) = audio.poll(&clock) {
                        stop_reason = Some(reason);
                        break;
                    }
                    audio.write_until(
                        active_ticks.saturating_sub(JITTER_TICKS),
                        |samples, time, duration| writer.audio(samples, time, duration),
                    )?;
                }
                if !clock.paused() && Instant::now() >= next_frame {
                    let timestamp = (clock.elapsed(qpc_now()?).map_err(|_| "录制时钟无效")? as i64)
                        .max(last_end);
                    let duration = FRAME_TICKS.min(ticks(MAX_ACTIVE).saturating_sub(timestamp));
                    if duration <= 0 {
                        break;
                    }
                    writer.write(&last_pixels, timestamp, duration)?;
                    last_end = timestamp + duration;
                    frames += 1;
                    next_frame = Instant::now() + FRAME_PERIOD;
                }
                if Instant::now() >= next_progress {
                    progress(Progress {
                        elapsed_ms: clock.elapsed(qpc_now()?).map_err(|_| "录制时钟无效")? / 10_000,
                        paused: clock.paused(),
                    });
                    next_progress = Instant::now() + HEARTBEAT;
                }
                let wait = if clock.paused() {
                    POLL
                } else {
                    next_frame
                        .saturating_duration_since(Instant::now())
                        .min(POLL)
                };
                match commands.recv_timeout(wait) {
                    Ok(command) => {
                        if receive_control(command, &mut clock, qpc_now()?)? {
                            break;
                        }
                        progress(Progress {
                            elapsed_ms: clock.elapsed(qpc_now()?).map_err(|_| "录制时钟无效")?
                                / 10_000,
                            paused: clock.paused(),
                        });
                    }
                    Err(RecvTimeoutError::Disconnected) => return Err("录制控制连接已断开".into()),
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
            let stopped = qpc_now()?;
            let end = clock
                .elapsed(stopped)
                .map_err(|_| "录制时钟无效")?
                .min(ticks(MAX_ACTIVE) as u64) as i64;
            clock
                .set_paused(true, stopped)
                .map_err(|_| "录制结束时钟无效")?;
            if let Some(audio) = &mut audio {
                audio.stop();
                if let Err(reason) = audio.poll(&clock) {
                    stop_reason.get_or_insert(reason);
                }
                if let Err(reason) = audio.finish_processing(end.max(last_end) as u64) {
                    stop_reason.get_or_insert(reason);
                }
            }
            // Encoded first/final frame coverage can exceed the stop instant by
            // one 15fps frame. Both streams end at this same reported boundary.
            if end > last_end {
                writer.write(&last_pixels, last_end, end - last_end)?;
                frames += 1;
                last_end = end;
            }
            if let Some(audio) = &mut audio {
                audio.write_until(last_end as u64, |samples, time, duration| {
                    writer.audio(samples, time, duration)
                })?;
            }
            Ok(Some(Report {
                width: config.width,
                height: config.height,
                duration_ms: (last_end / 10_000) as u64,
                frames,
                audio: config.audio,
                stop_reason,
            }))
        })();
        if let Some(audio) = &mut audio {
            audio.stop();
        }
        let stop_result = capture.stop().map_err(|e| failure("停止屏幕采集失败", e));
        match result {
            Ok(Some(report)) => {
                // A warning is still successful only after the complete MP4
                // finalization. Device loss never takes the generic Err cleanup.
                stop_result?;
                writer.finish()?;
                Ok(Some(report))
            }
            Ok(None) => {
                drop(writer);
                stop_result?;
                Ok(None)
            }
            Err(error) => {
                drop(writer);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            output: std::env::temp_dir().join("mewu-recording-validation.mp4"),
            origin_x: -1920,
            origin_y: 0,
            monitor_width: 1920,
            monitor_height: 1080,
            x: 10,
            y: 20,
            width: 640,
            height: 480,
            audio: AudioMode::Mute,
        }
    }

    #[test]
    fn region_bounds_reject_overflow_odd_and_excess_pixels() {
        let mut config = config();
        assert!(validate(&config).is_ok());
        config.width = 641;
        assert!(validate(&config).is_err());
        config.width = 640;
        config.x = u32::MAX;
        assert!(validate(&config).is_err());
        config.x = 1282;
        assert!(validate(&config).is_err());
        config.x = 0;
        config.y = 0;
        config.monitor_width = 4000;
        config.monitor_height = 2000;
        config.width = 4000;
        config.height = 2000;
        assert!(validate(&config).is_err());
    }

    #[test]
    fn pause_time_is_excluded_and_commands_are_idempotent() {
        let mut clock = crate::recording_audio_timeline::Timeline::new(0, false);
        clock.set_paused(true, 20_000_000).unwrap();
        clock.set_paused(true, 50_000_000).unwrap();
        assert_eq!(clock.elapsed(200_000_000).unwrap(), 20_000_000);
        clock.set_paused(false, 200_000_000).unwrap();
        clock.set_paused(false, 210_000_000).unwrap();
        assert_eq!(clock.elapsed(230_000_000).unwrap(), 50_000_000);
    }

    #[test]
    fn bgra_copy_preserves_color_and_top_down_rows_without_padding() {
        // Distinct colors/rows make accidental flipping or RGBA conversion visible.
        let source = [
            1, 2, 3, 255, 4, 5, 6, 255, 99, 99, 99, 99, 7, 8, 9, 255, 10, 11, 12, 255, 88, 88, 88,
            88,
        ];
        assert_eq!(
            packed_bgra(&source, 12, 2, 2).unwrap(),
            vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        assert!(packed_bgra(&source[..20], 12, 2, 2).is_err());
        assert!(packed_bgra(&source, 7, 2, 2).is_err());
    }

    #[test]
    fn recording_ipc_has_explicit_shapes() {
        assert!(matches!(
            serde_json::from_str::<Control>(r#"{"type":"pause"}"#).unwrap(),
            Control::Pause
        ));
        assert!(serde_json::from_str::<Control>(r#"{"type":"delete"}"#).is_err());
        assert!(serde_json::from_str::<Control>(r#"{"type":"stop","microphone":true}"#).is_err());
        assert_eq!(
            serde_json::to_value(Progress {
                elapsed_ms: 42,
                paused: true
            })
            .unwrap(),
            serde_json::json!({ "elapsedMs": 42, "paused": true })
        );
    }
}
