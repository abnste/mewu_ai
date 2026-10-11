// SPDX-License-Identifier: MPL-2.0
//! Streaming GIF with real decoded source frames and shared annotation time.
use super::{check_cancel, frames, inspect, Source};
use crate::{
    video_annotation_render::{RasterPlan, SourcePlaybackTicks},
    video_clip_contract::{
        AnnotatedGifSchedule, ClipError, GifMetadata, OverlayRaster, VideoMetadata, VideoRange,
        MAX_GIF_BYTES,
    },
    video_clip_output_macos::Destination,
};
use std::{
    io::{self, BufWriter, Write},
    path::Path,
    sync::atomic::AtomicBool,
    time::Duration,
};

struct Limited<'a, W> {
    inner: W,
    written: u64,
    cancel: &'a AtomicBool,
}
impl<W: Write> Write for Limited<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_cancel(self.cancel).map_err(|_| io::Error::other("cancelled"))?;
        if bytes.len() as u64 > MAX_GIF_BYTES.saturating_sub(self.written) {
            return Err(io::Error::other("GIF budget"));
        }
        let n = self.inner.write(bytes)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
pub(super) fn render(
    source: &mut Source,
    output: &Path,
    expected: &VideoMetadata,
    range: Option<&VideoRange>,
    overlays: &[OverlayRaster],
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<GifMetadata, ClipError> {
    check_cancel(cancel)?;
    expected.validate_range(range)?;
    if inspect(source, cancel)? != *expected {
        return Err(ClipError::SourceChanged);
    }
    let start = range.map_or(0, |v| v.start_ticks);
    let end = range.map_or(expected.duration_ticks, |v| v.end_ticks);
    let (metadata, requested, delays, plan) = if overlays.is_empty() {
        let m = GifMetadata::for_source(expected, range)?;
        let ticks = (0..m.frame_count)
            .map(|index| {
                start
                    + ((u128::from(index) * u128::from(end - start)
                        + u128::from(m.frame_count / 2))
                        / u128::from(m.frame_count))
                    .min(u128::from(end - start - 1)) as u64
            })
            .collect::<Vec<_>>();
        let delays = (0..m.frame_count)
            .map(|i| crate::video_gif_backend::frame_delay(i, &m))
            .collect::<Vec<_>>();
        (m, ticks, delays, None)
    } else {
        let schedule = AnnotatedGifSchedule::new(expected, range, overlays)?;
        let plan = RasterPlan::load(expected, range, overlays, cancel)?;
        (
            schedule.metadata,
            schedule.requested_ticks,
            schedule.delays,
            Some(plan),
        )
    };
    let mut destination = Destination::gif(output)?;
    let file = destination.create_file()?;
    let limited = Limited {
        inner: BufWriter::new(file.try_clone().map_err(|_| ClipError::FileUnavailable)?),
        written: 0,
        cancel,
    };
    let convert = |_| {
        if check_cancel(cancel).is_err() {
            ClipError::Cancelled
        } else {
            ClipError::EncodingFailed
        }
    };
    let mut encoder =
        gif::Encoder::new(limited, metadata.width as u16, metadata.height as u16, &[])
            .map_err(convert)?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(convert)?;
    progress(0);
    let mut previous = 0;
    frames::for_each_decoded(
        source,
        expected,
        &requested,
        Duration::from_secs(600),
        cancel,
        |index, sample, _, _, matrix| {
            destination.check()?;
            let rgb = frames::decoded_rgb(
                sample,
                matrix,
                if plan.is_some() {
                    (expected.width, expected.height)
                } else {
                    (metadata.width, metadata.height)
                },
                expected,
                cancel,
            )?;
            let mut rgba = image::DynamicImage::ImageRgb8(rgb).into_rgba8();
            if let Some(plan) = &plan {
                plan.paint(&mut rgba, SourcePlaybackTicks(requested[index]), cancel)?;
            }
            let mut rgba = if rgba.dimensions() == (metadata.width, metadata.height) {
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
            check_cancel(cancel)?;
            let mut frame = gif::Frame::from_rgba_speed(
                metadata.width as u16,
                metadata.height as u16,
                &mut rgba,
                10,
            );
            frame.delay = delays[index];
            frame.dispose = gif::DisposalMethod::Background;
            encoder.write_frame(&frame).map_err(convert)?;
            check_cancel(cancel)?;
            let percent = ((index + 1) * 99 / requested.len()) as u8;
            if percent > previous {
                previous = percent;
                progress(percent);
            }
            Ok(())
        },
    )?;
    let mut writer = encoder.into_inner().map_err(convert)?;
    writer.flush().map_err(|_| ClipError::FileUnavailable)?;
    writer
        .inner
        .get_ref()
        .sync_all()
        .map_err(|_| ClipError::FileUnavailable)?;
    drop(writer);
    check_cancel(cancel)?;
    source.check(cancel)?;
    destination.check()?;
    if file
        .metadata()
        .map_err(|_| ClipError::FileUnavailable)?
        .len()
        > MAX_GIF_BYTES
    {
        return Err(ClipError::OutputInvalid);
    }
    if plan.is_some() {
        crate::video_gif_backend::validate_annotated_file(
            destination.path(),
            &metadata,
            &delays,
            cancel,
        )?;
    } else {
        crate::video_gif_backend::validate_file(destination.path(), &metadata, cancel)?;
    }
    check_cancel(cancel)?;
    source.check(cancel)?;
    destination.check()?;
    destination.publish(&file)?;
    check_cancel(cancel)?;
    source.check(cancel)?;
    destination.check()?;
    destination.commit();
    progress(100);
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, sync::atomic::Ordering};
    #[test]
    #[ignore = "requires the explicitly supplied owned H264/AAC Mac fixture"]
    fn native_gif_frames_delays_annotation_boundaries_and_cancel_are_real() {
        let path =
            PathBuf::from(std::env::var_os("MEWU_MAC_MEDIA_TEST_SOURCE").expect("owned fixture"));
        let root = std::env::temp_dir().join(format!("mewu-mac-gif-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let cancel = AtomicBool::new(false);
        let mut source = Source::open(&path, &cancel).unwrap();
        let expected = inspect(&mut source, &cancel).unwrap();
        let hash = source.hash;
        let range = VideoRange {
            start_ticks: 10_882_315,
            end_ticks: expected.duration_ticks,
        };
        let output = root.join("actual.gif");
        for window in [None, Some(&range)] {
            let progress_events = std::cell::RefCell::new(Vec::new());
            let metadata = render(
                &mut source,
                &output,
                &expected,
                window,
                &[],
                &cancel,
                &|v| progress_events.borrow_mut().push(v),
            )
            .unwrap();
            let retained = std::fs::read(&output).unwrap();
            crate::video_gif_backend::validate_file(&output, &metadata, &cancel).unwrap();
            metadata.validate_for(&expected, window).unwrap();
            assert_eq!(progress_events.borrow().last(), Some(&100));
            assert!(progress_events.borrow().windows(2).all(|v| v[0] < v[1]));
            assert_eq!(
                render(
                    &mut source,
                    &output,
                    &expected,
                    window,
                    &[],
                    &cancel,
                    &|_| {}
                ),
                Err(ClipError::FileUnavailable)
            );
            assert_eq!(std::fs::read(&output).unwrap(), retained);
            println!(
                "{}",
                serde_json::json!({"clean_gif":metadata,"range":window,"bytes":retained.len(),"sha256":format!("{:x}",Sha256::digest(&retained)),"existing_output_preserved":true})
            );
            std::fs::remove_file(&output).unwrap();
        }
        let png = root.join("overlay.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 255, 255]))
            .save(&png)
            .unwrap();
        let entry = OverlayRaster {
            png: png.clone(),
            png_sha256: format!("{:x}", Sha256::digest(std::fs::read(&png).unwrap())),
            width: 2,
            height: 2,
            bounds: crate::video_clip_contract::OverlayBounds {
                x: 0.,
                y: 0.,
                width: f64::from(expected.width) / 2.,
                height: f64::from(expected.height) / 2.,
            },
            interval: VideoRange {
                start_ticks: 12_000_000,
                end_ticks: 22_000_000,
            },
        };
        let schedule =
            AnnotatedGifSchedule::new(&expected, Some(&range), std::slice::from_ref(&entry))
                .unwrap();
        let annotated = render(
            &mut source,
            &output,
            &expected,
            Some(&range),
            std::slice::from_ref(&entry),
            &cancel,
            &|_| {},
        )
        .unwrap();
        crate::video_gif_backend::validate_annotated_file(
            &output,
            &annotated,
            &schedule.delays,
            &cancel,
        )
        .unwrap();
        let mut decoder = gif::DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::RGBA);
        let mut reader = decoder
            .read_info(std::fs::File::open(&output).unwrap())
            .unwrap();
        let mut index = 0;
        let mut painted = 0;
        while let Some(frame) = reader.read_next_frame().unwrap() {
            let at = ((annotated.height / 4 * annotated.width + annotated.width / 4) * 4) as usize;
            let pixel = &frame.buffer[at..at + 3];
            let wanted = schedule.requested_ticks[index] >= 12_000_000
                && schedule.requested_ticks[index] < 22_000_000;
            let magenta = pixel
                .iter()
                .zip([255u8, 0, 255])
                .all(|(a, b)| a.abs_diff(b) <= 15);
            assert_eq!(
                magenta, wanted,
                "original source annotation time {}",
                schedule.requested_ticks[index]
            );
            painted += usize::from(wanted);
            index += 1;
        }
        assert_eq!(index, schedule.requested_ticks.len());
        assert!(painted > 0);
        println!(
            "{}",
            serde_json::json!({"annotated_gif":annotated,"actual_frames_checked":index,"painted_frames":painted,"half_open_start_end_pixels_passed":true})
        );
        drop(reader);
        std::fs::remove_file(&output).unwrap();
        std::fs::remove_file(png).unwrap();
        let observed = AtomicBool::new(false);
        assert_eq!(
            render(
                &mut source,
                &output,
                &expected,
                Some(&range),
                &[],
                &cancel,
                &|v| {
                    if v > 0 && v < 100 {
                        observed.store(true, Ordering::Release);
                        cancel.store(true, Ordering::Release);
                    }
                }
            ),
            Err(ClipError::Cancelled)
        );
        assert!(observed.load(Ordering::Acquire));
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        cancel.store(false, Ordering::Release);
        source.check(&cancel).unwrap();
        assert_eq!(source.hash, hash);
        println!(
            "{}",
            serde_json::json!({"cancel_after_encoded_frame":true,"output_and_staging_absent":true,"source_unchanged":true})
        );
        std::fs::remove_dir(root).unwrap();
    }
}
