// SPDX-License-Identifier: MPL-2.0
use crate::{capture_delay_work, recording, scroll_host, Host, HostSnapshot, SpaceTransition};
use image::{ImageFormat, Rgba, RgbaImage};
use mewu_core::{Asset, AssetKind, Drawing, RasterRole, Scene, VerifiedRichLayout};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};

struct BoardFile {
    path: PathBuf,
    adopted: bool,
}
impl Drop for BoardFile {
    fn drop(&mut self) {
        if !self.adopted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn prepare(
    root: &Path,
    width: u32,
    height: u32,
    work: &capture_delay_work::Work,
) -> Result<(Asset, BoardFile), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32 * 1024 * 1024
    {
        return Err("黑板尺寸超出支持范围".into());
    }
    work.check()?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.board.png"));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "无法创建黑板")?;
    let guard = BoardFile {
        path: path.clone(),
        adopted: false,
    };
    let encoded = (|| {
        RgbaImage::from_pixel(width, height, Rgba([51, 53, 58, 255]))
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|_| "无法保存黑板")?;
        output.sync_all().map_err(|_| "无法保存黑板")
    })();
    drop(output);
    encoded?;
    work.check()?;
    Ok((
        Asset {
            id,
            name: "黑板.png".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into(),
            width: Some(width),
            height: Some(height),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        },
        guard,
    ))
}
fn check_source(current: &Scene, original: &Scene) -> Result<(), String> {
    if current != original
        || current.closed
        || current.frozen
        || current
            .run
            .as_ref()
            .is_some_and(|run| run.status == mewu_core::RunStatus::Running)
    {
        return Err("会话已变化，请重新打开黑板".into());
    }
    Ok(())
}

