// SPDX-License-Identifier: MPL-2.0
use crate::{capture_delay_work, recording, scroll_host, Host, HostSnapshot, SpaceTransition};
use image::{ImageFormat, Rgba, RgbaImage};
use mewu_core::{Asset, AssetKind, Scene};
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
    let (work, _transition, prepared) = tauri::async_runtime::spawn_blocking(move || {
        let prepared = prepare(&root, size.width, size.height, &work);
        (work, transition, prepared)
    })
    .await
    .map_err(|_| "黑板创建中断")?;
    let (asset, mut file) = prepared?;
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
            .create_blackboard_document(&scene_id, asset)
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
