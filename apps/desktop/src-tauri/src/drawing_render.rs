// SPDX-License-Identifier: MPL-2.0
//! Render our bounded drawing document, never imported SVG markup. Drawing
//! documents remain readable after the authoring plugin is removed or disabled.

use crate::raster_compositor::{blend_overlay, blend_rgba};
use image::DynamicImage;
#[cfg(test)]
use image::Rgba;
use mewu_core::{Drawing, DrawingKind, DrawingPoint, Region};
use resvg::{tiny_skia, usvg};
use std::{
    fmt::Write as _,
    fs,
    io::Read,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

#[path = "drawing_mosaic.rs"]
mod mosaic;
pub use mosaic::mosaic_grid;

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_DRAWINGS_BYTES: usize = 4 * 1024 * 1024;
const MAX_FONT_BYTES: usize = 48 * 1024 * 1024;
#[cfg(not(target_os = "macos"))]
const FONT_FAMILY: &str =
    "Segoe UI, Microsoft YaHei, PingFang SC, Noto Sans CJK SC, DejaVu Sans, sans-serif";
#[cfg(target_os = "macos")]
const FONT_FAMILY: &str = "Helvetica, PingFang SC, Arial Unicode MS, sans-serif";
static FONTS: OnceLock<Result<Arc<usvg::fontdb::Database>, String>> = OnceLock::new();
static BOLD_FONTS: OnceLock<Result<Arc<usvg::fontdb::Database>, String>> = OnceLock::new();
static BOLD_FONT_IDENTITY: OnceLock<Result<String, String>> = OnceLock::new();

#[cfg(test)]
#[path = "drawing_rich_pixels_test.rs"]
mod rich_pixels;

/// Export and model attachments must share this path. Only the requested
/// region is allocated; geometry stays in original screenshot pixels.
pub fn crop_region(image: &DynamicImage, region: &Region) -> Result<DynamicImage, String> {
    crop_region_with_underlay(image, region, None)
}

/// The immutable translation raster sits below every annotation. Mosaics still
/// sample the original source grid, so on-screen preview and exported pixels
/// agree and a translated word cannot appear above a later redaction.
pub fn crop_region_with_underlay(
    image: &DynamicImage,
    region: &Region,
    underlay: Option<&image::RgbaImage>,
) -> Result<DynamicImage, String> {
    crop_region_with_rasters(image, region, underlay, &Default::default())
}

pub fn crop_region_with_rasters(
    image: &DynamicImage,
    region: &Region,
    underlay: Option<&image::RgbaImage>,
    rasters: &crate::rich_annotation_host::RichRasters,
) -> Result<DynamicImage, String> {
    crop_region_with_layouts(image, region, underlay, |reference| {
        rasters
            .get(&reference.layout_id)
            .filter(|r| &r.reference == reference)
            .map(|r| r.pixels.clone())
            .ok_or_else(|| "富图层图片不可用".into())
    })
}

/// Decode and release one immutable layout at a time. Total document history
/// must not become an export-only limit or a permanent RGBA cache.
pub fn crop_region_with_layouts<F>(
    image: &DynamicImage,
    region: &Region,
    underlay: Option<&image::RgbaImage>,
    mut load: F,
) -> Result<DynamicImage, String>
where
    F: FnMut(&mewu_core::RichDrawingRef) -> Result<image::RgbaImage, String>,
{
    let (x, y, width, height) = crop_bounds(image, region)?;
    if region.drawings.is_empty() && underlay.is_none() {
        return Ok(image.crop_imm(x, y, width, height));
    }
    validate_drawings(&region.drawings, image.width(), image.height())?;
    let mut bitmap = image.crop_imm(x, y, width, height).into_rgba8();
    let protected = region
        .drawings
        .iter()
        .any(|d| matches!(d.kind, DrawingKind::Highlighter | DrawingKind::Mark));
    // Derive protection from committed raster content, excluding translated
    // text and editable ink. Borrow the crop: no second full RGBA allocation.
    let foreground = if protected {
        paint_raster_base(image, region, &mut bitmap, x, y, &mut load)?;
        Some(crate::highlight_foreground::foreground(
            &bitmap, 0, 0, width, height,
        )?)
    } else {
        None
    };
    if let Some(layer) = underlay {
        if layer.dimensions() != (width, height) {
            return Err("译文图层尺寸与选区不符".into());
        }
        if protected {
            use image::GenericImageView;
            // Only translated protected exports need this second raster pass.
            // Reuse crop storage rather than clone a potentially huge capture.
            for (px, py, target) in bitmap.enumerate_pixels_mut() {
                *target = image.get_pixel(x + px, y + py);
            }
        }
        for (target, source) in bitmap.pixels_mut().zip(layer.pixels()) {
            blend_rgba(target, source);
        }
    }
    if !protected || underlay.is_some() {
        paint_raster_base(image, region, &mut bitmap, x, y, &mut load)?;
    }
    let drawings: Vec<_> = region
        .drawings
        .iter()
        .filter(|d| !d.rich.as_ref().is_some_and(|r| r.kind.is_raster_edit()))
        .cloned()
        .collect();
    if region
        .drawings
        .iter()
        .all(|drawing| drawing.kind == DrawingKind::Mosaic)
    {
        return Ok(DynamicImage::ImageRgba8(bitmap));
    }
    let mut segment = 0;
    for (index, drawing) in drawings.iter().enumerate() {
        if drawing.kind != DrawingKind::Rich {
            continue;
        }
        paint_vectors(
            &drawings[segment..index],
            image.width(),
            image.height(),
            &mut bitmap,
            x,
            y,
            foreground.as_ref(),
        )?;
        let reference = drawing.rich.as_ref().ok_or("富图层引用缺失")?;
        let raster = load(reference)?;
        if raster.dimensions() != (reference.width, reference.height) {
            return Err("富图层尺寸已变更".into());
        }
        let p = &drawing.points;
        crate::raster_compositor::paint_raster(
            &mut bitmap,
            &raster,
            p[0].x - f64::from(x),
            p[0].y - f64::from(y),
            p[1].x - p[0].x,
            p[1].y - p[0].y,
        )?;
        segment = index + 1;
    }
    paint_vectors(
        &drawings[segment..],
        image.width(),
        image.height(),
        &mut bitmap,
        x,
        y,
        foreground.as_ref(),
    )?;
    Ok(DynamicImage::ImageRgba8(bitmap))
}

fn paint_raster_base<F>(
    image: &DynamicImage,
    region: &Region,
    bitmap: &mut image::RgbaImage,
    x: u32,
    y: u32,
    load: &mut F,
) -> Result<(), String>
where
    F: FnMut(&mewu_core::RichDrawingRef) -> Result<image::RgbaImage, String>,
{
    mosaic::paint_mosaics(image, &region.drawings, bitmap, x, y)?;
    for drawing in region
        .drawings
        .iter()
        .filter(|d| d.rich.as_ref().is_some_and(|r| r.kind.is_raster_edit()))
    {
        let reference = drawing.rich.as_ref().ok_or("修补图层引用缺失")?;
        let raster = load(reference)?;
        if raster.dimensions() != (reference.width, reference.height) {
            return Err("修补图层尺寸已变更".into());
        }
        let p = &drawing.points;
        crate::raster_compositor::paint_raster(
            bitmap,
            &raster,
            p[0].x - f64::from(x),
            p[0].y - f64::from(y),
            p[1].x - p[0].x,
            p[1].y - p[0].y,
        )?;
    }
    Ok(())
}

fn paint_vectors(
    drawings: &[Drawing],
    source_width: u32,
    source_height: u32,
    bitmap: &mut image::RgbaImage,
    x: u32,
    y: u32,
    foreground: Option<&crate::highlight_foreground::Foreground>,
) -> Result<(), String> {
    // Highlight foreground is estimated from the frozen source, never from a
    // previous tint. Split vector segments to preserve document paint order.
    let Some(foreground) = foreground else {
        return paint_plain_vectors(drawings, source_width, source_height, bitmap, x, y);
    };
    let mut start = 0;
    for (index, drawing) in drawings.iter().enumerate() {
        if !matches!(drawing.kind, DrawingKind::Highlighter | DrawingKind::Mark) {
            continue;
        }
        paint_plain_vectors(
            &drawings[start..index],
            source_width,
            source_height,
            bitmap,
            x,
            y,
        )?;
        if let Some(overlay) = vector_overlay(
            std::slice::from_ref(drawing),
            source_width,
            source_height,
            bitmap.width(),
            bitmap.height(),
            x,
            y,
        )? {
            let color = [
                u8::from_str_radix(&drawing.color[1..3], 16),
                u8::from_str_radix(&drawing.color[3..5], 16),
                u8::from_str_radix(&drawing.color[5..7], 16),
            ];
            let color = color.map(|v| v.expect("validated RGB drawing color"));
            for (pixel, coverage) in overlay.pixels().iter().enumerate() {
                if coverage.alpha() == 0 {
                    continue;
                }
                let px = pixel as u32 % bitmap.width();
                let py = pixel as u32 / bitmap.width();
                if let Some(rgb) = foreground.tint(px, py, color) {
                    blend_rgba(
                        bitmap.get_pixel_mut(px, py),
                        &image::Rgba([rgb[0], rgb[1], rgb[2], coverage.alpha()]),
                    );
                }
            }
        }
        start = index + 1;
    }
    paint_plain_vectors(
        &drawings[start..],
        source_width,
        source_height,
        bitmap,
        x,
        y,
    )
}

fn paint_plain_vectors(
    drawings: &[Drawing],
    source_width: u32,
    source_height: u32,
    bitmap: &mut image::RgbaImage,
    x: u32,
    y: u32,
) -> Result<(), String> {
    if let Some(overlay) = vector_overlay(
        drawings,
        source_width,
        source_height,
        bitmap.width(),
        bitmap.height(),
        x,
        y,
    )? {
        blend_overlay(bitmap, &overlay);
    }
    Ok(())
}

fn vector_overlay(
    drawings: &[Drawing],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> Result<Option<tiny_skia::Pixmap>, String> {
    if drawings.iter().all(|d| d.kind == DrawingKind::Mosaic) {
        return Ok(None);
    }
    let xml = drawing_svg(drawings, source_width, source_height);
    let mut options = usvg::Options::default();
    // Defense in depth: even future serialization mistakes cannot dereference
    // files, embedded images, or network resources through usvg's defaults.
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    if drawings
        .iter()
        .any(|drawing| matches!(drawing.kind, DrawingKind::Text | DrawingKind::Number))
    {
        let bold = drawings.iter().any(|drawing| {
            drawing.kind == DrawingKind::Text && drawing.text_style.is_some_and(|style| style.bold)
        });
        options.fontdb = text_fonts(bold)?;
        if bold {
            prefer_matching_text_fallback(&mut options);
        }
        #[cfg(not(target_os = "macos"))]
        {
            options.font_family = "Segoe UI".into();
        }
        #[cfg(target_os = "macos")]
        {
            options.font_family = macos_default_family(&options.fontdb)?;
        }
    }
    let tree = usvg::Tree::from_str(&xml, &options).map_err(|_| "无法排版绘制内容")?;
    let mut overlay = tiny_skia::Pixmap::new(width, height).ok_or("选区过大，无法合成标注")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_translate(-(x as f32), -(y as f32)),
        &mut overlay.as_mut(),
    );
    // tiny-skia stores premultiplied RGBA; image::Pixel::blend expects straight
    // alpha. Demultiply before compositing to avoid dark anti-aliased edges.
    Ok(Some(overlay))
}

pub(crate) fn crop_bounds(
    image: &DynamicImage,
    region: &Region,
) -> Result<(u32, u32, u32, u32), String> {
    if [region.x, region.y, region.width, region.height]
        .iter()
        .any(|v| !v.is_finite())
        || region.x < 0.
        || region.y < 0.
        || region.width < 1.
        || region.height < 1.
        || region.x + region.width > image.width() as f64 + 0.001
        || region.y + region.height > image.height() as f64 + 0.001
    {
        return Err("选区超出图片范围".into());
    }
    let x = region.x.round() as u32;
    let y = region.y.round() as u32;
    let width = region.width.round() as u32;
    let height = region.height.round() as u32;
    if x.checked_add(width).is_none_or(|v| v > image.width())
        || y.checked_add(height).is_none_or(|v| v > image.height())
        || width as u64 * height as u64 > MAX_PIXELS
    {
        return Err("选区超出图片范围或超过 3200 万像素".into());
    }
    Ok((x, y, width, height))
}

fn validate_drawings(drawings: &[Drawing], width: u32, height: u32) -> Result<(), String> {
    if drawings.len() > 256
        || serde_json::to_vec(drawings)
            .map_err(|_| "绘制格式无效")?
            .len()
            > MAX_DRAWINGS_BYTES
    {
        return Err("绘制内容超过处理上限".into());
    }
    for drawing in drawings {
        if drawing.kind != DrawingKind::Text && drawing.text_style.is_some() {
            return Err("只有文字对象可以设置文字样式".into());
        }
        if drawing.kind != DrawingKind::Rich && drawing.rich.is_some() {
            return Err("普通图元不能引用富图层".into());
        }
        let color = drawing.color.as_bytes();
        if color.len() != 7
            || color[0] != b'#'
            || !color[1..].iter().all(u8::is_ascii_hexdigit)
            || !drawing.stroke_width.is_finite()
            || !(0.5..=64.).contains(&drawing.stroke_width)
        {
            return Err("绘制颜色或线宽无效".into());
        }
        let floating = drawing
            .rich
            .as_ref()
            .is_some_and(|r| r.kind == mewu_core::RichKind::Extracted || r.parent_id.is_some());
        let count = drawing.points.len();
        if !(match drawing.kind {
            DrawingKind::Pen | DrawingKind::Highlighter => (1..=4096).contains(&count),
            DrawingKind::Text | DrawingKind::Number => count == 1,
            _ => count == 2,
        }) || drawing.points.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < if floating { -(width as f64) * 16. } else { 0. }
                || p.y < if floating { -(height as f64) * 16. } else { 0. }
                || p.x
                    > if floating {
                        width as f64 * 16.
                    } else {
                        width as f64
                    }
                || p.y
                    > if floating {
                        height as f64 * 16.
                    } else {
                        height as f64
                    }
        }) {
            return Err("绘制坐标无效".into());
        }
        if drawing.kind == DrawingKind::Text {
            let text = drawing.text.as_deref().ok_or("绘制文字为空")?;
            if text.trim().is_empty()
                || text.chars().count() > 2000
                || text
                    .chars()
                    .any(|c| (c.is_control() && c != '\n') || matches!(c, '\u{fffe}' | '\u{ffff}'))
                || drawing
                    .font_size
                    .is_none_or(|v| !v.is_finite() || !(6. ..=256.).contains(&v))
            {
                return Err("绘制文字或字号无效".into());
            }
        } else if drawing.kind == DrawingKind::Number {
            let text = drawing.text.as_deref().unwrap_or_default();
            if text.is_empty()
                || text.len() > 4
                || text.starts_with('0')
                || !text.bytes().all(|c| c.is_ascii_digit())
                || drawing
                    .font_size
                    .is_none_or(|size| !size.is_finite() || !(22. ..=64.).contains(&size))
            {
                return Err("绘制序号或直径无效".into());
            }
        } else if drawing.text.is_some() || drawing.font_size.is_some() {
            return Err("图形不能携带文字字段".into());
        }
        if drawing.kind == DrawingKind::Mosaic
            && (!(6. ..=40.).contains(&drawing.stroke_width) || drawing.stroke_width.fract() != 0.)
        {
            return Err("马赛克块大小需为 6 至 40 的整数".into());
        }
        if drawing.kind == DrawingKind::Rich {
            let reference = drawing.rich.as_ref().ok_or("富图层引用缺失")?;
            let w = drawing.points[1].x - drawing.points[0].x;
            let h = drawing.points[1].y - drawing.points[0].y;
            let a = w * f64::from(reference.height);
            let b = h * f64::from(reference.width);
            if (reference.kind.is_raster_edit() == drawing.origin.is_some())
                || drawing.color != mewu_core::RICH_DRAWING_COLOR
                || drawing.stroke_width != mewu_core::RICH_DRAWING_STROKE_WIDTH
                || reference.width == 0
                || reference.height == 0
                || reference.width > 6000
                || reference.height > 6000
                || w <= 0.
                || h <= 0.
                || (a - b).abs() > a.abs().max(b.abs()) * 1e-6
                || (reference.kind == mewu_core::RichKind::Repair
                    && reference.parent_id.is_none()
                    && ((w - f64::from(reference.width)).abs() > 1e-6
                        || (h - f64::from(reference.height)).abs() > 1e-6))
            {
                return Err("富图层布局或样式无效".into());
            }
        }
        match drawing.kind {
            DrawingKind::Line | DrawingKind::Arrow if drawing.points[0] == drawing.points[1] => {
                return Err("直线或箭头不能只有一个位置".into());
            }
            DrawingKind::Rect
            | DrawingKind::Ellipse
            | DrawingKind::Mosaic
            | DrawingKind::Rich
            | DrawingKind::Mark
                if drawing.points[0].x == drawing.points[1].x
                    || drawing.points[0].y == drawing.points[1].y =>
            {
                return Err("图形宽高必须大于零".into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Native video reuses only the existing bounded, closed vector geometry.
/// This transient Drawing is never registered as screenshot data.
pub(crate) fn video_vector_svg(
    drawing: &Drawing,
    width: u32,
    height: u32,
) -> Result<String, String> {
    if !matches!(
        drawing.kind,
        DrawingKind::Pen
            | DrawingKind::Line
            | DrawingKind::Arrow
            | DrawingKind::Rect
            | DrawingKind::Ellipse
            | DrawingKind::Number
    ) || drawing.origin.is_some()
        || drawing.rich.is_some()
        || width == 0
        || height == 0
    {
        return Err("视频绘制类型无效".into());
    }
    validate_drawings(std::slice::from_ref(drawing), width, height)?;
    Ok(drawing_svg(std::slice::from_ref(drawing), width, height))
}

fn drawing_svg(drawings: &[Drawing], width: u32, height: u32) -> String {
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">");
    for drawing in drawings {
        let first = drawing.points[0];
        let color = &drawing.color;
        let stroke = drawing.stroke_width;
        if drawing.kind == DrawingKind::Highlighter {
            svg.push_str("<g opacity=\"0.35\">");
        }
        match drawing.kind {
            DrawingKind::Pen | DrawingKind::Highlighter if drawing.points.len() == 1 => {
                let _ = write!(
                    svg,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\"/>",
                    first.x,
                    first.y,
                    stroke / 2.
                );
            }
            DrawingKind::Pen | DrawingKind::Highlighter => {
                let _ = write!(svg, "<polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" stroke-linejoin=\"round\" points=\"");
                for point in &drawing.points {
                    let _ = write!(svg, "{},{} ", point.x, point.y);
                }
                svg.push_str("\"/>");
            }
            DrawingKind::Line => {
                let end = drawing.points[1];
                let _ = write!(svg, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"round\"/>", first.x, first.y, end.x, end.y);
            }
            DrawingKind::Mark => {
                let (border, fill) = mark_paths(first, drawing.points[1], stroke);
                let _ = write!(svg, "<path d=\"{border}\" fill=\"{color}\" fill-rule=\"evenodd\" fill-opacity=\"{}\"/><path d=\"{fill}\" fill=\"{color}\" fill-opacity=\"{}\"/>", 190. / 255., 84. / 255.);
            }
            DrawingKind::Arrow => {
                let end = drawing.points[1];
                let (left, right) = arrow_wings(first, end);
                let _ = write!(svg, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"round\"/><polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>", first.x, first.y, end.x, end.y, left.x, left.y, end.x, end.y, right.x, right.y);
            }
            DrawingKind::Rect | DrawingKind::Ellipse => {
                let end = drawing.points[1];
                let x = first.x.min(end.x);
                let y = first.y.min(end.y);
                let w = (end.x - first.x).abs();
                let h = (end.y - first.y).abs();
                if drawing.kind == DrawingKind::Rect {
                    let _ = write!(svg, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linejoin=\"round\"/>");
                } else {
                    let _ = write!(svg, "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke}\"/>", x+w/2., y+h/2., w/2., h/2.);
                }
            }
            DrawingKind::Text => {
                let size = drawing.font_size.unwrap();
                if let Some(style) = drawing
                    .text_style
                    .filter(|style| *style != Default::default())
                {
                    for (line, text) in drawing.text.as_deref().unwrap().split('\n').enumerate() {
                        svg.push_str(&styled_text_line_svg(
                            text,
                            color,
                            size,
                            first.x,
                            first.y + size + line as f64 * size * 1.25,
                            style,
                        ));
                    }
                    continue;
                }
                let _ = write!(svg, "<text fill=\"{color}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size}\" xml:space=\"preserve\">");
                for (line, text) in drawing.text.as_deref().unwrap().split('\n').enumerate() {
                    let _ = write!(
                        svg,
                        "<tspan x=\"{}\" y=\"{}\">{}</tspan>",
                        first.x,
                        first.y + size + line as f64 * size * 1.25,
                        escape_text(text)
                    );
                }
                svg.push_str("</text>");
            }
            DrawingKind::Number => {
                let diameter = drawing.font_size.unwrap();
                let text = drawing.text.as_deref().unwrap();
                let size = (diameter * 0.55).min(diameter * 0.78 / (text.len() as f64 * 0.6));
                let cx = first.x + diameter / 2.;
                let cy = first.y + diameter / 2.;
                let foreground = number_foreground(color);
                let _ = write!(svg, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{}\" fill=\"{color}\"/><text x=\"{cx}\" y=\"{}\" fill=\"{foreground}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size}\" font-weight=\"600\" text-anchor=\"middle\">{text}</text>", diameter/2., cy + size * 0.35);
            }
            DrawingKind::Mosaic | DrawingKind::Rich => {} // Raster layers are composed outside SVG.
        }
        if drawing.kind == DrawingKind::Highlighter {
            svg.push_str("</g>");
        }
    }
    svg.push_str("</svg>");
    svg
}

fn mark_paths(a: DrawingPoint, b: DrawingPoint, border: f64) -> (String, String) {
    fn path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
        if w <= 0. || h <= 0. {
            return String::new();
        }
        format!("M{} {y}h{}a{r} {r} 0 0 1 {r} {r}v{}a{r} {r} 0 0 1 {} {r}h{}a{r} {r} 0 0 1 {} {}v{}a{r} {r} 0 0 1 {r} {}z", x+r, w-2.*r, h-2.*r, -r, 2.*r-w, -r, -r, 2.*r-h, -r)
    }
    let (x, y, w, h) = (
        a.x.min(b.x),
        a.y.min(b.y),
        (a.x - b.x).abs(),
        (a.y - b.y).abs(),
    );
    let r = 3_f64.min(w / 2.).min(h / 2.);
    let t = border.min(w / 2.).min(h / 2.);
    let outer = path(x, y, w, h, r);
    let fill = path(x + t, y + t, w - 2. * t, h - 2. * t, (r - t).max(0.));
    (outer + &fill, fill)
}

fn number_foreground(color: &str) -> &'static str {
    // Same WCAG sRGB transfer and contrast comparison as the live SVG preview.
    let linear = |offset| {
        let s = u8::from_str_radix(&color[offset..offset + 2], 16).unwrap_or(0) as f64 / 255.;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(1) + 0.7152 * linear(3) + 0.0722 * linear(5);
    if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
        "#000000"
    } else {
        "#FFFFFF"
    }
}

fn arrow_wings(start: DrawingPoint, end: DrawingPoint) -> (DrawingPoint, DrawingPoint) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let distance = dx.hypot(dy);
    if distance < f64::EPSILON {
        return (end, end);
    }
    // Original Mewu uses open wings at ±30°, sized from the shaft length.
    let length = (distance * 0.25).min(24.);
    let back = length * 3_f64.sqrt() / 2.;
    let wing = length / 2.;
    let ux = dx / distance;
    let uy = dy / distance;
    let bx = end.x - ux * back;
    let by = end.y - uy * back;
    (
        DrawingPoint {
            x: bx - uy * wing,
            y: by + ux * wing,
        },
        DrawingPoint {
            x: bx + uy * wing,
            y: by - ux * wing,
        },
    )
}

fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Match drawing-geometry.ts. Each baseline is the transform origin so multiple
/// lines keep their x alignment; normal faces avoid double-italic Latin glyphs.
const DRAWING_TEXT_SKEW_DEGREES: i32 = -12;
pub(crate) fn styled_text_line_svg(
    text: &str,
    color: &str,
    size: f64,
    x: f64,
    baseline: f64,
    style: mewu_core::DrawingTextStyle,
) -> String {
    let decoration = match (style.underline, style.strikethrough) {
        (true, true) => "underline line-through",
        (true, false) => "underline",
        (false, true) => "line-through",
        (false, false) => "none",
    };
    let transform = if style.italic {
        format!(" transform=\"translate(0 {baseline}) skewX({DRAWING_TEXT_SKEW_DEGREES}) translate(0 {})\"", -baseline)
    } else {
        String::new()
    };
    format!("<text x=\"{x}\" y=\"{baseline}\" fill=\"{color}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size}\" font-weight=\"{}\" font-style=\"normal\" text-decoration=\"{decoration}\"{transform} xml:space=\"preserve\">{}</text>", if style.bold { 700 } else { 400 }, escape_text(text))
}

pub(crate) fn fonts() -> Result<Arc<usvg::fontdb::Database>, String> {
    FONTS
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            let mut bytes_left = MAX_FONT_BYTES;
            let mut loaded = 0;
            #[cfg(target_os = "macos")]
            let paths = crate::font_macos::font_paths()?;
            #[cfg(not(target_os = "macos"))]
            let paths = font_paths();
            for path in paths {
                if loaded == 3 {
                    break;
                }
                let Ok(file) = fs::File::open(path) else {
                    continue;
                };
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                if !metadata.is_file() || metadata.len() > bytes_left.min(32 * 1024 * 1024) as u64 {
                    continue;
                }
                let mut bytes = Vec::new();
                if file
                    .take(bytes_left.min(32 * 1024 * 1024) as u64 + 1)
                    .read_to_end(&mut bytes)
                    .is_err()
                    || bytes.len() > bytes_left.min(32 * 1024 * 1024)
                {
                    continue;
                }
                bytes_left -= bytes.len();
                database.load_font_data(bytes);
                loaded += 1;
            }
            if database.faces().next().is_none() {
                return Err("未找到可用于绘制文字的系统字体".into());
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                database.set_sans_serif_family("Segoe UI");
            }
            #[cfg(target_os = "linux")]
            {
                database.set_sans_serif_family(linux_default_family(&database)?);
            }
            #[cfg(target_os = "macos")]
            {
                database.set_sans_serif_family(macos_default_family(&database)?);
            }
            Ok(Arc::new(database))
        })
        .clone()
}

