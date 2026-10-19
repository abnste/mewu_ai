// SPDX-License-Identifier: MPL-2.0
//! Physical-pixel presentation only. No acquisition, input or persistent geometry.
//! Mirrors the original UpdateLongCapturePreview: grow with the image until the
//! available height is reached. Every UI rectangle stays outside acquired pixels.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BoxRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl BoxRect {
    pub fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }
    pub fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }
    fn intersects(self, other: Self) -> bool {
        i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }
    fn contains(self, other: Self) -> bool {
        self.x <= other.x
            && self.y <= other.y
            && self.right() >= other.right()
            && self.bottom() >= other.bottom()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub controls: Option<BoxRect>,
    pub preview: Option<BoxRect>,
}

pub(super) fn layout(
    work: BoxRect,
    roi: BoxRect,
    scale: f64,
    image: (u32, u32),
    f8: bool,
) -> Result<Layout, String> {
    if !scale.is_finite()
        || !(0.5..=8.).contains(&scale)
        || image.0 == 0
        || image.1 == 0
        || work.width == 0
        || work.height == 0
    {
        return Err("长截图预览尺寸无效".into());
    }
    let margin = (6. * scale).ceil() as i64;
    let gap = (10. * scale).ceil() as i64;
    let px = |value: f64| value.round().max(1.).min(u32::MAX as f64) as u32;
    let available = |side: u32| (i64::from(side) - margin * 2).max(1) as u32;
    let control_size = (
        px(300. * scale).min(available(work.width)),
        px(76. * scale).min(available(work.height)),
    );
    let candidate = |x: i64, y: i64, size: (u32, u32)| -> Option<BoxRect> {
        let value = BoxRect {
            x: i32::try_from(x).ok()?,
            y: i32::try_from(y).ok()?,
            width: size.0,
            height: size.1,
        };
        (work.contains(value) && !roi.intersects(value)).then_some(value)
    };
    let center_x = (i64::from(roi.x) + (i64::from(roi.width) - i64::from(control_size.0)) / 2)
        .clamp(
            i64::from(work.x) + margin,
            (work.right() - i64::from(control_size.0) - margin).max(i64::from(work.x) + margin),
        );
    let side_y = i64::from(roi.y).clamp(
        i64::from(work.y) + margin,
        (work.bottom() - i64::from(control_size.1) - margin).max(i64::from(work.y) + margin),
    );
    let controls = [
        (center_x, i64::from(roi.y) - i64::from(control_size.1) - gap),
        (center_x, roi.bottom() + gap),
        (roi.right() + gap, side_y),
        (i64::from(roi.x) - i64::from(control_size.0) - gap, side_y),
    ]
    .into_iter()
    .find_map(|(x, y)| candidate(x, y, control_size));
    if controls.is_none() && !f8 {
        return Err("F8 被占用，无法在选区外放置长截图控制条".into());
    }
    let width = px((f64::from(roi.width) * 0.34).clamp(120. * scale, 220. * scale))
        .min(available(work.width));
    let height = px(f64::from(width) * f64::from(image.1) / f64::from(image.0))
        .max(px(48. * scale))
        .min(px(f64::from(work.height) * 0.68))
        .min(available(work.height));
    let size = (width, height);
    let bottom = roi.bottom().min(work.bottom() - margin);
    let y = (bottom - i64::from(height)).max(i64::from(work.y) + margin);
    let left = i64::from(roi.x) - i64::from(width) - gap;
    let right = roi.right() + gap;
    let mut positions = vec![(left, y), (right, y)];
    if let Some(control) = controls {
        // A short screen may require the preview to grow away from its controls.
        for x in [left, right] {
            positions.push((x, control.bottom() + gap));
            positions.push((x, i64::from(control.y) - i64::from(height) - gap));
        }
    }
    let preview_center_x = (i64::from(roi.x) + (i64::from(roi.width) - i64::from(width)) / 2)
        .clamp(
            i64::from(work.x) + margin,
            (work.right() - i64::from(width) - margin).max(i64::from(work.x) + margin),
        );
    positions.extend([
        (preview_center_x, i64::from(roi.y) - i64::from(height) - gap),
        (preview_center_x, roi.bottom() + gap),
    ]);
    let preview = positions.into_iter().find_map(|(x, y)| {
        candidate(x, y, size)
            .filter(|value| !controls.is_some_and(|control| control.intersects(*value)))
    });
    // At a screen edge a growing image may outgrow the original side. Fit its
    // whole thumbnail into another genuinely free band instead of disappearing
    // or clamping the preview over the acquisition rectangle.
    let preview = preview.or_else(|| {
        let left = i64::from(work.x) + margin;
        let top = i64::from(work.y) + margin;
        let right = work.right() - margin;
        let bottom = work.bottom() - margin;
        let bands = [
            (left, top, i64::from(roi.x) - gap, bottom),
            (roi.right() + gap, top, right, bottom),
            (left, top, right, i64::from(roi.y) - gap),
            (left, roi.bottom() + gap, right, bottom),
        ];
        for (l, t, r, b) in bands {
            let (l, t, r, b) = (l.max(left), t.max(top), r.min(right), b.min(bottom));
            let mut free = vec![(l, t, r, b)];
            if let Some(c) = controls {
                free.extend([
                    (l, t, i64::from(c.x) - gap, b),
                    (c.right() + gap, t, r, b),
                    (l, t, r, i64::from(c.y) - gap),
                    (l, c.bottom() + gap, r, b),
                ]);
            }
            for (l, t, r, b) in free {
                let (l, t, r, b) = (l.max(left), t.max(top), r.min(right), b.min(bottom));
                if r <= l || b <= t {
                    continue;
                }
                let w = width.min(u32::try_from(r - l).ok()?);
                let h = px(f64::from(w) * f64::from(image.1) / f64::from(image.0))
                    .min(height)
                    .min(u32::try_from(b - t).ok()?);
                if w < px(48. * scale) || h < px(24. * scale) {
                    continue;
                }
                if let Some(value) = candidate(
                    l,
                    (roi.bottom() - i64::from(h)).clamp(t, b - i64::from(h)),
                    (w, h),
                )
                .filter(|value| !controls.is_some_and(|control| control.intersects(*value)))
                {
                    return Some(value);
                }
            }
        }
        None
    });
    Ok(Layout { controls, preview })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_grows_from_the_first_frame_until_the_old_height_cap() {
        let work = BoxRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let roi = BoxRect {
            x: 600,
            y: 300,
            width: 600,
            height: 400,
        };
        let mut heights = Vec::new();
        for image_height in [400, 800, 1200, 2400, 4800] {
            let frame = layout(work, roi, 1., (600, image_height), true).unwrap();
            let preview = frame.preview.unwrap();
            heights.push(preview.height);
            assert!(!roi.intersects(preview));
            assert!(work.contains(preview));
            assert!(!frame.controls.unwrap().intersects(preview));
        }
        assert!(heights[0] < heights[1] && heights[1] < heights[2] && heights[2] < heights[3]);
        assert_eq!(heights[3], heights[4]);
    }
    #[test]
    fn negative_origins_mixed_dpi_and_screen_edges_never_cover_the_acquired_region() {
        for scale in [0.75, 1., 1.25, 1.5, 2.] {
            let work = BoxRect {
                x: -2560,
                y: -200,
                width: 2560,
                height: 1360,
            };
            for (x, y, w, h) in [
                (-2560, -200, 640, 480),
                (-650, 300, 640, 800),
                (-1900, 100, 900, 650),
                (-2500, 100, 2400, 900),
            ] {
                let roi = BoxRect {
                    x,
                    y,
                    width: w,
                    height: h,
                };
                for image_height in [h, h * 2, h * 8] {
                    let result = layout(work, roi, scale, (w, image_height), true).unwrap();
                    for rectangle in [result.controls, result.preview].into_iter().flatten() {
                        assert!(work.contains(rectangle));
                        assert!(!roi.intersects(rectangle));
                    }
                    if let (Some(a), Some(b)) = (result.controls, result.preview) {
                        assert!(!a.intersects(b));
                    }
                }
            }
        }
    }
    #[test]
    fn fullscreen_keeps_explicit_hotkey_fallback_and_never_invents_room_inside_pixels() {
        let roi = BoxRect {
            x: -320,
            y: 0,
            width: 320,
            height: 360,
        };
        assert_eq!(
            layout(roi, roi, 1.5, (320, 1000), true).unwrap(),
            Layout {
                controls: None,
                preview: None
            }
        );
        assert!(layout(roi, roi, 1.5, (320, 1000), false).is_err());
    }
    #[test]
    fn nearly_full_width_selection_keeps_a_visible_growing_preview_in_the_free_top_band() {
        let work = BoxRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        };
        let roi = BoxRect {
            x: 10,
            y: 280,
            width: 1260,
            height: 510,
        };
        for h in [510, 1020, 2040, 8192] {
            let frame = layout(work, roi, 1., (1260, h), true).unwrap();
            let preview = frame
                .preview
                .expect("a real 280px band remains above the selection");
            assert!(work.contains(preview));
            assert!(!roi.intersects(preview));
            assert!(!frame.controls.unwrap().intersects(preview));
        }
    }
}
