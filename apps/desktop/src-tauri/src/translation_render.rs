// SPDX-License-Identifier: MPL-2.0
//! Bounded, host-generated text layout. The returned PNG and selection boxes
//! share one native layout; no imported SVG or browser font shaping is used.
use image::{DynamicImage, GenericImageView, RgbaImage};
use mewu_core::{OcrDocument, OcrLine, OcrWord, TranslationDocument};
use resvg::{tiny_skia, usvg};
use std::{
    fmt::Write as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};
use unicode_segmentation::UnicodeSegmentation;

const FAMILY: &str = "Segoe UI, Microsoft YaHei, Yu Gothic, Malgun Gothic, Segoe UI Emoji, PingFang SC, Noto Sans CJK SC, DejaVu Sans, sans-serif";
const MIN_FONT: f64 = 9.;
const MAX_MEASUREMENTS: usize = 8192;
const MAX_ROWS: usize = 512;
const NO_ROOM: &str = "译文在当前选区放不下，请缩小翻译范围";
const CANCELED: &str = "已取消原位翻译";

pub struct RenderedTranslation {
    pub overlay: RgbaImage,
    pub selection: OcrDocument,
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
impl Rect {
    fn right(self) -> f64 {
        self.x + self.w
    }
    fn bottom(self) -> f64 {
        self.y + self.h
    }
}
#[derive(Clone, Copy, Debug)]
struct Metrics {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
struct Row {
    text: String,
    metric: Metrics,
    x: f64,
    y: f64,
}
struct Layout {
    rows: Vec<Row>,
    background: Rect,
    size: f64,
}
struct Measure<'a> {
    options: usvg::Options<'static>,
    calls: usize,
    check: &'a dyn Fn() -> Result<(), String>,
}

impl<'a> Measure<'a> {
    fn new(check: &'a dyn Fn() -> Result<(), String>) -> Result<Self, String> {
        check()?;
        let mut options = usvg::Options::default();
        options.fontdb = fonts()?;
        options.font_family = "Segoe UI".into();
        options.image_href_resolver = usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        };
        check()?;
        Ok(Self {
            options,
            calls: 0,
            check,
        })
    }
    fn text(&mut self, text: &str, size: f64) -> Result<Metrics, String> {
        (self.check)()?;
        self.calls += 1;
        if self.calls > MAX_MEASUREMENTS {
            return Err("翻译排版超过处理上限，请缩小选区".into());
        }
        let xml=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16384\" height=\"16384\"><text id=\"measure\" x=\"0\" y=\"0\" font-family=\"{FAMILY}\" font-size=\"{size}\" xml:space=\"preserve\">{}</text></svg>",escape(text));
        let tree = usvg::Tree::from_str(&xml, &self.options).map_err(|_| "无法排版译文")?;
        let Some(usvg::Node::Text(node)) = tree.node_by_id("measure") else {
            return Err("系统字体无法显示译文".into());
        };
        if node
            .layouted()
            .iter()
            .flat_map(|span| &span.positioned_glyphs)
            .any(|glyph| glyph.id.0 == 0 && glyph.text.chars().any(|c| !c.is_whitespace()))
        {
            return Err("系统字体缺少译文所需字符".into());
        }
        let b = node.bounding_box();
        let metric = Metrics {
            x: b.x() as f64,
            y: b.y() as f64,
            w: b.width() as f64,
            h: b.height() as f64,
        };
        if metric.w <= 0. || metric.h <= 0. {
            return Err("无法测量译文文字".into());
        }
        Ok(metric)
    }
}

/// Source pixels must already include privacy mosaics and exclude previous
/// translations and ordinary annotations. Files, network and models are not
/// accessed here. Every line succeeds before any result is returned.
#[cfg(test)]
pub fn render(
    clean: &DynamicImage,
    document: &TranslationDocument,
) -> Result<RenderedTranslation, String> {
    render_cancellable(clean, document, &AtomicBool::new(false))
}

pub fn render_cancellable(
    clean: &DynamicImage,
    document: &TranslationDocument,
    canceled: &AtomicBool,
) -> Result<RenderedTranslation, String> {
    render_checked(clean, document, &|| {
        if canceled.load(Ordering::Acquire) {
            Err(CANCELED.into())
        } else {
            Ok(())
        }
    })
}

