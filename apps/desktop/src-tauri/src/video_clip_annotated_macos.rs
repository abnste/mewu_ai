// SPDX-License-Identifier: MPL-2.0
//! Local AVFoundation composition, with verified rasters on an independent
//! Core Animation tree. No AppKit views, UI state, external encoder or new IPC.
use super::{asset_for, at_ticks, check_cancel, inspect, loaded, ExportFlight, Source};
use crate::{
    video_annotation_render::{project_interval, RasterPlan},
    video_clip_contract::{validate_rendered, ClipError, OverlayRaster, VideoMetadata, VideoRange},
    video_clip_output_macos::Destination,
};
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_av_foundation::{
    AVAssetExportPresetHighestQuality, AVAssetExportSession, AVCoreAnimationBeginTimeAtZero,
    AVFileTypeMPEG4, AVMediaTypeVideo, AVMutableComposition, AVMutableVideoComposition,
    AVVideoCompositionCoreAnimationTool,
};
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_core_media::{kCMPersistentTrackID_Invalid, CMTime, CMTimeFlags, CMTimeRange};
use objc2_foundation::{NSArray, NSNumber, NSString, NSURL};
use objc2_quartz_core::{
    kCAAnimationDiscrete, kCAFillModeBoth, kCAGravityResize, CAKeyframeAnimation, CALayer,
    CAMediaTiming,
};
use std::{path::Path, sync::atomic::AtomicBool};

/// Discrete Core Animation has one value per interval, hence n-1 values for n
/// key times. These are source-clock intersections projected once onto the
/// composition's zero origin, never decoded-sample timestamps or wall time.
fn opacity_schedule(
    interval: &VideoRange,
    retained: &VideoRange,
    source: &VideoMetadata,
) -> Result<Option<(Vec<f64>, Vec<f64>)>, ClipError> {
    let Some(projected) = project_interval(interval, retained)? else {
        return Ok(None);
    };
    let duration = retained.end_ticks - retained.start_ticks;
    let start_ticks = projected.delay_ticks;
    let end_ticks = start_ticks
        .checked_add(projected.duration_ticks)
        .ok_or(ClipError::InvalidRange)?;
    let numerator = u128::from(source.frame_rate_numerator);
    let step = u128::from(source.frame_rate_denominator) * 10_000_000;
    if numerator == 0 || step == 0 {
        return Err(ClipError::InvalidMedia);
    }
    // The video composition below explicitly emits frame n at n*den/num.
    // Shared annotation semantics evaluate source.start + floor(n*step/num).
    // Solve that integer inequality, rather than rounding source intervals.
    let first_frame = |ticks: u64| (u128::from(ticks) * numerator).div_ceil(step);
    let count = first_frame(duration);
    let start = first_frame(start_ticks);
    let end = first_frame(end_ticks).min(count);
    if start >= end {
        return Ok(None); // The source interval contains no emitted sample.
    }
    let mut boundaries = vec![0, start, end, count];
    boundaries.sort_unstable();
    boundaries.dedup();
    let values = boundaries[..boundaries.len() - 1]
        .iter()
        .map(|&t| if start <= t && t < end { 1.0 } else { 0.0 })
        .collect();
    // CA reconstructs a continuous time from keyTime*duration. On long clips
    // an exact boundary (e.g. 3s) can round just AFTER its sample, retaining one
    // forbidden frame. Put each step at the rational midpoint between the
    // preceding sample and the first affected sample. This is not an epsilon
    // or extended interval: every emitted sample keeps its exact half-open
    // source-clock decision, including fractional interval/trim boundaries.
    let times = boundaries
        .into_iter()
        .map(|index| {
            if index == 0 {
                0.0
            } else if index == count {
                1.0
            } else {
                ((2 * index - 1) * step) as f64 / (2 * numerator * u128::from(duration)) as f64
            }
        })
        .collect();
    Ok(Some((times, values)))
}

