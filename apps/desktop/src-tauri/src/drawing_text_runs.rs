// SPDX-License-Identifier: MPL-2.0
//! Literal spans use the exporter's font resolver and grapheme wrap policy.
use super::text_wrap::TextMeasurer;
use mewu_core::{DrawingTextRun, DrawingTextStyle};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn text_runs_svg(
    text: &str,
    runs: &[DrawingTextRun],
    color: &str,
    style: Option<DrawingTextStyle>,
    size: f64,
    width: Option<f64>,
    x: f64,
    top: f64,
) -> Result<(String, usize), String> {
    mewu_core::validate_text_runs(text, Some(runs)).map_err(|e| e.to_string())?;
    let runs: Vec<_> = runs
        .iter()
        .map(|run| DrawingTextRun {
            text: run.text.replace('\t', "    "),
            color: Some(run.color.clone().unwrap_or_else(|| color.into())),
            text_style: Some(run.text_style.or(style).unwrap_or_default()),
        })
        .collect();
    let mut full = String::new();
    let mut starts = vec![];
    for run in &runs {
        starts.push(full.len());
        full.push_str(&run.text);
    }
    let slice = |from: usize, to: usize| -> Vec<DrawingTextRun> {
        let first = starts
            .partition_point(|start| *start <= from)
            .saturating_sub(1);
        let mut result = vec![];
        for (i, run) in runs.iter().enumerate().skip(first) {
            if starts[i] >= to {
                break;
            }
            let a = from.saturating_sub(starts[i]);
            let b = (to - starts[i]).min(run.text.len());
            if a < b {
                let mut part = run.clone();
                part.text = run.text[a..b].into();
                result.push(part);
            }
        }
        result
    };
    let mut normal = TextMeasurer::new(size, false)?;
    let mut bold = if runs.iter().any(|r| r.text_style.is_some_and(|s| s.bold)) {
        Some(TextMeasurer::new(size, true)?)
    } else {
        None
    };
    let mut advance = |run: &DrawingTextRun| {
        if run.text_style.is_some_and(|s| s.bold) {
            bold.as_mut().expect("bold resolver").width(&run.text)
        } else {
            normal.width(&run.text)
        }
    };
    let mut ranges = vec![];
    let mut offset = 0usize;
    for paragraph in full.split('\n') {
        if let Some(limit) = width.filter(|_| !paragraph.is_empty()) {
            if !limit.is_finite() || limit <= 0. {
                return Err("文字框宽度无效".into());
            }
            let mut ends: Vec<_> = paragraph.grapheme_indices(true).map(|(i, _)| i).collect();
            ends.push(paragraph.len());
            let count = ends.len() - 1;
            let mut start = 0;
            while start < count {
                let mut measure = |end| -> Result<f64, String> {
                    slice(offset + ends[start], offset + ends[end])
                        .iter()
                        .try_fold(0., |sum, run| Ok(sum + advance(run)?))
                };
                let mut fit = start + 1;
                let mut step = 1usize;
                let mut bound = count;
                while fit < count {
                    let probe = start.saturating_add(step.saturating_mul(2)).min(count);
                    if measure(probe)? > limit + 0.001 {
                        bound = probe;
                        break;
                    }
                    fit = probe;
                    step = step.saturating_mul(2);
                }
                while fit + 1 < bound {
                    let middle = fit + (bound - fit) / 2;
                    if measure(middle)? <= limit + 0.001 {
                        fit = middle
                    } else {
                        bound = middle
                    }
                }
                ranges.push((offset + ends[start], offset + ends[fit]));
                start = fit;
            }
        } else {
            ranges.push((offset, offset + paragraph.len()));
        }
        offset += paragraph.len() + 1;
    }
    let mut xml = String::new();
    for (index, (from, to)) in ranges.iter().enumerate() {
        let mut at = x;
        let baseline = top + size * (1. + index as f64 * 1.25);
        for run in slice(*from, *to) {
            xml.push_str(&super::styled_text_line_svg(
                &run.text,
                run.color.as_deref().expect("literal color"),
                size,
                at,
                baseline,
                run.text_style.unwrap_or_default(),
            ));
            at += advance(&run)?;
        }
    }
    Ok((xml, ranges.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    use mewu_core::{Drawing, Region};

    fn runs() -> Vec<DrawingTextRun> {
        vec![
            DrawingTextRun {
                text: "Red ".into(),
                color: Some("#FF0000".into()),
                text_style: None,
            },
            DrawingTextRun {
                text: "Blue".into(),
                color: Some("#0000FF".into()),
                text_style: Some(DrawingTextStyle {
                    bold: true,
                    underline: true,
                    ..Default::default()
                }),
            },
        ]
    }

    #[test]
    fn literal_fragments_export_real_red_blue_pixels_and_preserve_a_uniform_legacy_image() {
        let mut drawing: Drawing=serde_json::from_value(serde_json::json!({"id":uuid::Uuid::new_v4().to_string(),"kind":"text","color":"#FF0000","strokeWidth":1,"fontSize":48,"text":"Red Blue","points":[{"x":20,"y":20}]})).unwrap();
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(500, 160, Rgba([255, 255, 255, 255])));
        let render = |drawing: Drawing| {
            super::super::crop_region(
                &base,
                &Region {
                    width: 500.,
                    height: 160.,
                    drawings: vec![drawing],
                    ..Default::default()
                },
            )
            .unwrap()
            .into_rgba8()
        };
        let legacy = render(drawing.clone());
        drawing.text_runs = Some(vec![DrawingTextRun {
            text: "Red Blue".into(),
            color: None,
            text_style: None,
        }]);
        assert_eq!(legacy, render(drawing.clone()));
        drawing.text_runs = Some(["Red ", "Blue"].into_iter().map(|text| DrawingTextRun { text: text.into(), color: None, text_style: None }).collect());
        assert_eq!(legacy, render(drawing.clone()), "fragment boundaries must retain the word space");
        drawing.text_runs = Some(runs());
        let actual = render(drawing.clone());
        let count = |blue: bool| {
            actual
                .pixels()
                .filter(|p| {
                    if blue {
                        u16::from(p[2]) > u16::from(p[0]) + 40
                            && u16::from(p[2]) > u16::from(p[1]) + 40
                    } else {
                        u16::from(p[0]) > u16::from(p[1]) + 40
                            && u16::from(p[0]) > u16::from(p[2]) + 40
                    }
                })
                .count()
        };
        assert!(count(false) > 100);
        assert!(count(true) > 100);
        assert_ne!(legacy, actual);
        drawing.text_runs.as_mut().unwrap()[0].text = "bad".into();
        assert!(super::super::drawing_svg(&[drawing], 500, 160).is_err());
    }

    #[test]
    fn video_literal_fragments_use_same_raster_path_and_keep_content_identity() {
        let content = mewu_core::VideoTextContent {
            version: 1,
            text: "Red Blue".into(),
            color: "#FF0000".into(),
            font_size: 48.,
            text_style: None,
            text_box: None,
            text_runs: Some(runs()),
        };
        let raster = crate::video_annotation_raster::render_text(content.clone()).unwrap();
        assert_eq!(raster.content(), &content);
        assert_eq!(raster.renderer().style_version, 5);
        let png = image::load_from_memory(raster.png()).unwrap().into_rgba8();
        assert!(png
            .pixels()
            .any(|p| p[3] > 0 && p[0] > p[2].saturating_add(40)));
        assert!(png
            .pixels()
            .any(|p| p[3] > 0 && p[2] > p[0].saturating_add(40)));
    }

    #[test]
    fn literal_fragments_wrap_without_splitting_emoji_or_losing_escaped_content() {
        let runs = vec![DrawingTextRun {
            text: "中 <&>\né 👨‍👩‍👧".into(),
            color: None,
            text_style: None,
        }];
        let (xml, lines) = text_runs_svg(
            &runs[0].text,
            &runs,
            "#223344",
            None,
            24.,
            Some(50.),
            4.,
            4.,
        )
        .unwrap();
        assert!(lines >= 3);
        assert!(xml.contains("&lt;"));
        assert!(!xml.contains("<&>"));
        assert!(xml.contains("é"));
        assert!(xml.contains("👨‍👩‍👧"));
    }
}
