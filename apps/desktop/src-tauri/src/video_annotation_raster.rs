// SPDX-License-Identifier: MPL-2.0
//! One trusted raster for video preview and every materialized media output.
//! Model text is escaped literal content; SVG, paths and renderer IDs are ours.
use image::{ImageDecoder, ImageEncoder};
use mewu_core::{
    RichRendererIdentity, TimedVideoAnnotation, VerifiedVideoTextLayout, VideoAnnotationPrimitive,
    VideoPixelRect, VideoTextContent,
};
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const MAX_PIXELS: u64 = 4_000_000;
pub const MAX_TOTAL_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_PNG_BYTES: usize = 32 * 1024 * 1024;
const STYLE: &str = "mewu.video-raster.resvg-0.48.1.area-ring.v2";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RasterRef {
    pub raster_key: String,
    pub png_sha256: String,
    pub width: u32,
    pub height: u32,
}
pub struct Raster {
    pub reference: RasterRef,
    pub bounds: VideoPixelRect,
    pub png: Vec<u8>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn dimensions(width: f64, height: f64) -> Result<(u32, u32), String> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || width > 6000.
        || height > 6000.
    {
        return Err("视频批注尺寸无效".into());
    }
    let (w, h) = (width.ceil() as u32, height.ceil() as u32);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("视频批注图层过大".into());
    }
    Ok((w, h))
}
fn hex_color(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err("视频批注颜色无效".into());
    }
    Ok(())
}
fn options(text: bool) -> Result<usvg::Options<'static>, String> {
    let mut options = usvg::Options::default();
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    if text {
        options.fontdb = crate::drawing_render::fonts()?;
        options.font_family = "Segoe UI".into();
    }
    Ok(options)
}
pub(crate) fn rasterize(tree: &usvg::Tree, width: u32, height: u32) -> Result<Vec<u8>, String> {
    dimensions(f64::from(width), f64::from(height))?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("视频批注图层过大")?;
    resvg::render(tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixmap.pixels() {
        let pixel = pixel.demultiply();
        rgba.extend_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
    }
    encode_png(&rgba, width, height)
}
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|_| "无法生成视频批注图层")?;
    decode(&png, width, height)?;
    Ok(png)
}

/// Exact box-filter coverage for an axis-aligned inside border. The outer box
/// occupies the whole raster; its one inner hole is subtracted before opacity
/// is applied, so corners do not receive alpha twice. Keeping this arithmetic
/// in f64 avoids path/scanline quantization swallowing very narrow edges.
fn rectangle_png(
    bounds: &VideoPixelRect,
    style: &mewu_core::VideoRectStyle,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let color = [
        u8::from_str_radix(&style.color[1..3], 16),
        u8::from_str_radix(&style.color[3..5], 16),
        u8::from_str_radix(&style.color[5..7], 16),
    ];
    let [Ok(red), Ok(green), Ok(blue)] = color else {
        return Err("视频批注颜色无效".into());
    };
    let x_inset = style.stroke_width * f64::from(width) / bounds.width;
    let y_inset = style.stroke_width * f64::from(height) / bounds.height;
    let right = f64::from(width) - x_inset;
    let bottom = f64::from(height) - y_inset;
    let hole = x_inset < right && y_inset < bottom;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let inner_y = if hole {
            ((f64::from(y) + 1.).min(bottom) - f64::from(y).max(y_inset)).max(0.)
        } else {
            0.
        };
        for x in 0..width {
            let inner_x = if hole {
                ((f64::from(x) + 1.).min(right) - f64::from(x).max(x_inset)).max(0.)
            } else {
                0.
            };
            let coverage = (1. - inner_x * inner_y).clamp(0., 1.);
            let alpha = (255. * style.opacity * coverage).round() as u8;
            if alpha == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                rgba.extend_from_slice(&[red, green, blue, alpha]);
            }
        }
    }
    encode_png(&rgba, width, height)
}
pub(crate) fn has_missing_glyph(group: &usvg::Group) -> bool {
    group.children().iter().any(|node| {
        let own = match node {
            usvg::Node::Group(group) => has_missing_glyph(group),
            usvg::Node::Text(text) => text
                .layouted()
                .iter()
                .any(|span| span.positioned_glyphs.iter().any(|glyph| glyph.id.0 == 0)),
            _ => false,
        };
        let mut nested = false;
        node.subroots(|group| nested |= has_missing_glyph(group));
        own || nested
    })
}
pub fn decode(png: &[u8], width: u32, height: u32) -> Result<image::RgbaImage, String> {
    dimensions(f64::from(width), f64::from(height))?;
    if png.len() > MAX_PNG_BYTES {
        return Err("视频批注图片过大".into());
    }
    let mut decoder =
        image::codecs::png::PngDecoder::new(Cursor::new(png)).map_err(|_| "视频批注图片损坏")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(MAX_PIXELS * 8);
    decoder.set_limits(limits).map_err(|_| "视频批注图片过大")?;
    if decoder.dimensions() != (width, height) {
        return Err("视频批注图片尺寸不一致".into());
    }
    image::DynamicImage::from_decoder(decoder)
        .map(|image| image.into_rgba8())
        .map_err(|_| "视频批注图片损坏".into())
}

