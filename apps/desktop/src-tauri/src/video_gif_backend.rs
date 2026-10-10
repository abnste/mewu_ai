// SPDX-License-Identifier: MPL-2.0
//! Sequential, bounded GIF export. Paths arrive only after typed media leases.
use crate::video_clip_backend::check_cancel;
use crate::video_clip_contract::{
    ClipError, GifMetadata, VideoMetadata, VideoRange, MAX_GIF_BYTES,
};
use std::{
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
pub(crate) use platform::source_stream;

pub(crate) fn boundary(index: u32, total: u32, count: u32) -> u64 {
    (u64::from(index) * u64::from(total) + u64::from(count / 2)) / u64::from(count)
}
pub(crate) fn frame_delay(index: u32, metadata: &GifMetadata) -> u16 {
    (boundary(
        index + 1,
        metadata.duration_centiseconds,
        metadata.frame_count,
    ) - boundary(index, metadata.duration_centiseconds, metadata.frame_count)) as u16
}
fn frame_time(index: u32, duration: u64, count: u32) -> u64 {
    ((u128::from(index) * u128::from(duration) + u128::from(count / 2)) / u128::from(count))
        .min(u128::from(duration - 1)) as u64
}

struct Limited<'a, W> {
    inner: W,
    written: u64,
    limit: u64,
    cancel: &'a AtomicBool,
}
impl<W: Write> Write for Limited<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Interrupted would be retried by write_all rather than canceling.
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::other("cancelled"));
        }
        if bytes.len() as u64 > self.limit.saturating_sub(self.written) {
            return Err(io::Error::other("output budget"));
        }
        let count = self.inner.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Independent parent-side decode after the actual child/tree/I/O exit. Bounded
/// CPU buffers and file bytes, checking each complete frame and its exact delay.
pub fn validate_file(
    path: &Path,
    metadata: &GifMetadata,
    cancel: &AtomicBool,
) -> Result<(), ClipError> {
    validate_file_delays(path, metadata, None, cancel)
}
pub fn validate_annotated_file(
    path: &Path,
    metadata: &GifMetadata,
    delays: &[u16],
    cancel: &AtomicBool,
) -> Result<(), ClipError> {
    if delays.len() != metadata.frame_count as usize
        || delays.iter().map(|&n| u32::from(n)).sum::<u32>() != metadata.duration_centiseconds
        || delays.iter().any(|&n| n < 2)
    {
        return Err(ClipError::OutputInvalid);
    }
    validate_file_delays(path, metadata, Some(delays), cancel)
}
fn validate_file_delays(
    path: &Path,
    metadata: &GifMetadata,
    delays: Option<&[u16]>,
    cancel: &AtomicBool,
) -> Result<(), ClipError> {
    metadata.validate()?;
    check_cancel(cancel)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(path).map_err(|_| ClipError::FileUnavailable)?;
    let length = file
        .metadata()
        .map_err(|_| ClipError::FileUnavailable)?
        .len();
    if !(14..=MAX_GIF_BYTES).contains(&length) {
        return Err(ClipError::OutputInvalid);
    }
    // A decoder may accept EOF; require the producer's final GIF trailer too.
    file.seek(SeekFrom::End(-1))
        .map_err(|_| ClipError::OutputInvalid)?;
    let mut trailer = [0u8];
    file.read_exact(&mut trailer)
        .map_err(|_| ClipError::OutputInvalid)?;
    if trailer != [0x3b] {
        return Err(ClipError::OutputInvalid);
    }
    file.rewind().map_err(|_| ClipError::OutputInvalid)?;
    let mut decode = gif::DecodeOptions::new();
    decode.set_color_output(gif::ColorOutput::RGBA);
    decode.set_memory_limit(gif::MemoryLimit::Bytes(
        std::num::NonZeroU64::new(720 * 720 * 4).unwrap(),
    ));
    decode.check_frame_consistency(true);
    decode.check_lzw_end_code(true);
    let mut reader = decode
        .read_info(BufReader::new(file))
        .map_err(|_| ClipError::OutputInvalid)?;
    if u32::from(reader.width()) != metadata.width || u32::from(reader.height()) != metadata.height
    {
        return Err(ClipError::OutputInvalid);
    }
    let mut count = 0u32;
    loop {
        check_cancel(cancel)?;
        let Some(frame) = reader.read_next_frame().map_err(|error| {
            #[cfg(test)]
            eprintln!("GIF validation decode frame {count}: {error:?}");
            let _ = error;
            ClipError::OutputInvalid
        })?
        else {
            break;
        };
        if count >= metadata.frame_count
            || frame.left != 0
            || frame.top != 0
            || u32::from(frame.width) != metadata.width
            || u32::from(frame.height) != metadata.height
            || frame.transparent.is_some()
            || frame.delay
                != delays.map_or_else(|| frame_delay(count, metadata), |list| list[count as usize])
            || frame.buffer.len() != (metadata.width * metadata.height * 4) as usize
        {
            return Err(ClipError::OutputInvalid);
        }
        // read_next_frame stops when the exact pixel buffer is full. Consume
        // the remaining LZW data here: advancing directly to the next frame
        // skips those bytes in gif 0.14.2 and can miss a valid end code. One
        // RGBA pixel also detects any decoded data beyond the declared size.
        let mut extra_pixel = [0u8; 4];
        if reader.fill_buffer(&mut extra_pixel).map_err(|error| {
            #[cfg(test)]
            eprintln!("GIF validation end of frame {count}: {error:?}");
            let _ = error;
            ClipError::OutputInvalid
        })? {
            return Err(ClipError::OutputInvalid);
        }
        count += 1;
    }
    check_cancel(cancel)?;
    if count != metadata.frame_count || reader.repeat() != gif::Repeat::Infinite {
        return Err(ClipError::OutputInvalid);
    }
    Ok(())
}

