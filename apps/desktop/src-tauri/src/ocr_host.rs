// SPDX-License-Identifier: MPL-2.0
//! Local OCR has its own cancellation lifetime; it is not an Agent/model run.
use crate::{
    plugins::{OcrEngine, PluginSnapshot, PluginState},
    Host, HostSnapshot,
};
use mewu_core::{OcrTarget, Scene, Snapshot};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

struct Job {
    owner: String,
    target: OcrTarget,
    plugin_id: String,
    revision: u64,
    cancel: Arc<AtomicBool>,
}
struct Tombstone {
    owner: String,
    at: Instant,
}
#[derive(Default)]
pub(crate) struct OcrRegistry {
    jobs: HashMap<String, Job>,
    recent: HashMap<String, Tombstone>,
}
impl OcrRegistry {
    fn prune(&mut self) {
        self.recent
            .retain(|_, entry| entry.at.elapsed() < Duration::from_secs(300));
    }
    fn start(
        &mut self,
        id: &str,
        owner: &str,
        target: OcrTarget,
        plugin_id: String,
        revision: u64,
    ) -> Result<Arc<AtomicBool>, String> {
        valid_id(id)?;
        self.prune();
        if self.jobs.contains_key(id) || self.recent.contains_key(id) {
            return Err("识别请求已结束，请重新识别".into());
        }
        // Canceled workers retain their slot until they actually exit.
        if self.jobs.len() >= 2 || self.jobs.len() + self.recent.len() >= 512 {
            return Err("请稍后重新识别".into());
        }
        self.cancel_owner(owner);
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(
            id.into(),
            Job {
                owner: owner.into(),
                target,
                plugin_id,
                revision,
                cancel: cancel.clone(),
            },
        );
        Ok(cancel)
    }
    fn cancel(&mut self, id: &str, owner: &str) -> Result<(), String> {
        valid_id(id)?;
        self.prune();
        if let Some(job) = self.jobs.get(id) {
            if job.owner != owner {
                return Err("识别请求不属于此窗口".into());
            }
            job.cancel.store(true, Ordering::Release);
        } else if let Some(entry) = self.recent.get(id) {
            if entry.owner != owner {
                return Err("识别请求不属于此窗口".into());
            }
        } else {
            if self.jobs.len() + self.recent.len() >= 512 {
                return Err("识别请求过多，请稍后重试".into());
            }
            // Cancellation can reach IPC before the corresponding start.
            self.recent.insert(
                id.into(),
                Tombstone {
                    owner: owner.into(),
                    at: Instant::now(),
                },
            );
        }
        Ok(())
    }
    fn finish(&mut self, id: &str) {
        if let Some(job) = self.jobs.remove(id) {
            self.recent.insert(
                id.into(),
                Tombstone {
                    owner: job.owner,
                    at: Instant::now(),
                },
            );
        }
    }
    pub(crate) fn cancel_owner(&self, owner: &str) {
        for job in self.jobs.values().filter(|j| j.owner == owner) {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub(crate) fn cancel_scene(&self, owner: &str, scene_id: &str) {
        for job in self
            .jobs
            .values()
            .filter(|job| job.owner == owner && job.target.scene_id == scene_id)
        {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub(crate) fn reconcile(&self, snapshot: &Snapshot) {
        for job in self.jobs.values() {
            if target_scene(snapshot, &job.target).is_err() {
                job.cancel.store(true, Ordering::Release);
            }
        }
    }
    pub(crate) fn revoke_plugins(&self, snapshot: &PluginSnapshot) {
        for job in self.jobs.values() {
            if !snapshot.plugins.iter().any(|p| {
                p.manifest.id == job.plugin_id
                    && p.revision == job.revision
                    && p.state == PluginState::Enabled
                    && p.error.is_none()
            }) {
                job.cancel.store(true, Ordering::Release);
            }
        }
    }
    fn authorized(&self, id: &str, token: &Arc<AtomicBool>) -> bool {
        !token.load(Ordering::Acquire)
            && self
                .jobs
                .get(id)
                .is_some_and(|job| Arc::ptr_eq(token, &job.cancel))
    }
}
fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) {
        Ok(())
    } else {
        Err("识别请求标识无效".into())
    }
}
fn target_scene<'a>(snapshot: &'a Snapshot, target: &OcrTarget) -> Result<&'a Scene, String> {
    let scene = snapshot
        .scenes
        .iter()
        .find(|s| s.id == target.scene_id && !s.closed)
        .ok_or("会话已关闭")?;
    let background = scene.background.as_ref().ok_or("截图已变更")?;
    let region = scene
        .regions
        .iter()
        .find(|r| r.id == target.region_id)
        .ok_or("选区已移除")?;
    if background.id != target.background_id
        || region.drawing_revision != target.drawing_revision
        || region.x != target.x
        || region.y != target.y
        || region.width != target.width
        || region.height != target.height
    {
        return Err("选区已变更，请重新识别".into());
    }
    Ok(scene)
}
struct Lease {
    app: AppHandle,
    request_id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let host = self.app.state::<Host>();
        if let Ok(mut engine) = host.lock() {
            engine.ocr_jobs.finish(&self.request_id);
        };
    }
}