/// Ordinary capture/export keeps the original three-font cache. Only a bold
/// text request clones metadata (font bytes stay Arc-shared) and loads faces.
pub(crate) fn text_fonts(bold: bool) -> Result<Arc<usvg::fontdb::Database>, String> {
    if !bold {
        return fonts();
    }
    BOLD_FONTS
        .get_or_init(|| {
            let base = fonts()?;
            let mut unique = std::collections::HashSet::new();
            let mut used = 0usize;
            for face in base.faces() {
                base.with_face_data(face.id, |bytes, _| {
                    if unique.insert((bytes.as_ptr() as usize, bytes.len())) {
                        used += bytes.len();
                    }
                });
            }
            let mut bytes_left = MAX_FONT_BYTES.saturating_sub(used);
            let mut database = (*base).clone();
            #[cfg(target_os = "macos")]
            let paths = crate::font_macos::bold_font_paths();
            #[cfg(not(target_os = "macos"))]
            let paths = bold_font_paths();
            let mut seen = std::collections::HashSet::new();
            for path in paths.into_iter().take(3) {
                let Ok(path) = path.canonicalize() else {
                    continue;
                };
                if !seen.insert(path.clone()) {
                    continue;
                }
                let Ok(file) = fs::File::open(path) else {
                    continue;
                };
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                let limit = bytes_left.min(32 * 1024 * 1024);
                if !metadata.is_file() || metadata.len() > limit as u64 {
                    continue;
                }
                let mut bytes = Vec::new();
                if file.take(limit as u64 + 1).read_to_end(&mut bytes).is_err()
                    || bytes.len() > limit
                {
                    continue;
                }
                bytes_left -= bytes.len();
                database.load_font_data(bytes);
            }
            Ok(Arc::new(database))
        })
        .clone()
}

