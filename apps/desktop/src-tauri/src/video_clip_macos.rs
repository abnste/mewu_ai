// SPDX-License-Identifier: MPL-2.0
//! Local AVFoundation work inside the existing owned video child. No GUI or Host locks.
//! The parent holds the actual process/IO permit and the 15-second inspect deadline.
use super::check_cancel;
use crate::video_clip_contract::{
    ClipError, ClipJob, ClipJobResult, VideoMetadata, MAX_SOURCE_BYTES,
};
use block2::RcBlock;
use objc2::{msg_send, rc::autoreleasepool, runtime::AnyObject};
use objc2_av_foundation::{
    AVAsset, AVAssetImageGenerator, AVAssetReferenceRestrictions, AVAsynchronousKeyValueLoading,
    AVKeyValueStatus, AVMediaTypeAudio, AVMediaTypeVideo, AVURLAsset,
    AVURLAssetPreferPreciseDurationAndTimingKey, AVURLAssetReferenceRestrictionsKey,
};
use objc2_core_graphics::CGImage;
use objc2_core_media::{CMTime, CMTimeFlags};
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
#[allow(deprecated)]
fn inspect(source: &mut Source, cancel: &AtomicBool) -> Result<VideoMetadata, ClipError> {
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
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, Some(&options)) };
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
pub(super) fn execute(
    job: &ClipJob,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(u8),
) -> Result<ClipJobResult, ClipError> {
    let ClipJob::Inspect { source } = job else {
        return Err(ClipError::Unsupported);
    };
    autoreleasepool(|_| {
        let mut source = Source::open(source, cancel)?;
        on_progress(0);
        let result = inspect(&mut source, cancel)?;
        check_cancel(cancel)?;
        on_progress(100);
        Ok(ClipJobResult::Inspected(result))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
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