#[tauri::command]
pub async fn run_plugin_ocr(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: OcrTarget,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("请在截图选区中识别文字".into());
    }
    crate::recording::ensure_idle(&app)?;
    let (background, region, token, root, provider) = {
        let host = app.state::<Host>();
        let runtime = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        let provider = runtime.store.ocr(&plugin_id, revision, &contribution_id)?;
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        if host.capturing.load(Ordering::Acquire)
            || !window.is_visible().map_err(|_| "截图窗口不可用")?
        {
            return Err("截图正在切换，请稍后识别".into());
        }
        let (background, region) = engine
            .store
            .region_for_ocr(&target)
            .map_err(|e| e.to_string())?;
        engine
            .translation_jobs
            .cancel_scene(window.label(), &target.scene_id);
        let token = engine.ocr_jobs.start(
            &request_id,
            window.label(),
            target.clone(),
            plugin_id.clone(),
            revision,
        )?;
        (background, region, token, host.assets.clone(), provider)
    };
    // The lease lives inside the worker, so cancellation/panic cannot release
    // its capacity while native OCR still owns the bitmap.
    let lease = Lease {
        app: app.clone(),
        request_id: request_id.clone(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        if token.load(Ordering::Acquire) {
            return Err("已取消文字识别".into());
        }
        let cropped = crate::assets::crop_from_parts(&background, &region, &root)?.into_rgba8();
        if token.load(Ordering::Acquire) {
            return Err("已取消文字识别".into());
        }
        let document = match provider {
            OcrEngine::Windows => crate::ocr::recognize(cropped, token.clone())?,
        };
        let host = app.state::<Host>();
        // Revalidate installed package bytes as well as the registry revision.
        // Always plugin -> Engine; no global lock is held during recognition.
        let runtime = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        runtime.store.ocr(&plugin_id, revision, &contribution_id)?;
        let mut engine = host.lock()?;
        host.exit.ensure_background()?;
        if !engine.ocr_jobs.authorized(&request_id, &token) {
            return Err("已取消文字识别".into());
        }
        let snapshot = engine
            .store
            .set_region_ocr(&target, document)
            .map_err(|e| e.to_string())?;
        Ok(crate::publish(&app, &snapshot))
    })
    .await
    .map_err(|_| "文字识别任务中断".to_string())?
}

#[tauri::command]
pub fn cancel_plugin_ocr(
    host: tauri::State<'_, Host>,
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("请在截图选区中操作".into());
    }
    host.lock()?.ocr_jobs.cancel(&request_id, window.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> OcrTarget {
        OcrTarget {
            scene_id: "scene".into(),
            region_id: "region".into(),
            background_id: "background".into(),
            drawing_revision: 0,
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        }
    }
    fn start(registry: &mut OcrRegistry, owner: &str) -> (String, Arc<AtomicBool>) {
        let id = uuid::Uuid::new_v4().to_string();
        let token = registry
            .start(&id, owner, target(), "mewu.ocr".into(), 1)
            .unwrap();
        (id, token)
    }
    #[test]
    fn cancellation_before_start_and_replay_cannot_resurrect_a_job() {
        let mut registry = OcrRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        registry.cancel(&id, "space").unwrap();
        assert!(registry
            .start(&id, "space", target(), "mewu.ocr".into(), 1)
            .is_err());
        let (id, token) = start(&mut registry, "space");
        assert!(registry.authorized(&id, &token));
        assert!(registry.cancel(&id, "settings").is_err());
        registry.cancel(&id, "space").unwrap();
        assert!(!registry.authorized(&id, &token));
        registry.finish(&id);
        assert!(registry
            .start(&id, "space", target(), "mewu.ocr".into(), 1)
            .is_err());
    }
    #[test]
    fn cancelled_workers_keep_capacity_until_exit_and_new_work_does_not_refresh_tombstones() {
        let mut registry = OcrRegistry::default();
        let (first, token) = start(&mut registry, "space");
        let (second, _) = start(&mut registry, "space");
        assert!(token.load(Ordering::Acquire));
        assert!(registry
            .start(
                &uuid::Uuid::new_v4().to_string(),
                "space",
                target(),
                "mewu.ocr".into(),
                1
            )
            .is_err());
        registry.finish(&first);
        let at = registry.recent[&first].at;
        registry.cancel(&first, "space").unwrap();
        assert_eq!(registry.recent[&first].at, at);
        let (_, third) = start(&mut registry, "space");
        registry.finish(&second);
        assert!(!third.load(Ordering::Acquire));
    }
    #[test]
    fn revoked_plugins_and_removed_scenes_cancel_without_deleting_completed_documents() {
        let mut registry = OcrRegistry::default();
        let (id, token) = start(&mut registry, "space");
        registry.revoke_plugins(&PluginSnapshot {
            revision: 9,
            plugins: vec![],
        });
        assert!(!registry.authorized(&id, &token));
        registry.finish(&id);
        let (_, token) = start(&mut registry, "space");
        registry.reconcile(&mewu_core::Store::open_in_memory().unwrap().snapshot());
        assert!(token.load(Ordering::Acquire));
    }

    #[test]
    fn scene_cancellation_requires_both_owner_and_scene_identity() {
        // Separate registries avoid start's intentional one-job-per-owner
        // cancellation from masking an incorrect OR/owner-only/scene-only filter.
        for (owner, scene_id, expected) in [
            ("space", "scene", true),
            ("settings", "scene", false),
            ("space", "other-scene", false),
            ("settings", "other-scene", false),
        ] {
            let mut registry = OcrRegistry::default();
            let (id, token) = start(&mut registry, "space");
            let (other_id, other_token) = start(&mut registry, "other-window");
            registry.cancel_scene(owner, scene_id);
            assert_eq!(token.load(Ordering::Acquire), expected);
            assert_eq!(registry.authorized(&id, &token), !expected);
            assert!(registry.authorized(&other_id, &other_token));
            // Matching cancellation keeps the worker's slot until Lease exits.
            assert!(registry.jobs.contains_key(&id));
            assert!(registry.recent.is_empty());
        }
    }
}
