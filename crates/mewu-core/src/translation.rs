// SPDX-License-Identifier: MPL-2.0
//! Host-owned translation documents and matching immutable rendered pixels.
use crate::{Asset, AssetKind, CoreError, Region, TranslationDocument};
use std::collections::HashSet;

type Result<T> = std::result::Result<T, CoreError>;
fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}

pub fn validate_translation_document(document: &TranslationDocument) -> Result<()> {
    if document.version != 1
        || !matches!(
            document.target_language.as_str(),
            "zh-Hans" | "en" | "ja" | "ko" | "de" | "fr" | "es"
        )
        || document.width == 0
        || document.height == 0
        || document.width > 16384
        || document.height > 16384
        || u64::from(document.width) * u64::from(document.height) > 32 * 1024 * 1024
        || document.lines.is_empty()
        || document.lines.len() > 128
    {
        return Err(invalid("翻译文档格式、语言或尺寸无效"));
    }
    let angle = document.text_angle.unwrap_or(0.);
    if !angle.is_finite() || !(-360. ..=360.).contains(&angle) {
        return Err(invalid("翻译文字角度无效"));
    }
    // Same rotated coordinate frame as Windows OCR, around the crop center.
    let (sine, cosine) = angle.to_radians().sin_cos();
    let (width, height) = (document.width as f64, document.height as f64);
    let extent_x = cosine.abs() * width + sine.abs() * height;
    let extent_y = sine.abs() * width + cosine.abs() * height;
    let (left, top, right, bottom) = (
        (width - extent_x) / 2. - 1.,
        (height - extent_y) / 2. - 1.,
        (width + extent_x) / 2. + 1.,
        (height + extent_y) / 2. + 1.,
    );
    let mut ids = HashSet::new();
    let mut bytes = 0usize;
    for line in &document.lines {
        if line.id.is_empty()
            || line.id.len() > 64
            || !line
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            || !ids.insert(line.id.as_str())
        {
            return Err(invalid("翻译行身份无效或重复"));
        }
        for text in [&line.source, &line.text] {
            if text.trim().is_empty()
                || text.chars().count() > 2048
                || text.chars().any(|c| {
                    (c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
                        || matches!(c, '\u{fffe}' | '\u{ffff}')
                })
            {
                return Err(invalid("翻译行文字为空、过长或含无效字符"));
            }
            bytes = bytes.saturating_add(text.len());
        }
        let b = &line.r#box;
        if ![b.x, b.y, b.width, b.height].iter().all(|v| v.is_finite())
            || b.width <= 0.
            || b.height <= 0.
            || b.x < left
            || b.y < top
            || b.x + b.width > right
            || b.y + b.height > bottom
        {
            return Err(invalid("翻译文字框超出来源范围"));
        }
    }
    if bytes > 128 * 1024 || serde_json::to_vec(document)?.len() > 512 * 1024 {
        return Err(invalid("翻译文字或文档超过长度上限"));
    }
    Ok(())
}

pub(crate) fn validate_region(region: &Region, background: &Asset) -> Result<()> {
    let Some(saved) = &region.translation else {
        return Ok(());
    };
    let source = region.image_override.as_ref().unwrap_or(background);
    let (width, height) = if region.image_override.is_some() {
        (source.width.unwrap_or(0), source.height.unwrap_or(0))
    } else {
        (region.width.round() as u32, region.height.round() as u32)
    };
    validate_translation_document(&saved.document)?;
    let overlay = &saved.overlay;
    if saved.background_id != background.id
        || saved.source_id != source.id
        || saved.drawing_revision != region.drawing_revision
        || (saved.document.width, saved.document.height) != (width, height)
        || overlay.kind != AssetKind::Image
        || overlay.id == source.id
        || overlay.id == background.id
        || uuid::Uuid::parse_str(&overlay.id)
            .map_or(true, |id| id.hyphenated().to_string() != overlay.id)
        || (overlay.width, overlay.height) != (Some(width), Some(height))
        || overlay.origin_x.is_some_and(|value| value != 0)
        || overlay.origin_y.is_some_and(|value| value != 0)
        || overlay.scale_factor.is_some_and(|value| value != 1.)
        || saved.selection.width != width
        || saved.selection.height != height
        || saved.selection.text_angle != saved.document.text_angle
        || saved.selection.language != saved.document.target_language
        || saved.selection.engine != "mewu.translation.layout.v1"
        || saved.selection.lines.len() != saved.document.lines.len()
    {
        return Err(invalid("翻译图层与当前选区或布局不一致"));
    }
    crate::validate_ocr_document(&saved.selection)?;
    // Preserve exact translated content, including reading order. Wrapped
    // visual rows are words inside the original logical line, not new lines.
    for (line, selected) in saved.document.lines.iter().zip(&saved.selection.lines) {
        let normalized = line.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if selected.text != normalized {
            return Err(invalid("翻译选字内容与译文不一致"));
        }
        let expected: String = selected
            .text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let actual: String = selected
            .words
            .iter()
            .flat_map(|word| word.text.chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        if actual != expected {
            return Err(invalid("翻译选字内容不完整"));
        }
    }
    Ok(())
}
