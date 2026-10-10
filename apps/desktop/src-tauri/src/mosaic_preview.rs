// SPDX-License-Identifier: MPL-2.0
//! A first-party, read-only preview of the same pixel grid used for export.
//! Never grant canvas/CORS access to the remote-origin artifact transport.
use crate::{assets, drawing_render, Host};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::DynamicImage;
use serde::Serialize;
use std::io::Cursor;
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
static REQUESTS: Semaphore = Semaphore::const_new(16);
static DECODERS: Semaphore = Semaphore::const_new(2);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MosaicPreview {
    background_id: String,
    width: u32,
    height: u32,
    block_size: u32,
    columns: u32,
    rows: u32,
    data_url: String,
}

fn validate_request(owner: &str, block_size: u32) -> Result<(), String> {
    if owner != "space" {
        return Err("此窗口不能读取截图像素".into());
    }
    if !(6..=40).contains(&block_size) {
        return Err("马赛克尺寸无效".into());
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
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

fn encode_grid(
    image: &DynamicImage,
    background_id: String,
    block_size: u32,
) -> Result<MosaicPreview, String> {
    validate_dimensions(image.width(), image.height())?;
    let grid = drawing_render::mosaic_grid(image, block_size)?;
    let (columns, rows) = grid.dimensions();
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(grid)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "无法生成马赛克预览")?;
    let bytes = png.into_inner();
    if bytes.len() > MAX_PNG_BYTES {
        return Err("马赛克预览过大".into());
    }
    Ok(MosaicPreview {
        background_id,
        width: image.width(),
        height: image.height(),
        block_size,
        columns,
        rows,
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    })
}

#[tauri::command]
pub async fn get_mosaic_preview(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    background_id: String,
    block_size: u32,
    region_id: Option<String>,
    source_id: Option<String>,
    drawing_revision: Option<u64>,
) -> Result<MosaicPreview, String> {
    validate_request(window.label(), block_size)?;
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    // Bound both queued requests and actual image allocations. The permits move
    // into the blocking closure so a canceled JS future cannot release them early.
    let request = REQUESTS
        .try_acquire()
        .map_err(|_| "马赛克预览繁忙，请重试")?;
    let decoder = DECODERS.acquire().await.map_err(|_| "马赛克预览不可用")?;
    let override_target = match (region_id, source_id, drawing_revision) {
        (None, None, None) => None,
        (Some(region), Some(source), Some(revision)) => Some((region, source, revision)),
        _ => return Err("长截图预览目标不完整".into()),
    };
    let read = |store: &mewu_core::Store| {
        if let Some((region, source, revision)) = &override_target {
            store.region_image_for_preview(&scene_id, region, &background_id, source, *revision)
        } else {
            store.background_for_preview(&scene_id, &background_id)
        }
    };
    let background = {
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        read(&engine.store).map_err(|error| error.to_string())?
    };
    let root = host.assets.clone();
    let expected = background.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _decoder) = (request, decoder);
        let path = assets::verified_path(&background, &root)?;
        let (width, height) = image::image_dimensions(path).map_err(|_| "无法读取图片尺寸")?;
        validate_dimensions(width, height)?;
        let image = assets::image(&background, &root)?;
        encode_grid(&image, background.id, block_size)
    })
    .await
    .map_err(|_| "马赛克预览未完成")??;
    // This is document viewing, so disabling/removing the drawing plugin does
    // not hide existing marks. Closing/changing the source still fences results.
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    let current = read(&engine.store).map_err(|error| error.to_string())?;
    if current != expected {
        return Err("截图已变更".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_rejects_unprivileged_owners_and_out_of_budget_inputs() {
        for owner in ["settings", "frozen-widget", "recording-controls", ""] {
            assert!(validate_request(owner, 12).is_err());
        }
        for size in [0, 5, 41, u32::MAX] {
            assert!(validate_request("space", size).is_err());
        }
        for dimensions in [(0, 1), (1, 0), (16385, 1), (8192, 8192)] {
            assert!(validate_dimensions(dimensions.0, dimensions.1).is_err());
        }
        assert!(validate_dimensions(8192, 4096).is_ok());
    }

    #[test]
    fn preview_png_roundtrips_the_export_grid_including_partial_alpha_edges() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(13, 7, |x, y| {
            image::Rgba([
                x as u8 * 10,
                y as u8 * 20,
                90,
                if x < 6 { 128 } else { 255 },
            ])
        }));
        let preview = encode_grid(&image, "background".into(), 6).unwrap();
        assert_eq!(
            (preview.width, preview.height, preview.columns, preview.rows),
            (13, 7, 3, 2)
        );
        let bytes = STANDARD
            .decode(
                preview
                    .data_url
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        let actual = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(actual, drawing_render::mosaic_grid(&image, 6).unwrap());
        assert!(actual.pixels().all(|pixel| pixel[3] == 255));
    }
}
