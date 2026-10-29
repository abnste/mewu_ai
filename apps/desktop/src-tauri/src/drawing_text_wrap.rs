// SPDX-License-Identifier: MPL-2.0
//! Literal text wrapping shares the exporter's actual font resolver and glyph
//! advances. Keep this cache local to one text object; only font bytes are shared.
use super::{escape_text, prefer_matching_text_fallback, text_fonts, FONT_FAMILY};
use mewu_core::DrawingTextStyle;
use resvg::usvg;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

const WIDTH_EPSILON: f64 = 0.001;

/// No width preserves the legacy LF-only layout without loading fonts. A box
/// wraps at grapheme boundaries, never scales the font or limits the line count.
pub(crate) fn literal_text_lines(
    text: &str,
    size: f64,
    style: Option<DrawingTextStyle>,
    width: Option<f64>,
) -> Result<Vec<String>, String> {
    // Video literal text already permits tabs; ordinary Drawing's existing
    // validator still rejects them. Both presentations use four literal spaces.
    let text = text.replace('\t', "    ");
    let Some(width) = width else {
        return Ok(text.split('\n').map(str::to_owned).collect());
    };
    if !width.is_finite() || width <= 0. || !size.is_finite() || size <= 0. {
        return Err("文字框宽度或字号无效".into());
    }
    let mut measurer = TextMeasurer::new(size, style.is_some_and(|style| style.bold))?;
    wrap_lines(&text, width, |text| measurer.width(text))
}

fn wrap_lines(
    text: &str,
    width: f64,
    mut measure: impl FnMut(&str) -> Result<f64, String>,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut ends: Vec<_> = paragraph
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .collect();
        ends.push(paragraph.len());
        let count = ends.len() - 1;
        let mut start = 0;
        while start < count {
            // Exponential search locates this line's end before binary search.
            // Unlike measuring every growing prefix (quadratic shaping work),
            // measured text is O(n log n), including very narrow boxes. An
            // indivisible over-wide grapheme always occupies one whole line.
            let mut fit = start + 1;
            let mut step = 1usize;
            let mut limit = count;
            while fit < count {
                let probe = start.saturating_add(step.saturating_mul(2)).min(count);
                if measure(&paragraph[ends[start]..ends[probe]])? > width + WIDTH_EPSILON {
                    limit = probe;
                    break;
                }
                fit = probe;
                step = step.saturating_mul(2);
            }
            while fit + 1 < limit {
                let middle = fit + (limit - fit) / 2;
                if measure(&paragraph[ends[start]..ends[middle]])? <= width + WIDTH_EPSILON {
                    fit = middle;
                } else {
                    limit = middle;
                }
            }
            result.push(paragraph[ends[start]..ends[fit]].to_owned());
            start = fit;
        }
    }
    Ok(result)
}

pub(super) struct TextMeasurer {
    options: usvg::Options<'static>,
    size: f64,
    bold: bool,
    widths: HashMap<String, f64>,
    #[cfg(test)]
    shaped: usize,
}

impl TextMeasurer {
    pub(super) fn new(size: f64, bold: bool) -> Result<Self, String> {
        let mut options = usvg::Options::default();
        options.image_href_resolver = usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        };
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
            options.font_family = super::macos_default_family(&options.fontdb)?;
        }
        Ok(Self {
            options,
            size,
            bold,
            widths: HashMap::new(),
            #[cfg(test)]
            shaped: 0,
        })
    }

    pub(super) fn width(&mut self, text: &str) -> Result<f64, String> {
        if text.is_empty() {
            return Ok(0.);
        }
        if let Some(width) = self.widths.get(text) {
            return Ok(*width);
        }
        let size = self.size;
        // Text::bounding_box is the SVG glyph-metrics/advance box, including
        // preserved spaces, not flattened paths' tight ink bounds. Oblique is
        // applied per rendered line after wrapping, exactly as in the editor.
        let xml = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"6000\"><text id=\"measure\" x=\"0\" y=\"{size}\" font-family=\"{FONT_FAMILY}\" font-size=\"{size}\" font-weight=\"{}\" font-style=\"normal\" xml:space=\"preserve\">{}</text></svg>", if self.bold { 700 } else { 400 }, escape_text(text));
        let tree = usvg::Tree::from_str(&xml, &self.options).map_err(|_| "无法测量绘制文字")?;
        let width = match tree.node_by_id("measure") {
            Some(usvg::Node::Text(text)) => f64::from(text.bounding_box().width()),
            _ => return Err("无法测量绘制文字".into()),
        };
        if !width.is_finite() || width < 0. {
            return Err("绘制文字宽度无效".into());
        }
        #[cfg(test)]
        {
            self.shaped += 1;
        }
        self.widths.insert(text.to_owned(), width);
        Ok(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_lines_preserve_lf_empty_lines_spaces_and_expand_video_tabs() {
        assert_eq!(
            literal_text_lines("a\n\n b \t\n", 20., None, None).unwrap(),
            ["a", "", " b     ", ""]
        );
        assert_eq!(
            literal_text_lines("a\n\nb\n", 20., None, Some(1.)).unwrap(),
            ["a", "", "b", ""]
        );
        for width in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(literal_text_lines("x", 20., None, Some(width)).is_err());
        }
    }

    #[test]
    fn wrapping_uses_actual_variable_width_fonts_and_never_splits_graphemes() {
        let mut measure = TextMeasurer::new(28., false).unwrap();
        let narrow = measure.width("iiii").unwrap();
        assert!(measure.width("WWWW").unwrap() > narrow * 2.);
        assert_eq!(
            literal_text_lines("iiii", 28., None, Some(narrow)).unwrap(),
            ["iiii"]
        );
        assert!(
            literal_text_lines("WWWW", 28., None, Some(narrow))
                .unwrap()
                .len()
                > 1
        );
        let text = "A\u{301}👩\u{200d}👩\u{200d}👧\u{200d}👦🇨🇳中文";
        let lines = literal_text_lines(text, 28., None, Some(1.)).unwrap();
        assert_eq!(
            lines,
            text.graphemes(true).map(str::to_owned).collect::<Vec<_>>()
        );
        assert_eq!(lines.concat(), text);
    }

    #[test]
    fn line_measurement_is_bounded_and_identical_short_lines_share_one_shape() {
        let text = "W".repeat(2000);
        for width in [1., 2., 16., 127., 1024., 4000.] {
            let mut measured = 0;
            let lines = wrap_lines(&text, width, |part| {
                measured += part.len();
                Ok(part.len() as f64)
            })
            .unwrap();
            assert_eq!(lines.concat(), text);
            assert!(
                measured <= text.len() * 32,
                "quadratic prefix shaping: {width} {measured}"
            );
        }
        let mut measure = TextMeasurer::new(20., false).unwrap();
        let lines = wrap_lines(&text, 1., |part| measure.width(part)).unwrap();
        assert_eq!(lines.len(), 2000);
        assert_eq!(
            measure.shaped, 1,
            "same two-grapheme probe is only shaped once"
        );
        assert!(std::sync::Arc::ptr_eq(
            &measure.options.fontdb,
            &text_fonts(false).unwrap()
        ));
    }
}
