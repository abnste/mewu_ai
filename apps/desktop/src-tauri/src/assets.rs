// SPDX-License-Identifier: MPL-2.0
use image::{DynamicImage, ImageReader};
use mewu_core::{Asset, AssetKind, Scene, Snapshot};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
pub const MAX_TEXT: usize = 2 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
pub const MAX_IMPORT_FILES: usize = 12;
const MAX_IMPORT_BYTES: usize = 128 * 1024 * 1024;

pub fn resolve(snapshot: &Snapshot, id: &str) -> Option<Asset> {
    snapshot
        .scenes
        .iter()
        .flat_map(|s| {
            s.background
                .iter()
                .chain(
                    s.regions
                        .iter()
                        .filter_map(|region| region.image_override.as_ref()),
                )
                .chain(
                    s.regions.iter().filter_map(|region| {
                        region.translation.as_ref().map(|saved| &saved.overlay)
                    }),
                )
                .chain(s.items.iter().map(|i| &i.asset))
                .chain(
                    s.messages
                        .iter()
                        .flat_map(|m| m.attachments.iter().flatten()),
                )
        })
        .find(|a| a.id == id)
        .cloned()
}
pub fn verified_path(asset: &Asset, root: &Path) -> Result<PathBuf, String> {
    let path = crate::asset_locations::resolve_path(Path::new(&asset.path), root)
        .map_err(|_| "素材不存在或路径无效")?;
    let root = root.canonicalize().map_err(|_| "素材目录不可用")?;
    if !path.starts_with(root) {
        return Err("素材路径超出范围".into());
    }
    Ok(path)
}
/// Only completed, app-owned MP4 recordings may use the media transport.
/// This checks the container signature, not codec compatibility; the recorder
/// owns H.264 encoding and must finalize the file before registering its asset.
pub fn open_video(asset: &Asset, root: &Path) -> Result<(std::fs::File, u64), String> {
    if asset.kind != AssetKind::Video
        || uuid::Uuid::parse_str(&asset.id)
            .map_or(true, |id| id.hyphenated().to_string() != asset.id)
    {
        return Err("视频素材无效".into());
    }
    let path = verified_path(asset, root)?;
    let expected_name = format!("{}.mp4", asset.id);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        return Err("视频素材无效".into());
    }
    let mut file = std::fs::File::open(path).map_err(|_| "无法读取视频")?;
    let metadata = file.metadata().map_err(|_| "无法读取视频")?;
    let length = metadata.len();
    if !metadata.is_file() || !(24..=MAX_VIDEO_BYTES).contains(&length) {
        return Err("视频为空或超过 512 MB".into());
    }
    let mut header = [0_u8; 16];
    file.read_exact(&mut header).map_err(|_| "视频格式无效")?;
    let box_size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
    if &header[4..8] != b"ftyp"
        || !(16..=4096).contains(&box_size)
        || box_size > length
        || (box_size - 16) % 4 != 0
    {
        return Err("视频格式无效".into());
    }
    let mp4_brand = |brand: &[u8]| {
        matches!(
            brand,
            b"isom"
                | b"iso2"
                | b"iso4"
                | b"iso5"
                | b"iso6"
                | b"mp41"
                | b"mp42"
                | b"avc1"
                | b"M4V "
                | b"MSNV"
        )
    };
    let mut recognized = mp4_brand(&header[8..12]);
    for _ in 0..(box_size - 16) / 4 {
        let mut brand = [0_u8; 4];
        file.read_exact(&mut brand).map_err(|_| "视频格式无效")?;
        recognized |= mp4_brand(&brand);
    }
    if !recognized {
        return Err("视频格式无效".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| "无法读取视频")?;
    Ok((file, length))
}
pub fn read_text(asset: &Asset, root: &Path) -> Result<String, String> {
    let path = verified_path(asset, root)?;
    if std::fs::metadata(&path).map_err(|_| "无法读取素材")?.len() > MAX_TEXT as u64 {
        return Err("文档超过 2 MB".into());
    }
    let mut body = String::new();
    std::fs::File::open(path)
        .map_err(|_| "无法读取素材")?
        .take((MAX_TEXT + 1) as u64)
        .read_to_string(&mut body)
        .map_err(|_| "文档不是有效 UTF-8")?;
    if body.len() > MAX_TEXT {
        return Err("文档超过 2 MB".into());
    }
    Ok(body)
}
pub fn image(asset: &Asset, root: &Path) -> Result<DynamicImage, String> {
    let path = verified_path(asset, root)?;
    let mut reader = ImageReader::open(path)
        .map_err(|_| "无法读取图片")?
        .with_guessed_format()
        .map_err(|_| "图片格式无效")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| "无法解码图片或图片尺寸过大".into())
}
pub fn crop(scene: &Scene, region_id: &str, root: &Path) -> Result<DynamicImage, String> {
    let region = scene
        .regions
        .iter()
        .find(|r| r.id == region_id)
        .ok_or("选区不存在")?;
    let background = scene.background.as_ref().ok_or("尚未截图")?;
    crop_from_parts(background, region, root)
}

