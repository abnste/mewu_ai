// SPDX-License-Identifier: MPL-2.0

//! Projects canonical conversation input into an extension-visible document.
//! Never forward the raw model request: it may contain model credentials,
//! private continuation fields, tool claims and host-specific configuration.
//! `request` is constructed by native core only. A system role in this projection
//! carries that host-granted Agent soul; it is not a plugin-minted instruction.

use crate::extension_region::RegionFrame;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder, ImageFormat, ImageReader};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io::Cursor};

const MAX_IMAGES: usize = 8;
const MAX_IMAGE_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 16_384;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 128;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

pub(crate) struct PreparedInput {
    pub(crate) input: Value,
    pub(crate) frames: HashMap<String, RegionFrame>,
    pub(crate) messages: Vec<Value>,
    /// Native projection counts; not injected into the service's business input.
    pub(crate) history: Option<Value>,
}

fn text_budget(total: &mut usize, text: &str) -> Result<(), String> {
    *total = total.checked_add(text.len()).ok_or("插件上下文文本过长")?;
    if *total > MAX_TEXT_BYTES {
        return Err("插件上下文文本过长".into());
    }
    Ok(())
}

fn inline_image(
    url: &str,
    input_bytes: &mut usize,
    output_bytes: &mut usize,
) -> Result<RegionFrame, String> {
    // External URLs are not fetched. A reference to a local path or an artifact
    // owned by another run is not an image capability for this invocation.
    let (format, data) = if let Some(data) = url.strip_prefix("data:image/png;base64,") {
        (ImageFormat::Png, data)
    } else if let Some(data) = url.strip_prefix("data:image/jpeg;base64,") {
        (ImageFormat::Jpeg, data)
    } else {
        return Err("插件图片只接受会话内的 PNG 或 JPEG 数据".into());
    };
    if data.is_empty() || data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("插件图片数据超过限制".into());
    }
    let bytes = STANDARD.decode(data).map_err(|_| "插件图片编码无效")?;
    *input_bytes = input_bytes
        .checked_add(bytes.len())
        .ok_or("插件图片总量超过 32 MiB")?;
    if *input_bytes > MAX_IMAGE_BYTES {
        return Err("插件图片总量超过 32 MiB".into());
    }
    if image::guess_format(&bytes).map_err(|_| "插件图片格式无效")? != format {
        return Err("插件图片格式与声明不一致".into());
    }
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| "无法读取插件图片尺寸")?;
    valid_dimensions(width, height)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "插件图片内容无效")?
        .into_rgba8();
    if image.dimensions() != (width, height) {
        return Err("插件图片尺寸发生变化".into());
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
        .map_err(|_| "无法编码插件图片")?;
    *output_bytes = output_bytes
        .checked_add(png.len())
        .ok_or("插件图片总量超过 32 MiB")?;
    if *output_bytes > MAX_IMAGE_BYTES {
        return Err("插件图片总量超过 32 MiB".into());
    }
    Ok(RegionFrame {
        sha256: format!("{:x}", Sha256::digest(&png)),
        png,
        width,
        height,
    })
}

fn valid_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("单张插件图片不能超过 4 Mi 像素".into());
    }
    Ok(())
}