fn render_checked(
    clean: &DynamicImage,
    document: &TranslationDocument,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<RenderedTranslation, String> {
    // An already-canceled task must not even load fonts or allocate its raster.
    check()?;
    mewu_core::validate_translation_document(document).map_err(|e| e.to_string())?;
    if clean.dimensions() != (document.width, document.height) {
        return Err("翻译来源尺寸不一致".into());
    }
    let mut measure = Measure::new(check)?;
    let boxes: Vec<Rect> = document
        .lines
        .iter()
        .map(|line| Rect {
            x: line.r#box.x,
            y: line.r#box.y,
            w: line.r#box.width,
            h: line.r#box.height,
        })
        .collect();
    let cells = allocate_cells(
        &boxes,
        document.width as f64,
        document.height as f64,
        document.text_angle.unwrap_or(0.),
    )?;
    let mut layouts = Vec::with_capacity(boxes.len());
    let mut total_rows = 0;
    for ((line, source), cell) in document.lines.iter().zip(&boxes).zip(cells) {
        check()?;
        let text = normalize(&line.text);
        let layout = layout_line(&mut measure, &text, *source, cell, document)?;
        total_rows += layout.rows.len();
        if total_rows > MAX_ROWS {
            return Err("译文换行数量超过上限，请缩小选区".into());
        }
        layouts.push(layout);
    }
    let mut xml=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><g transform=\"rotate({} {} {})\">",document.width,document.height,document.width,document.height,document.text_angle.unwrap_or(0.),document.width as f64/2.,document.height as f64/2.);
    let mut selection = OcrDocument {
        engine: "mewu.translation.layout.v1".into(),
        language: document.target_language.clone(),
        width: document.width,
        height: document.height,
        text_angle: document.text_angle,
        lines: Vec::with_capacity(layouts.len()),
    };
    for (line, layout) in document.lines.iter().zip(&layouts) {
        check()?;
        let b = layout.background;
        let rgb = background_color(clean, b, document.text_angle.unwrap_or(0.));
        let color = if rgb[0] as u32 * 299 + rgb[1] as u32 * 587 + rgb[2] as u32 * 114 > 150000 {
            "#171717"
        } else {
            "#FAFAFA"
        };
        write!(
            xml,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#{:02X}{:02X}{:02X}\"/>",
            b.x, b.y, b.w, b.h, rgb[0], rgb[1], rgb[2]
        )
        .unwrap();
        let mut selected = OcrLine {
            text: normalize(&line.text),
            words: Vec::new(),
        };
        for row in &layout.rows {
            write!(xml,"<text x=\"{}\" y=\"{}\" font-family=\"{FAMILY}\" font-size=\"{}\" fill=\"{color}\" xml:space=\"preserve\">{}</text>",row.x-row.metric.x,row.y-row.metric.y,layout.size,escape(&row.text)).unwrap();
            selected.words.push(OcrWord {
                text: row.text.clone(),
                x: row.x,
                y: row.y,
                width: row.metric.w,
                height: row.metric.h,
            });
        }
        selection.lines.push(selected);
    }
    xml.push_str("</g></svg>");
    mewu_core::validate_ocr_document(&selection).map_err(|e| e.to_string())?;
    check()?;
    let tree = usvg::Tree::from_str(&xml, &measure.options).map_err(|_| "无法生成译文图层")?;
    check()?;
    let mut pixmap =
        tiny_skia::Pixmap::new(document.width, document.height).ok_or("译文图层过大")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    check()?;
    let mut overlay = RgbaImage::new(document.width, document.height);
    for (out, source) in overlay
        .as_mut()
        .chunks_exact_mut(document.width as usize * 4)
        .zip(pixmap.pixels().chunks_exact(document.width as usize))
    {
        check()?;
        for (out, pixel) in out.chunks_exact_mut(4).zip(source) {
            let pixel = pixel.demultiply();
            out.copy_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
        }
    }
    check()?;
    Ok(RenderedTranslation { overlay, selection })
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// XY cuts follow the original Mewu layout's whitespace partitions. A cell is
/// never allowed to cross another OCR line; overlapping source lines fail
/// explicitly instead of covering neighboring content unpredictably.
fn allocate_cells(
    boxes: &[Rect],
    width: f64,
    height: f64,
    angle: f64,
) -> Result<Vec<Rect>, String> {
    let (s, c) = angle.to_radians().sin_cos();
    let ew = c.abs() * width + s.abs() * height;
    let eh = s.abs() * width + c.abs() * height;
    let canvas = Rect {
        x: (width - ew) / 2.,
        y: (height - eh) / 2.,
        w: ew,
        h: eh,
    };
    let mut cells = vec![canvas; boxes.len()];
    let mut work = vec![((0..boxes.len()).collect::<Vec<_>>(), canvas)];
    while let Some((indices, cell)) = work.pop() {
        if indices.len() == 1 {
            cells[indices[0]] = cell;
            continue;
        }
        let mut best: Option<(f64, bool, f64, Vec<usize>, Vec<usize>)> = None;
        for horizontal in [true, false] {
            let start = |i: usize| if horizontal { boxes[i].y } else { boxes[i].x };
            let end = |i: usize| {
                if horizontal {
                    boxes[i].bottom()
                } else {
                    boxes[i].right()
                }
            };
            let mut sorted = indices.clone();
            sorted.sort_by(|a, b| start(*a).total_cmp(&start(*b)));
            let mut edge = end(sorted[0]);
            for split in 1..sorted.len() {
                let gap = start(sorted[split]) - edge;
                if gap > 0.01 && best.as_ref().is_none_or(|v| gap > v.0) {
                    best = Some((
                        gap,
                        horizontal,
                        edge + gap / 2.,
                        sorted[..split].to_vec(),
                        sorted[split..].to_vec(),
                    ));
                }
                edge = edge.max(end(sorted[split]));
            }
        }
        let Some((_, horizontal, cut, a, b)) = best else {
            return Err("原文字区域重叠，无法安全放置译文".into());
        };
        let (first, second) = if horizontal {
            (
                Rect {
                    h: cut - cell.y,
                    ..cell
                },
                Rect {
                    y: cut,
                    h: cell.bottom() - cut,
                    ..cell
                },
            )
        } else {
            (
                Rect {
                    w: cut - cell.x,
                    ..cell
                },
                Rect {
                    x: cut,
                    w: cell.right() - cut,
                    ..cell
                },
            )
        };
        work.push((a, first));
        work.push((b, second));
    }
    Ok(cells)
}

fn layout_line(
    measure: &mut Measure<'_>,
    text: &str,
    source: Rect,
    cell: Rect,
    doc: &TranslationDocument,
) -> Result<Layout, String> {
    let x = (source.x - 3.).max(cell.x) + 2.;
    let y = (source.y - 1.).max(cell.y) + 2.;
    let available = cell.right() - x - 2.;
    let bottom = cell.bottom() - 2.;
    if available <= 0. || bottom <= y {
        return Err(NO_ROOM.into());
    }
    let mut size = (source.h * 0.78).clamp(MIN_FONT, 28.).floor();
    loop {
        if let Some(rows) = wrap(measure, text, size, available, x, y, bottom)? {
            if rows.iter().all(|r| {
                inside_image(
                    Rect {
                        x: r.x,
                        y: r.y,
                        w: r.metric.w,
                        h: r.metric.h,
                    },
                    doc,
                )
            }) {
                let right = rows
                    .iter()
                    .map(|r| r.x + r.metric.w)
                    .fold(source.right(), f64::max);
                let last = rows.last().unwrap();
                let end = (last.y + last.metric.h).max(source.bottom());
                let background = Rect {
                    x: (source.x - 2.).max(cell.x),
                    y: (source.y - 2.).max(cell.y),
                    w: 0.,
                    h: 0.,
                };
                return Ok(Layout {
                    rows,
                    size,
                    background: Rect {
                        w: (right + 2.).min(cell.right()) - background.x,
                        h: (end + 2.).min(cell.bottom()) - background.y,
                        ..background
                    },
                });
            }
        }
        if size == MIN_FONT {
            return Err(NO_ROOM.into());
        }
        size = (size - 2.).max(MIN_FONT);
    }
}

fn wrap(
    measure: &mut Measure<'_>,
    text: &str,
    size: f64,
    width: f64,
    x: f64,
    y: f64,
    bottom: f64,
) -> Result<Option<Vec<Row>>, String> {
    let mut remaining = text;
    let mut rows = Vec::new();
    let mut top = y;
    while !remaining.is_empty() {
        let boundaries: Vec<usize> = remaining
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(remaining.len()))
            .collect();
        let mut lo = 1;
        let mut hi = boundaries.len() - 1;
        let mut best = 0;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let part = remaining[..boundaries[mid]].trim_end();
            let fits = part.chars().count() <= 512 && measure.text(part, size)?.w <= width;
            if fits {
                best = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        if best == 0 {
            return Ok(None);
        }
        let mut end = boundaries[best];
        if end < remaining.len() {
            if let Some((space, _)) = remaining[..end]
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
            {
                if space >= end / 2 {
                    end = space;
                }
            }
        }
        let part = remaining[..end].trim_end();
        let metric = measure.text(part, size)?;
        if top + metric.h > bottom || rows.len() >= MAX_ROWS {
            return Ok(None);
        }
        rows.push(Row {
            text: part.into(),
            metric,
            x,
            y: top,
        });
        top += metric.h + size * 0.15;
        remaining = remaining[end..].trim_start();
    }
    Ok(Some(rows))
}

fn rotated(x: f64, y: f64, width: f64, height: f64, angle: f64) -> (f64, f64) {
    let (s, c) = angle.to_radians().sin_cos();
    let (dx, dy) = (x - width / 2., y - height / 2.);
    (width / 2. + c * dx - s * dy, height / 2. + s * dx + c * dy)
}
fn inside_image(rect: Rect, doc: &TranslationDocument) -> bool {
    [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.x, rect.bottom()),
        (rect.right(), rect.bottom()),
    ]
    .iter()
    .all(|(x, y)| {
        let (x, y) = rotated(
            *x,
            *y,
            doc.width as f64,
            doc.height as f64,
            doc.text_angle.unwrap_or(0.),
        );
        x >= 0. && y >= 0. && x <= doc.width as f64 && y <= doc.height as f64
    })
}