fn prepare_captures(
    root: &Path,
    scene: &Scene,
    width: u32,
    height: u32,
    work: &capture_delay_work::Work,
) -> Result<(Vec<Drawing>, Vec<VerifiedRichLayout>), String> {
    if scene.regions.is_empty() {
        return Ok((vec![], vec![]));
    }
    let background = scene.background.as_ref().ok_or("截图不存在")?;
    let source_width = f64::from(background.width.ok_or("截图尺寸无效")?);
    let source_height = f64::from(background.height.ok_or("截图尺寸无效")?);
    let scale = (f64::from(width) / source_width).min(f64::from(height) / source_height);
    let offset_x = (f64::from(width) - source_width * scale) / 2.;
    let offset_y = (f64::from(height) - source_height * scale) / 2.;
    let mut drawings = Vec::new();
    let mut layouts = Vec::new();
    let mut total_bytes = 0usize;
    for region in &scene.regions {
        work.check()?;
        let source = region.image_override.as_ref().unwrap_or(background);
        let lease = crate::image_host::SourceLease::open(source, root)?;
        let translation = region
            .translation
            .as_ref()
            .map(|saved| crate::image_host::SourceLease::open(&saved.overlay, root))
            .transpose()?;
        let pixels = crate::assets::crop_from_parts(background, region, root)?.into_rgba8();
        lease.verify()?;
        if let Some(lease) = translation {
            lease.verify()?;
        }
        work.check()?;
        let raster_scale = (6000. / f64::from(pixels.width()))
            .min(6000. / f64::from(pixels.height()))
            .min(
                (16. * 1024. * 1024. / (f64::from(pixels.width()) * f64::from(pixels.height())))
                    .sqrt(),
            );
        let pixels = if raster_scale < 1. {
            let bounded_width = (f64::from(pixels.width()) * raster_scale).floor().max(1.) as u32;
            let bounded_height = (f64::from(pixels.height()) * raster_scale).floor().max(1.) as u32;
            image::DynamicImage::ImageRgba8(pixels)
                .resize(
                    bounded_width,
                    bounded_height,
                    image::imageops::FilterType::Lanczos3,
                )
                .into_rgba8()
        } else {
            pixels
        };
        let ratio = f64::from(pixels.width()) / f64::from(pixels.height());
        let displayed_width = (region.width * scale).min(region.height * scale * ratio);
        let displayed_height = displayed_width / ratio;
        let x = offset_x + region.x * scale + (region.width * scale - displayed_width) / 2.;
        let y = offset_y + region.y * scale + (region.height * scale - displayed_height) / 2.;
        let fence = mewu_core::visual_source_fence(background, region)
            .map_err(|error| error.to_string())?;
        let layout = crate::raster_edit_host::layout(pixels, RasterRole::Extracted, &fence)?;
        total_bytes = total_bytes
            .checked_add(layout.png().len())
            .ok_or("截图对象过大")?;
        if total_bytes > 32 * 1024 * 1024 {
            return Err("截图对象过大".into());
        }
        let mut drawing = crate::raster_edit_host::drawing(&layout, x, y);
        drawing.points[1].x = x + displayed_width;
        drawing.points[1].y = y + displayed_height;
        drawings.push(drawing);
        layouts.push(layout);
    }
    work.check()?;
    Ok((drawings, layouts))
}
#[tauri::command]
pub(crate) async fn create_blackboard(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("黑板来源窗口无效".into());
    }
    recording::ensure_idle(&app)?;
    scroll_host::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let transition = SpaceTransition::try_begin(&host.capturing).ok_or("空间正在切换")?;
    let original = {
        let engine = host.lock()?;
        let snapshot = engine.store.snapshot();
        if snapshot.active_scene_id != scene_id {
            return Err("会话已变化".into());
        }
        let scene = snapshot
            .scenes
            .into_iter()
            .find(|scene| scene.id == scene_id)
            .ok_or("会话不存在")?;
        check_source(&scene, &scene)?;
        scene
    };
    let size = window.inner_size().map_err(|_| "无法读取黑板尺寸")?;
    let work = host.capture_jobs.begin()?.ok_or("空间正在切换")?;
    let _invoke = work.cancel_on_drop();
    let root = host.assets.clone();
    let capture_source = original.clone();
    let (work, _transition, prepared) = tauri::async_runtime::spawn_blocking(move || {
        let prepared = (|| {
            let (asset, file) = prepare(&root, size.width, size.height, &work)?;
            let (captures, layouts) =
                prepare_captures(&root, &capture_source, size.width, size.height, &work)?;
            Ok::<_, String>((asset, file, captures, layouts))
        })();
        (work, transition, prepared)
    })
    .await
    .map_err(|_| "黑板创建中断")?;
    let (asset, mut file, captures, layouts) = prepared?;
    work.check()?;
    host.exit.ensure_new_operation()?;
    let snapshot = {
        let mut engine = host.lock()?;
        work.check()?;
        host.exit.ensure_new_operation()?;
        let snapshot = engine.store.snapshot();
        if snapshot.active_scene_id != scene_id {
            return Err("会话已变化".into());
        }
        check_source(
            snapshot
                .scenes
                .iter()
                .find(|scene| scene.id == scene_id)
                .ok_or("会话不存在")?,
            &original,
        )?;
        engine
            .store
            .create_blackboard_document_with_captures(&scene_id, asset, captures, layouts)
            .map_err(|error| error.to_string())?
    };
    file.adopted = true;
    Ok(crate::publish(&app, &snapshot))
}

fn prepare_preview(
    root: &Path,
    scene: &Scene,
    work: &capture_delay_work::Work,
) -> Result<(Asset, BoardFile), String> {
    work.check()?;
    let background = scene.background.as_ref().ok_or("黑板不存在")?;
    let region = scene
        .regions
        .first()
        .filter(|_| scene.regions.len() == 1)
        .ok_or("黑板区域已变化")?;
    let source = region.image_override.as_ref().unwrap_or(background);
    let lease = crate::image_host::SourceLease::open(source, root)?;
    let translation = region
        .translation
        .as_ref()
        .map(|saved| crate::image_host::SourceLease::open(&saved.overlay, root))
        .transpose()?;
    let pixels = crate::assets::crop_from_parts(background, region, root)?;
    lease.verify()?;
    if let Some(lease) = translation {
        lease.verify()?;
    }
    work.check()?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.board-preview.png"));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "无法保存黑板")?;
    let guard = BoardFile {
        path: path.clone(),
        adopted: false,
    };
    pixels
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| "无法保存黑板")?;
    output.sync_all().map_err(|_| "无法保存黑板")?;
    drop(output);
    work.check()?;
    Ok((
        Asset {
            id,
            name: "黑板.png".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into(),
            width: Some(pixels.width()),
            height: Some(pixels.height()),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        },
        guard,
    ))
}