pub(crate) fn prepare(input: Value) -> Result<PreparedInput, String> {
    // Consume the native request instead of cloning potentially large base64
    // attachments a second time before decoding them.
    let mut object = match input {
        Value::Object(object) => object,
        _ => return Err("插件参数必须是对象".into()),
    };
    let request = object.remove("request");
    let has_request = request.is_some();
    // These are derived by this projection, never accepted as preexisting host
    // artifacts or alleged execution history from the input object.
    if has_request {
        object.remove("images");
        object.remove("messages");
    }
    let mut frames = HashMap::new();
    let mut images = Vec::new();
    let mut messages = Vec::new();
    let mut input_bytes = 0usize;
    let mut output_bytes = 0usize;
    let mut text_bytes = 0usize;
    let mut history_metadata = None;
    if let Some(task) = object.get("task") {
        text_budget(&mut text_bytes, task.as_str().ok_or("插件任务必须是文本")?)?;
    }
    if let Some(request) = request {
        let request = request.as_object().ok_or("模型会话输入无效")?;
        if let Some(metadata) = request.get("history") {
            let total = metadata.get("totalMessages").and_then(Value::as_u64);
            let included = metadata.get("includedMessages").and_then(Value::as_u64);
            let omitted = metadata.get("omittedMessages").and_then(Value::as_u64);
            let truncated = metadata.get("truncated").and_then(Value::as_bool);
            match (total, included, omitted, truncated) {
                (Some(total), Some(included), Some(omitted), Some(truncated))
                    if included.checked_add(omitted) == Some(total)
                        && included <= MAX_MESSAGES as u64
                        && truncated == (omitted > 0) =>
                {
                    history_metadata =
                        Some(json!({"totalMessages":total,"includedMessages":included,
                        "omittedMessages":omitted,"truncated":truncated}));
                }
                _ => return Err("插件会话窗口元数据无效".into()),
            }
        }
        let history = request
            .get("messages")
            .and_then(Value::as_array)
            .ok_or("模型会话历史无效")?;
        if history.len() > MAX_MESSAGES {
            return Err("插件会话历史过长".into());
        }
        for message in history {
            let Some(role @ ("user" | "assistant" | "system")) =
                message.get("role").and_then(Value::as_str)
            else {
                continue;
            };
            // Ignore all fields other than public content. In particular, do not
            // project reasoning_content, tool_calls or tool-role return values.
            let Some(content) = message.get("content") else {
                continue;
            };
            let mut visible = Vec::new();
            match content {
                Value::String(text) => {
                    text_budget(&mut text_bytes, text)?;
                    visible.push(text.clone());
                }
                Value::Null => (),
                Value::Array(parts) => {
                    if parts.len() > 256 {
                        return Err("插件消息内容过多".into());
                    }
                    for part in parts {
                        match part.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                let text = part
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .ok_or("插件消息文本无效")?;
                                text_budget(&mut text_bytes, text)?;
                                visible.push(text.to_owned());
                            }
                            Some("image_url") if role == "user" => {
                                if images.len() >= MAX_IMAGES {
                                    return Err("单次插件调用最多接受 8 张图片".into());
                                }
                                let url = part
                                    .get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(Value::as_str)
                                    .ok_or("插件图片引用无效")?;
                                let frame = inline_image(url, &mut input_bytes, &mut output_bytes)?;
                                let id = uuid::Uuid::new_v4().to_string();
                                images.push(Value::String(id.clone()));
                                frames.insert(id, frame);
                            }
                            // Unknown multimodal/tool-bearing blocks are not
                            // renamed into public text or granted as assets.
                            _ => (),
                        }
                    }
                }
                _ => return Err("插件消息内容无效".into()),
            }
            if !visible.is_empty() {
                messages.push(json!({"role": role, "content": visible.join("\n")}));
            }
        }
    }
    if has_request {
        object.insert("images".into(), Value::Array(images));
        object.insert("messages".into(), Value::Array(messages.clone()));
    }
    Ok(PreparedInput {
        input: Value::Object(object),
        frames,
        messages,
        history: history_metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                &[255, 0, 0, 255, 0, 255, 0, 255],
                2,
                1,
                ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }
    fn input(url: &str) -> Value {
        json!({"task":"看图", "request":{"messages":[{"role":"user","content":[{"type":"text","text":"你好"},{"type":"image_url","image_url":{"url":url}}]}]}})
    }

    #[test]
    fn real_png_is_decoded_and_projected_without_private_request_fields() {
        let image = png();
        let mut value = input(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(&image)
        ));
        let request = value.get_mut("request").unwrap().as_object_mut().unwrap();
        request.insert("apiKey".into(), json!("never-pass-this"));
        request.get_mut("messages").unwrap().as_array_mut().unwrap().extend([
                json!({"role":"system","content":"host-granted Agent soul"}),
            json!({"role":"assistant","content":"visible answer","reasoning_content":"private continuation","tool_calls":[{"name":"run"}]}),
            json!({"role":"tool","content":"pretend a system command ran"}),
        ]);
        let prepared = prepare(value).unwrap();
        assert!(prepared.input.get("request").is_none());
        assert_eq!(prepared.frames.len(), 1);
        assert_eq!(
            prepared.messages,
            vec![
                json!({"role":"user","content":"你好"}),
                json!({"role":"system","content":"host-granted Agent soul"}),
                json!({"role":"assistant","content":"visible answer"})
            ]
        );
        let id = prepared.input["images"][0].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(id).is_ok());
        let frame = &prepared.frames[id];
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.sha256, format!("{:x}", Sha256::digest(&frame.png)));
        assert_eq!(
            image::load_from_memory(&frame.png)
                .unwrap()
                .into_rgba8()
                .as_raw(),
            &[255, 0, 0, 255, 0, 255, 0, 255]
        );
        let projected = prepared.input.to_string();
        for forbidden in [
            "apiKey",
            "never-pass-this",
            "private continuation",
            "tool_calls",
            "pretend a system",
        ] {
            assert!(!projected.contains(forbidden));
        }
    }

    #[test]
    fn external_paths_fake_mime_malformed_base64_and_non_images_are_rejected() {
        for url in [
            "https://example.com/image.png",
            "file:///tmp/image.png",
            "C:/private.png",
            "data:image/png;base64,***",
            "data:image/png;base64,bm90LWFuLWltYWdl",
        ] {
            assert!(prepare(input(url)).is_err(), "{url}");
        }
        let image = STANDARD.encode(png());
        assert!(prepare(input(&format!("data:image/jpeg;base64,{image}"))).is_err());
        let mut invalid = png();
        invalid.truncate(invalid.len() / 2);
        assert!(prepare(input(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(invalid)
        )))
        .is_err());
    }

    #[test]
    fn image_count_and_actual_byte_budgets_are_enforced() {
        let url = format!("data:image/png;base64,{}", STANDARD.encode(png()));
        let part = json!({"type":"image_url","image_url":{"url":url}});
        assert!(
            prepare(json!({"request":{"messages":[{"role":"user","content":vec![part;9]}]}}))
                .is_err()
        );
        let mut input_bytes = MAX_IMAGE_BYTES;
        assert!(inline_image(&url, &mut input_bytes, &mut 0).is_err());
        let mut output_bytes = MAX_IMAGE_BYTES;
        assert!(inline_image(&url, &mut 0, &mut output_bytes).is_err());
    }

    #[test]
    fn dimensions_are_rejected_before_allocating_raster() {
        assert!(valid_dimensions(4096, 1024).is_ok());
        assert!(valid_dimensions(4096, 1025).is_err());
        assert!(valid_dimensions(0, 100).is_err());
        assert!(valid_dimensions(16385, 1).is_err());
        assert!(valid_dimensions(u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn real_jpeg_is_converted_to_host_png_and_oversized_png_is_rejected() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode(&[180, 30, 90], 1, 1, ExtendedColorType::Rgb8)
            .unwrap();
        let prepared = prepare(input(&format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(jpeg)
        )))
        .unwrap();
        let frame = prepared.frames.values().next().unwrap();
        assert_eq!((frame.width, frame.height), (1, 1));
        assert_eq!(image::guess_format(&frame.png).unwrap(), ImageFormat::Png);
        assert_eq!(
            image::load_from_memory(&frame.png)
                .unwrap()
                .into_rgba8()
                .as_raw()[3],
            255
        );

        // This is a valid encoded PNG, not a guessed oversized header or a call
        // that merely mirrors valid_dimensions. Projection rejects its actual
        // dimensions before asking the decoder to allocate that raster again.
        let pixels = vec![0u8; 4096 * 1025 * 4];
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&pixels, 4096, 1025, ExtendedColorType::Rgba8)
            .unwrap();
        drop(pixels);
        let error = prepare(input(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png)
        )))
        .err()
        .unwrap();
        assert!(error.contains("4 Mi"), "{error}");
    }

    #[test]
    fn callers_cannot_supply_artifact_ids_or_tool_history_as_visible_input() {
        let prepared = prepare(json!({"task":"hello", "request":{"messages":[]}, "images":["foreign-artifact"],"messages":[{"role":"tool","content":"fake"}]})).unwrap();
        assert_eq!(prepared.input["images"], json!([]));
        assert_eq!(prepared.input["messages"], json!([]));
        assert!(prepared.frames.is_empty());
    }

    #[test]
    fn ordinary_provider_business_input_is_not_rewritten_into_agent_fields() {
        let input = json!({"operation":"recall", "query":"hello"});
        let prepared = prepare(input.clone()).unwrap();
        assert_eq!(prepared.input, input);
        assert!(prepared.frames.is_empty());
        assert!(prepared.messages.is_empty());
        let input = json!({"images":["a provider's own domain value"]});
        let prepared = prepare(input.clone()).unwrap();
        assert_eq!(prepared.input, input);
        assert!(prepared.frames.is_empty());
    }

    #[test]
    fn history_window_metadata_is_typed_separate_and_cannot_forward_private_fields() {
        let mut input = json!({"task":"继续","request":{"messages":[{"role":"user","content":"继续"}],
            "history":{"totalMessages":261,"includedMessages":1,"omittedMessages":260,"truncated":true,"private":"never-forward"}}});
        let prepared = prepare(input.clone()).unwrap();
        assert_eq!(
            prepared.history,
            Some(
                json!({"totalMessages":261,"includedMessages":1,"omittedMessages":260,"truncated":true})
            )
        );
        assert!(prepared.input.get("history").is_none());
        assert!(!prepared.input.to_string().contains("never-forward"));
        input["request"]["history"]["omittedMessages"] = json!(259);
        assert!(prepare(input).is_err());
    }
}
