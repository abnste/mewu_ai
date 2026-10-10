// SPDX-License-Identifier: MPL-2.0
// Adapted from the passed private probe without relaxing SVG/glyph/pixel checks.
use super::{protocol::*, MAX_INPUT_CHARS, MAX_PIXELS, MAX_PNG, MAX_SIDE, MAX_SVG};
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};
use ratex_types::{color::Color, display_item::DisplayItem, math_style::MathStyle};
use resvg::{tiny_skia, usvg};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::Write, path::Path, time::Instant};

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "output_io")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "output_io".into())
}

struct CheckedSvg {
    view_box: [f64; 4],
    width: u32,
    height: u32,
    paths: usize,
    missing: Vec<u32>,
}

// This is validation of locked renderer output, not a parser for model-authored SVG.
// No text, raster image, CSS, external references or local resource loading is accepted.
fn inspect_svg(svg: &str) -> Result<CheckedSvg, String> {
    if svg.len() > MAX_SVG {
        return Err("svg_budget".into());
    }
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: 20_000,
        ..Default::default()
    };
    let document =
        roxmltree::Document::parse_with_options(svg, options).map_err(|_| "invalid_svg")?;
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return Err("invalid_svg".into());
    }
    let values = root
        .attribute("viewBox")
        .ok_or("invalid_viewbox")?
        .split_ascii_whitespace()
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid_viewbox")?;
    if values.len() != 4
        || values.iter().any(|v| !v.is_finite())
        || values[0] != 0.0
        || values[1] != 0.0
        || values[2] <= 0.0
        || values[3] <= 0.0
    {
        return Err("invalid_viewbox".into());
    }
    let width = values[2].ceil() as u32;
    let height = values[3].ceil() as u32;
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("pixel_budget".into());
    }
    // RaTeX emits pt but its coordinates are the specified em_px. Use 72dpi
    // deliberately: one exporter unit becomes one pixel, no hidden 96/72 scale.
    for (attribute, expected) in [("width", values[2]), ("height", values[3])] {
        let actual = root
            .attribute(attribute)
            .and_then(|v| v.strip_suffix("pt"))
            .and_then(|v| v.parse::<f64>().ok())
            .ok_or("invalid_size")?;
        if (actual - expected).abs() > 0.000_002 {
            return Err("invalid_size".into());
        }
    }
    let mut paths = 0;
    let mut missing = Vec::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        let tag = node.tag_name().name();
        if node.tag_name().namespace() != Some("http://www.w3.org/2000/svg") {
            return Err("unsupported_svg".into());
        }
        if tag == "text" {
            missing.extend(
                node.descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .flat_map(str::chars)
                    .filter(|c| !c.is_whitespace())
                    .map(u32::from),
            );
            // Whitespace may legitimately lack an outline. It has no ink and
            // resvg is compiled without text; non-whitespace is rejected below.
        } else if !matches!(tag, "svg" | "path" | "rect" | "line") {
            return Err("unsupported_svg".into());
        }
        if tag == "path" {
            paths += 1;
        }
        for attribute in node.attributes() {
            // The pinned renderer's fallback text has this fixed attribute.
            // Recognize it only here, then continue validating the whole SVG:
            // missing glyphs must never mask CSS, links or unsupported nodes.
            let fallback_baseline = tag == "text"
                && attribute.name() == "dominant-baseline"
                && attribute.value() == "alphabetic";
            let supported_attribute = matches!(
                attribute.name(),
                "viewBox"
                    | "width"
                    | "height"
                    | "x"
                    | "y"
                    | "x1"
                    | "x2"
                    | "y1"
                    | "y2"
                    | "d"
                    | "fill"
                    | "stroke"
                    | "stroke-width"
                    | "stroke-dasharray"
                    | "fill-rule"
                    | "fill-opacity"
                    | "stroke-opacity"
                    | "opacity"
                    | "stroke-linecap"
                    | "stroke-linejoin"
                    | "font-family"
                    | "font-size"
                    | "font-weight"
                    | "font-style"
            );
            if attribute.namespace().is_some() || !(supported_attribute || fallback_baseline) {
                return Err("unsupported_svg_attribute".into());
            }
            if attribute.value().contains("url(")
                || attribute.value().contains("NaN")
                || attribute.value().contains("inf")
            {
                return Err("unsupported_svg_attribute".into());
            }
        }
    }
    Ok(CheckedSvg {
        view_box: [values[0], values[1], values[2], values[3]],
        width,
        height,
        paths,
        missing,
    })
}

