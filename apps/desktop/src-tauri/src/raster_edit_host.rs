// SPDX-License-Identifier: MPL-2.0
//! Screenshot-only local repairs. Never accepts a path, PNG, model output or plugin grant.
use image::{ImageEncoder, RgbaImage};
use mewu_core::{
    Asset, Drawing, DrawingKind, DrawingPoint, RasterRole, Region, RichContent,
    RichRendererIdentity, RunStatus, VerifiedRichLayout, VisualSourceFence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::{atomic::Ordering, Arc, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(4);
static WORKERS: Semaphore = Semaphore::const_new(1);
static JOBS: OnceLock<Arc<crate::capture_delay_work::CaptureJobs>> = OnceLock::new();
pub(crate) fn cancel_all() {
    if let Some(jobs) = JOBS.get() {
        jobs.cancel();
    }
}
pub(crate) async fn drain(timeout: Duration) -> Result<(), String> {
    if let Some(jobs) = JOBS.get() {
        jobs.drain(timeout)
            .await
            .map_err(|_| "修补任务尚未结束".into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
#[path = "raster_edit_store_test.rs"]
mod store_tests;
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Extract,
    Heal,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    scene_id: String,
    background: Asset,
    region: Region,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    snapshot: crate::HostSnapshot,
    selected_id: Option<String>,
}

fn read_target(app: &AppHandle, target: &Target) -> Result<(VisualSourceFence, u64), String> {
    let host = app.state::<crate::Host>();
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("空间正在切换".into());
    }
    crate::recording::ensure_idle(app)?;
    let snapshot = engine.store.snapshot();
    let scene = snapshot
        .scenes
        .iter()
        .find(|s| {
            s.id == target.scene_id && snapshot.active_scene_id == s.id && !s.closed && !s.frozen
        })
        .ok_or("截图会话已关闭或切换")?;
    if scene
        .run
        .as_ref()
        .is_some_and(|run| run.status == RunStatus::Running)
    {
        return Err("请等待当前回答结束".into());
    }
    if scene.background.as_ref() != Some(&target.background)
        || !scene.regions.iter().any(|r| r == &target.region)
    {
        return Err("选区或图层已更改".into());
    }
    Ok((
        mewu_core::visual_source_fence(&target.background, &target.region)
            .map_err(|e| e.to_string())?,
        host.revision.load(Ordering::Acquire),
    ))
}
fn layout(
    pixels: RgbaImage,
    role: RasterRole,
    source: &VisualSourceFence,
) -> Result<VerifiedRichLayout, String> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法生成修补图层")?;
    let result = VerifiedRichLayout::new(
        uuid::Uuid::new_v4().to_string(),
        RichContent::Raster {
            version: 1,
            role,
            source_sha256: source.visual_sha256.clone(),
        },
        RichRendererIdentity {
            id: "mewu.local-raster-edit".into(),
            version: "1".into(),
            font_sha256: format!("{:x}", Sha256::digest(b"")),
            style_version: 1,
        },
        pixels.width(),
        pixels.height(),
        png,
    )
    .map_err(|e| e.to_string())?;
    crate::rich_annotation_host::decode(&result)?;
    Ok(result)
}
fn drawing(layout: &VerifiedRichLayout, x: f64, y: f64) -> Drawing {
    Drawing {
        id: uuid::Uuid::new_v4().to_string(),
        kind: DrawingKind::Rich,
        color: mewu_core::RICH_DRAWING_COLOR.into(),
        stroke_width: mewu_core::RICH_DRAWING_STROKE_WIDTH,
        points: vec![
            DrawingPoint { x, y },
            DrawingPoint {
                x: x + f64::from(layout.width()),
                y: y + f64::from(layout.height()),
            },
        ],
        text: None,
        font_size: None,
        origin: None,
        rich: Some(layout.reference().clone()),
    }
}
#[tauri::command]
pub async fn apply_raster_edit(
    app: AppHandle,
    window: WebviewWindow,
    target: Target,
    mode: Mode,
    points: Vec<DrawingPoint>,
    width: f64,
) -> Result<Receipt, String> {
    if window.label() != "space" {
        return Err("此窗口不能修补截图".into());
    }
    if serde_json::to_vec(&target)
        .map_err(|_| "修补目标无效")?
        .len()
        > 8 * 1024 * 1024
    {
        return Err("修补图层历史过大".into());
    }
    if points.is_empty()
        || points.len() > 4096
        || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
        || !width.is_finite()
    {
        return Err("修补笔划无效".into());
    }
    // Registration precedes the source read, so an exit/capture/hide cancellation
    // cannot fall into an admission/worker-creation gap or revive after abort.
    let jobs = JOBS.get_or_init(|| Arc::new(crate::capture_delay_work::CaptureJobs::default()));
    let work = jobs.begin()?.ok_or("修补正在处理")?;
    let _cancel_invoke = work.cancel_on_drop();
    let (expected, epoch) = read_target(&app, &target)?;
    let request = REQUESTS.try_acquire().map_err(|_| "修补正在处理")?;
    let worker = WORKERS.acquire().await.map_err(|_| "修补处理不可用")?;
    let started = Instant::now();
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let check = || {
            work.check().map_err(|_| "修补已取消".to_string())?;
            if started.elapsed() > Duration::from_secs(30) {
                return Err("修补超时，请缩小范围".into());
            }
            if read_target(&app, &target)? != (expected.clone(), epoch) {
                return Err("空间状态已更改，请重试".into());
            }
            Ok(())
        };
        check()?;
        let mut raster_source = target.region.clone();
        raster_source.drawings.retain(|d| {
            d.kind == DrawingKind::Mosaic
                || d.rich.as_ref().is_some_and(|r| r.kind.is_raster_edit())
        });
        let root = app.state::<crate::Host>().assets.clone();
        let (image, mapping) =
            crate::assets::crop_from_parts_with_mapping(&target.background, &raster_source, &root)?;
        if image.width() > 16384
            || image.height() > 16384
            || u64::from(image.width()) * u64::from(image.height()) > 32 * 1024 * 1024
        {
            return Err("截图尺寸超过修补范围".into());
        }
        check()?;
        let image = image.into_rgba8();
        let local: Vec<_> = points
            .iter()
            .map(|p| DrawingPoint {
                x: p.x - f64::from(mapping.x),
                y: p.y - f64::from(mapping.y),
            })
            .collect();
        if local.iter().any(|p| {
            p.x < 0.
                || p.y < 0.
                || p.x > f64::from(image.width())
                || p.y > f64::from(image.height())
        }) {
            return Err("修补范围超出选区".into());
        }
        let (area, pixels, lifted) = match mode {
            Mode::Extract => {
                if local.len() != 2 {
                    return Err("无痕提取需框选一个范围".into());
                }
                let x = local[0].x.min(local[1].x).floor() as u32;
                let y = local[0].y.min(local[1].y).floor() as u32;
                let right = local[0]
                    .x
                    .max(local[1].x)
                    .ceil()
                    .min(f64::from(image.width())) as u32;
                let bottom = local[0]
                    .y
                    .max(local[1].y)
                    .ceil()
                    .min(f64::from(image.height())) as u32;
                let area = crate::raster_edit_algorithm::Area {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                };
                if area.width < 3 || area.height < 3 {
                    return Err("请框选更大的提取范围".into());
                }
                output_area(area)?;
                let background =
                    crate::raster_edit_algorithm::rectangle_patch(&image, area, &check)?;
                let extracted = crate::raster_edit_algorithm::extract_content(
                    &image,
                    area,
                    &background,
                    &check,
                )?;
                if !extracted.pixels().any(|p| p[3] != 0) {
                    return Err("框选范围没有可提取内容".into());
                }
                (area, background, Some(extracted))
            }
            Mode::Heal => {
                let (area, mask) = crate::raster_edit_algorithm::brush_mask(
                    image.dimensions(),
                    &local,
                    width,
                    &check,
                )?;
                output_area(area)?;
                let pixels =
                    crate::raster_edit_algorithm::masked_patch(&image, area, &mask, &check)?;
                (area, pixels, None)
            }
        };
        check()?;
        let mut layouts = vec![layout(pixels, RasterRole::Repair, &expected)?];
        if let Some(pixels) = lifted {
            layouts.push(layout(pixels, RasterRole::Extracted, &expected)?);
        }
        let x = f64::from(mapping.x + area.x);
        let y = f64::from(mapping.y + area.y);
        let drawings: Vec<_> = layouts.iter().map(|r| drawing(r, x, y)).collect();
        let selected_id = if drawings.len() == 2 {
            Some(drawings[1].id.clone())
        } else {
            None
        };
        check()?;
        let host = app.state::<crate::Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        work.check().map_err(|_| "修补已取消".to_string())?;
        if host.revision.load(Ordering::Acquire) != epoch {
            return Err("空间状态已更改，请重试".into());
        }
        if host.capturing.load(Ordering::Acquire) {
            return Err("空间正在切换".into());
        }
        crate::recording::ensure_idle(&app)?;
        let current = engine.store.snapshot();
        if !current.scenes.iter().any(|s| {
            s.id == target.scene_id
                && s.background.as_ref() == Some(&target.background)
                && s.regions.iter().any(|r| r == &target.region)
        }) {
            return Err("选区或图层已更改".into());
        }
        let next = engine
            .store
            .append_raster_edit(
                &target.scene_id,
                &target.region.id,
                &expected,
                drawings,
                layouts,
            )
            .map_err(|e| e.to_string())?;
        engine.ocr_jobs.reconcile(&next);
        engine.translation_jobs.reconcile(&next);
        Ok(Receipt {
            snapshot: crate::publish(&app, &next),
            selected_id,
        })
    })
    .await
    .map_err(|_| "修补处理已中断")?
}

fn output_area(area: crate::raster_edit_algorithm::Area) -> Result<(), String> {
    if area.width > 6000
        || area.height > 6000
        || u64::from(area.width) * u64::from(area.height) > 16 * 1024 * 1024
    {
        Err("单次修补范围过大，请缩小范围或分笔处理".into())
    } else {
        Ok(())
    }
}