/// usvg 0.48's default accepts the first covering face when any style trait
/// matches. With Latin-first families that can choose regular CJK before its
/// loaded bold face. Prefer all requested traits before retaining that fallback.
pub(crate) fn prefer_matching_text_fallback(options: &mut usvg::Options<'_>) {
    let fallback = usvg::FontResolver::default_fallback_selector();
    options.font_resolver.select_fallback = Box::new(move |character, excluded, database| {
        let base = database.face(*excluded.first()?)?;
        let (style, weight, stretch) = (base.style, base.weight, base.stretch);
        let mut preferred_excluded = excluded.to_vec();
        for face in database.faces() {
            if (face.style != style || face.weight != weight || face.stretch != stretch)
                && !preferred_excluded.contains(&face.id)
            {
                preferred_excluded.push(face.id);
            }
        }
        // Keep the real base ID first. The existing resolver owns actual glyph
        // coverage detection; no private parser API or font data rewrite.
        fallback(character, &preferred_excluded, database)
            .or_else(|| fallback(character, excluded, database))
    });
}

pub(crate) fn text_font_identity(bold: bool) -> Result<String, String> {
    if !bold {
        return crate::rich_annotation_host::table_font_identity();
    }
    BOLD_FONT_IDENTITY
        .get_or_init(|| {
            use sha2::{Digest, Sha256};
            let fonts = text_fonts(true)?;
            let mut faces = Vec::new();
            for face in fonts.faces() {
                faces.push(
                    fonts
                        .with_face_data(face.id, |bytes, index| {
                            let mut hash = Sha256::new();
                            hash.update(index.to_be_bytes());
                            hash.update(bytes);
                            format!("{:x}", hash.finalize())
                        })
                        .ok_or("无法核对绘制字体")?,
                );
            }
            faces.sort();
            faces.dedup();
            let mut hash = Sha256::new();
            hash.update(b"mewu.drawing-bold-fonts.v1\0");
            for face in faces {
                hash.update(face.as_bytes());
            }
            Ok(format!("{:x}", hash.finalize()))
        })
        .clone()
}

#[cfg(target_os = "macos")]
fn macos_default_family(database: &usvg::fontdb::Database) -> Result<String, String> {
    // Select from parsed, already-loaded faces. A queried file might exceed
    // the read budget or lack a valid face; never select its assumed family.
    for wanted in ["Helvetica", "Arial Unicode MS", "PingFang SC"] {
        if let Some((family, _)) = database
            .faces()
            .flat_map(|face| &face.families)
            .find(|(family, _)| family == wanted)
        {
            return Ok(family.clone());
        }
    }
    database
        .faces()
        .flat_map(|face| &face.families)
        .find(|(family, _)| !family.trim().is_empty())
        .map(|(family, _)| family.clone())
        .ok_or_else(|| "未找到可用于绘制文字的系统字体名称".into())
}

#[cfg(target_os = "linux")]
fn linux_default_family(database: &usvg::fontdb::Database) -> Result<String, String> {
    // Select a family from already parsed and bounded font data. Installing
    // Noto CJK provides Chinese glyphs; a path alone is not coverage evidence.
    for wanted in ["DejaVu Sans", "Noto Sans CJK SC"] {
        if let Some((family, _)) = database
            .faces()
            .flat_map(|face| &face.families)
            .find(|(family, _)| family == wanted)
        {
            return Ok(family.clone());
        }
    }
    database
        .faces()
        .flat_map(|face| &face.families)
        .find(|(family, _)| !family.trim().is_empty())
        .map(|(family, _)| family.clone())
        .ok_or_else(|| "未找到可用于绘制文字的系统字体名称".into())
}

#[cfg(not(target_os = "macos"))]
fn font_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let windows = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let fonts = windows.join("Fonts");
        return ["segoeui.ttf", "msyh.ttc", "arial.ttf"]
            .iter()
            .map(|name| fonts.join(name))
            .collect();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