pub(super) fn execute(config: &Config) -> RenderResult {
    let clock = Instant::now();
    let mut result = RenderResult {
        request_id: config.request_id.clone(),
        status: "pending".into(),
        elapsed_ms: 0,
        accepted_input_chars: 0,
        metrics: None,
        missing_glyphs: Vec::new(),
    };
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(config, &mut result)));
    result.status = match outcome {
        Ok(Ok(())) => "rendered".into(),
        Ok(Err(error)) => error,
        Err(_) => "renderer_panic".into(),
    };
    result.elapsed_ms = elapsed_millis(clock.elapsed());
    result
}

fn render(config: &Config, result: &mut RenderResult) -> Result<(), String> {
    config.input.validate()?;
    let length = config.input.tex.chars().count();
    if length == 0
        || length > MAX_INPUT_CHARS
        || config
            .input
            .tex
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("input_budget".into());
    }
    result.accepted_input_chars = length;
    let ast = parse(&config.input.tex).map_err(|_| "parse_error")?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::from_hex(&config.input.color).ok_or("formula_style_invalid")?);
    let list = to_display_list(&layout(&ast, &options));
    if list.items.len() > 8192
        || !list.width.is_finite()
        || !list.height.is_finite()
        || !list.depth.is_finite()
    {
        return Err("layout_budget".into());
    }
    let glyphs: Vec<_> = list
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::GlyphPath { char_code, .. } => Some(*char_code),
            _ => None,
        })
        .collect();
    let svg = render_to_svg_with_color_syntax(
        &list,
        &SvgOptions {
            font_size: config.input.font_size,
            padding: 10.0,
            stroke_width: 1.5,
            embed_glyphs: true,
            font_dir: config.font_directory.to_string_lossy().into_owned(),
        },
        SvgColorSyntax::Rgb,
    );
    if svg.len() > MAX_SVG {
        return Err("svg_budget".into());
    }
    // Production never publishes SVG or accepts model-authored SVG. The PNG is
    // the only persisted layout and is decoded again by the parent process.
    let checked = inspect_svg(&svg)?;
    result.missing_glyphs = checked.missing.clone();
    if !checked.missing.is_empty() {
        return Err("missing_glyph".into());
    }
    let options = usvg::Options {
        dpi: 72.0,
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options).map_err(|_| "resvg_parse")?;
    if (f64::from(tree.size().width()) - checked.view_box[2]).abs() > 0.01
        || (f64::from(tree.size().height()) - checked.view_box[3]).abs() > 0.01
    {
        return Err("resvg_size_mismatch".into());
    }
    let mut pixmap = tiny_skia::Pixmap::new(checked.width, checked.height).ok_or("allocation")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut ink = [checked.width, checked.height, 0, 0];
    let mut count = 0u64;
    for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        count += 1;
        let x = index as u32 % checked.width;
        let y = index as u32 / checked.width;
        ink[0] = ink[0].min(x);
        ink[1] = ink[1].min(y);
        ink[2] = ink[2].max(x + 1);
        ink[3] = ink[3].max(y + 1);
    }
    if count == 0 {
        return Err("empty_ink".into());
    }
    let png = pixmap.encode_png().map_err(|_| "png_encode")?;
    if png.len() > MAX_PNG {
        return Err("png_budget".into());
    }
    let decoded = tiny_skia::Pixmap::decode_png(&png).map_err(|_| "png_decode")?;
    if decoded.width() != checked.width
        || decoded.height() != checked.height
        || decoded.data() != pixmap.data()
    {
        return Err("png_roundtrip".into());
    }
    let edge = ink[0] == 0 || ink[1] == 0 || ink[2] == checked.width || ink[3] == checked.height;
    result.metrics = Some(Metrics {
        svg_bytes: svg.len(),
        svg_sha256: digest(svg.as_bytes()),
        view_box: checked.view_box,
        width: checked.width,
        height: checked.height,
        glyph_count: glyphs.len(),
        nonspace_glyph_count: glyphs
            .iter()
            .filter(|c| char::from_u32(**c).is_none_or(|v| !v.is_whitespace()))
            .count(),
        path_count: checked.paths,
        missing_glyphs: checked.missing,
        png_bytes: png.len(),
        png_sha256: digest(&png),
        rgba_sha256: digest(pixmap.data()),
        ink_bounds: ink,
        touches_canvas_edge: edge,
    });
    if edge {
        return Err("ink_touches_canvas_edge".into());
    }
    write_new(&config.output_directory.join("layout.png"), &png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_actual_fallback_svg_reports_missing_codepoints() {
        let rare = inspect_svg(include_str!("fixtures/rare-cjk-776c1d3.svg")).unwrap();
        assert_eq!(rare.missing, vec![0x20bb7]);
        assert_eq!(rare.paths, 3);
        let absent = inspect_svg(include_str!("fixtures/missing-font-776c1d3.svg")).unwrap();
        assert_eq!(absent.missing, vec!['x' as u32, '+' as u32, '1' as u32]);
        assert_eq!(absent.paths, 0);
    }

    #[test]
    fn fallback_baseline_is_scoped_to_text_and_exact_pinned_value() {
        for body in [
            r#"<text dominant-baseline="middle">x</text>"#,
            r#"<text dominant-baseline="url(https://example.invalid/a)">x</text>"#,
            r#"<path d="M1 1L2 2" dominant-baseline="alphabetic"/>"#,
            r#"<text xmlns:bad="urn:bad" bad:dominant-baseline="alphabetic">x</text>"#,
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="30pt" height="30pt" viewBox="0 0 30 30">{body}</svg>"#
            );
            assert_eq!(
                inspect_svg(&svg).err().as_deref(),
                Some("unsupported_svg_attribute")
            );
        }
    }

    #[test]
    fn fallback_missing_glyph_does_not_mask_unsupported_nodes() {
        let fallback = r#"<text dominant-baseline="alphabetic">𠮷</text>"#;
        for unsupported in [
            r#"<image href="https://example.invalid/a.png"/>"#,
            r#"<style>text{fill:red}</style>"#,
            r#"<foreignObject/>"#,
            r#"<script/>"#,
            r#"<path xmlns="urn:unexpected" d="M1 1L2 2"/>"#,
        ] {
            for body in [
                format!("{fallback}{unsupported}"),
                format!("{unsupported}{fallback}"),
            ] {
                let svg = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="30pt" height="30pt" viewBox="0 0 30 30">{body}</svg>"#
                );
                assert_eq!(inspect_svg(&svg).err().as_deref(), Some("unsupported_svg"));
            }
        }
    }

    #[test]
    fn fallback_missing_glyph_does_not_mask_css_links_or_unknown_attributes() {
        for attribute in [
            r#"href="https://example.invalid/a""#,
            r#"style="fill:red""#,
            r#"onload="anything""#,
            r#"fill="url(https://example.invalid/a)""#,
            r#"xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="file:///never""#,
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="30pt" height="30pt" viewBox="0 0 30 30"><text dominant-baseline="alphabetic" {attribute}>x</text></svg>"#
            );
            assert_eq!(
                inspect_svg(&svg).err().as_deref(),
                Some("unsupported_svg_attribute")
            );
        }
    }

    #[test]
    fn rejects_missing_glyphs_and_external_resources_without_resvg() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="30pt" height="30pt" viewBox="0 0 30 30"><text x="1" y="2">汉</text></svg>"#;
        assert_eq!(inspect_svg(svg).unwrap().missing, vec!['汉' as u32]);
        let external = svg.replace(
            "<text x=\"1\" y=\"2\">汉</text>",
            "<image href=\"https://example.invalid/a.png\"/>",
        );
        assert_eq!(
            inspect_svg(&external).err().as_deref(),
            Some("unsupported_svg")
        );
        let oversized = svg.replace("30", "4097");
        assert_eq!(
            inspect_svg(&oversized).err().as_deref(),
            Some("pixel_budget")
        );
    }
    #[test]
    fn rejects_dtd_instead_of_loading_external_entities() {
        assert!(inspect_svg(
            r#"<!DOCTYPE svg SYSTEM "file:///never"><svg xmlns="http://www.w3.org/2000/svg"/>"#
        )
        .is_err());
    }
}