/// Region geometry places an override on the scene; its drawing coordinates
/// belong to the full replacement image, never the desktop behind that image.
pub fn crop_from_parts(
    background: &Asset,
    region: &mewu_core::Region,
    root: &Path,
) -> Result<DynamicImage, String> {
    crop_from_parts_with_mapping(background, region, root).map(|(image, _)| image)
}

/// Mapping is produced from the exact checked integer bounds used to render.
pub fn crop_from_parts_with_mapping(
    background: &Asset,
    region: &mewu_core::Region,
    root: &Path,
) -> Result<(DynamicImage, mewu_core::VisualPixelRect), String> {
    let overlay = if let Some(saved) = &region.translation {
        let source = region.image_override.as_ref().unwrap_or(background);
        // Core OCR may supply an override as a virtual full-image background.
        // Its source ID stays exact even though the scene background ID differs.
        if saved.source_id != source.id || saved.drawing_revision != region.drawing_revision {
            return Err("译文图层已失效，请重新翻译".into());
        }
        let pixels = image(&saved.overlay, root)?.into_rgba8();
        if pixels.dimensions() != (saved.document.width, saved.document.height)
            || saved.overlay.width != Some(pixels.width())
            || saved.overlay.height != Some(pixels.height())
        {
            return Err("译文图层尺寸已变更".into());
        }
        Some(pixels)
    } else {
        None
    };
    crop_source_from_parts(background, region, root, overlay.as_ref())
}

/// OCR excludes the old translation and ordinary annotations, but keeps mosaic
/// redactions so text that the user obscured is not sent to the model.
pub fn clean_crop_from_parts(
    background: &Asset,
    region: &mewu_core::Region,
    root: &Path,
) -> Result<DynamicImage, String> {
    let mut clean = region.clone();
    clean
        .drawings
        .retain(|drawing| drawing.kind == mewu_core::DrawingKind::Mosaic
            || drawing.rich.as_ref().is_some_and(|r| r.kind.is_raster_edit()));
    crop_source_from_parts(background, &clean, root, None).map(|(image, _)| image)
}

