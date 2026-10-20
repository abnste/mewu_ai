// SPDX-License-Identifier: MPL-2.0
//! User-selected files enter one captured scene as an all-or-nothing batch.
use crate::{assets, Host, HostSnapshot};
use mewu_core::{Asset, AssetKind, Store};
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
struct ImportCancellation(Arc<AtomicBool>);
impl Drop for ImportCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn imported_video_metadata(
    asset: &mut Asset,
    result: crate::video_clip_contract::ClipJobResult,
) -> Result<(), String> {
    use crate::video_clip_contract::ClipJobResult;
    let ClipJobResult::Inspected(metadata) = result else {
        return Err("视频信息无效".into());
    };
    metadata.validate().map_err(|_| "视频格式或尺寸不受支持")?;
    if asset.kind != AssetKind::Video
        || asset.capture_coordinates.is_some()
        || asset.origin_x.is_some()
        || asset.origin_y.is_some()
        || asset.scale_factor.is_some()
    {
        return Err("导入视频信息无效".into());
    }
    asset.width = Some(metadata.width);
    asset.height = Some(metadata.height);
    Ok(())
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
    fn capture(store: &Store, scene_id: &str) -> Result<Self, String> {
        let scene = store.active_scene().map_err(|error| error.to_string())?;
        if scene.id != scene_id || scene.closed || scene.frozen {
            return Err("会话已切换或关闭，请重新添加文件".into());
        }
        Ok(Self {
            scene_id: scene_id.into(),
            agent_id: scene.agent_id.clone(),
            background: scene.background.clone(),
        })
    }
    fn validate(&self, store: &Store) -> Result<(), String> {
        let current = Self::capture(store, &self.scene_id)?;
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
    paths: Option<Vec<String>>,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("此窗口不能导入素材".into());
    }
    let explicit_paths = paths.map(|paths| {
        if paths.is_empty() || paths.len() > 32 || paths.iter().any(|path|
            path.is_empty() || path.len() > 16 * 1024 || path.chars().any(char::is_control)
            || !std::path::Path::new(path).is_absolute()) {
            return Err("请提供 1–32 个绝对文件路径".to_string());
        }
        Ok(paths.into_iter().map(std::path::PathBuf::from).collect::<Vec<_>>())
    }).transpose()?;
    crate::recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("截图尚未完成".into());
    }
    let permit = Permit::acquire(&host.asset_transfers.importing)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let _invoke = ImportCancellation(cancel.clone());
    host.exit.ensure_running()?;
    let (target, authoring_grant, cli_revision) = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        let cli_revision = if explicit_paths.is_some() {
            Some(plugins.store.cli_revision()?.ok_or("命令行插件未启用")?)
        } else { None };
        let engine = host.lock()?;
        (
            ImportTarget::capture(&engine.store, &scene_id)?,
            crate::optional_capabilities::scene_authoring(&engine, &scene_id)?,
            cli_revision,
        )
    };
    let root = host.assets.clone();
    let import_window = window.clone();
    let worker_app = app.clone();
    let worker_target = target.clone();
    let (permit, prepared) = tauri::async_runtime::spawn_blocking(move || {
        // The permit stays with actual blocking work even if its caller disappears.
        worker_app.state::<Host>().exit.ensure_running()?;
        let files = if let Some(files) = explicit_paths { files } else { rfd::FileDialog::new()
            .set_parent(&window)
            .add_filter(
                "素材",
                &[
                    "png", "jpg", "jpeg", "webp", "mp4", "html", "htm", "svg", "txt", "md", "json",
                    "csv",
                ],
            )
            .pick_files()
            .unwrap_or_default() };
        if files.is_empty() {
            return Ok((permit, None));
        }
        let host = worker_app.state::<Host>();
        host.exit.ensure_running()?;
        worker_target.validate(&host.lock()?.store)?;
        let mut batch = assets::import_batch(&files, &root)?;
        for asset in batch
            .assets
            .iter_mut()
            .filter(|asset| asset.kind == AssetKind::Video)
        {
            if cancel.load(Ordering::Acquire) {
                return Err("导入已取消".into());
            }
            host.exit.ensure_running()?;
            worker_target.validate(&host.lock()?.store)?;
            let (_source, _) = assets::open_video(asset, &root)?;
            let path = assets::verified_path(asset, &root)?;
            // This blocking task retains the import permit and staged files
            // until the supervised codec child/Job/IO has actually terminated,
            // including when the invoking WebView disappears during inspection.
            let result = tauri::async_runtime::block_on(crate::video_host::native_job(
                crate::video_clip_contract::ClipJob::Inspect { source: path },
                cancel.clone(),
                Arc::new(|_| {}),
            ))?;
            if cancel.load(Ordering::Acquire) {
                return Err("导入已取消".into());
            }
            imported_video_metadata(asset, result)?;
        }
        host.exit.ensure_running()?;
        worker_target.validate(&host.lock()?.store)?;
        Ok::<_, String>((permit, Some(batch)))
    })
    .await
    .map_err(|_| "导入任务中断")??;
    crate::recording::ensure_idle(&app)?;
    // Query native geometry before taking Engine; the UI thread must never wait
    // for that lock while servicing a window request. Use the current viewport
    // after the dialog, as the window may have moved between displays.
    let logical_size = import_window
        .inner_size()
        .map_err(|_| "无法读取空间尺寸")?
        .to_logical::<u32>(
            import_window
                .scale_factor()
                .map_err(|_| "无法读取空间缩放")?,
        );
    let published = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        if let Some(revision) = cli_revision {
            if plugins.store.cli_revision()? != Some(revision) { return Err("命令行插件已变更".into()); }
        }
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        crate::recording::ensure_idle(&app)?;
        if host.capturing.load(Ordering::Acquire) {
            return Err("截图尚未完成，请重新添加文件".into());
        }
        target.validate(&engine.store)?;
        crate::optional_capabilities::scene_authoring_current(
            &engine,
            &scene_id,
            &authoring_grant,
        )?;
        if let Some(mut batch) = prepared {
            let snapshot = engine
                .store
                .import_assets_in_viewport(
                    &scene_id,
                    batch.assets.clone(),
                    logical_size.width,
                    logical_size.height,
                )
                .map_err(|error| error.to_string())?;
            batch.commit();
            crate::publish(&app, &snapshot)
        } else {
            HostSnapshot {
                snapshot: engine.store.snapshot(),
                revision: host.revision.load(Ordering::Acquire),
            }
        }
    };
    // Import only commits scene objects. Pinning is a separate explicit action.
    drop(permit);
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
    let asset = host
        .lock()?
        .store
        .asset(&asset_id)
        .cloned()
        .ok_or("素材不存在")?;
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
        if host.lock()?.store.asset(&asset_id) != Some(&asset) {
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
    fn cancelled_video_import_does_not_release_its_worker_transfer_early() {
        let transfers = Transfers::default();
        let permit = Permit::acquire(&transfers.importing).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let invoke = ImportCancellation(cancel.clone());
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let (reaped_tx, reaped_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while !cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            cancelled_tx.send(()).unwrap();
            reaped_rx.recv().unwrap();
            drop(permit);
        });
        drop(invoke);
        cancelled_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(Permit::acquire(&transfers.importing).is_err());
        reaped_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(Permit::acquire(&transfers.importing).is_ok());
    }

    #[test]
    fn imported_video_receipt_rejects_bad_or_wrong_media_without_partial_metadata() {
        use crate::video_clip_contract::{ClipJobResult, VideoMetadata};
        let metadata = VideoMetadata {
            duration_ticks: 10_000_000,
            width: 640,
            height: 360,
            frame_rate_numerator: 30,
            frame_rate_denominator: 1,
            has_audio: true,
        };
        let original = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "clip.mp4".into(),
            path: "owned-but-unread-fixture.mp4".into(),
            kind: AssetKind::Video,
            width: None,
            height: None,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
            capture_coordinates: None,
        };
        let mut asset = original.clone();
        assert!(
            imported_video_metadata(&mut asset, ClipJobResult::Rendered(metadata.clone())).is_err()
        );
        assert_eq!(asset, original);
        let mut invalid = metadata.clone();
        invalid.duration_ticks = 0;
        assert!(imported_video_metadata(&mut asset, ClipJobResult::Inspected(invalid)).is_err());
        assert_eq!(asset, original);
        for kind in [AssetKind::Image, AssetKind::Text, AssetKind::Html] {
            let mut wrong = Asset {
                kind,
                ..original.clone()
            };
            let before = wrong.clone();
            assert!(imported_video_metadata(
                &mut wrong,
                ClipJobResult::Inspected(metadata.clone())
            )
            .is_err());
            assert_eq!(wrong, before);
        }
        imported_video_metadata(&mut asset, ClipJobResult::Inspected(metadata)).unwrap();
        assert_eq!(asset.width, Some(640));
        assert_eq!(asset.height, Some(360));
        assert_eq!(asset.path, original.path);
        assert_eq!(asset.id, original.id);
    }
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
        use mewu_core::SceneCommand;
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.active_scene().unwrap().id.clone();
        let target = ImportTarget::capture(&store, &scene_id).unwrap();
        assert!(target.validate(&store).is_ok());
        assert!(ImportTarget::capture(&store, "other").is_err());
        // A real draft transaction is unrelated to the captured import source.
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "仍在编辑的草稿".into(),
            })
            .unwrap();
        assert!(target.validate(&store).is_ok());
        let mut agent = store.snapshot().agents[0].clone();
        agent.id = uuid::Uuid::new_v4().to_string();
        let agent_id = agent.id.clone();
        store.apply(SceneCommand::SaveAgent { agent }).unwrap();
        store
            .apply(SceneCommand::SetAgent {
                scene_id: scene_id.clone(),
                agent_id,
            })
            .unwrap();
        assert!(target.validate(&store).is_err());
        store
            .apply(SceneCommand::SetAgent {
                scene_id: scene_id.clone(),
                agent_id: target.agent_id.clone(),
            })
            .unwrap();
        assert!(target.validate(&store).is_ok());
        store
            .set_background(
                &scene_id,
                Asset {
                    capture_coordinates: None,
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "synthetic".into(),
                    path: "unused".into(),
                    kind: AssetKind::Image,
                    width: Some(10),
                    height: Some(10),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
            )
            .unwrap();
        assert!(target.validate(&store).is_err());
        let target = ImportTarget::capture(&store, &scene_id).unwrap();
        for command in [
            SceneCommand::NewScene,
            SceneCommand::FreezeScene {
                scene_id: scene_id.clone(),
            },
        ] {
            store.apply(command).unwrap();
            assert!(target.validate(&store).is_err());
            store
                .apply(SceneCommand::ActivateScene {
                    scene_id: scene_id.clone(),
                })
                .unwrap();
            assert!(target.validate(&store).is_ok());
        }
        store.apply(SceneCommand::CloseScene { scene_id }).unwrap();
        assert!(target.validate(&store).is_err());
    }
}
