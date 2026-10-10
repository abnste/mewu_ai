// SPDX-License-Identifier: MPL-2.0
//! Small recording preference actor. The recording host owns capture and device lifetime.
pub use crate::recording_preferences::{RecordingAudioGrant, RecordingAudioSelection};
use crate::{
    plugins::{PluginContribution, PluginSnapshot, PluginState, RecordingAudioEngine},
    recording_backend::AudioMode,
    recording_preferences::{self as prefs, Loaded, Preferences, PreferencesFile, Stamp},
    Host,
};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingAudioState {
    pub revision: u64,
    pub sequence: u64,
    pub mode: AudioMode,
    pub grant: Option<RecordingAudioGrant>,
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
impl RecordingAudioState {
    fn selection(&self) -> RecordingAudioSelection {
        RecordingAudioSelection {
            revision: self.revision,
            mode: self.mode,
            grant: self.grant.clone(),
        }
    }
}

/// The snapshot comes from PluginStore's checked package view. Never acquires
/// plugins, Engine, RecordingState, a window handle, or any device itself.
pub fn grant_is_current(grant: &RecordingAudioGrant, plugins: &PluginSnapshot) -> bool {
    if grant.plugin_id == crate::plugins::CORE_RECORDING {
        return crate::plugins::core_recording_grant(
            &grant.plugin_id,
            grant.revision,
            &grant.contribution_id,
        ) && grant.contribution_id == "audio";
    }
    grant.validate().is_ok() && plugins.plugins.iter().any(|record| {
        record.manifest.id == grant.plugin_id && record.revision == grant.revision
            && record.state == PluginState::Enabled && record.error.is_none()
            && record.manifest.contributions.iter().any(|contribution| matches!(contribution,
                PluginContribution::RecordingAudio { id, engine:RecordingAudioEngine::WindowsWasapi, .. }
                    if id == &grant.contribution_id))
    })
}
fn effective(preferences: &Preferences, plugins: &PluginSnapshot) -> RecordingAudioSelection {
    let core_audio = || RecordingAudioGrant {
        plugin_id: crate::plugins::CORE_RECORDING.into(),
        revision: 1,
        contribution_id: "audio".into(),
    };
    // Missing preferences are a default, not a saved silent recording choice.
    // Keep the original bounded bytes/stamp so initialization writes nothing.
    if preferences.revision == 0 {
        return RecordingAudioSelection {
            revision: 0,
            mode: AudioMode::System,
            grant: Some(core_audio()),
        };
    }
    if preferences.mode != AudioMode::Mute
        && preferences.grant.as_ref().is_some_and(|grant| {
            grant.validate().is_ok()
                && grant.contribution_id == "audio"
                && plugins.plugins.iter().any(|record| {
                    record.manifest.id == grant.plugin_id
                        && grant.revision <= record.revision
                        && crate::plugins::legacy_official_audio_record(record)
                })
        })
    {
        return RecordingAudioSelection {
            revision: preferences.revision,
            mode: preferences.mode,
            grant: Some(core_audio()),
        };
    }
    if preferences.mode != AudioMode::Mute
        && preferences
            .grant
            .as_ref()
            .is_some_and(|grant| grant_is_current(grant, plugins))
    {
        RecordingAudioSelection {
            revision: preferences.revision,
            mode: preferences.mode,
            grant: preferences.grant.clone(),
        }
    } else {
        RecordingAudioSelection {
            revision: preferences.revision,
            mode: AudioMode::Mute,
            grant: None,
        }
    }
}
struct Pending {
    grant: Option<RecordingAudioGrant>,
    revoked: Arc<AtomicBool>,
}
struct Data {
    loaded: Loaded,
    fault: Option<prefs::Error>,
    warning: bool,
    view: RecordingAudioState,
    pending: Option<Pending>,
}
impl Data {
    fn new(loaded: Result<Loaded, prefs::Error>, plugins: &PluginSnapshot) -> Self {
        let (loaded, fault) = match loaded {
            Ok(value) => (value, None),
            Err(error) => (Loaded::missing(), Some(error)),
        };
        let mut data = Self {
            loaded,
            fault,
            warning: false,
            pending: None,
            view: RecordingAudioState {
                revision: 0,
                sequence: 0,
                mode: AudioMode::Mute,
                grant: None,
                editable: false,
                message: None,
            },
        };
        data.refresh(plugins);
        data
    }
    fn refresh(&mut self, plugins: &PluginSnapshot) -> bool {
        if let Some(pending) = &self.pending {
            if pending
                .grant
                .as_ref()
                .is_some_and(|grant| !grant_is_current(grant, plugins))
            {
                pending.revoked.store(true, Ordering::Release);
            }
        }
        let selection = effective(&self.loaded.preferences, plugins);
        let unavailable = self.fault.is_some();
        let message = self.fault.map(|e| e.to_string()).or_else(|| {
            if self.loaded.preferences.mode != AudioMode::Mute && selection.mode == AudioMode::Mute
            {
                Some("请重新选择声源".into())
            } else if self.warning {
                Some("设置已生效，但存储同步状态未确认".into())
            } else {
                None
            }
        });
        let next = RecordingAudioState {
            revision: selection.revision,
            sequence: self.view.sequence,
            mode: if unavailable {
                AudioMode::Mute
            } else {
                selection.mode
            },
            grant: if unavailable { None } else { selection.grant },
            editable: !unavailable,
            message,
        };
        if next == self.view {
            return false;
        }
        self.view = RecordingAudioState {
            sequence: self
                .view
                .sequence
                .checked_add(1)
                .expect("recording preference view sequence exhausted"),
            ..next
        };
        true
    }
}
struct Inner {
    busy: AtomicBool,
    abort: AtomicBool,
    faulted: AtomicBool,
    file: Option<PreferencesFile>,
    data: Mutex<Data>,
}
#[derive(Clone)]
pub struct RecordingAudioRuntime(Arc<Inner>);
impl RecordingAudioRuntime {
    fn view(&self) -> RecordingAudioState {
        self.0
            .data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .view
            .clone()
    }
    fn claim(&self) -> Result<Work, String> {
        if self.0.faulted.load(Ordering::Acquire) {
            return Err("录屏声音状态不可用，请重启后检查".into());
        }
        self.0
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "录屏声音设置正在保存")?;
        self.0.abort.store(false, Ordering::Release);
        Ok(Work(self.0.clone()))
    }
}
struct Work(Arc<Inner>);
impl Drop for Work {
    fn drop(&mut self) {
        let mut data = self.0.data.lock().unwrap_or_else(|e| e.into_inner());
        data.pending = None;
        if std::thread::panicking() {
            self.0.faulted.store(true, Ordering::Release);
            data.fault = Some(prefs::Error::Indeterminate);
            data.view.editable = false;
            data.view.mode = AudioMode::Mute;
            data.view.grant = None;
            data.view.message = Some(prefs::Error::Indeterminate.to_string());
            data.view.sequence = data.view.sequence.saturating_add(1);
        }
        drop(data);
        self.0.busy.store(false, Ordering::Release);
    }
}
fn emit(app: &AppHandle, view: RecordingAudioState) {
    let _ = app.emit_to("space", "recording-audio-state", &view);
    let _ = app.emit_to("settings", "recording-audio-state", &view);
}
fn runtime(app: &AppHandle) -> Result<RecordingAudioRuntime, String> {
    app.try_state::<RecordingAudioRuntime>()
        .map(|v| v.inner().clone())
        .ok_or_else(|| "录屏声音设置尚未就绪".into())
}
pub fn initialize(app: &AppHandle, root: &Path) -> Result<(), String> {
    if app.try_state::<RecordingAudioRuntime>().is_some() {
        return Err("录屏声音设置已初始化".into());
    }
    let (file, load) = match PreferencesFile::open(root) {
        Ok(file) => {
            let load = file.load();
            (Some(file), load)
        }
        Err(error) => (None, Err(error)),
    };
    let host = app.state::<Host>();
    let snapshot = host
        .plugins
        .lock()
        .map_err(|_| "插件存储需要重新启动")?
        .store
        .snapshot()?;
    app.manage(RecordingAudioRuntime(Arc::new(Inner {
        busy: AtomicBool::new(false),
        abort: AtomicBool::new(false),
        faulted: AtomicBool::new(false),
        file,
        data: Mutex::new(Data::new(load, &snapshot)),
    })));
    Ok(())
}
pub fn reconcile(app: &AppHandle, plugins: &PluginSnapshot) {
    let Some(runtime) = app.try_state::<RecordingAudioRuntime>() else {
        return;
    };
    let next = {
        let mut data = runtime.0.data.lock().unwrap_or_else(|e| e.into_inner());
        data.refresh(plugins).then(|| data.view.clone())
    };
    if let Some(next) = next {
        emit(app, next);
    }
}
pub fn invalidate_all(app: &AppHandle) {
    let Some(runtime) = app.try_state::<RecordingAudioRuntime>() else {
        return;
    };
    runtime.0.abort.store(true, Ordering::Release);
    let data = runtime.0.data.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pending) = &data.pending {
        pending.revoked.store(true, Ordering::Release);
    }
}
pub async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    let Some(runtime) = app
        .try_state::<RecordingAudioRuntime>()
        .map(|r| r.inner().clone())
    else {
        return Ok(());
    };
    invalidate_all(app);
    let start = Instant::now();
    while runtime.0.busy.load(Ordering::Acquire) {
        if start.elapsed() >= timeout {
            return Err("录屏声音设置尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

/// Call inside the recording host's plugins -> Engine -> RecordingState admission
/// section. Capturing is intentionally permitted here: that host owns the transition.
pub fn validate_selection(
    app: &AppHandle,
    selection: &RecordingAudioSelection,
    plugins: &PluginSnapshot,
) -> Result<AudioMode, String> {
    let runtime = runtime(app)?;
    validate_runtime_selection(&runtime, selection, plugins)
}
fn validate_runtime_selection(
    runtime: &RecordingAudioRuntime,
    selection: &RecordingAudioSelection,
    plugins: &PluginSnapshot,
) -> Result<AudioMode, String> {
    selection.validate().map_err(|_| "录屏声音选择无效")?;
    if runtime.0.busy.load(Ordering::Acquire) {
        return Err("录屏声音设置正在保存".into());
    }
    if runtime.0.faulted.load(Ordering::Acquire) {
        return Err("录屏声音状态不可用，请重启后检查".into());
    }
    let data = runtime.0.data.lock().unwrap_or_else(|e| e.into_inner());
    if data.fault.is_some() {
        // Broken optional preferences cannot remove the existing silent recorder.
        if *selection == data.view.selection() && selection.mode == AudioMode::Mute {
            return Ok(AudioMode::Mute);
        }
        return Err("录屏声音配置不可用".into());
    }
    if effective(&data.loaded.preferences, plugins) != *selection {
        return Err("录屏声音设置已改变，请重新选择".into());
    }
    Ok(selection.mode)
}
fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|value| value.0 as usize)
            .map_err(|_| "录屏声音窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("当前平台尚不支持录屏声音".into())
    }
}
fn visible(handle: usize) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        handle != 0 && IsWindow(handle as _) != 0 && IsWindowVisible(handle as _) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        false
    }
}
fn admission(app: &AppHandle, handle: usize) -> bool {
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    host.exit.is_running()
        && !host.capturing.load(Ordering::Acquire)
        && visible(handle)
        && crate::recording::ensure_idle(app).is_ok()
}
fn allowed(app: &AppHandle, inner: &Inner, handle: usize, revoked: &AtomicBool) -> bool {
    !inner.abort.load(Ordering::Acquire)
        && !inner.faulted.load(Ordering::Acquire)
        && !revoked.load(Ordering::Acquire)
        && admission(app, handle)
}
#[tauri::command]
pub fn get_recording_audio(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<RecordingAudioState, String> {
    if !matches!(window.label(), "space" | "settings") {
        return Err("此窗口不能读取录屏声音".into());
    }
    Ok(runtime(&app)?.view())
}
#[tauri::command]
pub async fn set_recording_audio(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    mode: AudioMode,
    grant: Option<RecordingAudioGrant>,
) -> Result<RecordingAudioState, String> {
    if !matches!(window.label(), "space" | "settings") {
        return Err("此窗口不能修改录屏声音".into());
    }
    prefs::validate_choice(mode, grant.as_ref()).map_err(|_| "录屏声音选择无效")?;
    let handle = window_handle(&window)?; // Never inside plugins, Engine or our data mutex.
    if !admission(&app, handle) {
        return Err("当前无法修改录屏声音".into());
    }
    let runtime = runtime(&app)?;
    let work = runtime.claim()?;
    let revoked = Arc::new(AtomicBool::new(false));
    let loaded = {
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        if !admission(&app, handle) {
            return Err("当前无法修改录屏声音".into());
        }
        if let Some(grant) = &grant {
            plugins.store.recording_audio(
                &grant.plugin_id,
                grant.revision,
                &grant.contribution_id,
            )?;
        }
        let mut data = runtime.0.data.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = data.fault {
            return Err(error.to_string());
        }
        if expected_revision != data.loaded.preferences.revision {
            return Err("录屏声音设置已改变，请载入最新设置".into());
        }
        data.pending = Some(Pending {
            grant: grant.clone(),
            revoked: revoked.clone(),
        });
        data.loaded.clone()
    };
    let inner = runtime.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _work = work; // Dropped only after this actual worker completes, not with invoke Future.
        let file = inner
            .file
            .as_ref()
            .ok_or_else(|| prefs::Error::Access.to_string())?;
        let previous = &loaded.preferences;
        let noop = previous.mode == mode
            && previous.grant == grant
            && !matches!(loaded.stamp, Stamp::Missing);
        let result: Result<Option<(Preferences, prefs::Commit)>, prefs::Error> = (|| {
            if !allowed(&app, &inner, handle, &revoked) {
                return Err(prefs::Error::NotAllowed);
            }
            if noop {
                file.check_current(&loaded.stamp)?;
                return Ok(None);
            }
            if previous.revision >= prefs::MAX_REVISION {
                return Err(prefs::Error::RevisionExhausted);
            }
            let next = Preferences {
                version: 1,
                revision: previous.revision + 1,
                mode,
                grant,
            };
            let receipt = file.commit(&loaded.stamp, &next, || {
                allowed(&app, &inner, handle, &revoked)
            })?;
            Ok(Some((next, receipt)))
        })();
        // No file I/O, window getter or recording lock below this point. Refresh
        // against the latest catalog while holding the same plugin admission lock.
        let host = app.state::<Host>();
        // Even a catalog fault after rename must retain the committed preference
        // in this actor. Fail audio authority closed instead of pretending that
        // the file rolled back and later starting from an obsolete choice.
        let plugins = host.plugins.lock().ok();
        let snapshot = plugins
            .as_ref()
            .and_then(|p| p.store.snapshot().ok())
            .unwrap_or(PluginSnapshot {
                revision: 0,
                plugins: Vec::new(),
            });
        let (view, error) = {
            let mut data = inner.data.lock().unwrap_or_else(|e| e.into_inner());
            let error = match result {
                Ok(Some((preferences, receipt))) => {
                    data.loaded = Loaded {
                        preferences,
                        stamp: receipt.stamp,
                    };
                    data.warning = receipt.durability_warning;
                    None
                }
                Ok(None) => None,
                Err(error) => {
                    if matches!(
                        error,
                        prefs::Error::Corrupt
                            | prefs::Error::UnsafePath
                            | prefs::Error::Conflict
                            | prefs::Error::Access
                            | prefs::Error::Indeterminate
                    ) {
                        data.fault = Some(error);
                    }
                    Some(error.to_string())
                }
            };
            data.refresh(&snapshot);
            (data.view.clone(), error)
        };
        drop(plugins);
        emit(&app, view.clone());
        match error {
            Some(error) => Err(error),
            None => Ok(view),
        }
    })
    .await
    .map_err(|_| "录屏声音保存任务中断，请重启后检查".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> PluginSnapshot {
        let manifest = crate::plugins::parse_manifest(include_bytes!(
            "../../plugins/migrations/official-recording-1.0.0.json"
        ))
        .unwrap();
        PluginSnapshot {
            revision: 1,
            plugins: vec![crate::plugins::PluginRecord {
                manifest,
                revision: 1,
                state: PluginState::Enabled,
                source: crate::plugins::PluginSource::Official,
                has_rollback: false,
                error: None,
            }],
        }
    }
    fn grant() -> RecordingAudioGrant {
        RecordingAudioGrant {
            plugin_id: "mewu.recording".into(),
            revision: 1,
            contribution_id: "audio".into(),
        }
    }
    fn loaded() -> Loaded {
        Loaded {
            preferences: Preferences {
                version: 1,
                revision: 1,
                mode: AudioMode::Both,
                grant: Some(grant()),
            },
            stamp: Stamp::Missing,
        }
    }
    fn third_party_snapshot() -> PluginSnapshot {
        let mut plugins = snapshot();
        plugins.plugins[0].manifest.id = "example.recording".into();
        plugins.plugins[0].source = crate::plugins::PluginSource::Github {
            repository: "example/recording".into(),
            commit: "a".repeat(40),
            path: "plugin.json".into(),
        };
        plugins
    }
    fn third_party_loaded() -> Loaded {
        let mut saved = loaded();
        saved.preferences.grant.as_mut().unwrap().plugin_id = "example.recording".into();
        saved
    }
    #[test]
    fn missing_preferences_default_to_system_core_without_creating_saved_bytes() {
        let data = Data::new(Ok(Loaded::missing()), &snapshot());
        assert_eq!(data.view.mode, AudioMode::System);
        assert_eq!(
            data.view.grant.as_ref().unwrap().plugin_id,
            crate::plugins::CORE_RECORDING
        );
        assert_eq!(data.view.revision, 0);
        assert_eq!(data.loaded.stamp, Stamp::Missing);
        assert_eq!(data.loaded.preferences, Preferences::missing());
        let mut saved_mute = Loaded::missing();
        saved_mute.preferences.revision = 1;
        let muted = Data::new(Ok(saved_mute), &snapshot());
        assert_eq!(muted.view.mode, AudioMode::Mute);
        assert!(muted.view.grant.is_none());
    }
    #[test]
    fn canonical_saved_audio_modes_project_to_core_without_rewriting_or_enabling_microphone() {
        for mode in [AudioMode::System, AudioMode::Microphone, AudioMode::Both] {
            let mut saved = loaded();
            saved.preferences.mode = mode;
            let bytes = serde_json::to_vec(&saved.preferences).unwrap();
            saved.stamp = Stamp::Present(bytes.clone());
            let mut plugins = snapshot();
            let mut data = Data::new(Ok(saved.clone()), &plugins);
            assert_eq!(data.view.mode, mode);
            let projected = data.view.grant.clone().unwrap();
            assert_eq!(projected.plugin_id, crate::plugins::CORE_RECORDING);
            let revoked = Arc::new(AtomicBool::new(false));
            data.pending = Some(Pending {
                grant: Some(projected.clone()),
                revoked: revoked.clone(),
            });
            for (state, revision) in [(PluginState::Disabled, 2), (PluginState::Removed, 3)] {
                plugins.plugins[0].state = state;
                plugins.plugins[0].revision = revision;
                data.refresh(&plugins);
                assert_eq!(data.view.mode, mode);
                assert_eq!(data.view.grant, Some(projected.clone()));
                assert!(!revoked.load(Ordering::Acquire));
                assert_eq!(data.loaded.stamp, Stamp::Present(bytes.clone()));
                assert_eq!(serde_json::to_vec(&data.loaded.preferences).unwrap(), bytes);
            }
        }
        let empty = PluginSnapshot {
            revision: 0,
            plugins: vec![],
        };
        let default = Data::new(Ok(Loaded::missing()), &empty);
        assert_eq!(default.view.mode, AudioMode::System);
        let mut core = loaded();
        core.preferences.grant = default.view.grant;
        assert_eq!(Data::new(Ok(core), &empty).view.mode, AudioMode::Both);
        let mut legacy = snapshot();
        legacy.plugins[0].manifest = crate::plugins::parse_manifest(include_bytes!(
            "../../plugins/migrations/official-recording-audio-1.0.0.json"
        ))
        .unwrap();
        let mut saved = loaded();
        saved.preferences.grant.as_mut().unwrap().plugin_id = "mewu.recording-audio".into();
        assert_eq!(Data::new(Ok(saved), &legacy).view.mode, AudioMode::Both);
    }
    #[test]
    fn forged_core_or_noncanonical_legacy_audio_authority_cannot_preserve_a_saved_mode() {
        for invalid in ["future", "kind", "source", "manifest", "file"] {
            let mut saved = loaded();
            let mut plugins = snapshot();
            match invalid {
                "future" => saved.preferences.grant.as_mut().unwrap().revision = 99,
                "kind" => {
                    saved.preferences.grant.as_mut().unwrap().contribution_id = "record".into()
                }
                "source" => {
                    plugins.plugins[0].source = crate::plugins::PluginSource::Github {
                        repository: "example/recording".into(),
                        commit: "a".repeat(40),
                        path: "plugin.json".into(),
                    }
                }
                "manifest" => plugins.plugins[0].manifest.description.push_str(" changed"),
                "file" => plugins.plugins[0].error = Some("tampered".into()),
                _ => unreachable!(),
            }
            let data = Data::new(Ok(saved), &plugins);
            // A still-current third-party declared grant is left on its original
            // path. It never becomes core authority from a forged provenance.
            if invalid == "source" || invalid == "manifest" {
                assert_ne!(
                    data.view.grant.as_ref().unwrap().plugin_id,
                    crate::plugins::CORE_RECORDING
                );
            } else {
                assert_eq!(data.view.mode, AudioMode::Mute);
            }
        }
        let empty = PluginSnapshot {
            revision: 0,
            plugins: vec![],
        };
        for revision in [0, 2, u64::MAX] {
            let grant = RecordingAudioGrant {
                plugin_id: crate::plugins::CORE_RECORDING.into(),
                revision,
                contribution_id: "audio".into(),
            };
            assert!(!grant_is_current(&grant, &empty));
        }
        for contribution_id in ["record", "trim", "gif", "other"] {
            assert!(!grant_is_current(
                &RecordingAudioGrant {
                    plugin_id: crate::plugins::CORE_RECORDING.into(),
                    revision: 1,
                    contribution_id: contribution_id.into()
                },
                &empty
            ));
        }
    }
    #[test]
    fn merged_bundle_never_rebinds_an_old_audio_authority() {
        let mut old = loaded();
        old.preferences.grant.as_mut().unwrap().plugin_id = "mewu.recording-audio".into();
        let before = serde_json::to_vec(&old.preferences).unwrap();
        let stamp = old.stamp.clone();
        let data = Data::new(Ok(old), &snapshot());
        assert_eq!(data.view.mode, AudioMode::Mute);
        assert!(data.view.grant.is_none());
        assert_eq!(data.view.message.as_deref(), Some("请重新选择声源"));
        assert_eq!(data.loaded.stamp, stamp);
        assert_eq!(
            serde_json::to_vec(&data.loaded.preferences).unwrap(),
            before
        );
        let current = Data::new(Ok(loaded()), &snapshot());
        assert_eq!(current.view.mode, AudioMode::Both);
    }
    #[test]
    fn every_real_plugin_revision_change_invalidates_without_rewriting_preferences() {
        let mut plugins = third_party_snapshot();
        let mut data = Data::new(Ok(third_party_loaded()), &plugins);
        assert_eq!(data.view.mode, AudioMode::Both);
        let stamp = data.loaded.stamp.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        data.pending = Some(Pending {
            grant: Some(data.view.grant.clone().unwrap()),
            revoked: cancelled.clone(),
        });
        plugins.plugins[0].revision = 2;
        plugins.plugins[0].state = PluginState::Disabled;
        assert!(data.refresh(&plugins));
        let sequence = data.view.sequence;
        assert_eq!(data.view.mode, AudioMode::Mute);
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(data.loaded.stamp, stamp);
        for revision in 3..7 {
            plugins.plugins[0].revision = revision;
            plugins.plugins[0].state = PluginState::Enabled;
            data.refresh(&plugins);
            assert_eq!(data.view.mode, AudioMode::Mute);
        }
        assert!(data.view.sequence >= sequence);
        assert_eq!(data.loaded.preferences.mode, AudioMode::Both);
        let reopened = Data::new(Ok(data.loaded.clone()), &plugins);
        assert_eq!(reopened.view.mode, AudioMode::Mute);
    }
    #[test]
    fn grant_requires_the_exact_live_local_audio_contribution() {
        let mut plugins = snapshot();
        assert!(grant_is_current(&grant(), &plugins));
        plugins.plugins[0].error = Some("missing".into());
        assert!(!grant_is_current(&grant(), &plugins));
        plugins.plugins[0].error = None;
        plugins.plugins[0].manifest.contributions.clear();
        assert!(!grant_is_current(&grant(), &plugins));
        let mut wrong = grant();
        wrong.contribution_id = "other".into();
        assert!(!grant_is_current(&wrong, &snapshot()));
    }
    #[test]
    fn permit_lives_until_real_worker_drop_and_invalidation_does_not_erase_saved_choice() {
        let inner = Arc::new(Inner {
            busy: AtomicBool::new(false),
            abort: AtomicBool::new(false),
            faulted: AtomicBool::new(false),
            file: None,
            data: Mutex::new(Data::new(Ok(loaded()), &snapshot())),
        });
        let runtime = RecordingAudioRuntime(inner.clone());
        let work = runtime.claim().unwrap();
        assert!(runtime.claim().is_err());
        assert!(inner.busy.load(Ordering::Acquire));
        inner.abort.store(true, Ordering::Release);
        drop(work);
        assert!(!inner.busy.load(Ordering::Acquire));
        assert_eq!(runtime.view().mode, AudioMode::Both);
        let _next = runtime.claim().unwrap();
        assert!(!inner.abort.load(Ordering::Acquire));
    }
    #[test]
    fn start_matches_the_visible_choice_and_rejects_an_actual_inflight_save() {
        let plugins = third_party_snapshot();
        let inner = Arc::new(Inner {
            busy: AtomicBool::new(false),
            abort: AtomicBool::new(false),
            faulted: AtomicBool::new(false),
            file: None,
            data: Mutex::new(Data::new(Ok(third_party_loaded()), &plugins)),
        });
        let runtime = RecordingAudioRuntime(inner);
        let current = runtime.view().selection();
        assert_eq!(
            validate_runtime_selection(&runtime, &current, &plugins).unwrap(),
            AudioMode::Both
        );
        let old_mute = RecordingAudioSelection {
            revision: 0,
            mode: AudioMode::Mute,
            grant: None,
        };
        assert!(validate_runtime_selection(&runtime, &old_mute, &plugins).is_err());
        let wrong_mode = RecordingAudioSelection {
            mode: AudioMode::System,
            ..current.clone()
        };
        assert!(validate_runtime_selection(&runtime, &wrong_mode, &plugins).is_err());
        let work = runtime.claim().unwrap();
        assert!(validate_runtime_selection(&runtime, &current, &plugins).is_err());
        drop(work);
        let mut disabled = plugins;
        disabled.plugins[0].state = PluginState::Disabled;
        disabled.plugins[0].revision = 2;
        assert!(validate_runtime_selection(&runtime, &current, &disabled).is_err());
        let fresh_mute = RecordingAudioSelection {
            revision: 1,
            mode: AudioMode::Mute,
            grant: None,
        };
        assert_eq!(
            validate_runtime_selection(&runtime, &fresh_mute, &disabled).unwrap(),
            AudioMode::Mute
        );
    }

    #[test]
    fn bad_preferences_leave_silent_readonly_state_without_touching_other_settings() {
        let data = Data::new(Err(prefs::Error::Corrupt), &snapshot());
        assert_eq!(data.view.mode, AudioMode::Mute);
        assert!(!data.view.editable);
        assert!(data.view.message.is_some());
    }
}
