// SPDX-License-Identifier: MPL-2.0
//! Only a successfully committed import batch can create these image pins.
//! This is a host operation, not a renderer command or plugin contribution.
use crate::{pin_host, pin_window};
use mewu_core::{Asset, AssetKind, Snapshot, SpaceItem};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone)]
pub(crate) struct ImportedImageOrigin {
    pub(crate) scene_id: String,
    agent_id: String,
    background: Option<Asset>,
    pub(crate) item: SpaceItem,
}
impl ImportedImageOrigin {
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
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportPinOutcome {
    pub(crate) asset_id: String,
    pub(crate) item_id: String,
    pub(crate) pin_id: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) async fn pin_imported_images(
    app: &AppHandle,
    scene_id: &str,
    committed: &Snapshot,
    imported_asset_ids: &[String],
) -> Vec<ImportPinOutcome> {
    // Resolve only against the exact committed batch snapshot. Never look up a
    // replacement item in the active scene after asynchronous work completes.
    let origins: Vec<_> = imported_asset_ids
        .iter()
        .filter_map(|id| ImportedImageOrigin::capture(committed, scene_id, id))
        .collect();
    let mut outcomes = Vec::with_capacity(origins.len());
    for origin in origins {
        let asset_id = origin.item.asset.id.clone();
        let item_id = origin.item.id.clone();
        let result = pin_host::pin_imported_image(app.clone(), origin).await;
        let (pin_id, error) = match result {
            Ok(id) => (Some(id), None),
            Err(error) => (None, Some(format!("图片已导入，置顶失败：{error}"))),
        };
        let outcome = ImportPinOutcome {
            asset_id,
            item_id,
            pin_id,
            error,
        };
        // Completion is separate from the import transaction: neither a failed
        // pin nor an unavailable UI can undo successfully registered assets.
        let _ = app.emit_to("space", "import-pin-result", &outcome);
        outcomes.push(outcome);
    }
    outcomes
}

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
