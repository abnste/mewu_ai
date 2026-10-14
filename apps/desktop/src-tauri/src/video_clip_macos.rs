// SPDX-License-Identifier: MPL-2.0
//! Local AVFoundation work inside the existing owned video child. No GUI or Host locks.
//! The parent holds the actual process/IO permit and the 15-second inspect deadline.
use super::check_cancel;
use crate::video_clip_contract::{
    validate_rendered, ClipError, ClipJob, ClipJobResult, VideoMetadata, VideoRange,
    MAX_SOURCE_BYTES,
};
use crate::video_clip_output_macos::Destination;
use block2::RcBlock;
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    runtime::AnyObject,
};
use objc2_av_foundation::{
    AVAsset, AVAssetExportPresetHighestQuality, AVAssetExportSession, AVAssetExportSessionStatus,
    AVAssetImageGenerator, AVAssetReferenceRestrictions, AVAsynchronousKeyValueLoading,
    AVFileTypeMPEG4, AVKeyValueStatus, AVMediaTypeAudio, AVMediaTypeVideo,
    AVMutableVideoComposition, AVURLAsset, AVURLAssetPreferPreciseDurationAndTimingKey,
    AVURLAssetReferenceRestrictionsKey,
};
use objc2_core_graphics::CGImage;
use objc2_core_media::{kCMPersistentTrackID_Invalid, CMTime, CMTimeFlags, CMTimeRange};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString, NSURL};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[path = "video_clip_frames_macos.rs"]
mod frames;
#[path = "video_clip_gif_macos.rs"]
mod gif;
#[path = "video_clip_annotated_macos.rs"]
mod annotated;

