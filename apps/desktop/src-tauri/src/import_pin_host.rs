// SPDX-License-Identifier: MPL-2.0
//! Explicit scene-item pin origins and physical geometry. Import never pins.
use crate::pin_window;
use mewu_core::{Asset, AssetKind, Snapshot, SpaceItem};

#[derive(Clone)]
pub(crate) struct ImportedImageOrigin {
    pub(crate) scene_id: String,
    agent_id: String,
    background: Option<Asset>,
    pub(crate) item: SpaceItem,
}
impl ImportedImageOrigin {
    pub(crate) fn capture_item(
        snapshot: &Snapshot,
        scene_id: &str,
        item_id: &str,
        expected_asset_id: &str,
    ) -> Option<Self> {
        let scene = snapshot.scenes.iter().find(|scene| {
            snapshot.active_scene_id == scene_id
                && scene.id == scene_id
                && !scene.closed
                && !scene.frozen
        })?;
        let item = scene.items.iter().find(|item| {
            item.id == item_id
                && item.asset.id == expected_asset_id
                && matches!(item.asset.kind, AssetKind::Image | AssetKind::Video)
        })?;
        Some(Self {
            scene_id: scene_id.into(),
            agent_id: scene.agent_id.clone(),
            background: scene.background.clone(),
            item: item.clone(),
        })
    }
    #[cfg(test)]
    pub(crate) fn capture(snapshot: &Snapshot, scene_id: &str, asset_id: &str) -> Option<Self> {
        let scene = snapshot.scenes.iter().find(|scene| {
            snapshot.active_scene_id == scene_id
                && scene.id == scene_id
                && !scene.closed
                && !scene.frozen
        })?;
        let item = scene
            .items
            .iter()
            .find(|item| item.asset.id == asset_id && item.asset.kind == AssetKind::Image)?;
        Some(Self {
            scene_id: scene_id.into(),
            agent_id: scene.agent_id.clone(),
            background: scene.background.clone(),
            item: item.clone(),
        })
    }
    pub(crate) fn matches(&self, snapshot: &Snapshot) -> bool {
        snapshot.active_scene_id == self.scene_id
            && snapshot.scenes.iter().any(|scene| {
                scene.id == self.scene_id
                    && !scene.closed
                    && !scene.frozen
                    && scene.agent_id == self.agent_id
                    && scene.background == self.background
                    && scene.items.iter().find(|item| item.id == self.item.id) == Some(&self.item)
            })
    }
    pub(crate) fn rect(
        &self,
        viewport: pin_window::Rect,
        device_scale: f64,
    ) -> Result<pin_window::Rect, String> {
        item_rect(&self.item, self.background.as_ref(), viewport, device_scale)
    }
}

pub(crate) fn item_rect(
    item: &SpaceItem,
    background: Option<&Asset>,
    viewport: pin_window::Rect,
    device_scale: f64,
) -> Result<pin_window::Rect, String> {
    if viewport.width == 0
        || viewport.height == 0
        || ![item.x, item.y, item.width, item.height]
            .iter()
            .all(|value| value.is_finite())
        || item.width <= 0.
        || item.height <= 0.
    {
        return Err("素材位置无效".into());
    }
    let (vw, vh) = (f64::from(viewport.width), f64::from(viewport.height));
    let (mut left, mut top, mut box_width, mut box_height) =
        (item.x * vw, item.y * vh, item.width * vw, item.height * vh);
    if item.asset.kind == AssetKind::Video
        && item.state.as_ref().is_some_and(|state| {
            state
                .get("coordinateSpace")
                .and_then(|value| value.as_str())
                == Some("background")
                && state.get("backgroundId").and_then(|value| value.as_str())
                    == background.map(|asset| asset.id.as_str())
        })
    {
        let background = background.ok_or("视频截图来源不存在")?;
        let (bw, bh) = (
            f64::from(background.width.filter(|v| *v > 0).ok_or("截图宽度无效")?),
            f64::from(background.height.filter(|v| *v > 0).ok_or("截图高度无效")?),
        );
        let native = background
            .capture_coordinates
            .as_ref()
            .is_some_and(|coordinates| {
                coordinates.space == mewu_core::CaptureCoordinateSpace::PhysicalPixels
                    && coordinates.width == bw
                    && coordinates.height == bh
                    && device_scale.is_finite()
                    && device_scale > 0.
                    && (vw - bw).abs() <= device_scale * 0.5
                    && (vh - bh).abs() <= device_scale * 0.5
            });
        let scale = if native { 1. } else { (vw / bw).min(vh / bh) };
        let (bx, by) = if native {
            (0., 0.)
        } else {
            ((vw - bw * scale) / 2., (vh - bh * scale) / 2.)
        };
        (left, top, box_width, box_height) = (
            bx + item.x * bw * scale,
            by + item.y * bh * scale,
            item.width * bw * scale,
            item.height * bh * scale,
        );
    }
    // Match ArtifactCard's intrinsic aspect fit, not its wider saved bounds.
    let ratio = f64::from(item.asset.width.filter(|v| *v > 0).ok_or("素材宽度无效")?)
        / f64::from(item.asset.height.filter(|v| *v > 0).ok_or("素材高度无效")?);
    let width = box_width.min(box_height * ratio).round().max(1.);
    let height = (box_width.min(box_height * ratio) / ratio).round().max(1.);
    if width + 24. > 8192.
        || height + 24. > 8192.
        || (width + 24.) * (height + 24.) > 16. * 1024. * 1024.
    {
        return Err("置顶尺寸超过窗口限制".into());
    }
    let x = f64::from(viewport.x) + left - f64::from(pin_window::PADDING);
    let y = f64::from(viewport.y) + top - f64::from(pin_window::PADDING);
    if x < f64::from(i32::MIN)
        || x > f64::from(i32::MAX)
        || y < f64::from(i32::MIN)
        || y > f64::from(i32::MAX)
    {
        return Err("置顶位置无效".into());
    }
    Ok(pin_window::Rect {
        x: x.round() as i32,
        y: y.round() as i32,
        width: width as u32 + pin_window::PADDING * 2,
        height: height as u32 + pin_window::PADDING * 2,
    })
}