#[tauri::command]
pub(crate) async fn finish_blackboard(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("黑板来源窗口无效".into());
    }
    recording::ensure_idle(&app)?;
    scroll_host::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let transition = SpaceTransition::try_begin(&host.capturing).ok_or("空间正在切换")?;
    let original = {
        let engine = host.lock()?;
        let snapshot = engine.store.snapshot();
        if snapshot.active_scene_id != scene_id {
            return Err("会话已变化".into());
        }
        let source = snapshot
            .scenes
            .into_iter()
            .find(|scene| scene.id == scene_id)
            .ok_or("黑板不存在")?;
        check_source(&source, &source)?;
        source
    };
    let work = host.capture_jobs.begin()?.ok_or("空间正在切换")?;
    let _invoke = work.cancel_on_drop();
    let root = host.assets.clone();
    let source = original.clone();
    let (work, _transition, prepared) = tauri::async_runtime::spawn_blocking(move || {
        let prepared = prepare_preview(&root, &source, &work);
        (work, transition, prepared)
    })
    .await
    .map_err(|_| "黑板保存中断")?;
    let (asset, mut file) = prepared?;
    work.check()?;
    host.exit.ensure_new_operation()?;
    let snapshot = {
        let mut engine = host.lock()?;
        work.check()?;
        host.exit.ensure_new_operation()?;
        engine
            .store
            .finish_blackboard_document(&original, asset)
            .map_err(|error| error.to_string())?
    };
    file.adopted = true;
    Ok(crate::publish(&app, &snapshot))
}