fn literal_text_svg(content: &VideoTextContent) -> Result<(String, f64), String> {
    let mut text = format!("<text fill=\"{}\" font-family=\"Segoe UI, Microsoft YaHei, PingFang SC, Noto Sans CJK SC, DejaVu Sans, sans-serif\" font-size=\"{}\" xml:space=\"preserve\">", content.color, content.font_size);
    let mut last_baseline = 0.;
    for (index, line) in content.text.split('\n').enumerate() {
        last_baseline = 4. + content.font_size * (1. + index as f64 * 1.25);
        let literal = crate::table_document::escape(&line.replace('\t', "    "));
        write!(
            text,
            "<tspan x=\"4\" y=\"{last_baseline}\">{literal}</tspan>"
        )
        .map_err(|_| "无法排版视频批注文字")?;
    }
    text.push_str("</text>");
    Ok((text, last_baseline))
}

pub fn render_text(content: VideoTextContent) -> Result<VerifiedVideoTextLayout, String> {
    hex_color(&content.color)?;
    if content.version != 1
        || content.text.trim().is_empty()
        || content.text.chars().count() > 500
        || content.text.len() > 4096
        || !content.font_size.is_finite()
        || !(8. ..=128.).contains(&content.font_size)
        || content
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("视频批注文字无效".into());
    }
    // The measurable tree converts all literal text to the same native glyphs
    // used by the final raster. No character-count width guess clips CJK/math.
    let (text, last_baseline) = literal_text_svg(&content)?;
    let mut opts = options(true)?;
    let missing = Arc::new(AtomicBool::new(false));
    let flag = missing.clone();
    let fallback = usvg::FontResolver::default_fallback_selector();
    opts.font_resolver.select_fallback = Box::new(move |character, used, database| {
        let result = fallback(character, used, database);
        if result.is_none() {
            flag.store(true, Ordering::Relaxed);
        }
        result
    });
    let initial = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"6000\">{text}</svg>"
    );
    let measured = usvg::Tree::from_str(&initial, &opts).map_err(|_| "无法排版视频批注文字")?;
    if missing.load(Ordering::Relaxed) || has_missing_glyph(measured.root()) {
        return Err("视频批注含当前字体无法绘制的字符".into());
    }
    if measured.root().children().is_empty() {
        return Err("视频批注文字无法排版".into());
    }
    let bounds = measured.root().abs_bounding_box();
    // Font ascenders and bearings may extend above/left of the requested text
    // origin. Translate the complete measured glyphs, never crop or shrink them.
    // Integer translation preserves the raster grid. Keep the old successful
    // SVG byte-for-byte unchanged; its stored PNG/renderer identity stays valid.
    // https://www.w3.org/TR/SVG2/coords.html#TransformProperty
    let translated = bounds.x() < 0. || bounds.y() < 0.;
    let (shift_x, shift_y) = if translated {
        (
            (4. - f64::from(bounds.x())).max(0.).ceil(),
            (4. - f64::from(bounds.y())).max(0.).ceil(),
        )
    } else {
        (0., 0.)
    };
    let (width, height) = dimensions(
        f64::from(bounds.right()) + shift_x + 4.,
        (f64::from(bounds.bottom()) + shift_y + 4.)
            .max(last_baseline + shift_y + content.font_size * 0.3 + 4.),
    )?;
    let text = if translated {
        format!("<g transform=\"translate({shift_x} {shift_y})\">{text}</g>")
    } else {
        text
    };
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">{text}</svg>");
    let tree = usvg::Tree::from_str(&svg, &opts).map_err(|_| "无法排版视频批注文字")?;
    if missing.load(Ordering::Relaxed) || has_missing_glyph(tree.root()) {
        return Err("视频批注含当前字体无法绘制的字符".into());
    }
    let png = rasterize(&tree, width, height)?;
    VerifiedVideoTextLayout::new(
        uuid::Uuid::new_v4().to_string(),
        content,
        RichRendererIdentity {
            id: "mewu.video-text.resvg".into(),
            version: "resvg-0.48.1".into(),
            font_sha256: crate::rich_annotation_host::table_font_identity()?,
            style_version: if translated { 2 } else { 1 },
        },
        width,
        height,
        png,
    )
    .map_err(|e| e.to_string())
}

