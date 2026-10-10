// SPDX-License-Identifier: MPL-2.0
//! Immutable native layouts are document data, independent of authoring plugins.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageDecoder, ImageEncoder};
use mewu_core::{
    DrawingKind, RichAlignment, RichContent, RichDrawingRef, RichLayout, RichRendererIdentity,
    SceneCommand, Store, VerifiedRichLayout,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Cursor, path::Path, sync::OnceLock};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);
static WORKERS: Semaphore = Semaphore::const_new(2);
static TABLE_FONTS: OnceLock<Result<String, String>> = OnceLock::new();

pub fn table_document(
    content: &RichContent,
) -> Result<crate::table_document::TableDocument, String> {
    let RichContent::Table {
        version,
        header,
        rows,
        align,
    } = content
    else {
        return Err("该对象不是表格".into());
    };
    if *version != 1
        || header.is_empty()
        || header.len() > 32
        || rows.len() > 199
        || align.len() != header.len()
        || rows.iter().any(|r| r.len() != header.len())
        || serde_json::to_vec(content).map_err(|_| "表格无效")?.len()
            > mewu_core::MAX_RICH_CONTENT_BYTES
        || header
            .iter()
            .chain(rows.iter().flatten())
            .any(|s| s.chars().any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        return Err("表格内容或尺寸无效".into());
    }
    Ok(crate::table_document::TableDocument {
        index: 0,
        header: header.clone(),
        rows: rows.clone(),
        align: align
            .iter()
            .map(|a| {
                a.map(|a| match a {
                    RichAlignment::Left => "left",
                    RichAlignment::Center => "center",
                    RichAlignment::Right => "right",
                })
            })
            .collect(),
    })
}

pub(crate) fn table_font_identity() -> Result<String, String> {
    TABLE_FONTS
        .get_or_init(|| {
            let fonts = crate::drawing_render::fonts()?;
            let mut faces = Vec::new();
            for face in fonts.faces() {
                let value = fonts
                    .with_face_data(face.id, |bytes, index| {
                        let mut hash = Sha256::new();
                        hash.update(index.to_be_bytes());
                        hash.update(bytes);
                        format!("{:x}", hash.finalize())
                    })
                    .ok_or("无法核对表格字体")?;
                faces.push(value);
            }
            faces.sort();
            faces.dedup();
            if faces.is_empty() {
                return Err("表格字体不可用".into());
            }
            let mut hash = Sha256::new();
            hash.update(b"mewu.table-fonts.v1\0");
            for face in faces {
                hash.update(face.as_bytes());
                hash.update([0]);
            }
            Ok(format!("{:x}", hash.finalize()))
        })
        .clone()
}

pub fn render_table(content: RichContent) -> Result<VerifiedRichLayout, String> {
    let document = table_document(&content)?;
    let pixels = crate::table_document::render(&document)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法生成表格图层")?;
    if png.len() > mewu_core::MAX_RICH_PNG_BYTES {
        return Err("表格图层过大".into());
    }
    let reference = VerifiedRichLayout::new(
        uuid::Uuid::new_v4().to_string(),
        content,
        RichRendererIdentity {
            id: "mewu.table.resvg".into(),
            version: "resvg-0.48.1".into(),
            font_sha256: table_font_identity()?,
            style_version: 1,
        },
        pixels.width(),
        pixels.height(),
        png,
    )
    .map_err(|e| e.to_string())?;
    // The same complete bounded decode is required at the persistence boundary.
    decode(&reference)?;
    Ok(reference)
}

pub fn decode(layout: &RichLayout) -> Result<image::RgbaImage, String> {
    if layout.png().len() > mewu_core::MAX_RICH_PNG_BYTES {
        return Err("图层图片过大".into());
    }
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(layout.png()))
        .map_err(|_| "图层图片损坏")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(mewu_core::MAX_RICH_PIXELS * 8);
    decoder.set_limits(limits).map_err(|_| "图层图片过大")?;
    if decoder.dimensions() != (layout.width(), layout.height()) {
        return Err("图层图片尺寸已变更".into());
    }
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "图层图片损坏")?
        .into_rgba8();
    if u64::from(image.width()) * u64::from(image.height()) > mewu_core::MAX_RICH_PIXELS {
        return Err("图层图片过大".into());
    }
    Ok(image)
}

/// Only a trusted assets root may supply this database path. No model/renderer
/// path or virtual Asset enters the shared crop/export composition path.
pub struct RichRaster {
    pub reference: RichDrawingRef,
    pub pixels: image::RgbaImage,
}
pub type RichRasters = BTreeMap<String, RichRaster>;
pub fn load_raster(
    assets_root: &Path,
    reference: &RichDrawingRef,
) -> Result<image::RgbaImage, String> {
    let database = assets_root
        .parent()
        .ok_or("素材目录无效")?
        .join("spaces.db");
    let layout = Store::read_rich_layout(&database, reference).map_err(|e| e.to_string())?;
    decode(&layout)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutTarget {
    pub scene_id: String,
    pub region_id: String,
    pub drawing_id: String,
    pub expected_revision: u64,
    pub reference: RichDrawingRef,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    pub target: LayoutTarget,
    pub data_url: String,
}
fn read_target(app: &AppHandle, owner: &str, target: &LayoutTarget) -> Result<RichLayout, String> {
    if owner != "space" {
        return Err("此窗口不能读取截图图层".into());
    }
    let host = app.state::<crate::Host>();
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    let snapshot = engine.store.snapshot();
    let scene = snapshot
        .scenes
        .iter()
        .find(|s| s.id == target.scene_id && !s.closed)
        .ok_or("会话已关闭")?;
    let region = scene
        .regions
        .iter()
        .find(|r| r.id == target.region_id)
        .ok_or("选区不存在")?;
    if region.drawing_revision != target.expected_revision {
        return Err("图层已变更，请重试".into());
    }
    let drawing = region
        .drawings
        .iter()
        .find(|d| d.id == target.drawing_id)
        .ok_or("图层不存在")?;
    if drawing.kind != DrawingKind::Rich || drawing.rich.as_ref() != Some(&target.reference) {
        return Err("图层来源已变更".into());
    }
    engine
        .store
        .rich_layout(&target.reference)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_drawing_layout_preview(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    region_id: String,
    drawing_id: String,
    expected_revision: u64,
    reference: RichDrawingRef,
) -> Result<Preview, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "图层正在处理")?;
    let worker = WORKERS.acquire().await.map_err(|_| "图层处理不可用")?;
    let target = LayoutTarget {
        scene_id,
        region_id,
        drawing_id,
        expected_revision,
        reference,
    };
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let layout = read_target(&app, &owner, &target)?;
        decode(&layout)?;
        read_target(&app, &owner, &target)?;
        Ok(Preview {
            target,
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(layout.png())),
        })
    })
    .await
    .map_err(|_| "图层读取中断")?
}

