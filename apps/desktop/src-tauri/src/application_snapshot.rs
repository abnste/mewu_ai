// SPDX-License-Identifier: MPL-2.0
//! Automatic window capture: verified pixels determine stitching, UIA only
//! moves the selected viewport. Publication is gated on confirmed restoration.
use crate::scroll_stitch::{ScrollStitcher, StitchLimits, StitchState, StitchStatus};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScrollPosition {
    pub viewport: Viewport,
    pub horizontal: f64,
    pub vertical: f64,
    pub view_size: f64,
}
impl ScrollPosition {
    pub fn validate(&self) -> Result<(), String> {
        let r = self.viewport;
        if r.width < 16
            || r.height < 64
            || r.width > 8192
            || r.height > 8192
            || r.width as u64 * r.height as u64 > 4 * 1024 * 1024
            || !self.vertical.is_finite()
            || !(0.0..=100.0).contains(&self.vertical)
            || !self.horizontal.is_finite()
            || !(self.horizontal == -1.0 || (0.0..=100.0).contains(&self.horizontal))
            || !self.view_size.is_finite()
            || !(0.0..100.0).contains(&self.view_size)
            || self.view_size == 0.0
        {
            return Err("应用未提供有效的滚动区域".into());
        }
        Ok(())
    }
    pub fn same_view(&self, other: &Self) -> bool {
        self.viewport == other.viewport
            && (self.view_size - other.view_size).abs() <= 0.001
            && (self.horizontal - other.horizontal).abs() <= 0.05
    }
    pub fn restored(&self, other: &Self) -> bool {
        self.same_view(other) && (self.vertical - other.vertical).abs() <= 0.05
    }
}
pub(crate) struct WindowFrame {
    pub state: ScrollPosition,
    pub image: RgbaImage,
}
pub(crate) trait Session {
    fn original(&self) -> &ScrollPosition;
    fn frame_at(&mut self, position: f64, cancel: &AtomicBool) -> Result<WindowFrame, String>;
    // This operation must ignore cancellation so restoration is still attempted.
    fn restore(&mut self) -> Result<ScrollPosition, String>;
}
fn check_frame(
    original: &ScrollPosition,
    frame: &WindowFrame,
    expected: f64,
) -> Result<(), String> {
    frame.state.validate()?;
    if !original.same_view(&frame.state)
        || (frame.state.vertical - expected).abs() > 0.05
        || (frame.image.width(), frame.image.height())
            != (original.viewport.width, original.viewport.height)
    {
        return Err("应用滚动区域或位置已改变，请重新截图".into());
    }
    Ok(())
}
pub(crate) fn capture<S: Session>(
    session: &mut S,
    cancel: &AtomicBool,
    mut preview: impl FnMut(&ScrollStitcher, StitchState) -> Result<(), String>,
) -> Result<Option<RgbaImage>, String> {
    let original = session.original().clone();
    original.validate()?;
    let started = Instant::now();
    let result = (|| {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let first = session.frame_at(0.0, cancel)?;
        check_frame(&original, &first, 0.0)?;
        let mut stitch = ScrollStitcher::new(
            first.image,
            StitchLimits {
                max_output_height: 16384,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        preview(&stitch, stitch.state())?;
        let mut position = first.state.vertical;
        let nominal_step =
            (original.view_size / (100.0 - original.view_size) * 25.0).clamp(0.001, 100.0);
        while position < 99.95 {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            if started.elapsed() > Duration::from_secs(240) {
                return Err("应用快照超时，未保存未完成的画面".into());
            }
            let prior = stitch.state();
            let mut step = nominal_step.min(100.0 - position);
            let mut accepted = None;
            for _ in 0..6 {
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                let next = (position + step).min(100.0);
                let frame = session.frame_at(next, cancel)?;
                check_frame(&original, &frame, next)?;
                let state = stitch
                    .accept_with_cancel(frame.image, cancel)
                    .map_err(|e| e.to_string())?;
                if state.status == StitchStatus::LimitReached {
                    return Err("应用快照超过图像容量，未保存不完整画面".into());
                }
                if state.status == StitchStatus::Extended
                    && state.offset > prior.offset
                    && state.added_bottom > 0
                {
                    accepted = Some((frame.state.vertical, state));
                    break;
                }
                // Rejected frames never contribute output strips. Retry closer
                // to the last accepted position, not to the last attempted one.
                if state.offset != prior.offset || state.height != prior.height {
                    return Err("应用画面未按预期向下滚动".into());
                }
                step *= 0.5;
            }
            let (next, state) = accepted.ok_or("无法确认应用画面的连续性，请使用手动长截图")?;
            if next <= position + 0.00001 {
                return Err("应用滚动未前进".into());
            }
            position = next;
            preview(&stitch, state)?;
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        stitch.finish().map(Some).map_err(|e| e.to_string())
    })();
    // Never publish a successful or partial image before this acknowledgement.
    match session.restore() {
        Ok(restored) if original.restored(&restored) => {
            if cancel.load(Ordering::Acquire) {
                Ok(None)
            } else {
                result
            }
        }
        _ => Err("未能确认应用已恢复原滚动位置；快照未保存".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        original: ScrollPosition,
        restores: usize,
        fail_restore: bool,
        cancel_after: bool,
        fail_frame: bool,
        blank: bool,
        requests: Vec<f64>,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                original: ScrollPosition {
                    viewport: Viewport {
                        x: -800,
                        y: 20,
                        width: 96,
                        height: 96,
                    },
                    horizontal: -1.0,
                    vertical: 42.0,
                    view_size: 25.0,
                },
                restores: 0,
                fail_restore: false,
                cancel_after: false,
                fail_frame: false,
                blank: false,
                requests: vec![],
            }
        }
    }
    fn pixels(top: u32, blank: bool) -> RgbaImage {
        RgbaImage::from_fn(96, 96, |x, y| {
            let y = y + top;
            if blank {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([
                    ((x * 17 + y * 23) % 251) as u8,
                    ((x * 29 + y * 11) % 253) as u8,
                    ((x * 3 + y * 37) % 249) as u8,
                    255,
                ])
            }
        })
    }
    impl Session for Fake {
        fn original(&self) -> &ScrollPosition {
            &self.original
        }
        fn frame_at(&mut self, p: f64, cancel: &AtomicBool) -> Result<WindowFrame, String> {
            self.requests.push(p);
            if self.fail_frame {
                return Err("synthetic source failure".into());
            }
            let mut state = self.original.clone();
            state.vertical = p;
            if self.cancel_after {
                cancel.store(true, Ordering::Release);
            }
            Ok(WindowFrame {
                state,
                image: pixels((p / 100.0 * 288.0).round() as u32, self.blank),
            })
        }
        fn restore(&mut self) -> Result<ScrollPosition, String> {
            self.restores += 1;
            if self.fail_restore {
                Err("synthetic restore failure".into())
            } else {
                Ok(self.original.clone())
            }
        }
    }
    #[test]
    fn automatic_capture_uses_verified_overlap_and_restores_before_returning_all_pixels() {
        let mut session = Fake::new();
        let mut seen = 0;
        let result = capture(&mut session, &AtomicBool::new(false), |_, s| {
            seen += 1;
            assert!(s.width == 96);
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_eq!(session.restores, 1);
        assert!(seen > 1);
        assert_eq!(session.requests[0], 0.0);
        assert!((session.requests.last().unwrap() - 100.0).abs() < 0.05);
        assert_eq!(result.dimensions(), (96, 384));
        assert_eq!(
            result,
            RgbaImage::from_fn(96, 384, |x, y| {
                image::Rgba([
                    ((x * 17 + y * 23) % 251) as u8,
                    ((x * 29 + y * 11) % 253) as u8,
                    ((x * 3 + y * 37) % 249) as u8,
                    255,
                ])
            })
        );
    }
    #[test]
    fn errors_cancellation_ambiguous_pixels_and_preview_failure_all_restore_and_never_publish_partial(
    ) {
        for mode in 0..4 {
            let mut session = Fake::new();
            session.fail_frame = mode == 0;
            session.cancel_after = mode == 1;
            session.blank = mode == 2;
            let result = capture(&mut session, &AtomicBool::new(false), |_, _| {
                if mode == 3 {
                    Err("synthetic preview failure".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(session.restores, 1);
            if mode == 1 {
                assert!(result.unwrap().is_none());
            } else {
                assert!(result.is_err());
            }
        }
    }
    #[test]
    fn restoration_is_required_even_when_already_cancelled_and_failed_restoration_discards_success()
    {
        let mut session = Fake::new();
        assert!(capture(&mut session, &AtomicBool::new(true), |_, _| Ok(()))
            .unwrap()
            .is_none());
        assert_eq!(session.restores, 1);
        assert!(session.requests.is_empty());
        let mut session = Fake::new();
        session.fail_restore = true;
        assert!(
            capture(&mut session, &AtomicBool::new(false), |_, _| Ok(()))
                .unwrap_err()
                .contains("恢复原滚动位置")
        );
        assert_eq!(session.restores, 1);
    }
    #[test]
    fn position_and_image_contract_rejects_invalid_or_changed_viewports() {
        let original = Fake::new().original;
        let mut frame = WindowFrame {
            state: original.clone(),
            image: pixels(0, false),
        };
        assert!(check_frame(&original, &frame, 42.0).is_ok());
        frame.state.viewport.x += 1;
        assert!(check_frame(&original, &frame, 42.0).is_err());
        frame.state = original.clone();
        frame.state.vertical = 43.0;
        assert!(check_frame(&original, &frame, 42.0).is_err());
        for view in [0.0, 100.0, f64::NAN, f64::INFINITY] {
            let mut value = original.clone();
            value.view_size = view;
            assert!(value.validate().is_err());
        }
        let mut wrong = original.clone();
        wrong.horizontal = 0.0;
        assert!(!original.restored(&wrong));
    }
}