/// All display/export consumers use the same integer source projection.
pub(crate) fn render_in_source(
    annotation: &TimedVideoAnnotation,
    source_width: u32,
    source_height: u32,
    load_text: impl FnOnce(&mewu_core::VideoTextLayoutRef) -> Result<VerifiedVideoTextLayout, String>,
    load_vector: impl FnOnce(
        &mewu_core::VideoVectorLayoutRef,
    ) -> Result<mewu_core::VerifiedVideoVectorLayout, String>,
) -> Result<Raster, String> {
    if let VideoAnnotationPrimitive::Vector { top_left, layout } = &annotation.primitive {
        return crate::video_vector_raster::project(
            *top_left,
            layout,
            &load_vector(layout)?,
            source_width,
            source_height,
        );
    }
    render(annotation, load_text)
}

pub fn render(
    annotation: &TimedVideoAnnotation,
    load_text: impl FnOnce(&mewu_core::VideoTextLayoutRef) -> Result<VerifiedVideoTextLayout, String>,
) -> Result<Raster, String> {
    let (bounds, png, width, height) = match &annotation.primitive {
        VideoAnnotationPrimitive::Text { top_left, layout } => {
            let text = load_text(layout)?;
            if text.reference() != layout {
                return Err("视频文字图层已改变".into());
            }
            decode(text.png(), layout.width, layout.height)?;
            (
                VideoPixelRect {
                    x: top_left.x,
                    y: top_left.y,
                    width: f64::from(layout.width),
                    height: f64::from(layout.height),
                },
                text.png().to_vec(),
                layout.width,
                layout.height,
            )
        }
        VideoAnnotationPrimitive::Rect { bounds, style } => {
            hex_color(&style.color)?;
            let (width, height) = dimensions(bounds.width, bounds.height)?;
            if !style.stroke_width.is_finite()
                || style.stroke_width <= 0.
                || style.stroke_width > bounds.width.min(bounds.height)
                || !style.opacity.is_finite()
                || !(0. ..=1.).contains(&style.opacity)
            {
                return Err("视频批注矩形样式无效".into());
            }
            (
                *bounds,
                rectangle_png(bounds, style, width, height)?,
                width,
                height,
            )
        }
        VideoAnnotationPrimitive::Vector { .. } => return Err("视频矢量渲染缺少来源尺寸".into()),
    };
    let identity =
        serde_json::to_vec(&(STYLE, &annotation.primitive)).map_err(|_| "视频批注身份无效")?;
    Ok(Raster {
        reference: RasterRef {
            raster_key: hash(&identity),
            png_sha256: hash(&png),
            width,
            height,
        },
        bounds,
        png,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{VideoRange, VideoRectStyle};

    fn text_content(text: &str, font_size: f64) -> VideoTextContent {
        VideoTextContent {
            version: 1,
            text: text.into(),
            color: "#1357b9".into(),
            font_size,
        }
    }

    #[test]
    fn chinese_sizes_have_complete_canvas_and_diagnostic_measured_bounds() {
        for font_size in [8., 16., 24., 40., 64., 72., 80., 96., 128.] {
            let content = text_content("旧新中文\n<&> x²", font_size);
            let (text, _) = literal_text_svg(&content).unwrap();
            let opts = options(true).unwrap();
            let measured = usvg::Tree::from_str(
                &format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"6000\">{text}</svg>"),
                &opts,
            ).unwrap();
            let bounds = measured.root().abs_bounding_box();
            eprintln!(
                "Chinese layout font={font_size} measured=({}, {}, {}, {})",
                bounds.x(),
                bounds.y(),
                bounds.right(),
                bounds.bottom()
            );
            let layout = render_text(content.clone()).unwrap();
            assert_eq!(layout.content(), &content);
            let image = decode(layout.png(), layout.width(), layout.height()).unwrap();
            assert!(image.pixels().any(|p| p[3] != 0));
            let (shift_x, shift_y) = if bounds.x() < 0. || bounds.y() < 0. {
                (
                    (4. - f64::from(bounds.x())).max(0.).ceil() as u32,
                    (4. - f64::from(bounds.y())).max(0.).ceil() as u32,
                )
            } else {
                (0, 0)
            };
            eprintln!(
                "Chinese layout font={font_size} shift=({shift_x},{shift_y}) canvas={}x{} style={}",
                image.width(),
                image.height(),
                layout.renderer().style_version
            );
            // An independent, deliberately oversized constant-margin canvas
            // catches any glyph cropped from the measured final canvas.
            assert!(shift_x <= 256 && shift_y <= 256);
            let (rw, rh) = (image.width() + 512, image.height() + 512);
            let reference_tree = usvg::Tree::from_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{rw}\" height=\"{rh}\"><g transform=\"translate(256 256)\">{text}</g></svg>"), &opts).unwrap();
            let reference = decode(&rasterize(&reference_tree, rw, rh).unwrap(), rw, rh).unwrap();
            assert_eq!(
                image.pixels().filter(|p| p[3] != 0).count(),
                reference.pixels().filter(|p| p[3] != 0).count(),
                "font={font_size} missing glyph ink"
            );
            for (x, y, pixel) in image.enumerate_pixels() {
                assert_eq!(
                    pixel,
                    reference.get_pixel(x + 256 - shift_x, y + 256 - shift_y),
                    "font={font_size} glyph pixels changed at ({x},{y})"
                );
            }
            // Previously rejected bounds get four full transparent pixels; old
            // successful layouts retain their original glyph position/bytes.
            let margin = if bounds.x() < 0. || bounds.y() < 0. {
                4
            } else {
                1
            };
            for (x, y, pixel) in image.enumerate_pixels() {
                if x < margin
                    || y < margin
                    || x >= image.width() - margin
                    || y >= image.height() - margin
                {
                    assert_eq!(
                        pixel[3], 0,
                        "font={font_size} nontransparent canvas edge ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn already_valid_text_keeps_legacy_png_bytes() {
        for (literal, font_size) in [
            ("旧文本", 40.),
            ("新内容", 40.),
            ("中文标注\n<svg>&公式 x²", 24.),
        ] {
            let content = text_content(literal, font_size);
            let (text, baseline) = literal_text_svg(&content).unwrap();
            let opts = options(true).unwrap();
            let measured = usvg::Tree::from_str(
                &format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"6000\">{text}</svg>"),
                &opts,
            ).unwrap();
            let b = measured.root().abs_bounding_box();
            assert!(
                b.x() >= 0. && b.y() >= 0.,
                "old successful fixture unexpectedly changed"
            );
            let (width, height) = dimensions(
                f64::from(b.right()) + 4.,
                (f64::from(b.bottom()) + 4.).max(baseline + content.font_size * 0.3 + 4.),
            )
            .unwrap();
            let tree = usvg::Tree::from_str(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">{text}</svg>"), &opts).unwrap();
            let legacy = rasterize(&tree, width, height).unwrap();
            let actual = render_text(content).unwrap();
            assert_eq!(actual.png(), legacy);
            assert_eq!(actual.renderer().style_version, 1);
        }
    }

    #[test]
    fn text_canvas_translation_does_not_relax_input_or_pixel_limits() {
        for (text, size) in [
            ("中".repeat(501), 24.),
            ("中".repeat(1), 128.01),
            ("中".repeat(1), 7.99),
            ("x".repeat(4097), 24.),
            ("中".repeat(500), 128.),
            (("中文\n").repeat(100), 128.),
        ] {
            assert!(render_text(text_content(&text, size)).is_err());
        }
    }

    #[test]
    fn rectangle_is_transparent_inside_and_preserves_half_opacity() {
        let annotation = TimedVideoAnnotation {
            id: uuid::Uuid::new_v4().to_string(),
            interval: VideoRange {
                start_ticks: 0,
                end_ticks: 1_000_000,
            },
            primitive: VideoAnnotationPrimitive::Rect {
                bounds: VideoPixelRect {
                    x: 10.,
                    y: 20.,
                    width: 40.,
                    height: 30.,
                },
                style: VideoRectStyle {
                    color: "#ff0000".into(),
                    stroke_width: 4.,
                    opacity: 0.5,
                },
            },
            origin: None,
        };
        let raster = render(&annotation, |_| Err("unexpected text".into())).unwrap();
        let image = decode(&raster.png, 40, 30).unwrap();
        assert_eq!(image.get_pixel(20, 15).0, [0, 0, 0, 0]);
        assert_eq!(image.get_pixel(1, 15).0, [255, 0, 0, 128]);
        assert_eq!(
            raster.bounds,
            VideoPixelRect {
                x: 10.,
                y: 20.,
                width: 40.,
                height: 30.
            }
        );
        assert_eq!(
            render(&annotation, |_| Err("unexpected text".into()))
                .unwrap()
                .reference,
            raster.reference
        );
    }
    #[test]
    fn chinese_multiline_text_is_measured_and_markup_stays_literal() {
        let layout = render_text(VideoTextContent {
            version: 1,
            text: "中文标注\n<svg>&公式 x²".into(),
            color: "#1357b9".into(),
            font_size: 24.,
        })
        .unwrap();
        let image = decode(layout.png(), layout.width(), layout.height()).unwrap();
        assert!(image.pixels().any(|p| p[3] > 200));
        assert!(layout.height() > 60 && layout.width() > 120);
        assert!(image
            .enumerate_pixels()
            .any(|(_, y, p)| y > 35 && p[3] > 200));
        assert_eq!(layout.content().text, "中文标注\n<svg>&公式 x²");
    }
    #[test]
    fn mixed_missing_glyph_is_rejected_instead_of_persisting_a_placeholder() {
        assert!(render_text(VideoTextContent {
            version: 1,
            text: "中文\u{10ffff}".into(),
            color: "#123456".into(),
            font_size: 24.
        })
        .is_err());
    }
    #[test]
    fn saturated_inside_stroke_fills_small_rectangle() {
        let annotation = TimedVideoAnnotation {
            id: uuid::Uuid::new_v4().to_string(),
            interval: VideoRange {
                start_ticks: 0,
                end_ticks: 1_000_000,
            },
            primitive: VideoAnnotationPrimitive::Rect {
                bounds: VideoPixelRect {
                    x: 0.,
                    y: 0.,
                    width: 4.,
                    height: 8.,
                },
                style: VideoRectStyle {
                    color: "#ff0000".into(),
                    stroke_width: 4.,
                    opacity: 0.5,
                },
            },
            origin: None,
        };
        let raster = render(&annotation, |_| Err("unexpected text".into())).unwrap();
        let image = decode(&raster.png, 4, 8).unwrap();
        assert!(image.pixels().all(|p| p.0 == [255, 0, 0, 128]));
    }
    #[test]
    fn fractional_bounds_fill_the_raster_without_aspect_letterboxing() {
        let annotation = TimedVideoAnnotation {
            id: uuid::Uuid::new_v4().to_string(),
            interval: VideoRange {
                start_ticks: 0,
                end_ticks: 1_000_000,
            },
            primitive: VideoAnnotationPrimitive::Rect {
                bounds: VideoPixelRect {
                    x: 1.25,
                    y: 2.75,
                    width: 1.01,
                    height: 30.99,
                },
                style: VideoRectStyle {
                    color: "#ff0000".into(),
                    stroke_width: 0.3,
                    opacity: 1.,
                },
            },
            origin: None,
        };
        let raster = render(&annotation, |_| Err("unexpected text".into())).unwrap();
        assert_eq!((raster.reference.width, raster.reference.height), (2, 31));
        let image = decode(&raster.png, 2, 31).unwrap();
        for x in 0..2 {
            let pixel = image.get_pixel(x, 15);
            assert_eq!(&pixel.0[..3], &[255, 0, 0]);
            assert!(
                pixel[3] >= 140,
                "fractional edge must use the full canvas: {pixel:?}"
            );
        }
    }
}