/// The dominant perimeter-color bin is less likely to pick dark original ink
/// than a flat average. Every alpha sample is first composited onto white.
fn background_color(image: &DynamicImage, rect: Rect, angle: f64) -> [u8; 3] {
    let mut bins = vec![[0u32; 4]; 4096];
    for edge in 0..4 {
        for step in 0..32 {
            let t = step as f64 / 31.;
            let (x, y) = match edge {
                0 => (rect.x - 1., rect.y + t * rect.h),
                1 => (rect.right() + 1., rect.y + t * rect.h),
                2 => (rect.x + t * rect.w, rect.y - 1.),
                _ => (rect.x + t * rect.w, rect.bottom() + 1.),
            };
            let (x, y) = rotated(x, y, image.width() as f64, image.height() as f64, angle);
            let p = image.get_pixel(
                x.round().clamp(0., image.width() as f64 - 1.) as u32,
                y.round().clamp(0., image.height() as f64 - 1.) as u32,
            );
            let color = [0, 1, 2]
                .map(|i| (p[i] as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255);
            let key = ((color[0] / 16) << 8 | (color[1] / 16) << 4 | color[2] / 16) as usize;
            bins[key][0] += color[0];
            bins[key][1] += color[1];
            bins[key][2] += color[2];
            bins[key][3] += 1;
        }
    }
    let bin = bins.iter().max_by_key(|v| v[3]).unwrap();
    [0, 1, 2].map(|i| (bin[i] / bin[3]) as u8)
}

fn fonts() -> Result<Arc<usvg::fontdb::Database>, String> {
    static FONTS: OnceLock<Result<Arc<usvg::fontdb::Database>, String>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            #[allow(unused_mut)]
            let mut database = (*crate::drawing_render::fonts()?).clone();
            #[cfg(windows)]
            {
                use std::{fs::File, io::Read, path::PathBuf};
                let windows = std::env::var_os("WINDIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
                let mut bytes_left = 48 * 1024 * 1024;
                for name in ["seguiemj.ttf", "malgun.ttf", "YuGothM.ttc"] {
                    if let Ok(file) = File::open(windows.join("Fonts").join(name)) {
                        if file.metadata().is_ok_and(|meta| {
                            meta.is_file() && meta.len() <= bytes_left.min(32 * 1024 * 1024) as u64
                        }) {
                            let mut bytes = Vec::new();
                            if file
                                .take(bytes_left.min(32 * 1024 * 1024) as u64 + 1)
                                .read_to_end(&mut bytes)
                                .is_ok()
                                && bytes.len() <= bytes_left.min(32 * 1024 * 1024)
                            {
                                bytes_left -= bytes.len();
                                database.load_font_data(bytes);
                            }
                        }
                    }
                }
            }
            Ok(Arc::new(database))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use mewu_core::{TranslationBox, TranslationLine};
    fn document(text: &str) -> TranslationDocument {
        TranslationDocument {
            version: 1,
            target_language: "zh-Hans".into(),
            width: 480,
            height: 240,
            text_angle: None,
            lines: vec![TranslationLine {
                id: "line_0".into(),
                source: "source".into(),
                text: text.into(),
                r#box: TranslationBox {
                    x: 30.,
                    y: 40.,
                    width: 180.,
                    height: 24.,
                },
            }],
        }
    }
    fn clean() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(480, 240, Rgba([240, 242, 244, 255])))
    }
    #[test]
    fn chinese_uses_real_glyphs_full_canvas_and_transparent_margins() {
        let doc = document("中文翻译可以选择并复制");
        let out = render(&clean(), &doc).unwrap();
        assert_eq!(out.overlay.dimensions(), (480, 240));
        assert_eq!(out.overlay.get_pixel(0, 0)[3], 0);
        assert_eq!(out.selection.lines[0].text, doc.lines[0].text);
        assert!(out.overlay.pixels().any(|p| p[3] > 0 && p[0] < 100));
        let word = &out.selection.lines[0].words[0];
        assert!(out
            .overlay
            .enumerate_pixels()
            .any(|(x, y, p)| x as f64 >= word.x
                && x as f64 <= word.x + word.width
                && y as f64 >= word.y
                && y as f64 <= word.y + word.height
                && p[3] > 0
                && p[0] < 100));
    }
    #[test]
    fn wraps_long_translation_without_changing_copy_text_or_cutting_graphemes() {
        let doc = document(&"清楚的译文需要保持可读字号和正确顺序。".repeat(12));
        let out = render(&clean(), &doc).unwrap();
        assert!(out.selection.lines[0].words.len() > 1);
        assert_eq!(
            out.selection.lines[0]
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<String>(),
            doc.lines[0].text
        );
        let check = || Ok(());
        let mut measure = Measure::new(&check).unwrap();
        let text = "e\u{301}".repeat(16);
        let rows = wrap(&mut measure, &text, 18., 35., 0., 0., 1000.)
            .unwrap()
            .unwrap();
        assert!(rows
            .iter()
            .all(|row| row.text.starts_with('e') && row.text.ends_with('\u{301}')));
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<String>(),
            text
        );
    }
    #[test]
    fn emoji_clusters_are_not_split_and_font_failure_is_explicit() {
        let check = || Ok(());
        let mut measure = Measure::new(&check).unwrap();
        let text = "👩‍💻👩‍💻";
        match wrap(&mut measure, text, 18., 40., 0., 0., 400.) {
            Ok(Some(rows)) => {
                assert_eq!(
                    rows.iter().map(|r| r.text.as_str()).collect::<String>(),
                    text
                );
                assert!(rows
                    .iter()
                    .all(|r| r.text.graphemes(true).all(|g| g == "👩‍💻")));
            }
            Err(error) => assert!(error.contains("字体")),
            _ => panic!("sufficient room must render or explicitly report unavailable font"),
        }
    }
    #[test]
    fn angle_selection_and_pixels_share_one_rotation() {
        let mut doc = document("旋转文字");
        doc.text_angle = Some(14.);
        doc.lines[0].r#box.x = 140.;
        doc.lines[0].r#box.y = 90.;
        let out = render(&clean(), &doc).unwrap();
        assert_eq!(out.selection.text_angle, Some(14.));
        let w = &out.selection.lines[0].words[0];
        let (x, y) = rotated(w.x + w.width / 2., w.y + w.height / 2., 480., 240., 14.);
        assert_eq!(out.overlay.get_pixel(x as u32, y as u32)[3], 255);
        assert!(inside_image(
            Rect {
                x: w.x,
                y: w.y,
                w: w.width,
                h: w.height
            },
            &doc
        ));
    }
    #[test]
    fn literal_markup_is_text_and_unfit_document_returns_no_partial_result() {
        let doc = document("<script>alert('x')</script> & <image href=\"https://invalid/\"/>");
        let out = render(&clean(), &doc).unwrap();
        assert_eq!(out.selection.lines[0].text, doc.lines[0].text);
        let mut too_small = document(&"长".repeat(2048));
        too_small.width = 40;
        too_small.height = 30;
        too_small.lines[0].r#box = TranslationBox {
            x: 3.,
            y: 3.,
            width: 34.,
            height: 20.,
        };
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 30, Rgba([255, 255, 255, 255])));
        assert_eq!(render(&image, &too_small).err().unwrap(), NO_ROOM);
    }
    #[test]
    fn xy_cells_do_not_cover_neighbor_and_transparent_sampling_is_white() {
        let a = Rect {
            x: 10.,
            y: 10.,
            w: 100.,
            h: 20.,
        };
        let b = Rect {
            x: 10.,
            y: 50.,
            w: 100.,
            h: 20.,
        };
        let cells = allocate_cells(&[a, b], 200., 100., 0.).unwrap();
        assert!(cells[0].bottom() <= cells[1].y);
        assert!(allocate_cells(&[a, a], 200., 100., 0.).is_err());
        let transparent =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 0])));
        assert_eq!(background_color(&transparent, a, 0.), [255, 255, 255]);
    }

    #[test]
    fn cancellation_precedes_validation_font_loading_and_raster_allocation() {
        let mut doc = document("取消");
        doc.width = 0;
        assert_eq!(
            render_cancellable(&clean(), &doc, &AtomicBool::new(true))
                .err()
                .unwrap(),
            CANCELED
        );
    }

    #[test]
    fn cancellation_during_measuring_returns_no_partial_layer() {
        let checks = std::cell::Cell::new(0);
        let check = || {
            let n = checks.get() + 1;
            checks.set(n);
            if n >= 6 {
                Err(CANCELED.into())
            } else {
                Ok(())
            }
        };
        assert_eq!(
            render_checked(&clean(), &document(&"需要多次测量的文字".repeat(8)), &check)
                .err()
                .unwrap(),
            CANCELED
        );
        assert_eq!(checks.get(), 6);
    }

    #[test]
    #[ignore = "writes only a synthetic native layout fixture under workspace .private"]
    fn export_synthetic_translation_fixture() {
        let mut doc = document("中文译文保持同一份像素，选字与导出使用相同布局。");
        doc.lines.push(TranslationLine {
            id: "line_1".into(),
            source: "A second synthetic line".into(),
            text: "较长的第二行会按实际字体测量换行，保留完整译文，不会缩成看不清的小字。"
                .repeat(3),
            r#box: TranslationBox {
                x: 30.,
                y: 100.,
                width: 360.,
                height: 24.,
            },
        });
        let rendered = render(&clean(), &doc).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.private/remake-review");
        std::fs::create_dir_all(&root).unwrap();
        rendered
            .overlay
            .save(root.join("translation-render-fixture.png"))
            .unwrap();
        std::fs::write(
            root.join("translation-render-fixture.json"),
            serde_json::to_vec_pretty(&rendered.selection).unwrap(),
        )
        .unwrap();
    }
}