struct Layers {
    root: Retained<CALayer>,
    video: Retained<CALayer>,
    // Keep the exact CGImage/CFData backing alive through the actual export
    // completion (and cancellation drain), independent of layer copy semantics.
    _images: Vec<CFRetained<CGImage>>,
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width, height },
    }
}
fn layers(
    plan: &RasterPlan,
    source: &VideoMetadata,
    retained: &VideoRange,
    cancel: &AtomicBool,
) -> Result<Layers, ClipError> {
    let root = CALayer::layer();
    let video = CALayer::layer();
    let frame = rect(0.0, 0.0, f64::from(source.width), f64::from(source.height));
    root.setFrame(frame);
    video.setFrame(frame);
    root.addSublayer(&video);
    let duration = retained.end_ticks - retained.start_ticks;
    let mut images = Vec::new();
    plan.visit_rasters(|entry, pixels| {
        check_cancel(cancel)?;
        let Some((times, opacities)) = opacity_schedule(&entry.interval, retained, source)? else {
            return Ok(());
        };
        let bytes = pixels.as_raw();
        let data = unsafe {
            CFData::new(
                None,
                bytes.as_ptr(),
                bytes
                    .len()
                    .try_into()
                    .map_err(|_| ClipError::InvalidMedia)?,
            )
        }
        .ok_or(ClipError::EncodingFailed)?;
        let provider =
            CGDataProvider::with_cf_data(Some(&data)).ok_or(ClipError::EncodingFailed)?;
        let space = CGColorSpace::new_device_rgb().ok_or(ClipError::EncodingFailed)?;
        // image::RgbaImage holds straight RGBA, not premultiplied BGRA.
        let image = unsafe {
            CGImage::new(
                entry.width as usize,
                entry.height as usize,
                8,
                32,
                entry.width as usize * 4,
                Some(&space),
                CGBitmapInfo(CGImageAlphaInfo::Last.0),
                Some(&provider),
                std::ptr::null(),
                true,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .ok_or(ClipError::EncodingFailed)?;
        let layer = CALayer::layer();
        let b = entry.bounds;
        // Stored annotation bounds use the same top-left display coordinates
        // as screenshot drawing; export Core Animation has bottom-left origin.
        layer.setFrame(rect(
            b.x,
            f64::from(source.height) - b.y - b.height,
            b.width,
            b.height,
        ));
        layer.setContentsScale(1.0);
        layer.setOpacity(0.0);
        unsafe {
            layer.setContentsGravity(kCAGravityResize);
            // Apple's CALayer.contents explicitly accepts the toll-free CGImage.
            layer.setContents(Some(&*((&*image as *const CGImage).cast::<AnyObject>())));
        }
        let animation =
            CAKeyframeAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
        let numbers = opacities
            .into_iter()
            .map(NSNumber::new_f64)
            .collect::<Vec<_>>();
        let values = NSArray::from_slice(
            &numbers
                .iter()
                .map(|n| &**n as &AnyObject)
                .collect::<Vec<_>>(),
        );
        let times = NSArray::from_retained_slice(
            &times.into_iter().map(NSNumber::new_f64).collect::<Vec<_>>(),
        );
        unsafe {
            animation.setValues(Some(&values));
            animation.setCalculationMode(kCAAnimationDiscrete);
            animation.setBeginTime(AVCoreAnimationBeginTimeAtZero);
            animation.setFillMode(kCAFillModeBoth);
        }
        animation.setKeyTimes(Some(&times));
        animation.setDuration(duration as f64 / 10_000_000.0);
        animation.setRemovedOnCompletion(false);
        layer.addAnimation_forKey(
            &animation,
            Some(&NSString::from_str("mewu-source-interval")),
        );
        root.addSublayer(&layer);
        images.push(image);
        Ok(())
    })?;
    Ok(Layers {
        root,
        video,
        _images: images,
    })
}

#[allow(deprecated)]
pub(super) fn render(
    source: &mut Source,
    output: &Path,
    expected: &VideoMetadata,
    range: Option<&VideoRange>,
    overlays: &[OverlayRaster],
    cancel: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<VideoMetadata, ClipError> {
    check_cancel(cancel)?;
    let original = inspect(source, cancel)?;
    if original != *expected {
        return Err(ClipError::SourceChanged);
    }
    original.validate_range(range)?;
    let retained = range.cloned().unwrap_or(VideoRange {
        start_ticks: 0,
        end_ticks: original.duration_ticks,
    });
    let plan = RasterPlan::load(&original, Some(&retained), overlays, cancel)?;
    let mut destination = Destination::new(output)?;
    let asset = asset_for(source)?;
    loaded(&asset, cancel)?;
    source.check(cancel)?;
    let duration = at_ticks(retained.end_ticks - retained.start_ticks)?;
    let zero = at_ticks(0)?;
    let composition = unsafe { AVMutableComposition::composition() };
    unsafe {
        // Insert ALL source tracks together, including the original sound. This
        // trims their common clock to zero without synthesizing/remapping audio.
        composition
            .insertTimeRange_ofAsset_atTime_error(
                CMTimeRange {
                    start: at_ticks(retained.start_ticks)?,
                    duration,
                },
                &asset,
                zero,
            )
            .map_err(|_| ClipError::EncodingFailed)?;
        let media_type = AVMediaTypeVideo.ok_or(ClipError::WorkerUnavailable)?;
        let source_tracks = asset.tracksWithMediaType(media_type);
        let tracks = composition.tracksWithMediaType(media_type);
        if source_tracks.count() != 1 || tracks.count() != 1 {
            return Err(ClipError::InvalidMedia);
        }
        let source_track = source_tracks.firstObject().ok_or(ClipError::InvalidMedia)?;
        let track = tracks.firstObject().ok_or(ClipError::InvalidMedia)?;
        // Apple explicitly does not copy track properties when inserting an
        // asset. Preserve its validated display rotation before composition.
        let _: () = msg_send![&*track, setPreferredTransform: source_track.preferredTransform()];
    }
    let tree = layers(&plan, &original, &retained, cancel)?;
    let session = unsafe {
        AVAssetExportSession::exportSessionWithAsset_presetName(
            &composition,
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
        let video = AVMutableVideoComposition::videoCompositionWithPropertiesOfAsset(&composition);
        video.setFrameDuration(CMTime {
            value: i64::from(original.frame_rate_denominator),
            timescale: original
                .frame_rate_numerator
                .try_into()
                .map_err(|_| ClipError::InvalidMedia)?,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
        video.setSourceTrackIDForFrameTiming(kCMPersistentTrackID_Invalid);
        video.setRenderSize(CGSize {
            width: f64::from(original.width),
            height: f64::from(original.height),
        });
        let tool = AVVideoCompositionCoreAnimationTool::videoCompositionCoreAnimationToolWithPostProcessingAsVideoLayer_inLayer(&tree.video, &tree.root);
        video.setAnimationTool(Some(&tool));
        session.setVideoComposition(Some(&video));
        session.setOutputURL(Some(&url));
        session.setOutputFileType(Some(AVFileTypeMPEG4.ok_or(ClipError::WorkerUnavailable)?));
        session.setTimeRange(CMTimeRange {
            start: zero,
            duration,
        });
    }
    let mut flight = ExportFlight::start(session);
    // This retains the existing supervised 600s deadline, byte limit, source
    // leases, cancellation and real completion drain. No fixed sleep/GUI work.
    flight.wait(&destination, cancel, progress)?;
    drop(flight);
    drop(tree);
    drop(composition);
    drop(asset);
    check_cancel(cancel)?;
    source.check(cancel)?;
    let output_error = |e| {
        if e == ClipError::Cancelled {
            e
        } else {
            ClipError::OutputInvalid
        }
    };
    let mut exported = Source::open(destination.path(), cancel).map_err(output_error)?;
    let actual = inspect(&mut exported, cancel).map_err(output_error)?;
    validate_rendered(&original, &retained, &actual)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
    #[test]
    fn opacity_uses_source_clock_half_open_intervals_and_discrete_segments() {
        for (num, den) in [(15, 1), (30, 1), (2997, 100), (60000, 1001)] {
            let source = VideoMetadata {
                duration_ticks: 1018666458,
                width: 760,
                height: 440,
                frame_rate_numerator: num,
                frame_rate_denominator: den,
                has_audio: true,
            };
            for retained in [
                VideoRange {
                    start_ticks: 0,
                    end_ticks: 1018666458,
                },
                VideoRange {
                    start_ticks: 10_123_457,
                    end_ticks: 50_888_519,
                },
            ] {
                for (start, end) in [
                    (0, 1),
                    (0, 10_000_000),
                    (0, source.duration_ticks),
                    (20_000_000, 30_000_000),
                    (20_123_457, 30_567_891),
                    (retained.end_ticks, source.duration_ticks + 1),
                    (1, 2),
                ] {
                    let interval = VideoRange {
                        start_ticks: start,
                        end_ticks: end,
                    };
                    let schedule = opacity_schedule(&interval, &retained, &source).unwrap();
                    if let Some((times, values)) = &schedule {
                        assert_eq!(times.len(), values.len() + 1);
                        assert_eq!(times.first(), Some(&0.0));
                        assert_eq!(times.last(), Some(&1.0));
                        assert!(times.windows(2).all(|p| p[0] < p[1]));
                    }
                    let duration = retained.end_ticks - retained.start_ticks;
                    let step = u128::from(den) * 10_000_000;
                    let count = (u128::from(duration) * u128::from(num)).div_ceil(step);
                    for n in 0..count {
                        // Independent exact integer source-frame oracle; do not
                        // derive expected visibility from the CA schedule.
                        let source_ticks =
                            retained.start_ticks + (n * step / u128::from(num)) as u64;
                        let expected = start <= source_ticks && source_ticks < end;
                        let actual = schedule.as_ref().is_some_and(|(times, values)| {
                            let normalized =
                                (n * step) as f64 / (u128::from(num) * u128::from(duration)) as f64;
                            let index =
                                times[..values.len()].partition_point(|t| *t <= normalized) - 1;
                            values[index] == 1.0
                        });
                        assert_eq!(
                            actual, expected,
                            "rate={num}/{den}, frame={n}, interval={interval:?}, trim={retained:?}"
                        );
                    }
                }
            }
        }
        let source = VideoMetadata {
            duration_ticks: 100000000,
            width: 760,
            height: 440,
            frame_rate_numerator: 15,
            frame_rate_denominator: 1,
            has_audio: true,
        };
        assert_eq!(
            opacity_schedule(
                &VideoRange {
                    start_ticks: 50,
                    end_ticks: 50
                },
                &VideoRange {
                    start_ticks: 0,
                    end_ticks: 100000000
                },
                &source
            ),
            Err(ClipError::InvalidRange)
        );
    }

    #[test]
    #[ignore = "requires the explicitly supplied, owned H264/AAC Mac media fixture"]
    fn native_annotated_mp4_renders_pixels_source_intervals_sound_and_cancellation() {
        objc2::rc::autoreleasepool(|_| {
            let path = PathBuf::from(
                std::env::var_os("MEWU_MAC_MEDIA_TEST_SOURCE").expect("owned source"),
            );
            let directory =
                std::env::temp_dir().join(format!("mewu-mac-annotated-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let directory = directory.canonicalize().unwrap();
            let cancel = AtomicBool::new(false);
            let mut source = Source::open(&path, &cancel).unwrap();
            let expected = inspect(&mut source, &cancel).unwrap();
            let hash = source.hash;
            assert!(expected.has_audio && expected.width >= 320 && expected.height >= 200);
            let range = VideoRange {
                start_ticks: 10_000_000,
                end_ticks: 50_000_000,
            };
            let png = directory.join("verified-overlay.png");
            let mut pixels = image::RgbaImage::from_pixel(64, 64, image::Rgba([250, 0, 20, 255]));
            for y in 32..64 {
                for x in 0..64 {
                    pixels.put_pixel(x, y, image::Rgba([0, 240, 250, 255]));
                }
            }
            pixels.save(&png).unwrap();
            let sha = format!("{:x}", Sha256::digest(std::fs::read(&png).unwrap()));
            let full = OverlayRaster {
                png: png.clone(),
                png_sha256: sha,
                width: 64,
                height: 64,
                bounds: crate::video_clip_contract::OverlayBounds {
                    x: 20.,
                    y: 20.,
                    width: 64.,
                    height: 64.,
                },
                interval: VideoRange {
                    start_ticks: 0,
                    end_ticks: expected.duration_ticks,
                },
            };
            let gated_png = directory.join("verified-gated-overlay.png");
            image::RgbaImage::from_pixel(32, 32, image::Rgba([250, 20, 230, 255]))
                .save(&gated_png)
                .unwrap();
            let gated = OverlayRaster {
                png: gated_png.clone(),
                png_sha256: format!("{:x}", Sha256::digest(std::fs::read(&gated_png).unwrap())),
                width: 32,
                height: 32,
                bounds: crate::video_clip_contract::OverlayBounds {
                    x: 120.,
                    y: 20.,
                    width: 32.,
                    height: 32.,
                },
                interval: VideoRange {
                    start_ticks: 20_000_000,
                    end_ticks: 30_000_000,
                },
            };
            let alpha_png = directory.join("verified-alpha-overlay.png");
            image::RgbaImage::from_pixel(32, 32, image::Rgba([250, 250, 250, 128]))
                .save(&alpha_png)
                .unwrap();
            let alpha = OverlayRaster {
                png: alpha_png.clone(),
                png_sha256: format!("{:x}", Sha256::digest(std::fs::read(&alpha_png).unwrap())),
                width: 32,
                height: 32,
                bounds: crate::video_clip_contract::OverlayBounds {
                    x: 220.,
                    y: 20.,
                    width: 32.,
                    height: 32.,
                },
                interval: VideoRange {
                    start_ticks: 0,
                    end_ticks: expected.duration_ticks,
                },
            };
            let overlays = vec![full, gated, alpha];
            let output = directory.join("nonzero-trim-annotated.mp4");
            let percent = std::cell::RefCell::new(Vec::new());
            let actual = render(
                &mut source,
                &output,
                &expected,
                Some(&range),
                &overlays,
                &cancel,
                &|v| percent.borrow_mut().push(v),
            )
            .unwrap();
            validate_rendered(&expected, &range, &actual).unwrap();
            assert!(actual.has_audio);
            assert_eq!(percent.borrow().last(), Some(&100));
            assert!(percent.borrow().windows(2).all(|p| p[0] < p[1]));
            let exported = Source::open(&output, &cancel).unwrap();
            let ticks = [0, 12_000_000, 20_000_000, 32_000_000];
            let mut underlay = Vec::new();
            super::super::frames::for_each_decoded(
                &source,
                &expected,
                &ticks
                    .iter()
                    .map(|t| t + range.start_ticks)
                    .collect::<Vec<_>>(),
                Duration::from_secs(20),
                &cancel,
                |_, sample, _, _, matrix| {
                    let rgb = super::super::frames::decoded_rgb(
                        sample,
                        matrix,
                        (expected.width, expected.height),
                        &expected,
                        &cancel,
                    )?;
                    underlay.push(rgb.get_pixel(236, 36).0);
                    Ok(())
                },
            )
            .unwrap();
            super::super::frames::for_each_decoded(
                &exported,
                &actual,
                &ticks,
                Duration::from_secs(20),
                &cancel,
                |index, sample, _, _, matrix| {
                    let rgb = super::super::frames::decoded_rgb(
                        sample,
                        matrix,
                        (actual.width, actual.height),
                        &actual,
                        &cancel,
                    )?;
                    let red = rgb.get_pixel(36, 36).0;
                    let cyan = rgb.get_pixel(36, 68).0;
                    assert!(
                        red[0] > 200 && red[1] < 60 && red[2] < 80,
                        "top-left red at {index}: {red:?}"
                    );
                    assert!(
                        cyan[0] < 60 && cyan[1] > 190 && cyan[2] > 190,
                        "bottom-left cyan at {index}: {cyan:?}"
                    );
                    let magenta = rgb.get_pixel(136, 36).0;
                    let visible = magenta[0] > 200 && magenta[1] < 60 && magenta[2] > 180;
                    assert_eq!(
                        visible,
                        index == 1,
                        "source interval at {index}: {magenta:?}"
                    );
                    let blended = rgb.get_pixel(236, 36).0;
                    for channel in 0..3 {
                        let expected = ((u16::from(underlay[index][channel]) * 127 + 250 * 128 + 127) / 255) as u8;
                        assert!(blended[channel].abs_diff(expected) <= 16, "straight alpha at {index}/{channel}: base={:?}, actual={blended:?}, expected={expected}", underlay[index]);
                    }
                    Ok(())
                },
            )
            .unwrap();
            let retained_bytes = std::fs::read(&output).unwrap();
            assert_eq!(
                render(
                    &mut source,
                    &output,
                    &expected,
                    Some(&range),
                    &overlays,
                    &cancel,
                    &|_| {}
                ),
                Err(ClipError::FileUnavailable)
            );
            assert_eq!(std::fs::read(&output).unwrap(), retained_bytes);
            let cancelled_output = directory.join("cancelled.mp4");
            let started = AtomicBool::new(false);
            assert_eq!(
                render(
                    &mut source,
                    &cancelled_output,
                    &expected,
                    Some(&range),
                    &overlays,
                    &cancel,
                    &|v| {
                        if v == 0 {
                            started.store(true, Ordering::Release);
                            cancel.store(true, Ordering::Release);
                        }
                    }
                ),
                Err(ClipError::Cancelled)
            );
            assert!(started.load(Ordering::Acquire));
            assert!(!cancelled_output.exists());
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 4);
            cancel.store(false, Ordering::Release);
            assert_eq!(Source::open(&path, &cancel).unwrap().hash, hash);
            let full_output = directory.join("full-source-annotated.mp4");
            let full_actual = render(
                &mut source,
                &full_output,
                &expected,
                None,
                &overlays,
                &cancel,
                &|_| {},
            )
            .unwrap();
            validate_rendered(
                &expected,
                &VideoRange {
                    start_ticks: 0,
                    end_ticks: expected.duration_ticks,
                },
                &full_actual,
            )
            .unwrap();
            let full_exported = Source::open(&full_output, &cancel).unwrap();
            super::super::frames::for_each_decoded(
                &full_exported,
                &full_actual,
                &[0, 22_000_000, 30_000_000, 60_000_000, 1_000_000_000],
                Duration::from_secs(30),
                &cancel,
                |index, sample, _, _, matrix| {
                    let rgb = super::super::frames::decoded_rgb(
                        sample,
                        matrix,
                        (full_actual.width, full_actual.height),
                        &full_actual,
                        &cancel,
                    )?;
                    let red = rgb.get_pixel(36, 36).0;
                    assert!(red[0] > 200 && red[1] < 60 && red[2] < 80);
                    let magenta = rgb.get_pixel(136, 36).0;
                    assert_eq!(
                        magenta[0] > 200 && magenta[1] < 60 && magenta[2] > 180,
                        index == 1
                    );
                    Ok(())
                },
            )
            .unwrap();
            println!(
                "{}",
                serde_json::json!({"fullRangeAnnotated":true,"output":full_output,"actual":full_actual,"sha256":format!("{:x}",Sha256::digest(std::fs::read(&full_output).unwrap()))})
            );
            println!(
                "{}",
                serde_json::json!({"nativeAnnotated":true,"output":output,"source":expected,"retained":range,"actual":actual,"outputSha256":format!("{:x}",Sha256::digest(&retained_bytes)),"pixelsAndIntervals":true,"existingOutputPreserved":true,"cancelledAfterExportStarted":true,"sourceUnchanged":true})
            );
        });
    }

    #[test]
    #[ignore = "requires the explicitly supplied, owned H264/AAC Mac media fixture"]
    #[allow(deprecated)]
    fn native_annotated_mp4_preserves_90_degree_display_geometry_and_audio() {
        objc2::rc::autoreleasepool(|_| {
            let path = PathBuf::from(
                std::env::var_os("MEWU_MAC_MEDIA_TEST_SOURCE").expect("owned source"),
            );
            let directory =
                std::env::temp_dir().join(format!("mewu-mac-rotated-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let directory = directory.canonicalize().unwrap();
            let cancel = AtomicBool::new(false);
            let mut source = Source::open(&path, &cancel).unwrap();
            let original = inspect(&mut source, &cancel).unwrap();
            let hash = source.hash;
            let retained = VideoRange {
                start_ticks: 0,
                end_ticks: 40_000_000,
            };
            original.validate_range(Some(&retained)).unwrap();
            let rotated_path = directory.join("owned-90-degree-source.mp4");
            let mut destination = Destination::new(&rotated_path).unwrap();
            let asset = asset_for(&source).unwrap();
            loaded(&asset, &cancel).unwrap();
            let composition = unsafe { AVMutableComposition::composition() };
            unsafe {
                composition
                    .insertTimeRange_ofAsset_atTime_error(
                        CMTimeRange {
                            start: at_ticks(0).unwrap(),
                            duration: at_ticks(40_000_000).unwrap(),
                        },
                        &asset,
                        at_ticks(0).unwrap(),
                    )
                    .unwrap();
                let track = composition
                    .tracksWithMediaType(AVMediaTypeVideo.unwrap())
                    .firstObject()
                    .unwrap();
                let transform = objc2_core_foundation::CGAffineTransform {
                    a: 0.,
                    b: 1.,
                    c: -1.,
                    d: 0.,
                    tx: f64::from(original.height),
                    ty: 0.,
                };
                let _: () = msg_send![&*track,setPreferredTransform:transform];
            }
            // Native passthrough writes an actual rotated track without baking
            // that rotation into the pixels first. The renderer must handle it.
            let session = unsafe {
                AVAssetExportSession::exportSessionWithAsset_presetName(
                    &composition,
                    objc2_av_foundation::AVAssetExportPresetPassthrough,
                )
            }
            .unwrap();
            let url =
                NSURL::fileURLWithPath(&NSString::from_str(destination.path().to_str().unwrap()));
            unsafe {
                session.setOutputURL(Some(&url));
                session.setOutputFileType(Some(AVFileTypeMPEG4.unwrap()));
                session.setTimeRange(CMTimeRange {
                    start: at_ticks(0).unwrap(),
                    duration: at_ticks(40_000_000).unwrap(),
                });
            }
            let mut flight = ExportFlight::start(session);
            flight.wait(&destination, &cancel, &|_| {}).unwrap();
            drop(flight);
            let mut staged = Source::open(destination.path(), &cancel).unwrap();
            let rotated = inspect(&mut staged, &cancel).unwrap();
            assert_eq!(
                (rotated.width, rotated.height),
                (original.height, original.width)
            );
            assert!(rotated.has_audio);
            let rotated_asset = asset_for(&staged).unwrap();
            loaded(&rotated_asset, &cancel).unwrap();
            let transform = unsafe {
                rotated_asset
                    .tracksWithMediaType(AVMediaTypeVideo.unwrap())
                    .firstObject()
                    .unwrap()
                    .preferredTransform()
            };
            assert_eq!(
                [transform.a, transform.b, transform.c, transform.d],
                [0., 1., -1., 0.]
            );
            destination.publish(&staged.file).unwrap();
            destination.commit();
            let png = directory.join("verified-top-left.png");
            let mut pixels = image::RgbaImage::from_pixel(64, 64, image::Rgba([250, 0, 20, 255]));
            for y in 32..64 {
                for x in 0..64 {
                    pixels.put_pixel(x, y, image::Rgba([0, 240, 250, 255]));
                }
            }
            pixels.save(&png).unwrap();
            let overlay = OverlayRaster {
                png: png.clone(),
                png_sha256: format!("{:x}", Sha256::digest(std::fs::read(&png).unwrap())),
                width: 64,
                height: 64,
                bounds: crate::video_clip_contract::OverlayBounds {
                    x: 20.,
                    y: 20.,
                    width: 64.,
                    height: 64.,
                },
                interval: VideoRange {
                    start_ticks: 0,
                    end_ticks: rotated.duration_ticks,
                },
            };
            let output = directory.join("90-degree-annotated.mp4");
            let mut rotated_source = Source::open(&rotated_path, &cancel).unwrap();
            let actual = render(
                &mut rotated_source,
                &output,
                &rotated,
                None,
                &[overlay],
                &cancel,
                &|_| {},
            )
            .unwrap();
            validate_rendered(
                &rotated,
                &VideoRange {
                    start_ticks: 0,
                    end_ticks: rotated.duration_ticks,
                },
                &actual,
            )
            .unwrap();
            let exported = Source::open(&output, &cancel).unwrap();
            super::super::frames::for_each_decoded(
                &exported,
                &actual,
                &[0, 10_000_000, 30_000_000],
                Duration::from_secs(20),
                &cancel,
                |_, sample, _, _, matrix| {
                    let rgb = super::super::frames::decoded_rgb(
                        sample,
                        matrix,
                        (actual.width, actual.height),
                        &actual,
                        &cancel,
                    )?;
                    let red = rgb.get_pixel(36, 36).0;
                    let cyan = rgb.get_pixel(36, 68).0;
                    assert!(red[0] > 200 && red[1] < 60 && red[2] < 80);
                    assert!(cyan[0] < 60 && cyan[1] > 190 && cyan[2] > 190);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(Source::open(&path, &cancel).unwrap().hash, hash);
            println!(
                "{}",
                serde_json::json!({"rotated90Annotated":true,"source":rotated_path,"sourceSha256":format!("{:x}",Sha256::digest(std::fs::read(&rotated_path).unwrap())),"output":output,"outputSha256":format!("{:x}",Sha256::digest(std::fs::read(&output).unwrap())),"actual":actual,"preferredTransform90":true,"topLeftPixels":true,"audio":true,"originalSourceUnchanged":true})
            );
        });
    }
}
