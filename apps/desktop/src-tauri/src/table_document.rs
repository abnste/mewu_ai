// SPDX-License-Identifier: MPL-2.0
//! Completed Markdown is parsed once with a CommonMark/GFM parser. Views and
//! every export consume these plain cells, never model-supplied HTML or scripts.
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_CELL_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableDocument {
    pub index: usize,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub align: Vec<Option<&'static str>>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Block {
    Markdown {
        text: String,
    },
    Table {
        #[serde(rename = "tableIndex")]
        table_index: usize,
    },
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMessage {
    pub source_hash: String,
    pub blocks: Vec<Block>,
    pub tables: Vec<TableDocument>,
}

pub fn parse(source: &str) -> Result<TableMessage, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("回答过大，无法提取表格".into());
    }
    let mut tables = Vec::new();
    let mut ranges = Vec::new();
    let mut current: Option<TableDocument> = None;
    let mut row = Vec::new();
    let mut last_cell_end = None;
    let mut cell: Option<String> = None;
    let mut start = 0;
    let mut total = 0;
    let mut total_rows = 0;
    for (event, range) in Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    )
    .into_offset_iter()
    {
        match event {
            Event::Start(Tag::Table(align)) => {
                if current.is_some() || tables.len() >= 12 || align.is_empty() || align.len() > 32 {
                    return Err("表格最多 12 张、每表 32 列，未复制部分结果".into());
                }
                start = range.start;
                current = Some(TableDocument {
                    index: tables.len(),
                    header: vec![],
                    rows: vec![],
                    align: align
                        .iter()
                        .map(|a| match a {
                            Alignment::None => None,
                            Alignment::Left => Some("left"),
                            Alignment::Center => Some("center"),
                            Alignment::Right => Some("right"),
                        })
                        .collect(),
                });
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                total_rows += 1;
                if total_rows > 200 {
                    return Err("表格合计最多 200 行，未复制部分结果".into());
                }
                row.clear();
                last_cell_end = None;
            }
            Event::Start(Tag::TableCell) => cell = Some(String::new()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(cell) = &mut cell {
                    cell.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(cell) = &mut cell {
                    cell.push('\n');
                }
            }
            Event::InlineHtml(html)
                if matches!(
                    html.to_ascii_lowercase().as_str(),
                    "<br>" | "<br/>" | "<br />"
                ) =>
            {
                if let Some(cell) = &mut cell {
                    cell.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => {
                last_cell_end = Some(range.end);
                let value = cell.take().ok_or("表格单元格不完整")?;
                if value
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
                {
                    return Err("表格包含无效控制字符".into());
                }
                total += value.len();
                if total > MAX_CELL_BYTES {
                    return Err("表格文字超过 256 KB，未复制部分结果".into());
                }
                row.push(value);
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(table) = &mut current {
                    validate_row_tail(source, last_cell_end, range.end)?;
                    table.header = std::mem::take(&mut row);
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(table) = &mut current {
                    validate_row_tail(source, last_cell_end, range.end)?;
                    if table.rows.len() >= 199 {
                        return Err("每张表格最多 200 行，未复制部分结果".into());
                    }
                    table.rows.push(std::mem::take(&mut row));
                }
            }
            Event::End(TagEnd::Table) => {
                let table = current.take().ok_or("表格不完整")?;
                if table.header.len() != table.align.len()
                    || table.rows.iter().any(|r| r.len() != table.header.len())
                {
                    return Err("表格列数不一致".into());
                }
                ranges.push((start, range.end));
                tables.push(table);
            }
            _ => {}
        }
    }
    if current.is_some() {
        return Err("表格不完整".into());
    }
    let mut blocks = Vec::new();
    let mut cursor = 0;
    for (index, (start, end)) in ranges.into_iter().enumerate() {
        if start < cursor
            || end < start
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            return Err("表格位置无效".into());
        }
        if start > cursor {
            blocks.push(Block::Markdown {
                text: source[cursor..start].into(),
            });
        }
        blocks.push(Block::Table { table_index: index });
        cursor = end;
    }
    if cursor < source.len() || blocks.is_empty() {
        blocks.push(Block::Markdown {
            text: source[cursor..].into(),
        });
    }
    Ok(TableMessage {
        source_hash: format!("{:x}", Sha256::digest(source.as_bytes())),
        blocks,
        tables,
    })
}

fn validate_row_tail(
    source: &str,
    last_cell_end: Option<usize>,
    row_end: usize,
) -> Result<(), String> {
    // pulldown-cmark's GFM parser intentionally drops cells beyond the header's
    // width (firstpass.rs::parse_table_row_inner). Its row/cell offsets still
    // cover the original source, so check only that omitted tail rather than
    // inventing a second pipe/code/escape parser. A legal tail is whitespace and
    // at most one optional closing pipe. Autocompleted empty cells end at row_end.
    let tail = source
        .get(last_cell_end.ok_or("表格行没有单元格")?..row_end)
        .ok_or("表格行位置无效")?;
    let whitespace = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let tail = tail.trim_matches(whitespace);
    if tail.is_empty() || tail == "|" {
        Ok(())
    } else {
        Err("表格数据列多于表头，无法完整复制，请调整表格列数".into())
    }
}

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
impl TableDocument {
    pub fn all_rows(&self) -> impl Iterator<Item = &Vec<String>> {
        std::iter::once(&self.header).chain(self.rows.iter())
    }
    fn column_alignment(&self, column: usize) -> &'static str {
        // Only the GFM alignment enum contributes CSS/SVG values. The public
        // document must never interpolate an arbitrary alignment as markup.
        match self.align.get(column).copied().flatten() {
            Some("center") => "center",
            Some("right") => "right",
            _ => "left",
        }
    }
    pub fn html(&self) -> String {
        let mut html = String::from("<table xmlns:x=\"urn:schemas-microsoft-com:office:excel\" border=\"1\" cellspacing=\"0\" cellpadding=\"5\" style=\"border-collapse:collapse;font-family:Calibri,Arial,sans-serif\">");
        for (index, row) in self.all_rows().enumerate() {
            let tag = if index == 0 { "th" } else { "td" };
            html.push_str("<tr>");
            for (column, value) in row.iter().enumerate() {
                let _ = write!(html, "<{tag} x:str style=\"mso-number-format:'\\@';white-space:pre-wrap;text-align:{}\">{}</{tag}>", self.column_alignment(column), escape(value).replace('\n', "<br>"));
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        html
    }
    pub fn delimited(&self, separator: char) -> String {
        self.all_rows()
            .map(|row| {
                row.iter()
                    .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(&separator.to_string())
            })
            .collect::<Vec<_>>()
            .join("\r\n")
    }
    pub fn markdown(&self) -> String {
        let literal = |cell: &str| {
            let mut output = String::new();
            for c in cell.chars() {
                if c == '\n' {
                    output.push_str("<br>");
                } else if c.is_ascii_punctuation() || matches!(c, ' ' | '\t' | '\r') {
                    let _ = write!(output, "&#{};", c as u32);
                } else {
                    output.push(c);
                }
            }
            output
        };
        let row = |cells: &[String]| {
            format!(
                "| {} |",
                cells
                    .iter()
                    .map(|cell| literal(cell))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        };
        let mut lines = vec![
            row(&self.header),
            format!(
                "| {} |",
                self.align
                    .iter()
                    .map(|a| match a {
                        Some("left") => ":---",
                        Some("center") => ":---:",
                        Some("right") => "---:",
                        _ => "---",
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        ];
        lines.extend(self.rows.iter().map(|r| row(r)));
        lines.join("\n")
    }
}

fn wrap(value: &str, columns: usize) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in value.replace('\t', "    ").split('\n') {
        let mut line = String::new();
        let mut length = 0;
        for grapheme in paragraph.graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme).max(1);
            if length + width > columns && !line.is_empty() {
                result.push(std::mem::take(&mut line));
                length = 0;
            }
            line.push_str(grapheme);
            length += width;
        }
        result.push(line);
    }
    result
}

pub fn render(table: &TableDocument) -> Result<image::RgbaImage, String> {
    let widths: Vec<f64> = (0..table.header.len())
        .map(|column| {
            table
                .all_rows()
                .flat_map(|row| row[column].split('\n'))
                .map(|s| UnicodeWidthStr::width(s).min(32) as f64 * 14.0 + 20.0)
                .fold(80.0, f64::max)
                .min(280.0)
        })
        .collect();
    let rows: Vec<Vec<Vec<String>>> = table
        .all_rows()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(col, cell)| wrap(cell, ((widths[col] - 20.0) / 14.0) as usize))
                .collect()
        })
        .collect();
    let heights: Vec<f64> = rows
        .iter()
        .map(|row| row.iter().map(Vec::len).max().unwrap_or(1) as f64 * 20.0 + 14.0)
        .collect();
    let width = widths.iter().sum::<f64>() + 36.0;
    let height = heights.iter().sum::<f64>() + 36.0;
    let scale = 1.0_f64
        .min(6000.0 / width)
        .min(6000.0 / height)
        .min((32.0 * 1024.0 * 1024.0 / (width * height)).sqrt());
    if scale < 0.65 {
        return Err("表格超出图片范围，请复制 CSV 或文本".into());
    }
    let out_width = (width * scale).ceil() as u32;
    let out_height = (height * scale).ceil() as u32;
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><g font-family=\"Segoe UI,Microsoft YaHei,Noto Sans CJK SC,sans-serif\" font-size=\"14\" fill=\"#25364a\" xml:space=\"preserve\">");
    let mut y = 18.0;
    for (r, row) in rows.iter().enumerate() {
        let mut x = 18.0;
        for (c, lines) in row.iter().enumerate() {
            let fill = if r == 0 { "#f1f4f8" } else { "#ffffff" };
            let _ = write!(svg, "<rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{}\" stroke=\"#bec7d3\" stroke-width=\"1\" fill=\"{fill}\"/>", widths[c], heights[r]);
            // SVG2 text-anchor positions each preformatted line relative to its
            // own x. Keep the original left padding and baseline unchanged.
            let (text_x, anchor) = match table.column_alignment(c) {
                "center" => (x + widths[c] / 2.0, "middle"),
                "right" => (x + widths[c] - 10.0, "end"),
                _ => (x + 10.0, "start"),
            };
            for (i, line) in lines.iter().enumerate() {
                let _ = write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\">{}</text>",
                    text_x,
                    y + 21.0 + i as f64 * 20.0,
                    escape(line)
                );
            }
            x += widths[c];
        }
        y += heights[r];
    }
    svg.push_str("</g></svg>");
    let mut options = resvg::usvg::Options::default();
    options.fontdb = crate::drawing_render::fonts()?;
    options.image_href_resolver = resvg::usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|_| "无法排版表格图片")?;
    let mut pixels = resvg::tiny_skia::Pixmap::new(out_width, out_height).ok_or("表格图片过大")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
        &mut pixels.as_mut(),
    );
    image::RgbaImage::from_raw(out_width, out_height, pixels.take())
        .ok_or_else(|| "无法生成表格图片".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ast_preserves_unicode_empty_cells_labels_escapes_and_ignores_code_fences() {
        let text = "前言\n\n```md\n|fake|x|\n|---|---|\n```\n\n| 名称 | 值 |\n| :--- | ---: |\n| **中文** &amp; `a\\|b` | |\n| [链接](https://example.com) | &lt;svg&gt;<br>下一行 |\n\n结尾";
        let result = parse(text).unwrap();
        assert_eq!(result.tables.len(), 1);
        let table = &result.tables[0];
        assert_eq!(table.align, vec![Some("left"), Some("right")]);
        assert_eq!(table.rows[0], vec!["中文 & a|b", ""]);
        assert_eq!(table.rows[1], vec!["链接", "<svg>\n下一行"]);
        assert!(
            matches!(result.blocks.first(),Some(Block::Markdown{text}) if text.contains("fake"))
        );
        assert!(
            matches!(result.blocks.last(),Some(Block::Markdown{text}) if text.contains("结尾"))
        );
        assert!(!table.html().contains("<svg>"));
        assert!(table.html().contains("&lt;svg&gt;"));
    }
    #[test]
    fn complete_table_limits_reject_instead_of_truncating() {
        let start = "|h|\n|---|\n";
        let mut source = start.to_string();
        source.push_str(&"|v|\n".repeat(199));
        assert_eq!(parse(&source).unwrap().tables[0].rows.len(), 199);
        source.push_str("|last|\n");
        assert!(parse(&source).is_err());
        assert!(parse(&format!("{}\n", start).repeat(13)).is_err());
        assert!(
            parse(&format!("{}\n", format!("{start}{}", "|v|\n".repeat(100))).repeat(2)).is_err()
        );
        assert!(parse(&format!("{start}|{}|\n", "x".repeat(MAX_CELL_BYTES + 1))).is_err());
        assert!(parse(&format!(
            "|{}|\n|{}|\n",
            vec!["h"; 33].join("|"),
            vec!["---"; 33].join("|")
        ))
        .is_err());
    }
    #[test]
    fn gfm_extra_cells_are_rejected_instead_of_silently_disappearing() {
        for row in [
            "|1|2|3|",
            "1|2|3",
            "|1|2||",
            "|1|2|  |",
            "|1|2|<br>|",
            "|1|2|`extra`|",
            "|1|2|\\|",
            "|1|2|\u{a0}",
        ] {
            let source = format!("前文\n\n|A|B|\n|---|---|\n{row}\n\n后文");
            let error = parse(&source).unwrap_err();
            assert!(error.contains("数据列多于表头"), "row {row:?}: {error}");
        }
    }

    #[test]
    fn row_offset_check_accepts_gfm_escaping_code_br_and_optional_closing_pipe() {
        for row in [
            "|1|2|",
            "1|2",
            "|1|2| \t",
            "|1|2  ",
            "|1|",
            "|1||",
            "|a\\|b|`c\\|d`|",
            "|a<br>b|c|",
            "|**a**|[b](https://example.com)|",
        ] {
            let source = format!("前文\r\n\r\n|A|B|\r\n|---|---|\r\n{row}\r\n\r\n后文");
            let parsed = parse(&source).unwrap_or_else(|error| panic!("row {row:?}: {error}"));
            assert_eq!(parsed.tables[0].rows[0].len(), 2);
            assert!(
                matches!(parsed.blocks.first(), Some(Block::Markdown { text }) if text.contains("前文"))
            );
            assert!(
                matches!(parsed.blocks.last(), Some(Block::Markdown { text }) if text.contains("后文"))
            );
        }
        let nested =
            parse("> 前言\n>\n> |A|B|\n> |---|---|\n> |a\\|b|`c\\|d`|\n>\n> 后文").unwrap();
        assert_eq!(nested.tables[0].rows[0], ["a|b", "c|d"]);
        let table = parse("|A|B|\n|---|---|\n|a<br>b|c|")
            .unwrap()
            .tables
            .remove(0);
        assert_eq!(table.rows[0], ["a\nb", "c"]);
    }
    #[test]
    fn formats_keep_values_and_grapheme_wrapping_does_not_split_emoji() {
        let table = parse("|A|B|\n|---|---|\n|a,b|\"quote\"|\n|0012|👩‍💻é|")
            .unwrap()
            .tables
            .remove(0);
        assert!(table.delimited(',').contains("\"a,b\",\"\"\"quote\"\"\""));
        assert!(table.delimited('\t').contains("\"0012\"\t\"👩‍💻é\""));
        assert_eq!(parse(&table.markdown()).unwrap().tables[0], table);
        assert_eq!(wrap("👩‍💻é", 2), vec!["👩‍💻", "é"]);
        let table = TableDocument {
            index: 0,
            header: vec![" [x](https://example.com) ".into()],
            rows: vec![vec!["~~保留~~ ![x](y) `x` | & \\\t\r\n001".into()]],
            align: vec![None],
        };
        assert_eq!(parse(&table.markdown()).unwrap().tables[0], table);
    }
    #[test]
    fn rendered_table_has_opaque_grid_and_real_unicode_glyphs() {
        let table = parse("|名称|编号|\n|---|---|\n|中文测试|0012|")
            .unwrap()
            .tables
            .remove(0);
        let image = render(&table).unwrap();
        assert!(image.width() > 150 && image.height() > 75);
        assert!(image.pixels().all(|p| p[3] == 255));
        assert!(
            image
                .pixels()
                .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 150)
                .count()
                > 50
        );
    }

    #[derive(Debug)]
    struct InkLine {
        left: u32,
        right: u32,
        top: u32,
        bottom: u32,
        pixels: usize,
    }

    fn ink_lines(image: &image::RgbaImage, left: u32, right: u32) -> Vec<InkLine> {
        // Observe rasterized dark glyph ink, excluding the lighter grid and
        // header fill. No SVG inspection or font advance/layout calculation.
        let mut lines: Vec<InkLine> = Vec::new();
        for y in 0..image.height() {
            let ink: Vec<_> = (left..right)
                .filter(|&x| {
                    let p = image.get_pixel(x, y);
                    p[0] < 100 && p[1] < 115 && p[2] < 160
                })
                .collect();
            let (Some(&first), Some(&last)) = (ink.first(), ink.last()) else {
                continue;
            };
            if let Some(line) = lines.last_mut().filter(|line| y <= line.bottom + 3) {
                line.left = line.left.min(first);
                line.right = line.right.max(last);
                line.bottom = y;
                line.pixels += ink.len();
            } else {
                lines.push(InkLine {
                    left: first,
                    right: last,
                    top: y,
                    bottom: y,
                    pixels: ink.len(),
                });
            }
        }
        lines
    }

    #[test]
    fn rendered_alignment_moves_real_unicode_header_body_and_every_wrapped_line_without_clipping() {
        let long = "天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往";
        let source = format!(
            "| 中文 | 中文 | 中文 |\n| :--- | :---: | ---: |\n| 汉字<br>ALIGN | 汉字<br>ALIGN | 汉字<br>ALIGN |\n| {long} | {long} | {long} |"
        );
        let table = parse(&source).unwrap().tables.remove(0);
        assert_eq!(
            table.align,
            vec![Some("left"), Some("center"), Some("right")]
        );
        let image = render(&table).unwrap();
        assert!(image.pixels().all(|p| p[3] == 255));
        // Equal source cells create three equal columns; measure their actual
        // rendered ink, including short and long lines, against each cell edge.
        let cell_width = (image.width() - 36) / 3;
        let columns: Vec<_> = (0..3)
            .map(|column| {
                let edge = 18 + column * cell_width;
                ink_lines(&image, edge + 1, edge + cell_width - 1)
            })
            .collect();
        assert_eq!(
            columns[0].len(),
            6,
            "header, two explicit and three wrapped lines"
        );
        assert_eq!(columns[1].len(), columns[0].len());
        assert_eq!(columns[2].len(), columns[0].len());
        for line_index in 0..columns[0].len() {
            let left = &columns[0][line_index];
            let center = &columns[1][line_index];
            let right = &columns[2][line_index];
            let center_of = |line: &InkLine, column: u32| {
                (line.left + line.right) as f64 / 2.0 - (18 + column * cell_width) as f64
            };
            assert!(center_of(center, 1) > center_of(left, 0) + 16.0);
            assert!(center_of(right, 2) > center_of(center, 1) + 16.0);
            assert!((center_of(center, 1) - cell_width as f64 / 2.0).abs() <= 4.0);
            assert!(left.left - 18 <= 14);
            assert!(18 + 3 * cell_width - right.right <= 15);
            for (column, actual) in [left, center, right].into_iter().enumerate() {
                let edge = 18 + column as u32 * cell_width;
                assert!(actual.left >= edge + 7 && actual.right <= edge + cell_width - 7);
                assert!(actual.top.abs_diff(left.top) <= 1);
                assert!(actual.bottom.abs_diff(left.bottom) <= 1);
                assert!((actual.right - actual.left).abs_diff(left.right - left.left) <= 2);
                assert!(actual.pixels.abs_diff(left.pixels) <= (left.pixels / 8).max(10));
            }
        }
    }

    #[test]
    fn default_and_explicit_left_alignment_keep_identical_raster_pixels() {
        let default = parse("|中文|Latin|\n|---|---|\n|汉字<br>第二行|ALIGN<br>WIDE|\n|0012| |");
        let table = default.unwrap().tables.remove(0);
        assert!(table.align.iter().all(Option::is_none));
        let mut left = table.clone();
        left.align.fill(Some("left"));
        let original = render(&table).unwrap();
        let aligned = render(&left).unwrap();
        assert_eq!(original.dimensions(), aligned.dimensions());
        assert_eq!(original.into_raw(), aligned.into_raw());
    }

    #[test]
    fn gfm_alignment_reaches_excel_html_cells_with_literal_values_and_escaping() {
        let source = "| *左列* | `中列` | [右列](https://example.com) | 默认 |\n| :--- | :---: | ---: | --- |\n| =SUM(A1:A2) | &lt;img src=x onerror=alert(1)&gt;<br>下行 | +2<br>-3 | &#64;cmd |";
        let table = parse(source).unwrap().tables.remove(0);
        assert_eq!(table.header, ["左列", "中列", "右列", "默认"]);
        assert_eq!(
            table.rows[0],
            [
                "=SUM(A1:A2)",
                "<img src=x onerror=alert(1)>\n下行",
                "+2\n-3",
                "@cmd"
            ]
        );
        let html = table.html();
        assert!(!html.contains("<img") && !html.contains("<script"));
        // Product Excel HTML deliberately uses a boolean x:str and HTML <br>.
        // Normalize only those fixed writer tokens for the existing XML parser
        // in this test; product syntax and cell content are left unchanged.
        let xml = html
            .replace(" x:str style=", " x:str=\"\" style=")
            .replace("<br>", "<br/>");
        let document = resvg::usvg::roxmltree::Document::parse(&xml).unwrap();
        let rows: Vec<_> = document
            .root_element()
            .children()
            .filter(|node| node.has_tag_name("tr"))
            .collect();
        assert_eq!(rows.len(), 2);
        for (row_index, row) in rows.iter().enumerate() {
            let cells: Vec<_> = row.children().filter(|node| node.is_element()).collect();
            assert_eq!(cells.len(), 4);
            for (column, cell) in cells.iter().enumerate() {
                assert!(cell.has_tag_name(if row_index == 0 { "th" } else { "td" }));
                assert_eq!(
                    cell.attribute(("urn:schemas-microsoft-com:office:excel", "str")),
                    Some("")
                );
                let style = cell.attribute("style").unwrap();
                assert!(style.contains("mso-number-format:'\\@'"));
                let expected_alignment = ["left", "center", "right", "left"][column];
                assert!(style
                    .split(';')
                    .any(|rule| rule == format!("text-align:{expected_alignment}")));
                let literal: String = cell
                    .children()
                    .map(|node| {
                        if node.has_tag_name("br") {
                            "\n"
                        } else {
                            node.text().unwrap_or_default()
                        }
                    })
                    .collect();
                assert_eq!(literal, table.all_rows().nth(row_index).unwrap()[column]);
                assert!(cell
                    .children()
                    .all(|node| !node.is_element() || node.has_tag_name("br")));
            }
        }
    }

    #[test]
    fn alignment_is_a_fixed_enum_and_cannot_inject_excel_html_attributes() {
        let mut table = parse("|中文|\n|---|\n|=1+1|").unwrap().tables.remove(0);
        table.align[0] = Some("right;\" onclick=\"evil");
        let html = table.html();
        assert!(!html.contains("onclick") && !html.contains("evil"));
        assert_eq!(html.matches("text-align:left").count(), 2);
        let untrusted = render(&table).unwrap();
        table.align[0] = None;
        assert_eq!(untrusted.into_raw(), render(&table).unwrap().into_raw());
    }
}