fn crop_source_from_parts(
    background: &Asset,
    region: &mewu_core::Region,
    root: &Path,
    overlay: Option<&image::RgbaImage>,
) -> Result<(DynamicImage, mewu_core::VisualPixelRect), String> {
    if let Some(source) = &region.image_override {
        let image = image(source, root)?;
        if source.width != Some(image.width()) || source.height != Some(image.height()) {
            return Err("长截图尺寸已变更".into());
        }
        let virtual_region = mewu_core::Region {
            x: 0.,
            y: 0.,
            width: image.width() as f64,
            height: image.height() as f64,
            image_override: None,
            ..region.clone()
        };
        let (x, y, width, height) = crate::drawing_render::crop_bounds(&image, &virtual_region)?;
        Ok((
            crate::drawing_render::crop_region_with_layouts(
                &image,
                &virtual_region,
                overlay,
                |reference| crate::rich_annotation_host::load_raster(root, reference),
            )?,
            mewu_core::VisualPixelRect {
                x,
                y,
                width,
                height,
            },
        ))
    } else {
        let image = image(background, root)?;
        let (x, y, width, height) = crate::drawing_render::crop_bounds(&image, region)?;
        Ok((
            crate::drawing_render::crop_region_with_layouts(
                &image,
                region,
                overlay,
                |reference| crate::rich_annotation_host::load_raster(root, reference),
            )?,
            mewu_core::VisualPixelRect {
                x,
                y,
                width,
                height,
            },
        ))
    }
}
/// Owns only this attempt's new UUID files until the whole database batch commits.
pub struct ImportedAssets {
    pub assets: Vec<Asset>,
    created: Vec<PathBuf>,
}
impl ImportedAssets {
    pub fn commit(&mut self) {
        self.created.clear();
    }
}
impl Drop for ImportedAssets {
    fn drop(&mut self) {
        for path in &self.created {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn import_batch(sources: &[PathBuf], root: &Path) -> Result<ImportedAssets, String> {
    if sources.is_empty() || sources.len() > MAX_IMPORT_FILES {
        return Err("一次请选择 1 到 12 个文件".into());
    }
    let mut batch = ImportedAssets {
        assets: Vec::new(),
        created: Vec::new(),
    };
    let mut total = 0;
    for source in sources {
        let (asset, bytes) = read_import(source, MAX_IMPORT_BYTES - total)?;
        total += bytes.len();
        let destination = root.join(format!(
            "{}.{}",
            asset.id,
            source
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|_| "无法保存素材")?;
        batch.created.push(destination.clone());
        file.write_all(&bytes).map_err(|_| "无法保存素材")?;
        file.sync_all().map_err(|_| "无法保存素材")?;
        batch.assets.push(Asset {
            path: destination.to_string_lossy().into_owned(),
            ..asset
        });
    }
    Ok(batch)
}

#[cfg(test)]
pub fn import(source: &Path, root: &Path) -> Result<Asset, String> {
    let mut batch = import_batch(&[source.to_path_buf()], root)?;
    let asset = batch.assets[0].clone();
    batch.commit();
    Ok(asset)
}

fn read_import(source: &Path, remaining: usize) -> Result<(Asset, Vec<u8>), String> {
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" => AssetKind::Image,
        "html" | "htm" => AssetKind::Html,
        "svg" => AssetKind::Svg,
        "txt" | "md" | "json" | "csv" => AssetKind::Text,
        _ => return Err("目前支持图片、HTML、SVG、TXT、Markdown、JSON 和 CSV".into()),
    };
    // Validate and save the exact same bytes from one opened source. A growing
    // file cannot escape a metadata-only size check or replace validated content.
    let maximum = if kind == AssetKind::Image {
        MAX_FILE as usize
    } else {
        MAX_TEXT
    };
    let file = std::fs::File::open(source).map_err(|_| "无法读取文件")?;
    let metadata = file.metadata().map_err(|_| "无法读取文件")?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(if kind == AssetKind::Image {
            "图片超过 32 MB"
        } else {
            "文档超过 2 MB"
        }
        .into());
    }
    let limit = maximum.min(remaining);
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取文件")?;
    if bytes.len() > limit {
        return Err(if remaining < maximum {
            "所选文件合计超过 128 MB"
        } else {
            "文件大小超过限制"
        }
        .into());
    }
    let mut width = None;
    let mut height = None;
    if matches!(kind, AssetKind::Image) {
        let dimensions = ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| "图片格式无效")?
            .into_dimensions()
            .map_err(|_| "图片格式无效")?;
        if dimensions.0 == 0
            || dimensions.1 == 0
            || u64::from(dimensions.0) * u64::from(dimensions.1) > 40_000_000
        {
            return Err("图片超过 4000 万像素".into());
        }
        width = Some(dimensions.0);
        height = Some(dimensions.1);
    } else {
        std::str::from_utf8(&bytes).map_err(|_| "文档不是有效 UTF-8")?;
        if kind == AssetKind::Text && bytes.contains(&0) {
            return Err("文本文件包含无效空字符".into());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    Ok((
        Asset {
            id,
            name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            kind,
            path: String::new(),
            width,
            height,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        },
        bytes,
    ))
}

/// Text is data only, including JSON/Markdown. No parsing, rendering or scripts.
pub fn text_export_bytes(asset: &Asset, root: &Path) -> Result<Vec<u8>, String> {
    if asset.kind != AssetKind::Text {
        return Err("此素材不是文本文件".into());
    }
    let body = read_text(asset, root)?;
    if body.contains('\0') {
        return Err("文本文件包含无效空字符".into());
    }
    Ok(body.into_bytes())
}

pub fn save_text_copy(bytes: &[u8], target: &Path, managed_root: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or("保存路径无效")?
        .canonicalize()
        .map_err(|_| "保存目录不存在")?;
    let managed = managed_root
        .canonicalize()
        .map_err(|_| "应用数据目录不可用")?;
    let destination = parent.join(target.file_name().ok_or("保存路径无效")?);
    if parent.starts_with(&managed)
        || destination
            .canonicalize()
            .is_ok_and(|p| p.starts_with(&managed))
    {
        return Err("请选择应用数据目录之外的位置".into());
    }
    let temporary = parent.join(format!(".mewu-text-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "无法创建导出文件")?;
        file.write_all(bytes).map_err(|_| "无法保存文件")?;
        file.sync_all().map_err(|_| "无法保存文件")?;
        drop(file);
        std::fs::rename(&temporary, &destination).map_err(|_| "无法替换导出文件".into())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
pub fn from_code(answer: &str, root: &Path) -> Result<Vec<Asset>, String> {
    let mut result = Vec::new();
    let mut rest = answer;
    // This parses Markdown fences, never JSON/HTML with substring field extraction.
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        let Some(newline) = rest.find('\n') else {
            break;
        };
        let language = rest[..newline].trim().to_ascii_lowercase();
        rest = &rest[newline + 1..];
        let Some(end) = rest.find("```") else { break };
        let body = &rest[..end];
        rest = &rest[end + 3..];
        if !matches!(language.as_str(), "html" | "svg")
            || body.len() > MAX_TEXT
            || result.len() >= 4
        {
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.{language}"));
        std::fs::write(&path, body).map_err(|_| "无法保存生成内容")?;
        result.push(Asset {
            id,
            name: format!("生成内容.{}", language),
            kind: if language == "html" {
                AssetKind::Html
            } else {
                AssetKind::Svg
            },
            path: path.to_string_lossy().into_owned(),
            width: None,
            height: None,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("mewu-asset-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(p.join("assets")).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(root), Ok(parent)) =
                (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            {
                if root.parent() == Some(parent.as_path())
                    && root
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("mewu-asset-test-"))
                {
                    let _ = std::fs::remove_dir_all(root);
                }
            }
        }
    }

    fn synthetic_image(root: &Path, width: u32, height: u32, color: [u8; 4]) -> Asset {
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.png"));
        image::RgbaImage::from_pixel(width, height, image::Rgba(color))
            .save(&path)
            .unwrap();
        Asset {
            id,
            name: "synthetic.png".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into_owned(),
            width: Some(width),
            height: Some(height),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        }
    }

    fn translated(
        background: &Asset,
        region: &mewu_core::Region,
        overlay: Asset,
    ) -> mewu_core::SavedTranslation {
        let (width, height) = (overlay.width.unwrap(), overlay.height.unwrap());
        mewu_core::SavedTranslation {
            background_id: background.id.clone(),
            drawing_revision: region.drawing_revision,
            source_id: region
                .image_override
                .as_ref()
                .unwrap_or(background)
                .id
                .clone(),
            document: mewu_core::TranslationDocument {
                version: 1,
                target_language: "zh-Hans".into(),
                width,
                height,
                text_angle: None,
                lines: vec![mewu_core::TranslationLine {
                    id: "line-1".into(),
                    source: "source".into(),
                    text: "译文".into(),
                    r#box: mewu_core::TranslationBox {
                        x: 2.,
                        y: 2.,
                        width: 20.,
                        height: 14.,
                    },
                }],
            },
            overlay,
            selection: mewu_core::OcrDocument {
                engine: "mewu.translation.layout.v1".into(),
                language: "zh-Hans".into(),
                width,
                height,
                text_angle: None,
                lines: vec![mewu_core::OcrLine {
                    text: "译文".into(),
                    words: vec![mewu_core::OcrWord {
                        text: "译文".into(),
                        x: 2.,
                        y: 2.,
                        width: 20.,
                        height: 14.,
                    }],
                }],
            },
        }
    }

    #[test]
    fn translation_overlay_uses_image_pixels_and_is_readable_through_virtual_ocr_source() {
        use image::GenericImageView;
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 100, 100, [10, 20, 30, 255]);
        let source = synthetic_image(&root, 32, 80, [230, 240, 250, 255]);
        let overlay = synthetic_image(&root, 32, 80, [0, 180, 0, 255]);
        let mut region = mewu_core::Region {
            id: "long".into(),
            x: 10.,
            y: 10.,
            width: 16.,
            height: 40.,
            image_override: Some(source.clone()),
            ..Default::default()
        };
        region.translation = Some(translated(&background, &region, overlay.clone()));
        assert_eq!(
            crop_from_parts(&background, &region, &root)
                .unwrap()
                .get_pixel(12, 50)
                .0,
            [0, 180, 0, 255]
        );
        assert_eq!(
            clean_crop_from_parts(&background, &region, &root)
                .unwrap()
                .get_pixel(12, 50)
                .0,
            [230, 240, 250, 255]
        );
        // Core OCR supplies a full override as virtual background while keeping
        // the saved document's original scene background identity.
        let virtual_region = mewu_core::Region {
            x: 0.,
            y: 0.,
            width: 32.,
            height: 80.,
            image_override: None,
            ..region.clone()
        };
        assert_eq!(
            crop_from_parts(&source, &virtual_region, &root)
                .unwrap()
                .get_pixel(12, 50)
                .0,
            [0, 180, 0, 255]
        );
        region.translation.as_mut().unwrap().source_id = uuid::Uuid::new_v4().to_string();
        assert!(crop_from_parts(&background, &region, &root).is_err());
        region.translation.as_mut().unwrap().source_id = source.id.clone();
        image::RgbaImage::new(31, 80).save(&overlay.path).unwrap();
        assert!(crop_from_parts(&background, &region, &root).is_err());
    }

    #[test]
    fn moving_resizing_and_undoing_a_translated_long_image_preserve_exported_pixels() {
        use image::GenericImageView;
        use mewu_core::{
            Drawing, DrawingKind, DrawingPoint, OcrTarget, Region, RegionGeometry, SceneCommand,
            Store,
        };
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 100, 100, [10, 20, 30, 255]);
        let source = synthetic_image(&root, 32, 80, [230, 240, 250, 255]);
        let overlay = synthetic_image(&root, 32, 80, [0, 180, 0, 255]);
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let region_id = uuid::Uuid::new_v4().to_string();
        store.set_background(&scene_id, background.clone()).unwrap();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene_id.clone(),
                region: Region {
                    id: region_id.clone(),
                    x: 10.,
                    y: 10.,
                    width: 16.,
                    height: 40.,
                    ..Default::default()
                },
            })
            .unwrap();
        let target = OcrTarget {
            scene_id: scene_id.clone(),
            region_id: region_id.clone(),
            background_id: background.id.clone(),
            drawing_revision: 0,
            x: 10.,
            y: 10.,
            width: 16.,
            height: 40.,
        };
        store.set_region_image(&target, source.clone()).unwrap();
        let current = |store: &Store| {
            store
                .snapshot()
                .scenes
                .into_iter()
                .find(|s| s.id == scene_id)
                .unwrap()
                .regions
                .into_iter()
                .find(|r| r.id == region_id)
                .unwrap()
        };
        let original = current(&store);
        store
            .apply(SceneCommand::AddDrawing {
                scene_id: scene_id.clone(),
                region_id: region_id.clone(),
                background_id: background.id.clone(),
                expected_revision: original.drawing_revision,
                drawing: Drawing {
                    origin: None,
                    rich: None,
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: DrawingKind::Pen,
                    color: "#ff0000".into(),
                    stroke_width: 4.,
                    points: vec![
                        DrawingPoint { x: 4., y: 50. },
                        DrawingPoint { x: 26., y: 50. },
                    ],
                    text: None,
                    font_size: None,
                },
            })
            .unwrap();
        let inked = current(&store);
        let target = OcrTarget {
            drawing_revision: inked.drawing_revision,
            x: inked.x,
            y: inked.y,
            width: inked.width,
            height: inked.height,
            ..target
        };
        let translation = translated(&background, &inked, overlay);
        store
            .set_region_translation(
                &target,
                translation.document,
                translation.overlay,
                translation.selection,
            )
            .unwrap();
        let before = current(&store);
        let pixels = crop_from_parts(&background, &before, &root).unwrap();
        assert_eq!(pixels.dimensions(), (32, 80));
        assert_eq!(pixels.get_pixel(12, 50).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(12, 30).0, [0, 180, 0, 255]);
        store
            .apply(SceneCommand::SetRegionGeometry {
                scene_id: scene_id.clone(),
                region_id: region_id.clone(),
                background_id: background.id.clone(),
                source_id: source.id,
                expected_revision: before.drawing_revision,
                edit_id: None,
                from: RegionGeometry {
                    x: before.x,
                    y: before.y,
                    width: before.width,
                    height: before.height,
                },
                to: RegionGeometry {
                    x: 40.,
                    y: 20.,
                    width: 32.,
                    height: 64.,
                },
            })
            .unwrap();
        let moved = current(&store);
        let result = crop_from_parts(&background, &moved, &root).unwrap();
        assert_eq!(result.dimensions(), pixels.dimensions());
        assert_eq!(result.as_bytes(), pixels.as_bytes());
        assert_eq!(moved.drawings, before.drawings);
        assert_eq!(moved.drawing_history, before.drawing_history);
        let scene = store
            .snapshot()
            .scenes
            .into_iter()
            .find(|value| value.id == scene_id)
            .unwrap();
        let edit = scene.geometry_history.undo.last().unwrap();
        store
            .apply(SceneCommand::UndoRegionGeometry {
                scene_id: scene_id.clone(),
                expected_history_revision: scene.geometry_history.revision,
                expected_operation_id: edit.id.clone(),
                region_id: region_id.clone(),
                background_id: background.id.clone(),
                source_id: moved.image_override.as_ref().unwrap().id.clone(),
                expected_revision: moved.drawing_revision,
                from: RegionGeometry::from(&moved),
            })
            .unwrap();
        let undone = current(&store);
        assert_eq!(RegionGeometry::from(&undone), RegionGeometry::from(&before));
        assert!(undone.drawing_revision > moved.drawing_revision);
        assert_eq!(
            crop_from_parts(&background, &undone, &root)
                .unwrap()
                .as_bytes(),
            pixels.as_bytes()
        );
        let scene = store
            .snapshot()
            .scenes
            .into_iter()
            .find(|value| value.id == scene_id)
            .unwrap();
        let edit = scene.geometry_history.redo.last().unwrap();
        store
            .apply(SceneCommand::RedoRegionGeometry {
                scene_id: scene_id.clone(),
                expected_history_revision: scene.geometry_history.revision,
                expected_operation_id: edit.id.clone(),
                region_id: region_id.clone(),
                background_id: background.id.clone(),
                source_id: undone.image_override.as_ref().unwrap().id.clone(),
                expected_revision: undone.drawing_revision,
                from: RegionGeometry::from(&undone),
            })
            .unwrap();
        let redone = current(&store);
        assert_eq!(RegionGeometry::from(&redone), RegionGeometry::from(&moved));
        assert!(redone.drawing_revision > undone.drawing_revision);
        assert_eq!(
            crop_from_parts(&background, &redone, &root)
                .unwrap()
                .as_bytes(),
            pixels.as_bytes()
        );
        assert_eq!(redone.drawings, before.drawings);
        assert_eq!(redone.drawing_history, before.drawing_history);
    }

    #[test]
    fn translation_ocr_keeps_redactions_and_excludes_previous_translation_and_other_ink() {
        use image::GenericImageView;
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 64, 64, [255, 255, 255, 255]);
        let pixels = image::RgbaImage::from_fn(64, 64, |x, y| {
            let value = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgba([value, value, value, 255])
        });
        pixels.save(&background.path).unwrap();
        let overlay = synthetic_image(&root, 64, 64, [0, 180, 0, 255]);
        let mut region = mewu_core::Region {
            id: "region".into(),
            width: 64.,
            height: 64.,
            ..Default::default()
        };
        let object = |kind, points| mewu_core::Drawing {
            origin: None,
            rich: None,
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            color: "#ff0000".into(),
            stroke_width: 4.,
            points,
            text: None,
            font_size: None,
        };
        region.drawings = vec![
            object(
                mewu_core::DrawingKind::Mosaic,
                vec![
                    mewu_core::DrawingPoint { x: 0., y: 0. },
                    mewu_core::DrawingPoint { x: 24., y: 24. },
                ],
            ),
            object(
                mewu_core::DrawingKind::Pen,
                vec![
                    mewu_core::DrawingPoint { x: 0., y: 50. },
                    mewu_core::DrawingPoint { x: 63., y: 50. },
                ],
            ),
        ];
        region.drawings[0].stroke_width = 8.;
        region.translation = Some(translated(&background, &region, overlay));
        let input = clean_crop_from_parts(&background, &region, &root).unwrap();
        assert_eq!(input.get_pixel(2, 2).0, [127, 127, 127, 255]);
        assert_eq!(input.get_pixel(40, 50), pixels.get_pixel(40, 50).clone());
        let visible = crop_from_parts(&background, &region, &root).unwrap();
        assert_eq!(visible.get_pixel(2, 2).0, [127, 127, 127, 255]);
        assert_eq!(visible.get_pixel(40, 50).0, [255, 0, 0, 255]);
        assert_eq!(visible.get_pixel(40, 30).0, [0, 180, 0, 255]);
    }

    #[test]
    fn override_crop_uses_complete_replacement_pixels_and_its_own_drawing_coordinates() {
        use image::GenericImageView;
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 100, 100, [10, 20, 30, 255]);
        let replacement = synthetic_image(&root, 24, 160, [230, 240, 250, 255]);
        let mut region = mewu_core::Region {
            id: "long-region".into(),
            x: 40.,
            y: 20.,
            width: 12.,
            height: 80.,
            image_override: Some(replacement.clone()),
            drawings: vec![mewu_core::Drawing {
                origin: None,
                rich: None,
                id: uuid::Uuid::new_v4().to_string(),
                kind: mewu_core::DrawingKind::Pen,
                color: "#ff0000".into(),
                stroke_width: 4.,
                points: vec![
                    mewu_core::DrawingPoint { x: 2., y: 120. },
                    mewu_core::DrawingPoint { x: 22., y: 120. },
                ],
                text: None,
                font_size: None,
            }],
            ..Default::default()
        };
        let image = crop_from_parts(&background, &region, &root).unwrap();
        assert_eq!(image.dimensions(), (24, 160));
        assert_eq!(image.get_pixel(10, 20).0, [230, 240, 250, 255]);
        assert_eq!(image.get_pixel(10, 120).0, [255, 0, 0, 255]);
        // Export does not mutate either the captured page or the replacement PNG.
        assert_eq!(
            image::open(&replacement.path).unwrap().get_pixel(10, 120).0,
            [230, 240, 250, 255]
        );
        assert_eq!(
            image::open(&background.path).unwrap().get_pixel(10, 20).0,
            [10, 20, 30, 255]
        );
        region.drawings.clear();
        assert_eq!(
            crop_from_parts(&background, &region, &root)
                .unwrap()
                .get_pixel(10, 120)
                .0,
            [230, 240, 250, 255]
        );
        // The fully captured replacement is independent of loading desktop pixels.
        std::fs::remove_file(&background.path).unwrap();
        assert_eq!(
            crop_from_parts(&background, &region, &root)
                .unwrap()
                .dimensions(),
            (24, 160)
        );
    }

    #[test]
    fn ordinary_region_crop_preserves_background_coordinate_contract() {
        use image::GenericImageView;
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 100, 100, [2, 3, 4, 255]);
        let mut pixels = image::open(&background.path).unwrap().to_rgba8();
        pixels.put_pixel(20, 30, image::Rgba([99, 88, 77, 255]));
        pixels.save(&background.path).unwrap();
        let region = mewu_core::Region {
            id: "normal".into(),
            x: 20.,
            y: 30.,
            width: 10.,
            height: 15.,
            ..Default::default()
        };
        let cropped = crop_from_parts(&background, &region, &root).unwrap();
        assert_eq!(cropped.dimensions(), (10, 15));
        assert_eq!(cropped.get_pixel(0, 0).0, [99, 88, 77, 255]);
    }