#[cfg(test)]
pub(crate) fn original_rect(
    asset: &Asset,
    viewport: pin_window::Rect,
) -> Result<pin_window::Rect, String> {
    let width = asset
        .width
        .filter(|value| *value != 0)
        .ok_or("图片宽度无效")?
        .checked_add(pin_window::PADDING * 2)
        .ok_or("贴图尺寸无效")?;
    let height = asset
        .height
        .filter(|value| *value != 0)
        .ok_or("图片高度无效")?
        .checked_add(pin_window::PADDING * 2)
        .ok_or("贴图尺寸无效")?;
    if viewport.width == 0 || viewport.height == 0 {
        return Err("空间尺寸无效".into());
    }
    // Source pixels are physical pixels, not CSS pixels or a 900px preview.
    // Preserve original size, including on mixed-DPI/negative-origin displays.
    if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        return Err("原尺寸贴图超过窗口显示限制".into());
    }
    let x = i64::from(viewport.x) + (i64::from(viewport.width) - i64::from(width)).div_euclid(2);
    let y = i64::from(viewport.y) + (i64::from(viewport.height) - i64::from(height)).div_euclid(2);
    Ok(pin_window::Rect {
        x: x.try_into().map_err(|_| "贴图位置无效")?,
        y: y.try_into().map_err(|_| "贴图位置无效")?,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn asset(kind: AssetKind, width: u32, height: u32) -> Asset {
        Asset {
            capture_coordinates: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "synthetic.png".into(),
            kind,
            path: "synthetic.png".into(),
            width: Some(width),
            height: Some(height),
            origin_x: None,
            origin_y: None,
            scale_factor: Some(1.5),
        }
    }
    #[test]
    fn explicit_scene_item_pin_fences_video_edit_and_keeps_original_screen_geometry() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let image = asset(AssetKind::Image, 160, 90);
        let snapshot = store.import_assets(&scene_id, vec![image.clone()]).unwrap();
        let item = &snapshot.scenes[0].items[0];
        let origin =
            ImportedImageOrigin::capture_item(&snapshot, &scene_id, &item.id, &image.id).unwrap();
        assert!(
            ImportedImageOrigin::capture_item(&snapshot, &scene_id, "other-item", &image.id)
                .is_none()
        );
        assert!(
            ImportedImageOrigin::capture_item(&snapshot, &scene_id, &item.id, "other-asset")
                .is_none()
        );
        let mut changed = snapshot.clone();
        changed.scenes[0].items[0].state = Some(std::collections::BTreeMap::from([(
            "blackboardSceneId".into(),
            serde_json::json!("other-board"),
        )]));
        assert!(!origin.matches(&changed));
        let mut video_snapshot = snapshot.clone();
        let video = &mut video_snapshot.scenes[0].items[0];
        video.asset.kind = AssetKind::Video;
        video.asset.scale_factor = None;
        video.video_edit = Some(mewu_core::VideoEdit {
            source_id: image.id.clone(),
            source_duration_ticks: 50_000_000,
            revision: 1,
            range: Some(mewu_core::VideoRange {
                start_ticks: 10_000_000,
                end_ticks: 40_000_000,
            }),
            undo: vec![],
            redo: vec![],
        });
        let video_origin =
            ImportedImageOrigin::capture_item(&video_snapshot, &scene_id, &item.id, &image.id)
                .unwrap();
        video_snapshot.scenes[0].items[0]
            .video_edit
            .as_mut()
            .unwrap()
            .range
            .as_mut()
            .unwrap()
            .start_ticks += 1;
        assert!(!video_origin.matches(&video_snapshot));
        let mut placed = item.clone();
        placed.x = -0.2;
        placed.y = -0.1;
        placed.width = 0.4;
        placed.height = 0.3;
        let rect = item_rect(
            &placed,
            None,
            pin_window::Rect {
                x: -1920,
                y: -100,
                width: 1920,
                height: 1080,
            },
            1.,
        )
        .unwrap();
        assert_eq!(
            rect,
            pin_window::Rect {
                x: -2316,
                y: -220,
                width: 600,
                height: 348
            }
        );
        placed.width = f64::NAN;
        assert!(item_rect(
            &placed,
            None,
            pin_window::Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            },
            1.,
        )
        .is_err());
    }
    #[test]
    fn background_video_pin_projects_letterboxing_and_fits_actual_media() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let snapshot = store
            .import_assets(&scene_id, vec![asset(AssetKind::Image, 160, 90)])
            .unwrap();
        let mut item = snapshot.scenes[0].items[0].clone();
        let background = asset(AssetKind::Image, 1000, 1000);
        item.asset.kind = AssetKind::Video;
        item.x = 0.1;
        item.y = 0.2;
        item.width = 0.5;
        item.height = 0.5;
        item.state = Some(std::collections::BTreeMap::from([
            ("coordinateSpace".into(), serde_json::json!("background")),
            ("backgroundId".into(), serde_json::json!(background.id)),
        ]));
        let viewport = pin_window::Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let rect = item_rect(&item, Some(&background), viewport, 1.25).unwrap();
        assert_eq!(
            rect,
            pin_window::Rect {
                x: -1404,
                y: 204,
                width: 564,
                height: 328
            }
        );
        item.state
            .as_mut()
            .unwrap()
            .insert("backgroundId".into(), serde_json::json!("unrelated"));
        assert_ne!(
            item_rect(&item, Some(&background), viewport, 1.25).unwrap(),
            rect
        );
    }
    #[test]
    fn imported_pin_captures_only_this_batch_image_and_fences_the_full_item() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let image = asset(AssetKind::Image, 200, 100);
        let old = asset(AssetKind::Image, 80, 60);
        store.import_assets(&scene_id, vec![old.clone()]).unwrap();
        let snapshot = store.import_assets(&scene_id, vec![image.clone()]).unwrap();
        let origin = ImportedImageOrigin::capture(&snapshot, &scene_id, &image.id).unwrap();
        assert!(origin.matches(&snapshot));
        assert_eq!(origin.item.asset, image);
        assert_ne!(origin.item.asset.id, old.id);
        assert!(ImportedImageOrigin::capture(&snapshot, &scene_id, "not-in-batch").is_none());
        let mut typing = snapshot.clone();
        typing.scenes[0].draft = "unrelated draft".into();
        assert!(origin.matches(&typing));
        for case in 0..9 {
            let mut changed = snapshot.clone();
            let scene = &mut changed.scenes[0];
            let item = scene
                .items
                .iter_mut()
                .find(|item| item.id == origin.item.id)
                .unwrap();
            match case {
                0 => item.asset.path.push('x'),
                1 => item.asset.id = uuid::Uuid::new_v4().to_string(),
                2 => item.width += 1.,
                3 => item.x += 1.,
                4 => scene.items.retain(|item| item.id != origin.item.id),
                5 => scene.frozen = true,
                6 => scene.closed = true,
                7 => scene.agent_id.push('x'),
                _ => changed.active_scene_id = "other".into(),
            }
            assert!(!origin.matches(&changed), "case {case}");
        }
    }
    #[test]
    fn imported_non_images_are_never_pin_targets() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let mut text = asset(AssetKind::Text, 1, 1);
        text.width = None;
        text.height = None;
        text.scale_factor = None;
        let snapshot = store.import_assets(&scene_id, vec![text.clone()]).unwrap();
        assert!(ImportedImageOrigin::capture(&snapshot, &scene_id, &text.id).is_none());
    }
    #[test]
    fn imported_pin_preserves_physical_original_pixels_and_rejects_silent_thumbnails() {
        let viewport = pin_window::Rect {
            x: -1920,
            y: 50,
            width: 1920,
            height: 1080,
        };
        let rect = original_rect(&asset(AssetKind::Image, 1600, 900), viewport).unwrap();
        assert_eq!(
            rect,
            pin_window::Rect {
                x: -1772,
                y: 128,
                width: 1624,
                height: 924
            }
        );
        assert!(original_rect(&asset(AssetKind::Image, 8200, 100), viewport).is_err());
        assert!(original_rect(&asset(AssetKind::Image, 4096, 4096), viewport).is_err());
        assert!(original_rect(&asset(AssetKind::Image, 0, 100), viewport).is_err());
        let large = original_rect(&asset(AssetKind::Image, 2000, 1200), viewport).unwrap();
        assert_eq!((large.width, large.height), (2024, 1224));
    }
}
