// SPDX-License-Identifier: MPL-2.0
//! Strict host-only local media job and pipe protocol.
//! No path in these types is a renderer-controlled IPC argument.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u32 = 1;
pub const MIN_RANGE_TICKS: u64 = 1_000_000; // 100ms
pub const MAX_DURATION_TICKS: u64 = 18_000_000_000; // 30min source admission only
pub const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_SOURCE_PIXELS: u64 = 4_000_000; // current internally recorded assets
pub const MAX_GIF_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_GIF_FRAMES: u32 = 240;
pub const MAX_GIF_PIXELS: u64 = 24_000_000;
pub const MAX_GIF_SIDE: u32 = 720;
pub const MAX_EXTRACTED_FRAMES: usize = 6;
pub const MAX_FRAME_JPEG_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_FRAME_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_FRAME_SIDE: u32 = 1024;
pub const MAX_OVERLAYS: usize = 48;
pub const MAX_OVERLAY_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_OVERLAY_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_OVERLAY_TOTAL_PIXELS: u64 = 32 * 1024 * 1024;

/// The native MPEG-4 sink stores video at an integer milli-fps timebase.
/// Annotated export explicitly uses that representable CFR cadence, including
/// NTSC's 29970/1000. This is not preservation of an original VFR schedule.
pub fn annotated_output_cadence(source: &VideoMetadata) -> Result<(u32, u32), ClipError> {
    source.validate()?;
    Ok((
        (u64::from(source.frame_rate_numerator) * 1000 / u64::from(source.frame_rate_denominator))
            as u32,
        1000,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayRaster {
    pub png: PathBuf,
    pub png_sha256: String,
    pub width: u32,
    pub height: u32,
    pub bounds: OverlayBounds,
    pub interval: VideoRange,
}
pub fn validate_overlay_plan(
    source: &VideoMetadata,
    range: Option<&VideoRange>,
    overlays: &[OverlayRaster],
) -> Result<(), ClipError> {
    source.validate_range(range)?;
    if overlays.is_empty() || overlays.len() > MAX_OVERLAYS {
        return Err(ClipError::InvalidMedia);
    }
    let mut pixels = 0u64;
    let retained = range.cloned().unwrap_or(VideoRange {
        start_ticks: 0,
        end_ticks: source.duration_ticks,
    });
    let mut affects = false;
    for entry in overlays {
        let bounds = entry.bounds;
        pixels = pixels
            .checked_add(u64::from(entry.width) * u64::from(entry.height))
            .ok_or(ClipError::InvalidMedia)?;
        if !entry.png.is_absolute()
            || entry.width == 0
            || entry.height == 0
            || u64::from(entry.width) * u64::from(entry.height) > MAX_SOURCE_PIXELS
            || pixels > MAX_OVERLAY_TOTAL_PIXELS
            || entry.png_sha256.len() != 64
            || !entry
                .png_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || ![bounds.x, bounds.y, bounds.width, bounds.height]
                .iter()
                .all(|v| v.is_finite())
            || bounds.x < 0.
            || bounds.y < 0.
            || bounds.width <= 0.
            || bounds.height <= 0.
            || bounds.x + bounds.width > f64::from(source.width)
            || bounds.y + bounds.height > f64::from(source.height)
            || entry.interval.end_ticks > source.duration_ticks
            || entry
                .interval
                .end_ticks
                .checked_sub(entry.interval.start_ticks)
                .is_none_or(|n| n < MIN_RANGE_TICKS)
        {
            return Err(ClipError::InvalidMedia);
        }
        affects |= entry.interval.start_ticks < retained.end_ticks
            && entry.interval.end_ticks > retained.start_ticks;
    }
    if !affects {
        return Err(ClipError::InvalidRange);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameDescriptor {
    pub index: u32,
    pub requested_ticks: u64,
    pub actual_pts_ticks: u64,
    pub sample_duration_ticks: u64,
    pub encoded_width: u32,
    pub encoded_height: u32,
    pub jpeg_sha256: String,
    pub bytes_length: u64,
}
pub fn extracted_dimensions(source: &VideoMetadata) -> (u32, u32) {
    let longest = source.width.max(source.height);
    if longest <= MAX_FRAME_SIDE {
        (source.width, source.height)
    } else {
        (
            (u64::from(source.width) * u64::from(MAX_FRAME_SIDE) / u64::from(longest)).max(1)
                as u32,
            (u64::from(source.height) * u64::from(MAX_FRAME_SIDE) / u64::from(longest)).max(1)
                as u32,
        )
    }
}
pub fn validate_extraction_request(
    source: &VideoMetadata,
    window: &VideoRange,
    requested: &[u64],
) -> Result<(), ClipError> {
    source.validate_range(Some(window))?;
    if !(2..=MAX_EXTRACTED_FRAMES).contains(&requested.len())
        || requested
            .iter()
            .any(|&t| t < window.start_ticks || t >= window.end_ticks)
        || requested.windows(2).any(|t| t[0] >= t[1])
        || u64::from(source.width) * u64::from(source.height) * requested.len() as u64 > 24_000_000
    {
        return Err(ClipError::InvalidRange);
    }
    Ok(())
}
pub fn validate_frame_descriptors(
    source: &VideoMetadata,
    window: &VideoRange,
    requested: &[u64],
    frames: &[FrameDescriptor],
) -> Result<(), ClipError> {
    validate_extraction_request(source, window, requested)?;
    let dimensions = extracted_dimensions(source);
    if frames.len() != requested.len() {
        return Err(ClipError::Protocol);
    }
    let mut total = 0u64;
    for (index, frame) in frames.iter().enumerate() {
        total = total
            .checked_add(frame.bytes_length)
            .ok_or(ClipError::Protocol)?;
        if frame.index != index as u32
            || frame.requested_ticks != requested[index]
            || frame.actual_pts_ticks > frame.requested_ticks
            || frame.actual_pts_ticks >= source.duration_ticks
            || frame.sample_duration_ticks == 0
            || frame.sample_duration_ticks > MAX_DURATION_TICKS
            || (frame.encoded_width, frame.encoded_height) != dimensions
            || !(1..=MAX_FRAME_JPEG_BYTES).contains(&frame.bytes_length)
            || total > MAX_FRAME_TOTAL_BYTES
            || frame.jpeg_sha256.len() != 64
            || !frame
                .jpeg_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ClipError::Protocol);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GifMetadata {
    pub width: u32,
    pub height: u32,
    pub frame_count: u32,
    pub duration_centiseconds: u32,
}

/// Original logical source times, with exact half-open annotation boundaries.
/// GIF only stores centiseconds; unrepresentable adjacent critical boundaries
/// fail rather than silently stretching a short interval or omitting it.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotatedGifSchedule {
    pub metadata: GifMetadata,
    pub requested_ticks: Vec<u64>,
    pub delays: Vec<u16>,
}
impl AnnotatedGifSchedule {
    pub fn new(
        source: &VideoMetadata,
        range: Option<&VideoRange>,
        overlays: &[OverlayRaster],
    ) -> Result<Self, ClipError> {
        validate_overlay_plan(source, range, overlays)?;
        let start = range.map_or(0, |r| r.start_ticks);
        let end = range.map_or(source.duration_ticks, |r| r.end_ticks);
        let duration = end - start;
        let cs = |tick: u64| (tick - start + 50_000) / 100_000;
        let mut positions = std::collections::BTreeSet::from([start, end]);
        for entry in overlays {
            let left = entry.interval.start_ticks.max(start);
            let right = entry.interval.end_ticks.min(end);
            if left < right {
                positions.insert(left);
                positions.insert(right);
            }
        }
        if positions.len() - 1 > MAX_GIF_FRAMES as usize
            || positions
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| cs(pair[1]) - cs(pair[0]) < 2)
        {
            return Err(ClipError::InvalidRange);
        }
        let target = GifMetadata::for_source(source, range)?
            .frame_count
            .max((positions.len() - 1) as u32);
        let fillers = target as usize - (positions.len() - 1);
        for index in 1..=fillers {
            let tick =
                start + (u128::from(duration) * index as u128 / (fillers + 1) as u128) as u64;
            let left = *positions
                .range(..=tick)
                .next_back()
                .ok_or(ClipError::InvalidRange)?;
            let right = *positions
                .range(tick..)
                .next()
                .ok_or(ClipError::InvalidRange)?;
            if cs(tick) >= cs(left) + 2 && cs(right) >= cs(tick) + 2 {
                positions.insert(tick);
            }
        }
        let positions: Vec<_> = positions.into_iter().collect();
        let count = (positions.len() - 1) as u32;
        let scale = 1.0f64
            .min(f64::from(MAX_GIF_SIDE) / f64::from(source.width.max(source.height)))
            .min(
                (MAX_GIF_PIXELS as f64
                    / f64::from(count)
                    / (f64::from(source.width) * f64::from(source.height)))
                .sqrt(),
            );
        let metadata = GifMetadata {
            width: (f64::from(source.width) * scale).floor().max(1.) as u32,
            height: (f64::from(source.height) * scale).floor().max(1.) as u32,
            frame_count: count,
            duration_centiseconds: cs(end) as u32,
        };
        metadata.validate()?;
        let delays: Result<Vec<u16>, _> = positions
            .windows(2)
            .map(|pair| u16::try_from(cs(pair[1]) - cs(pair[0])))
            .collect();
        Ok(Self {
            metadata,
            requested_ticks: positions[..positions.len() - 1].to_vec(),
            delays: delays.map_err(|_| ClipError::InvalidRange)?,
        })
    }
}
impl GifMetadata {
    pub fn for_source(
        source: &VideoMetadata,
        range: Option<&VideoRange>,
    ) -> Result<Self, ClipError> {
        source.validate_range(range)?;
        let duration = range.map_or(source.duration_ticks, |r| r.end_ticks - r.start_ticks);
        let frame_count = (u128::from(duration) * 15)
            .div_ceil(10_000_000)
            .clamp(1, u128::from(MAX_GIF_FRAMES)) as u32;
        let duration_centiseconds =
            ((duration + 50_000) / 100_000).max(u64::from(frame_count) * 2) as u32;
        let scale = 1.0f64
            .min(f64::from(MAX_GIF_SIDE) / f64::from(source.width.max(source.height)))
            .min(
                (MAX_GIF_PIXELS as f64
                    / f64::from(frame_count)
                    / (f64::from(source.width) * f64::from(source.height)))
                .sqrt(),
            );
        let result = Self {
            width: (f64::from(source.width) * scale).floor().max(1.0) as u32,
            height: (f64::from(source.height) * scale).floor().max(1.0) as u32,
            frame_count,
            duration_centiseconds,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), ClipError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_GIF_SIDE
            || self.height > MAX_GIF_SIDE
            || !(1..=MAX_GIF_FRAMES).contains(&self.frame_count)
            || u64::from(self.width) * u64::from(self.height) * u64::from(self.frame_count)
                > MAX_GIF_PIXELS
            || self.duration_centiseconds < self.frame_count * 2
            || self.duration_centiseconds > 180_000
        {
            return Err(ClipError::OutputInvalid);
        }
        Ok(())
    }
    pub fn validate_for(
        &self,
        source: &VideoMetadata,
        range: Option<&VideoRange>,
    ) -> Result<(), ClipError> {
        if self == &Self::for_source(source, range)? {
            Ok(())
        } else {
            Err(ClipError::OutputInvalid)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoRange {
    pub start_ticks: u64,
    pub end_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoMetadata {
    pub duration_ticks: u64,
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub has_audio: bool,
}
impl VideoMetadata {
    pub fn validate(&self) -> Result<(), ClipError> {
        if self.duration_ticks == 0
            || self.duration_ticks > MAX_DURATION_TICKS
            || self.width < 2
            || self.height < 2
            || self.width > 16_384
            || self.height > 16_384
            || u64::from(self.width) * u64::from(self.height) > MAX_SOURCE_PIXELS
            || self.frame_rate_numerator == 0
            || self.frame_rate_denominator == 0
        {
            return Err(ClipError::InvalidMedia);
        }
        let fps = self.frame_rate_numerator as f64 / self.frame_rate_denominator as f64;
        if !(0.1..=240.0).contains(&fps) {
            return Err(ClipError::InvalidMedia);
        }
        Ok(())
    }

    /// None always means the complete actual source, including a short source.
    /// Full explicit range is canonicalized to None by the Host/domain before
    /// invoking render. Renderer accepts only a real, non-full retained range.
    pub fn validate_range(&self, range: Option<&VideoRange>) -> Result<(), ClipError> {
        self.validate()?;
        if let Some(range) = range {
            if self.duration_ticks < MIN_RANGE_TICKS
                || range.end_ticks > self.duration_ticks
                || range
                    .end_ticks
                    .checked_sub(range.start_ticks)
                    .is_none_or(|n| n < MIN_RANGE_TICKS)
            {
                return Err(ClipError::InvalidRange);
            }
        }
        Ok(())
    }
}

/// Host-only API payload; these paths are never frontend IPC arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipJob {
    Inspect {
        source: PathBuf,
    },
    Render {
        source: PathBuf,
        output: PathBuf,
        expected: VideoMetadata,
        range: VideoRange,
    },
    RenderGif {
        source: PathBuf,
        output: PathBuf,
        expected: VideoMetadata,
        range: Option<VideoRange>,
    },
    ExtractFrames {
        source: PathBuf,
        output_directory: PathBuf,
        expected: VideoMetadata,
        window: VideoRange,
        requested_ticks: Vec<u64>,
    },
    RenderAnnotated {
        source: PathBuf,
        output: PathBuf,
        expected: VideoMetadata,
        range: Option<VideoRange>,
        overlays: Vec<OverlayRaster>,
    },
    RenderGifAnnotated {
        source: PathBuf,
        output: PathBuf,
        expected: VideoMetadata,
        range: Option<VideoRange>,
        overlays: Vec<OverlayRaster>,
    },
}
impl ClipJob {
    pub fn validate(&self) -> Result<(), ClipError> {
        let source = match self {
            Self::Inspect { source } => source,
            Self::RenderAnnotated {
                source,
                output,
                expected,
                range,
                overlays,
            }
            | Self::RenderGifAnnotated {
                source,
                output,
                expected,
                range,
                overlays,
            } => {
                validate_overlay_plan(expected, range.as_ref(), overlays)?;
                if !output.is_absolute() || output == source {
                    return Err(ClipError::UnsupportedPath);
                }
                source
            }
            Self::ExtractFrames {
                source,
                output_directory,
                expected,
                window,
                requested_ticks,
            } => {
                validate_extraction_request(expected, window, requested_ticks)?;
                if !output_directory.is_absolute() || output_directory == source {
                    return Err(ClipError::UnsupportedPath);
                }
                source
            }
            Self::RenderGif {
                source,
                output,
                expected,
                range,
            } => {
                expected.validate_range(range.as_ref())?;
                if !output.is_absolute() || output == source {
                    return Err(ClipError::UnsupportedPath);
                }
                source
            }
            Self::Render {
                source,
                output,
                expected,
                range,
            } => {
                expected.validate_range(Some(range))?;
                if range.start_ticks == 0 && range.end_ticks == expected.duration_ticks {
                    return Err(ClipError::InvalidRange);
                }
                if !output.is_absolute() || output == source {
                    return Err(ClipError::UnsupportedPath);
                }
                source
            }
        };
        if !source.is_absolute() {
            return Err(ClipError::UnsupportedPath);
        }
        Ok(())
    }
}

/// Container padding may add one frame/AAC tail; it must not change source
/// geometry, frame cadence, or audio presence. This is not byte-identical audio.
pub fn validate_rendered(
    source: &VideoMetadata,
    range: &VideoRange,
    output: &VideoMetadata,
) -> Result<(), ClipError> {
    source.validate_range(Some(range))?;
    output.validate().map_err(|_| ClipError::OutputInvalid)?;
    let expected = range.end_ticks - range.start_ticks;
    let frame_ticks = (10_000_000_u64 * u64::from(source.frame_rate_denominator))
        .div_ceil(u64::from(source.frame_rate_numerator));
    let tolerance = MIN_RANGE_TICKS.max(frame_ticks + 500_000);
    let source_fps =
        f64::from(source.frame_rate_numerator) / f64::from(source.frame_rate_denominator);
    let output_fps =
        f64::from(output.frame_rate_numerator) / f64::from(output.frame_rate_denominator);
    if output.duration_ticks.abs_diff(expected) > tolerance
        || output.width != source.width
        || output.height != source.height
        || output.has_audio != source.has_audio
        || (output_fps - source_fps).abs() > 0.001
    {
        return Err(ClipError::OutputInvalid);
    }
    Ok(())
}

/// Annotated MF export proves actual sample count/cadence after Finalize in the
/// worker. MediaClip's rate can be a whole-file average (including a short last
/// sample), so this terminal/independent Inspect check preserves shape, audio
/// and duration only. It is not a replacement for native sample verification.
/// Legacy MediaComposition output keeps its original stricter validator above.
pub fn validate_annotated_rendered(
    source: &VideoMetadata,
    range: &VideoRange,
    output: &VideoMetadata,
) -> Result<(), ClipError> {
    source.validate_range(Some(range))?;
    output.validate().map_err(|_| ClipError::OutputInvalid)?;
    let (num, den) = annotated_output_cadence(source)?;
    let frame_ticks = (10_000_000_u64 * u64::from(den)).div_ceil(u64::from(num));
    let tolerance = MIN_RANGE_TICKS.max(frame_ticks + 500_000);
    if output
        .duration_ticks
        .abs_diff(range.end_ticks - range.start_ticks)
        > tolerance
        || output.width != source.width
        || output.height != source.height
        || output.has_audio != source.has_audio
    {
        return Err(ClipError::OutputInvalid);
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub version: u32,
    pub job: ClipJob,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Cancel {},
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerEvent {
    Ready { version: u32 },
    Progress { percent: u8 },
    Inspected { metadata: VideoMetadata },
    Rendered { metadata: VideoMetadata },
    GifRendered { metadata: GifMetadata },
    FramesExtracted { frames: Vec<FrameDescriptor> },
    Cancelled {},
    Failed { error: ClipError },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClipJobResult {
    Inspected(VideoMetadata),
    Rendered(VideoMetadata),
    GifRendered(GifMetadata),
    FramesExtracted { frames: Vec<FrameDescriptor> },
}
impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "当前系统不支持视频裁剪",
            Self::Busy => "视频正在处理中",
            Self::Cancelled => "视频处理已取消",
            Self::TimedOut => "视频处理超时",
            Self::CleanupTimedOut => "视频处理尚未停止，请稍后重试",
            Self::InvalidMedia => "视频格式或尺寸不受支持",
            Self::InvalidRange => "裁剪范围无效",
            Self::SourceChanged => "视频来源已变化，请重新打开",
            Self::UnsupportedPath => "当前视频路径不受支持",
            Self::EncodingFailed => "视频裁剪失败",
            Self::OutputInvalid => "裁剪结果校验失败",
            Self::FileUnavailable => "无法读取或保存视频文件",
            Self::Protocol | Self::WorkerUnavailable => "视频处理不可用",
        })
    }
}
impl std::error::Error for ClipError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ClipError {
    Unsupported,
    Busy,
    Cancelled,
    TimedOut,
    CleanupTimedOut,
    InvalidMedia,
    InvalidRange,
    SourceChanged,
    UnsupportedPath,
    EncodingFailed,
    OutputInvalid,
    FileUnavailable,
    Protocol,
    WorkerUnavailable,
}
impl From<ClipError> for String {
    fn from(error: ClipError) -> Self {
        match error {
            ClipError::Unsupported => "unsupported",
            ClipError::Busy => "busy",
            ClipError::Cancelled => "cancelled",
            ClipError::TimedOut => "timed_out",
            ClipError::CleanupTimedOut => "cleanup_timed_out",
            ClipError::InvalidMedia => "invalid_media",
            ClipError::InvalidRange => "invalid_range",
            ClipError::SourceChanged => "source_changed",
            ClipError::UnsupportedPath => "unsupported_path",
            ClipError::EncodingFailed => "encoding_failed",
            ClipError::OutputInvalid => "output_invalid",
            ClipError::FileUnavailable => "file_unavailable",
            ClipError::Protocol => "protocol",
            ClipError::WorkerUnavailable => "worker_unavailable",
        }
        .into()
    }
}
impl TryFrom<String> for ClipError {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "unsupported" => Ok(Self::Unsupported),
            "busy" => Ok(Self::Busy),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "cleanup_timed_out" => Ok(Self::CleanupTimedOut),
            "invalid_media" => Ok(Self::InvalidMedia),
            "invalid_range" => Ok(Self::InvalidRange),
            "source_changed" => Ok(Self::SourceChanged),
            "unsupported_path" => Ok(Self::UnsupportedPath),
            "encoding_failed" => Ok(Self::EncodingFailed),
            "output_invalid" => Ok(Self::OutputInvalid),
            "file_unavailable" => Ok(Self::FileUnavailable),
            "protocol" => Ok(Self::Protocol),
            "worker_unavailable" => Ok(Self::WorkerUnavailable),
            _ => Err("unknown video clip error".into()),
        }
    }
}

// Public worker signatures (no Host/App/Engine dependency):
// pub async fn run(job: ClipJob, cancel: Arc<AtomicBool>,
//   on_progress: Arc<dyn Fn(u8) + Send + Sync>) -> Result<ClipJobResult, ClipError>;
// pub fn busy() -> bool;
// pub fn cancel_all();
// pub async fn wait_until_idle(timeout: Duration) -> bool;
// pub fn worker_main() -> i32;  // exact --video-clip-worker before Tauri/DB
//
// Backend is synchronous in the owned MTA child. The parent performs deadlines
// and Job recovery; the child polls WinRT async status and calls Cancel once,
// retaining composition/source/output references until native terminal status.
// Full range stays a Host exact-byte copy path; it never invokes Render.

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(duration_ticks: u64) -> VideoMetadata {
        VideoMetadata {
            duration_ticks,
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            has_audio: false,
        }
    }
    #[test]
    fn boundaries_are_actual_ticks_not_recording_wall_clock() {
        let actual = metadata(6_000_139_455);
        assert!(actual.validate_range(None).is_ok());
        assert!(actual
            .validate_range(Some(&VideoRange {
                start_ticks: 6_000_000_000,
                end_ticks: 6_000_139_455
            }))
            .is_err());
        assert!(actual
            .validate_range(Some(&VideoRange {
                start_ticks: 5_999_139_455,
                end_ticks: 6_000_139_455
            }))
            .is_ok());
        assert!(metadata(999_999).validate_range(None).is_ok());
        assert!(metadata(999_999)
            .validate_range(Some(&VideoRange {
                start_ticks: 0,
                end_ticks: 999_999
            }))
            .is_err());
        assert!(metadata(MAX_DURATION_TICKS).validate().is_ok());
        assert!(metadata(MAX_DURATION_TICKS + 1).validate().is_err());
    }
    #[test]
    fn extracted_receipt_binds_each_requested_time_jpeg_identity_and_aggregate_budget() {
        let source = metadata(MAX_DURATION_TICKS);
        let window = VideoRange {
            start_ticks: 0,
            end_ticks: source.duration_ticks,
        };
        let requested = [2_150_000, 17_990_000_000];
        let good = requested
            .iter()
            .enumerate()
            .map(|(index, &tick)| FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: tick - 150_000,
                sample_duration_ticks: 333_333,
                encoded_width: 320,
                encoded_height: 180,
                jpeg_sha256: "a".repeat(64),
                bytes_length: 1000,
            })
            .collect::<Vec<_>>();
        assert!(validate_frame_descriptors(&source, &window, &requested, &good).is_ok());
        for mutate in 0..8 {
            let mut bad = good.clone();
            match mutate {
                0 => bad[1].index = 0,
                1 => bad[1].requested_ticks -= 1,
                2 => bad[0].actual_pts_ticks = bad[0].requested_ticks + 1,
                3 => bad[1].sample_duration_ticks = 0,
                4 => bad[1].encoded_width = 319,
                5 => bad[1].bytes_length = MAX_FRAME_JPEG_BYTES + 1,
                6 => bad[1].jpeg_sha256 = "A".repeat(64),
                _ => bad[1].sample_duration_ticks = MAX_DURATION_TICKS + 1,
            }
            assert!(
                validate_frame_descriptors(&source, &window, &requested, &bad).is_err(),
                "mutation {mutate}"
            );
        }
        assert!(validate_frame_descriptors(&source, &window, &requested, &good[..1]).is_err());
        let six_requested = (0..6u64).map(|i| i * 1_000_000).collect::<Vec<_>>();
        let six_frames = six_requested
            .iter()
            .enumerate()
            .map(|(index, &tick)| FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: tick,
                sample_duration_ticks: 333_333,
                encoded_width: 320,
                encoded_height: 180,
                jpeg_sha256: "b".repeat(64),
                bytes_length: MAX_FRAME_JPEG_BYTES,
            })
            .collect::<Vec<_>>();
        assert!(validate_frame_descriptors(&source, &window, &six_requested, &six_frames).is_err());
        assert!(
            serde_json::to_vec(&WorkerEvent::FramesExtracted { frames: good })
                .unwrap()
                .len()
                < 4096
        );
    }
    #[test]
    fn distinct_logical_requests_may_reference_one_held_source_frame() {
        let mut source = metadata(2_000_000);
        source.frame_rate_numerator = 15;
        source.frame_rate_denominator = 1;
        let window = VideoRange {
            start_ticks: 0,
            end_ticks: 1_000_000,
        };
        let frames = [0, 500_000]
            .into_iter()
            .enumerate()
            .map(|(index, tick)| FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: 0,
                sample_duration_ticks: 666_666,
                encoded_width: 320,
                encoded_height: 180,
                jpeg_sha256: "b".repeat(64),
                bytes_length: 1000,
            })
            .collect::<Vec<_>>();
        assert!(validate_frame_descriptors(&source, &window, &[0, 500_000], &frames).is_ok());
        assert!(validate_frame_descriptors(&source, &window, &[0, 0], &frames).is_err());
    }
    #[test]
    fn annotated_gif_preserves_short_intervals_in_long_source_and_rejects_unrepresentable_boundaries(
    ) {
        let source = metadata(MAX_DURATION_TICKS);
        let entry = OverlayRaster {
            png: std::env::temp_dir().join("host.png"),
            png_sha256: "a".repeat(64),
            width: 12,
            height: 10,
            bounds: OverlayBounds {
                x: 20.,
                y: 10.,
                width: 12.,
                height: 10.,
            },
            interval: VideoRange {
                start_ticks: 9_000_215_000,
                end_ticks: 9_001_215_000,
            },
        };
        let plan = AnnotatedGifSchedule::new(&source, None, &[entry.clone()]).unwrap();
        assert!(plan.requested_ticks.contains(&entry.interval.start_ticks));
        assert!(plan.requested_ticks.contains(&entry.interval.end_ticks));
        assert_eq!(
            plan.delays.iter().map(|&n| u32::from(n)).sum::<u32>(),
            180_000
        );
        assert!(plan.metadata.frame_count <= 240);
        let mut close = entry.clone();
        close.interval.start_ticks += 10_000;
        close.interval.end_ticks += 10_000;
        assert_eq!(
            AnnotatedGifSchedule::new(&source, None, &[entry, close]).unwrap_err(),
            ClipError::InvalidRange
        );
    }
    #[test]
    fn wire_is_strict_even_for_empty_variants() {
        assert!(serde_json::from_str::<Control>(r#"{"type":"cancel","extra":1}"#).is_err());
        assert!(serde_json::from_str::<WorkerEvent>(r#"{"type":"cancelled","extra":1}"#).is_err());
        assert!(serde_json::from_str::<ClipError>(r#"{"busy":null}"#).is_err());
        assert!(serde_json::from_str::<GifMetadata>(
            r#"{"width":320,"height":180,"frameCount":90,"durationCentiseconds":600,"audio":true}"#
        )
        .is_err());
    }
    #[test]
    fn gif_report_is_bound_to_exact_range_and_fixed_budget() {
        let source = metadata(60_000_000);
        let expected = GifMetadata::for_source(&source, None).unwrap();
        assert_eq!(
            expected,
            GifMetadata {
                width: 320,
                height: 180,
                frame_count: 90,
                duration_centiseconds: 600
            }
        );
        assert!(expected.validate_for(&source, None).is_ok());
        assert!(expected
            .validate_for(
                &source,
                Some(&VideoRange {
                    start_ticks: 12_500_000,
                    end_ticks: 42_500_000
                })
            )
            .is_err());
        for invalid in [
            GifMetadata {
                frame_count: 241,
                ..expected.clone()
            },
            GifMetadata {
                duration_centiseconds: 599,
                ..expected.clone()
            },
            GifMetadata {
                width: 321,
                ..expected.clone()
            },
        ] {
            assert!(invalid.validate_for(&source, None).is_err());
        }
    }
    #[test]
    fn annotated_inspection_is_shape_audio_duration_while_legacy_keeps_rate() {
        let source = VideoMetadata {
            frame_rate_numerator: 30_000,
            frame_rate_denominator: 1001,
            ..metadata(20_000_000)
        };
        let range = VideoRange {
            start_ticks: 2_150_000,
            end_ticks: 11_150_000,
        };
        let output = VideoMetadata {
            duration_ticks: 9_000_000,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            ..source.clone()
        };
        assert_eq!(
            validate_rendered(&source, &range, &output),
            Err(ClipError::OutputInvalid)
        );
        assert!(validate_annotated_rendered(&source, &range, &output).is_ok());
        for invalid in [
            VideoMetadata {
                width: 640,
                ..output.clone()
            },
            VideoMetadata {
                has_audio: !source.has_audio,
                ..output.clone()
            },
            VideoMetadata {
                duration_ticks: 10_000_001,
                ..output.clone()
            },
            VideoMetadata {
                frame_rate_denominator: 0,
                ..output.clone()
            },
        ] {
            assert_eq!(
                validate_annotated_rendered(&source, &range, &invalid),
                Err(ClipError::OutputInvalid)
            );
        }
    }
    #[test]
    fn output_validation_preserves_cadence_audio_and_rejects_wrong_interval() {
        let source = metadata(60_000_000);
        let range = VideoRange {
            start_ticks: 12_500_000,
            end_ticks: 42_500_000,
        };
        let output = metadata(30_185_714);
        assert!(validate_rendered(&source, &range, &output).is_ok());
        for invalid in [
            VideoMetadata {
                has_audio: true,
                ..output.clone()
            },
            VideoMetadata {
                width: 640,
                ..output.clone()
            },
            VideoMetadata {
                frame_rate_numerator: 25,
                ..output.clone()
            },
            VideoMetadata {
                duration_ticks: 31_000_001,
                ..output.clone()
            },
        ] {
            assert_eq!(
                validate_rendered(&source, &range, &invalid),
                Err(ClipError::OutputInvalid)
            );
        }
        assert!(validate_rendered(
            &source,
            &VideoRange {
                start_ticks: 50_000_000,
                end_ticks: 40_000_000
            },
            &output
        )
        .is_err());
    }
}
