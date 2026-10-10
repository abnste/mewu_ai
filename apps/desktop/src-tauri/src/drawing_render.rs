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
const FONT_FAMILY: &str =
    "Segoe UI, Microsoft YaHei, PingFang SC, Noto Sans CJK SC, DejaVu Sans, sans-serif";
static FONTS: OnceLock<Result<Arc<usvg::fontdb::Database>, String>> = OnceLock::new();

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
    if let Some(layer) = underlay {
        if layer.dimensions() != (width, height) {
            return Err("译文图层尺寸与选区不符".into());
        }
        for (target, source) in bitmap.pixels_mut().zip(layer.pixels()) {
            blend_rgba(target, source);
        }
    }
    mosaic::paint_mosaics(image, &region.drawings, &mut bitmap, x, y)?;
    // Screenshot repairs and lifted pixels are raster content, beneath editable
    // ink/text. Their exact immutable bytes also drive preview, exports and AI.
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
            &mut bitmap,
            &raster,
            p[0].x - f64::from(x),
            p[0].y - f64::from(y),
            p[1].x - p[0].x,
            p[1].y - p[0].y,
        )?;
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
    )?;
    Ok(DynamicImage::ImageRgba8(bitmap))
}

fn paint_vectors(
    drawings: &[Drawing],
    source_width: u32,
    source_height: u32,
    bitmap: &mut image::RgbaImage,
    x: u32,
    y: u32,
) -> Result<(), String> {
    if drawings.iter().all(|d| d.kind == DrawingKind::Mosaic) {
        return Ok(());
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
        options.fontdb = fonts()?;
        options.font_family = "Segoe UI".into();
    }
    let tree = usvg::Tree::from_str(&xml, &options).map_err(|_| "无法排版绘制内容")?;
    let mut overlay =
        tiny_skia::Pixmap::new(bitmap.width(), bitmap.height()).ok_or("选区过大，无法合成标注")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_translate(-(x as f32), -(y as f32)),
        &mut overlay.as_mut(),
    );
    // tiny-skia stores premultiplied RGBA; image::Pixel::blend expects straight
    // alpha. Demultiply before compositing to avoid dark anti-aliased edges.
    blend_overlay(bitmap, &overlay);
    Ok(())
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
            DrawingKind::Rect | DrawingKind::Ellipse | DrawingKind::Mosaic | DrawingKind::Rich
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
            DrawingKind::Arrow => {
                let end = drawing.points[1];
                let (left, right) = arrow_wings(first, end, stroke);
                let _ = write!(svg, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"round\"/><polygon points=\"{},{} {},{} {},{}\" fill=\"{color}\"/>", first.x, first.y, end.x, end.y, end.x, end.y, left.x, left.y, right.x, right.y);
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
                let _ = write!(svg, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{}\" fill=\"{color}\"/><text x=\"{cx}\" y=\"{}\" fill=\"#ffffff\" font-family=\"{FONT_FAMILY}\" font-size=\"{size}\" font-weight=\"600\" text-anchor=\"middle\">{text}</text>", diameter/2., cy + size * 0.35);
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

fn arrow_wings(
    start: DrawingPoint,
    end: DrawingPoint,
    stroke: f64,
) -> (DrawingPoint, DrawingPoint) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let distance = dx.hypot(dy);
    if distance < f64::EPSILON {
        return (end, end);
    }
    let length = (stroke * 4.).max(12.).min(distance);
    let ux = dx / distance;
    let uy = dy / distance;
    let bx = end.x - ux * length;
    let by = end.y - uy * length;
    (
        DrawingPoint {
            x: bx - uy * length * 0.48,
            y: by + ux * length * 0.48,
        },
        DrawingPoint {
            x: bx + uy * length * 0.48,
            y: by - ux * length * 0.48,
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

pub(crate) fn fonts() -> Result<Arc<usvg::fontdb::Database>, String> {
    FONTS
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            let mut bytes_left = MAX_FONT_BYTES;
            let mut loaded = 0;
            for path in font_paths() {
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
            database.set_sans_serif_family("Segoe UI");
            Ok(Arc::new(database))
        })
        .clone()
}

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
    #[cfg(target_os = "macos")]
    {
        return [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/SFNS.ttf",
        ]
        .iter()
        .map(PathBuf::from)
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgba, RgbaImage};

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
        }
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
        let (left, right) = arrow_wings(
            DrawingPoint { x: 0., y: 0. },
            DrawingPoint { x: 3., y: 0. },
            4.,
        );
        assert_eq!(left.x, 0.);
        assert_eq!(right.x, 0.);
        assert!((left.y - 1.44).abs() < 0.001);
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
            let white_inside = (left + 5..left + 35)
                .flat_map(|x| (32..48).map(move |y| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y)[1] > 180)
                .count();
            assert!(
                white_inside > 8,
                "number {} has no readable white label",
                values[index as usize]
            );
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