#[cfg(not(target_os = "macos"))]
fn bold_font_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let windows = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        return ["segoeuib.ttf", "msyhbd.ttc", "arialbd.ttf"]
            .iter()
            .map(|name| windows.join("Fonts").join(name))
            .collect();
    }
    #[cfg(not(windows))]
    {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgba, RgbaImage};

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_loaded_font_families_shape_complete_chinese_latin_and_digits() {
        let database = fonts().unwrap();
        let family = linux_default_family(&database).unwrap();
        assert_eq!(
            database.family_name(&usvg::fontdb::Family::SansSerif),
            family
        );
        assert!(database
            .faces()
            .any(|face| face.families.iter().any(|(name, _)| name == &family)));
        let mut item = drawing(DrawingKind::Text, &[(24., 20.)]);
        let expected = "中文测试 Hello 0123456789";
        item.text = Some(expected.into());
        item.font_size = Some(48.);
        let mut options = usvg::Options::default();
        options.font_family = family;
        options.fontdb = database;
        let tree = usvg::Tree::from_str(&drawing_svg(&[item], 1000, 160), &options).unwrap();
        let glyphs: Vec<_> = tree
            .root()
            .children()
            .iter()
            .filter_map(|node| match node {
                usvg::Node::Text(text) => Some(text),
                _ => None,
            })
            .flat_map(|text| text.layouted())
            .flat_map(|span| &span.positioned_glyphs)
            .filter(|glyph| glyph.text.chars().any(|c| !c.is_whitespace()))
            .collect();
        assert!(!glyphs.is_empty());
        assert!(
            glyphs.iter().all(|glyph| glyph.id.0 != 0),
            "missing glyphs are not Chinese coverage"
        );
        let actual: String = glyphs.iter().map(|glyph| glyph.text.as_str()).collect();
        assert_eq!(
            actual
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            expected
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_fonts_shape_real_chinese_english_and_digit_glyphs() {
        let text = "中文测试 Hello 0123456789";
        let mut item = drawing(DrawingKind::Text, &[(24., 20.)]);
        item.text = Some(text.into());
        item.font_size = Some(48.);
        let database = fonts().unwrap();
        let family = macos_default_family(&database).unwrap();
        assert!(database
            .faces()
            .any(|face| face.families.iter().any(|(name, _)| name == &family)));
        assert_eq!(
            database.family_name(&usvg::fontdb::Family::SansSerif),
            family
        );
        let mut options = usvg::Options::default();
        options.font_family = family;
        options.fontdb = database;
        options.image_href_resolver = usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        };
        let tree =
            usvg::Tree::from_str(&drawing_svg(&[item.clone()], 1000, 160), &options).unwrap();
        let glyphs: Vec<_> = tree
            .root()
            .children()
            .iter()
            .filter_map(|node| match node {
                usvg::Node::Text(text) => Some(text),
                _ => None,
            })
            .flat_map(|text| text.layouted())
            .flat_map(|span| &span.positioned_glyphs)
            .filter(|glyph| glyph.text.chars().any(|c| !c.is_whitespace()))
            .collect();
        assert!(!glyphs.is_empty(), "the actual text must be shaped");
        assert!(
            glyphs.iter().all(|glyph| glyph.id.0 != 0),
            "missing-glyph boxes do not count as Chinese/Latin/digit coverage"
        );
        let shaped: String = glyphs.iter().map(|glyph| glyph.text.as_str()).collect();
        assert_eq!(
            shaped
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        );
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(1000, 160, Rgba([255, 255, 255, 255])));
        let raster = crop_region(
            &base,
            &Region {
                id: "mac-system-font-glyphs".into(),
                x: 0.,
                y: 0.,
                width: 1000.,
                height: 160.,
                drawings: vec![item],
                ..Default::default()
            },
        )
        .unwrap()
        .into_rgba8();
        assert!(raster.pixels().filter(|p| p[1] < 128).count() > 1000);
    }

    fn drawing(kind: DrawingKind, points: &[(f64, f64)]) -> Drawing {
        Drawing {
            origin: None,
            rich: None,
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            color: "#ff0000".into(),
            stroke_width: 4.,
            points: points.iter().map(|&(x, y)| DrawingPoint { x, y }).collect(),
            text: None,
            font_size: None,
            text_style: None,
        }
    }

    #[test]
    fn text_styles_svg_keeps_legacy_bytes_and_each_italic_line_baseline_aligned() {
        let mut item = drawing(DrawingKind::Text, &[(40., 20.)]);
        item.text = Some("中文 <&>\nAbc".into());
        item.font_size = Some(40.);
        let legacy = drawing_svg(&[item.clone()], 800, 240);
        assert!(legacy.contains("<tspan x=\"40\" y=\"60\">中文 &lt;&amp;&gt;</tspan>"));
        item.text_style = Some(Default::default());
        assert_eq!(drawing_svg(&[item.clone()], 800, 240), legacy);
        item.text_style = Some(mewu_core::DrawingTextStyle {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: true,
        });
        let styled = drawing_svg(&[item], 800, 240);
        assert_eq!(styled.matches("font-weight=\"700\"").count(), 2);
        assert_eq!(styled.matches("font-style=\"normal\"").count(), 2);
        assert_eq!(
            styled
                .matches("text-decoration=\"underline line-through\"")
                .count(),
            2
        );
        assert!(styled.contains("translate(0 60) skewX(-12) translate(0 -60)"));
        assert!(styled.contains("translate(0 110) skewX(-12) translate(0 -110)"));
        assert!(!styled.contains("<tspan"));
        assert!(styled.contains("中文 &lt;&amp;&gt;"));
    }

    #[test]
    fn text_styles_native_cjk_and_latin_pngs_show_every_effect_and_combination() {
        use mewu_core::DrawingTextStyle as S;
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(800, 240, Rgba([255, 255, 255, 255])));
        let mut item = drawing(DrawingKind::Text, &[(80., 30.)]);
        item.text = Some("中文测试\nAbc Mm".into());
        item.font_size = Some(48.);
        let render = |value: Drawing| {
            let output = crop_region(
                &base,
                &Region {
                    id: "synthetic-text".into(),
                    x: 0.,
                    y: 0.,
                    width: 800.,
                    height: 240.,
                    drawings: vec![value],
                    ..Default::default()
                },
            )
            .unwrap();
            let mut png = std::io::Cursor::new(Vec::new());
            output.write_to(&mut png, image::ImageFormat::Png).unwrap();
            image::load_from_memory(png.get_ref()).unwrap().into_rgba8()
        };
        let regular = render(item.clone());
        let mut outputs = vec![regular.clone()];
        for style in [
            S {
                bold: true,
                ..Default::default()
            },
            S {
                italic: true,
                ..Default::default()
            },
            S {
                underline: true,
                ..Default::default()
            },
            S {
                strikethrough: true,
                ..Default::default()
            },
            S {
                bold: true,
                italic: true,
                underline: true,
                strikethrough: true,
            },
        ] {
            item.text_style = Some(style);
            let output = render(item.clone());
            for previous in &outputs {
                assert_ne!(
                    output, *previous,
                    "native PNG effect must actually change pixels: {style:?}"
                );
            }
            if style.italic && !style.bold {
                assert!(
                    output
                        .enumerate_pixels()
                        .any(|(x, y, p)| y < 90 && *p != *regular.get_pixel(x, y)),
                    "CJK oblique must change pixels, not only Latin fallback"
                );
            }
            if style.bold {
                let ink = |image: &RgbaImage| {
                    image
                        .enumerate_pixels()
                        .filter(|(_, y, p)| *y < 90 && p[1] < 128)
                        .count()
                };
                assert!(
                    ink(&output) > ink(&regular),
                    "CJK bold requires a real loaded bold face"
                );
            }
            outputs.push(output);
        }
        assert_eq!(outputs.len(), 6);
    }

    #[test]
    fn text_styles_do_not_turn_nontext_objects_into_text() {
        let mut line = drawing(DrawingKind::Line, &[(20., 20.), (80., 80.)]);
        line.text_style = Some(mewu_core::DrawingTextStyle {
            bold: true,
            ..Default::default()
        });
        assert!(validate_drawings(&[line], 100, 100).is_err());
        let regular = fonts().unwrap();
        assert!(
            Arc::ptr_eq(&regular, &text_fonts(false).unwrap()),
            "normal text must keep the original cache"
        );
        let bold = text_fonts(true).unwrap();
        assert!(Arc::ptr_eq(&bold, &text_fonts(true).unwrap()));
        let shared = regular.faces().next().unwrap().id;
        assert_eq!(
            regular.with_face_data(shared, |bytes, _| bytes.as_ptr() as usize),
            bold.with_face_data(shared, |bytes, _| bytes.as_ptr() as usize),
            "cloned metadata must share regular font byte buffers"
        );
    }
    fn background() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255])))
    }
    fn region(drawings: Vec<Drawing>) -> Region {
        Region {
            id: "fixture".into(),
            x: 20.,
            y: 10.,
            width: 100.,
            height: 80.,
            drawings,
            ..Default::default()
        }
    }

    #[test]
    fn pixel_eraser_persisted_fragments_export_the_same_gap_and_undo_redo_pixels() {
        use mewu_core::{
            Asset, AssetKind, DrawingStrokeErase, DrawingStrokeFragment, SceneCommand, Store,
        };
        for kind in [
            DrawingKind::Pen,
            DrawingKind::Line,
            DrawingKind::Highlighter,
        ] {
            let mut store = Store::open_in_memory().unwrap();
            let scene_id = store.snapshot().active_scene_id;
            let background_id = uuid::Uuid::new_v4().to_string();
            store
                .set_background(
                    &scene_id,
                    Asset {
                        id: background_id.clone(),
                        name: "eraser-export.png".into(),
                        kind: AssetKind::Image,
                        path: "eraser-export.png".into(),
                        width: Some(200),
                        height: Some(100),
                        origin_x: None,
                        origin_y: None,
                        scale_factor: None,
                        capture_coordinates: None,
                    },
                )
                .unwrap();
            let region_id = uuid::Uuid::new_v4().to_string();
            store
                .apply(SceneCommand::AddRegion {
                    scene_id: scene_id.clone(),
                    region: Region {
                        id: region_id.clone(),
                        width: 200.,
                        height: 100.,
                        ..Default::default()
                    },
                })
                .unwrap();
            let ink = drawing(kind, &[(20., 50.), (180., 50.)]);
            store
                .apply(SceneCommand::AddDrawing {
                    scene_id: scene_id.clone(),
                    region_id: region_id.clone(),
                    background_id: background_id.clone(),
                    expected_revision: 0,
                    drawing: ink.clone(),
                })
                .unwrap();
            let current = |store: &Store| {
                store
                    .snapshot()
                    .scenes
                    .into_iter()
                    .find(|s| s.id == scene_id)
                    .unwrap()
                    .regions
                    .remove(0)
            };
            let original = crop_region(&background(), &current(&store))
                .unwrap()
                .into_rgba8();
            store
                .apply(SceneCommand::EraseDrawingStrokes {
                    scene_id: scene_id.clone(),
                    region_id: region_id.clone(),
                    background_id: background_id.clone(),
                    expected_revision: 1,
                    erasures: vec![DrawingStrokeErase {
                        drawing_id: ink.id.clone(),
                        fragments: vec![
                            DrawingStrokeFragment {
                                id: ink.id.clone(),
                                points: vec![
                                    DrawingPoint { x: 20., y: 50. },
                                    DrawingPoint { x: 90., y: 50. },
                                ],
                            },
                            DrawingStrokeFragment {
                                id: uuid::Uuid::new_v4().to_string(),
                                points: vec![
                                    DrawingPoint { x: 110., y: 50. },
                                    DrawingPoint { x: 180., y: 50. },
                                ],
                            },
                        ],
                    }],
                })
                .unwrap();
            let erased = crop_region(&background(), &current(&store))
                .unwrap()
                .into_rgba8();
            assert_ne!(original.get_pixel(100, 50).0, [255, 255, 255, 255]);
            assert_eq!(erased.get_pixel(100, 50).0, [255, 255, 255, 255]);
            for x in [40, 80, 120, 160] {
                assert_eq!(erased.get_pixel(x, 50), original.get_pixel(x, 50));
            }
            store
                .apply(SceneCommand::UndoDrawing {
                    scene_id: scene_id.clone(),
                    region_id: region_id.clone(),
                    background_id: background_id.clone(),
                    expected_revision: 2,
                })
                .unwrap();
            assert_eq!(
                crop_region(&background(), &current(&store))
                    .unwrap()
                    .into_rgba8(),
                original
            );
            store
                .apply(SceneCommand::RedoDrawing {
                    scene_id: scene_id.clone(),
                    region_id: region_id.clone(),
                    background_id: background_id.clone(),
                    expected_revision: 3,
                })
                .unwrap();
            assert_eq!(
                crop_region(&background(), &current(&store))
                    .unwrap()
                    .into_rgba8(),
                erased
            );
        }
    }

    #[test]
    fn rectangle_mark_preserves_frozen_text_and_uses_disjoint_original_fill_border_alpha() {
        let mut pixels = RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255]));
        for y in 35..58 {
            pixels.put_pixel(60, y, Rgba([0, 0, 0, 255]));
            pixels.put_pixel(61, y, Rgba([128, 128, 128, 255]));
        }
        let base = DynamicImage::ImageRgba8(pixels);
        let mut mark = drawing(DrawingKind::Mark, &[(100., 70.), (30., 20.)]);
        mark.color = "#ffff00".into();
        mark.stroke_width = 1.;
        let once = crop_region(&base, &region(vec![mark.clone()])).unwrap();
        let twice = crop_region(&base, &region(vec![mark.clone(), mark.clone()])).unwrap();
        assert_eq!(once.get_pixel(50, 30), Rgba([255, 255, 171, 255]));
        assert_eq!(once.get_pixel(50, 10), Rgba([255, 255, 65, 255]));
        assert_eq!(once.get_pixel(40, 30), Rgba([0, 0, 0, 255]));
        assert_eq!(twice.get_pixel(40, 30), Rgba([0, 0, 0, 255]));
        assert!((83..=87).contains(&once.get_pixel(41, 30)[2]));
        assert!((112..=116).contains(&twice.get_pixel(50, 30)[2]));
        assert_eq!(once.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
        assert_eq!(base.get_pixel(70, 40), Rgba([255, 255, 255, 255]));
        assert_eq!(base.get_pixel(61, 40), Rgba([128, 128, 128, 255]));
        mark.points[1].x = mark.points[0].x;
        assert!(crop_region(&base, &region(vec![mark])).is_err());
    }

    #[test]
    fn highlight_protection_uses_visible_mosaic_pixels_so_redacted_old_text_does_not_leave_holes() {
        let mut pixels = RgbaImage::from_pixel(180, 120, Rgba([255, 255, 255, 255]));
        for y in 40..60 {
            pixels.put_pixel(60, y, Rgba([0, 0, 0, 255]));
        }
        let source = DynamicImage::ImageRgba8(pixels);
        let mut redaction = drawing(DrawingKind::Mosaic, &[(40., 30.), (85., 85.)]);
        redaction.stroke_width = 40.;
        let mut mark = drawing(DrawingKind::Mark, &[(35., 25.), (110., 75.)]);
        mark.color = "#ffff00".into();
        mark.stroke_width = 1.;
        let clean = crop_region(&source, &region(vec![redaction.clone()])).unwrap();
        let result = crop_region(&source, &region(vec![redaction, mark])).unwrap();
        let old_text = clean.get_pixel(40, 40);
        let tinted = result.get_pixel(40, 40);
        assert!(old_text[0] > 180, "mosaic actually conceals old text");
        assert!(
            u16::from(tinted[2]) + 30 < u16::from(old_text[2]),
            "protected source follows mosaic: {tinted:?} / {old_text:?}"
        );
        assert_eq!(old_text, Rgba([251, 251, 251, 255]));
        assert_eq!(tinted, Rgba([252, 252, 168, 255]));
        assert_eq!(source.get_pixel(60, 50), Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn strokes_are_cropped_in_background_coordinates_with_correct_alpha() {
        let base = background();
        let result = crop_region(
            &base,
            &region(vec![drawing(DrawingKind::Pen, &[(0., 40.), (180., 40.)])]),
        )
        .unwrap();
        assert_eq!(result.dimensions(), (100, 80));
        assert_eq!(result.get_pixel(50, 30), Rgba([255, 0, 0, 255]));
        assert_eq!(result.get_pixel(50, 20), Rgba([255, 255, 255, 255]));
        assert_eq!(base.get_pixel(70, 40), Rgba([255, 255, 255, 255]));
        let dot = crop_region(
            &base,
            &region(vec![drawing(DrawingKind::Pen, &[(50., 30.)])]),
        )
        .unwrap();
        assert_eq!(dot.get_pixel(30, 20), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn translation_underlay_is_clipped_below_mosaics_and_vector_annotations() {
        let base = background();
        let overlay = RgbaImage::from_pixel(100, 80, Rgba([0, 200, 0, 255]));
        let mut mosaic = drawing(DrawingKind::Mosaic, &[(32., 22.), (70., 52.)]);
        mosaic.stroke_width = 8.;
        let area = region(vec![
            mosaic,
            drawing(DrawingKind::Pen, &[(20., 70.), (119., 70.)]),
        ]);
        let output = crop_region_with_underlay(&base, &area, Some(&overlay)).unwrap();
        assert_eq!(output.get_pixel(80, 10), Rgba([0, 200, 0, 255]));
        // Original white source, not the green translated raster, supplies the
        // mosaic preview and actual export. It obscures the translated pixels.
        assert_eq!(output.get_pixel(30, 25), Rgba([255, 255, 255, 255]));
        assert_eq!(output.get_pixel(40, 60), Rgba([255, 0, 0, 255]));
        assert_eq!(base.get_pixel(100, 20), Rgba([255, 255, 255, 255]));
        assert!(crop_region_with_underlay(&base, &area, Some(&RgbaImage::new(99, 80))).is_err());
    }

    #[test]
    fn document_rendering_does_not_require_authoring_plugin_and_keeps_shapes_clipped() {
        let mut shape = drawing(DrawingKind::Rect, &[(10., 5.), (60., 50.)]);
        shape.stroke_width = 2.;
        let result = crop_region(&background(), &region(vec![shape])).unwrap();
        assert_eq!(result.get_pixel(40, 20), Rgba([255, 0, 0, 255]));
        assert_eq!(result.get_pixel(39, 40), Rgba([255, 0, 0, 255]));
        assert_eq!(result.get_pixel(20, 20), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn arrow_wings_follow_original_length_rule_and_preserve_direction() {
        for (shaft, size) in [
            (4., 1.),
            (8., 2.),
            (32., 8.),
            (80., 20.),
            (96., 24.),
            (200., 24.),
        ] {
            for angle in (0..8).map(|i| i as f64 * std::f64::consts::FRAC_PI_4) {
                let start = DrawingPoint {
                    x: 12000.,
                    y: 13000.,
                };
                let end = DrawingPoint {
                    x: start.x + shaft * angle.cos(),
                    y: start.y + shaft * angle.sin(),
                };
                let (left, right) = arrow_wings(start, end);
                for (point, side) in [(left, -1.), (right, 1.)] {
                    let wing_angle = angle + side * std::f64::consts::FRAC_PI_6;
                    assert!((point.x - (end.x - size * wing_angle.cos())).abs() < 1e-9);
                    assert!((point.y - (end.y - size * wing_angle.sin())).abs() < 1e-9);
                }
            }
        }
        let point = DrawingPoint { x: 40., y: 50. };
        assert_eq!(arrow_wings(point, point), (point, point));
    }

    #[test]
    fn exported_arrow_has_open_wings_with_the_same_stroke_as_its_shaft() {
        let mut arrow = drawing(DrawingKind::Arrow, &[(30., 50.), (110., 50.)]);
        arrow.stroke_width = 2.;
        let output = crop_region(&background(), &region(vec![arrow.clone()])).unwrap();
        // Wing ends are at (92.68, 60)/(92.68, 40), not a filled triangle.
        assert_eq!(output.get_pixel(76, 47), Rgba([255, 0, 0, 255]));
        assert_eq!(output.get_pixel(78, 44), Rgba([255, 255, 255, 255]));
        assert_eq!(output.get_pixel(40, 40), Rgba([255, 0, 0, 255]));
        let narrow = drawing_svg(&[arrow.clone()], 200, 100);
        arrow.stroke_width = 12.;
        let wide = drawing_svg(&[arrow], 200, 100);
        let wing_points = |svg: &str| {
            roxmltree::Document::parse(svg)
                .unwrap()
                .descendants()
                .find(|node| node.has_tag_name("polyline"))
                .unwrap()
                .attribute("points")
                .unwrap()
                .to_owned()
        };
        assert_eq!(wing_points(&narrow), wing_points(&wide));
        for svg in [&narrow, &wide] {
            let document = roxmltree::Document::parse(svg).unwrap();
            assert!(!document
                .descendants()
                .any(|node| node.has_tag_name("polygon")));
            let head = document
                .descendants()
                .find(|node| node.has_tag_name("polyline"))
                .unwrap();
            assert_eq!(head.attribute("fill"), Some("none"));
            assert_eq!(head.attribute("stroke-linejoin"), Some("round"));
        }
    }

    #[test]
    fn line_has_round_ends_and_a_highlighter_is_composited_once_per_stroke() {
        let mut line = drawing(DrawingKind::Line, &[(30., 30.), (100., 30.)]);
        line.stroke_width = 8.;
        let image = crop_region(&background(), &region(vec![line])).unwrap();
        assert_eq!(image.get_pixel(8, 20), Rgba([255, 0, 0, 255]));
        assert_eq!(image.get_pixel(4, 20), Rgba([255, 255, 255, 255]));
        let mut marker = drawing(DrawingKind::Highlighter, &[(30., 50.), (100., 50.)]);
        marker.stroke_width = 10.;
        let once = crop_region(&background(), &region(vec![marker.clone()])).unwrap();
        let mut retraced = marker.clone();
        retraced.points.extend([
            DrawingPoint { x: 30., y: 50. },
            DrawingPoint { x: 100., y: 50. },
        ]);
        let retraced = crop_region(&background(), &region(vec![retraced])).unwrap();
        let sample = once.get_pixel(50, 40);
        assert_eq!(sample[0], 255);
        assert!((165..=167).contains(&sample[1]));
        assert_eq!(retraced.get_pixel(50, 40), sample);
        let twice = crop_region(&background(), &region(vec![marker.clone(), marker])).unwrap();
        assert!(twice.get_pixel(50, 40)[1] < sample[1] - 30);
        let mut dot = drawing(DrawingKind::Highlighter, &[(70., 50.)]);
        dot.stroke_width = 10.;
        let dot = crop_region(&background(), &region(vec![dot])).unwrap();
        assert_eq!(dot.get_pixel(50, 40), sample);
    }

    #[test]
    fn protected_marker_keeps_frozen_text_pixels_through_repeats_and_crop_offsets() {
        let mut source = RgbaImage::from_pixel(180, 120, Rgba([255, 255, 255, 255]));
        for y in 40..60 {
            source.put_pixel(60, y, Rgba([0, 0, 0, 255]));
            source.put_pixel(61, y, Rgba([128, 128, 128, 255]));
        }
        let source = DynamicImage::ImageRgba8(source);
        let mut marker = drawing(DrawingKind::Highlighter, &[(30., 50.), (100., 50.)]);
        marker.color = "#ffff00".into();
        marker.stroke_width = 18.;
        let once = crop_region(&source, &region(vec![marker.clone()])).unwrap();
        let twice = crop_region(&source, &region(vec![marker.clone(), marker])).unwrap();
        // region starts at 20,10; samples stay in the marker's fully covered interior.
        for result in [&once, &twice] {
            assert_eq!(result.get_pixel(40, 40), Rgba([0, 0, 0, 255]));
            assert!(result.get_pixel(41, 40)[0].abs_diff(128) <= 1);
            assert!(result.get_pixel(41, 40)[1].abs_diff(128) <= 1);
        }
        assert!(once.get_pixel(41, 40)[2] < 128);
        assert!(twice.get_pixel(41, 40)[2] < once.get_pixel(41, 40)[2]);
        assert_eq!(once.get_pixel(41, 5), source.get_pixel(61, 15));
        if let Some(root) = std::env::var_os("MEWU_HIGHLIGHT_PIXEL_PROBE") {
            let root = std::path::PathBuf::from(root);
            assert!(root.is_absolute() && root.is_dir());
            let write_png = |name: &str, image: &DynamicImage| {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(root.join(name))
                    .unwrap();
                image.write_to(&mut file, image::ImageFormat::Png).unwrap();
            };
            write_png("source.png", &source);
            write_png("once.png", &once);
            write_png("twice.png", &twice);
            let mut marker = drawing(DrawingKind::Highlighter, &[(30., 50.), (100., 50.)]);
            marker.color = "#ffff00".into();
            marker.stroke_width = 18.;
            let geometry = region(vec![]);
            let (x, y, w, h) = crop_bounds(&source, &geometry).unwrap();
            for (name, marks) in [
                ("plain_once.png", vec![marker.clone()]),
                ("plain_twice.png", vec![marker.clone(), marker.clone()]),
            ] {
                let mut pixels = source.crop_imm(x, y, w, h).into_rgba8();
                paint_plain_vectors(&marks, source.width(), source.height(), &mut pixels, x, y)
                    .unwrap();
                write_png(name, &DynamicImage::ImageRgba8(pixels));
            }
            let record = serde_json::json!({"region":region(vec![marker]),"sourceWidth":180,"sourceHeight":120,"nativeExpectedOnce":"once.png","nativeExpectedTwice":"twice.png"});
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("fixture.json"))
                .unwrap();
            std::io::Write::write_all(&mut file, &serde_json::to_vec(&record).unwrap()).unwrap();
        }
    }

    #[test]
    fn mosaic_clips_in_background_grid_and_all_vectors_remain_above_raster() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_fn(70, 50, |x, y| {
            Rgba([(x * 3) as u8, (y * 4) as u8, (x + y) as u8, 255])
        }));
        let before = base.clone();
        let mut small = drawing(DrawingKind::Mosaic, &[(27.1, 31.2), (8.4, 9.7)]);
        small.stroke_width = 6.;
        let mut large = drawing(DrawingKind::Mosaic, &[(18., 16.), (34., 35.)]);
        large.stroke_width = 12.;
        let line = drawing(DrawingKind::Line, &[(0., 20.), (70., 20.)]);
        let target = Region {
            x: 5.,
            y: 7.,
            width: 30.,
            height: 30.,
            drawings: vec![line, small.clone(), large.clone()],
            ..Default::default()
        };
        let result = crop_region(&base, &target).unwrap();
        let six = mosaic_grid(&base, 6).unwrap();
        let twelve = mosaic_grid(&base, 12).unwrap();
        assert_eq!(result.get_pixel(3, 2), *six.get_pixel(1, 1)); // floor(8.4,9.7)
        assert_eq!(result.get_pixel(22, 23), *twelve.get_pixel(2, 2));
        assert_eq!(result.get_pixel(23, 3), base.get_pixel(28, 10)); // ceil right is exclusive
        assert_eq!(result.get_pixel(2, 2), base.get_pixel(7, 9));
        assert_eq!(result.get_pixel(15, 18), *twelve.get_pixel(1, 2)); // overlap uses original, not old mosaic
        assert_eq!(result.get_pixel(15, 13), Rgba([255, 0, 0, 255])); // vector added before mosaics is still above
        let reversed = crop_region(
            &base,
            &Region {
                drawings: vec![large, small],
                ..target
            },
        )
        .unwrap();
        assert_eq!(reversed.get_pixel(15, 18), *six.get_pixel(3, 4));
        assert_eq!(base, before);
    }

    #[test]
    fn new_tool_rendering_rejects_invalid_documents_before_building_svg() {
        for text in ["01", "0", "10000", "<image href='x'/>", "１"] {
            let mut number = drawing(DrawingKind::Number, &[(30., 20.)]);
            number.text = Some(text.into());
            number.font_size = Some(30.);
            assert!(crop_region(&background(), &region(vec![number])).is_err());
        }
        let mut mosaic = drawing(DrawingKind::Mosaic, &[(30., 20.), (60., 50.)]);
        mosaic.stroke_width = 6.5;
        assert!(crop_region(&background(), &region(vec![mosaic])).is_err());
        assert!(crop_region(
            &background(),
            &region(vec![drawing(DrawingKind::Line, &[(30., 20.), (30., 20.)])])
        )
        .is_err());
        assert!(crop_region(
            &background(),
            &region(vec![drawing(DrawingKind::Highlighter, &[])])
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn numbered_circles_fit_four_digits_and_keep_top_left_geometry() {
        let values = ["1", "12", "123", "9999"];
        let drawings = values
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let mut number = drawing(DrawingKind::Number, &[(4. + index as f64 * 48., 20.)]);
                number.text = Some((*text).into());
                number.font_size = Some(40.);
                number
            })
            .collect();
        let image = crop_region(
            &background(),
            &Region {
                x: 0.,
                y: 0.,
                width: 200.,
                height: 100.,
                drawings,
                ..Default::default()
            },
        )
        .unwrap();
        for index in 0..4 {
            let left = 4 + index * 48;
            assert_eq!(image.get_pixel(left + 20, 22), Rgba([255, 0, 0, 255]));
            assert_eq!(image.get_pixel(left + 20, 18), Rgba([255, 255, 255, 255]));
            let dark_inside = (left + 5..left + 35)
                .flat_map(|x| (32..48).map(move |y| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y)[0] < 90)
                .count();
            assert!(
                dark_inside > 8,
                "number {} has no readable dark label",
                values[index as usize]
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn number_exports_use_contrasting_glyphs_on_light_and_dark_fills() {
        for (fill, foreground) in [("#FFFFFF", 0_u8), ("#242832", 255_u8), ("#F4C94D", 0_u8)] {
            let mut number = drawing(DrawingKind::Number, &[(10., 10.)]);
            number.color = fill.into();
            number.text = Some("8".into());
            number.font_size = Some(48.);
            let image = crop_region(
                &background(),
                &Region {
                    x: 0.,
                    y: 0.,
                    width: 80.,
                    height: 80.,
                    drawings: vec![number],
                    ..Default::default()
                },
            )
            .unwrap();
            let glyph_pixels = (20..48)
                .flat_map(|x| (22..48).map(move |y| (x, y)))
                .filter(|&(x, y)| {
                    image.get_pixel(x, y).0[..3]
                        .iter()
                        .all(|value| value.abs_diff(foreground) < 20)
                })
                .count();
            assert!(glyph_pixels > 8, "exported number is unreadable on {fill}");
        }
        for (fill, expected) in [
            ("#757575", "#FFFFFF"),
            ("#767676", "#000000"),
            ("#0000FF", "#FFFFFF"),
            ("#00FF00", "#000000"),
        ] {
            assert_eq!(number_foreground(fill), expected);
        }
    }

    #[test]
    fn malicious_xml_and_invalid_geometry_never_become_resources() {
        assert_eq!(escape_text("<&>\"'"), "&lt;&amp;&gt;&quot;&apos;");
        let mut text = drawing(DrawingKind::Text, &[(25., 20.)]);
        text.text = Some("</text><image href=\"file:///private\"/>".into());
        text.font_size = Some(14.);
        let xml = drawing_svg(&[text], 200, 100);
        assert!(!xml.contains("<image"));
        assert!(xml.contains("&lt;image"));
        let mut invalid = region(vec![]);
        invalid.x = f64::NAN;
        assert!(crop_region(&background(), &invalid).is_err());
        let mut unsafe_color = drawing(DrawingKind::Pen, &[(20., 20.)]);
        unsafe_color.color = "url(file:///private)".into();
        assert!(crop_region(&background(), &region(vec![unsafe_color])).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn chinese_text_uses_cached_system_fonts_and_preserves_line_positions() {
        let first = fonts().unwrap();
        let second = fonts().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(first
            .faces()
            .any(|face| face.families.iter().any(|(name, _)| name.contains("YaHei"))));
        let mut text = drawing(DrawingKind::Text, &[(25., 10.)]);
        text.text = Some("中文\n标注<&>".into());
        text.font_size = Some(20.);
        let image = crop_region(&background(), &region(vec![text])).unwrap();
        let row_pixels = |from: u32, to: u32| {
            (from..to)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y)[1] < 200)
                .count()
        };
        assert!(row_pixels(0, 23) > 30);
        assert!(row_pixels(25, 50) > 30);
    }
}
