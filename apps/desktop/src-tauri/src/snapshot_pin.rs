// SPDX-License-Identifier: MPL-2.0
//! Native-only receipt for automatically pinning a completed window snapshot.
//! It is never serialized to a WebView and uses the ordinary pin authorization,
//! bounded decoding, immutable staged pixels and window-ready handshake.
use crate::plugins::{PluginContribution, PluginSnapshot, PluginSource, PluginState};
use mewu_core::{Asset, OcrTarget, ReferenceKind, Region, Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Manager};

pub(crate) struct CompletedSnapshotPin {
    target: OcrTarget,
    background: Asset,
    region: Region,
    plugin_revision: u64,
    cancel: Arc<AtomicBool>,
}
impl CompletedSnapshotPin {
    pub(crate) fn prepare(
        snapshot: &Snapshot,
        original: &OcrTarget,
        saved: &Asset,
        plugins: &PluginSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Option<Self> {
        if cancel.load(Ordering::Acquire) || snapshot.active_scene_id != original.scene_id {
            return None;
        }
        let scene = snapshot
            .scenes
            .iter()
            .find(|s| s.id == original.scene_id && !s.closed && !s.frozen)?;
        let background = scene
            .background
            .as_ref()
            .filter(|b| b.id == original.background_id)?;
        let region = scene.regions.iter().find(|r| r.id == original.region_id)?;
        if region.image_override.as_ref() != Some(saved)
            || original.drawing_revision.checked_add(1) != Some(region.drawing_revision)
            || !region.drawings.is_empty()
            || region.ocr.is_some()
            || region.translation.is_some()
            || !scene
                .refs
                .iter()
                .any(|r| r.kind == ReferenceKind::Region && r.id == original.region_id)
        {
            return None;
        }
        let grant = plugins.plugins.iter().find(|p| {
            p.manifest.id == "mewu.pin"
                && p.source == PluginSource::Official
                && p.state == PluginState::Enabled
                && p.error.is_none()
                && p.revision > 0
                && p.manifest
                    .contributions
                    .iter()
                    .any(|c| matches!(c, PluginContribution::Pin { id, .. } if id == "pin"))
        })?;
        Some(Self {
            target: OcrTarget {
                scene_id: original.scene_id.clone(),
                region_id: original.region_id.clone(),
                background_id: background.id.clone(),
                drawing_revision: region.drawing_revision,
                x: region.x,
                y: region.y,
                width: region.width,
                height: region.height,
            },
            background: background.clone(),
            region: region.clone(),
            plugin_revision: grant.revision,
            cancel,
        })
    }
    fn matches(&self, snapshot: &Snapshot) -> bool {
        !self.cancel.load(Ordering::Acquire)
            && snapshot.active_scene_id == self.target.scene_id
            && snapshot.scenes.iter().any(|s| {
                s.id == self.target.scene_id
                    && !s.closed
                    && !s.frozen
                    && s.background.as_ref() == Some(&self.background)
                    && s.regions.iter().find(|r| r.id == self.target.region_id)
                        == Some(&self.region)
            })
    }
    fn is_current(&self, app: &AppHandle) -> Result<bool, String> {
        let host = app.state::<crate::Host>();
        if !host.exit.is_running() {
            return Ok(false);
        }
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        if plugins
            .store
            .pin("mewu.pin", self.plugin_revision, "pin")
            .is_err()
        {
            return Ok(false);
        }
        let engine = host.lock()?;
        Ok(self.matches(&engine.store.snapshot()))
    }
    pub(crate) async fn start(self, app: &AppHandle) -> Result<(), String> {
        if !self.is_current(app)? {
            return Ok(());
        }
        let window = app.get_webview_window("space").ok_or("截图窗口不存在")?;
        // No new command, URL, arbitrary asset path or blanket window grant.
        // run_plugin_pin performs another source/package check before admission
        // and again after rendering and before the new window becomes independent.
        let result = crate::pin_host::run_plugin_pin(
            app.clone(),
            window,
            uuid::Uuid::new_v4().to_string(),
            "mewu.pin".into(),
            self.plugin_revision,
            "pin".into(),
            self.target.clone(),
            None,
        )
        .await;
        if result.is_err() && !self.is_current(app)? {
            return Ok(());
        }
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginManifest, PluginRecord};
    use mewu_core::{AssetKind, SceneCommand, Store};
    fn fixture() -> (Snapshot, OcrTarget, Asset, PluginSnapshot, Arc<AtomicBool>) {
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let background = Asset {
            capture_coordinates: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "screen.png".into(),
            kind: AssetKind::Image,
            path: "screen.png".into(),
            width: Some(800),
            height: Some(600),
            origin_x: Some(-800),
            origin_y: Some(-100),
            scale_factor: Some(1.5),
        };
        store.set_background(&scene_id, background.clone()).unwrap();
        let original = OcrTarget {
            scene_id: scene_id.clone(),
            region_id: uuid::Uuid::new_v4().to_string(),
            background_id: background.id,
            drawing_revision: 0,
            x: 120.,
            y: 100.,
            width: 320.,
            height: 420.,
        };
        store
            .apply(SceneCommand::AddRegion {
                scene_id,
                region: Region {
                    id: original.region_id.clone(),
                    x: original.x,
                    y: original.y,
                    width: original.width,
                    height: original.height,
                    ..Default::default()
                },
            })
            .unwrap();
        let saved = Asset {
            capture_coordinates: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "snapshot.png".into(),
            kind: AssetKind::Image,
            path: "snapshot.png".into(),
            width: Some(640),
            height: Some(2400),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let snapshot = store.set_region_snapshot(&original, saved.clone()).unwrap();
        let plugins = PluginSnapshot {
            revision: 3,
            plugins: vec![PluginRecord {
                manifest: PluginManifest {
                    schema_version: 1,
                    id: "mewu.pin".into(),
                    version: "1.0.0".into(),
                    name: "Pin".into(),
                    description: String::new(),
                    publisher: None,
                    license: "MPL-2.0".into(),
                    host_api: 1,
                    tags: vec![],
                    contributions: vec![PluginContribution::Pin {
                        id: "pin".into(),
                        title: "Pin".into(),
                    }],
                },
                source: PluginSource::Official,
                revision: 3,
                state: PluginState::Enabled,
                has_rollback: false,
                error: None,
            }],
        };
        (
            snapshot,
            original,
            saved,
            plugins,
            Arc::new(AtomicBool::new(false)),
        )
    }
    #[test]
    fn completed_store_receipt_uses_fitted_geometry_new_revision_and_existing_reference() {
        let (snapshot, original, saved, plugins, cancel) = fixture();
        let ticket =
            CompletedSnapshotPin::prepare(&snapshot, &original, &saved, &plugins, cancel).unwrap();
        assert!(ticket.matches(&snapshot));
        assert_eq!(
            ticket.target.drawing_revision,
            original.drawing_revision + 1
        );
        assert_eq!(ticket.target.height, 600. * 0.85);
        assert_eq!(ticket.target.width, 640. * (600. * 0.85 / 2400.));
        assert_eq!(ticket.plugin_revision, 3);
        let viewport = crate::pin_window::Rect {
            x: -800,
            y: -100,
            width: 800,
            height: 600,
        };
        let placement =
            crate::pin_window::initial_rect(&ticket.background, &ticket.region, &saved, viewport)
                .unwrap();
        assert_eq!(placement.height, ticket.target.height.round() as u32 + 24);
        assert_eq!(placement.width, ticket.target.width.round() as u32 + 24);
    }
    #[test]
    fn non_snapshot_region_and_incomplete_registration_cannot_prepare_a_pin() {
        let (snapshot, original, saved, plugins, cancel) = fixture();
        for case in 0..7 {
            let mut changed = snapshot.clone();
            let scene = &mut changed.scenes[0];
            match case {
                0 => scene.refs.clear(),
                1 => scene.regions[0].image_override = None,
                2 => scene.regions[0].drawing_revision = original.drawing_revision,
                3 => scene.regions[0]
                    .image_override
                    .as_mut()
                    .unwrap()
                    .path
                    .push('x'),
                4 => scene.closed = true,
                5 => scene.frozen = true,
                _ => changed.active_scene_id = "different".into(),
            }
            assert!(
                CompletedSnapshotPin::prepare(
                    &changed,
                    &original,
                    &saved,
                    &plugins,
                    cancel.clone()
                )
                .is_none(),
                "case {case}"
            );
        }
    }
    #[test]
    fn disabled_removed_broken_or_community_packages_do_not_auto_pin() {
        let (snapshot, original, saved, plugins, cancel) = fixture();
        for case in 0..7 {
            let mut changed = plugins.clone();
            let record = &mut changed.plugins[0];
            match case {
                0 => record.state = PluginState::Disabled,
                1 => record.state = PluginState::Removed,
                2 => record.error = Some("broken".into()),
                3 => record.revision = 0,
                4 => record.manifest.contributions.clear(),
                5 => {
                    record.source = PluginSource::Local {
                        path: "local.json".into(),
                    }
                }
                _ => changed.plugins.clear(),
            }
            assert!(
                CompletedSnapshotPin::prepare(
                    &snapshot,
                    &original,
                    &saved,
                    &changed,
                    cancel.clone()
                )
                .is_none(),
                "case {case}"
            );
        }
    }
    #[test]
    fn close_switch_cancel_and_same_revision_pixel_source_changes_revoke_pending_receipt() {
        let (snapshot, original, saved, plugins, cancel) = fixture();
        let ticket =
            CompletedSnapshotPin::prepare(&snapshot, &original, &saved, &plugins, cancel.clone())
                .unwrap();
        for case in 0..7 {
            let mut changed = snapshot.clone();
            let scene = &mut changed.scenes[0];
            match case {
                0 => scene.closed = true,
                1 => scene.frozen = true,
                2 => scene.background.as_mut().unwrap().path.push('x'),
                3 => scene.regions[0].x += 1.,
                4 => scene.regions[0]
                    .image_override
                    .as_mut()
                    .unwrap()
                    .id
                    .push('x'),
                5 => scene.regions[0].drawing_revision += 1,
                _ => changed.active_scene_id = "different".into(),
            }
            assert!(!ticket.matches(&changed), "case {case}");
        }
        let mut typing = snapshot.clone();
        typing.scenes[0].draft = "unrelated typing".into();
        assert!(ticket.matches(&typing));
        cancel.store(true, Ordering::Release);
        assert!(!ticket.matches(&snapshot));
        assert!(
            CompletedSnapshotPin::prepare(&snapshot, &original, &saved, &plugins, cancel).is_none()
        );
    }
}