#[tauri::command]
pub(crate) fn open_blackboard(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    item_id: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("黑板来源窗口无效".into());
    }
    recording::ensure_idle(&app)?;
    scroll_host::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let _transition = SpaceTransition::try_begin(&host.capturing).ok_or("空间正在切换")?;
    let snapshot = host
        .lock()?
        .store
        .open_blackboard_document(&scene_id, &item_id)
        .map_err(|error| error.to_string())?;
    Ok(crate::publish(&app, &snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn selected_regions_become_editable_images_with_ink_zoom_history_and_reopen() {
        let directory =
            std::env::temp_dir().join(format!("mewu-board-captures-{}", uuid::Uuid::new_v4()));
        let root = directory.join("assets");
        std::fs::create_dir_all(&root).unwrap();
        let database = directory.join("spaces.db");
        let jobs = Arc::new(capture_delay_work::CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let (mut capture, capture_file) = prepare(&root, 160, 100, &work).unwrap();
        capture.name = "截图.png".into();
        let mut pixels = RgbaImage::from_pixel(160, 100, Rgba([30, 160, 80, 255]));
        for y in 40..80 {
            for x in 90..150 {
                pixels.put_pixel(x, y, Rgba([30, 100, 220, 255]));
            }
        }
        pixels.save(&capture_file.path).unwrap();
        let mut store = mewu_core::Store::open(&database).unwrap();
        let parent = store.snapshot().active_scene_id;
        store.set_background(&parent, capture.clone()).unwrap();
        for (x, y, width, height) in [(10., 10., 60., 40.), (90., 40., 60., 40.)] {
            store
                .apply(mewu_core::SceneCommand::AddRegion {
                    scene_id: parent.clone(),
                    region: mewu_core::Region {
                        id: uuid::Uuid::new_v4().to_string(),
                        x,
                        y,
                        width,
                        height,
                        ..Default::default()
                    },
                })
                .unwrap();
        }
        let original = store.snapshot().scenes[0].clone();
        let first = &original.regions[0];
        store
            .apply(mewu_core::SceneCommand::AddDrawing {
                scene_id: parent.clone(),
                region_id: first.id.clone(),
                background_id: capture.id.clone(),
                expected_revision: 0,
                drawing: Drawing {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: mewu_core::DrawingKind::Line,
                    color: "#ffffff".into(),
                    stroke_width: 4.,
                    points: vec![
                        mewu_core::DrawingPoint { x: 15., y: 30. },
                        mewu_core::DrawingPoint { x: 65., y: 30. },
                    ],
                    text: None,
                    font_size: None,
                    origin: None,
                    rich: None,
                },
            })
            .unwrap();
        let original = store.snapshot().scenes[0].clone();
        let (board_asset, board_file) = prepare(&root, 320, 200, &work).unwrap();
        let (objects, layouts) = prepare_captures(&root, &original, 320, 200, &work).unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(
            objects[0].points,
            vec![
                mewu_core::DrawingPoint { x: 20., y: 20. },
                mewu_core::DrawingPoint { x: 140., y: 100. }
            ]
        );
        let state = store
            .create_blackboard_document_with_captures(
                &parent,
                board_asset.clone(),
                objects,
                layouts,
            )
            .unwrap();
        let board_id = state.active_scene_id;
        let board = state.scenes.iter().find(|s| s.id == board_id).unwrap();
        let region_id = board.regions[0].id.clone();
        let (preview, preview_file) = prepare_preview(&root, board, &work).unwrap();
        let pixels = image::open(&preview.path).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(50, 60).0, [255, 255, 255, 255]); // Existing capture ink came along.
        assert_eq!(pixels.get_pixel(200, 120).0, [30, 100, 220, 255]);
        assert_eq!(pixels.get_pixel(0, 0).0, [51, 53, 58, 255]);
        drop(preview_file);
        let mut resized = board.regions[0].drawings[1].clone();
        resized.points = vec![
            mewu_core::DrawingPoint { x: 170., y: 40. },
            mewu_core::DrawingPoint { x: 230., y: 80. },
        ];
        store
            .apply(mewu_core::SceneCommand::UpdateDrawing {
                scene_id: board_id.clone(),
                region_id: region_id.clone(),
                background_id: board_asset.id.clone(),
                expected_revision: 1,
                drawing: resized.clone(),
            })
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::UndoDrawing {
                scene_id: board_id.clone(),
                region_id: region_id.clone(),
                background_id: board_asset.id.clone(),
                expected_revision: 2,
            })
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::RedoDrawing {
                scene_id: board_id.clone(),
                region_id: region_id.clone(),
                background_id: board_asset.id.clone(),
                expected_revision: 3,
            })
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::AddDrawing {
                scene_id: board_id.clone(),
                region_id,
                background_id: board_asset.id,
                expected_revision: 4,
                drawing: Drawing {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: mewu_core::DrawingKind::Line,
                    color: "#ff0000".into(),
                    stroke_width: 4.,
                    points: vec![
                        mewu_core::DrawingPoint { x: 175., y: 60. },
                        mewu_core::DrawingPoint { x: 225., y: 60. },
                    ],
                    text: None,
                    font_size: None,
                    origin: None,
                    rich: None,
                },
            })
            .unwrap();
        let state = store.snapshot();
        let board = state.scenes.iter().find(|s| s.id == board_id).unwrap();
        assert_eq!(board.regions[0].drawings[1], resized);
        let (preview, preview_file) = prepare_preview(&root, board, &work).unwrap();
        let pixels = image::open(&preview.path).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(200, 60).0, [255, 0, 0, 255]); // New ink stays above the resized image.
        assert_eq!(pixels.get_pixel(200, 45).0, [30, 100, 220, 255]);
        let edited_region = board.regions[0].clone();
        store.finish_blackboard_document(board, preview).unwrap();
        let state = store.snapshot();
        let returned = state.scenes.iter().find(|s| s.id == parent).unwrap();
        assert_eq!(returned.regions, original.regions);
        let item = returned.items[0].id.clone();
        drop(store);
        let mut store = mewu_core::Store::open(&database).unwrap();
        let state = store.open_blackboard_document(&parent, &item).unwrap();
        assert_eq!(
            state
                .scenes
                .iter()
                .find(|s| s.id == state.active_scene_id)
                .unwrap()
                .regions[0],
            edited_region
        );
        drop(store);
        drop(preview_file);
        drop(board_file);
        drop(capture_file);
        drop(work);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn selected_image_and_board_transaction_roll_back_on_database_failure() {
        let directory =
            std::env::temp_dir().join(format!("mewu-board-rollback-{}", uuid::Uuid::new_v4()));
        let root = directory.join("assets");
        std::fs::create_dir_all(&root).unwrap();
        let database = directory.join("spaces.db");
        let jobs = Arc::new(capture_delay_work::CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let (capture, capture_file) = prepare(&root, 64, 48, &work).unwrap();
        let mut store = mewu_core::Store::open(&database).unwrap();
        let parent = store.snapshot().active_scene_id;
        store.set_background(&parent, capture).unwrap();
        store
            .apply(mewu_core::SceneCommand::AddRegion {
                scene_id: parent.clone(),
                region: mewu_core::Region {
                    id: uuid::Uuid::new_v4().to_string(),
                    x: 2.,
                    y: 3.,
                    width: 32.,
                    height: 24.,
                    ..Default::default()
                },
            })
            .unwrap();
        let before = store.snapshot();
        let (board, board_file) = prepare(&root, 64, 48, &work).unwrap();
        let (objects, layouts) = prepare_captures(&root, &before.scenes[0], 64, 48, &work).unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch("CREATE TRIGGER fail_board BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'test failure');END;").unwrap();
        assert!(store
            .create_blackboard_document_with_captures(&parent, board, objects, layouts)
            .is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM drawing_layouts", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
        drop(store);
        drop(db);
        drop(board_file);
        drop(capture_file);
        drop(work);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn preview_png_contains_saved_ink_and_does_not_mutate_the_editable_background() {
        let root =
            std::env::temp_dir().join(format!("mewu-board-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let jobs = Arc::new(capture_delay_work::CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let (background, original_file) = prepare(&root, 64, 48, &work).unwrap();
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let parent_id = store.snapshot().active_scene_id;
        let snapshot = store
            .create_blackboard_document(&parent_id, background.clone())
            .unwrap();
        let board = snapshot
            .scenes
            .iter()
            .find(|scene| scene.id == snapshot.active_scene_id)
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::AddDrawing {
                scene_id: board.id.clone(),
                region_id: board.regions[0].id.clone(),
                background_id: background.id,
                expected_revision: 0,
                drawing: mewu_core::Drawing {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: mewu_core::DrawingKind::Line,
                    color: "#ffffff".into(),
                    stroke_width: 4.,
                    points: vec![
                        mewu_core::DrawingPoint { x: 8., y: 24. },
                        mewu_core::DrawingPoint { x: 56., y: 24. },
                    ],
                    text: None,
                    font_size: None,
                    origin: None,
                    rich: None,
                },
            })
            .unwrap();
        let snapshot = store.snapshot();
        let board = snapshot
            .scenes
            .iter()
            .find(|scene| scene.id == snapshot.active_scene_id)
            .unwrap();
        let (asset, preview_file) = prepare_preview(&root, board, &work).unwrap();
        let decoded = image::open(&preview_file.path).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (64, 48));
        assert_eq!(decoded.get_pixel(32, 24).0, [255, 255, 255, 255]);
        assert_eq!(decoded.get_pixel(0, 0).0, [51, 53, 58, 255]);
        assert!(image::open(&original_file.path)
            .unwrap()
            .to_rgba8()
            .pixels()
            .all(|pixel| pixel.0 == [51, 53, 58, 255]));
        assert_ne!(asset.id, board.background.as_ref().unwrap().id);
        drop(preview_file);
        drop(original_file);
        drop(work);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn real_blank_png_is_uniform_and_unadopted_output_is_removed() {
        let root = std::env::temp_dir().join(format!("mewu-board-png-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let jobs = Arc::new(capture_delay_work::CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let (asset, guard) = prepare(&root, 64, 48, &work).unwrap();
        let decoded = image::open(&guard.path).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (64, 48));
        assert!(decoded.pixels().all(|pixel| pixel.0 == [51, 53, 58, 255]));
        assert!(asset.path.ends_with(&format!("{}.board.png", asset.id)));
        let path = guard.path.clone();
        drop(guard);
        assert!(!path.exists());
        drop(work);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn canceled_or_oversized_board_creates_no_output() {
        let root = std::env::temp_dir().join(format!("mewu-board-png-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let jobs = Arc::new(capture_delay_work::CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        assert!(prepare(&root, 16384, 16384, &work).is_err());
        jobs.cancel();
        assert!(prepare(&root, 64, 48, &work).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        drop(work);
        std::fs::remove_dir(root).unwrap();
    }
}
