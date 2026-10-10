// SPDX-License-Identifier: MPL-2.0
//! Bounded source-frame extraction and shared native video raster composition.
//! A decoded sample PTS is evidence, never an annotation lifecycle clock.
use crate::video_clip_contract::{
    self, ClipError, FrameDescriptor, VideoMetadata, VideoRange, MAX_FRAME_JPEG_BYTES,
    MAX_FRAME_TOTAL_BYTES,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourcePlaybackTicks(pub u64);

/// Native decoded frame plus two deliberately distinct clocks. The caller
/// keeps its immutable source lease/fence alive through extraction and commit.
pub struct ExtractedFrame {
    pub source_playback_ticks: SourcePlaybackTicks,
    pub actual_pts_ticks: u64,
    pub sample_duration_ticks: u64,
    pub next_actual_pts_ticks: Option<u64>,
    pub rgba: image::RgbaImage,
}

fn validate_frame_request(
    expected: &VideoMetadata,
    window: &VideoRange,
    requested: &[SourcePlaybackTicks],
) -> Result<(), ClipError> {
    video_clip_contract::validate_extraction_request(
        expected,
        window,
        &requested.iter().map(|t| t.0).collect::<Vec<_>>(),
    )
}

struct LimitedJpeg {
    bytes: Vec<u8>,
}
impl Write for LimitedJpeg {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > MAX_FRAME_JPEG_BYTES.saturating_sub(self.bytes.len() as u64) {
            return Err(std::io::Error::other("frame JPEG budget"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct CreatedFrames {
    paths: Vec<PathBuf>,
    committed: bool,
}
impl Drop for CreatedFrames {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Called inside the existing supervised MTA child under source and staging
/// directory leases. Only native ordinal JPEG names are created; no metadata
/// sidecar, path or giant pixels are placed on the result pipe.
pub fn extract_frames_to_files(
    source: &Path,
    output_directory: &Path,
    expected: &VideoMetadata,
    window: &VideoRange,
    requested_ticks: &[u64],
    cancel: &AtomicBool,
) -> Result<Vec<FrameDescriptor>, ClipError> {
    crate::video_clip_backend::check_cancel(cancel)?;
    video_clip_contract::validate_extraction_request(expected, window, requested_ticks)?;
    if !source.is_absolute() || !output_directory.is_absolute() {
        return Err(ClipError::UnsupportedPath);
    }
    if fs::read_dir(output_directory)
        .map_err(|_| ClipError::FileUnavailable)?
        .next()
        .is_some()
    {
        return Err(ClipError::FileUnavailable);
    }
    #[cfg(windows)]
    let _directory = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000)
            .open(output_directory)
            .map_err(|_| ClipError::FileUnavailable)?
    };
    let requested = requested_ticks
        .iter()
        .copied()
        .map(SourcePlaybackTicks)
        .collect::<Vec<_>>();
    let decoded = extract_frames(source, expected, window, &requested, cancel)?;
    let (width, height) = video_clip_contract::extracted_dimensions(expected);
    let mut created = CreatedFrames {
        paths: Vec::new(),
        committed: false,
    };
    let mut descriptors = Vec::with_capacity(decoded.len());
    let mut total = 0u64;
    for (index, frame) in decoded.into_iter().enumerate() {
        crate::video_clip_backend::check_cancel(cancel)?;
        let image = if frame.rgba.dimensions() == (width, height) {
            frame.rgba
        } else {
            image::imageops::resize(
                &frame.rgba,
                width,
                height,
                image::imageops::FilterType::Triangle,
            )
        };
        let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
        let mut output = LimitedJpeg { bytes: Vec::new() };
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 95)
            .encode_image(&rgb)
            .map_err(|_| ClipError::OutputInvalid)?;
        total = total
            .checked_add(output.bytes.len() as u64)
            .ok_or(ClipError::OutputInvalid)?;
        if total > MAX_FRAME_TOTAL_BYTES {
            return Err(ClipError::OutputInvalid);
        }
        crate::video_clip_backend::check_cancel(cancel)?;
        let path = output_directory.join(format!("{index}.jpg"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| ClipError::FileUnavailable)?;
        created.paths.push(path);
        file.write_all(&output.bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| ClipError::FileUnavailable)?;
        descriptors.push(FrameDescriptor {
            index: index as u32,
            requested_ticks: frame.source_playback_ticks.0,
            actual_pts_ticks: frame.actual_pts_ticks,
            sample_duration_ticks: frame.sample_duration_ticks,
            encoded_width: width,
            encoded_height: height,
            jpeg_sha256: format!("{:x}", Sha256::digest(&output.bytes)),
            bytes_length: output.bytes.len() as u64,
        });
    }
    video_clip_contract::validate_frame_descriptors(
        expected,
        window,
        requested_ticks,
        &descriptors,
    )?;
    validate_extracted_files(
        output_directory,
        expected,
        window,
        requested_ticks,
        &descriptors,
        cancel,
    )?;
    crate::video_clip_backend::check_cancel(cancel)?;
    created.committed = true;
    Ok(descriptors)
}

/// Parent-side independent read/hash/decode after actual child/Job/IO exit.
/// The supervisor retains its real permit for the entire bounded validation.
pub fn validate_extracted_files(
    directory: &Path,
    expected: &VideoMetadata,
    window: &VideoRange,
    requested_ticks: &[u64],
    frames: &[FrameDescriptor],
    cancel: &AtomicBool,
) -> Result<(), ClipError> {
    crate::video_clip_backend::check_cancel(cancel)?;
    video_clip_contract::validate_frame_descriptors(expected, window, requested_ticks, frames)?;
    let names = frames
        .iter()
        .map(|f| format!("{}.jpg", f.index))
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen = std::collections::BTreeSet::new();
    for entry in fs::read_dir(directory).map_err(|_| ClipError::FileUnavailable)? {
        let entry = entry.map_err(|_| ClipError::FileUnavailable)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ClipError::OutputInvalid)?;
        if !names.contains(&name)
            || !entry
                .file_type()
                .map_err(|_| ClipError::FileUnavailable)?
                .is_file()
        {
            return Err(ClipError::OutputInvalid);
        }
        seen.insert(name);
    }
    if seen != names {
        return Err(ClipError::OutputInvalid);
    }
    for frame in frames {
        crate::video_clip_backend::check_cancel(cancel)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options
            .open(directory.join(format!("{}.jpg", frame.index)))
            .map_err(|_| ClipError::FileUnavailable)?;
        if file
            .metadata()
            .map_err(|_| ClipError::FileUnavailable)?
            .len()
            != frame.bytes_length
        {
            return Err(ClipError::OutputInvalid);
        }
        let mut bytes = Vec::with_capacity(frame.bytes_length as usize);
        Read::by_ref(&mut file)
            .take(MAX_FRAME_JPEG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ClipError::FileUnavailable)?;
        if bytes.len() as u64 != frame.bytes_length
            || bytes.len() < 4
            || !bytes.starts_with(&[0xff, 0xd8])
            || !bytes.ends_with(&[0xff, 0xd9])
            || format!("{:x}", Sha256::digest(&bytes)) != frame.jpeg_sha256
        {
            return Err(ClipError::OutputInvalid);
        }
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|_| ClipError::OutputInvalid)?;
        if (decoded.width(), decoded.height()) != (frame.encoded_width, frame.encoded_height) {
            return Err(ClipError::OutputInvalid);
        }
        crate::video_clip_backend::check_cancel(cancel)?;
    }
    Ok(())
}

/// At most six independent bounded keyframe seeks with current/next samples. The
/// logical requested time labels the captured playback position; the actual
/// source sample PTS/duration independently identify the image. No provider or
/// task-generated tick is accepted at this native-only entry point.
pub fn extract_frames(
    source: &Path,
    expected: &VideoMetadata,
    window: &VideoRange,
    requested: &[SourcePlaybackTicks],
    cancel: &AtomicBool,
) -> Result<Vec<ExtractedFrame>, ClipError> {
    crate::video_clip_backend::check_cancel(cancel)?;
    validate_frame_request(expected, window, requested)?;
    #[cfg(windows)]
    {
        use crate::video_gif_backend::source_stream::Reader;
        let convert = |error: Box<dyn std::error::Error + Send + Sync>| {
            error
                .downcast_ref::<ClipError>()
                .copied()
                .unwrap_or(ClipError::InvalidMedia)
        };
        let started = std::time::Instant::now();
        let check = || {
            crate::video_clip_backend::check_cancel(cancel)?;
            if started.elapsed() >= std::time::Duration::from_secs(30) {
                return Err(ClipError::TimedOut);
            }
            Ok(())
        };
        let nominal_frame = (10_000_000u64 * u64::from(expected.frame_rate_denominator))
            .div_ceil(u64::from(expected.frame_rate_numerator));
        let mut output = Vec::with_capacity(requested.len());
        for &tick in requested {
            check()?;
            let mut reader =
                Reader::open(source, expected.width, expected.height).map_err(convert)?;
            reader
                .seek_once(tick.0.saturating_sub(nominal_frame.saturating_mul(2)), 240)
                .map_err(convert)?;
            check()?;
            let mut current = reader
                .next(cancel)
                .map_err(convert)?
                .ok_or(ClipError::InvalidMedia)?;
            if current.pts > tick.0 {
                return Err(ClipError::InvalidRange);
            }
            let mut next = reader.next(cancel).map_err(convert)?;
            while next.as_ref().is_some_and(|frame| frame.pts <= tick.0) {
                check()?;
                current = next.take().unwrap();
                next = reader.next(cancel).map_err(convert)?;
            }
            check()?;
            if current.pts >= expected.duration_ticks
                || next.is_none()
                    && current
                        .pts
                        .checked_add(current.duration)
                        .is_none_or(|end| tick.0 >= end)
            {
                return Err(ClipError::InvalidRange);
            }
            let rgba = reader.rgba(&current, cancel).map_err(convert)?;
            output.push(ExtractedFrame {
                source_playback_ticks: tick,
                actual_pts_ticks: current.pts,
                sample_duration_ticks: current.duration,
                next_actual_pts_ticks: next.as_ref().map(|frame| frame.pts),
                rgba,
            });
        }
        check()?;
        Ok(output)
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        Err(ClipError::Unsupported)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectedInterval {
    pub source_start: SourcePlaybackTicks,
    pub source_end: SourcePlaybackTicks,
    pub delay_ticks: u64,
    pub duration_ticks: u64,
}

/// Host-generated immutable raster leases. Original source time remains
/// separate from the decoded frame PTS, including trim and held frames.
pub(crate) struct RasterPlan {
    entries: Vec<(
        crate::video_clip_contract::OverlayRaster,
        image::RgbaImage,
        std::fs::File,
    )>,
}
impl RasterPlan {
    pub(crate) fn load(
        source: &VideoMetadata,
        range: Option<&VideoRange>,
        entries: &[crate::video_clip_contract::OverlayRaster],
        cancel: &AtomicBool,
    ) -> Result<Self, ClipError> {
        use crate::video_clip_contract::{
            validate_overlay_plan, MAX_OVERLAY_BYTES, MAX_OVERLAY_TOTAL_BYTES,
        };
        use sha2::{Digest, Sha256};
        use std::io::Read;
        validate_overlay_plan(source, range, entries)?;
        let mut total = 0u64;
        let mut output = Vec::with_capacity(entries.len());
        for entry in entries {
            crate::video_clip_backend::check_cancel(cancel)?;
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(1);
            }
            let mut file = options
                .open(&entry.png)
                .map_err(|_| ClipError::FileUnavailable)?;
            let meta = file.metadata().map_err(|_| ClipError::FileUnavailable)?;
            total = total
                .checked_add(meta.len())
                .ok_or(ClipError::InvalidMedia)?;
            if !meta.is_file()
                || !(24..=MAX_OVERLAY_BYTES).contains(&meta.len())
                || total > MAX_OVERLAY_TOTAL_BYTES
            {
                return Err(ClipError::InvalidMedia);
            }
            let mut bytes = Vec::with_capacity(meta.len() as usize);
            Read::by_ref(&mut file)
                .take(MAX_OVERLAY_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ClipError::FileUnavailable)?;
            if bytes.len() as u64 != meta.len()
                || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
                || format!("{:x}", Sha256::digest(&bytes)) != entry.png_sha256
            {
                return Err(ClipError::SourceChanged);
            }
            let dimensions = image::ImageReader::with_format(
                std::io::Cursor::new(&bytes),
                image::ImageFormat::Png,
            )
            .into_dimensions()
            .map_err(|_| ClipError::InvalidMedia)?;
            if dimensions != (entry.width, entry.height) {
                return Err(ClipError::SourceChanged);
            }
            let mut decoder = image::ImageReader::with_format(
                std::io::Cursor::new(&bytes),
                image::ImageFormat::Png,
            );
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(entry.width);
            limits.max_image_height = Some(entry.height);
            limits.max_alloc = Some(64 * 1024 * 1024);
            decoder.limits(limits);
            let rgba = decoder
                .decode()
                .map_err(|_| ClipError::InvalidMedia)?
                .into_rgba8();
            if rgba.dimensions() != dimensions {
                return Err(ClipError::SourceChanged);
            }
            output.push((entry.clone(), rgba, file));
        }
        crate::video_clip_backend::check_cancel(cancel)?;
        Ok(Self { entries: output })
    }
    pub(crate) fn paint(
        &self,
        rgba: &mut image::RgbaImage,
        time: SourcePlaybackTicks,
        cancel: &AtomicBool,
    ) -> Result<(), ClipError> {
        for (entry, pixels, _lease) in &self.entries {
            crate::video_clip_backend::check_cancel(cancel)?;
            if entry.interval.start_ticks <= time.0 && time.0 < entry.interval.end_ticks {
                let b = entry.bounds;
                crate::raster_compositor::paint_raster(rgba, pixels, b.x, b.y, b.width, b.height)
                    .map_err(|_| ClipError::InvalidMedia)?;
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) struct PlaybackFrames {
    reader: crate::video_gif_backend::source_stream::Reader,
    current: crate::video_gif_backend::source_stream::Frame,
    next: Option<crate::video_gif_backend::source_stream::Frame>,
    duration: u64,
    last_requested: Option<u64>,
}
#[cfg(windows)]
fn media_error(error: Box<dyn std::error::Error + Send + Sync>) -> ClipError {
    error
        .downcast_ref::<ClipError>()
        .copied()
        .unwrap_or(ClipError::InvalidMedia)
}
#[cfg(windows)]
impl PlaybackFrames {
    pub(crate) fn open(
        source: &Path,
        expected: &VideoMetadata,
        start: u64,
        cancel: &AtomicBool,
    ) -> Result<Self, ClipError> {
        let mut reader = crate::video_gif_backend::source_stream::Reader::open(
            source,
            expected.width,
            expected.height,
        )
        .map_err(media_error)?;
        let nominal = (10_000_000u64 * u64::from(expected.frame_rate_denominator))
            .div_ceil(u64::from(expected.frame_rate_numerator));
        reader
            .seek_for_export(start.saturating_sub(nominal.saturating_mul(2)))
            .map_err(media_error)?;
        let current = reader
            .next(cancel)
            .map_err(media_error)?
            .ok_or(ClipError::InvalidMedia)?;
        if current.pts > start {
            return Err(ClipError::InvalidRange);
        }
        let next = reader.next(cancel).map_err(media_error)?;
        Ok(Self {
            reader,
            current,
            next,
            duration: expected.duration_ticks,
            last_requested: None,
        })
    }
    pub(crate) fn rgba_at(
        &mut self,
        time: SourcePlaybackTicks,
        cancel: &AtomicBool,
    ) -> Result<image::RgbaImage, ClipError> {
        if time.0 >= self.duration || self.last_requested.is_some_and(|last| time.0 < last) {
            return Err(ClipError::InvalidRange);
        }
        while self.next.as_ref().is_some_and(|next| next.pts <= time.0) {
            self.current = self.next.take().unwrap();
            self.next = self.reader.next(cancel).map_err(media_error)?;
        }
        if self.current.pts > time.0
            || self.next.is_none()
                && self
                    .current
                    .pts
                    .checked_add(self.current.duration)
                    .is_none_or(|end| time.0 >= end)
        {
            return Err(ClipError::InvalidRange);
        }
        self.last_requested = Some(time.0);
        self.reader.rgba(&self.current, cancel).map_err(media_error)
    }
}

#[cfg(windows)]
pub fn render_mp4(
    source: &Path,
    output: &Path,
    expected: &VideoMetadata,
    range: Option<&VideoRange>,
    overlays: &[crate::video_clip_contract::OverlayRaster],
    pixel_aspect: (u32, u32),
    bitrate: u32,
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<(), ClipError> {
    mf_composition::render(
        source,
        output,
        expected,
        range,
        overlays,
        pixel_aspect,
        bitrate,
        cancel,
        progress,
    )
}

#[cfg(windows)]
mod mf_composition {
    use super::*;
    use windows::{
        core::{GUID, HSTRING, PCWSTR},
        Win32::Media::MediaFoundation::*,
    };
    #[track_caller]
    fn native<T>(result: windows::core::Result<T>) -> Result<T, ClipError> {
        #[cfg(test)]
        let location = std::panic::Location::caller();
        result.map_err(|error| {
            #[cfg(test)]
            eprintln!(
                "synthetic native media HRESULT {:08x} at line {}",
                error.code().0 as u32,
                location.line()
            );
            let _ = error;
            ClipError::EncodingFailed
        })
    }
    struct Mf;
    impl Drop for Mf {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
            }
        }
    }
    pub(crate) fn cadence(index: u64, source: &VideoMetadata) -> u64 {
        (u128::from(index) * 10_000_000 * u128::from(source.frame_rate_denominator)
            / u128::from(source.frame_rate_numerator)) as u64
    }
    /// This writer's explicit milli-fps format produces an MP4 video track at
    /// that integer time scale. Header duration truncates the original end;
    /// sample durations are rounded to the nearest unit. Keep both values: the
    /// final sample may end one native unit after mdhd's declared duration.
    #[derive(Clone, Copy, Debug)]
    struct TrackClock {
        scale: u32,
        regular_units: u64,
        tail_units: u64,
        declared_units: u64,
        count: u64,
    }
    impl TrackClock {
        fn planned(info: &VideoMetadata, duration: u64) -> Result<Self, ClipError> {
            if info.frame_rate_numerator == 0
                || info.frame_rate_denominator != 1000
                || duration == 0
                || duration > video_clip_contract::MAX_DURATION_TICKS
            {
                return Err(ClipError::InvalidRange);
            }
            let count = (u128::from(duration) * u128::from(info.frame_rate_numerator))
                .div_ceil(10_000_000 * u128::from(info.frame_rate_denominator))
                as u64;
            let last = cadence(count - 1, info);
            let scale = info.frame_rate_numerator;
            let tail_units =
                ((u128::from(duration - last) * u128::from(scale) + 5_000_000) / 10_000_000) as u64;
            if count > 432_000 || tail_units == 0 || tail_units > 1000 {
                return Err(ClipError::InvalidRange);
            }
            Ok(Self {
                scale,
                regular_units: 1000,
                tail_units,
                declared_units: (u128::from(duration) * u128::from(scale) / 10_000_000) as u64,
                count,
            })
        }
        fn duration_ticks(&self, last: bool) -> u64 {
            let units = if last {
                self.tail_units
            } else {
                self.regular_units
            };
            (u128::from(units) * 10_000_000 / u128::from(self.scale)) as u64
        }
    }
    #[derive(Clone, Copy)]
    struct IsoBox {
        kind: [u8; 4],
        body: u64,
        end: u64,
    }
    fn read_iso_bytes<const N: usize>(
        file: &mut fs::File,
        offset: u64,
    ) -> Result<[u8; N], ClipError> {
        use std::io::{Read, Seek, SeekFrom};
        let mut bytes = [0; N];
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut bytes))
            .map_err(|_| ClipError::OutputInvalid)?;
        Ok(bytes)
    }
    fn iso_children(
        file: &mut fs::File,
        start: u64,
        end: u64,
        budget: &mut u32,
        cancel: &AtomicBool,
    ) -> Result<Vec<IsoBox>, ClipError> {
        let mut boxes = Vec::new();
        let mut at = start;
        while at < end {
            crate::video_clip_backend::check_cancel(cancel)?;
            if *budget == 0 || end - at < 8 {
                return Err(ClipError::OutputInvalid);
            }
            *budget -= 1;
            let header = read_iso_bytes::<8>(file, at)?;
            let short = u32::from_be_bytes(header[..4].try_into().unwrap());
            let (size, header_len) = match short {
                0 => (end - at, 8),
                1 if end - at >= 16 => (u64::from_be_bytes(read_iso_bytes::<8>(file, at + 8)?), 16),
                1 => return Err(ClipError::OutputInvalid),
                n => (u64::from(n), 8),
            };
            let stop = at.checked_add(size).ok_or(ClipError::OutputInvalid)?;
            if size < header_len || stop > end {
                return Err(ClipError::OutputInvalid);
            }
            boxes.push(IsoBox {
                kind: header[4..].try_into().unwrap(),
                body: at + header_len,
                end: stop,
            });
            at = stop;
        }
        Ok(boxes)
    }
    fn one_box(boxes: &[IsoBox], kind: [u8; 4]) -> Result<IsoBox, ClipError> {
        let mut found = boxes.iter().copied().filter(|b| b.kind == kind);
        let one = found.next().ok_or(ClipError::OutputInvalid)?;
        if found.next().is_some() {
            return Err(ClipError::OutputInvalid);
        }
        Ok(one)
    }
    /// Structured, bounded ISO-BMFF reads. mdat is skipped by Seek; no movie or
    /// moov blob is loaded into memory. Depth is fixed at the five needed
    /// containers, at most 4096 boxes/eight tracks/two CFR stts runs are read.
    fn verify_track_clock(
        output: &Path,
        expected: TrackClock,
        cancel: &AtomicBool,
    ) -> Result<fs::File, ClipError> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
        let mut file = options.open(output).map_err(|_| ClipError::OutputInvalid)?;
        let meta = file.metadata().map_err(|_| ClipError::OutputInvalid)?;
        if !meta.is_file() || !(12..=video_clip_contract::MAX_SOURCE_BYTES).contains(&meta.len()) {
            return Err(ClipError::OutputInvalid);
        }
        let mut budget = 4096;
        let roots = iso_children(&mut file, 0, meta.len(), &mut budget, cancel)?;
        let moov = one_box(&roots, *b"moov")?;
        if moov.end - moov.body > 32 * 1024 * 1024 {
            return Err(ClipError::OutputInvalid);
        }
        let boxes = iso_children(&mut file, moov.body, moov.end, &mut budget, cancel)?;
        let tracks: Vec<_> = boxes
            .iter()
            .filter(|b| b.kind == *b"trak")
            .copied()
            .collect();
        if tracks.is_empty() || tracks.len() > 8 {
            return Err(ClipError::OutputInvalid);
        }
        let mut video_count = 0;
        for track in tracks {
            crate::video_clip_backend::check_cancel(cancel)?;
            let children = iso_children(&mut file, track.body, track.end, &mut budget, cancel)?;
            let mdia = one_box(&children, *b"mdia")?;
            let media = iso_children(&mut file, mdia.body, mdia.end, &mut budget, cancel)?;
            let handler = one_box(&media, *b"hdlr")?;
            if handler.end - handler.body < 12 {
                return Err(ClipError::OutputInvalid);
            }
            if read_iso_bytes::<4>(&mut file, handler.body + 8)? != *b"vide" {
                continue;
            }
            video_count += 1;
            if video_count > 1 {
                return Err(ClipError::OutputInvalid);
            }
            let mdhd = one_box(&media, *b"mdhd")?;
            if mdhd.end - mdhd.body < 4 {
                return Err(ClipError::OutputInvalid);
            }
            let version = read_iso_bytes::<4>(&mut file, mdhd.body)?;
            if version[1..] != [0, 0, 0] {
                return Err(ClipError::OutputInvalid);
            }
            let (scale, duration) = match version[0] {
                0 if mdhd.end - mdhd.body >= 24 => (
                    u32::from_be_bytes(read_iso_bytes::<4>(&mut file, mdhd.body + 12)?),
                    u64::from(u32::from_be_bytes(read_iso_bytes::<4>(
                        &mut file,
                        mdhd.body + 16,
                    )?)),
                ),
                1 if mdhd.end - mdhd.body >= 36 => (
                    u32::from_be_bytes(read_iso_bytes::<4>(&mut file, mdhd.body + 20)?),
                    u64::from_be_bytes(read_iso_bytes::<8>(&mut file, mdhd.body + 24)?),
                ),
                _ => return Err(ClipError::OutputInvalid),
            };
            if scale != expected.scale || duration != expected.declared_units {
                return Err(ClipError::OutputInvalid);
            }
            let minf = one_box(&media, *b"minf")?;
            let boxes = iso_children(&mut file, minf.body, minf.end, &mut budget, cancel)?;
            let stbl = one_box(&boxes, *b"stbl")?;
            let boxes = iso_children(&mut file, stbl.body, stbl.end, &mut budget, cancel)?;
            let stts = one_box(&boxes, *b"stts")?;
            if stts.end - stts.body < 8 {
                return Err(ClipError::OutputInvalid);
            }
            let header = read_iso_bytes::<8>(&mut file, stts.body)?;
            let entries = u32::from_be_bytes(header[4..].try_into().unwrap());
            if header[..4] != [0, 0, 0, 0]
                || !(1..=2).contains(&entries)
                || stts.end - stts.body != 8 + 8 * u64::from(entries)
            {
                return Err(ClipError::OutputInvalid);
            }
            let mut count = 0u64;
            for entry in 0..entries {
                crate::video_clip_backend::check_cancel(cancel)?;
                let row = read_iso_bytes::<8>(&mut file, stts.body + 8 + 8 * u64::from(entry))?;
                let n = u64::from(u32::from_be_bytes(row[..4].try_into().unwrap()));
                let delta = u64::from(u32::from_be_bytes(row[4..].try_into().unwrap()));
                let next = count.checked_add(n).ok_or(ClipError::OutputInvalid)?;
                if n == 0
                    || next > expected.count
                    || delta == 0
                    || (next < expected.count && delta != expected.regular_units)
                    || (next == expected.count
                        && (delta != expected.tail_units
                            || n > 1 && delta != expected.regular_units))
                {
                    return Err(ClipError::OutputInvalid);
                }
                count = next;
            }
            if count != expected.count {
                return Err(ClipError::OutputInvalid);
            }
        }
        if video_count != 1 {
            return Err(ClipError::OutputInvalid);
        }
        Ok(file) // read-only/no-write/delete lease survives all native reads
    }
    /// Require every native H.264 packet to retain the submitted CFR clock,
    /// including the capped last sample. A separate RGB decoder proves the
    /// first samples and the final sample are actually decodable at those PTS.
    /// Native packet scan does not allocate or decode a whole movie into memory.
    pub(super) fn verify_finalized_timing(
        output: &Path,
        cadence_info: &VideoMetadata,
        duration: u64,
        submitted: u64,
        cancel: &AtomicBool,
    ) -> Result<(), ClipError> {
        use crate::video_gif_backend::source_stream::Reader;
        let invalid = |error: Box<dyn std::error::Error + Send + Sync>| {
            if error.downcast_ref::<ClipError>() == Some(&ClipError::Cancelled) {
                ClipError::Cancelled
            } else {
                ClipError::OutputInvalid
            }
        };
        let expected = (u128::from(duration) * u128::from(cadence_info.frame_rate_numerator))
            .div_ceil(10_000_000 * u128::from(cadence_info.frame_rate_denominator))
            as u64;
        if submitted == 0 || submitted != expected || submitted > 432_000 {
            return Err(ClipError::OutputInvalid);
        }
        let clock =
            TrackClock::planned(cadence_info, duration).map_err(|_| ClipError::OutputInvalid)?;
        let _output_lease = verify_track_clock(output, clock, cancel)?;
        unsafe {
            let text = HSTRING::from(output.as_os_str());
            let reader = MFCreateSourceReaderFromURL(PCWSTR(text.as_ptr()), None)
                .map_err(|_| ClipError::OutputInvalid)?;
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(|_| ClipError::OutputInvalid)?;
            reader
                .SetStreamSelection(stream, true)
                .map_err(|_| ClipError::OutputInvalid)?;
            let mut count = 0u64;
            let mut eof = false;
            let mut checked_type = false;
            for _ in 0..500_000 {
                crate::video_clip_backend::check_cancel(cancel)?;
                let (mut flags, mut time, mut sample) = (0, 0, None);
                reader
                    .ReadSample(
                        stream,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut time),
                        Some(&mut sample),
                    )
                    .map_err(|_| ClipError::OutputInvalid)?;
                crate::video_clip_backend::check_cancel(cancel)?;
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    return Err(ClipError::OutputInvalid);
                }
                if !checked_type
                    || flags
                        & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                            | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32)
                        != 0
                {
                    let media = reader
                        .GetCurrentMediaType(stream)
                        .map_err(|_| ClipError::OutputInvalid)?;
                    let dimensions = media
                        .GetUINT64(&MF_MT_FRAME_SIZE)
                        .map_err(|_| ClipError::OutputInvalid)?;
                    if media
                        .GetGUID(&MF_MT_SUBTYPE)
                        .map_err(|_| ClipError::OutputInvalid)?
                        != MFVideoFormat_H264
                        || (dimensions >> 32) as u32 != cadence_info.width
                        || dimensions as u32 != cadence_info.height
                    {
                        return Err(ClipError::OutputInvalid);
                    }
                    checked_type = true;
                }
                if let Some(sample) = sample {
                    if count >= submitted {
                        return Err(ClipError::OutputInvalid);
                    }
                    let planned = cadence(count, cadence_info);
                    let expected_duration = clock.duration_ticks(count + 1 == submitted);
                    let pts = u64::try_from(time).map_err(|_| ClipError::OutputInvalid)?;
                    let sample_pts = u64::try_from(
                        sample
                            .GetSampleTime()
                            .map_err(|_| ClipError::OutputInvalid)?,
                    )
                    .map_err(|_| ClipError::OutputInvalid)?;
                    let sample_duration = u64::try_from(
                        sample
                            .GetSampleDuration()
                            .map_err(|_| ClipError::OutputInvalid)?,
                    )
                    .map_err(|_| ClipError::OutputInvalid)?;
                    if pts.abs_diff(planned) > 1
                        || sample_pts.abs_diff(planned) > 1
                        || sample_duration == 0
                        || sample_duration.abs_diff(expected_duration) > 1
                        || sample
                            .GetTotalLength()
                            .map_err(|_| ClipError::OutputInvalid)?
                            > 32 * 1024 * 1024
                    {
                        #[cfg(test)]
                        eprintln!("synthetic finalized packet mismatch: index={count}, pts={pts}, samplePts={sample_pts}, duration={sample_duration}, plannedPts={planned}, trackDuration={expected_duration}");
                        return Err(ClipError::OutputInvalid);
                    }
                    count += 1;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    eof = true;
                    break;
                }
            }
            if !eof || count != submitted {
                return Err(ClipError::OutputInvalid);
            }
        }
        let mut first =
            Reader::open(output, cadence_info.width, cadence_info.height).map_err(invalid)?;
        for index in 0..submitted.min(3) {
            let frame = first
                .next(cancel)
                .map_err(invalid)?
                .ok_or(ClipError::OutputInvalid)?;
            if frame.pts.abs_diff(cadence(index, cadence_info)) > 1 {
                return Err(ClipError::OutputInvalid);
            }
            let _decoded = first.rgba(&frame, cancel).map_err(invalid)?;
        }
        drop(first);
        let last = cadence(submitted - 1, cadence_info);
        let mut tail =
            Reader::open(output, cadence_info.width, cadence_info.height).map_err(invalid)?;
        tail.seek_for_export(last.saturating_sub(cadence(2, cadence_info)))
            .map_err(invalid)?;
        let mut final_decoded = None;
        let mut eof = false;
        for _ in 0..4096 {
            crate::video_clip_backend::check_cancel(cancel)?;
            let Some(frame) = tail.next(cancel).map_err(invalid)? else {
                eof = true;
                break;
            };
            if frame.pts > last.saturating_add(1) {
                return Err(ClipError::OutputInvalid);
            }
            final_decoded = Some(frame);
        }
        let Some(final_frame) = final_decoded else {
            return Err(ClipError::OutputInvalid);
        };
        let (tail_pts, tail_duration) = (final_frame.pts, final_frame.duration);
        if !eof
            || tail_pts.abs_diff(last) > 1
            || tail_duration.abs_diff(clock.duration_ticks(true)) > 1
        {
            return Err(ClipError::OutputInvalid);
        }
        let _decoded = tail.rgba(&final_frame, cancel).map_err(invalid)?;
        #[cfg(test)]
        eprintln!("synthetic finalized cadence verified: count={submitted}, firstPts=0, lastPts={tail_pts}, lastDuration={tail_duration}, rate={}/{}, trackScale={}, mdhdUnits={}, tailUnits={}",cadence_info.frame_rate_numerator,cadence_info.frame_rate_denominator,clock.scale,clock.declared_units,clock.tail_units);
        Ok(())
    }
    unsafe fn bytes_sample(
        bytes: &[u8],
        time: u64,
        duration: u64,
    ) -> windows::core::Result<IMFSample> {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
        let mut destination = std::ptr::null_mut();
        buffer.Lock(&mut destination, None, None)?;
        if destination.is_null() {
            let _ = buffer.Unlock();
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(bytes.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(time as i64)?;
        sample.SetSampleDuration(duration as i64)?;
        Ok(sample)
    }
    fn video_type(
        source: &VideoMetadata,
        pixel_aspect: (u32, u32),
        subtype: &GUID,
    ) -> windows::core::Result<IMFMediaType> {
        unsafe {
            let ty = MFCreateMediaType()?;
            ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            ty.SetGUID(&MF_MT_SUBTYPE, subtype)?;
            ty.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            ty.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(source.width) << 32) | u64::from(source.height),
            )?;
            ty.SetUINT64(
                &MF_MT_FRAME_RATE,
                (u64::from(source.frame_rate_numerator) << 32)
                    | u64::from(source.frame_rate_denominator),
            )?;
            ty.SetUINT64(
                &MF_MT_PIXEL_ASPECT_RATIO,
                (u64::from(pixel_aspect.0) << 32) | u64::from(pixel_aspect.1),
            )?;
            Ok(ty)
        }
    }
    /// Microsoft's color-only DSP has no frame-rate conversion. Keep this
    /// single-image RGB32 -> NV12 stage outside the sink writer; its implicit
    /// conversion discarded a short final input frame in the real native gate.
    struct PixelConverter {
        transform: IMFTransform,
        bytes: u32,
        capacity: u32,
        alignment: u32,
    }
    impl PixelConverter {
        fn new(source: &VideoMetadata, pixel_aspect: (u32, u32)) -> Result<Self, ClipError> {
            unsafe {
                use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
                if source.width % 2 != 0 || source.height % 2 != 0 {
                    return Err(ClipError::InvalidMedia);
                }
                let transform: IMFTransform = native(CoCreateInstance(
                    &CColorConvertDMO,
                    None,
                    CLSCTX_INPROC_SERVER,
                ))?;
                let input = native(video_type(source, pixel_aspect, &MFVideoFormat_RGB32))?;
                native(input.SetUINT32(&MF_MT_DEFAULT_STRIDE, source.width * 4))?;
                native(input.DeleteItem(&MF_MT_FRAME_RATE))?;
                let output = native(video_type(source, pixel_aspect, &MFVideoFormat_NV12))?;
                native(output.DeleteItem(&MF_MT_FRAME_RATE))?;
                native(output.SetUINT32(&MF_MT_DEFAULT_STRIDE, source.width))?;
                native(transform.SetInputType(0, &input, 0))?;
                native(transform.SetOutputType(0, &output, 0))?;
                let info = native(transform.GetOutputStreamInfo(0))?;
                let bytes = source.width * source.height * 3 / 2;
                if info.cbSize < bytes
                    || info.cbSize > source.width * source.height * 4
                    || info.cbAlignment > 4096
                    || info.cbAlignment > 0 && !info.cbAlignment.is_power_of_two()
                {
                    return Err(ClipError::InvalidMedia);
                }
                native(transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0))?;
                native(transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0))?;
                Ok(Self {
                    transform,
                    bytes,
                    capacity: info.cbSize,
                    alignment: info.cbAlignment.saturating_sub(1),
                })
            }
        }
        fn sample(
            &self,
            rgba: &image::RgbaImage,
            time: u64,
            duration: u64,
        ) -> Result<IMFSample, ClipError> {
            unsafe {
                use std::mem::ManuallyDrop;
                let mut bytes = rgba.as_raw().clone();
                for pixel in bytes.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                let input = native(bytes_sample(&bytes, time, duration))?;
                native(self.transform.ProcessInput(0, &input, 0))?;
                let sample = native(MFCreateSample())?;
                let buffer = native(MFCreateAlignedMemoryBuffer(self.capacity, self.alignment))?;
                native(sample.AddBuffer(&buffer))?;
                let mut output = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(Some(sample)),
                    ..Default::default()
                };
                let mut status = 0;
                let result =
                    self.transform
                        .ProcessOutput(0, std::slice::from_mut(&mut output), &mut status);
                let sample = ManuallyDrop::take(&mut output.pSample);
                drop(ManuallyDrop::take(&mut output.pEvents));
                native(result)?;
                let sample = sample.ok_or(ClipError::EncodingFailed)?;
                if native(sample.ConvertToContiguousBuffer())?
                    .GetCurrentLength()
                    .map_err(|_| ClipError::EncodingFailed)?
                    != self.bytes
                {
                    return Err(ClipError::OutputInvalid);
                }
                native(sample.SetSampleTime(time as i64))?;
                native(sample.SetSampleDuration(duration as i64))?;
                Ok(sample)
            }
        }
    }
    #[derive(Clone, Copy, Debug)]
    struct AudioFormat {
        rate: u32,
        channels: u32,
        mask: u32,
    }
    impl AudioFormat {
        fn block(self) -> usize {
            self.channels as usize * 2
        }
        fn pcm(self) -> windows::core::Result<IMFMediaType> {
            unsafe {
                let ty = MFCreateMediaType()?;
                ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                ty.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                ty.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, self.channels)?;
                ty.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, self.rate)?;
                ty.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                ty.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, self.block() as u32)?;
                ty.SetUINT32(
                    &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                    self.rate * self.block() as u32,
                )?;
                if self.mask != 0 {
                    ty.SetUINT32(&MF_MT_AUDIO_CHANNEL_MASK, self.mask)?;
                }
                Ok(ty)
            }
        }
    }
    struct AudioPacket {
        pts: i64,
        bytes: Vec<u8>,
        cursor: usize,
        end: usize,
    }
    struct AudioReader {
        reader: IMFSourceReader,
        format: AudioFormat,
        pending: Option<AudioPacket>,
        eof: bool,
        reads: u64,
        previous_end: Option<i64>,
        start: u64,
        end: u64,
    }
    impl AudioReader {
        fn open(path: &Path, start: u64, end: u64) -> Result<Self, ClipError> {
            unsafe {
                let text = HSTRING::from(path.as_os_str());
                let reader = native(MFCreateSourceReaderFromURL(PCWSTR(text.as_ptr()), None))?;
                let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
                native(reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false))?;
                native(reader.SetStreamSelection(stream, true))?;
                let source = native(reader.GetNativeMediaType(stream, 0))?;
                let rate = native(source.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND))?;
                let channels = native(source.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS))?;
                if !matches!(rate, 44100 | 48000) || !matches!(channels, 1 | 2 | 6) {
                    return Err(ClipError::InvalidMedia);
                }
                let format = AudioFormat {
                    rate,
                    channels,
                    mask: source.GetUINT32(&MF_MT_AUDIO_CHANNEL_MASK).unwrap_or(0),
                };
                native(reader.SetCurrentMediaType(
                    stream,
                    None,
                    &format.pcm().map_err(|_| ClipError::EncodingFailed)?,
                ))?;
                let actual = native(reader.GetCurrentMediaType(stream))?;
                if native(actual.GetGUID(&MF_MT_SUBTYPE))? != MFAudioFormat_PCM
                    || native(actual.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE))? != 16
                    || native(actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS))? != channels
                    || native(actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND))? != rate
                {
                    return Err(ClipError::InvalidMedia);
                }
                use windows::Win32::System::{
                    Com::StructuredStorage::{PropVariantClear, PROPVARIANT},
                    Variant::VT_I8,
                };
                let mut position = PROPVARIANT::default();
                (*position.Anonymous.Anonymous).vt = VT_I8;
                (*position.Anonymous.Anonymous).Anonymous.hVal =
                    start.saturating_sub(20_000_000) as i64;
                let result = reader.SetCurrentPosition(&GUID::zeroed(), &position);
                let clear = PropVariantClear(&mut position);
                native(result)?;
                native(clear)?;
                Ok(Self {
                    reader,
                    format,
                    pending: None,
                    eof: false,
                    reads: 0,
                    previous_end: None,
                    start,
                    end,
                })
            }
        }
        fn sample_time(&self, packet: &AudioPacket, index: usize) -> i64 {
            packet.pts + (index as i128 * 10_000_000 / i128::from(self.format.rate)) as i64
        }
        fn cut(&self, tick: u64, pts: i64, count: usize) -> usize {
            let delta = i128::from(tick) - i128::from(pts);
            if delta <= 0 {
                0
            } else {
                ((delta * i128::from(self.format.rate) + 9_999_999) / 10_000_000).min(count as i128)
                    as usize
            }
        }
        fn read_packet(&mut self, cancel: &AtomicBool) -> Result<Option<AudioPacket>, ClipError> {
            unsafe {
                let mut empty = 0;
                loop {
                    crate::video_clip_backend::check_cancel(cancel)?;
                    if self.eof {
                        return Ok(None);
                    }
                    self.reads += 1;
                    if self.reads > 500_000 || empty > 1024 {
                        return Err(ClipError::InvalidMedia);
                    }
                    let (mut flags, mut pts, mut sample) = (0, 0, None);
                    native(self.reader.ReadSample(
                        MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut pts),
                        Some(&mut sample),
                    ))?;
                    if flags
                        & (MF_SOURCE_READERF_ERROR.0 as u32
                            | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                            | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32)
                        != 0
                    {
                        return Err(ClipError::InvalidMedia);
                    }
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        self.eof = true;
                    }
                    let Some(sample) = sample else {
                        empty += 1;
                        continue;
                    };
                    if native(sample.GetSampleTime())? != pts
                        || native(sample.GetSampleDuration())? <= 0
                    {
                        return Err(ClipError::InvalidMedia);
                    }
                    let buffer = native(sample.ConvertToContiguousBuffer())?;
                    let length = native(buffer.GetCurrentLength())? as usize;
                    if length == 0 || length > 1024 * 1024 || length % self.format.block() != 0 {
                        return Err(ClipError::InvalidMedia);
                    }
                    let count = length / self.format.block();
                    let packet_end =
                        pts + (count as i128 * 10_000_000 / i128::from(self.format.rate)) as i64;
                    if self.previous_end.is_some_and(|last| pts < last - 1) {
                        return Err(ClipError::InvalidMedia);
                    }
                    self.previous_end = Some(packet_end);
                    if pts >= self.end as i64 {
                        self.eof = true;
                        return Ok(None);
                    }
                    if packet_end <= self.start as i64 {
                        continue;
                    }
                    let mut data = std::ptr::null_mut();
                    let mut capacity = 0;
                    let mut locked_len = 0;
                    native(buffer.Lock(&mut data, Some(&mut capacity), Some(&mut locked_len)))?;
                    if data.is_null() || locked_len as usize != length || capacity < locked_len {
                        let _ = buffer.Unlock();
                        return Err(ClipError::InvalidMedia);
                    }
                    let bytes = std::slice::from_raw_parts(data, length).to_vec();
                    native(buffer.Unlock())?;
                    let cursor = self.cut(self.start, pts, count);
                    let end = self.cut(self.end, pts, count);
                    if cursor < end {
                        return Ok(Some(AudioPacket {
                            pts,
                            bytes,
                            cursor,
                            end,
                        }));
                    }
                }
            }
        }
        fn drain_until(
            &mut self,
            until: u64,
            sink: &IMFSinkWriter,
            stream: u32,
            cancel: &AtomicBool,
        ) -> Result<(), ClipError> {
            loop {
                crate::video_clip_backend::check_cancel(cancel)?;
                if self.pending.is_none() {
                    self.pending = self.read_packet(cancel)?;
                }
                let Some(packet) = self.pending.as_ref() else {
                    return Ok(());
                };
                let finish = self.cut(until, packet.pts, packet.end).min(packet.end);
                if finish <= packet.cursor {
                    return Ok(());
                }
                let start_time = self.sample_time(packet, packet.cursor);
                let end_time = self.sample_time(packet, finish).min(self.end as i64);
                if start_time < self.start as i64 || end_time <= start_time {
                    return Err(ClipError::InvalidMedia);
                }
                let bytes = &packet.bytes
                    [packet.cursor * self.format.block()..finish * self.format.block()];
                unsafe {
                    let sample = native(bytes_sample(
                        bytes,
                        (start_time - self.start as i64) as u64,
                        (end_time - start_time) as u64,
                    ))?;
                    native(sink.WriteSample(stream, &sample))?;
                }
                let packet = self.pending.as_mut().unwrap();
                packet.cursor = finish;
                if packet.cursor == packet.end {
                    self.pending = None;
                } else {
                    return Ok(());
                }
            }
        }
    }
    pub(super) fn render(
        source: &Path,
        output: &Path,
        expected: &VideoMetadata,
        range: Option<&VideoRange>,
        overlays: &[crate::video_clip_contract::OverlayRaster],
        pixel_aspect: (u32, u32),
        bitrate: u32,
        cancel: &AtomicBool,
        progress: &dyn Fn(u8),
    ) -> Result<(), ClipError> {
        crate::video_clip_backend::check_cancel(cancel)?;
        if pixel_aspect.0 == 0 || pixel_aspect.1 == 0 {
            return Err(ClipError::InvalidMedia);
        }
        let plan = RasterPlan::load(expected, range, overlays, cancel)?;
        let start = range.map_or(0, |r| r.start_ticks);
        let end = range.map_or(expected.duration_ticks, |r| r.end_ticks);
        let duration = end - start;
        let mut encoded_cadence = expected.clone();
        let (rate_num, rate_den) = crate::video_clip_contract::annotated_output_cadence(expected)?;
        encoded_cadence.frame_rate_numerator = rate_num;
        encoded_cadence.frame_rate_denominator = rate_den;
        // Fail before decoder/encoder work if rounding would make the final
        // native MP4 sample empty; never silently omit that source interval.
        TrackClock::planned(&encoded_cadence, duration)?;
        unsafe {
            native(MFStartup(MF_VERSION, MFSTARTUP_FULL))?;
        }
        let _mf = Mf;
        let mut video = PlaybackFrames::open(source, expected, start, cancel)?;
        let converter = PixelConverter::new(&encoded_cadence, pixel_aspect)?;
        let mut audio = if expected.has_audio {
            Some(AudioReader::open(source, start, end)?)
        } else {
            None
        };
        let (sink, stream, audio_stream) = unsafe {
            let mut attrs = None;
            native(MFCreateAttributes(&mut attrs, 2))?;
            let attrs = attrs.ok_or(ClipError::EncodingFailed)?;
            native(attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4))?;
            // Microsoft forbids ENABLE_HARDWARE_TRANSFORMS together with
            // DISABLE_CONVERTERS. This exact-cadence export uses the native
            // software encoder and directly supported NV12/PCM16 inputs.
            native(attrs.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 1))?;
            let text = HSTRING::from(output.as_os_str());
            let sink = native(MFCreateSinkWriterFromURL(
                PCWSTR(text.as_ptr()),
                None,
                &attrs,
            ))?;
            let output_type = native(video_type(
                &encoded_cadence,
                pixel_aspect,
                &MFVideoFormat_H264,
            ))?;
            native(output_type.SetUINT32(
                &MF_MT_AVG_BITRATE,
                if bitrate == 0 { 8_000_000 } else { bitrate },
            ))?;
            let stream = native(sink.AddStream(&output_type))?;
            let input = native(video_type(
                &encoded_cadence,
                pixel_aspect,
                &MFVideoFormat_NV12,
            ))?;
            native(input.SetUINT32(&MF_MT_DEFAULT_STRIDE, expected.width))?;
            native(input.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1))?;
            native(input.SetUINT32(&MF_MT_SAMPLE_SIZE, expected.width * expected.height * 3 / 2))?;
            native(sink.SetInputMediaType(stream, &input, None))?;
            let audio_stream = if let Some(audio) = &audio {
                let ty = native(MFCreateMediaType())?;
                native(ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio))?;
                native(ty.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC))?;
                native(ty.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio.format.channels))?;
                native(ty.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio.format.rate))?;
                native(ty.SetUINT32(
                    &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                    24_000 * if audio.format.channels == 6 { 6 } else { 1 },
                ))?;
                native(ty.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16))?;
                native(ty.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0))?;
                let id = native(sink.AddStream(&ty))?;
                native(sink.SetInputMediaType(id, &native(audio.format.pcm())?, None))?;
                Some(id)
            } else {
                None
            };
            native(sink.BeginWriting())?;
            (sink, stream, audio_stream)
        };
        progress(0);
        let mut prior = 0;
        let mut index = 0u64;
        loop {
            crate::video_clip_backend::check_cancel(cancel)?;
            let tick = cadence(index, &encoded_cadence);
            if tick >= duration {
                break;
            }
            let next = cadence(index + 1, &encoded_cadence).min(duration);
            if next <= tick {
                return Err(ClipError::InvalidMedia);
            }
            let source_time = SourcePlaybackTicks(start + tick);
            let mut rgba = video.rgba_at(source_time, cancel)?;
            plan.paint(&mut rgba, source_time, cancel)?;
            unsafe {
                let sample = converter.sample(&rgba, tick, next - tick)?;
                native(sink.WriteSample(stream, &sample))?;
            }
            if let (Some(audio), Some(audio_stream)) = (&mut audio, audio_stream) {
                audio.drain_until(start + next, &sink, audio_stream, cancel)?;
            }
            if std::fs::metadata(output)
                .is_ok_and(|m| m.len() > crate::video_clip_contract::MAX_SOURCE_BYTES)
            {
                return Err(ClipError::OutputInvalid);
            }
            let percent = (u128::from(next) * 99 / u128::from(duration)) as u8;
            if percent > prior {
                progress(percent);
                prior = percent;
            }
            index += 1;
        }
        if let (Some(audio), Some(audio_stream)) = (&mut audio, audio_stream) {
            audio.drain_until(end, &sink, audio_stream, cancel)?;
        }
        crate::video_clip_backend::check_cancel(cancel)?;
        #[cfg(test)]
        unsafe {
            let mut stats = MF_SINK_WRITER_STATISTICS {
                cb: std::mem::size_of::<MF_SINK_WRITER_STATISTICS>() as u32,
                ..Default::default()
            };
            if sink.GetStatistics(stream, &mut stats).is_ok() {
                eprintln!("synthetic sink before Finalize: submitted={index}, received={}, encoded={}, processed={}, lastReceived={}, lastEncoded={}",stats.qwNumSamplesReceived,stats.qwNumSamplesEncoded,stats.qwNumSamplesProcessed,stats.llLastTimestampReceived,stats.llLastTimestampEncoded);
            }
        }
        unsafe {
            native(sink.Finalize())?;
        }
        unsafe {
            let mut stats = MF_SINK_WRITER_STATISTICS {
                cb: std::mem::size_of::<MF_SINK_WRITER_STATISTICS>() as u32,
                ..Default::default()
            };
            native(sink.GetStatistics(stream, &mut stats))?;
            #[cfg(test)]
            eprintln!("synthetic sink after Finalize: submitted={index}, received={}, encoded={}, processed={}, lastReceived={}, lastEncoded={}",stats.qwNumSamplesReceived,stats.qwNumSamplesEncoded,stats.qwNumSamplesProcessed,stats.llLastTimestampReceived,stats.llLastTimestampEncoded);
            let last = cadence(index.saturating_sub(1), &encoded_cadence);
            if stats.qwNumSamplesReceived != index
                || stats.qwNumSamplesEncoded != index
                || stats.qwNumSamplesProcessed != index
                || u64::try_from(stats.llLastTimestampReceived)
                    .ok()
                    .is_none_or(|pts| pts.abs_diff(last) > 1)
                || u64::try_from(stats.llLastTimestampEncoded)
                    .ok()
                    .is_none_or(|pts| pts.abs_diff(last) > 1)
            {
                return Err(ClipError::OutputInvalid);
            }
        }
        drop(sink);
        drop(audio);
        drop(video);
        crate::video_clip_backend::check_cancel(cancel)?;
        let size = std::fs::metadata(output)
            .map_err(|_| ClipError::FileUnavailable)?
            .len();
        if !(12..=crate::video_clip_contract::MAX_SOURCE_BYTES).contains(&size) {
            return Err(ClipError::OutputInvalid);
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(output)
            .and_then(|f| f.sync_all())
            .map_err(|_| ClipError::FileUnavailable)?;
        verify_finalized_timing(output, &encoded_cadence, duration, index, cancel)?;
        Ok(())
    }
    #[cfg(test)]
    mod track_clock_tests {
        use super::*;
        // Real native SinkWriter moov from synthetic tail-256.mp4; no user media.
        const NATIVE_MOOV: &[u8] = &[
            0x00, 0x00, 0x03, 0x39, 0x6d, 0x6f, 0x6f, 0x76, 0x00, 0x00, 0x00, 0x6c, 0x6d, 0x76,
            0x68, 0x64, 0x00, 0x00, 0x00, 0x00, 0xe6, 0xee, 0x57, 0xd4, 0xe6, 0xee, 0x57, 0xd4,
            0x00, 0x00, 0x75, 0x30, 0x00, 0x00, 0x69, 0x78, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x02, 0x80, 0x74, 0x72, 0x61, 0x6b, 0x00, 0x00,
            0x00, 0x5c, 0x74, 0x6b, 0x68, 0x64, 0x00, 0x00, 0x00, 0x01, 0xe6, 0xee, 0x57, 0xd4,
            0xe6, 0xee, 0x57, 0xd4, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x69, 0x78, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x01, 0x40,
            0x00, 0x00, 0x00, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x02, 0x1c, 0x6d, 0x64, 0x69, 0x61,
            0x00, 0x00, 0x00, 0x20, 0x6d, 0x64, 0x68, 0x64, 0x00, 0x00, 0x00, 0x00, 0xe6, 0xee,
            0x57, 0xd4, 0xe6, 0xee, 0x57, 0xd4, 0x00, 0x00, 0x75, 0x30, 0x00, 0x00, 0x69, 0x78,
            0x55, 0xc4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2d, 0x68, 0x64, 0x6c, 0x72, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x76, 0x69, 0x64, 0x65, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x56, 0x69, 0x64, 0x65, 0x6f, 0x48,
            0x61, 0x6e, 0x64, 0x6c, 0x65, 0x72, 0x00, 0x00, 0x00, 0x01, 0xc7, 0x6d, 0x69, 0x6e,
            0x66, 0x00, 0x00, 0x00, 0x14, 0x76, 0x6d, 0x68, 0x64, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x24, 0x64, 0x69, 0x6e,
            0x66, 0x00, 0x00, 0x00, 0x1c, 0x64, 0x72, 0x65, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0c, 0x75, 0x72, 0x6c, 0x20, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x01, 0x87, 0x73, 0x74, 0x62, 0x6c, 0x00, 0x00, 0x00, 0x97, 0x73,
            0x74, 0x73, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x87, 0x61, 0x76, 0x63, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x40, 0x00, 0xb4, 0x00, 0x48, 0x00, 0x00, 0x00, 0x48, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x0a, 0x41, 0x56, 0x43, 0x20, 0x43, 0x6f, 0x64, 0x69,
            0x6e, 0x67, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0xff, 0xff, 0x00,
            0x00, 0x00, 0x31, 0x61, 0x76, 0x63, 0x43, 0x01, 0x42, 0xc0, 0x0d, 0xff, 0xe1, 0x00,
            0x1a, 0x67, 0x42, 0xc0, 0x0d, 0x95, 0xb0, 0x50, 0x67, 0xe7, 0xc0, 0x44, 0x00, 0x00,
            0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x36, 0x82, 0x21, 0x1b, 0x80, 0x01,
            0x00, 0x04, 0x68, 0xca, 0x8f, 0x20, 0x00, 0x00, 0x00, 0x20, 0x73, 0x74, 0x74, 0x73,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x1b, 0x00, 0x00,
            0x03, 0xe8, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1c,
            0x73, 0x74, 0x73, 0x63, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x01, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x84,
            0x73, 0x74, 0x73, 0x7a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x1c, 0x00, 0x00, 0x04, 0x3c, 0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x01, 0xb9,
            0x00, 0x00, 0x01, 0x06, 0x00, 0x00, 0x01, 0xa8, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00,
            0x01, 0x60, 0x00, 0x00, 0x00, 0xb2, 0x00, 0x00, 0x01, 0x9f, 0x00, 0x00, 0x00, 0xb5,
            0x00, 0x00, 0x02, 0x13, 0x00, 0x00, 0x00, 0xc2, 0x00, 0x00, 0x01, 0x27, 0x00, 0x00,
            0x00, 0xad, 0x00, 0x00, 0x01, 0x87, 0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x00, 0xf9,
            0x00, 0x00, 0x00, 0xc7, 0x00, 0x00, 0x03, 0x06, 0x00, 0x00, 0x00, 0xb6, 0x00, 0x00,
            0x01, 0x08, 0x00, 0x00, 0x00, 0xbb, 0x00, 0x00, 0x01, 0x4c, 0x00, 0x00, 0x00, 0xbe,
            0x00, 0x00, 0x01, 0x0c, 0x00, 0x00, 0x00, 0xb8, 0x00, 0x00, 0x02, 0x6b, 0x00, 0x00,
            0x00, 0xb7, 0x00, 0x00, 0x00, 0x14, 0x73, 0x74, 0x63, 0x6f, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x4f, 0x00, 0x00, 0x00, 0x14, 0x73, 0x74,
            0x73, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x45, 0x75, 0x64, 0x74, 0x61, 0x00, 0x00, 0x00, 0x35, 0x6d, 0x65,
            0x74, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x68, 0x64, 0x6c, 0x72,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x64, 0x69, 0x72, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x08, 0x69, 0x6c, 0x73, 0x74, 0x00, 0x00, 0x00, 0x08, 0x58, 0x74, 0x72, 0x61,
        ];

        fn locations(file: &mut fs::File) -> (IsoBox, IsoBox, IsoBox) {
            let cancel = AtomicBool::new(false);
            let mut budget = 4096;
            let length = file.metadata().unwrap().len();
            let roots = iso_children(file, 0, length, &mut budget, &cancel).unwrap();
            let moov = one_box(&roots, *b"moov").unwrap();
            let boxes = iso_children(file, moov.body, moov.end, &mut budget, &cancel).unwrap();
            let trak = one_box(&boxes, *b"trak").unwrap();
            let boxes = iso_children(file, trak.body, trak.end, &mut budget, &cancel).unwrap();
            let mdia = one_box(&boxes, *b"mdia").unwrap();
            let boxes = iso_children(file, mdia.body, mdia.end, &mut budget, &cancel).unwrap();
            let mdhd = one_box(&boxes, *b"mdhd").unwrap();
            let minf = one_box(&boxes, *b"minf").unwrap();
            let boxes = iso_children(file, minf.body, minf.end, &mut budget, &cancel).unwrap();
            let stbl = one_box(&boxes, *b"stbl").unwrap();
            let boxes = iso_children(file, stbl.body, stbl.end, &mut budget, &cancel).unwrap();
            (trak, mdhd, one_box(&boxes, *b"stts").unwrap())
        }
        #[test]
        fn native_track_golden_rejects_corrupt_units_bounds_duplicates_and_cancel() {
            let directory = crate::video_gif_backend::TestDirectory::new();
            let output = directory.path.join("native-track.mp4");
            let cancel = AtomicBool::new(false);
            let info = VideoMetadata {
                duration_ticks: 9_000_256,
                width: 320,
                height: 180,
                frame_rate_numerator: 30_000,
                frame_rate_denominator: 1000,
                has_audio: false,
            };
            let expected = TrackClock::planned(&info, info.duration_ticks).unwrap();
            fs::write(&output, NATIVE_MOOV).unwrap();
            assert!(verify_track_clock(&output, expected, &cancel).is_ok());
            let (trak, mdhd, stts) = locations(&mut fs::File::open(&output).unwrap());
            // This real native moov has mdhd duration27000 but STTS sums27001.
            // A valid short native sample must preserve these distinct values.
            assert_eq!(expected.declared_units, 27_000);
            assert_eq!(expected.tail_units, 1);
            assert_eq!(expected.count, 28);
            let rejects = |bytes: &[u8]| {
                fs::write(&output, bytes).unwrap();
                assert_eq!(
                    verify_track_clock(&output, expected, &cancel).err(),
                    Some(ClipError::OutputInvalid)
                );
            };
            let mut bounds = NATIVE_MOOV.to_vec();
            bounds[..4].copy_from_slice(&u32::MAX.to_be_bytes());
            rejects(&bounds);
            rejects(&[0, 0, 0, 1, b'm', b'o', b'o', b'v']); // truncated extended-size header
            rejects(&NATIVE_MOOV[..NATIVE_MOOV.len() - 1]);
            for (offset, value) in [
                (mdhd.body + 12, 0u32),
                (mdhd.body + 12, 29_970),
                (mdhd.body + 16, 27_001), // cannot substitute STTS sum for mdhd end
                (stts.body + 4, u32::MAX), // bounded entry count, no allocation
                (stts.body + 8, 0),
                (stts.body + 8, u32::MAX), // count overflow/excess
                (stts.body + 12, 999),     // wrong regular cadence
                (stts.body + 20, 0),       // erased tail
                (stts.body + 20, 2),       // lengthened tail
            ] {
                let mut bytes = NATIVE_MOOV.to_vec();
                bytes[offset as usize..offset as usize + 4].copy_from_slice(&value.to_be_bytes());
                rejects(&bytes);
            }
            let mut version = NATIVE_MOOV.to_vec();
            version[mdhd.body as usize] = 2;
            rejects(&version);
            let mut duplicate = NATIVE_MOOV.to_vec();
            duplicate.extend_from_slice(&NATIVE_MOOV[trak.body as usize - 8..trak.end as usize]);
            let length = duplicate.len() as u32;
            duplicate[..4].copy_from_slice(&length.to_be_bytes());
            rejects(&duplicate);
            let mut many = NATIVE_MOOV.to_vec();
            for _ in 0..4096 {
                many.extend_from_slice(&[0, 0, 0, 8, b'f', b'r', b'e', b'e']);
            }
            rejects(&many);
            fs::write(&output, NATIVE_MOOV).unwrap();
            assert_eq!(
                verify_track_clock(&output, expected, &AtomicBool::new(true)).err(),
                Some(ClipError::Cancelled)
            );
            let mut file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&output)
                .unwrap();
            use std::io::Write;
            file.write_all(&(32u32 * 1024 * 1024 + 17).to_be_bytes())
                .unwrap();
            file.write_all(b"moov").unwrap();
            file.set_len(32 * 1024 * 1024 + 17).unwrap();
            drop(file);
            assert_eq!(
                verify_track_clock(&output, expected, &cancel).err(),
                Some(ClipError::OutputInvalid)
            );
        }
    }
}