struct Source {
    path: PathBuf,
    file: File,
    dev: u64,
    ino: u64,
    len: u64,
    hash: [u8; 32],
}
fn digest(file: &mut File, cancel: &AtomicBool) -> Result<[u8; 32], ClipError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ClipError::FileUnavailable)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        check_cancel(cancel)?;
        let n = file
            .read(&mut buffer)
            .map_err(|_| ClipError::FileUnavailable)?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or(ClipError::InvalidMedia)?;
        if total > MAX_SOURCE_BYTES {
            return Err(ClipError::InvalidMedia);
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
impl Source {
    fn open(path: &Path, cancel: &AtomicBool) -> Result<Self, ClipError> {
        if !path.is_absolute() {
            return Err(ClipError::UnsupportedPath);
        }
        let entry = std::fs::symlink_metadata(path).map_err(|_| ClipError::FileUnavailable)?;
        if !entry.is_file() || entry.file_type().is_symlink() {
            return Err(ClipError::InvalidMedia);
        }
        let path = path
            .canonicalize()
            .map_err(|_| ClipError::FileUnavailable)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| ClipError::FileUnavailable)?;
        let meta = file.metadata().map_err(|_| ClipError::FileUnavailable)?;
        if !meta.is_file() || !(12..=MAX_SOURCE_BYTES).contains(&meta.len()) {
            return Err(ClipError::InvalidMedia);
        }
        let mut magic = [0u8; 12];
        file.read_exact(&mut magic)
            .map_err(|_| ClipError::InvalidMedia)?;
        if &magic[4..8] != b"ftyp" {
            return Err(ClipError::InvalidMedia);
        }
        let hash = digest(&mut file, cancel)?;
        let mut result = Self {
            path,
            file,
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            hash,
        };
        result.check(cancel)?;
        Ok(result)
    }
    fn check(&mut self, cancel: &AtomicBool) -> Result<(), ClipError> {
        check_cancel(cancel)?;
        let path = std::fs::symlink_metadata(&self.path).map_err(|_| ClipError::SourceChanged)?;
        let opened = self.file.metadata().map_err(|_| ClipError::SourceChanged)?;
        if !path.is_file()
            || path.file_type().is_symlink()
            || [path.dev(), opened.dev()] != [self.dev, self.dev]
            || [path.ino(), opened.ino()] != [self.ino, self.ino]
            || [path.len(), opened.len()] != [self.len, self.len]
            || digest(&mut self.file, cancel)? != self.hash
        {
            return Err(ClipError::SourceChanged);
        }
        Ok(())
    }
}
fn time_ticks(time: CMTime) -> Result<u64, ClipError> {
    if time.epoch != 0
        || time.value <= 0
        || time.timescale <= 0
        || !time.flags.contains(CMTimeFlags::Valid)
        || time.flags.intersects(
            CMTimeFlags::PositiveInfinity | CMTimeFlags::NegativeInfinity | CMTimeFlags::Indefinite,
        )
    {
        return Err(ClipError::InvalidMedia);
    }
    let scale = time.timescale as u128;
    u64::try_from((time.value as u128 * 10_000_000 + scale / 2) / scale)
        .map_err(|_| ClipError::InvalidMedia)
}
fn dimension(value: f64) -> Result<u32, ClipError> {
    if !value.is_finite() || value < 2. || value > 16_384. || (value - value.round()).abs() > 0.001
    {
        return Err(ClipError::InvalidMedia);
    }
    Ok(value.round() as u32)
}
fn rate(value: f32) -> Result<(u32, u32), ClipError> {
    if !value.is_finite() || !(0.1..=240.).contains(&value) {
        return Err(ClipError::InvalidMedia);
    }
    let mut numerator = (f64::from(value) * 1000.).round() as u32;
    let mut denominator = 1000u32;
    let (mut a, mut b) = (numerator, denominator);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    numerator /= a;
    denominator /= a;
    Ok((numerator, denominator))
}
#[allow(deprecated)]
fn loaded(asset: &AVAsset, cancel: &AtomicBool) -> Result<(), ClipError> {
    let names = [
        "duration",
        "tracks",
        "readable",
        "hasProtectedContent",
        "providesPreciseDurationAndTiming",
    ];
    let strings: Vec<_> = names.into_iter().map(NSString::from_str).collect();
    let keys = NSArray::from_retained_slice(&strings);
    let (tx, rx) = mpsc::sync_channel(1);
    let done = RcBlock::new(move || {
        let _ = tx.try_send(());
    });
    unsafe {
        asset.loadValuesAsynchronouslyForKeys_completionHandler(&keys, Some(&done));
    }
    let mut cancelled = false;
    loop {
        if cancel.load(Ordering::Acquire) && !cancelled {
            cancelled = true;
            unsafe {
                asset.cancelLoading();
            }
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err(ClipError::WorkerUnavailable),
        }
    }
    check_cancel(cancel)?;
    for key in &strings {
        let status: isize = unsafe {
            msg_send![asset, statusOfValueForKey: &**key, error: std::ptr::null_mut::<*mut NSError>()]
        };
        if status != AVKeyValueStatus::Loaded.0 {
            return Err(ClipError::InvalidMedia);
        }
    }
    Ok(())
}
fn asset_for(source: &Source) -> Result<Retained<AVURLAsset>, ClipError> {
    let path = source.path.to_str().ok_or(ClipError::UnsupportedPath)?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let precise = NSNumber::numberWithBool(true);
    let restrictions =
        NSNumber::numberWithUnsignedInteger(AVAssetReferenceRestrictions::ForbidAll.bits());
    let values: [&AnyObject; 2] = [&precise, &restrictions];
    let options = NSDictionary::from_slices(
        &[
            unsafe { AVURLAssetPreferPreciseDurationAndTimingKey },
            unsafe { AVURLAssetReferenceRestrictionsKey },
        ],
        &values,
    );
    Ok(unsafe { AVURLAsset::URLAssetWithURL_options(&url, Some(&options)) })
}
#[allow(deprecated)]
fn inspect(source: &mut Source, cancel: &AtomicBool) -> Result<VideoMetadata, ClipError> {
    let asset = asset_for(source)?;
    loaded(&asset, cancel)?;
    if unsafe {
        asset.hasProtectedContent()
            || !asset.isReadable()
            || !asset.providesPreciseDurationAndTiming()
    } {
        return Err(ClipError::InvalidMedia);
    }
    let duration_ticks = time_ticks(unsafe { asset.duration() })?;
    let tracks =
        unsafe { asset.tracksWithMediaType(AVMediaTypeVideo.ok_or(ClipError::WorkerUnavailable)?) };
    if tracks.count() != 1 {
        return Err(ClipError::InvalidMedia);
    }
    let track = tracks.firstObject().ok_or(ClipError::InvalidMedia)?;
    let size = unsafe { track.naturalSize() };
    let transform = unsafe { track.preferredTransform() };
    if ![
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.tx,
        transform.ty,
    ]
    .iter()
    .all(|v| v.is_finite())
        || ![transform.a, transform.b, transform.c, transform.d]
            .iter()
            .all(|v| [-1., 0., 1.].iter().any(|n| (v - n).abs() < 0.000001))
        || (transform.a.abs() + transform.b.abs() - 1.).abs() > 0.000001
        || (transform.c.abs() + transform.d.abs() - 1.).abs() > 0.000001
        || (transform.a * transform.d - transform.b * transform.c).abs() < 0.999999
    {
        return Err(ClipError::InvalidMedia);
    }
    let width = dimension(size.width * transform.a.abs() + size.height * transform.c.abs())?;
    let height = dimension(size.width * transform.b.abs() + size.height * transform.d.abs())?;
    let (frame_rate_numerator, frame_rate_denominator) = rate(unsafe { track.nominalFrameRate() })?;
    let has_audio =
        unsafe { asset.tracksWithMediaType(AVMediaTypeAudio.ok_or(ClipError::WorkerUnavailable)?) }
            .count()
            > 0;
    let result = VideoMetadata {
        duration_ticks,
        width,
        height,
        frame_rate_numerator,
        frame_rate_denominator,
        has_audio,
    };
    result.validate()?;
    check_cancel(cancel)?;
    // Inspect admits a decodable visual sample, not only container claims.
    let generator = unsafe { AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset) };
    unsafe {
        generator.setAppliesPreferredTrackTransform(true);
    }
    let zero = CMTime {
        value: 0,
        timescale: 10_000_000,
        flags: CMTimeFlags::Valid,
        epoch: 0,
    };
    let image = unsafe { generator.copyCGImageAtTime_actualTime_error(zero, std::ptr::null_mut()) }
        .map_err(|_| ClipError::InvalidMedia)?;
    if CGImage::width(Some(&image)) != width as usize
        || CGImage::height(Some(&image)) != height as usize
    {
        return Err(ClipError::InvalidMedia);
    }
    source.check(cancel)?;
    Ok(result)
}

