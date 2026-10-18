// SPDX-License-Identifier: MPL-2.0
//! First-party frozen foreground preview. No arbitrary path or imported markup.
use crate::{assets, highlight_foreground, Host};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
static REQUESTS: Semaphore = Semaphore::const_new(8);
static DECODERS: Semaphore = Semaphore::const_new(1);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightPreview {
    background_id: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    data_url: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HighlightTarget {
    region_id: String,
    source_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    raster_drawings: Vec<mewu_core::Drawing>,
}
impl HighlightTarget {
    fn check(&self, region: &mewu_core::Region) -> Result<(), String> {
        if self.region_id != region.id
            || self.x != region.x
            || self.y != region.y
            || self.width != region.width
            || self.height != region.height
            || self.raster_drawings != region.drawings
        {
            return Err("截图图层已变更".into());
        }
        Ok(())
    }
}
fn validate(owner: &str, width: u32, height: u32) -> Result<(), String> {
    if owner != "space" {
        return Err("此窗口不能读取截图像素".into());
    }
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("图片尺寸超过绘制范围".into());
    }
    Ok(())
}
fn encode(source: &DynamicImage, background_id: String) -> Result<HighlightPreview, String> {
    validate("space", source.width(), source.height())?;
    let foreground =
        highlight_foreground::foreground(source, 0, 0, source.width(), source.height())?.straight();
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(foreground)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| "无法生成荧光笔预览")?;
    let bytes = output.into_inner();
    if bytes.len() > MAX_PNG_BYTES {
        return Err("荧光笔预览过大".into());
    }
    Ok(HighlightPreview {
        background_id,
        x: 0,
        y: 0,
        width: source.width(),
        height: source.height(),
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    })
}

#[tauri::command]
pub async fn get_highlight_preview(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    background_id: String,
    target: Option<HighlightTarget>,
) -> Result<HighlightPreview, String> {
    if window.label() != "space" {
        return Err("此窗口不能读取截图像素".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let request = REQUESTS
        .try_acquire()
        .map_err(|_| "荧光笔预览繁忙，请重试")?;
    let decoder = DECODERS.acquire().await.map_err(|_| "荧光笔预览不可用")?;
    if target.as_ref().is_some_and(|value| {
        value.raster_drawings.len() > 4096
            || serde_json::to_vec(&value.raster_drawings)
                .map_or(true, |bytes| bytes.len() > 4 * 1024 * 1024)
    }) {
        return Err("截图图层过大".into());
    }
    let read = |store: &mewu_core::Store| -> Result<(mewu_core::Asset, mewu_core::Region), String> {
        if let Some(target) = &target {
            let value = store
                .highlight_region_for_preview(
                    &scene_id,
                    &target.region_id,
                    &background_id,
                    &target.source_id,
                )
                .map_err(|e| e.to_string())?;
            target.check(&value.1)?;
            Ok(value)
        } else {
            store
                .background_for_preview(&scene_id, &background_id)
                .map(|asset| (asset, mewu_core::Region::default()))
                .map_err(|e| e.to_string())
        }
    };
    let (background, region) = {
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        read(&engine.store).map_err(|e| e.to_string())?
    };
    let expected = (background.clone(), region.clone());
    let has_target = target.is_some();
    let root = host.assets.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _decoder) = (request, decoder);
        let source = region.image_override.as_ref().unwrap_or(&background);
        let path = assets::verified_path(source, &root)?;
        let (width, height) = image::image_dimensions(path).map_err(|_| "无法读取图片尺寸")?;
        validate("space", width, height)?;
        if source.width != Some(width) || source.height != Some(height) {
            return Err("图片尺寸已变更".into());
        }
        if !has_target {
            return encode(&assets::image(&background, &root)?, background.id);
        }
        let (pixels, mapping) = assets::crop_from_parts_with_mapping(&background, &region, &root)?;
        let mut preview = encode(&pixels, source.id.clone())?;
        preview.x = mapping.x;
        preview.y = mapping.y;
        Ok(preview)
    })
    .await
    .map_err(|_| "荧光笔预览未完成")??;
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    if read(&engine.store).map_err(|e| e.to_string())? != expected {
        return Err("截图已变更".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_target_rejects_geometry_content_and_unknown_fields_but_does_not_accept_caller_paths()
    {
        let region = mewu_core::Region {
            id: "region".into(),
            x: 20.,
            y: 10.,
            width: 80.,
            height: 40.,
            ..Default::default()
        };
        let input = serde_json::json!({"regionId":"region","sourceId":"source","x":20,"y":10,"width":80,"height":40,"rasterDrawings":[]});
        let target: HighlightTarget = serde_json::from_value(input.clone()).unwrap();
        assert!(target.check(&region).is_ok());
        for field in ["x", "y", "width", "height"] {
            let mut changed = input.clone();
            changed[field] = serde_json::json!(99);
            assert!(serde_json::from_value::<HighlightTarget>(changed)
                .unwrap()
                .check(&region)
                .is_err());
        }
        let mut changed = input.clone();
        changed["path"] = serde_json::json!("C:/private.png");
        assert!(serde_json::from_value::<HighlightTarget>(changed).is_err());
        let mut changed_region = region.clone();
        changed_region.id = "other".into();
        assert!(target.check(&changed_region).is_err());
        changed_region = region.clone();
        changed_region.drawings.push(mewu_core::Drawing {
            id: "different".into(),
            kind: mewu_core::DrawingKind::Pen,
            color: "#000000".into(),
            stroke_width: 2.,
            points: vec![mewu_core::DrawingPoint { x: 30., y: 25. }],
            text: None,
            font_size: None, text_style: None,
            rich: None,
            origin: None,
        });
        assert!(target.check(&changed_region).is_err());
    }
    #[test]
    fn owners_dimensions_and_encoded_pixels_are_checked() {
        for owner in ["settings", "frozen-widget", "recording-controls", ""] {
            assert!(validate(owner, 100, 100).is_err());
        }
        for (w, h) in [(0, 1), (1, 0), (16385, 1), (8192, 8192)] {
            assert!(validate("space", w, h).is_err());
        }
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            80,
            40,
            image::Rgba([255, 255, 255, 255]),
        ));
        let preview = encode(&source, "source".into()).unwrap();
        let bytes = STANDARD
            .decode(
                preview
                    .data_url
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().to_rgba8(),
            highlight_foreground::foreground(&source, 0, 0, 80, 40)
                .unwrap()
                .straight()
        );
    }
}
