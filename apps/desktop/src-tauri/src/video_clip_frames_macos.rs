// SPDX-License-Identifier: MPL-2.0
//! Real decoded presentation intervals, bounded JPEGs and held-directory writes.
use super::{asset_for, check_cancel, inspect, loaded, time_ticks, Source};
use crate::video_clip_contract::{
    self as contract, ClipError, FrameDescriptor, VideoMetadata, VideoRange, MAX_FRAME_JPEG_BYTES,
    MAX_FRAME_TOTAL_BYTES, MAX_SOURCE_PIXELS,
};
use objc2::rc::Retained;
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVMediaTypeVideo,
};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, CVPixelBuffer,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetDataSize,
    CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
    CVPixelBufferIsPlanar, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn identity(file: &File) -> Result<(u64, u64), ClipError> {
    let m = file.metadata().map_err(|_| ClipError::FileUnavailable)?;
    Ok((m.dev(), m.ino()))
}
struct Directory {
    path: PathBuf,
    held: File,
    created: Vec<(CString, (u64, u64))>,
    committed: bool,
}
impl Directory {
    fn open(path: &Path) -> Result<Self, ClipError> {
        if !path.is_absolute()
            || path
                .canonicalize()
                .map_err(|_| ClipError::FileUnavailable)?
                != path
        {
            return Err(ClipError::UnsupportedPath);
        }
        let held = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| ClipError::FileUnavailable)?;
        let result = Self {
            path: path.into(),
            held,
            created: Vec::new(),
            committed: false,
        };
        result.check()?;
        if std::fs::read_dir(path)
            .map_err(|_| ClipError::FileUnavailable)?
            .next()
            .is_some()
        {
            return Err(ClipError::FileUnavailable);
        }
        Ok(result)
    }
    fn check(&self) -> Result<(), ClipError> {
        let m = std::fs::symlink_metadata(&self.path).map_err(|_| ClipError::FileUnavailable)?;
        if !m.is_dir() || m.file_type().is_symlink() || (m.dev(), m.ino()) != identity(&self.held)?
        {
            return Err(ClipError::FileUnavailable);
        }
        Ok(())
    }
    fn write(&mut self, index: usize, bytes: &[u8]) -> Result<(), ClipError> {
        self.check()?;
        let name = CString::new(format!("{index}.jpg")).unwrap();
        let fd = unsafe {
            libc::openat(
                self.held.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(ClipError::FileUnavailable);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        self.created.push((name, identity(&file)?));
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| ClipError::FileUnavailable)?;
        self.check()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if !self.committed {
            for (name, expected) in &self.created {
                let fd = unsafe {
                    libc::openat(
                        self.held.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd >= 0 {
                    let file = unsafe { File::from_raw_fd(fd) };
                    if identity(&file) == Ok(*expected) {
                        unsafe {
                            libc::unlinkat(self.held.as_raw_fd(), name.as_ptr(), 0);
                        }
                    }
                }
            }
        }
    }
}
struct ReaderFlight(Retained<AVAssetReader>);
impl Drop for ReaderFlight {
    fn drop(&mut self) {
        // This thread is the only caller of copyNextSampleBuffer/cancelReading.
        unsafe {
            self.0.cancelReading();
        }
    }
}
struct PixelLock<'a>(&'a CVPixelBuffer);
impl Drop for PixelLock<'_> {
    fn drop(&mut self) {
        unsafe {
            CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags::ReadOnly);
        }
    }
}
struct Jpeg(Vec<u8>);
impl Write for Jpeg {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > MAX_FRAME_JPEG_BYTES.saturating_sub(self.0.len() as u64) {
            return Err(std::io::Error::other("frame JPEG budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn pts_ticks(t: CMTime) -> Result<u64, ClipError> {
    if t.value == 0 && t.timescale > 0 && t.epoch == 0 && t.flags == CMTimeFlags::Valid {
        Ok(0)
    } else {
        time_ticks(t)
    }
}
fn decoded_jpeg(
    sample: &CMSampleBuffer,
    matrix: [i32; 4],
    encoded: (u32, u32),
    expected: &VideoMetadata,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, ClipError> {
    let pixel = unsafe { sample.image_buffer() }.ok_or(ClipError::InvalidMedia)?;
    if CVPixelBufferGetPixelFormatType(&pixel) != kCVPixelFormatType_32BGRA
        || CVPixelBufferIsPlanar(&pixel)
    {
        return Err(ClipError::InvalidMedia);
    }
    let (w, h) = (
        CVPixelBufferGetWidth(&pixel),
        CVPixelBufferGetHeight(&pixel),
    );
    let [a, b, c, d] = matrix;
    let (ow, oh) = (
        w * a.unsigned_abs() as usize + h * c.unsigned_abs() as usize,
        w * b.unsigned_abs() as usize + h * d.unsigned_abs() as usize,
    );
    if w == 0
        || h == 0
        || w.checked_mul(h)
            .is_none_or(|n| n as u64 > MAX_SOURCE_PIXELS)
        || (ow, oh) != (expected.width as usize, expected.height as usize)
    {
        return Err(ClipError::InvalidMedia);
    }
    if unsafe { CVPixelBufferLockBaseAddress(&pixel, CVPixelBufferLockFlags::ReadOnly) } != 0 {
        return Err(ClipError::InvalidMedia);
    }
    let lock = PixelLock(&pixel);
    let stride = CVPixelBufferGetBytesPerRow(&pixel);
    let size = stride.checked_mul(h).ok_or(ClipError::InvalidMedia)?;
    let base = CVPixelBufferGetBaseAddress(&pixel);
    if base.is_null()
        || stride < w * 4
        || size > CVPixelBufferGetDataSize(&pixel)
        || size > 64 * 1024 * 1024
    {
        return Err(ClipError::InvalidMedia);
    }
    // The retained buffer is locked read-only for the entire row/stride access.
    let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), size) };
    let mut rgb = image::RgbImage::new(ow as u32, oh as u32);
    for y in 0..h {
        check_cancel(cancel)?;
        for x in 0..w {
            let dx = if a != 0 {
                if a > 0 {
                    x
                } else {
                    w - 1 - x
                }
            } else if c > 0 {
                y
            } else {
                h - 1 - y
            };
            let dy = if b != 0 {
                if b > 0 {
                    x
                } else {
                    w - 1 - x
                }
            } else if d > 0 {
                y
            } else {
                h - 1 - y
            };
            let at = y * stride + x * 4;
            rgb.put_pixel(
                dx as u32,
                dy as u32,
                image::Rgb([bytes[at + 2], bytes[at + 1], bytes[at]]),
            );
        }
    }
    drop(lock);
    let rgb = if rgb.dimensions() == encoded {
        rgb
    } else {
        image::imageops::resize(
            &rgb,
            encoded.0,
            encoded.1,
            image::imageops::FilterType::Triangle,
        )
    };
    check_cancel(cancel)?;
    let mut jpeg = Jpeg(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&rgb)
        .map_err(|_| ClipError::OutputInvalid)?;
    check_cancel(cancel)?;
    Ok(jpeg.0)
}

#[allow(deprecated)]
pub(super) fn extract(
    source: &mut Source,
    output: &Path,
    expected: &VideoMetadata,
    window: &VideoRange,
    requested: &[u64],
    cancel: &AtomicBool,
) -> Result<Vec<FrameDescriptor>, ClipError> {
    contract::validate_extraction_request(expected, window, requested)?;
    let started = Instant::now();
    let check = || {
        check_cancel(cancel)?;
        if started.elapsed() >= Duration::from_secs(30) {
            return Err(ClipError::TimedOut);
        }
        Ok(())
    };
    check()?;
    if inspect(source, cancel)? != *expected {
        return Err(ClipError::SourceChanged);
    }
    let mut directory = Directory::open(output)?;
    let asset = asset_for(source)?;
    loaded(&asset, cancel)?;
    let tracks =
        unsafe { asset.tracksWithMediaType(AVMediaTypeVideo.ok_or(ClipError::WorkerUnavailable)?) };
    let track = tracks.firstObject().ok_or(ClipError::InvalidMedia)?;
    let transform = unsafe { track.preferredTransform() };
    let matrix = [
        transform.a.round() as i32,
        transform.b.round() as i32,
        transform.c.round() as i32,
        transform.d.round() as i32,
    ];
    // Inspect has admitted only finite orthogonal transforms. Normalize their
    // translation to the image bounds; do not silently ignore rotation/mirroring.
    let track_range = unsafe { track.timeRange() };
    let track_end = pts_ticks(track_range.start)?
        .checked_add(time_ticks(track_range.duration)?)
        .ok_or(ClipError::InvalidMedia)?
        .min(expected.duration_ticks);
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
        .map_err(|_| ClipError::InvalidMedia)?;
    let format = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_32BGRA);
    let key = NSString::from_str(&unsafe { kCVPixelBufferPixelFormatTypeKey }.to_string());
    let settings = NSDictionary::from_slices(&[&*key], &[&*format as &objc2::runtime::AnyObject]);
    let samples = unsafe {
        AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            &track,
            Some(&settings),
        )
    };
    unsafe {
        samples.setAlwaysCopiesSampleData(false);
        if !reader.canAddOutput(&samples) {
            return Err(ClipError::InvalidMedia);
        }
        reader.addOutput(&samples);
        if !reader.startReading() {
            return Err(ClipError::InvalidMedia);
        }
    }
    let flight = ReaderFlight(reader);
    let read = || -> Result<Option<(Retained<CMSampleBuffer>, u64, Option<u64>)>, ClipError> {
        check()?;
        let sample = unsafe { samples.copyNextSampleBuffer() };
        match sample {
            Some(sample) => {
                if unsafe { sample.num_samples() } != 1 {
                    return Err(ClipError::InvalidMedia);
                }
                let pts = pts_ticks(unsafe { sample.output_presentation_time_stamp() })?;
                let duration = time_ticks(unsafe { sample.output_duration() }).ok();
                Ok(Some((sample, pts, duration)))
            }
            None if unsafe { flight.0.status() } == AVAssetReaderStatus::Completed => Ok(None),
            None => Err(ClipError::InvalidMedia),
        }
    };
    let mut current = read()?.ok_or(ClipError::InvalidMedia)?;
    let mut frames = Vec::with_capacity(requested.len());
    let mut total = 0u64;
    loop {
        check()?;
        directory.check()?;
        let next = read()?;
        if next.as_ref().is_some_and(|v| v.1 <= current.1) {
            return Err(ClipError::InvalidMedia);
        }
        // Decoded buffers may omit duration. The next actual decoded PTS gives
        // their presentation interval; only a clean EOF admits the track end.
        let mut end = next.as_ref().map_or(track_end, |v| v.1).min(track_end);
        if let Some(duration) = current.2 {
            end = end.min(
                current
                    .1
                    .checked_add(duration)
                    .ok_or(ClipError::InvalidMedia)?,
            );
        }
        if end <= current.1 {
            return Err(ClipError::InvalidMedia);
        }
        let mut jpeg = None;
        while frames.len() < requested.len() && requested[frames.len()] < end {
            let index = frames.len();
            let tick = requested[index];
            if tick < current.1 {
                return Err(ClipError::InvalidRange);
            }
            if jpeg.is_none() {
                jpeg = Some(decoded_jpeg(
                    &current.0,
                    matrix,
                    contract::extracted_dimensions(expected),
                    expected,
                    cancel,
                )?);
            }
            let bytes = jpeg.as_ref().unwrap();
            total = total
                .checked_add(bytes.len() as u64)
                .ok_or(ClipError::OutputInvalid)?;
            if total > MAX_FRAME_TOTAL_BYTES {
                return Err(ClipError::OutputInvalid);
            }
            check()?;
            directory.write(index, bytes)?;
            let (width, height) = contract::extracted_dimensions(expected);
            frames.push(FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: current.1,
                sample_duration_ticks: end - current.1,
                encoded_width: width,
                encoded_height: height,
                jpeg_sha256: format!("{:x}", Sha256::digest(bytes)),
                bytes_length: bytes.len() as u64,
            });
        }
        if frames.len() == requested.len() {
            break;
        }
        current = next.ok_or(ClipError::InvalidRange)?;
    }
    drop(flight);
    check()?;
    source.check(cancel)?;
    directory.check()?;
    contract::validate_frame_descriptors(expected, window, requested, &frames)?;
    crate::video_annotation_render::validate_extracted_files(
        output, expected, window, requested, &frames, cancel,
    )?;
    check()?;
    source.check(cancel)?;
    directory.check()?;
    directory
        .held
        .sync_all()
        .map_err(|_| ClipError::FileUnavailable)?;
    directory.committed = true;
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!("mewu-mac-frames-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    #[test]
    fn existing_names_and_replaced_directories_are_preserved() {
        let root = root();
        let output = root.join("frames");
        std::fs::create_dir(&output).unwrap();
        let mut held = Directory::open(&output).unwrap();
        std::fs::write(output.join("0.jpg"), b"existing").unwrap();
        assert_eq!(held.write(0, b"new"), Err(ClipError::FileUnavailable));
        drop(held);
        assert_eq!(std::fs::read(output.join("0.jpg")).unwrap(), b"existing");
        assert!(Directory::open(&output).is_err());
        std::fs::remove_file(output.join("0.jpg")).unwrap();
        let mut held = Directory::open(&output).unwrap();
        held.write(0, b"owned").unwrap();
        let old = root.join("old");
        std::fs::rename(&output, &old).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("0.jpg"), b"unrelated").unwrap();
        assert_eq!(held.check(), Err(ClipError::FileUnavailable));
        drop(held);
        assert_eq!(std::fs::read_dir(&old).unwrap().count(), 0);
        assert_eq!(std::fs::read(output.join("0.jpg")).unwrap(), b"unrelated");
        std::fs::remove_file(output.join("0.jpg")).unwrap();
        std::fs::remove_dir(output).unwrap();
        std::fs::remove_dir(old).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn failed_frame_set_drops_only_its_own_uncommitted_files() {
        let root = root();
        let mut held = Directory::open(&root).unwrap();
        held.write(0, b"owned").unwrap();
        std::fs::remove_file(root.join("0.jpg")).unwrap();
        std::fs::write(root.join("0.jpg"), b"replacement").unwrap();
        held.write(1, b"owned1").unwrap();
        drop(held);
        assert!(!root.join("1.jpg").exists());
        assert_eq!(std::fs::read(root.join("0.jpg")).unwrap(), b"replacement");
        std::fs::remove_file(root.join("0.jpg")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    #[ignore = "requires the explicitly supplied owned H264/AAC Mac fixture"]
    fn native_decoded_intervals_jpeg_hashes_and_cancelled_writes_are_real() {
        use std::sync::{atomic::Ordering, Arc};
        let path =
            PathBuf::from(std::env::var_os("MEWU_MAC_MEDIA_TEST_SOURCE").expect("owned fixture"));
        let root = root();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut source = Source::open(&path, &cancel).unwrap();
        let expected = inspect(&mut source, &cancel).unwrap();
        let hash = source.hash;
        let window = VideoRange {
            start_ticks: 0,
            end_ticks: expected.duration_ticks,
        };
        let requested = [
            0,
            1,
            666_666,
            666_667,
            12_345_678,
            expected.duration_ticks - 1,
        ];
        let result = extract(&mut source, &root, &expected, &window, &requested, &cancel).unwrap();
        assert_eq!(result.len(), 6);
        assert_eq!(result[0].actual_pts_ticks, 0);
        assert_eq!(result[0].jpeg_sha256, result[1].jpeg_sha256);
        for (frame, &tick) in result.iter().zip(&requested) {
            assert!(frame.actual_pts_ticks <= tick);
            assert!(tick < frame.actual_pts_ticks + frame.sample_duration_ticks);
            let bytes = std::fs::read(root.join(format!("{}.jpg", frame.index))).unwrap();
            assert_eq!(frame.jpeg_sha256, format!("{:x}", Sha256::digest(&bytes)));
            assert_eq!(
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                    .unwrap()
                    .width(),
                expected.width
            );
        }
        source.check(&cancel).unwrap();
        assert_eq!(source.hash, hash);
        println!(
            "{}",
            serde_json::json!({"frames":result,"source":expected,"source_unchanged":true})
        );
        for index in 0..6 {
            std::fs::remove_file(root.join(format!("{index}.jpg"))).unwrap();
        }
        let watch_root = root.clone();
        let watch_cancel = cancel.clone();
        let watcher = std::thread::spawn(move || {
            let started = Instant::now();
            while !watch_root.join("0.jpg").exists() && started.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(1));
            }
            let observed = watch_root.join("0.jpg").exists();
            if observed {
                watch_cancel.store(true, Ordering::Release);
            }
            observed
        });
        let cancelled = extract(&mut source, &root, &expected, &window, &requested, &cancel);
        assert!(
            watcher.join().unwrap(),
            "must observe actual JPEG output, not an API-start cancellation"
        );
        assert_eq!(cancelled, Err(ClipError::Cancelled));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        cancel.store(false, Ordering::Release);
        source.check(&cancel).unwrap();
        assert_eq!(source.hash, hash);
        println!(
            "{}",
            serde_json::json!({"cancel_after_actual_jpeg_created":true,"directory_empty":true,"source_unchanged":true})
        );
        std::fs::remove_dir(root).unwrap();
    }
}