#[tauri::command]
pub async fn copy_drawing_table(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    region_id: String,
    drawing_id: String,
    expected_revision: u64,
    reference: RichDrawingRef,
    format: crate::table_host::ExportFormat,
) -> Result<(), String> {
    let request = REQUESTS.try_acquire().map_err(|_| "表格正在处理")?;
    let worker = WORKERS.acquire().await.map_err(|_| "表格处理不可用")?;
    let target = LayoutTarget {
        scene_id,
        region_id,
        drawing_id,
        expected_revision,
        reference,
    };
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let layout = read_target(&app, &owner, &target)?;
        let table = table_document(layout.content())?;
        let _clipboard = crate::table_host::CLIPBOARD
            .lock()
            .map_err(|_| "剪贴板正在使用")?;
        read_target(&app, &owner, &target)?;
        use crate::table_host::ExportFormat as F;
        match format {
            F::Markdown | F::Csv | F::Tsv => {
                let text = match format {
                    F::Markdown => table.markdown(),
                    F::Csv => table.delimited(','),
                    _ => table.delimited('\t'),
                };
                arboard::Clipboard::new()
                    .map_err(|_| "无法打开剪贴板")?
                    .set_text(text)
                    .map_err(|_| "无法复制表格")?;
            }
            F::Png => {
                let image = decode(&layout)?;
                read_target(&app, &owner, &target)?;
                arboard::Clipboard::new()
                    .map_err(|_| "无法打开剪贴板")?
                    .set_image(arboard::ImageData {
                        width: image.width() as usize,
                        height: image.height() as usize,
                        bytes: std::borrow::Cow::Owned(image.into_raw()),
                    })
                    .map_err(|_| "无法复制表格图片")?;
            }
            F::Table => {
                let image = decode(&layout)?;
                let path = crate::table_staging::stage(&app.state::<crate::Host>().root, &image)?;
                read_target(&app, &owner, &target)?;
                crate::table_clipboard::write(crate::table_clipboard::PreparedTableClipboard {
                    html: table.html(),
                    markdown: table.markdown(),
                    csv: table.delimited(','),
                    image,
                    png_path: path,
                })?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "表格复制中断")?
}

#[tauri::command]
pub fn apply_drawing_document(
    app: AppHandle,
    window: WebviewWindow,
    command: SceneCommand,
) -> Result<crate::HostSnapshot, String> {
    if window.label() != "space" {
        return Err("此窗口不能编辑截图图层".into());
    }
    let host = app.state::<crate::Host>();
    let mut engine = host.lock()?;
    crate::ensure_scene_command_admission(&app, &host, window.label(), &command)?;
    let snapshot = engine.store.snapshot();
    match &command {
        SceneCommand::UpdateDrawing {
            scene_id,
            region_id,
            drawing,
            ..
        } => {
            let scene = snapshot
                .scenes
                .iter()
                .find(|s| &s.id == scene_id)
                .ok_or("会话不存在")?;
            let region = scene
                .regions
                .iter()
                .find(|r| &r.id == region_id)
                .ok_or("选区不存在")?;
            let previous = region
                .drawings
                .iter()
                .find(|d| d.id == drawing.id)
                .ok_or("图层不存在")?;
            if previous.kind != DrawingKind::Rich
                || previous.rich != drawing.rich
                || previous.origin != drawing.origin
            {
                return Err("此入口仅编辑已有富图层".into());
            }
        }
        SceneCommand::RemoveDrawing {
            scene_id,
            region_id,
            drawing_id,
            ..
        } => {
            let scene = snapshot
                .scenes
                .iter()
                .find(|s| &s.id == scene_id)
                .ok_or("会话不存在")?;
            let region = scene
                .regions
                .iter()
                .find(|r| &r.id == region_id)
                .ok_or("选区不存在")?;
            if !region
                .drawings
                .iter()
                .any(|d| &d.id == drawing_id && d.kind == DrawingKind::Rich)
            {
                return Err("此入口仅移除已有富图层".into());
            }
        }
        SceneCommand::UndoDrawing { .. } | SceneCommand::RedoDrawing { .. } => {}
        _ => return Err("此入口不创建绘制对象".into()),
    }
    let next = engine.store.apply(command).map_err(|e| e.to_string())?;
    engine.ocr_jobs.reconcile(&next);
    engine.translation_jobs.reconcile(&next);
    Ok(crate::publish(&app, &next))
}
