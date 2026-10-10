// SPDX-License-Identifier: MPL-2.0
//! MediaComposition runs only in the supervised child, never under host locks.
use crate::video_clip_contract::{ClipError, ClipJob, ClipJobResult};
use std::sync::atomic::{AtomicBool, Ordering};

pub fn check_cancel(cancel: &AtomicBool) -> Result<(), ClipError> {
    if cancel.load(Ordering::Acquire) {
        Err(ClipError::Cancelled)
    } else {
        Ok(())
    }
}
pub fn execute(
    job: &ClipJob,
    cancel: &AtomicBool,
    on_progress: &dyn Fn(u8),
) -> Result<ClipJobResult, ClipError> {
    check_cancel(cancel)?;
    job.validate()?;
    #[cfg(windows)]
    {
        platform::execute(job, cancel, on_progress)
    }
    #[cfg(not(windows))]
    {
        let _ = on_progress;
        Err(ClipError::Unsupported)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::video_clip_contract::{validate_rendered, VideoMetadata, MAX_SOURCE_BYTES};
    use std::{
        ffi::OsString,
        fs::{self, File, OpenOptions},
        io::Read,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            fs::OpenOptionsExt,
        },
        path::{Component, Path, PathBuf, Prefix},
        sync::{atomic::AtomicU8, Arc},
        time::Duration,
    };
    use windows::core::Interface;
    use windows::{
        core::{RuntimeType, HSTRING},
        Foundation::TimeSpan,
        Media::{
            Editing::{MediaClip, MediaComposition, MediaTrimmingPreference},
            MediaProperties::{MediaEncodingProfile, VideoEncodingQuality},
            Transcoding::TranscodeFailureReason,
        },
        Storage::{CreationCollisionOption, StorageFile, StorageFolder},
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    use windows_future::{AsyncOperationProgressHandler, IAsyncInfo, IAsyncOperation};

    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self, ClipError> {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|_| ClipError::WorkerUnavailable)?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }

    /// Cancel/Close only after terminal status. A stuck native call is terminated
    /// by the parent's Job; dropping an unfinished future is not its cleanup.
    fn wait_info(
        info: &IAsyncInfo,
        cancel: &AtomicBool,
        error: ClipError,
        poll: &dyn Fn(),
    ) -> Result<(), ClipError> {
        let mut cancelled = false;
        loop {
            if cancel.load(Ordering::Acquire) && !cancelled {
                let _ = info.Cancel();
                cancelled = true;
            }
            match info.Status().map_err(|_| error)?.0 {
                0 => {
                    poll();
                    std::thread::sleep(Duration::from_millis(10));
                }
                1 if !cancelled => return Ok(()),
                _ => {
                    let _ = info.Close();
                    return Err(if cancelled {
                        ClipError::Cancelled
                    } else {
                        error
                    });
                }
            }
        }
    }
    fn wait<T: RuntimeType + 'static>(
        op: IAsyncOperation<T>,
        cancel: &AtomicBool,
        error: ClipError,
    ) -> Result<T, ClipError> {
        let info: IAsyncInfo = op.cast().map_err(|_| error)?;
        wait_info(&info, cancel, error, &|| {})?;
        let result = op.GetResults().map_err(|_| error);
        let _ = info.Close();
        check_cancel(cancel)?;
        result
    }

    /// Convert only typed absolute filesystem prefixes. File identity is checked
    /// separately; verbatim trailing dots/spaces must never become another file.
    fn ordinary_path(path: &Path) -> Result<PathBuf, ClipError> {
        if !path.is_absolute() {
            return Err(ClipError::UnsupportedPath);
        }
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(ClipError::UnsupportedPath);
        };
        let mut wide: Vec<u16> = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => vec![u16::from(drive), 58, 92],
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                let mut result = vec![92, 92];
                result.extend(server.encode_wide());
                result.push(92);
                result.extend(share.encode_wide());
                result.push(92);
                result
            }
            _ => return Err(ClipError::UnsupportedPath),
        };
        for component in components {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    let name: Vec<u16> = name.encode_wide().collect();
                    if name.is_empty()
                        || name.last().is_some_and(|c| matches!(c, 32 | 46))
                        || name
                            .iter()
                            .any(|c| matches!(c, 0 | 47 | 58 | 92) || *c < 32)
                    {
                        return Err(ClipError::UnsupportedPath);
                    }
                    if wide.last() != Some(&92) {
                        wide.push(92);
                    }
                    wide.extend(name);
                }
                _ => return Err(ClipError::UnsupportedPath),
            }
        }
        if wide.len() > 32_000 {
            return Err(ClipError::UnsupportedPath);
        }
        Ok(PathBuf::from(OsString::from_wide(&wide)))
    }
    fn hstring(path: &Path) -> HSTRING {
        HSTRING::from(path.as_os_str())
    }
    fn same(first: &Path, second: &Path) -> Result<(), ClipError> {
        if same_file::is_same_file(first, second).map_err(|_| ClipError::FileUnavailable)? {
            Ok(())
        } else {
            Err(ClipError::SourceChanged)
        }
    }
    struct Source {
        // No write/delete sharing: WinRT can read, concurrent replacement cannot
        // change the bytes halfway through a media decoder's path-based open.
        _file: File,
        normal: PathBuf,
    }
    impl Source {
        fn open(path: &Path) -> Result<Self, ClipError> {
            let canonical = path
                .canonicalize()
                .map_err(|_| ClipError::FileUnavailable)?;
            let normal = ordinary_path(&canonical)?;
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&canonical)
                .map_err(|_| ClipError::FileUnavailable)?;
            let meta = file.metadata().map_err(|_| ClipError::FileUnavailable)?;
            if !meta.is_file() || !(12..=MAX_SOURCE_BYTES).contains(&meta.len()) {
                return Err(ClipError::InvalidMedia);
            }
            same(&canonical, &normal)?;
            let mut magic = [0; 12];
            file.read_exact(&mut magic)
                .map_err(|_| ClipError::InvalidMedia)?;
            if &magic[4..8] != b"ftyp" {
                return Err(ClipError::InvalidMedia);
            }
            Ok(Self {
                _file: file,
                normal,
            })
        }
        fn storage_file(&self, cancel: &AtomicBool) -> Result<StorageFile, ClipError> {
            wait(
                StorageFile::GetFileFromPathAsync(&hstring(&self.normal))
                    .map_err(|_| ClipError::FileUnavailable)?,
                cancel,
                ClipError::FileUnavailable,
            )
        }
    }
    struct Destination {
        normal: PathBuf,
        _directory: File,
    }
    impl Destination {
        fn new(path: &Path) -> Result<Self, ClipError> {
            Self::with_extension(path, "mp4")
        }
        fn with_extension(path: &Path, extension: &str) -> Result<Self, ClipError> {
            if !path.is_absolute()
                || path.exists()
                || path.extension().and_then(|v| v.to_str()) != Some(extension)
            {
                return Err(ClipError::FileUnavailable);
            }
            let parent = path
                .parent()
                .ok_or(ClipError::UnsupportedPath)?
                .canonicalize()
                .map_err(|_| ClipError::FileUnavailable)?;
            let normal_parent = ordinary_path(&parent)?;
            let directory = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02000000)
                .open(&parent)
                .map_err(|_| ClipError::FileUnavailable)?;
            same(&parent, &normal_parent)?;
            let normal = ordinary_path(
                &normal_parent.join(path.file_name().ok_or(ClipError::UnsupportedPath)?),
            )?;
            Ok(Self {
                normal,
                _directory: directory,
            })
        }
        fn create(&self, cancel: &AtomicBool) -> Result<StorageFile, ClipError> {
            let parent = self.normal.parent().ok_or(ClipError::UnsupportedPath)?;
            let folder = wait(
                StorageFolder::GetFolderFromPathAsync(&hstring(parent))
                    .map_err(|_| ClipError::FileUnavailable)?,
                cancel,
                ClipError::FileUnavailable,
            )?;
            let name = HSTRING::from(self.normal.file_name().ok_or(ClipError::UnsupportedPath)?);
            wait(
                folder
                    .CreateFileAsync(&name, CreationCollisionOption::FailIfExists)
                    .map_err(|_| ClipError::FileUnavailable)?,
                cancel,
                ClipError::FileUnavailable,
            )
        }
    }
    #[cfg(test)]
    fn preserve_synthetic_failure(output: &Path) {
        // Explicit gate-only evidence. Normal tests and production cannot
        // preserve an arbitrary export or write outside repository .private.
        let Some(configured) = std::env::var_os("MEWU_VIDEO_ANNOTATION_TEST_OUTPUT") else {
            return;
        };
        let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) else {
            return;
        };
        let (Ok(private), Ok(root), Ok(actual)) = (
            repo.join(".private").canonicalize(),
            PathBuf::from(configured).canonicalize(),
            output.canonicalize(),
        ) else {
            return;
        };
        if !root.starts_with(private) || actual.parent() != Some(root.as_path()) {
            return;
        }
        let evidence = actual.with_extension("failed-validation.mp4");
        let saved = (|| -> std::io::Result<()> {
            use std::io::{Read, Write};
            let mut input = fs::File::open(&actual)?;
            if !(12..=32 * 1024 * 1024).contains(&input.metadata()?.len()) {
                return Err(std::io::Error::other("synthetic evidence budget"));
            }
            let mut bytes = Vec::new();
            (&mut input)
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err(std::io::Error::other("synthetic evidence budget"));
            }
            let mut copy = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(evidence)?;
            copy.write_all(&bytes)?;
            copy.sync_all()
        })();
        if saved.is_err() {
            eprintln!("synthetic failed-output evidence unavailable");
        }
    }
    fn metadata(clip: &MediaClip) -> Result<VideoMetadata, ClipError> {
        let get = || -> windows::core::Result<VideoMetadata> {
            let video = clip.GetVideoEncodingProperties()?;
            let tracks = clip.EmbeddedAudioTracks()?.Size()?;
            // MediaComposition selects one track; do not silently discard more.
            if tracks > 1 {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070057_u32 as i32,
                )));
            }
            Ok(VideoMetadata {
                duration_ticks: clip.OriginalDuration()?.Duration.try_into().unwrap_or(0),
                width: video.Width()?,
                height: video.Height()?,
                frame_rate_numerator: video.FrameRate()?.Numerator()?,
                frame_rate_denominator: video.FrameRate()?.Denominator()?,
                has_audio: tracks != 0,
            })
        };
        let result = get().map_err(|_| ClipError::InvalidMedia)?;
        result.validate()?;
        Ok(result)
    }
    fn profile(
        clip: &MediaClip,
        file: &StorageFile,
        info: &VideoMetadata,
        cancel: &AtomicBool,
    ) -> Result<MediaEncodingProfile, ClipError> {
        let build = || -> windows::core::Result<MediaEncodingProfile> {
            let source = clip.GetVideoEncodingProperties()?;
            let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::Auto)?;
            let video = profile.Video()?;
            video.SetWidth(info.width)?;
            video.SetHeight(info.height)?;
            video.FrameRate()?.SetNumerator(info.frame_rate_numerator)?;
            video
                .FrameRate()?
                .SetDenominator(info.frame_rate_denominator)?;
            video
                .PixelAspectRatio()?
                .SetNumerator(source.PixelAspectRatio()?.Numerator()?)?;
            video
                .PixelAspectRatio()?
                .SetDenominator(source.PixelAspectRatio()?.Denominator()?)?;
            if source.Bitrate()? > 0 {
                video.SetBitrate(source.Bitrate()?)?;
            }
            Ok(profile)
        };
        let result = build().map_err(|_| ClipError::EncodingFailed)?;
        if !info.has_audio {
            result
                .SetAudio(None)
                .map_err(|_| ClipError::EncodingFailed)?;
        } else {
            let stream = wait(
                file.OpenReadAsync().map_err(|_| ClipError::InvalidMedia)?,
                cancel,
                ClipError::InvalidMedia,
            )?;
            let read = (|| {
                let input = wait(
                    MediaEncodingProfile::CreateFromStreamAsync(&stream)
                        .map_err(|_| ClipError::InvalidMedia)?,
                    cancel,
                    ClipError::InvalidMedia,
                )?;
                let set = || -> windows::core::Result<()> {
                    let audio = input.Audio()?;
                    let output = result.Audio()?;
                    output.SetChannelCount(audio.ChannelCount()?)?;
                    output.SetSampleRate(audio.SampleRate()?)?;
                    if audio.Bitrate()? > 0 {
                        output.SetBitrate(audio.Bitrate()?)?;
                    }
                    Ok(())
                };
                set().map_err(|_| ClipError::EncodingFailed)
            })();
            let _ = stream.Close();
            read?;
        }
        Ok(result)
    }
    pub(super) fn execute(
        job: &ClipJob,
        cancel: &AtomicBool,
        progress: &dyn Fn(u8),
    ) -> Result<ClipJobResult, ClipError> {
        let _apartment = Apartment::new()?;
        let source = match job {
            ClipJob::Inspect { source }
            | ClipJob::Render { source, .. }
            | ClipJob::RenderGif { source, .. }
            | ClipJob::RenderAnnotated { source, .. }
            | ClipJob::RenderGifAnnotated { source, .. }
            | ClipJob::ExtractFrames { source, .. } => source,
        };
        let source = Source::open(source)?;
        check_cancel(cancel)?;
        let file = source.storage_file(cancel)?;
        let clip = wait(
            MediaClip::CreateFromFileAsync(&file).map_err(|_| ClipError::InvalidMedia)?,
            cancel,
            ClipError::InvalidMedia,
        )?;
        let original = metadata(&clip)?;
        if let ClipJob::RenderGifAnnotated {
            output,
            expected,
            range,
            overlays,
            ..
        } = job
        {
            if &original != expected {
                return Err(ClipError::SourceChanged);
            }
            let destination = Destination::with_extension(output, "gif")?;
            return crate::video_gif_backend::render_annotated(
                &source.normal,
                &destination.normal,
                &original,
                range.as_ref(),
                overlays,
                cancel,
                progress,
            )
            .map(ClipJobResult::GifRendered);
        }
        if let ClipJob::RenderAnnotated {
            output,
            expected,
            range,
            overlays,
            ..
        } = job
        {
            if &original != expected {
                return Err(ClipError::SourceChanged);
            }
            crate::video_clip_contract::validate_overlay_plan(&original, range.as_ref(), overlays)?;
            let properties = clip
                .GetVideoEncodingProperties()
                .map_err(|_| ClipError::InvalidMedia)?;
            let par = properties
                .PixelAspectRatio()
                .map_err(|_| ClipError::InvalidMedia)?;
            let pixel_aspect = (
                par.Numerator().map_err(|_| ClipError::InvalidMedia)?,
                par.Denominator().map_err(|_| ClipError::InvalidMedia)?,
            );
            let bitrate = properties.Bitrate().map_err(|_| ClipError::InvalidMedia)?;
            let destination = Destination::new(output)?;
            let output_file = destination.create(cancel)?;
            // Only the file created above is ours to remove on failure. Source,
            // PNG and directory leases outlive the complete decoder/encoder.
            let rendered = (|| {
                crate::video_annotation_render::render_mp4(
                    &source.normal,
                    &destination.normal,
                    &original,
                    range.as_ref(),
                    overlays,
                    pixel_aspect,
                    bitrate,
                    cancel,
                    progress,
                )?;
                let actual_clip = wait(
                    MediaClip::CreateFromFileAsync(&output_file)
                        .map_err(|_| ClipError::OutputInvalid)?,
                    cancel,
                    ClipError::OutputInvalid,
                )?;
                let actual = metadata(&actual_clip).map_err(|_| ClipError::OutputInvalid)?;
                let retained = range
                    .clone()
                    .unwrap_or(crate::video_clip_contract::VideoRange {
                        start_ticks: 0,
                        end_ticks: original.duration_ticks,
                    });
                #[cfg(test)]
                eprintln!(
                    "synthetic annotated metadata: sourceRate={}/{}, outputRate={}/{}, outputDuration={}, retainedDuration={}, width={}, height={}, audio={}",
                    original.frame_rate_numerator,
                    original.frame_rate_denominator,
                    actual.frame_rate_numerator,
                    actual.frame_rate_denominator,
                    actual.duration_ticks,
                    retained.end_ticks - retained.start_ticks,
                    actual.width,
                    actual.height,
                    actual.has_audio,
                );
                crate::video_clip_contract::validate_annotated_rendered(
                    &original, &retained, &actual,
                )?;
                check_cancel(cancel)?;
                progress(100);
                Ok(ClipJobResult::Rendered(actual))
            })();
            drop(output_file);
            if rendered.is_err() {
                #[cfg(test)]
                preserve_synthetic_failure(&destination.normal);
                let _ = fs::remove_file(&destination.normal);
            }
            return rendered;
        }
        if let ClipJob::ExtractFrames {
            output_directory,
            expected,
            window,
            requested_ticks,
            ..
        } = job
        {
            if &original != expected {
                return Err(ClipError::SourceChanged);
            }
            // Source's immutable handle and the MTA apartment survive the
            // extraction. Each frame path is fixed by its native ordinal.
            return crate::video_annotation_render::extract_frames_to_files(
                &source.normal,
                output_directory,
                &original,
                window,
                requested_ticks,
                cancel,
            )
            .map(|frames| ClipJobResult::FramesExtracted { frames });
        }
        if let ClipJob::RenderGif {
            output,
            expected,
            range,
            ..
        } = job
        {
            if &original != expected {
                return Err(ClipError::SourceChanged);
            }
            original.validate_range(range.as_ref())?;
            let destination = Destination::with_extension(output, "gif")?;
            // The source and destination directory leases and COM apartment stay
            // alive across the streaming encoder. No renderer-controlled path.
            return crate::video_gif_backend::render(
                &source.normal,
                &destination.normal,
                &original,
                range.as_ref(),
                cancel,
                progress,
            )
            .map(ClipJobResult::GifRendered);
        }
        let ClipJob::Render {
            output,
            expected,
            range,
            ..
        } = job
        else {
            return Ok(ClipJobResult::Inspected(original));
        };
        if &original != expected {
            return Err(ClipError::SourceChanged);
        }
        original.validate_range(Some(range))?;
        let encoding = profile(&clip, &file, &original, cancel)?;
        clip.SetTrimTimeFromStart(TimeSpan {
            Duration: range.start_ticks as i64,
        })
        .map_err(|_| ClipError::InvalidRange)?;
        clip.SetTrimTimeFromEnd(TimeSpan {
            Duration: (original.duration_ticks - range.end_ticks) as i64,
        })
        .map_err(|_| ClipError::InvalidRange)?;
        let composition = MediaComposition::new().map_err(|_| ClipError::EncodingFailed)?;
        composition
            .Clips()
            .and_then(|v| v.Append(&clip))
            .map_err(|_| ClipError::EncodingFailed)?;
        check_cancel(cancel)?;
        let destination = Destination::new(output)?;
        let output_file = destination.create(cancel)?;
        let operation = composition
            .RenderToFileWithProfileAsync(&output_file, MediaTrimmingPreference::Precise, &encoding)
            .map_err(|_| ClipError::EncodingFailed)?;
        let percent = Arc::new(AtomicU8::new(0));
        let updated = percent.clone();
        operation
            .SetProgress(&AsyncOperationProgressHandler::new(
                move |_, value: windows::core::Ref<'_, f64>| {
                    let value = *value;
                    if value.is_finite() {
                        updated.fetch_max(value.clamp(0.0, 99.0) as u8, Ordering::Release);
                    }
                    Ok(())
                },
            ))
            .map_err(|_| ClipError::EncodingFailed)?;
        let info: IAsyncInfo = operation.cast().map_err(|_| ClipError::EncodingFailed)?;
        progress(0);
        let last = std::cell::Cell::new(0u8);
        let oversized = std::cell::Cell::new(false);
        let render = wait_info(&info, cancel, ClipError::EncodingFailed, &|| {
            if fs::metadata(&destination.normal).is_ok_and(|v| v.len() > MAX_SOURCE_BYTES) {
                oversized.set(true);
                let _ = operation.Cancel();
            }
            let value = percent.load(Ordering::Acquire);
            if value > last.get() {
                last.set(value);
                progress(value);
            }
        })
        .and_then(|_| {
            operation
                .GetResults()
                .map_err(|_| ClipError::EncodingFailed)
        })
        .and_then(|reason| {
            if reason == TranscodeFailureReason::None {
                Ok(())
            } else {
                Err(ClipError::EncodingFailed)
            }
        });
        let _ = info.Close();
        let _ = composition.Clips().and_then(|v| v.Clear());
        if oversized.get() {
            return Err(ClipError::OutputInvalid);
        }
        render?;
        check_cancel(cancel)?;
        // Drop decoder/encoder references before exclusive host publication.
        drop(operation);
        drop(composition);
        drop(clip);
        drop(encoding);
        let actual_clip = wait(
            MediaClip::CreateFromFileAsync(&output_file).map_err(|_| ClipError::OutputInvalid)?,
            cancel,
            ClipError::OutputInvalid,
        )?;
        let actual = metadata(&actual_clip).map_err(|_| ClipError::OutputInvalid)?;
        validate_rendered(&original, range, &actual)?;
        drop(actual_clip);
        drop(output_file);
        let size = fs::metadata(&destination.normal)
            .map_err(|_| ClipError::FileUnavailable)?
            .len();
        if !(12..=MAX_SOURCE_BYTES).contains(&size) {
            return Err(ClipError::OutputInvalid);
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&destination.normal)
            .and_then(|file| file.sync_all())
            .map_err(|_| ClipError::FileUnavailable)?;
        check_cancel(cancel)?;
        progress(100);
        Ok(ClipJobResult::Rendered(actual))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::video_clip_contract::{VideoRange, MIN_RANGE_TICKS};
        #[test]
        fn path_conversion_preserves_unicode_and_rejects_verbatim_aliases() {
            assert_eq!(
                ordinary_path(Path::new(r"\\?\D:\自有 数据\a.mp4")).unwrap(),
                PathBuf::from(r"D:\自有 数据\a.mp4")
            );
            assert_eq!(
                ordinary_path(Path::new(r"\\?\UNC\server\share\片段.mp4")).unwrap(),
                PathBuf::from(r"\\server\share\片段.mp4")
            );
            for name in [
                r"\\.\PhysicalDrive0",
                r"relative.mp4",
                r"\\?\D:\data\name.\a.mp4",
                r"\\?\D:\data\name \a.mp4",
                r"D:\data\..\a.mp4",
            ] {
                assert!(ordinary_path(Path::new(name)).is_err(), "{name}");
            }
        }
        #[test]
        fn normalized_source_is_same_file_and_held_read_only() {
            let root =
                std::env::temp_dir().join(format!("mewu-clip-path-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let path = root.join("空 格.mp4");
            fs::write(&path, b"\0\0\0\x0cftypisom").unwrap();
            let source = Source::open(&path).unwrap();
            assert!(same_file::is_same_file(&source.normal, &path).unwrap());
            assert!(OpenOptions::new().write(true).open(&path).is_err());
            assert!(fs::remove_file(&path).is_err());
            drop(source);
            fs::remove_file(&path).unwrap();
            fs::remove_dir(&root).unwrap();
        }

        // Opt-in platform media verification. Images and files are synthetic;
        // no capture, microphone, player window or user media is involved.
        #[test]
        #[ignore = "explicit synthetic Windows MediaComposition codec probe"]
        fn synthetic_minimum_range_short_source_and_unicode_paths() {
            let root = crate::video_gif_backend::TestDirectory::new();
            let directory = root.path.join("路径 空格");
            fs::create_dir_all(&directory).unwrap();
            let _apartment = Apartment::new().unwrap();
            let cancel = AtomicBool::new(false);
            let image = directory.join("原图.png");
            image::RgbaImage::from_fn(320, 180, |x, y| {
                image::Rgba([
                    if x < 160 { 230 } else { 20 },
                    if y < 90 { 190 } else { 30 },
                    70,
                    255,
                ])
            })
            .save(&image)
            .unwrap();
            fn make(image: &Path, output: &Path, ticks: i64, cancel: &AtomicBool) {
                let normal = ordinary_path(&image.canonicalize().unwrap()).unwrap();
                let file = wait(
                    StorageFile::GetFileFromPathAsync(&hstring(&normal)).unwrap(),
                    cancel,
                    ClipError::FileUnavailable,
                )
                .unwrap();
                let clip = wait(
                    MediaClip::CreateFromImageFileAsync(&file, TimeSpan { Duration: ticks })
                        .unwrap(),
                    cancel,
                    ClipError::EncodingFailed,
                )
                .unwrap();
                let composition = MediaComposition::new().unwrap();
                composition.Clips().unwrap().Append(&clip).unwrap();
                let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::Auto).unwrap();
                let video = profile.Video().unwrap();
                video.SetWidth(320).unwrap();
                video.SetHeight(180).unwrap();
                video.FrameRate().unwrap().SetNumerator(30).unwrap();
                video.FrameRate().unwrap().SetDenominator(1).unwrap();
                video.PixelAspectRatio().unwrap().SetNumerator(1).unwrap();
                video.PixelAspectRatio().unwrap().SetDenominator(1).unwrap();
                video.SetBitrate(500_000).unwrap();
                profile.SetAudio(None).unwrap();
                let destination = Destination::new(output).unwrap();
                let output = destination.create(cancel).unwrap();
                let op = composition
                    .RenderToFileWithProfileAsync(
                        &output,
                        MediaTrimmingPreference::Precise,
                        &profile,
                    )
                    .unwrap();
                let info: IAsyncInfo = op.cast().unwrap();
                wait_info(&info, cancel, ClipError::EncodingFailed, &|| {}).unwrap();
                assert_eq!(op.GetResults().unwrap(), TranscodeFailureReason::None);
                info.Close().unwrap();
                composition.Clips().unwrap().Clear().unwrap();
            }
            let source = directory.join("原视频.mp4");
            make(&image, &source, 20_000_000, &cancel);
            let ClipJobResult::Inspected(original) = super::execute(
                &ClipJob::Inspect {
                    source: source.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("inspect result");
            };
            let ClipJobResult::Inspected(verbatim) = super::execute(
                &ClipJob::Inspect {
                    source: source.canonicalize().unwrap(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("inspect result");
            };
            assert_eq!(original, verbatim);
            let output = directory.join("裁剪 100毫秒.mp4");
            let range = VideoRange {
                start_ticks: 1_000_000,
                end_ticks: 2_000_000,
            };
            let ClipJobResult::Rendered(trimmed) = super::execute(
                &ClipJob::Render {
                    source: source.clone(),
                    output: output.clone(),
                    expected: original.clone(),
                    range: range.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("render result");
            };
            validate_rendered(&original, &range, &trimmed).unwrap();
            assert!(trimmed.duration_ticks.abs_diff(MIN_RANGE_TICKS) <= 333_334);
            let short = directory.join("短视频.mp4");
            make(&image, &short, 500_000, &cancel);
            let ClipJobResult::Inspected(short_meta) = super::execute(
                &ClipJob::Inspect {
                    source: short.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("inspect result");
            };
            assert!(short_meta.duration_ticks > 0 && short_meta.duration_ticks < MIN_RANGE_TICKS);
            assert!(short_meta.validate_range(None).is_ok());
            let short_gif = directory.join("完整短片.gif");
            let ClipJobResult::GifRendered(short_gif_meta) = super::execute(
                &ClipJob::RenderGif {
                    source: short.clone(),
                    output: short_gif.clone(),
                    expected: short_meta.clone(),
                    range: None,
                },
                &cancel,
                &|_| {},
            )
            .unwrap() else {
                panic!("short GIF result")
            };
            assert_eq!(short_gif_meta.frame_count, 1);
            crate::video_gif_backend::validate_file(&short_gif, &short_gif_meta, &cancel).unwrap();
            assert!(short_meta
                .validate_range(Some(&VideoRange {
                    start_ticks: 0,
                    end_ticks: short_meta.duration_ticks
                }))
                .is_err());
            let copy = directory.join("完整短片.mp4");
            fs::copy(&short, &copy).unwrap();
            assert_eq!(fs::read(&short).unwrap(), fs::read(&copy).unwrap());
            // Existing output is rejected, never silently replaced/truncated.
            let before = fs::read(&output).unwrap();
            assert!(super::execute(
                &ClipJob::Render {
                    source,
                    output: output.clone(),
                    expected: original,
                    range
                },
                &cancel,
                &|_| {}
            )
            .is_err());
            assert_eq!(fs::read(output).unwrap(), before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_cancelled_work_does_not_open_any_path_or_initialize_media() {
        assert_eq!(
            execute(
                &ClipJob::Inspect {
                    source: "nonexistent.mp4".into()
                },
                &AtomicBool::new(true),
                &|_| panic!("no progress")
            ),
            Err(ClipError::Cancelled)
        );
    }
}
