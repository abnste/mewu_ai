// SPDX-License-Identifier: MPL-2.0
//! User-selected files enter one captured scene as an all-or-nothing batch.
use crate::{assets, Host, HostSnapshot};
use mewu_core::{Asset, AssetKind, Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Default)]
pub struct Transfers {
    importing: Arc<AtomicBool>,
    exporting: Arc<AtomicBool>,
}
struct Permit(Arc<AtomicBool>);
impl Permit {
    fn acquire(flag: &Arc<AtomicBool>) -> Result<Self, String> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "文件操作尚未完成")?;
        Ok(Self(Arc::clone(flag)))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub async fn drain(app: &AppHandle) -> Result<(), String> {
    let host = app.state::<Host>();
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.asset_transfers.importing.load(Ordering::Acquire)
        || host.asset_transfers.exporting.load(Ordering::Acquire)
    {
        if Instant::now() >= deadline {
            return Err("文件操作尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

#[derive(Clone)]
struct ImportTarget {
    scene_id: String,
    agent_id: String,
    background: Option<Asset>,
}
impl ImportTarget {
    fn capture(snapshot: &Snapshot, scene_id: &str) -> Result<Self, String> {
        let scene = snapshot
            .scenes
            .iter()
            .find(|scene| scene.id == scene_id)
            .ok_or("会话不存在")?;
        if snapshot.active_scene_id != scene_id || scene.closed || scene.frozen {
            return Err("会话已切换或关闭，请重新添加文件".into());
        }
        Ok(Self {
            scene_id: scene_id.into(),
            agent_id: scene.agent_id.clone(),
            background: scene.background.clone(),
        })
    }
    fn validate(&self, snapshot: &Snapshot) -> Result<(), String> {
        let current = Self::capture(snapshot, &self.scene_id)?;
        if self.agent_id != current.agent_id || self.background != current.background {
            return Err("当前空间已更改，请重新添加文件".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn import_assets(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("此窗口不能导入素材".into());
    }
    crate::recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("截图尚未完成".into());
    }
    let permit = Permit::acquire(&host.asset_transfers.importing)?;
    host.exit.ensure_running()?;
    let target = ImportTarget::capture(&host.lock()?.store.snapshot(), &scene_id)?;
    let root = host.assets.clone();
    let worker_app = app.clone();
    let worker_target = target.clone();
    let (permit, prepared) = tauri::async_runtime::spawn_blocking(move || {
        // The permit stays with actual blocking work even if its caller disappears.
        worker_app.state::<Host>().exit.ensure_running()?;
        let files = rfd::FileDialog::new()
            .set_parent(&window)
            .add_filter(
                "素材",
                &[
                    "png", "jpg", "jpeg", "webp", "html", "htm", "svg", "txt", "md", "json", "csv",
                ],
            )
            .pick_files()
            .unwrap_or_default();
        if files.is_empty() {
            return Ok((permit, None));
        }
        let host = worker_app.state::<Host>();
        host.exit.ensure_running()?;
        worker_target.validate(&host.lock()?.store.snapshot())?;
        let batch = assets::import_batch(&files, &root)?;
        Ok::<_, String>((permit, Some(batch)))
    })
    .await
    .map_err(|_| "导入任务中断")??;
    crate::recording::ensure_idle(&app)?;
    let (published, imported_images) = {
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        crate::recording::ensure_idle(&app)?;
        if host.capturing.load(Ordering::Acquire) {
            return Err("截图尚未完成，请重新添加文件".into());
        }
        target.validate(&engine.store.snapshot())?;
        if let Some(mut batch) = prepared {
            let imported_images = batch
                .assets
                .iter()
                .filter(|asset| asset.kind == AssetKind::Image)
                .map(|asset| asset.id.clone())
                .collect::<Vec<_>>();
            let snapshot = engine
                .store
                .import_assets(&scene_id, batch.assets.clone())
                .map_err(|error| error.to_string())?;
            batch.commit();
            (crate::publish(&app, &snapshot), imported_images)
        } else {
            (
                HostSnapshot {
                    snapshot: engine.store.snapshot(),
                    revision: host.revision.load(Ordering::Acquire),
                },
                Vec::new(),
            )
        }
    };
    // File transaction is complete. PinRegistry owns any later image work and
    // native windows; never hold Engine or the import-drain permit across it.
    drop(permit);
    if !imported_images.is_empty() {
        crate::import_pin_host::pin_imported_images(
            &app,
            &scene_id,
            &published.snapshot,
            &imported_images,
        )
        .await;
    }
    Ok(published)
}

#[tauri::command]
pub async fn export_text_asset(
    app: AppHandle,
    window: WebviewWindow,
    asset_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能保存素材".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let permit = Permit::acquire(&host.asset_transfers.exporting)?;
    host.exit.ensure_running()?;
    let asset = assets::resolve(&host.lock()?.store.snapshot(), &asset_id).ok_or("素材不存在")?;
    if asset.kind != AssetKind::Text {
        return Err("此素材不是文本文件".into());
    }
    let root = host.assets.clone();
    let managed = host.root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        app.state::<Host>().exit.ensure_running()?;
        let bytes = assets::text_export_bytes(&asset, &root)?;
        app.state::<Host>().exit.ensure_running()?;
        let Some(path) = rfd::FileDialog::new()
            .set_parent(&window)
            .set_file_name(&asset.name)
            .save_file()
        else {
            return Ok(());
        };
        let host = app.state::<Host>();
        host.exit.ensure_running()?;
        if assets::resolve(&host.lock()?.store.snapshot(), &asset_id).as_ref() != Some(&asset) {
            return Err("素材已更改，请重新保存".into());
        }
        assets::save_text_copy(&bytes, &path, &managed)
    })
    .await
    .map_err(|_| "保存任务中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_slot_remains_owned_by_worker_until_its_actual_drop() {
        let transfers = Transfers::default();
        let permit = Permit::acquire(&transfers.importing).unwrap();
        assert!(Permit::acquire(&transfers.importing).is_err());
        let worker = std::thread::spawn(move || {
            assert!(permit.0.load(Ordering::Acquire));
            drop(permit);
        });
        worker.join().unwrap();
        assert!(Permit::acquire(&transfers.importing).is_ok());
    }
    #[test]
    fn import_identity_rejects_scene_agent_background_and_visibility_changes() {
        let store = mewu_core::Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let scene_id = snapshot.active_scene_id.clone();
        let target = ImportTarget::capture(&snapshot, &scene_id).unwrap();
        assert!(target.validate(&snapshot).is_ok());
        snapshot.active_scene_id = "other".into();
        assert!(target.validate(&snapshot).is_err());
        snapshot.active_scene_id = scene_id.clone();
        let index = snapshot
            .scenes
            .iter()
            .position(|s| s.id == scene_id)
            .unwrap();
        snapshot.scenes[index].frozen = true;
        assert!(target.validate(&snapshot).is_err());
        snapshot.scenes[index].frozen = false;
        snapshot.scenes[index].closed = true;
        assert!(target.validate(&snapshot).is_err());
        snapshot.scenes[index].closed = false;
        snapshot.scenes[index].agent_id = "another-agent".into();
        assert!(target.validate(&snapshot).is_err());
        snapshot.scenes[index].agent_id = target.agent_id.clone();
        snapshot.scenes[index].background = Some(Asset {
            id: "new-background".into(),
            name: "synthetic".into(),
            path: "unused".into(),
            kind: AssetKind::Image,
            width: Some(10),
            height: Some(10),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        });
        assert!(target.validate(&snapshot).is_err());
    }
}
