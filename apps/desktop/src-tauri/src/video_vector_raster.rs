// SPDX-License-Identifier: MPL-2.0
//! Private candidate: immutable, complete video vector ink and integer projection.
use crate::{drawing_render, video_annotation_raster as raster};
use mewu_core::{
    Drawing, DrawingKind, DrawingPoint, RichRendererIdentity, VerifiedVideoVectorLayout,
    VideoPixelPoint, VideoPixelRect, VideoVectorContent, VideoVectorLayoutRef,
};
use resvg::usvg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicBool;

const STYLE: &str = "mewu.video-vector.full-ink.integer-projection.v1";
const NO_FONT: &[u8] = b"mewu.video-vector.no-font.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VideoSourceVector {
    Pen {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Line {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Arrow {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Rect {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Ellipse {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Number {
        version: u32,
        source_points: Vec<VideoPixelPoint>,
        color: String,
        number: u16,
        diameter: f64,
    },
}
impl VideoSourceVector {
    pub fn points(&self) -> &[VideoPixelPoint] {
        match self {
            Self::Pen { source_points, .. }
            | Self::Line { source_points, .. }
            | Self::Arrow { source_points, .. }
            | Self::Rect { source_points, .. }
            | Self::Ellipse { source_points, .. }
            | Self::Number { source_points, .. } => source_points,
        }
    }
    pub fn tool(&self) -> crate::plugins::VideoDrawingTool {
        use crate::plugins::VideoDrawingTool as T;
        match self {
            Self::Pen { .. } => T::Pen,
            Self::Line { .. } => T::Line,
            Self::Arrow { .. } => T::Arrow,
            Self::Rect { .. } => T::Rect,
            Self::Ellipse { .. } => T::Ellipse,
            Self::Number { .. } => T::Number,
        }
    }
    pub fn local(&self, origin: VideoPixelPoint) -> VideoVectorContent {
        let points = self
            .points()
            .iter()
            .map(|p| VideoPixelPoint {
                x: p.x - origin.x,
                y: p.y - origin.y,
            })
            .collect();
        match self {
            Self::Pen {
                version,
                color,
                stroke_width,
                ..
            } => VideoVectorContent::Pen {
                version: *version,
                local_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            Self::Line {
                version,
                color,
                stroke_width,
                ..
            } => VideoVectorContent::Line {
                version: *version,
                local_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            Self::Arrow {
                version,
                color,
                stroke_width,
                ..
            } => VideoVectorContent::Arrow {
                version: *version,
                local_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            Self::Rect {
                version,
                color,
                stroke_width,
                ..
            } => VideoVectorContent::Rect {
                version: *version,
                local_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            Self::Ellipse {
                version,
                color,
                stroke_width,
                ..
            } => VideoVectorContent::Ellipse {
                version: *version,
                local_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            Self::Number {
                version,
                color,
                number,
                diameter,
                ..
            } => VideoVectorContent::Number {
                version: *version,
                local_points: points,
                color: color.clone(),
                number: *number,
                diameter: *diameter,
            },
        }
    }
    pub fn from_local(content: &VideoVectorContent, origin: VideoPixelPoint) -> Self {
        let points = content
            .points()
            .iter()
            .map(|p| VideoPixelPoint {
                x: p.x + origin.x,
                y: p.y + origin.y,
            })
            .collect();
        match content {
            VideoVectorContent::Pen {
                version,
                color,
                stroke_width,
                ..
            } => Self::Pen {
                version: *version,
                source_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            VideoVectorContent::Line {
                version,
                color,
                stroke_width,
                ..
            } => Self::Line {
                version: *version,
                source_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            VideoVectorContent::Arrow {
                version,
                color,
                stroke_width,
                ..
            } => Self::Arrow {
                version: *version,
                source_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            VideoVectorContent::Rect {
                version,
                color,
                stroke_width,
                ..
            } => Self::Rect {
                version: *version,
                source_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            VideoVectorContent::Ellipse {
                version,
                color,
                stroke_width,
                ..
            } => Self::Ellipse {
                version: *version,
                source_points: points,
                color: color.clone(),
                stroke_width: *stroke_width,
            },
            VideoVectorContent::Number {
                version,
                color,
                number,
                diameter,
                ..
            } => Self::Number {
                version: *version,
                source_points: points,
                color: color.clone(),
                number: *number,
                diameter: *diameter,
            },
        }
    }
    // A temporary pure geometry adapter only: never persisted as a Drawing,
    // and never accompanied by a fabricated Region or screenshot Asset.
    fn drawing(&self, origin: VideoPixelPoint) -> Result<Drawing, String> {
        let (version, kind, color, stroke, text, size) = match self {
            Self::Pen {
                version,
                color,
                stroke_width,
                ..
            } => (*version, DrawingKind::Pen, color, *stroke_width, None, None),
            Self::Line {
                version,
                color,
                stroke_width,
                ..
            } => (
                *version,
                DrawingKind::Line,
                color,
                *stroke_width,
                None,
                None,
            ),
            Self::Arrow {
                version,
                color,
                stroke_width,
                ..
            } => (
                *version,
                DrawingKind::Arrow,
                color,
                *stroke_width,
                None,
                None,
            ),
            Self::Rect {
                version,
                color,
                stroke_width,
                ..
            } => (
                *version,
                DrawingKind::Rect,
                color,
                *stroke_width,
                None,
                None,
            ),
            Self::Ellipse {
                version,
                color,
                stroke_width,
                ..
            } => (
                *version,
                DrawingKind::Ellipse,
                color,
                *stroke_width,
                None,
                None,
            ),
            Self::Number {
                version,
                color,
                number,
                diameter,
                ..
            } => {
                if !(1..=9999).contains(number) {
                    return Err("视频序号无效".into());
                }
                (
                    *version,
                    DrawingKind::Number,
                    color,
                    1.,
                    Some(number.to_string()),
                    Some(*diameter),
                )
            }
        };
        if version != 1
            || serde_json::to_vec(self)
                .map_err(|_| "视频绘制内容无效")?
                .len()
                > 256 * 1024
        {
            return Err("视频绘制内容超限".into());
        }
        Ok(Drawing {
            id: String::new(),
            kind,
            color: color.clone(),
            stroke_width: stroke,
            points: self
                .points()
                .iter()
                .map(|p| DrawingPoint {
                    x: p.x - origin.x,
                    y: p.y - origin.y,
                })
                .collect(),
            text,
            font_size: size,
            origin: None,
            rich: None,
        })
    }
    pub fn validate(&self, width: u32, height: u32) -> Result<(), String> {
        if width < 2
            || height < 2
            || width > 16384
            || height > 16384
            || u64::from(width) * u64::from(height) > 4_000_000
        {
            return Err("视频来源尺寸无效".into());
        }
        let drawing = self.drawing(VideoPixelPoint { x: 0., y: 0. })?;
        drawing_render::video_vector_svg(&drawing, width, height)?;
        if let Self::Number {
            source_points,
            diameter,
            ..
        } = self
        {
            let p = source_points.first().ok_or("视频序号位置无效")?;
            if p.x + diameter > f64::from(width) || p.y + diameter > f64::from(height) {
                return Err("视频序号超出原视频".into());
            }
        }
        Ok(())
    }
}

pub struct RenderedVector {
    pub top_left: VideoPixelPoint,
    pub layout: VerifiedVideoVectorLayout,
}
fn options(text: bool) -> Result<usvg::Options<'static>, String> {
    let mut opts = usvg::Options::default();
    opts.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    if text {
        opts.fontdb = drawing_render::fonts()?;
        opts.font_family = "Segoe UI".into();
    }
    Ok(opts)
}
pub fn render(
    source: &VideoSourceVector,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<RenderedVector, String> {
    crate::video_host::check_cancel(cancel)?;
    source.validate(width, height)?;
    let original = source.drawing(VideoPixelPoint { x: 0., y: 0. })?;
    let number = matches!(source, VideoSourceVector::Number { .. });
    let opts = options(number)?;
    let svg = drawing_render::video_vector_svg(&original, width, height)?;
    let tree = usvg::Tree::from_str(&svg, &opts).map_err(|_| "无法绘制视频标注")?;
    if raster::has_missing_glyph(tree.root()) {
        return Err("视频序号字体不可用".into());
    }
    let b = tree.root().abs_stroke_bounding_box();
    let top_left = VideoPixelPoint {
        x: f64::from(b.x()).floor(),
        y: f64::from(b.y()).floor(),
    };
    let (w, h) = raster::dimensions(
        f64::from(b.right()).ceil() - top_left.x,
        f64::from(b.bottom()).ceil() - top_left.y,
    )?;
    // Complete ink is kept even when a stroke/cap/arrow extends outside source.
    let local = source.drawing(top_left)?;
    let svg = drawing_render::video_vector_svg(&local, w, h)?;
    let tree = usvg::Tree::from_str(&svg, &opts).map_err(|_| "无法绘制视频标注")?;
    if raster::has_missing_glyph(tree.root()) {
        return Err("视频序号字体不可用".into());
    }
    crate::video_host::check_cancel(cancel)?;
    let png = raster::rasterize(&tree, w, h)?;
    crate::video_host::check_cancel(cancel)?;
    let font_sha256 = if number {
        crate::rich_annotation_host::table_font_identity()?
    } else {
        format!("{:x}", Sha256::digest(NO_FONT))
    };
    let layout = VerifiedVideoVectorLayout::new(
        uuid::Uuid::new_v4().to_string(),
        source.local(top_left),
        RichRendererIdentity {
            id: "mewu.video-vector.resvg".into(),
            version: "resvg-0.48.1".into(),
            font_sha256,
            style_version: 1,
        },
        w,
        h,
        png,
    )
    .map_err(|e| e.to_string())?;
    Ok(RenderedVector { top_left, layout })
}

/// Move never decodes points or changes the immutable full PNG. Only this
/// consumer projection clips against the source, with an exact integer crop.
pub fn project(
    top_left: VideoPixelPoint,
    reference: &VideoVectorLayoutRef,
    layout: &VerifiedVideoVectorLayout,
    source_width: u32,
    source_height: u32,
) -> Result<raster::Raster, String> {
    let geometry = reference.geometry_bounds;
    if source_width == 0
        || source_height == 0
        || u64::from(source_width) * u64::from(source_height) > 4_000_000
        || top_left.x + geometry.x < 0.
        || top_left.y + geometry.y < 0.
        || top_left.x + geometry.x + geometry.width > f64::from(source_width)
        || top_left.y + geometry.y + geometry.height > f64::from(source_height)
    {
        return Err("视频绘制控制点超出原视频".into());
    }
    if layout.reference() != reference
        || !top_left.x.is_finite()
        || !top_left.y.is_finite()
        || top_left.x.fract() != 0.
        || top_left.y.fract() != 0.
        || top_left.x < -256.
        || top_left.y < -256.
        || top_left.x > f64::from(source_width)
        || top_left.y > f64::from(source_height)
    {
        return Err("视频绘制图层位置或引用无效".into());
    }
    let x = top_left.x as i64;
    let y = top_left.y as i64;
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + i64::from(reference.width)).min(i64::from(source_width));
    let bottom = (y + i64::from(reference.height)).min(i64::from(source_height));
    if right <= left || bottom <= top {
        return Err("视频绘制图层超出原视频".into());
    }
    let (w, h) = ((right - left) as u32, (bottom - top) as u32);
    let rgba = raster::decode(layout.png(), reference.width, reference.height)?;
    let png = if left == x && top == y && w == reference.width && h == reference.height {
        layout.png().to_vec()
    } else {
        let cropped =
            image::imageops::crop_imm(&rgba, (left - x) as u32, (top - y) as u32, w, h).to_image();
        raster::encode_png(cropped.as_raw(), w, h)?
    };
    let key = serde_json::to_vec(&(STYLE, top_left, reference, source_width, source_height))
        .map_err(|_| "视频绘制身份无效")?;
    Ok(raster::Raster {
        reference: raster::RasterRef {
            raster_key: format!("{:x}", Sha256::digest(&key)),
            png_sha256: format!("{:x}", Sha256::digest(&png)),
            width: w,
            height: h,
        },
        bounds: VideoPixelRect {
            x: left as f64,
            y: top as f64,
            width: f64::from(w),
            height: f64::from(h),
        },
        png,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f64, y: f64) -> VideoPixelPoint {
        VideoPixelPoint { x, y }
    }
    #[test]
    fn edge_dot_keeps_full_ink_and_integer_move_restores_it() {
        let s = VideoSourceVector::Pen {
            version: 1,
            source_points: vec![p(0., 0.)],
            color: "#123456".into(),
            stroke_width: 12.,
        };
        let rendered = render(&s, 100, 100, &AtomicBool::new(false)).unwrap();
        assert!(rendered.top_left.x < 0. && rendered.top_left.y < 0.);
        let full = rendered.layout.png().to_vec();
        let clipped = project(
            rendered.top_left,
            rendered.layout.reference(),
            &rendered.layout,
            100,
            100,
        )
        .unwrap();
        assert_eq!((clipped.bounds.x, clipped.bounds.y), (0., 0.));
        let inside = project(
            p(20., 20.),
            rendered.layout.reference(),
            &rendered.layout,
            100,
            100,
        )
        .unwrap();
        assert_eq!(inside.png, full);
        assert!(inside.reference.width > clipped.reference.width);
        assert!(project(
            p(-0.5, 0.),
            rendered.layout.reference(),
            &rendered.layout,
            100,
            100
        )
        .is_err());
    }
    #[test]
    fn complete_arrow_bbox_contains_wings_and_style_keeps_source_points() {
        let s = VideoSourceVector::Arrow {
            version: 1,
            source_points: vec![p(10.25, 40.5), p(1.25, 40.5)],
            color: "#ff0088".into(),
            stroke_width: 20.,
        };
        let r = render(&s, 100, 100, &AtomicBool::new(false)).unwrap();
        assert!(r.top_left.x < 0.);
        assert_eq!(
            VideoSourceVector::from_local(r.layout.content(), r.top_left),
            s
        );
        let image = raster::decode(r.layout.png(), r.layout.width(), r.layout.height()).unwrap();
        assert!(image.pixels().any(|p| p[3] == 255));
    }
    #[test]
    fn strict_source_wire_limits_and_cancel_precede_render() {
        for kind in ["mosaic", "rich", "highlighter", "text"] {
            assert!(serde_json::from_value::<VideoSourceVector>(serde_json::json!({"kind":kind,"version":1,"sourcePoints":[{"x":0,"y":0}],"color":"#ffffff","strokeWidth":2})).is_err());
        }
        let s = VideoSourceVector::Pen {
            version: 1,
            source_points: vec![p(5., 5.); 4097],
            color: "#ffffff".into(),
            stroke_width: 2.,
        };
        assert!(s.validate(100, 100).is_err());
        let s = VideoSourceVector::Line {
            version: 1,
            source_points: vec![p(1., 1.), p(20., 20.)],
            color: "#ffffff".into(),
            stroke_width: 2.,
        };
        assert!(render(&s, 100, 100, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn all_six_vector_bounds_contain_unclipped_constant_margin_ink() {
        let items = vec![
            VideoSourceVector::Pen {
                version: 1,
                source_points: vec![p(0., 0.), p(1.25, 12.75), p(10.5, 2.25)],
                color: "#bf2468".into(),
                stroke_width: 64.,
            },
            VideoSourceVector::Line {
                version: 1,
                source_points: vec![p(0., 10.), p(30., 0.)],
                color: "#bf2468".into(),
                stroke_width: 64.,
            },
            VideoSourceVector::Arrow {
                version: 1,
                source_points: vec![p(2., 2.), p(20., 0.)],
                color: "#bf2468".into(),
                stroke_width: 64.,
            },
            VideoSourceVector::Rect {
                version: 1,
                source_points: vec![p(0., 0.), p(10., 10.)],
                color: "#bf2468".into(),
                stroke_width: 64.,
            },
            VideoSourceVector::Ellipse {
                version: 1,
                source_points: vec![p(0., 0.), p(12., 20.)],
                color: "#bf2468".into(),
                stroke_width: 64.,
            },
            VideoSourceVector::Number {
                version: 1,
                source_points: vec![p(0., 0.)],
                color: "#bf2468".into(),
                number: 9999,
                diameter: 64.,
            },
        ];
        for source in items {
            let complete = render(&source, 160, 120, &AtomicBool::new(false)).unwrap();
            let actual = raster::decode(
                complete.layout.png(),
                complete.layout.width(),
                complete.layout.height(),
            )
            .unwrap();
            let shifted = source.drawing(p(-256., -256.)).unwrap();
            let svg = drawing_render::video_vector_svg(&shifted, 672, 632).unwrap();
            let tree = usvg::Tree::from_str(
                &svg,
                &options(matches!(source, VideoSourceVector::Number { .. })).unwrap(),
            )
            .unwrap();
            let reference =
                raster::decode(&raster::rasterize(&tree, 672, 632).unwrap(), 672, 632).unwrap();
            // Independent large viewport must not expose ink beyond the tight
            // immutable image. This is a no-cropping assertion, not a claim
            // about floating rasterizer translations being bit-identical.
            let ox = (complete.top_left.x + 256.) as u32;
            let oy = (complete.top_left.y + 256.) as u32;
            assert!(actual.pixels().any(|p| p[3] != 0));
            assert!(reference.pixels().any(|p| p[3] != 0));
            for (x, y, pixel) in reference.enumerate_pixels() {
                if pixel[3] != 0 {
                    assert!(
                        x >= ox && y >= oy && x < ox + actual.width() && y < oy + actual.height(),
                        "{source:?}: clipped ink at {x},{y}"
                    );
                }
            }
        }
    }
}