/// Intersect an original logical-source interval once. Source bounds/order do
/// not change when the retained range changes. An empty intersection is absent.
pub fn project_interval(
    interval: &VideoRange,
    retained: &VideoRange,
) -> Result<Option<ProjectedInterval>, ClipError> {
    if interval.start_ticks >= interval.end_ticks || retained.start_ticks >= retained.end_ticks {
        return Err(ClipError::InvalidRange);
    }
    let start = interval.start_ticks.max(retained.start_ticks);
    let end = interval.end_ticks.min(retained.end_ticks);
    Ok((start < end).then_some(ProjectedInterval {
        source_start: SourcePlaybackTicks(start),
        source_end: SourcePlaybackTicks(end),
        delay_ticks: start.saturating_sub(retained.start_ticks),
        duration_ticks: end.saturating_sub(start),
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn project_intervals_preserves_logical_clock_order_and_half_open_bounds() {
        let retained = VideoRange {
            start_ticks: 2_150_000,
            end_ticks: 11_150_000,
        };
        let project = |start_ticks, end_ticks| {
            project_interval(
                &VideoRange {
                    start_ticks,
                    end_ticks,
                },
                &retained,
            )
            .unwrap()
        };
        assert_eq!(project(0, 2_150_000), None);
        assert_eq!(project(11_150_000, 20_000_000), None);
        assert_eq!(
            project(1_000_000, 4_150_000),
            Some(ProjectedInterval {
                source_start: SourcePlaybackTicks(2_150_000),
                source_end: SourcePlaybackTicks(4_150_000),
                delay_ticks: 0,
                duration_ticks: 2_000_000,
            })
        );
        assert_eq!(
            project(5_250_000, 20_000_000),
            Some(ProjectedInterval {
                source_start: SourcePlaybackTicks(5_250_000),
                source_end: SourcePlaybackTicks(11_150_000),
                delay_ticks: 3_100_000,
                duration_ticks: 5_900_000,
            })
        );
        assert!(project_interval(
            &VideoRange {
                start_ticks: 7,
                end_ticks: 7
            },
            &retained
        )
        .is_err());
    }
    #[test]
    fn frame_request_covers_full_long_range_and_rejects_invalid_points_before_io() {
        let source = VideoMetadata {
            duration_ticks: 18_000_000_000,
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            has_audio: false,
        };
        let window = VideoRange {
            start_ticks: 0,
            end_ticks: source.duration_ticks,
        };
        let good = [
            SourcePlaybackTicks(2_150_000),
            SourcePlaybackTicks(17_990_000_000),
        ];
        assert!(validate_frame_request(&source, &window, &good).is_ok());
        for invalid in [
            vec![],
            vec![good[0]],
            vec![good[0], good[0]],
            vec![good[1], good[0]],
            vec![good[0], SourcePlaybackTicks(source.duration_ticks)],
            vec![good[0]; 7],
        ] {
            assert!(validate_frame_request(&source, &window, &invalid).is_err());
            assert_eq!(
                extract_frames(
                    Path::new("never-open-invalid-points.mp4"),
                    &source,
                    &window,
                    &invalid,
                    &AtomicBool::new(false)
                )
                .err(),
                Some(ClipError::InvalidRange)
            );
        }
        assert_eq!(
            extract_frames(
                Path::new("never-open-cancelled.mp4"),
                &source,
                &window,
                &good,
                &AtomicBool::new(true)
            )
            .err(),
            Some(ClipError::Cancelled)
        );
    }

    #[cfg(windows)]
    pub(crate) mod media_gate {
        use super::*;
        use crate::video_gif_backend::source_stream::Reader;
        use image::{Rgba, RgbaImage};
        use serde::Serialize;
        use sha2::{Digest, Sha256};
        use std::{
            fs,
            path::{Path, PathBuf},
            sync::atomic::AtomicBool,
        };
        use windows::{
            core::{HSTRING, PCWSTR},
            Win32::{
                Media::MediaFoundation::*,
                System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
            },
        };

        const W: u32 = 320;
        const H: u32 = 180;
        const PIXEL_TOLERANCE: u8 = 12; // Declared before native execution; flat H.264/YUV interiors.
        const POINTS: [(u32, u32); 5] = [(40, 56), (128, 68), (160, 68), (192, 88), (192, 64)];
        type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
        struct Runtime;
        impl Runtime {
            fn new() -> Result<Self> {
                unsafe {
                    RoInitialize(RO_INIT_MULTITHREADED)?;
                    if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                        RoUninitialize();
                        return Err(error.into());
                    }
                }
                Ok(Self)
            }
        }
        impl Drop for Runtime {
            fn drop(&mut self) {
                unsafe {
                    let _ = MFShutdown();
                    RoUninitialize();
                }
            }
        }
        #[derive(Clone, Debug, Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Trace {
            pts_ticks: u64,
            duration_ticks: u64,
            barcode: u32,
            rgb: [[u8; 3]; 5],
        }
        struct Decoded {
            trace: Trace,
            image: RgbaImage,
        }
        fn sha(path: &Path) -> String {
            format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
        }
        fn private_output() -> PathBuf {
            let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .to_path_buf();
            let private = repo.join(".private").canonicalize().unwrap();
            let configured = PathBuf::from(
                std::env::var_os("MEWU_VIDEO_ANNOTATION_TEST_OUTPUT")
                    .expect("explicit empty repository .private output directory required"),
            );
            let resolved = configured.canonicalize().unwrap();
            let relative = resolved
                .strip_prefix(&private)
                .expect("output must be inside repository .private");
            assert!(!relative.as_os_str().is_empty());
            assert!(
                fs::read_dir(&resolved).unwrap().next().is_none(),
                "output must be empty; no overwrite"
            );
            // StorageFile accepts an ordinary path. The canonical containment
            // check above is performed before rebuilding its typed relative path.
            repo.join(".private").join(relative)
        }
        unsafe fn geometry(
            media: &IMFMediaType,
            numerator: u32,
            denominator: u32,
        ) -> windows::core::Result<()> {
            unsafe {
                media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                media.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(W) << 32) | u64::from(H))?;
                media.SetUINT64(
                    &MF_MT_FRAME_RATE,
                    (u64::from(numerator) << 32) | u64::from(denominator),
                )?;
                media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)
            }
        }
        fn frame_pixels(code: u32) -> Vec<u8> {
            let mut image = RgbaImage::from_pixel(W, H, Rgba([40, 100, 160, 255]));
            for bit in 0..6 {
                let value = if code & (1 << bit) == 0 { 8 } else { 248 };
                for y in 150..174 {
                    for x in (12 + bit * 32)..(36 + bit * 32) {
                        image.put_pixel(x, y, Rgba([value, value, value, 255]));
                    }
                }
            }
            image
                .into_raw()
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], 0])
                .collect()
        }
        fn make_source(path: &Path, numerator: u32, denominator: u32, gap: bool) -> Result<()> {
            make_source_with_audio(path, numerator, denominator, gap, false)
        }
        fn make_source_with_audio(
            path: &Path,
            numerator: u32,
            denominator: u32,
            gap: bool,
            audio: bool,
        ) -> Result<()> {
            make_source_audio_format(
                path,
                numerator,
                denominator,
                gap,
                audio.then_some((48_000, 2)),
            )
        }
        fn make_source_audio_format(
            path: &Path,
            numerator: u32,
            denominator: u32,
            gap: bool,
            audio: Option<(u32, u32)>,
        ) -> Result<()> {
            assert!(!path.exists());
            unsafe {
                let mut attrs = None;
                MFCreateAttributes(&mut attrs, 1)?;
                let attrs = attrs.ok_or("writer attributes")?;
                attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
                let text = HSTRING::from(path.as_os_str());
                let writer = MFCreateSinkWriterFromURL(PCWSTR(text.as_ptr()), None, &attrs)?;
                let encoded = MFCreateMediaType()?;
                encoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                encoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                encoded.SetUINT32(&MF_MT_AVG_BITRATE, 2_000_000)?;
                geometry(&encoded, numerator, denominator)?;
                let stream = writer.AddStream(&encoded)?;
                let input = MFCreateMediaType()?;
                input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                geometry(&input, numerator, denominator)?;
                input.SetUINT32(&MF_MT_DEFAULT_STRIDE, W * 4)?;
                input.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
                input.SetUINT32(&MF_MT_SAMPLE_SIZE, W * H * 4)?;
                writer.SetInputMediaType(stream, &input, None)?;
                let audio_stream = if let Some((rate, channels)) = audio {
                    let encoded = MFCreateMediaType()?;
                    encoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                    encoded.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                    encoded.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
                    encoded.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                    encoded.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                    encoded.SetUINT32(
                        &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                        24_000 * if channels == 6 { 6 } else { 1 },
                    )?;
                    encoded.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
                    let id = writer.AddStream(&encoded)?;
                    let pcm = MFCreateMediaType()?;
                    pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                    pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                    pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
                    pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                    pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                    pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * 2)?;
                    pcm.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * 2)?;
                    pcm.SetUINT32(
                        &MF_MT_AUDIO_CHANNEL_MASK,
                        match channels {
                            1 => 4,
                            2 => 3,
                            6 => 63,
                            _ => panic!("fixture channels"),
                        },
                    )?;
                    writer.SetInputMediaType(id, &pcm, None)?;
                    Some(id)
                } else {
                    None
                };
                writer.BeginWriting()?;
                let mut audio_cursor = 0u64;
                let tick = |i: u64| {
                    i * 10_000_000 * u64::from(denominator) / u64::from(numerator)
                        + if gap && i >= 7 { 4_000_000 } else { 0 }
                };
                for code in 0..60u32 {
                    let pixels = frame_pixels(code);
                    let buffer = MFCreateMemoryBuffer(pixels.len() as u32)?;
                    let mut raw = std::ptr::null_mut();
                    buffer.Lock(&mut raw, None, None)?;
                    if raw.is_null() {
                        let _ = buffer.Unlock();
                        return Err("null writer buffer".into());
                    }
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), raw, pixels.len());
                    buffer.Unlock()?;
                    buffer.SetCurrentLength(pixels.len() as u32)?;
                    let sample = MFCreateSample()?;
                    sample.AddBuffer(&buffer)?;
                    sample.SetSampleTime(tick(u64::from(code)) as i64)?;
                    // A missing-frame gap is not one long input sample. Follow
                    // SendStreamTick's documented per-missing-video-frame rule
                    // and mark the first resumed sample as discontinuous.
                    let nominal =
                        |i: u64| i * 10_000_000 * u64::from(denominator) / u64::from(numerator);
                    sample.SetSampleDuration(
                        (nominal(u64::from(code) + 1) - nominal(u64::from(code))) as i64,
                    )?;
                    if gap && code == 7 {
                        for missing in 0..12 {
                            writer.SendStreamTick(stream, nominal(7 + missing) as i64)?;
                        }
                        sample.SetUINT32(&MFSampleExtension_Discontinuity, 1)?;
                    }
                    writer.WriteSample(stream, &sample)?;
                    if let Some(id) = audio_stream {
                        let (rate, channels) = audio.unwrap();
                        let until = (u128::from(tick(u64::from(code) + 1)) * u128::from(rate)
                            / 10_000_000) as u64;
                        while audio_cursor < until {
                            let end = (audio_cursor + 480).min(until);
                            let mut bytes = Vec::with_capacity(
                                (end - audio_cursor) as usize * channels as usize * 2,
                            );
                            for sample in audio_cursor..end {
                                let time = sample as f64 / f64::from(rate);
                                let left = if time < 0.65 { 440. } else { 880. };
                                for channel in 0..channels {
                                    let hz = if channel == 0 {
                                        left
                                    } else {
                                        660. + f64::from(channel - 1) * 110.
                                    };
                                    let value = (12_000.
                                        * (std::f64::consts::TAU * time * hz).sin())
                                    .round() as i16;
                                    bytes.extend_from_slice(&value.to_le_bytes());
                                }
                            }
                            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
                            let mut raw = std::ptr::null_mut();
                            buffer.Lock(&mut raw, None, None)?;
                            if raw.is_null() {
                                let _ = buffer.Unlock();
                                return Err("fixture PCM buffer".into());
                            }
                            std::ptr::copy_nonoverlapping(bytes.as_ptr(), raw, bytes.len());
                            buffer.Unlock()?;
                            buffer.SetCurrentLength(bytes.len() as u32)?;
                            let sample = MFCreateSample()?;
                            sample.AddBuffer(&buffer)?;
                            let begin_tick = audio_cursor * 10_000_000 / u64::from(rate);
                            let end_tick = end * 10_000_000 / u64::from(rate);
                            sample.SetSampleTime(begin_tick as i64)?;
                            sample.SetSampleDuration((end_tick - begin_tick) as i64)?;
                            writer.WriteSample(id, &sample)?;
                            audio_cursor = end;
                        }
                    }
                }
                writer.Finalize()?;
            }
            Ok(())
        }
        fn decode(path: &Path, cancel: &AtomicBool) -> Result<Vec<Decoded>> {
            let mut reader = Reader::open(path, W, H)?;
            let mut frames = Vec::new();
            while let Some(frame) = reader.next(cancel)? {
                if frames.len() >= 240 {
                    return Err("synthetic decode frame budget".into());
                }
                let image = reader.rgba(&frame, cancel)?;
                let mut barcode = 0;
                for bit in 0..6 {
                    let p = image.get_pixel(24 + bit * 32, 160);
                    if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) > 384 {
                        barcode |= 1 << bit;
                    }
                }
                let rgb = POINTS.map(|(x, y)| {
                    let p = image.get_pixel(x, y);
                    [p[0], p[1], p[2]]
                });
                frames.push(Decoded {
                    trace: Trace {
                        pts_ticks: frame.pts,
                        duration_ticks: frame.duration,
                        barcode,
                        rgb,
                    },
                    image,
                });
            }
            if frames.is_empty() {
                return Err("empty synthetic decode".into());
            }
            Ok(frames)
        }
        fn native_packets(path: &Path) -> Result<Vec<(i64, i64)>> {
            unsafe {
                let text = HSTRING::from(path.as_os_str());
                let reader = MFCreateSourceReaderFromURL(PCWSTR(text.as_ptr()), None)?;
                let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
                reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                reader.SetStreamSelection(stream, true)?;
                assert_eq!(
                    reader
                        .GetCurrentMediaType(stream)?
                        .GetGUID(&MF_MT_SUBTYPE)?,
                    MFVideoFormat_H264,
                    "diagnostic must read native packets, not RGB processor"
                );
                let mut output = Vec::new();
                for _ in 0..512 {
                    let (mut flags, mut pts, mut sample) = (0, 0, None);
                    reader.ReadSample(
                        stream,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut pts),
                        Some(&mut sample),
                    )?;
                    if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                        return Err("native packet diagnostic failed".into());
                    }
                    if let Some(sample) = sample {
                        output.push((pts, sample.GetSampleDuration()?));
                    }
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        return Ok(output);
                    }
                }
                Err("native packet diagnostic budget".into())
            }
        }
        fn active(range: &VideoRange, tick: u64) -> bool {
            range.start_ticks <= tick && tick < range.end_ticks
        }
        fn direct_rasters(root: &Path) -> Vec<crate::video_clip_contract::OverlayRaster> {
            use crate::video_clip_contract::{OverlayBounds, OverlayRaster};
            let ramp = root.join("direct-straight.png");
            RgbaImage::from_fn(96, 64, |x, _| {
                if x < 32 {
                    Rgba([0, 0, 0, 0])
                } else {
                    Rgba([220, 20, 20, if x < 64 { 128 } else { 255 }])
                }
            })
            .save(&ramp)
            .unwrap();
            let first = root.join("direct-first.png");
            RgbaImage::from_pixel(96, 64, Rgba([20, 220, 220, 255]))
                .save(&first)
                .unwrap();
            let top = root.join("direct-top.png");
            RgbaImage::from_pixel(20, 20, Rgba([240, 220, 20, 255]))
                .save(&top)
                .unwrap();
            [
                (
                    first,
                    96,
                    64,
                    OverlayBounds {
                        x: 8.,
                        y: 24.,
                        width: 96.,
                        height: 64.,
                    },
                    1_000_000,
                    3_150_000,
                ),
                (
                    ramp,
                    96,
                    64,
                    OverlayBounds {
                        x: 112.25,
                        y: 36.25,
                        width: 96.,
                        height: 64.,
                    },
                    3_300_000,
                    6_800_000,
                ),
                (
                    top,
                    20,
                    20,
                    OverlayBounds {
                        x: 182.,
                        y: 52.,
                        width: 20.,
                        height: 20.,
                    },
                    4_500_000,
                    5_500_000,
                ),
            ]
            .into_iter()
            .map(
                |(png, width, height, bounds, start_ticks, end_ticks)| OverlayRaster {
                    png_sha256: sha(&png),
                    png,
                    width,
                    height,
                    bounds,
                    interval: VideoRange {
                        start_ticks,
                        end_ticks,
                    },
                },
            )
            .collect()
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct PcmTrace {
            pts_ticks: i64,
            duration_ticks: i64,
            frames: usize,
        }
        struct PcmDecoded {
            rate: u32,
            channels: u32,
            samples: std::collections::BTreeMap<i64, Vec<i16>>,
            trace: Vec<PcmTrace>,
        }
        /// Independent native PCM decoder oracle. Does not call product clip /
        /// sample-cut code and retains absolute packet PTS as evidence.
        fn decode_pcm(path: &Path) -> Result<PcmDecoded> {
            unsafe {
                let text = HSTRING::from(path.as_os_str());
                let reader = MFCreateSourceReaderFromURL(PCWSTR(text.as_ptr()), None)?;
                let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
                reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                reader.SetStreamSelection(stream, true)?;
                let original = reader.GetNativeMediaType(stream, 0)?;
                let rate = original.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
                let channels = original.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?;
                let ty = MFCreateMediaType()?;
                ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                ty.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                ty.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                ty.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
                ty.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                reader.SetCurrentMediaType(stream, None, &ty)?;
                let mut samples = std::collections::BTreeMap::new();
                let mut trace = Vec::new();
                let mut eof = false;
                for _ in 0..256 {
                    let (mut flags, mut pts, mut sample) = (0, 0, None);
                    reader.ReadSample(
                        stream,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut pts),
                        Some(&mut sample),
                    )?;
                    if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                        return Err("PCM oracle decode".into());
                    }
                    if let Some(sample) = sample {
                        assert_eq!(sample.GetSampleTime()?, pts);
                        let duration = sample.GetSampleDuration()?;
                        let buffer = sample.ConvertToContiguousBuffer()?;
                        let length = buffer.GetCurrentLength()? as usize;
                        if length > 1024 * 1024 || length % (channels as usize * 2) != 0 {
                            return Err("PCM oracle allocation".into());
                        }
                        let mut raw = std::ptr::null_mut();
                        buffer.Lock(&mut raw, None, None)?;
                        if raw.is_null() {
                            let _ = buffer.Unlock();
                            return Err("PCM oracle null".into());
                        }
                        let bytes = std::slice::from_raw_parts(raw, length).to_vec();
                        buffer.Unlock()?;
                        let count = length / (channels as usize * 2);
                        trace.push(PcmTrace {
                            pts_ticks: pts,
                            duration_ticks: duration,
                            frames: count,
                        });
                        // nearest integer native sample-grid identity; only the
                        // probe uses this grid, product retains exact packet ticks.
                        let first = (i128::from(pts) * i128::from(rate) + 5_000_000) / 10_000_000;
                        for (index, frame) in bytes.chunks_exact(channels as usize * 2).enumerate()
                        {
                            let values = frame
                                .chunks_exact(2)
                                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                                .collect::<Vec<_>>();
                            assert!(
                                samples
                                    .insert(first as i64 + index as i64, values)
                                    .is_none(),
                                "overlapping PCM packets"
                            );
                        }
                    }
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        eof = true;
                        break;
                    }
                }
                assert!(eof && samples.len() > rate as usize / 2);
                Ok(PcmDecoded {
                    rate,
                    channels,
                    samples,
                    trace,
                })
            }
        }
        #[test]
        #[ignore = "explicit synthetic MF original PCM16 -> AAC trim, multichannel identity, decoded timing and cancellation gate"]
        fn synthetic_direct_audio_source_pcm_trim_and_channel_identity() {
            use crate::video_clip_contract::{ClipJob, ClipJobResult};
            let root = private_output();
            let _runtime = Runtime::new().unwrap();
            let cancel = AtomicBool::new(false);
            let overlays = direct_rasters(&root);
            let retained = VideoRange {
                start_ticks: 2_150_123,
                end_ticks: 11_150_379,
            };
            let mut cases = Vec::new();
            for (rate, channels) in [(44_100, 1), (48_000, 2), (48_000, 6)] {
                let name = format!("pcm{rate}-{channels}ch");
                let source = root.join(format!("{name}-source.mp4"));
                make_source_audio_format(&source, 30, 1, false, Some((rate, channels))).unwrap();
                let hash = sha(&source);
                let info = inspect_source(&source, &cancel);
                assert!(info.has_audio);
                let before = decode_pcm(&source).unwrap();
                assert_eq!((before.rate, before.channels), (rate, channels));
                let output = root.join(format!("{name}-direct.mp4"));
                let ClipJobResult::Rendered(meta) = crate::video_clip_backend::execute(
                    &ClipJob::RenderAnnotated {
                        source: source.clone(),
                        output: output.clone(),
                        expected: info.clone(),
                        range: Some(retained.clone()),
                        overlays: overlays.clone(),
                    },
                    &cancel,
                    &|_| {},
                )
                .unwrap() else {
                    panic!("rendered")
                };
                assert!(meta.has_audio);
                let after = decode_pcm(&output).unwrap();
                assert_eq!((after.rate, after.channels), (rate, channels));
                let mut windows = Vec::new();
                let mut failures = Vec::new();
                for (left, right) in [(0.10, 0.30), (0.35, 0.55), (0.65, 0.80)] {
                    for channel in 0..channels as usize {
                        let mut dot = 0.;
                        let mut a2 = 0.;
                        let mut b2 = 0.;
                        let mut count = 0;
                        for (&index, values) in after.samples.range(
                            (left * f64::from(rate)) as i64..(right * f64::from(rate)) as i64,
                        ) {
                            let original_index =
                                ((u128::from(retained.start_ticks) * u128::from(rate) + 5_000_000)
                                    / 10_000_000) as i64
                                    + index;
                            let Some(expected) = before.samples.get(&original_index) else {
                                continue;
                            };
                            let (a, b) = (f64::from(expected[channel]), f64::from(values[channel]));
                            dot += a * b;
                            a2 += a * a;
                            b2 += b * b;
                            count += 1;
                        }
                        let correlation = dot / (a2 * b2).sqrt();
                        let rms_ratio = (b2 / a2).sqrt();
                        if (count as f64) < (right - left) * f64::from(rate) * 0.95
                            || !correlation.is_finite()
                            || correlation < 0.96
                            || !(0.8..=1.2).contains(&rms_ratio)
                        {
                            failures.push(format!("{name} channel{channel} window{left}-{right}: correlation{correlation} rms{rms_ratio} count{count}"));
                        }
                        windows.push(serde_json::json!({"start":left,"end":right,"channel":channel,"samples":count,"zeroLagCorrelation":correlation,"rmsRatio":rms_ratio}));
                    }
                }
                assert_eq!(sha(&source), hash);
                let video = decode(&output, &cancel).unwrap();
                assert_eq!(video[0].trace.barcode, 6);
                assert_eq!(video[0].trace.pts_ticks, 0);
                cases.push(serde_json::json!({"name":name,"sourceSha256":hash,"range":retained,"sourcePcmPackets":before.trace,"outputPcmPackets":after.trace,"metadata":meta,"windows":windows,"failures":failures}));
                fs::write(
                    root.join("direct-audio-gate-partial.json"),
                    serde_json::to_vec_pretty(&cases).unwrap(),
                )
                .unwrap();
            }
            let proof = serde_json::json!({"syntheticOnly":true,"codec":"original PCM16/rate/channels -> native AAC-LC","clock":"sourcePlaybackTicks","range":retained,"oracle":"independent source/output PCM decode; same original source clock sample grid, no AAC-frame offset; <=2 PCM samples of integer timeline quantization is absorbed only by zero-lag correlation threshold","minimumCorrelation":0.96,"rmsRatioBounds":[0.8,1.2],"cases":cases});
            fs::write(
                root.join("direct-audio-gate.json"),
                serde_json::to_vec_pretty(&proof).unwrap(),
            )
            .unwrap();
            assert!(
                proof["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["failures"].as_array().unwrap().is_empty()),
                "real decoded audio mismatch; do not add offsets to pass"
            );
            let source = root.join("pcm48000-2ch-source.mp4");
            let output = root.join("mid-cancel.mp4");
            let gate = AtomicBool::new(false);
            let result = crate::video_clip_backend::execute(
                &ClipJob::RenderAnnotated {
                    expected: inspect_source(&source, &cancel),
                    source,
                    output: output.clone(),
                    range: Some(retained),
                    overlays,
                },
                &gate,
                &|percent| {
                    if percent >= 5 {
                        gate.store(true, std::sync::atomic::Ordering::Release)
                    }
                },
            );
            assert_eq!(result.unwrap_err(), ClipError::Cancelled);
            assert!(!output.exists());
        }
        fn inspect_source(source: &Path, cancel: &AtomicBool) -> VideoMetadata {
            match crate::video_clip_backend::execute(
                &crate::video_clip_contract::ClipJob::Inspect {
                    source: source.to_path_buf(),
                },
                cancel,
                &|_| {},
            )
            .unwrap()
            {
                crate::video_clip_contract::ClipJobResult::Inspected(m) => m,
                _ => panic!("inspect"),
            }
        }
        // Insert within video_annotation_render::tests::media_gate; not production.
        pub(crate) fn verify_manual_points(
            source: &Path,
            mp4: &Path,
            gif: &Path,
            range: &VideoRange,
            schedule: &crate::video_clip_contract::AnnotatedGifSchedule,
            points: &[(u32, u32, u64, u64, bool)],
        ) -> serde_json::Value {
            let _runtime = Runtime::new().unwrap();
            let cancel = AtomicBool::new(false);
            let originals = decode(source, &cancel).unwrap();
            let frames = decode(mp4, &cancel).unwrap();
            let check = |image: &RgbaImage, logical: u64| {
                let original = originals
                    .iter()
                    .rev()
                    .find(|f| f.trace.pts_ticks <= logical)
                    .unwrap();
                for &(x, y, start, end, ink) in points {
                    let p = image.get_pixel(x, y);
                    let expected = if ink && start <= logical && logical < end {
                        [255, 255, 255]
                    } else {
                        let s = original.image.get_pixel(x, y);
                        [s[0], s[1], s[2]]
                    };
                    assert!(
                        close([p[0], p[1], p[2]], expected),
                        "point ({x},{y}) logical {logical} expected {expected:?} actual {p:?}"
                    );
                }
            };
            assert_eq!(frames.len(), 27);
            for (i, f) in frames.iter().enumerate() {
                assert!(f.trace.pts_ticks.abs_diff(i as u64 * 10_000_000 / 30) <= 1);
                check(&f.image, range.start_ticks + f.trace.pts_ticks);
            }
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = options.read_info(fs::File::open(gif).unwrap()).unwrap();
            let mut index = 0;
            while let Some(f) = decoder.read_next_frame().unwrap() {
                assert_eq!(
                    (f.left, f.top, f.width, f.height),
                    (0, 0, W as u16, H as u16)
                );
                assert_eq!(f.delay, schedule.delays[index]);
                let image = RgbaImage::from_raw(W, H, f.buffer.to_vec()).unwrap();
                check(&image, schedule.requested_ticks[index]);
                index += 1;
            }
            assert_eq!(index, schedule.requested_ticks.len());
            for boundary in [3_150_000, 4_150_000, 4_500_000, 9_500_000] {
                assert!(schedule.requested_ticks.contains(&boundary));
            }
            let audio = decode_pcm(mp4).unwrap();
            assert_eq!((audio.rate, audio.channels), (48_000, 2));
            serde_json::json!({"mp4":mp4,"gif":gif,"mp4Sha256":sha(mp4),"gifSha256":sha(gif),"mp4Frames":frames.len(),"gifFrames":index,"points":points,"pixelTolerance":PIXEL_TOLERANCE,"oracle":"decoded MF/GIF pixels against native glyph white interior or original decoded source; no product compositor","audioRate":audio.rate,"audioChannels":audio.channels})
        }

        pub(crate) fn worker_fixture() -> (
            PathBuf,
            PathBuf,
            VideoMetadata,
            Vec<crate::video_clip_contract::OverlayRaster>,
        ) {
            let root = private_output();
            let _runtime = Runtime::new().unwrap();
            let source = root.join("worker-source.mp4");
            make_source_audio_format(&source, 30, 1, false, Some((48_000, 2))).unwrap();
            let metadata = inspect_source(&source, &AtomicBool::new(false));
            let overlays = direct_rasters(&root);
            (root, source, metadata, overlays)
        }
        pub(crate) fn verify_worker_video(
            source: &Path,
            output: &Path,
            range: &VideoRange,
            overlays: &[crate::video_clip_contract::OverlayRaster],
        ) -> serde_json::Value {
            let _runtime = Runtime::new().unwrap();
            let cancel = AtomicBool::new(false);
            let originals = decode(source, &cancel).unwrap();
            let frames = decode(output, &cancel).unwrap();
            let audio = decode_pcm(output).unwrap();
            assert_eq!((audio.rate, audio.channels), (48_000, 2));
            let mut errors = Vec::new();
            for (index, frame) in frames.iter().enumerate() {
                let tick = index as u64 * 10_000_000 / 30;
                assert!(frame.trace.pts_ticks.abs_diff(tick) <= 1);
                let logical = range.start_ticks + tick;
                let original = originals
                    .iter()
                    .rev()
                    .find(|f| f.trace.pts_ticks <= logical)
                    .unwrap();
                assert_eq!(frame.trace.barcode, original.trace.barcode);
                assert_direct_pixels(
                    &frame.trace,
                    &original.trace,
                    logical,
                    overlays,
                    &mut errors,
                );
            }
            assert_eq!(frames.len(), 27);
            assert!(errors.is_empty(), "{errors:?}");
            serde_json::json!({"videoFrames":frames.iter().map(|f|&f.trace).collect::<Vec<_>>(),"audioRate":audio.rate,"audioChannels":audio.channels,"audioPackets":audio.trace,"errors":errors})
        }
        fn assert_direct_pixels(
            actual: &Trace,
            source: &Trace,
            logical: u64,
            overlays: &[crate::video_clip_contract::OverlayRaster],
            errors: &mut Vec<String>,
        ) {
            let mut expected = source.rgb;
            if active(&overlays[0].interval, logical) {
                expected[0] = [20, 220, 220];
            }
            if active(&overlays[1].interval, logical) {
                expected[2] = over(expected[2], [220, 20, 20], 128);
                expected[3] = [220, 20, 20];
                expected[4] = [220, 20, 20];
            }
            if active(&overlays[2].interval, logical) {
                expected[4] = [240, 220, 20];
            }
            for index in 0..POINTS.len() {
                if !close(actual.rgb[index], expected[index]) {
                    errors.push(format!(
                        "logical {logical} point {:?} actual {:?} expected {:?}",
                        POINTS[index], actual.rgb[index], expected[index]
                    ));
                }
            }
        }
        #[test]
        #[ignore = "explicit synthetic MP4 track-unit rounding and empty-tail rejection gate"]
        fn synthetic_direct_track_quantization_and_nonrepresentable_tail() {
            use crate::video_clip_contract::{ClipJob, ClipJobResult};
            let root = private_output();
            let _runtime = Runtime::new().unwrap();
            let cancel = AtomicBool::new(false);
            let source = root.join("track-quantization-source.mp4");
            make_source(&source, 30, 1, false).unwrap();
            let source_sha = sha(&source);
            let info = inspect_source(&source, &cancel);
            let overlays = direct_rasters(&root);
            let mut cases = Vec::new();
            for (tail_ticks, expected_units) in [(256u64, 1u64), (450, 1), (550, 2)] {
                let output = root.join(format!("tail-{tail_ticks}.mp4"));
                let range = VideoRange {
                    start_ticks: 2_150_000,
                    end_ticks: 11_150_000 + tail_ticks,
                };
                let result = crate::video_clip_backend::execute(
                    &ClipJob::RenderAnnotated {
                        source: source.clone(),
                        output: output.clone(),
                        expected: info.clone(),
                        range: Some(range),
                        overlays: overlays.clone(),
                    },
                    &cancel,
                    &|_| {},
                )
                .unwrap();
                assert!(matches!(result, ClipJobResult::Rendered(_)));
                let packets = native_packets(&output).unwrap();
                let frames = decode(&output, &cancel).unwrap();
                assert_eq!(packets.len(), 28);
                assert_eq!(frames.len(), 28);
                let expected_duration = expected_units * 10_000_000 / 30_000;
                assert_eq!(packets.last(), Some(&(9_000_000, expected_duration as i64)));
                assert_eq!(frames.last().unwrap().trace.pts_ticks, 9_000_000);
                assert_eq!(
                    frames.last().unwrap().trace.duration_ticks,
                    expected_duration
                );
                cases.push(serde_json::json!({"tailRequestedTicks":tail_ticks,"actualNativeTailUnits":expected_units,
                    "packets":packets,"decodedTail":frames.last().unwrap().trace,"outputSha256":sha(&output)}));
            }
            let zero_tail = root.join("unrepresentable-tail.mp4");
            assert_eq!(
                crate::video_clip_backend::execute(
                    &ClipJob::RenderAnnotated {
                        source: source.clone(),
                        output: zero_tail.clone(),
                        expected: info,
                        range: Some(VideoRange {
                            start_ticks: 2_150_000,
                            end_ticks: 11_150_100
                        }),
                        overlays,
                    },
                    &cancel,
                    &|_| panic!("no encoder work for zero native tail")
                ),
                Err(ClipError::InvalidRange)
            );
            assert!(!zero_tail.exists());
            assert_eq!(sha(&source), source_sha);
            fs::write(
                root.join("track-quantization-gate.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "sourceSha256":source_sha,"sourceUntouched":true,"nativeTrackScale":30000,
                    "cases":cases,"zeroUnitTailRejectedBeforeEncoder":true,
                }))
                .unwrap(),
            )
            .unwrap();
        }
        #[test]
        #[ignore = "explicit synthetic MF direct MP4/GIF alpha, z-order, arbitrary trim and held-frame clock gate"]
        fn synthetic_direct_mp4_gif_alpha_non_frame_trim_and_held_clock() {
            use crate::video_clip_contract::{AnnotatedGifSchedule, ClipJob, ClipJobResult};
            let root = private_output();
            let _runtime = Runtime::new().unwrap();
            let cancel = AtomicBool::new(false);
            let overlays = direct_rasters(&root);
            let retained = VideoRange {
                start_ticks: 2_150_000,
                end_ticks: 11_150_000,
            };
            let mut cases = Vec::new();
            for (name, num, den, gap) in [
                ("cfr30", 30, 1, false),
                ("ntsc", 30000, 1001, false),
                ("vfr-hold", 30, 1, true),
            ] {
                let source = root.join(format!("{name}-source.mp4"));
                make_source(&source, num, den, gap).unwrap();
                let source_sha = sha(&source);
                let info = inspect_source(&source, &cancel);
                let source_frames = decode(&source, &cancel).unwrap();
                assert_eq!(source_frames.len(), 60);
                let output = root.join(format!("{name}-direct.mp4"));
                let result = crate::video_clip_backend::execute(
                    &ClipJob::RenderAnnotated {
                        source: source.clone(),
                        output: output.clone(),
                        expected: info.clone(),
                        range: Some(retained.clone()),
                        overlays: overlays.clone(),
                    },
                    &cancel,
                    &|_| {},
                )
                .unwrap();
                let ClipJobResult::Rendered(actual_metadata) = result else {
                    panic!("rendered")
                };
                assert!(!actual_metadata.has_audio);
                let frames = decode(&output, &cancel).unwrap();
                fs::write(
                    root.join(format!("{name}-direct-native-packets.json")),
                    serde_json::to_vec_pretty(&native_packets(&output).unwrap()).unwrap(),
                )
                .unwrap();
                fs::write(
                    root.join(format!("{name}-direct-decoded-ticks.json")),
                    serde_json::to_vec_pretty(&frames.iter().map(|f| &f.trace).collect::<Vec<_>>())
                        .unwrap(),
                )
                .unwrap();
                let duration = retained.end_ticks - retained.start_ticks;
                // MediaClip reports the actual VFR file's average 25 fps (60
                // packets across 2.4 s), not the fixture writer's nominal 30.
                // CFR export is bound to the inspected source contract.
                let encoded_rate = match name {
                    "ntsc" => 29_970u128,
                    "vfr-hold" => 25_000,
                    _ => 30_000,
                };
                assert_eq!(
                    crate::video_clip_contract::annotated_output_cadence(&info).unwrap(),
                    (encoded_rate as u32, 1000)
                );
                let expected_count =
                    (u128::from(duration) * encoded_rate).div_ceil(10_000_000 * 1000) as usize;
                assert_eq!(
                    frames.len(),
                    expected_count,
                    "CFR exact submitted sample count {name}"
                );
                let mut coded = actual_metadata.clone();
                coded.frame_rate_numerator = encoded_rate as u32;
                coded.frame_rate_denominator = 1000;
                super::super::mf_composition::verify_finalized_timing(
                    &output,
                    &coded,
                    duration,
                    expected_count as u64,
                    &cancel,
                )
                .unwrap();
                let mut wrong_rate = coded.clone();
                wrong_rate.frame_rate_numerator += 100;
                assert_eq!(
                    super::super::mf_composition::verify_finalized_timing(
                        &output,
                        &wrong_rate,
                        duration,
                        expected_count as u64,
                        &cancel,
                    ),
                    Err(ClipError::OutputInvalid),
                    "actual native timestamps must reject another cadence"
                );
                assert_eq!(
                    super::super::mf_composition::verify_finalized_timing(
                        &output,
                        &coded,
                        duration + 50_000,
                        expected_count as u64,
                        &cancel,
                    ),
                    Err(ClipError::OutputInvalid),
                    "actual native final duration must reject an extended tail"
                );
                assert_eq!(
                    super::super::mf_composition::verify_finalized_timing(
                        &output,
                        &coded,
                        duration,
                        expected_count as u64 - 1,
                        &cancel,
                    ),
                    Err(ClipError::OutputInvalid),
                    "missing native samples cannot pass"
                );
                let mut errors = Vec::new();
                for (index, frame) in frames.iter().enumerate() {
                    let tick = (index as u128 * 10_000_000 * 1000 / encoded_rate) as u64;
                    if frame.trace.pts_ticks.abs_diff(tick) > 1 {
                        errors.push(format!("actual milli-fps clock mismatch: sample {index}, submitted {tick}, actual decoded {}",frame.trace.pts_ticks));
                    }
                    let logical = retained.start_ticks + tick;
                    let original = source_frames
                        .iter()
                        .rev()
                        .find(|f| f.trace.pts_ticks <= logical)
                        .unwrap();
                    assert_eq!(
                        frame.trace.barcode, original.trace.barcode,
                        "real floor source image {name}/{index}"
                    );
                    assert_direct_pixels(
                        &frame.trace,
                        &original.trace,
                        logical,
                        &overlays,
                        &mut errors,
                    );
                    if index == 0
                        || index + 1 == frames.len()
                        || active(&overlays[2].interval, logical)
                    {
                        frame
                            .image
                            .save(root.join(format!("{name}-direct-{index}.png")))
                            .unwrap();
                    }
                }
                let gif = root.join(format!("{name}-direct.gif"));
                let schedule =
                    AnnotatedGifSchedule::new(&info, Some(&retained), &overlays).unwrap();
                let ClipJobResult::GifRendered(gif_metadata) = crate::video_clip_backend::execute(
                    &ClipJob::RenderGifAnnotated {
                        source: source.clone(),
                        output: gif.clone(),
                        expected: info.clone(),
                        range: Some(retained.clone()),
                        overlays: overlays.clone(),
                    },
                    &cancel,
                    &|_| {},
                )
                .unwrap() else {
                    panic!("gif")
                };
                assert_eq!(gif_metadata, schedule.metadata);
                crate::video_gif_backend::validate_annotated_file(
                    &gif,
                    &gif_metadata,
                    &schedule.delays,
                    &cancel,
                )
                .unwrap();
                let mut decoder = gif::DecodeOptions::new();
                decoder.set_color_output(gif::ColorOutput::RGBA);
                let mut reader = decoder
                    .read_info(std::fs::File::open(&gif).unwrap())
                    .unwrap();
                let mut index = 0;
                while let Some(frame) = reader.read_next_frame().unwrap() {
                    assert_eq!(frame.delay, schedule.delays[index]);
                    let image = RgbaImage::from_raw(W, H, frame.buffer.to_vec()).unwrap();
                    let logical = schedule.requested_ticks[index];
                    let original = source_frames
                        .iter()
                        .rev()
                        .find(|f| f.trace.pts_ticks <= logical)
                        .unwrap();
                    let barcode = (0..6).fold(0, |code, bit| {
                        let p = image.get_pixel(24 + bit * 32, 160);
                        if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) > 384 {
                            code | (1 << bit)
                        } else {
                            code
                        }
                    });
                    assert_eq!(
                        barcode, original.trace.barcode,
                        "GIF real floor source image {name}/{index}"
                    );
                    let trace = Trace {
                        pts_ticks: logical,
                        duration_ticks: u64::from(frame.delay) * 100_000,
                        barcode,
                        rgb: POINTS.map(|(x, y)| {
                            let p = image.get_pixel(x, y);
                            [p[0], p[1], p[2]]
                        }),
                    };
                    assert_direct_pixels(&trace, &original.trace, logical, &overlays, &mut errors);
                    index += 1;
                }
                assert_eq!(index, schedule.requested_ticks.len());
                assert_eq!(sha(&source), source_sha);
                cases.push(serde_json::json!({"name":name,"sourceSha256":source_sha,"metadata":actual_metadata,"mp4Frames":frames.iter().map(|f|&f.trace).collect::<Vec<_>>(),"gifRequestedSourcePlaybackTicks":schedule.requested_ticks,"gifDelays":schedule.delays,"errors":errors}));
                fs::write(
                    root.join("direct-composition-gate-partial.json"),
                    serde_json::to_vec_pretty(&cases).unwrap(),
                )
                .unwrap();
            }
            let proof = serde_json::json!({"syntheticOnly":true,"clock":"sourcePlaybackTicks","sourceImage":"latest actual source PTS <= requested; actual source PTS never evaluates overlays","mp4Clock":"rangeStart + explicit floor(sourceFPS*1000)/1000 CFR tick; real decoded MP4 timestamp must match within fixed 100 ns; native packet and decoded traces saved independently","cadenceLimitation":"milli-fps CFR, not original VFR or exact nominal NTSC; ordinary 15/30 unchanged","pixelTolerance":PIXEL_TOLERANCE,"cases":cases});
            fs::write(
                root.join("direct-composition-gate.json"),
                serde_json::to_vec_pretty(&proof).unwrap(),
            )
            .unwrap();
            assert!(
                proof["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["errors"].as_array().unwrap().is_empty()),
                "real alpha / z-order / endpoint mismatch; inspect fixed evidence"
            );
            let cancelled = AtomicBool::new(true);
            let output = root.join("cancelled.mp4");
            assert_eq!(
                crate::video_clip_backend::execute(
                    &ClipJob::RenderAnnotated {
                        source: root.join("cfr30-source.mp4"),
                        output: output.clone(),
                        expected: inspect_source(&root.join("cfr30-source.mp4"), &cancel),
                        range: Some(retained),
                        overlays
                    },
                    &cancelled,
                    &|_| {}
                )
                .unwrap_err(),
                ClipError::Cancelled
            );
            assert!(!output.exists());
        }
        fn close(actual: [u8; 3], expected: [u8; 3]) -> bool {
            actual
                .into_iter()
                .zip(expected)
                .all(|(a, e)| a.abs_diff(e) <= PIXEL_TOLERANCE)
        }
        fn over(base: [u8; 3], foreground: [u8; 3], alpha: u32) -> [u8; 3] {
            std::array::from_fn(|i| {
                ((u32::from(foreground[i]) * alpha + u32::from(base[i]) * (255 - alpha) + 127)
                    / 255) as u8
            })
        }
        #[test]
        #[ignore = "explicit synthetic native frame extraction/JPEG/file receipt gate; no devices/user media"]
        fn synthetic_frame_extract_files_decode_hash_tamper_cancel_and_source_immutability() {
            use crate::video_clip_contract::{ClipJob, ClipJobResult};
            let root = private_output();
            let _runtime = Runtime::new().unwrap();
            let source = root.join("原视频.mp4");
            make_source(&source, 30, 1, false).unwrap();
            let source_hash = sha(&source);
            let cancel = AtomicBool::new(false);
            let ClipJobResult::Inspected(expected) = crate::video_clip_backend::execute(
                &ClipJob::Inspect {
                    source: source.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("metadata");
            };
            let window = VideoRange {
                start_ticks: 0,
                end_ticks: expected.duration_ticks,
            };
            let requested_ticks = vec![
                2_150_000, 4_300_000, 6_200_000, 8_500_000, 14_000_000, 18_500_000,
            ];
            let directory = root.join("独立 帧目录");
            fs::create_dir(&directory).unwrap();
            let job = ClipJob::ExtractFrames {
                source: source.clone(),
                output_directory: directory.clone(),
                expected: expected.clone(),
                window: window.clone(),
                requested_ticks: requested_ticks.clone(),
            };
            let ClipJobResult::FramesExtracted { frames } =
                crate::video_clip_backend::execute(&job, &cancel, &|_| {
                    panic!("no export progress")
                })
                .unwrap()
            else {
                panic!("frame result");
            };
            validate_extracted_files(
                &directory,
                &expected,
                &window,
                &requested_ticks,
                &frames,
                &cancel,
            )
            .unwrap();
            let oracle = decode(&source, &cancel).unwrap();
            for frame in &frames {
                let image = image::open(directory.join(format!("{}.jpg", frame.index)))
                    .unwrap()
                    .to_rgb8();
                let mut code = 0;
                for bit in 0..6 {
                    let p = image.get_pixel(24 + bit * 32, 160);
                    if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) > 384 {
                        code |= 1 << bit;
                    }
                }
                let actual = oracle
                    .iter()
                    .find(|source| source.trace.pts_ticks == frame.actual_pts_ticks)
                    .unwrap();
                assert_eq!(
                    code, actual.trace.barcode,
                    "native file ordinal {}",
                    frame.index
                );
                assert_eq!(frame.requested_ticks, requested_ticks[frame.index as usize]);
                assert_eq!((frame.encoded_width, frame.encoded_height), (W, H));
            }
            let evidence = serde_json::json!({"syntheticOnly":true,"sourceSha256":source_hash,"frames":frames,
                "scope":"real native backend extraction/JPEG/independent file validation; product child supervisor tested separately"});
            fs::write(
                root.join("frame-extraction-gate.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
            let before = fs::read(directory.join("0.jpg")).unwrap();
            assert_eq!(
                crate::video_clip_backend::execute(&job, &cancel, &|_| {}).err(),
                Some(ClipError::FileUnavailable)
            );
            assert_eq!(
                fs::read(directory.join("0.jpg")).unwrap(),
                before,
                "existing frame never overwritten"
            );
            let changed = directory.join("1.jpg");
            let mut bytes = fs::read(&changed).unwrap();
            bytes[100] ^= 1;
            fs::write(&changed, &bytes).unwrap();
            assert_eq!(
                validate_extracted_files(
                    &directory,
                    &expected,
                    &window,
                    &requested_ticks,
                    &frames,
                    &cancel
                ),
                Err(ClipError::OutputInvalid)
            );
            let empty = root.join("取消目录");
            fs::create_dir(&empty).unwrap();
            let cancelled = ClipJob::ExtractFrames {
                source: source.clone(),
                output_directory: empty.clone(),
                expected: expected.clone(),
                window: window.clone(),
                requested_ticks: requested_ticks.clone(),
            };
            assert_eq!(
                crate::video_clip_backend::execute(&cancelled, &AtomicBool::new(true), &|_| {})
                    .err(),
                Some(ClipError::Cancelled)
            );
            assert!(fs::read_dir(empty).unwrap().next().is_none());
            assert_eq!(sha(&source), source_hash);
        }
    }
}