#[cfg(windows)]
pub fn render(
    source: &Path,
    output: &Path,
    expected: &VideoMetadata,
    range: Option<&VideoRange>,
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<GifMetadata, ClipError> {
    check_cancel(cancel)?;
    platform::render(source, output, expected, range, cancel, progress).map_err(|error| {
        if cancel.load(Ordering::Acquire) {
            ClipError::Cancelled
        } else {
            error
                .downcast_ref::<ClipError>()
                .copied()
                .unwrap_or(ClipError::EncodingFailed)
        }
    })
}

#[cfg(windows)]
pub fn render_annotated(
    source: &Path,
    output: &Path,
    expected: &VideoMetadata,
    range: Option<&VideoRange>,
    overlays: &[crate::video_clip_contract::OverlayRaster],
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<GifMetadata, ClipError> {
    check_cancel(cancel)?;
    platform::render_annotated(source, output, expected, range, overlays, cancel, progress).map_err(
        |error| {
            if cancel.load(Ordering::Acquire) {
                ClipError::Cancelled
            } else {
                error
                    .downcast_ref::<ClipError>()
                    .copied()
                    .unwrap_or(ClipError::EncodingFailed)
            }
        },
    )
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        error::Error,
        fs::{self, OpenOptions},
        io::BufWriter,
    };
    type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
    fn check(cancel: &AtomicBool) -> Result<()> {
        check_cancel(cancel).map_err(Into::into)
    }

    pub(super) fn render(
        source: &Path,
        output: &Path,
        expected: &VideoMetadata,
        range: Option<&VideoRange>,
        cancel: &AtomicBool,
        progress: &dyn Fn(u8),
    ) -> Result<GifMetadata> {
        let metadata = GifMetadata::for_source(expected, range)?;
        let start = range.map_or(0, |r| r.start_ticks);
        let end = range.map_or(expected.duration_ticks, |r| r.end_ticks);
        let duration = end - start;
        let mut reader = source_stream::Reader::open(source, expected.width, expected.height)?;
        let mut current = loop {
            let frame = reader.next(cancel)?.ok_or(ClipError::InvalidRange)?;
            if frame.pts >= end {
                return Err(ClipError::InvalidRange.into());
            }
            if frame.pts >= start {
                break frame;
            }
        };
        let mut next = reader.next(cancel)?;
        check(cancel)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|_| ClipError::FileUnavailable)?;
        let result = (|| {
            let limited = Limited {
                inner: BufWriter::new(file),
                written: 0,
                limit: MAX_GIF_BYTES,
                cancel,
            };
            let mut encoder =
                gif::Encoder::new(limited, metadata.width as u16, metadata.height as u16, &[])?;
            encoder.set_repeat(gif::Repeat::Infinite)?;
            progress(0);
            let mut previous_percent = 0;
            for index in 0..metadata.frame_count {
                check(cancel)?;
                let target = start + frame_time(index, duration, metadata.frame_count);
                while next
                    .as_ref()
                    .is_some_and(|frame| frame.pts <= target && frame.pts < end)
                {
                    current = next.take().unwrap();
                    next = reader.next(cancel)?;
                }
                if current.pts < start || current.pts >= end {
                    return Err(ClipError::InvalidRange.into());
                }
                let rgba = reader.rgba(&current, cancel)?;
                let mut rgba = if rgba.width() == metadata.width && rgba.height() == metadata.height
                {
                    rgba.into_raw()
                } else {
                    image::imageops::resize(
                        &rgba,
                        metadata.width,
                        metadata.height,
                        image::imageops::FilterType::Triangle,
                    )
                    .into_raw()
                };
                check(cancel)?;
                let mut frame = gif::Frame::from_rgba_speed(
                    metadata.width as u16,
                    metadata.height as u16,
                    &mut rgba,
                    10,
                );
                frame.delay = frame_delay(index, &metadata);
                frame.dispose = gif::DisposalMethod::Background;
                check(cancel)?;
                encoder.write_frame(&frame)?;
                check(cancel)?;
                let percent = ((index + 1) * 99 / metadata.frame_count) as u8;
                if percent > previous_percent {
                    progress(percent);
                    previous_percent = percent;
                }
            }
            let mut writer = encoder.into_inner()?;
            writer.flush()?;
            writer.inner.get_ref().sync_all()?;
            check(cancel)?;
            drop(writer);
            validate_file(output, &metadata, cancel)?;
            progress(100);
            Ok(metadata)
        })();
        if result.is_err() {
            let _ = fs::remove_file(output);
        }
        result
    }
    pub(super) fn render_annotated(
        source: &Path,
        output: &Path,
        expected: &VideoMetadata,
        range: Option<&VideoRange>,
        overlays: &[crate::video_clip_contract::OverlayRaster],
        cancel: &AtomicBool,
        progress: &dyn Fn(u8),
    ) -> Result<GifMetadata> {
        let schedule =
            crate::video_clip_contract::AnnotatedGifSchedule::new(expected, range, overlays)?;
        let plan =
            crate::video_annotation_render::RasterPlan::load(expected, range, overlays, cancel)?;
        let mut input = crate::video_annotation_render::PlaybackFrames::open(
            source,
            expected,
            schedule.requested_ticks[0],
            cancel,
        )?;
        check(cancel)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|_| ClipError::FileUnavailable)?;
        let result = (|| {
            let limited = Limited {
                inner: BufWriter::new(file),
                written: 0,
                limit: MAX_GIF_BYTES,
                cancel,
            };
            let m = &schedule.metadata;
            let mut encoder = gif::Encoder::new(limited, m.width as u16, m.height as u16, &[])?;
            encoder.set_repeat(gif::Repeat::Infinite)?;
            progress(0);
            let mut previous = 0;
            for (index, &tick) in schedule.requested_ticks.iter().enumerate() {
                check(cancel)?;
                let time = crate::video_annotation_render::SourcePlaybackTicks(tick);
                let mut rgba = input.rgba_at(time, cancel)?;
                plan.paint(&mut rgba, time, cancel)?;
                let mut bytes = if rgba.dimensions() == (m.width, m.height) {
                    rgba.into_raw()
                } else {
                    image::imageops::resize(
                        &rgba,
                        m.width,
                        m.height,
                        image::imageops::FilterType::Triangle,
                    )
                    .into_raw()
                };
                check(cancel)?;
                let mut frame =
                    gif::Frame::from_rgba_speed(m.width as u16, m.height as u16, &mut bytes, 10);
                frame.delay = schedule.delays[index];
                frame.dispose = gif::DisposalMethod::Background;
                encoder.write_frame(&frame)?;
                check(cancel)?;
                let percent = ((index + 1) * 99 / schedule.requested_ticks.len()) as u8;
                if percent > previous {
                    progress(percent);
                    previous = percent;
                }
            }
            let mut writer = encoder.into_inner()?;
            writer.flush()?;
            writer.inner.get_ref().sync_all()?;
            drop(writer);
            validate_annotated_file(output, m, &schedule.delays, cancel)?;
            progress(100);
            Ok(schedule.metadata)
        })();
        if result.is_err() {
            let _ = fs::remove_file(output);
        }
        result
    }

    pub(crate) mod source_stream {
        // SPDX-License-Identifier: MPL-2.0
        //! One sequential decoded stream; keep only the current and next native sample.
        use super::{check, Result};
        use std::{path::Path, sync::atomic::AtomicBool};
        use windows::{
            core::{GUID, HSTRING, PCWSTR},
            Win32::Media::MediaFoundation::*,
        };
        struct Mf;
        impl Drop for Mf {
            fn drop(&mut self) {
                unsafe {
                    let _ = MFShutdown();
                }
            }
        }
        pub struct Frame {
            pub pts: u64,
            pub duration: u64,
            sample: IMFSample,
        }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        struct Layout {
            storage_width: u32,
            storage_height: u32,
            stride: i32,
            x: u32,
            y: u32,
        }
        unsafe fn aperture(media: &IMFMediaType, key: &GUID) -> Result<Option<MFVideoArea>> {
            let size = match media.GetBlobSize(key) {
                Ok(value) => value as usize,
                Err(error) if error.code() == MF_E_ATTRIBUTENOTFOUND => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if size != std::mem::size_of::<MFVideoArea>() {
                return Err("invalid display aperture".into());
            }
            let mut area = MFVideoArea::default();
            let bytes =
                std::slice::from_raw_parts_mut((&mut area as *mut MFVideoArea).cast::<u8>(), size);
            let mut actual = 0;
            media.GetBlob(key, bytes, Some(&mut actual))?;
            if actual as usize != size {
                return Err("display aperture changed".into());
            }
            Ok(Some(area))
        }
        unsafe fn layout(media: &IMFMediaType, width: u32, height: u32) -> Result<Layout> {
            if media.GetGUID(&MF_MT_SUBTYPE)? != MFVideoFormat_RGB32 {
                return Err("source subtype changed".into());
            }
            let dimensions = media.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (storage_width, storage_height) = ((dimensions >> 32) as u32, dimensions as u32);
            let stride = media.GetUINT32(&MF_MT_DEFAULT_STRIDE)? as i32;
            let pan_scan = match media.GetUINT32(&MF_MT_PAN_SCAN_ENABLED) {
                Ok(value) => value != 0,
                Err(error) if error.code() == MF_E_ATTRIBUTENOTFOUND => false,
                Err(error) => return Err(error.into()),
            };
            // Microsoft GetVideoDisplayArea: enabled pan-scan, minimum display,
            // geometric compatibility aperture, then the whole stored frame.
            let mut area = if pan_scan {
                aperture(media, &MF_MT_PAN_SCAN_APERTURE)?
            } else {
                None
            };
            if area.is_none() {
                area = aperture(media, &MF_MT_MINIMUM_DISPLAY_APERTURE)?;
            }
            if area.is_none() {
                area = aperture(media, &MF_MT_GEOMETRIC_APERTURE)?;
            }
            checked_layout(storage_width, storage_height, stride, width, height, area)
        }
        fn checked_layout(
            storage_width: u32,
            storage_height: u32,
            stride: i32,
            width: u32,
            height: u32,
            area: Option<MFVideoArea>,
        ) -> Result<Layout> {
            if width == 0
                || height == 0
                || width > 16384
                || height > 16384
                || u64::from(width) * u64::from(height) > 4_000_000
            {
                return Err("source display pixels".into());
            }
            // Codec storage padding has its own budget; a valid 4M display
            // must not be rejected merely because H.264 aligns its rows.
            if storage_width == 0
                || storage_height == 0
                || storage_width > 16384
                || storage_height > 16384
                || u64::from(storage_width) * u64::from(storage_height) > 5_000_000
            {
                return Err("source storage pixels".into());
            }
            if stride.unsigned_abs() < storage_width * 4
                || stride.unsigned_abs() > storage_width * 4 + 4096
                || u64::from(stride.unsigned_abs()) * u64::from(storage_height) > 20_000_000
            {
                return Err("source stride".into());
            }
            let (x, y, display_width, display_height) = if let Some(area) = area {
                // Native recorded sources have integral apertures. Reject fractional
                // cropping instead of rounding and silently moving content.
                if area.OffsetX.fract != 0 || area.OffsetY.fract != 0 {
                    return Err("fractional display aperture unsupported".into());
                }
                (
                    u32::try_from(area.OffsetX.value)?,
                    u32::try_from(area.OffsetY.value)?,
                    u32::try_from(area.Area.cx)?,
                    u32::try_from(area.Area.cy)?,
                )
            } else {
                (0, 0, storage_width, storage_height)
            };
            if display_width != width
                || display_height != height
                || x.checked_add(width)
                    .is_none_or(|right| right > storage_width)
                || y.checked_add(height)
                    .is_none_or(|bottom| bottom > storage_height)
            {
                return Err("source display dimensions changed".into());
            }
            Ok(Layout {
                storage_width,
                storage_height,
                stride,
                x,
                y,
            })
        }
        #[cfg(test)]
        mod tests {
            use super::*;
            fn area(width: i32, height: i32) -> MFVideoArea {
                let mut result = MFVideoArea::default();
                result.Area.cx = width;
                result.Area.cy = height;
                result
            }
            #[test]
            fn visible_aperture_and_storage_budgets_are_separate_and_exact() {
                assert!(
                    checked_layout(2000, 2016, 8000, 2000, 2000, Some(area(2000, 2000))).is_ok()
                );
                assert!(
                    checked_layout(2000, 2016, -8000, 2000, 2000, Some(area(2000, 2000))).is_ok()
                );
                assert!(checked_layout(2000, 2016, 8000, 2000, 2000, None).is_err());
                assert!(
                    checked_layout(2000, 2016, 8000, 2000, 2001, Some(area(2000, 2001))).is_err()
                );
                assert!(
                    checked_layout(2000, 2501, 8000, 2000, 2000, Some(area(2000, 2000))).is_err()
                );
                let mut fractional = area(320, 180);
                assert!(
                    checked_layout(2000, 2016, 12096, 2000, 2000, Some(area(2000, 2000))).is_err()
                );
                fractional.OffsetY.fract = 1;
                assert!(checked_layout(320, 192, 1280, 320, 180, Some(fractional)).is_err());
                let mut outside = area(320, 180);
                outside.OffsetY.value = 13;
                assert!(checked_layout(320, 192, 1280, 320, 180, Some(outside)).is_err());
            }
        }
        pub struct Reader {
            reader: IMFSourceReader,
            width: u32,
            height: u32,
            layout: Layout,
            previous: Option<u64>,
            eof: bool,
            read_calls: u64,
            read_limit: u64,
            _mf: Mf,
        }
        impl Reader {
            pub fn open(path: &Path, width: u32, height: u32) -> Result<Self> {
                unsafe {
                    if width == 0
                        || height == 0
                        || width > 16384
                        || height > 16384
                        || u64::from(width) * u64::from(height) > 4_000_000
                    {
                        return Err("source pixels".into());
                    }
                    MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
                    let mf = Mf;
                    let text = HSTRING::from(path.as_os_str());
                    let mut attributes = None;
                    MFCreateAttributes(&mut attributes, 1)?;
                    let attributes = attributes.ok_or("source attributes")?;
                    attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
                    let reader = MFCreateSourceReaderFromURL(PCWSTR(text.as_ptr()), &attributes)?;
                    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
                    reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                    reader.SetStreamSelection(stream, true)?;
                    let format = MFCreateMediaType()?;
                    format.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    format.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    reader.SetCurrentMediaType(stream, None, &format)?;
                    let current = reader.GetCurrentMediaType(stream)?;
                    let layout = layout(&current, width, height)?;
                    Ok(Self {
                        reader,
                        width,
                        height,
                        layout,
                        previous: None,
                        eof: false,
                        read_calls: 0,
                        read_limit: 500_000,
                        _mf: mf,
                    })
                }
            }
            /// Bounded input analysis may seek once before its first read. GIF
            /// export never calls this method and retains its original stream.
            pub fn seek_once(&mut self, ticks: u64, max_read_calls: u64) -> Result<()> {
                use windows::Win32::System::{
                    Com::StructuredStorage::{PropVariantClear, PROPVARIANT},
                    Variant::VT_I8,
                };
                if self.read_calls != 0
                    || self.previous.is_some()
                    || self.read_limit != 500_000
                    || ticks > i64::MAX as u64
                    || !(1..=240).contains(&max_read_calls)
                {
                    return Err("invalid bounded input seek".into());
                }
                unsafe {
                    let mut position = PROPVARIANT::default();
                    (*position.Anonymous.Anonymous).vt = VT_I8;
                    (*position.Anonymous.Anonymous).Anonymous.hVal = ticks as i64;
                    let result = self.reader.SetCurrentPosition(&GUID::zeroed(), &position);
                    let cleared = PropVariantClear(&mut position);
                    result?;
                    cleared?;
                }
                self.read_limit = max_read_calls;
                self.eof = false;
                Ok(())
            }
            /// Export may span the whole admitted source. Keep its original
            /// total decode bound after a single initial keyframe seek.
            pub fn seek_for_export(&mut self, ticks: u64) -> Result<()> {
                self.seek_once(ticks, 240)?;
                self.read_limit = 500_000;
                Ok(())
            }
            pub fn next(&mut self, cancel: &AtomicBool) -> Result<Option<Frame>> {
                unsafe {
                    if self.eof {
                        return Ok(None);
                    }
                    let mut empty_reads = 0u16;
                    loop {
                        check(cancel)?;
                        self.read_calls += 1;
                        if self.read_calls > self.read_limit {
                            return Err("decode work budget".into());
                        }
                        let (mut flags, mut time, mut sample) = (0, 0, None);
                        self.reader.ReadSample(
                            MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                            0,
                            None,
                            Some(&mut flags),
                            Some(&mut time),
                            Some(&mut sample),
                        )?;
                        check(cancel)?;
                        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                            return Err("decode error".into());
                        }
                        if flags
                            & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                                | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32)
                            != 0
                        {
                            let current = self.reader.GetCurrentMediaType(
                                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                            )?;
                            let next_layout = layout(&current, self.width, self.height)?;
                            if self.previous.is_some() && next_layout != self.layout {
                                return Err("source format changed".into());
                            }
                            // First decoded type can expose codec storage padding.
                            // Display remains exact; never reinterpret a retained sample.
                            self.layout = next_layout;
                        }
                        self.eof = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
                        if let Some(sample) = sample {
                            let pts = u64::try_from(time)?;
                            let duration = u64::try_from(sample.GetSampleDuration()?)?;
                            if duration == 0
                                || self.previous.is_some_and(|previous| pts <= previous)
                            {
                                return Err("source timestamp order".into());
                            }
                            self.previous = Some(pts);
                            return Ok(Some(Frame {
                                pts,
                                duration,
                                sample,
                            }));
                        }
                        if self.eof {
                            return Ok(None);
                        }
                        empty_reads += 1;
                        if empty_reads > 128 {
                            return Err("empty sample budget".into());
                        }
                    }
                }
            }
            pub fn rgba(&self, frame: &Frame, cancel: &AtomicBool) -> Result<image::RgbaImage> {
                unsafe {
                    check(cancel)?;
                    let buffer = frame.sample.ConvertToContiguousBuffer()?;
                    let (mut raw, mut length) = (std::ptr::null_mut(), 0);
                    buffer.Lock(&mut raw, None, Some(&mut length))?;
                    let result = (|| {
                        let pitch = self.layout.stride.unsigned_abs() as usize;
                        if raw.is_null()
                            || (length as usize) < pitch * self.layout.storage_height as usize
                        {
                            return Err("source buffer size".into());
                        }
                        let pixels = std::slice::from_raw_parts(raw, length as usize);
                        let mut rgba = image::RgbaImage::new(self.width, self.height);
                        for y in 0..self.height as usize {
                            check(cancel)?;
                            let display_row = y + self.layout.y as usize;
                            let row = if self.layout.stride >= 0 {
                                display_row
                            } else {
                                self.layout.storage_height as usize - 1 - display_row
                            };
                            let offset = row * pitch + self.layout.x as usize * 4;
                            let source = &pixels[offset..offset + self.width as usize * 4];
                            let target = &mut rgba.as_mut()
                                [y * self.width as usize * 4..(y + 1) * self.width as usize * 4];
                            for (from, to) in source.chunks_exact(4).zip(target.chunks_exact_mut(4))
                            {
                                to.copy_from_slice(&[from[2], from[1], from[0], 255]);
                            }
                        }
                        Ok::<image::RgbaImage, Box<dyn std::error::Error + Send + Sync>>(rgba)
                    })();
                    let unlocked = buffer.Unlock();
                    let rgba = result?;
                    unlocked?;
                    Ok(rgba)
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) struct TestDirectory {
    pub(crate) path: std::path::PathBuf,
    parent: std::path::PathBuf,
    leaf: String,
}
#[cfg(test)]
impl TestDirectory {
    pub(crate) fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let leaf = format!("mewu-video-test-{}", uuid::Uuid::new_v4());
        let path = parent.join(&leaf);
        std::fs::create_dir(&path).unwrap();
        assert_eq!(path.canonicalize().unwrap(), path);
        Self { path, parent, leaf }
    }
}
#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Check the final absolute target at cleanup time, never follow a
        // replacement symlink/reparse point into somebody else's directory.
        if let Ok(resolved) = self.path.canonicalize() {
            if resolved == self.parent.join(&self.leaf)
                && resolved.parent() == Some(self.parent.as_path())
                && self
                    .leaf
                    .strip_prefix("mewu-video-test-")
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                let _ = std::fs::remove_dir_all(&resolved);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(ticks: u64) -> VideoMetadata {
        VideoMetadata {
            duration_ticks: ticks,
            width: 320,
            height: 180,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            has_audio: false,
        }
    }
    #[test]
    fn cumulative_boundaries_do_not_drift_and_whole_short_source_is_supported() {
        for (ticks, count, total) in [
            (60_000_000, 90, 600),
            (30_000_000, 45, 300),
            (600_000_000, 240, 6000),
            (18_000_000_000, 240, 180000),
        ] {
            let metadata = GifMetadata::for_source(&source(ticks), None).unwrap();
            assert_eq!(metadata.frame_count, count);
            let mut sum = 0u64;
            for i in 0..count {
                assert!(frame_delay(i, &metadata) >= 2);
                assert!(
                    sum.checked_mul(u64::from(count))
                        .unwrap()
                        .abs_diff(u64::from(i) * total)
                        * 2
                        <= u64::from(count)
                );
                sum += u64::from(frame_delay(i, &metadata));
                assert!(frame_time(i, ticks, count) < ticks);
            }
            assert_eq!(sum, total);
        }
        assert_eq!(
            GifMetadata::for_source(&source(500_000), None)
                .unwrap()
                .frame_count,
            1
        );
        let large = VideoMetadata {
            width: 2000,
            height: 2000,
            ..source(600_000_000)
        };
        let meta = GifMetadata::for_source(&large, None).unwrap();
        assert!(
            u64::from(meta.width) * u64::from(meta.height) * u64::from(meta.frame_count)
                <= 24_000_000
        );
    }
    #[test]
    fn writer_cancellation_and_size_fail_without_retry_or_partial_prefix() {
        let cancel = AtomicBool::new(false);
        let mut writer = Limited {
            inner: Vec::new(),
            written: 0,
            limit: 3,
            cancel: &cancel,
        };
        assert!(writer.write_all(&[0; 4]).is_err());
        assert_eq!(writer.written, 0);
        cancel.store(true, Ordering::Release);
        assert_ne!(
            writer.write_all(&[1]).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }
    #[test]
    fn decoder_rejects_wrong_delay_truncated_trailer_and_extra_frames() {
        let dir = TestDirectory::new();
        let path = dir.path.join("proof.gif");
        let meta = GifMetadata {
            width: 2,
            height: 2,
            frame_count: 1,
            duration_centiseconds: 10,
        };
        for (delay, count, truncate, valid) in [
            (10, 1, false, true),
            (11, 1, false, false),
            (10, 2, false, false),
            (10, 1, true, false),
        ] {
            let mut encoder = gif::Encoder::new(Vec::new(), 2, 2, &[]).unwrap();
            encoder.set_repeat(gif::Repeat::Infinite).unwrap();
            for _ in 0..count {
                let mut pixels = vec![255; 16];
                let mut frame = gif::Frame::from_rgba_speed(2, 2, &mut pixels, 10);
                frame.delay = delay;
                encoder.write_frame(&frame).unwrap();
            }
            let mut bytes = encoder.into_inner().unwrap();
            if truncate {
                bytes.pop();
            }
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(
                validate_file(&path, &meta, &AtomicBool::new(false)).is_ok(),
                valid
            );
        }
        assert_eq!(
            validate_file(&path, &meta, &AtomicBool::new(true)),
            Err(ClipError::Cancelled)
        );
    }
    fn pre_encoded_gif(width: u16, height: u16, count: u32, lzw: &[u8]) -> Vec<u8> {
        let mut encoder =
            gif::Encoder::new(Vec::new(), width, height, &[0, 0, 0, 255, 255, 255]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for _ in 0..count {
            let frame = gif::Frame {
                width,
                height,
                delay: 10,
                buffer: std::borrow::Cow::Borrowed(lzw),
                ..Default::default()
            };
            encoder.write_lzw_pre_encoded_frame(&frame).unwrap();
        }
        encoder.into_inner().unwrap()
    }
    #[test]
    fn decoder_accepts_complete_frames_with_end_code_in_next_subblock() {
        // Reset before every pixel so all codes remain three bits wide. The
        // 340 pixels fill exactly one 255-byte GIF subblock; the valid EOI is
        // alone in the next subblock. Decoder prefetch is implementation-
        // dependent; this fixture checks validity, not where decoding pauses.
        let mut codes = Vec::with_capacity(681);
        for _ in 0..340 {
            codes.extend_from_slice(&[4u8, 0]);
        }
        codes.push(5);
        let mut lzw = vec![2u8];
        lzw.resize(1 + (codes.len() * 3).div_ceil(8), 0);
        for (index, code) in codes.into_iter().enumerate() {
            for bit in 0..3 {
                let position = index * 3 + bit;
                lzw[1 + position / 8] |= ((code >> bit) & 1) << (position % 8);
            }
        }
        assert_eq!(lzw.len(), 257);
        let bytes = pre_encoded_gif(20, 17, 2, &lzw);
        let dir = TestDirectory::new();
        let path = dir.path.join("split-end-code.gif");
        std::fs::write(&path, bytes).unwrap();
        let metadata = GifMetadata {
            width: 20,
            height: 17,
            frame_count: 2,
            duration_centiseconds: 20,
        };
        assert_eq!(
            validate_file(&path, &metadata, &AtomicBool::new(false)),
            Ok(())
        );
    }
    #[test]
    fn decoder_rejects_missing_end_code_and_one_extra_index_pixel() {
        let dir = TestDirectory::new();
        let path = dir.path.join("strict-end-code.gif");
        let metadata = GifMetadata {
            width: 1,
            height: 1,
            frame_count: 1,
            duration_centiseconds: 10,
        };
        // Two-bit palette, three-bit LSB-first codes. Clear=4, index=0,
        // EOI=5. The malformed fixtures retain a complete GIF trailer.
        for (lzw, expected) in [
            (&[2, 0x44, 0x01][..], Ok(())),                  // clear, pixel, EOI
            (&[2, 0x04][..], Err(ClipError::OutputInvalid)), // no EOI
            (&[2, 0x04, 0x0a][..], Err(ClipError::OutputInvalid)), // two pixels
        ] {
            std::fs::write(&path, pre_encoded_gif(1, 1, 1, lzw)).unwrap();
            assert_eq!(
                validate_file(&path, &metadata, &AtomicBool::new(false)),
                expected
            );
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "explicit local synthetic MP4 to GIF; no screen or audio devices"]
    fn synthetic_product_gif_timecodes_range_cancel_and_unicode() {
        use crate::video_clip_contract::{ClipJob, ClipJobResult};
        use sha2::{Digest, Sha256};
        use std::path::PathBuf;
        let fixtures =
            PathBuf::from(std::env::var_os("MEWU_SYNTHETIC_MEDIA_ROOT").expect(
                "set MEWU_SYNTHETIC_MEDIA_ROOT to the explicit synthetic fixture directory",
            ))
            .canonicalize()
            .unwrap();
        let inputs: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join("inputs.json")).unwrap()).unwrap();
        assert_eq!(inputs["syntheticOnly"], true);
        let directory = TestDirectory::new();
        let root = &directory.path;
        let silent = fixtures.join("synthetic-silent.mp4");
        let audio = fixtures.join("synthetic-audio.mp4");
        let silent_hash = Sha256::digest(std::fs::read(&silent).unwrap());
        let audio_hash = Sha256::digest(std::fs::read(&audio).unwrap());
        let unicode = root.join("原片 有声.mp4");
        std::fs::copy(&audio, &unicode).unwrap();
        let mut final_job = None;
        for (i, path, range) in [
            (0, silent.clone(), None),
            (
                1,
                audio.clone(),
                Some(VideoRange {
                    start_ticks: 12_500_000,
                    end_ticks: 42_500_000,
                }),
            ),
            (
                2,
                unicode.canonicalize().unwrap(),
                Some(VideoRange {
                    start_ticks: 12_500_000,
                    end_ticks: 42_500_000,
                }),
            ),
            (
                3,
                silent.clone(),
                Some(VideoRange {
                    start_ticks: 1_000_000,
                    end_ticks: 2_000_000,
                }),
            ),
        ] {
            let cancel = AtomicBool::new(false);
            let ClipJobResult::Inspected(source) = crate::video_clip_backend::execute(
                &ClipJob::Inspect {
                    source: path.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("inspect");
            };
            let output = root.join(format!("结果 {i}.gif"));
            let job = ClipJob::RenderGif {
                source: path,
                output: output.clone(),
                expected: source.clone(),
                range: range.clone(),
            };
            let ClipJobResult::GifRendered(meta) =
                crate::video_clip_backend::execute(&job, &cancel, &|_| {}).unwrap()
            else {
                panic!("gif");
            };
            validate_file(&output, &meta, &cancel).unwrap();
            let start = range.as_ref().map_or(0, |r| r.start_ticks);
            let end = range
                .as_ref()
                .map_or(source.duration_ticks, |r| r.end_ticks);
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = options
                .read_info(std::fs::File::open(&output).unwrap())
                .unwrap();
            for index in 0..meta.frame_count {
                let frame = decoder.read_next_frame().unwrap().unwrap();
                let mut code = 0u64;
                for bit in 0..5 {
                    let offset = (160 * 320 + 18 + bit * 24) * 4;
                    if frame.buffer[offset..offset + 3]
                        .iter()
                        .map(|&v| u32::from(v))
                        .sum::<u32>()
                        > 384
                    {
                        code |= 1 << bit;
                    }
                }
                let time = start + frame_time(index, end - start, meta.frame_count);
                assert_eq!(
                    code,
                    (time / 2_500_000).min(23),
                    "case {i} frame {index} source ticks {time}"
                );
            }
            assert!(decoder.read_next_frame().unwrap().is_none());
            drop(decoder);
            let before = std::fs::read(&output).unwrap();
            assert!(crate::video_clip_backend::execute(&job, &cancel, &|_| {}).is_err());
            assert_eq!(std::fs::read(&output).unwrap(), before);
            final_job = Some(job);
        }
        let ClipJob::RenderGif {
            source, expected, ..
        } = final_job.unwrap()
        else {
            unreachable!()
        };
        let cancel = AtomicBool::new(false);
        let output = root.join("取消.gif");
        let result = crate::video_clip_backend::execute(
            &ClipJob::RenderGif {
                source,
                output: output.clone(),
                expected,
                range: None,
            },
            &cancel,
            &|value| {
                if value > 0 {
                    cancel.store(true, Ordering::Release)
                }
            },
        );
        assert_eq!(result, Err(ClipError::Cancelled));
        assert!(!output.exists());
        assert_eq!(Sha256::digest(std::fs::read(&silent).unwrap()), silent_hash);
        assert_eq!(Sha256::digest(std::fs::read(&audio).unwrap()), audio_hash);
    }
}