    #[test]
    fn changed_or_outside_replacement_cannot_be_rendered_as_approved_override() {
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 20, 20, [0, 0, 0, 255]);
        let replacement = synthetic_image(&root, 10, 40, [255, 255, 255, 255]);
        let mut region = mewu_core::Region {
            id: "long".into(),
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
            image_override: Some(replacement.clone()),
            ..Default::default()
        };
        image::RgbaImage::new(10, 41)
            .save(&replacement.path)
            .unwrap();
        assert_eq!(
            crop_from_parts(&background, &region, &root).unwrap_err(),
            "长截图尺寸已变更"
        );
        let outside = synthetic_image(&fixture.0, 10, 40, [255, 0, 0, 255]);
        region.image_override = Some(outside);
        assert!(crop_from_parts(&background, &region, &root).is_err());
    }

    #[test]
    fn resolver_keeps_closed_scene_override_and_immutable_message_attachment_separate() {
        let fixture = Fixture::new();
        let root = fixture.0.join("assets");
        let background = synthetic_image(&root, 20, 20, [255, 255, 255, 255]);
        let replacement = synthetic_image(&root, 10, 40, [255, 0, 0, 255]);
        let historical = synthetic_image(&root, 10, 20, [0, 0, 255, 255]);
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        store
            .apply(mewu_core::SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "历史图像".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene_id).unwrap();
        store
            .attach_run_assets(&scene_id, &run.run_id, vec![historical.clone()])
            .unwrap();
        store.finish_run(&scene_id, &run.run_id, "完成").unwrap();
        store
            .apply(mewu_core::SceneCommand::CloseScene {
                scene_id: scene_id.clone(),
            })
            .unwrap();
        let mut snapshot = store.snapshot();
        let scene = snapshot
            .scenes
            .iter_mut()
            .find(|scene| scene.id == scene_id)
            .unwrap();
        scene.background = Some(background.clone());
        scene.regions.push(mewu_core::Region {
            id: "long".into(),
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
            image_override: Some(replacement.clone()),
            ..Default::default()
        });
        assert_eq!(resolve(&snapshot, &background.id), Some(background));
        assert_eq!(
            resolve(&snapshot, &replacement.id),
            Some(replacement.clone())
        );
        assert_eq!(resolve(&snapshot, &historical.id), Some(historical.clone()));
        snapshot
            .scenes
            .iter_mut()
            .find(|scene| scene.id == scene_id)
            .unwrap()
            .regions
            .clear();
        assert!(resolve(&snapshot, &replacement.id).is_none());
        assert_eq!(resolve(&snapshot, &historical.id), Some(historical));
        assert!(resolve(&snapshot, "unregistered-file").is_none());
    }

    #[test]
    fn imported_document_uses_owned_copy_and_rejects_outside_path() {
        let f = Fixture::new();
        let source = f.0.join("source.html");
        std::fs::write(&source, "<b>original</b>").unwrap();
        let root = f.0.join("assets");
        let mut asset = import(&source, &root).unwrap();
        std::fs::write(&source, "changed").unwrap();
        assert_eq!(read_text(&asset, &root).unwrap(), "<b>original</b>");
        asset.path = source.to_string_lossy().into_owned();
        assert!(read_text(&asset, &root).is_err());
    }
    #[test]
    fn text_import_keeps_exact_utf8_bytes_and_cleans_uncommitted_batches() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        let body = "\u{feff}# 标题\n<script>not executable</script>\n{\"value\":1}\n";
        let mut sources = Vec::new();
        for extension in ["txt", "md", "json", "csv"] {
            let source = f.0.join(format!("source.{extension}"));
            std::fs::write(&source, body).unwrap();
            sources.push(source);
        }
        {
            let batch = import_batch(&sources, &root).unwrap();
            assert_eq!(batch.assets.len(), 4);
            for asset in &batch.assets {
                assert_eq!(asset.kind, AssetKind::Text);
                assert_eq!(text_export_bytes(asset, &root).unwrap(), body.as_bytes());
                assert!(asset.width.is_none() && asset.height.is_none());
            }
            std::fs::write(&sources[0], "edited source").unwrap();
            assert_eq!(
                text_export_bytes(&batch.assets[0], &root).unwrap(),
                body.as_bytes()
            );
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let mut batch = import_batch(&sources, &root).unwrap();
        batch.commit();
        drop(batch);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);
    }
    #[test]
    fn failed_import_batch_removes_only_its_own_new_files() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        let keep = root.join("keep.txt");
        std::fs::write(&keep, "already registered").unwrap();
        let first = f.0.join("good.md");
        let second = f.0.join("bad.txt");
        std::fs::write(&first, "good").unwrap();
        std::fs::write(&second, [0xff, 0xfe]).unwrap();
        assert!(import_batch(&[first.clone(), second.clone()], &root).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(std::fs::read_to_string(keep).unwrap(), "already registered");
        std::fs::write(&second, b"NUL\0text").unwrap();
        assert!(import_batch(&[first.clone(), second], &root).is_err());
        assert!(import_batch(&vec![first; 13], &root).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
    #[test]
    fn text_import_accepts_empty_but_never_truncates_over_budget() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        let path = f.0.join("notes.txt");
        std::fs::write(&path, "").unwrap();
        let batch = import_batch(&[path.clone()], &root).unwrap();
        assert!(text_export_bytes(&batch.assets[0], &root)
            .unwrap()
            .is_empty());
        drop(batch);
        std::fs::write(&path, b"four").unwrap();
        assert!(read_import(&path, 3).is_err());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_TEXT as u64 + 1)
            .unwrap();
        assert!(import_batch(&[path], &root).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
    #[test]
    fn text_export_preserves_bom_and_rejects_managed_destinations() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        let source = f.0.join("source.csv");
        let bytes = "\u{feff}名字,值\n合成,1\n".as_bytes();
        std::fs::write(&source, bytes).unwrap();
        let asset = import(&source, &root).unwrap();
        let body = text_export_bytes(&asset, &root).unwrap();
        let out = f.0.join("copy.csv");
        std::fs::write(&out, "old export").unwrap();
        save_text_copy(&body, &out, &root).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        assert!(save_text_copy(&body, &root.join("not-allowed.csv"), &root).is_err());
        assert!(save_text_copy(&body, Path::new(&asset.path), &root).is_err());
        assert_eq!(text_export_bytes(&asset, &root).unwrap(), bytes);
        assert!(!std::fs::read_dir(&f.0).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mewu-text-")));
    }
    #[test]
    fn generated_documents_require_complete_fences_and_are_bounded() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        assert!(from_code("```html\nunfinished", &root).unwrap().is_empty());
        let items = from_code(
            "```html\n<button>test</button>\n```\n```svg\n<svg/>\n```",
            &root,
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            read_text(&items[0], &root).unwrap(),
            "<button>test</button>\n"
        );
    }

    #[test]
    fn recording_transport_checks_owned_uuid_container_and_size_without_importing_videos() {
        let f = Fixture::new();
        let root = f.0.join("assets");
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.mp4"));
        let bytes = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom\0\0\0\x08mdat";
        std::fs::write(&path, bytes).unwrap();
        let mut asset = Asset {
            id,
            name: "recording.mp4".into(),
            kind: AssetKind::Video,
            path: path.to_string_lossy().into_owned(),
            width: Some(640),
            height: Some(360),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let (mut file, length) = open_video(&asset, &root).unwrap();
        assert_eq!(length, bytes.len() as u64);
        let mut signature = [0; 8];
        file.read_exact(&mut signature).unwrap();
        assert_eq!(&signature[4..], b"ftyp");
        drop(file);
        assert!(import(&path, &root).is_err());
        asset.kind = AssetKind::File;
        assert!(open_video(&asset, &root).is_err());
        asset.kind = AssetKind::Video;
        asset.id = uuid::Uuid::new_v4().to_string();
        assert!(open_video(&asset, &root).is_err());
        asset.id = path.file_stem().unwrap().to_str().unwrap().into();
        let outside = f.0.join(format!("{}.mp4", asset.id));
        std::fs::write(&outside, bytes).unwrap();
        asset.path = outside.to_string_lossy().into_owned();
        assert!(open_video(&asset, &root).is_err());
        asset.path = path.to_string_lossy().into_owned();
        std::fs::write(&path, b"<html>not a recorded video</html>").unwrap();
        assert!(open_video(&asset, &root).is_err());
        std::fs::write(&path, b"\0\0\0\x18ftypavif\0\0\0\0avifmif1\0\0\0\x08mdat").unwrap();
        assert!(open_video(&asset, &root).is_err());
        std::fs::write(&path, bytes).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_VIDEO_BYTES + 1)
            .unwrap();
        assert!(open_video(&asset, &root).is_err());
    }
}