// Retain the session and its completion block until AVFoundation actually finishes.
// An unwind or cancellation must not let staging disappear under an active encoder.
struct ExportFlight {
    session: Retained<AVAssetExportSession>,
    receive: mpsc::Receiver<()>,
    _completion: RcBlock<dyn Fn()>,
    finished: bool,
}
impl ExportFlight {
    #[allow(deprecated)]
    fn start(session: Retained<AVAssetExportSession>) -> Self {
        let (send, receive) = mpsc::sync_channel(1);
        let completion: RcBlock<dyn Fn()> = RcBlock::new(move || {
            let _ = send.try_send(());
        });
        let result = Self {
            session,
            receive,
            _completion: completion,
            finished: false,
        };
        unsafe {
            result
                .session
                .exportAsynchronouslyWithCompletionHandler(&result._completion);
        }
        result
    }
    #[allow(deprecated)]
    fn wait(
        &mut self,
        output: &Destination,
        cancel: &AtomicBool,
        progress: &dyn Fn(u8),
    ) -> Result<(), ClipError> {
        let mut failure = None;
        let mut last = 0;
        progress(0);
        loop {
            if failure.is_none() {
                failure = if cancel.load(Ordering::Acquire) {
                    Some(ClipError::Cancelled)
                } else if let Err(error) = output.check() {
                    Some(error)
                } else if std::fs::symlink_metadata(output.path())
                    .is_ok_and(|v| v.len() > MAX_SOURCE_BYTES)
                {
                    Some(ClipError::OutputInvalid)
                } else {
                    None
                };
                if failure.is_some() {
                    unsafe {
                        self.session.cancelExport();
                    }
                }
            }
            // Keep waiting for the real completion after cancelExport. The owned
            // parent enforces the existing 600s render deadline and final IO drain.
            match self.receive.recv_timeout(Duration::from_millis(20)) {
                Ok(()) => {
                    self.finished = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(ClipError::WorkerUnavailable),
            }
            if failure.is_none() {
                let value = unsafe { self.session.progress() };
                if value.is_finite() {
                    let value = (value.clamp(0., 1.) * 99.).floor() as u8;
                    if value > last {
                        last = value;
                        progress(value);
                    }
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        check_cancel(cancel)?;
        if unsafe { self.session.status() } != AVAssetExportSessionStatus::Completed {
            return Err(ClipError::EncodingFailed);
        }
        Ok(())
    }
}
impl Drop for ExportFlight {
    #[allow(deprecated)]
    fn drop(&mut self) {
        if !self.finished {
            unsafe {
                self.session.cancelExport();
            }
            // Never turn a request to stop the encoder into false completion.
            // The parent can terminate this child at its unchanged deadline.
            if self.receive.recv().is_ok() {
                self.finished = true;
            }
        }
    }
}
fn at_ticks(ticks: u64) -> Result<CMTime, ClipError> {
    Ok(CMTime {
        value: ticks.try_into().map_err(|_| ClipError::InvalidRange)?,
        timescale: 10_000_000,
        flags: CMTimeFlags::Valid,
        epoch: 0,
    })
}
#[allow(deprecated)]
fn render(
    source: &mut Source,
    output: &Path,
    expected: &VideoMetadata,
    range: &VideoRange,
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<VideoMetadata, ClipError> {
    let original = inspect(source, cancel)?;
    if &original != expected {
        return Err(ClipError::SourceChanged);
    }
    original.validate_range(Some(range))?;
    let mut destination = Destination::new(output)?;
    let asset = asset_for(source)?;
    loaded(&asset, cancel)?;
    source.check(cancel)?;
    let session = unsafe {
        AVAssetExportSession::exportSessionWithAsset_presetName(
            &asset,
            AVAssetExportPresetHighestQuality,
        )
    }
    .ok_or(ClipError::EncodingFailed)?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(
        destination
            .path()
            .to_str()
            .ok_or(ClipError::UnsupportedPath)?,
    ));
    unsafe {
        // Fractional cuts must use the declared cadence rather than partial source-frame timing.
        let composition = AVMutableVideoComposition::videoCompositionWithPropertiesOfAsset(&asset);
        composition.setFrameDuration(CMTime {
            value: i64::from(original.frame_rate_denominator),
            timescale: original
                .frame_rate_numerator
                .try_into()
                .map_err(|_| ClipError::InvalidMedia)?,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
        composition.setSourceTrackIDForFrameTiming(kCMPersistentTrackID_Invalid);
        session.setVideoComposition(Some(&composition));
        session.setOutputURL(Some(&url));
        session.setOutputFileType(Some(AVFileTypeMPEG4.ok_or(ClipError::WorkerUnavailable)?));
        session.setTimeRange(CMTimeRange {
            start: at_ticks(range.start_ticks)?,
            duration: at_ticks(
                range
                    .end_ticks
                    .checked_sub(range.start_ticks)
                    .ok_or(ClipError::InvalidRange)?,
            )?,
        });
    }
    let mut flight = ExportFlight::start(session);
    flight.wait(&destination, cancel, progress)?;
    drop(flight);
    drop(asset);
    check_cancel(cancel)?;
    source.check(cancel)?;
    // Decode the actual exported container and first visual sample before publication.
    let output_error = |error| {
        if error == ClipError::Cancelled {
            error
        } else {
            ClipError::OutputInvalid
        }
    };
    let mut exported = Source::open(destination.path(), cancel).map_err(output_error)?;
    let actual = inspect(&mut exported, cancel).map_err(|error| {
        if error == ClipError::Cancelled {
            error
        } else {
            ClipError::OutputInvalid
        }
    })?;
    validate_rendered(&original, range, &actual)?;
    exported.check(cancel)?;
    source.check(cancel)?;
    destination.publish(&exported.file)?;
    check_cancel(cancel)?;
    source.check(cancel)?;
    exported.check(cancel).map_err(output_error)?;
    destination.check()?;
    destination.commit();
    progress(100);
    Ok(actual)
}
pub(super) fn execute(
    job: &ClipJob,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(u8),
) -> Result<ClipJobResult, ClipError> {
    autoreleasepool(|_| match job {
        ClipJob::Inspect { source } => {
            let mut source = Source::open(source, cancel)?;
            let result = inspect(&mut source, cancel)?;
            check_cancel(cancel)?;
            Ok(ClipJobResult::Inspected(result))
        }
        ClipJob::Render {
            source,
            output,
            expected,
            range,
        } => {
            let mut source = Source::open(source, cancel)?;
            render(&mut source, output, expected, range, cancel, on_progress)
                .map(ClipJobResult::Rendered)
        }
        ClipJob::ExtractFrames {
            source,
            output_directory,
            expected,
            window,
            requested_ticks,
        } => {
            let mut source = Source::open(source, cancel)?;
            frames::extract(
                &mut source,
                output_directory,
                expected,
                window,
                requested_ticks,
                cancel,
            )
            .map(|frames| ClipJobResult::FramesExtracted { frames })
        }
        ClipJob::RenderAnnotated {
            source,
            output,
            expected,
            range,
            overlays,
        } => {
            let mut source = Source::open(source, cancel)?;
            annotated::render(
                &mut source,
                output,
                expected,
                range.as_ref(),
                overlays,
                cancel,
                on_progress,
            )
            .map(ClipJobResult::Rendered)
        }
        ClipJob::RenderGif {
            source,
            output,
            expected,
            range,
        } => {
            let mut source = Source::open(source, cancel)?;
            gif::render(
                &mut source,
                output,
                expected,
                range.as_ref(),
                &[],
                cancel,
                on_progress,
            )
            .map(ClipJobResult::GifRendered)
        }
        ClipJob::RenderGifAnnotated {
            source,
            output,
            expected,
            range,
            overlays,
        } => {
            let mut source = Source::open(source, cancel)?;
            gif::render(
                &mut source,
                output,
                expected,
                range.as_ref(),
                overlays,
                cancel,
                on_progress,
            )
            .map(ClipJobResult::GifRendered)
        }
        _ => Err(ClipError::Unsupported),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the explicitly supplied, owned H264/AAC Mac media fixture"]
    fn native_mp4_render_decodes_output_preserves_source_and_cancels_started_session() {
        let path = PathBuf::from(
            std::env::var_os("MEWU_MAC_MEDIA_TEST_SOURCE").expect("owned fixture path"),
        );
        let directory =
            std::env::temp_dir().join(format!("mewu-mac-render-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let directory = directory.canonicalize().unwrap();
        let cancel = AtomicBool::new(false);
        let (expected, source_hash) = autoreleasepool(|_| {
            let mut source = Source::open(&path, &cancel).unwrap();
            (inspect(&mut source, &cancel).unwrap(), source.hash)
        });
        assert!(expected.has_audio);
        assert!(expected.duration_ticks > 37_654_321);
        for (index, range) in [
            VideoRange {
                start_ticks: 10_000_000,
                end_ticks: expected.duration_ticks,
            },
            // The real GUI range that previously failed output validation.
            VideoRange {
                start_ticks: 10_882_315,
                end_ticks: expected.duration_ticks,
            },
            VideoRange {
                start_ticks: 0,
                end_ticks: 32_345_679,
            },
            VideoRange {
                start_ticks: 1_234_567,
                end_ticks: 37_654_321,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let output = directory.join(format!("实际裁剪-{index}.mp4"));
            let job = ClipJob::Render {
                source: path.clone(),
                output: output.clone(),
                expected: expected.clone(),
                range: range.clone(),
            };
            let percent = std::cell::RefCell::new(Vec::new());
            let ClipJobResult::Rendered(actual) =
                execute(&job, &cancel, &|value| percent.borrow_mut().push(value)).unwrap()
            else {
                panic!("real rendered result");
            };
            validate_rendered(&expected, &range, &actual).unwrap();
            assert!(percent.borrow().windows(2).all(|pair| pair[0] < pair[1]));
            assert_eq!(percent.borrow().last(), Some(&100));
            let retained = std::fs::read(&output).unwrap();
            assert_eq!(
                execute(&job, &cancel, &|_| {}),
                Err(ClipError::FileUnavailable)
            );
            assert_eq!(std::fs::read(&output).unwrap(), retained);
            println!(
                "{}",
                serde_json::json!({
                    "source": expected,
                    "range": range,
                    "rendered": actual,
                    "output_bytes": retained.len(),
                    "output_sha256": format!("{:x}", Sha256::digest(&retained)),
                    "progress": *percent.borrow(),
                    "existing_output_preserved": true,
                })
            );
            std::fs::remove_file(&output).unwrap();
            let started = AtomicBool::new(false);
            assert_eq!(
                execute(&job, &cancel, &|value| {
                    if value == 0 {
                        started.store(true, Ordering::Release);
                        cancel.store(true, Ordering::Release);
                    }
                }),
                Err(ClipError::Cancelled)
            );
            assert!(started.load(Ordering::Acquire));
            assert!(!output.exists());
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            cancel.store(false, Ordering::Release);
            assert_eq!(Source::open(&path, &cancel).unwrap().hash, source_hash);
            println!(
                "{}",
                serde_json::json!({
                    "cancelled_after_export_api_started": true,
                    "final_output_absent": true,
                    "private_directory_empty": true,
                    "source_sha256": source_hash.iter().map(|v| format!("{v:02x}")).collect::<String>(),
                    "source_unchanged": true,
                })
            );
        }
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn metadata_rejects_indefinite_time_and_non_integral_dimensions() {
        let t = |value, scale, flags| CMTime {
            value,
            timescale: scale,
            flags,
            epoch: 0,
        };
        assert_eq!(time_ticks(t(1, 3, CMTimeFlags::Valid)), Ok(3_333_333));
        assert_eq!(
            time_ticks(t(0, 1, CMTimeFlags::Valid)),
            Err(ClipError::InvalidMedia)
        );
        assert_eq!(
            time_ticks(t(1, 0, CMTimeFlags::Valid)),
            Err(ClipError::InvalidMedia)
        );
        assert_eq!(
            time_ticks(t(1, 1, CMTimeFlags::Valid | CMTimeFlags::Indefinite)),
            Err(ClipError::InvalidMedia)
        );
        for invalid in [f64::NAN, f64::INFINITY, 1., 2.5, 16_385.] {
            assert_eq!(dimension(invalid), Err(ClipError::InvalidMedia));
        }
        assert_eq!(rate(29.97), Ok((2997, 100)));
        assert_eq!(rate(f32::NAN), Err(ClipError::InvalidMedia));
    }

    #[test]
    fn opened_source_rejects_replacement_even_with_identical_bytes_and_rejects_mutation() {
        let directory =
            std::env::temp_dir().join(format!("mewu-mac-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("source.mp4");
        let replacement = directory.join("replacement.mp4");
        let mut bytes = vec![0u8; 64];
        bytes[4..8].copy_from_slice(b"ftyp");
        std::fs::write(&path, &bytes).unwrap();
        let cancel = AtomicBool::new(false);
        let mut source = Source::open(&path, &cancel).unwrap();
        std::fs::write(&replacement, &bytes).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(source.check(&cancel), Err(ClipError::SourceChanged));
        let mut source = Source::open(&path, &cancel).unwrap();
        bytes[63] = 1;
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(source.check(&cancel), Err(ClipError::SourceChanged));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
