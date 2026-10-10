// SPDX-License-Identifier: MPL-2.0
//! A manual input session, independent of model runs and persistent memory.
use crate::{
    plugins::{PluginContribution, PluginSnapshot, PluginState, SpeechEngine},
    speech_backend::{
        DictationResult, VoiceCapabilities, VoiceLanguage, VoicePhase, WorkerConfig, WorkerOutput,
    },
    speech_worker, Host,
};
use mewu_core::Snapshot;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

const CANCELED: &str = "已取消语音输入";
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    scene_id: String,
    agent_id: String,
    background_id: Option<String>,
    run_id: Option<String>,
}
impl Target {
    fn read(snapshot: &Snapshot, scene_id: &str) -> Result<Self, String> {
        let scene = snapshot
            .scenes
            .iter()
            .find(|s| s.id == scene_id && !s.closed && !s.frozen)
            .filter(|s| s.id == snapshot.active_scene_id)
            .ok_or("请先打开要输入的会话")?;
        Ok(Self {
            scene_id: scene.id.clone(),
            agent_id: scene.agent_id.clone(),
            background_id: scene.background.as_ref().map(|b| b.id.clone()),
            run_id: scene.run.as_ref().map(|r| r.id.clone()),
        })
    }
    fn current(&self, snapshot: &Snapshot) -> bool {
        Self::read(snapshot, &self.scene_id).is_ok_and(|value| value == *self)
    }
}
#[derive(Clone)]
struct Grant {
    plugin_id: String,
    revision: u64,
    contribution_id: String,
}
impl Grant {
    fn current(&self, plugins: &PluginSnapshot) -> bool {
        plugins.plugins.iter().any(|p| p.manifest.id == self.plugin_id && p.revision == self.revision
            && p.state == PluginState::Enabled && p.error.is_none() && p.manifest.contributions.iter().any(|c|
            matches!(c, PluginContribution::InputSpeechToText { id, engine: SpeechEngine::WindowsSapi, .. } if id == &self.contribution_id)))
    }
}
struct Job {
    id: String,
    owner: String,
    target: Option<Target>,
    grant: Option<Grant>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    job: Option<Job>,
    recent: HashMap<String, (String, Instant)>,
}
#[derive(Default)]
pub(crate) struct SpeechRegistry(Mutex<State>);
impl SpeechRegistry {
    #[cfg(test)]
    fn start(
        &self,
        id: &str,
        owner: &str,
        target: Option<Target>,
        grant: Option<Grant>,
    ) -> Result<Arc<AtomicBool>, String> {
        self.start_if(id, owner, target, grant, || Ok(()))
    }
    fn start_if(
        &self,
        id: &str,
        owner: &str,
        target: Option<Target>,
        grant: Option<Grant>,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<Arc<AtomicBool>, String> {
        valid_id(id)?;
        let mut state = self.0.lock().map_err(|_| "语音状态不可用")?;
        state
            .recent
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(300));
        if state.recent.contains_key(id) {
            return Err(CANCELED.into());
        }
        if state.job.is_some() {
            return Err("请等待上次语音输入结束".into());
        }
        if state.recent.len() >= 512 {
            return Err("语音请求过多，请稍后重试".into());
        }
        // A transition first raises its atomic gate and then cancels under this
        // same mutex. Either admission observes that gate, or cancellation sees
        // the registered token; an empty-registry cancellation cannot be lost.
        admit()?;
        let token = Arc::new(AtomicBool::new(false));
        state.job = Some(Job {
            id: id.into(),
            owner: owner.into(),
            target,
            grant,
            cancel: token.clone(),
        });
        Ok(token)
    }
    fn finish(&self, id: &str) {
        if let Ok(mut state) = self.0.lock() {
            if state.job.as_ref().is_some_and(|job| job.id == id) {
                let job = state.job.take().unwrap();
                state.recent.insert(job.id, (job.owner, Instant::now()));
            }
        }
    }
    fn cancel(&self, id: &str, owner: &str) -> Result<bool, String> {
        valid_id(id)?;
        let mut state = self.0.lock().map_err(|_| "语音状态不可用")?;
        state
            .recent
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(300));
        if let Some(job) = state.job.as_ref().filter(|job| job.id == id) {
            if job.owner != owner {
                return Err("语音请求不属于此窗口".into());
            }
            job.cancel.store(true, Ordering::Release);
            return Ok(true);
        }
        if let Some((previous, _)) = state.recent.get(id) {
            if previous != owner {
                return Err("语音请求不属于此窗口".into());
            }
        } else {
            if state.recent.len() >= 512 {
                return Err("语音请求过多，请稍后重试".into());
            }
            state
                .recent
                .insert(id.into(), (owner.into(), Instant::now()));
        }
        Ok(false)
    }
    fn invalidate(&self, invalid: impl Fn(&Job) -> bool) -> Vec<String> {
        let Ok(state) = self.0.lock() else {
            speech_worker::cancel_all();
            return vec![];
        };
        state
            .job
            .iter()
            .filter(|job| invalid(job) && !job.cancel.swap(true, Ordering::AcqRel))
            .filter(|job| job.owner == "space" && job.target.is_some())
            .map(|job| job.id.clone())
            .collect()
    }
    fn authorized(&self, id: &str, token: &Arc<AtomicBool>) -> bool {
        !token.load(Ordering::Acquire)
            && self.0.lock().is_ok_and(|state| {
                state
                    .job
                    .as_ref()
                    .is_some_and(|job| job.id == id && Arc::ptr_eq(token, &job.cancel))
            })
    }
}
fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) {
        Ok(())
    } else {
        Err("语音请求标识无效".into())
    }
}
// Resolve the Tauri handle without the Engine lock; query the native visibility
// again inside admission, so a completed hide cannot leave a stale true value.
fn space_handle(window: &tauri::WebviewWindow) -> Result<usize, String> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|handle| handle.0 as usize)
            .map_err(|_| "空间窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("此平台尚不支持语音输入".into())
    }
}
fn ensure_visible(handle: usize) -> Result<(), String> {
    #[cfg(windows)]
    let visible = unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        IsWindow(handle as _) != 0 && IsWindowVisible(handle as _) != 0
    };
    #[cfg(not(windows))]
    let visible = {
        let _ = handle;
        false
    };
    if visible {
        Ok(())
    } else {
        Err("空间已收起".into())
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Invalidated {
    request_id: String,
}
fn emit_invalidated(app: &AppHandle, ids: Vec<String>) {
    for request_id in ids {
        let _ = app.emit_to("space", "voice-invalidated", Invalidated { request_id });
    }
}
pub(crate) fn cancel_owner(app: &AppHandle, owner: &str) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(app, host.speech.invalidate(|job| job.owner == owner));
    }
}
pub(crate) fn cancel_all(app: &AppHandle) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(app, host.speech.invalidate(|_| true));
    }
    speech_worker::cancel_all();
}
pub(crate) fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    emit_invalidated(
        app,
        app.state::<Host>().speech.invalidate(|job| {
            job.target
                .as_ref()
                .is_some_and(|target| !target.current(snapshot))
        }),
    );
}
pub(crate) fn revoke_plugins(app: &AppHandle, snapshot: &PluginSnapshot) {
    emit_invalidated(
        app,
        app.state::<Host>().speech.invalidate(|job| {
            job.grant
                .as_ref()
                .is_some_and(|grant| !grant.current(snapshot))
        }),
    );
}
struct Lease {
    app: AppHandle,
    id: String,
    token: Arc<AtomicBool>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.token.store(true, Ordering::Release);
        self.app.state::<Host>().speech.finish(&self.id);
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    request_id: String,
    scene_id: String,
    phase: VoicePhase,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationReply {
    request_id: String,
    scene_id: String,
    #[serde(flatten)]
    result: DictationResult,
}

#[tauri::command]
pub async fn get_voice_capabilities(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<VoiceCapabilities, String> {
    if !matches!(window.label(), "space" | "settings") {
        return Err("此窗口不能查询语音输入".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let id = uuid::Uuid::new_v4().to_string();
    let token = host.speech.start_if(&id, window.label(), None, None, || {
        host.exit.ensure_running()
    })?;
    let _lease = Lease {
        app: app.clone(),
        id: id.clone(),
        token: token.clone(),
    };
    host.exit.ensure_running()?;
    let result = speech_worker::run(WorkerConfig::Capabilities, token.clone(), |_| {})
        .await
        .map_err(|error| error.to_string())?;
    host.exit.ensure_running()?;
    if !host.speech.authorized(&id, &token) {
        return Err(CANCELED.into());
    }
    match result {
        WorkerOutput::Capabilities(value) => Ok(value),
        _ => Err("无效语音能力结果".into()),
    }
}

#[tauri::command]
pub async fn run_plugin_dictation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    scene_id: String,
    language: VoiceLanguage,
) -> Result<DictationReply, String> {
    if window.label() != "space" {
        return Err("请在对话条中使用语音输入".into());
    }
    crate::recording::ensure_idle(&app)?;
    let native_window = space_handle(&window)?;
    ensure_visible(native_window)?;
    let host = app.state::<Host>();
    let grant = Grant {
        plugin_id,
        revision,
        contribution_id,
    };
    let (target, token) = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins
            .store
            .speech_to_text(&grant.plugin_id, grant.revision, &grant.contribution_id)?;
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        if host.capturing.load(Ordering::Acquire) {
            return Err("截图正在切换".into());
        }
        ensure_visible(native_window)?;
        crate::recording::ensure_idle(&app)?;
        let target = Target::read(&engine.store.snapshot(), &scene_id)?;
        let token = host.speech.start_if(
            &request_id,
            "space",
            Some(target.clone()),
            Some(grant.clone()),
            || {
                host.exit.ensure_running()?;
                if host.capturing.load(Ordering::Acquire) {
                    return Err("截图正在切换".into());
                }
                ensure_visible(native_window)
            },
        )?;
        (target, token)
    };
    let _lease = Lease {
        app: app.clone(),
        id: request_id.clone(),
        token: token.clone(),
    };
    let progress_app = app.clone();
    let progress_id = request_id.clone();
    let progress_scene = scene_id.clone();
    let progress_token = token.clone();
    let output = speech_worker::run(
        WorkerConfig::Dictation { language },
        token.clone(),
        move |phase| {
            let host = progress_app.state::<Host>();
            if host.exit.is_running() && host.speech.authorized(&progress_id, &progress_token) {
                let _ = progress_app.emit_to(
                    "space",
                    "voice-progress",
                    Progress {
                        request_id: progress_id.clone(),
                        scene_id: progress_scene.clone(),
                        phase,
                    },
                );
            }
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let WorkerOutput::Dictation(result) = output else {
        return Err("无效语音识别结果".into());
    };
    if result.text.trim().is_empty()
        || result.text.len() > 32768
        || result.text.chars().count() > 8000
    {
        return Err("语音结果为空或过长".into());
    }
    crate::recording::ensure_idle(&app)?;
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    plugins
        .store
        .speech_to_text(&grant.plugin_id, grant.revision, &grant.contribution_id)?;
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    ensure_visible(native_window)?;
    if host.capturing.load(Ordering::Acquire)
        || !target.current(&engine.store.snapshot())
        || !host.speech.authorized(&request_id, &token)
    {
        return Err(CANCELED.into());
    }
    Ok(DictationReply {
        request_id,
        scene_id,
        result,
    })
}

#[tauri::command]
pub async fn cancel_plugin_dictation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("请在对话条中操作".into());
    }
    let live = app.state::<Host>().speech.cancel(&request_id, "space")?;
    if live && !speech_worker::wait_until_idle(Duration::from_secs(5)).await {
        return Err("语音输入尚未停止".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_gate_and_invalidation_cannot_miss_each_other() {
        let registry = SpeechRegistry::default();
        let transitioning = AtomicBool::new(true);
        let gate = || {
            if transitioning.load(Ordering::Acquire) {
                Err("transition".into())
            } else {
                Ok(())
            }
        };
        // Cancellation already ran against an empty registry.
        assert!(registry.invalidate(|_| true).is_empty());
        let id = uuid::Uuid::new_v4().to_string();
        assert!(registry.start_if(&id, "space", None, None, gate).is_err());
        assert!(registry.0.lock().unwrap().job.is_none());
        transitioning.store(false, Ordering::Release);
        let token = registry.start_if(&id, "space", None, None, gate).unwrap();
        // If registration wins, cancellation observes and permanently revokes it.
        transitioning.store(true, Ordering::Release);
        registry.invalidate(|_| true);
        assert!(!registry.authorized(&id, &token));
        transitioning.store(false, Ordering::Release);
        assert!(!registry.authorized(&id, &token));
    }
    #[test]
    fn transition_after_locked_admission_observes_the_new_token() {
        let registry = SpeechRegistry::default();
        let transitioning = AtomicBool::new(false);
        let gate_entered = std::sync::Barrier::new(2);
        let transition_started = std::sync::Barrier::new(2);
        let id = uuid::Uuid::new_v4().to_string();
        std::thread::scope(|scope| {
            let cancel = scope.spawn(|| {
                gate_entered.wait();
                transitioning.store(true, Ordering::Release);
                transition_started.wait();
                registry.invalidate(|_| true);
            });
            let token = registry
                .start_if(&id, "space", None, None, || {
                    assert!(!transitioning.load(Ordering::Acquire));
                    gate_entered.wait();
                    transition_started.wait();
                    Ok(())
                })
                .unwrap();
            cancel.join().unwrap();
            assert!(transitioning.load(Ordering::Acquire));
            assert!(!registry.authorized(&id, &token));
        });
    }
    #[test]
    fn early_cancel_replay_and_owner_are_fenced() {
        let registry = SpeechRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        assert!(!registry.cancel(&id, "space").unwrap());
        assert!(registry.start(&id, "space", None, None).is_err());
        assert!(registry.cancel(&id, "settings").is_err());
        assert!(valid_id("not-a-request").is_err());
    }
    #[test]
    fn cancel_does_not_free_registry_and_stale_completion_cannot_clear_successor() {
        let registry = SpeechRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let token = registry.start(&id, "space", None, None).unwrap();
        assert!(registry.authorized(&id, &token));
        assert!(registry.cancel(&id, "space").unwrap());
        assert!(!registry.authorized(&id, &token));
        assert!(registry
            .start(&uuid::Uuid::new_v4().to_string(), "space", None, None)
            .is_err());
        registry.finish(&id);
        let next = uuid::Uuid::new_v4().to_string();
        let new_token = registry.start(&next, "space", None, None).unwrap();
        registry.finish(&id);
        assert!(registry.authorized(&next, &new_token));
        assert!(!registry.authorized(&next, &token));
    }
    #[test]
    fn draft_updates_remain_valid_but_scene_agent_and_freeze_invalidate() {
        let store = mewu_core::Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let id = snapshot.active_scene_id.clone();
        let target = Target::read(&snapshot, &id).unwrap();
        snapshot
            .scenes
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .draft = "边说边输入".into();
        assert!(target.current(&snapshot));
        snapshot
            .scenes
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .agent_id = "different".into();
        assert!(!target.current(&snapshot));
        snapshot = store.snapshot();
        snapshot
            .scenes
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .frozen = true;
        assert!(!target.current(&snapshot));
        snapshot = store.snapshot();
        snapshot.active_scene_id = "other".into();
        assert!(!target.current(&snapshot));
    }
    #[test]
    fn revoke_is_monotonic_and_canceled_probe_emits_no_fake_dictation_id() {
        let registry = SpeechRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let token = registry.start(&id, "settings", None, None).unwrap();
        assert!(registry.invalidate(|_| true).is_empty());
        assert!(!registry.authorized(&id, &token));
        assert!(registry.invalidate(|_| false).is_empty());
    }
    #[test]
    fn plugin_grant_requires_same_revision_contribution_and_enabled_bytes() {
        let record = crate::plugins::PluginRecord {
            manifest: serde_json::from_str(include_str!("../../plugins/official-voice.json"))
                .unwrap(),
            source: crate::plugins::PluginSource::Official,
            revision: 2,
            state: PluginState::Enabled,
            has_rollback: false,
            error: None,
        };
        let mut snapshot = PluginSnapshot {
            revision: 2,
            plugins: vec![record],
        };
        let grant = Grant {
            plugin_id: "mewu.voice".into(),
            revision: 2,
            contribution_id: "dictation".into(),
        };
        assert!(grant.current(&snapshot));
        snapshot.plugins[0].revision = 3;
        assert!(!grant.current(&snapshot));
        snapshot.plugins[0].revision = 2;
        snapshot.plugins[0].state = PluginState::Disabled;
        assert!(!grant.current(&snapshot));
        snapshot.plugins[0].state = PluginState::Enabled;
        snapshot.plugins[0].error = Some("包文件变化".into());
        assert!(!grant.current(&snapshot));
        snapshot.plugins[0].error = None;
        snapshot.plugins[0].manifest.contributions.clear();
        assert!(!grant.current(&snapshot));
    }
}
