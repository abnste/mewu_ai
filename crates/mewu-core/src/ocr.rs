// SPDX-License-Identifier: MPL-2.0
//! Recognition is host-owned data, never executable content or a scene command.
use crate::{Asset, CoreError, OcrDocument, OcrTarget, Region, Scene};

type Result<T> = std::result::Result<T, CoreError>;
const MAX_LINES: usize = 512;
const MAX_WORDS: usize = 4096;
const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_DOCUMENT_BYTES: usize = 512 * 1024;

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}

fn valid_text(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// Shared by native recognizers and the persisted-document validator.
/// Empty lines is a valid completed result for an image without text.
pub fn validate_ocr_document(document: &OcrDocument) -> Result<()> {
    for label in [&document.engine, &document.language] {
        if label.trim().is_empty()
            || label.chars().count() > 64
            || label.chars().any(char::is_control)
        {
            return Err(invalid("文字识别引擎或语言无效"));
        }
    }
    if document.width == 0 || document.height == 0 {
        return Err(invalid("文字识别图像尺寸无效"));
    }
    let angle = document.text_angle.unwrap_or(0.);
    if !angle.is_finite() || !(-360. ..=360.).contains(&angle) {
        return Err(invalid("文字识别角度无效"));
    }
    if document.lines.len() > MAX_LINES {
        return Err(invalid("文字识别行数超过上限"));
    }
    // Windows reports rectangles in the rotated image's coordinate space.
    // Bound that space around the original image center; do not clamp its
    // legitimate negative coordinates to the unrotated image rectangle.
    // https://learn.microsoft.com/uwp/api/windows.media.ocr.ocrresult.textangle
    let (sine, cosine) = angle.to_radians().sin_cos();
    let width = document.width as f64;
    let height = document.height as f64;
    let extent_x = cosine.abs() * width + sine.abs() * height;
    let extent_y = sine.abs() * width + cosine.abs() * height;
    let min_x = (width - extent_x) / 2. - 1.;
    let min_y = (height - extent_y) / 2. - 1.;
    let max_x = (width + extent_x) / 2. + 1.;
    let max_y = (height + extent_y) / 2. + 1.;
    let mut words = 0usize;
    let mut text_bytes = 0usize;
    for line in &document.lines {
        if !valid_text(&line.text, 8192) || line.words.is_empty() {
            return Err(invalid("文字识别行内容无效"));
        }
        words = words.saturating_add(line.words.len());
        text_bytes = text_bytes.saturating_add(line.text.len());
        if words > MAX_WORDS {
            return Err(invalid("文字识别词数超过上限"));
        }
        for word in &line.words {
            text_bytes = text_bytes.saturating_add(word.text.len());
            if !valid_text(&word.text, 512)
                || ![word.x, word.y, word.width, word.height]
                    .iter()
                    .all(|value| value.is_finite())
                || word.width <= 0.
                || word.height <= 0.
                || word.x < min_x
                || word.y < min_y
                || word.x + word.width > max_x
                || word.y + word.height > max_y
            {
                return Err(invalid("文字识别词内容或坐标无效"));
            }
        }
        if text_bytes > MAX_TEXT_BYTES {
            return Err(invalid("文字识别文本超过长度上限"));
        }
    }
    if serde_json::to_vec(document)?.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("文字识别文档超过长度上限"));
    }
    Ok(())
}

pub(crate) fn target_region(scene: &Scene, target: &OcrTarget) -> Result<usize> {
    if scene.closed
        || scene.id != target.scene_id
        || scene.background.as_ref().map(|asset| &asset.id) != Some(&target.background_id)
    {
        return Err(CoreError::OcrConflict);
    }
    scene
        .regions
        .iter()
        .position(|region| {
            region.id == target.region_id
                && region.drawing_revision == target.drawing_revision
                && (region.x, region.y, region.width, region.height)
                    == (target.x, target.y, target.width, target.height)
        })
        .ok_or(CoreError::OcrConflict)
}

pub(crate) fn validate_region(region: &Region, background: &Asset) -> Result<()> {
    let Some(result) = &region.ocr else {
        return Ok(());
    };
    let (width, height) = match &region.image_override {
        Some(source) => (source.width.map(f64::from), source.height.map(f64::from)),
        None => (Some(region.width.round()), Some(region.height.round())),
    };
    if result.background_id != background.id
        || result.drawing_revision != region.drawing_revision
        || Some(result.document.width as f64) != width
        || Some(result.document.height as f64) != height
    {
        return Err(invalid("文字识别结果与当前选区不一致"));
    }
    validate_ocr_document(&result.document)
}
