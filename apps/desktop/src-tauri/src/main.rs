// SPDX-License-Identifier: MPL-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod agent_loop;
mod agent_tools;
mod ai;
mod inline_reasoning;
mod asset_host;
mod asset_locations;
mod assets;
mod capture;
mod blackboard;
mod capture_cursor;
mod capture_delay_work;
mod capture_preferences;
mod capture_visibility;
mod capture_shortcut;
mod capture_shortcut_host;
mod clipboard_video_storage;
mod code_decode;
mod code_host;
mod code_open;
mod code_worker;
mod connection_host;
mod content_server;
mod credentials;
mod drawing_render;
mod raster_edit_algorithm;
mod raster_edit_host;
mod export_variant_dialog;
mod formula_layout;
mod frozen;
mod hindsight;
mod host_preferences;
mod image_export;
mod image_host;
mod image_save_dialog;
mod import_pin_host;
mod journal_commands;
mod journal_runtime;
mod journal_tool_host;
mod legacy_connections;
mod lifecycle;
mod mcp_host;
mod mcp_runtime;
mod memory_export;
mod memory_host;
mod memory_runtime;
mod memory_tools;
mod memory_worker;
mod mosaic_preview;
mod network_policy;
mod app_update;
mod ocr;
mod ocr_host;
mod owned_process;
mod pin_host;
mod pin_window;
mod pixel_sampler;
mod plugin_host;
mod plugin_sources;
mod plugins;
mod provider_transport;
mod raster_compositor;
mod recording;
mod recording_audio;
mod recording_audio_host;
mod recording_audio_timeline;
mod recording_backend;
mod recording_preferences;
mod recording_storage;
mod recording_worker;
mod rich_annotation_host;
mod save_dialog;
mod scroll_capture;
mod scroll_host;
mod scroll_stitch;
mod settings_host;
mod settings_info;
mod settings_window;
mod speech_backend;
mod speech_host;
mod speech_worker;
mod startup_windows;
mod system_preferences;
mod storage_cli;
mod storage_dependencies;
mod storage_host;
mod data_host;
mod data_storage;
mod blackboard_text_host;
#[cfg(test)]
mod storage_integration_tests;
mod storage_root;
mod table_clipboard;
mod table_document;
mod table_host;
mod table_staging;
mod translation_document;
mod translation_host;
mod translation_render;
mod ui_language;
mod video_annotation_host;
mod video_annotation_raster;
mod video_annotation_render;
mod video_annotation_tool;
mod video_clip_backend;
mod video_clip_contract;
mod video_clip_worker;
mod video_drawing_host;
mod video_gif_backend;
mod video_host;
mod video_input_host;
mod video_overlay_host;
mod video_save_dialog;
mod video_vector_raster;
mod visual_annotation_host;
mod visual_annotation_input;
mod web_link_host;
mod window_snap;

use mewu_core::{
    Asset, AssetKind, ConnectionProfile, RunContext, RunStatus, SceneCommand, Snapshot, Store,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

pub(crate) struct Engine {
    store: Store,
    runs: HashMap<String, oneshot::Sender<()>>,
    plugin_runs: HashMap<String, (String, String, u64)>,
    visual_runs: HashMap<String, visual_annotation_host::RunOrigin>,
    ocr_jobs: ocr_host::OcrRegistry,
    translation_jobs: translation_host::TranslationRegistry,
}

/// Revocation must stay effective even if persisting scene cancellation fails.
/// The registry origin is an additional fence, never a substitute for Store's run fence.
fn run_is_authorized(
    engine: &Engine,
    scene_id: &str,
    run_id: &str,
    plugin_origin: Option<&(String, String, u64)>,
) -> bool {
    engine.runs.contains_key(run_id)
        && engine.store.run_is_active(scene_id, run_id)
        && plugin_origin.is_none_or(|origin| engine.plugin_runs.get(run_id) == Some(origin))
}
pub(crate) struct Host {
    storage: storage_host::StorageRuntime,
    engine: Mutex<Engine>,
    plugins: Mutex<plugin_host::PluginRuntime>,
    connection_probes: Arc<Mutex<connection_host::ProbeRegistry>>,
    root: PathBuf,
    assets: PathBuf,
    capturing: Arc<AtomicBool>,
    capture_jobs: Arc<capture_delay_work::CaptureJobs>,
    capture_preferences: capture_preferences::CapturePreferencesActor,
    system_preferences: system_preferences::SystemPreferencesActor,
    settings_resources: Result<settings_info::SettingsResources, settings_info::Error>,
    legacy_import_warning: AtomicBool,
    exit: lifecycle::ExitState,
    frozen_dismissed: AtomicBool,
    revision: AtomicU64,
    content_origin: String,
    window_snap: window_snap::WindowSnapRegistry,
    asset_transfers: asset_host::Transfers,
    memory: memory_runtime::Runtime,
    speech: speech_host::SpeechRegistry,
    codes: code_host::CodeRegistry,
    video: video_host::VideoRegistry,
    image_exports: image_host::Registry,
    // Struct fields drop in declaration order: close Engine/Store, plugin DBs,
    // preference actors and all asset owners before releasing this lock.
    _storage_lock: storage_root::RuntimeLock,
}
/// Reads only the immutable map collected beside this background image. No OS
/// window enumeration occurs here. A capture publishes its new background while
/// still hidden, so visibility/capture guards would lose that first map load.
#[tauri::command]
fn get_window_snap_map(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    scene_id: String,
    background_id: String,
) -> Result<Option<window_snap::FrameSnapMap>, String> {
    if window.label() != "space" {
        return Err("此窗口不能读取截图选区".into());
    }
    host.exit.ensure_running()?;
    let engine = host.lock()?;
    let background = engine
        .store
        .active_background_for_preview(&scene_id, &background_id)
        .map_err(|error| error.to_string())?;
    let result = host.window_snap.get(&background);
    host.exit.ensure_running()?;
    Ok(result)
}
impl Host {
    fn lock(&self) -> Result<MutexGuard<'_, Engine>, String> {
        self.engine
            .lock()
            .map_err(|_| "空间存储需要重新启动".into())
    }
}
#[derive(Clone, Serialize)]
pub(crate) struct HostSnapshot {
    #[serde(flatten)]
    snapshot: Snapshot,
    revision: u64,
}
fn publish(app: &AppHandle, snapshot: &Snapshot) -> HostSnapshot {
    speech_host::reconcile(app, snapshot);
    code_host::reconcile(app, snapshot);
    video_host::reconcile(app, snapshot);
    image_host::reconcile(app, snapshot);
    pixel_sampler::reconcile(app, snapshot);
    pin_host::reconcile(app, snapshot);
    let revision = app.state::<Host>().revision.fetch_add(1, Ordering::AcqRel) + 1;
    let snapshot = HostSnapshot {
        snapshot: snapshot.clone(),
        revision,
    };
    let _ = app.emit_to("space", "snapshot", &snapshot);
    let _ = app.emit_to("settings", "snapshot", &snapshot);
    // A completed background run updates the small widget without showing or focusing it.
    let _ = app.emit_to(
        frozen::LABEL,
        "frozen-scenes",
        frozen::snapshot(&snapshot.snapshot, revision),
    );
    snapshot
}
fn show(app: &AppHandle) {
    if recording::busy(app)
        || scroll_host::busy(app)
        || !app.state::<Host>().exit.accepts_new_operations()
    {
        return;
    }
    let _ = frozen::hide(app);
    if let Some(window) = app.get_webview_window("space") {
        if let Err(error) = capture_visibility::apply(&window) { let _ = app.emit_to("space", "host-error", error); return; }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        pin_window::arrange_for_space(app);
    }
}

#[tauri::command]
fn get_snapshot(app: AppHandle, host: tauri::State<'_, Host>) -> Result<HostSnapshot, String> {
    if host.legacy_import_warning.swap(false, Ordering::AcqRel) {
        let _ = app.emit_to("space", "host-error", "部分旧版连接未导入，请检查连接设置");
    }
    {
        let engine = host.lock()?;
        Ok(HostSnapshot {
            snapshot: engine.store.snapshot(),
            revision: host.revision.load(Ordering::Acquire),
        })
    }
}
#[tauri::command]
async fn memory_page(
    app: AppHandle,
    agent_id: String,
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<mewu_core::MemoryPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        engine
            .store
            .memory_page(
                &agent_id,
                query.as_deref().unwrap_or(""),
                cursor.as_deref(),
                limit.unwrap_or(25),
            )
            .map_err(agent_tools::domain_error)
    })
    .await
    .map_err(|_| "记忆检索中断")?
}
#[tauri::command]
async fn memory_entry(
    app: AppHandle,
    agent_id: String,
    id: String,
) -> Result<Option<mewu_core::MemoryEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        engine
            .store
            .memory_entry(&agent_id, &id)
            .map_err(agent_tools::domain_error)
    })
    .await
    .map_err(|_| "记忆读取中断")?
}
#[tauri::command]
async fn apply_scene_command(
    app: AppHandle,
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    command: SceneCommand,
) -> Result<HostSnapshot, String> {
    ensure_scene_command_admission(&app, &host, window.label(), &command)?;
    let command = match command {
        SceneCommand::FreezeScene { scene_id } => return freeze_space(app, scene_id).await,
        SceneCommand::CloseScene { scene_id } => return close_scene_window(app, scene_id).await,
        SceneCommand::AddDrawing { .. }
        | SceneCommand::UpdateDrawing { .. } | SceneCommand::UpdateDrawings { .. }
        | SceneCommand::RemoveDrawing { .. }
        | SceneCommand::UndoDrawing { .. }
        | SceneCommand::RedoDrawing { .. } => return Err("请从绘制工具操作".into()),
        command => command,
    };
    let mut engine = host.lock()?;
    ensure_scene_command_admission(&app, &host, window.label(), &command)?;
    let finishing_geometry = lifecycle::is_geometry_finish_command(&command);
    let disabled_memory = match &command {
        SceneCommand::SaveAgent { agent } if !agent.memory_enabled => Some(agent.id.clone()),
        _ => None,
    };
    let snapshot = engine.store.apply(command).map_err(|e| e.to_string())?;
    if let Some(agent_id) = disabled_memory {
        host.memory.cancel_where(|task| {
            engine
                .store
                .memory_binding(&task.binding_id)
                .is_ok_and(|binding| binding.agent_id == agent_id)
        });
    }
    if finishing_geometry {
        // Closing a matching transient group changes no persisted state. Do not
        // publish a new host revision or reconcile unrelated background jobs.
        return Ok(HostSnapshot {
            snapshot,
            revision: host.revision.load(Ordering::Acquire),
        });
    }
    cancel_ended_runs(&mut engine, &snapshot);
    Ok(publish(&app, &snapshot))
}

fn ensure_scene_command_source(owner: &str, command: &SceneCommand) -> Result<(), String> {
    match command {
        SceneCommand::UpdateRegion { .. } => Err("选区更新方式已失效".into()),
        SceneCommand::SetRegionGeometry { .. }
        | SceneCommand::FinishRegionGeometryEdit { .. }
        | SceneCommand::UndoRegionGeometry { .. }
        | SceneCommand::RedoRegionGeometry { .. }
            if owner != "space" =>
        {
            Err("此窗口不能调整选区".into())
        }
        _ => Ok(()),
    }
}

fn ensure_geometry_transition(command: &SceneCommand, capturing: bool) -> Result<(), String> {
    if capturing
        && matches!(
            command,
            SceneCommand::NewScene
                | SceneCommand::NewConversation { .. }
                | SceneCommand::ActivateScene { .. }
                | SceneCommand::SetRegionGeometry { .. }
                | SceneCommand::UndoRegionGeometry { .. }
                | SceneCommand::RedoRegionGeometry { .. }
        )
    {
        Err("空间正在切换".into())
    } else {
        Ok(())
    }
}

fn ensure_scene_command_admission(
    app: &AppHandle,
    host: &Host,
    owner: &str,
    command: &SceneCommand,
) -> Result<(), String> {
    ensure_scene_command_source(owner, command)?;
    host.exit.ensure_scene_command(command)?;
    // A matching finish only clears a transient edit token, so late cleanup may
    // run during capture. It is not a draft write and grants no replay exception.
    // This helper also runs under Engine: never call window getters here.
    if requires_capture_idle(command) {
        recording::ensure_idle(app)?; // also checks scroll capture
    }
    ensure_geometry_transition(command, host.capturing.load(Ordering::Acquire))
}

fn requires_capture_idle(command: &SceneCommand) -> bool {
    !lifecycle::is_draft_command(command) && !lifecycle::is_geometry_finish_command(command)
}

#[cfg(test)]
mod geometry_admission_tests {
    use super::*;

    fn geometry() -> SceneCommand {
        SceneCommand::SetRegionGeometry {
            scene_id: "scene".into(),
            region_id: "region".into(),
            background_id: "background".into(),
            source_id: "background".into(),
            expected_revision: 2,
            edit_id: None,
            from: mewu_core::RegionGeometry {
                x: 1.,
                y: 2.,
                width: 30.,
                height: 40.,
            },
            to: mewu_core::RegionGeometry {
                x: 2.,
                y: 2.,
                width: 30.,
                height: 40.,
            },
        }
    }

    fn replay(redo: bool) -> SceneCommand {
        let from = mewu_core::RegionGeometry {
            x: 2.,
            y: 2.,
            width: 30.,
            height: 40.,
        };
        if redo {
            SceneCommand::RedoRegionGeometry {
                scene_id: "scene".into(),
                expected_history_revision: 3,
                expected_operation_id: uuid::Uuid::new_v4().to_string(),
                region_id: "region".into(),
                background_id: "background".into(),
                source_id: "background".into(),
                expected_revision: 2,
                from,
            }
        } else {
            SceneCommand::UndoRegionGeometry {
                scene_id: "scene".into(),
                expected_history_revision: 3,
                expected_operation_id: uuid::Uuid::new_v4().to_string(),
                region_id: "region".into(),
                background_id: "background".into(),
                source_id: "background".into(),
                expected_revision: 2,
                from,
            }
        }
    }

    fn finish() -> SceneCommand {
        SceneCommand::FinishRegionGeometryEdit {
            scene_id: "scene".into(),
            edit_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    #[test]
    fn typed_geometry_is_space_only_and_legacy_updates_cannot_bypass_cas() {
        for command in [geometry(), replay(false), replay(true), finish()] {
            assert!(ensure_scene_command_source("space", &command).is_ok());
            for owner in [
                "settings",
                "frozen-widget",
                "recording-controls",
                "pin-owned",
                "",
            ] {
                assert!(ensure_scene_command_source(owner, &command).is_err());
            }
        }
        let legacy = SceneCommand::UpdateRegion {
            scene_id: "scene".into(),
            region: mewu_core::Region::default(),
        };
        for owner in ["space", "settings", "pin-owned"] {
            assert!(ensure_scene_command_source(owner, &legacy).is_err());
        }
        // Existing settings and conversation command routes keep their scope.
        assert!(ensure_scene_command_source("settings", &SceneCommand::NewScene).is_ok());
    }

    #[test]
    fn geometry_never_inherits_draft_capture_exception() {
        let geometry = geometry();
        assert!(!lifecycle::is_draft_command(&geometry));
        assert!(requires_capture_idle(&geometry));
        assert!(lifecycle::is_exit_flush_command(&geometry));
        assert!(ensure_geometry_transition(&geometry, true).is_err());
        assert!(ensure_geometry_transition(&geometry, false).is_ok());
        let draft = SceneCommand::SetDraft {
            scene_id: "scene".into(),
            draft: "pending".into(),
        };
        assert!(lifecycle::is_draft_command(&draft));
        assert!(!requires_capture_idle(&draft));
        assert!(ensure_geometry_transition(&draft, true).is_ok());
    }

    #[test]
    fn finish_cleanup_is_independent_of_replay_and_draft_permissions() {
        let finish = finish();
        assert!(!lifecycle::is_draft_command(&finish));
        assert!(lifecycle::is_geometry_finish_command(&finish));
        assert!(lifecycle::is_exit_flush_command(&finish));
        assert!(!requires_capture_idle(&finish));
        assert!(ensure_geometry_transition(&finish, true).is_ok());
        for replay in [replay(false), replay(true)] {
            assert!(!lifecycle::is_draft_command(&replay));
            assert!(!lifecycle::is_geometry_finish_command(&replay));
            assert!(!lifecycle::is_exit_flush_command(&replay));
            assert!(requires_capture_idle(&replay));
            assert!(ensure_geometry_transition(&replay, true).is_err());
            assert!(ensure_geometry_transition(&replay, false).is_ok());
        }
    }
}

fn cancel_ended_runs(engine: &mut Engine, snapshot: &Snapshot) {
    engine.ocr_jobs.reconcile(snapshot);
    engine.translation_jobs.reconcile(snapshot);
    // A revoked memory/tool grant settles runs in the same Store transaction.
    // Drop transport futures so cached permissions cannot start another round.
    let ended: Vec<_> = engine
        .runs
        .keys()
        .filter(|id| {
            !snapshot.scenes.iter().any(|scene| {
                scene
                    .run
                    .as_ref()
                    .is_some_and(|run| &run.id == *id && run.status == RunStatus::Running)
            })
        })
        .cloned()
        .collect();
    for id in ended {
        engine.plugin_runs.remove(&id);
        engine.visual_runs.remove(&id);
        if let Some(cancel) = engine.runs.remove(&id) {
            let _ = cancel.send(());
        }
    }
    engine.plugin_runs.retain(|id, (scene_id, _, _)| {
        snapshot.scenes.iter().any(|scene| {
            &scene.id == scene_id
                && scene
                    .run
                    .as_ref()
                    .is_some_and(|run| &run.id == id && run.status == RunStatus::Running)
        })
    });
    engine.visual_runs.retain(|id, origin| {
        snapshot.scenes.iter().any(|scene| {
            scene.id == origin.scene_id
                && scene
                    .run
                    .as_ref()
                    .is_some_and(|run| &run.id == id && run.status == RunStatus::Running)
        })
    });
}
#[tauri::command]
async fn hide_space(app: AppHandle) -> Result<(), String> {
    hide_space_checked(app, |_| true).await
}

/// Once the transition is owned, scene activation cannot race a successful
/// action's final source check and hide a different, newly opened space.
pub(crate) async fn hide_space_checked(
    app: AppHandle,
    current: impl FnOnce(&Snapshot) -> bool,
) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    host.capture_jobs.cancel();
    raster_edit_host::cancel_all();
    host.capture_jobs
        .drain(std::time::Duration::from_secs(5))
        .await?;
    let _transition = SpaceTransition::try_begin(&host.capturing)
        .ok_or_else(|| "请先完成当前采集或空间切换".to_string())?;
    host.exit.ensure_new_operation()?;
    recording::ensure_idle(&app)?;
    let has_frozen = {
        let engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        let snapshot = engine.store.snapshot();
        if !current(&snapshot) {
            return Ok(());
        }
        engine.ocr_jobs.cancel_owner("space");
        engine
            .translation_jobs
            .cancel_scene("space", &snapshot.active_scene_id);
        snapshot
            .scenes
            .iter()
            .any(|scene| scene.frozen && !scene.closed && frozen::has_content(scene))
    };
    speech_host::cancel_owner(&app, "space");
    code_host::cancel_owner(&app, "space");
    video_host::cancel_all(&app);
    image_host::cancel_owner(&app, "space");
    pixel_sampler::invalidate(&app);
    pin_host::cancel_pending(&app, "space");
    if has_frozen && !host.frozen_dismissed.load(Ordering::Acquire) {
        frozen::layout(&app, false, true).await
    } else {
        app.get_webview_window("space")
            .ok_or("窗口不存在")?
            .hide()
            .map_err(|_| "无法隐藏窗口".into())
    }
}

#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    let host = app.state::<Host>();
    let _transition = begin_space_transition(&app, &host)?;
    recording::ensure_idle(&app)?;
    let target = app.clone();
    tauri::async_runtime::spawn_blocking(move || settings_window::show(&target))
        .await
        .map_err(|_| "无法打开设置".to_string())?
}

#[tauri::command]
fn hide_frozen_widget(app: AppHandle) -> Result<(), String> {
    let host = app.state::<Host>();
    let _transition = begin_space_transition(&app, &host)?;
    host.frozen_dismissed.store(true, Ordering::Release);
    frozen::hide(&app)
}

#[tauri::command]
async fn close_scene_window(app: AppHandle, scene_id: String) -> Result<HostSnapshot, String> {
    let host = app.state::<Host>();
    let _transition = begin_space_transition(&app, &host)?;
    recording::ensure_idle(&app)?;
    let (was_active, has_frozen) = {
        let mut engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        let was_active = engine.store.snapshot().active_scene_id == scene_id;
        let snapshot = engine
            .store
            .apply(SceneCommand::CloseScene { scene_id })
            .map_err(|error| error.to_string())?;
        cancel_ended_runs(&mut engine, &snapshot);
        let has_frozen = snapshot
            .scenes
            .iter()
            .any(|scene| scene.frozen && !scene.closed && frozen::has_content(scene));
        publish(&app, &snapshot);
        (was_active, has_frozen)
    };
    if was_active {
        if has_frozen && !host.frozen_dismissed.load(Ordering::Acquire) {
            frozen::layout(&app, false, true).await?;
        } else if let Some(window) = app.get_webview_window("space") {
            window.hide().map_err(|_| "会话已关闭，窗口未能隐藏")?;
        }
    }
    if !has_frozen {
        frozen::hide(&app)?;
    }
    // Closing is persisted even if a subsequent native window operation fails.
    let engine = host.lock()?;
    Ok(publish(&app, &engine.store.snapshot()))
}

#[tauri::command]
fn get_frozen_scenes(host: tauri::State<'_, Host>) -> Result<frozen::FrozenSnapshot, String> {
    let engine = host.lock()?;
    Ok(frozen::snapshot(
        &engine.store.snapshot(),
        host.revision.load(Ordering::Acquire),
    ))
}

struct SpaceTransition(Arc<AtomicBool>);
impl SpaceTransition {
    fn try_begin(active: &Arc<AtomicBool>) -> Option<Self> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(active.clone()))
    }
}
impl Drop for SpaceTransition {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn begin_space_transition(app: &AppHandle, host: &Host) -> Result<SpaceTransition, String> {
    host.exit.ensure_new_operation()?;
    if !host.storage.is_ready() {
        return Err("正在启动，请稍候".into());
    }
    let transition = SpaceTransition::try_begin(&host.capturing)
        .ok_or_else(|| "请先完成当前采集或空间切换".to_string())?;
    host.exit.ensure_new_operation()?;
    speech_host::cancel_owner(app, "space");
    code_host::cancel_owner(app, "space");
    video_host::cancel_all(app);
    raster_edit_host::cancel_all();
    image_host::cancel_owner(app, "space");
    Ok(transition)
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn ignored_capture_cannot_release_another_windows_transition() {
        let active = Arc::new(AtomicBool::new(false));
        let owner = SpaceTransition::try_begin(&active).unwrap();
        assert!(SpaceTransition::try_begin(&active).is_none());
        assert!(active.load(Ordering::Acquire));
        assert!(SpaceTransition::try_begin(&active).is_none());
        drop(owner);
        let next = SpaceTransition::try_begin(&active).unwrap();
        drop(next);
        assert!(!active.load(Ordering::Acquire));
    }

    #[test]
    fn scene_activation_cannot_overtake_an_owned_hide_or_capture() {
        let active = Arc::new(AtomicBool::new(false));
        let owner = SpaceTransition::try_begin(&active).unwrap();
        for command in [
            SceneCommand::NewScene,
            SceneCommand::ActivateScene {
                scene_id: "another-scene".into(),
            },
        ] {
            assert!(ensure_geometry_transition(&command, active.load(Ordering::Acquire)).is_err());
        }
        drop(owner);
        assert!(ensure_geometry_transition(&SceneCommand::NewScene, false).is_ok());
    }

    #[test]
    fn concurrent_capture_requests_admit_one_owner_without_queuing_replays() {
        let active = Arc::new(AtomicBool::new(false));
        let admitted = std::sync::atomic::AtomicUsize::new(0);
        let ready = std::sync::Barrier::new(8);
        let attempted = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    ready.wait();
                    let owner = SpaceTransition::try_begin(&active);
                    if owner.is_some() {
                        admitted.fetch_add(1, Ordering::Relaxed);
                    }
                    attempted.wait();
                    drop(owner);
                });
            }
        });
        assert_eq!(admitted.load(Ordering::Relaxed), 1);
        assert!(!active.load(Ordering::Acquire));
    }
}

#[tauri::command]
async fn freeze_space(app: AppHandle, scene_id: String) -> Result<HostSnapshot, String> {
    let host = app.state::<Host>();
    let _transition = begin_space_transition(&app, &host)?;
    pixel_sampler::invalidate(&app);
    pin_host::cancel_pending(&app, "space");
    recording::ensure_idle(&app)?;
    {
        let mut engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        let current = engine.store.snapshot();
        let scene = current
            .scenes
            .iter()
            .find(|scene| scene.id == scene_id)
            .ok_or("会话不存在")?;
        if !frozen::has_content(scene) {
            return Err("当前空间没有可冻结的内容".into());
        }
        engine
            .store
            .apply(SceneCommand::FreezeScene {
                scene_id: scene_id.clone(),
            })
            .map_err(|error| error.to_string())?;
    }
    if let Err(error) = frozen::layout(&app, false, true).await {
        // Window operations cannot share SQLite's transaction. Compensate selection
        // if the native transition fails so the user does not see a blank new scene.
        if let Ok(mut engine) = host.lock().and_then(|engine| {
            host.exit.ensure_background()?;
            Ok(engine)
        }) {
            if let Ok(restored) = engine.store.apply(SceneCommand::ActivateScene { scene_id }) {
                publish(&app, &restored);
            }
        }
        show(&app);
        return Err(error);
    }
    host.frozen_dismissed.store(false, Ordering::Release);
    // A run may have completed while the main loop was hiding the window.
    // Publish current storage, never the snapshot captured before the await.
    let engine = host.lock()?;
    Ok(publish(&app, &engine.store.snapshot()))
}

#[tauri::command]
async fn restore_frozen_scene(app: AppHandle, scene_id: String) -> Result<HostSnapshot, String> {
    let host = app.state::<Host>();
    let _transition = begin_space_transition(&app, &host)?;
    pixel_sampler::invalidate(&app);
    pin_host::cancel_pending(&app, "space");
    recording::ensure_idle(&app)?;
    let scene = host
        .lock()?
        .store
        .snapshot()
        .scenes
        .into_iter()
        .find(|scene| scene.id == scene_id)
        .ok_or("会话不存在")?;
    if !scene.frozen {
        return Err("该会话已在空间中打开".into());
    }
    frozen::prepare_restore(&app, scene.background.as_ref()).await?;
    let mut engine = host.lock()?;
    host.exit.ensure_new_operation()?;
    let snapshot = engine
        .store
        .apply(SceneCommand::ActivateScene { scene_id })
        .map_err(|error| error.to_string())?;
    let snapshot = publish(&app, &snapshot);
    show(&app);
    Ok(snapshot)
}

#[tauri::command]
async fn resize_frozen_widget(app: AppHandle, expanded: bool) -> Result<(), String> {
    frozen::layout(&app, expanded, false).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
    hotkey: String,
    capture_backend: &'static str,
    content_origin: String,
}
#[tauri::command]
fn runtime_info(app: AppHandle, host: tauri::State<'_, Host>) -> Result<RuntimeInfo, String> {
    Ok(RuntimeInfo {
        hotkey: capture_shortcut_host::active_display(&app),
        capture_backend: if cfg!(windows) { "GDI" } else { "CoreGraphics" },
        content_origin: host.content_origin.clone(),
    })
}

/// A queued hide() from a worker is not proof that the native window is hidden.
/// Handshake with the main loop before acquiring desktop pixels.
async fn hide_for_capture(window: &tauri::WebviewWindow) -> Result<(), String> {
    let (send, receive) = oneshot::channel();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                target
                    .app_handle()
                    .state::<Host>()
                    .exit
                    .ensure_new_operation()?;
                #[cfg(windows)]
                {
                    use windows_sys::Win32::Graphics::Dwm::{
                        DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED,
                    };
                    let hwnd = target.hwnd().map_err(|_| "无法准备截图窗口")?.0;
                    let disabled: i32 = 1;
                    let hr = unsafe {
                        DwmSetWindowAttribute(
                            hwnd,
                            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
                            (&disabled as *const i32).cast(),
                            std::mem::size_of::<i32>() as u32,
                        )
                    };
                    if hr < 0 {
                        return Err("无法关闭截图窗口过渡动画".into());
                    }
                }
                target.hide().map_err(|_| "无法隐藏截图窗口")?;
                // The shared floating widget is also part of Mewu and must not be captured.
                frozen::hide(target.app_handle())?;
                // Settings has its own lifetime. Capture only hides the space
                // and conversation widget, as in the original desktop app.
                #[cfg(windows)]
                {
                    let hwnd = target.hwnd().map_err(|_| "截图窗口不可用")?.0;
                    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd) }
                        != 0
                    {
                        return Err("截图窗口尚未隐藏".into());
                    }
                    if unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() } < 0 {
                        return Err("桌面画面尚未就绪".into());
                    }
                }
                Ok(())
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "无法准备截图")?;
    receive.await.map_err(|_| "截图准备中断")?
}

/// Complete the native layout before publishing the new screenshot or showing it.
/// Merely sizing a resizable borderless window leaves resize chrome and does not
/// tell the Windows shell that the taskbar must yield to a fullscreen surface.
async fn expand_capture_space(
    window: &tauri::WebviewWindow,
    captured: &capture::CapturedScreen,
) -> Result<(), String> {
    let (send, receive) = oneshot::channel();
    let target = window.clone();
    let captured = captured.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                target
                    .app_handle()
                    .state::<Host>()
                    .exit
                    .ensure_new_operation()?;
                // Reset the previous monitor's fullscreen placement while hidden,
                // so a later capture on another screen cannot reuse its bounds.
                target
                    .set_simple_fullscreen(false)
                    .map_err(|_| "无法调整空间")?;
                target.set_decorations(false).map_err(|_| "无法展开空间")?;
                target
                    .set_resizable(false)
                    .map_err(|_| "无法固定空间边界")?;
                match captured.coordinate_space {
                    capture::CoordinateSpace::PhysicalPixels => {
                        let monitors = target.available_monitors().map_err(|_| "无法定位显示器")?;
                        if !monitors.iter().any(|monitor| {
                            monitor.position().x == captured.origin_x
                                && monitor.position().y == captured.origin_y
                                && monitor.size().width == captured.display_width
                                && monitor.size().height == captured.display_height
                        }) {
                            return Err("显示器已变化，请重新截图".into());
                        }
                        target
                            .set_position(tauri::PhysicalPosition::new(
                                captured.origin_x,
                                captured.origin_y,
                            ))
                            .map_err(|_| "无法定位空间")?;
                        target
                            .set_size(tauri::PhysicalSize::new(
                                captured.display_width,
                                captured.display_height,
                            ))
                            .map_err(|_| "无法调整空间")?;
                        // Use a point inside the captured screen, including for
                        // negative virtual-desktop origins and mixed-DPI screens.
                        target
                            .set_fullscreen_on_monitor(tauri::PhysicalPosition::new(
                                f64::from(captured.origin_x)
                                    + f64::from(captured.display_width) / 2.,
                                f64::from(captured.origin_y)
                                    + f64::from(captured.display_height) / 2.,
                            ))
                            .map_err(|_| "无法全屏显示空间")?;
                        if !target.is_fullscreen().map_err(|_| "无法确认全屏状态")? {
                            return Err("无法全屏显示空间".into());
                        }
                    }
                    capture::CoordinateSpace::LogicalPoints => {
                        target
                            .set_position(tauri::LogicalPosition::new(
                                captured.origin_x,
                                captured.origin_y,
                            ))
                            .map_err(|_| "无法定位空间")?;
                        target
                            .set_size(tauri::LogicalSize::new(
                                captured.display_width,
                                captured.display_height,
                            ))
                            .map_err(|_| "无法调整空间")?;
                        // macOS capture origins are points. Do not multiply a
                        // global origin by one screen's scale or create a new Space.
                        target
                            .set_simple_fullscreen(true)
                            .map_err(|_| "无法全屏显示空间")?;
                    }
                }
                capture_visibility::apply(&target)?;
                target.set_always_on_top(true).map_err(|_| "无法置顶空间")?;
                Ok(())
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "无法展开空间")?;
    receive.await.map_err(|_| "空间展开中断")?
}

#[tauri::command]
async fn capture_screen(app: AppHandle) -> Result<HostSnapshot, String> {
    recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let Some(transition) = SpaceTransition::try_begin(&host.capturing) else {
        // Match the original BeginCaptureAsync: ignore another request while
        // capture or a window transition owns the surface. This is not an error
        // or a progress notification, and must not release the current owner.
        return get_snapshot(app.clone(), host);
    };
    host.system_preferences.snapshot_for_operation().map_err(|error| error.to_string())?;
    let preferences = host
        .capture_preferences
        .snapshot_for_capture()
        .map_err(|e| e.to_string())?;
    let Some(work) = host.capture_jobs.begin()? else {
        return get_snapshot(app.clone(), host);
    };
    let _invoke = work.cancel_on_drop();
    speech_host::cancel_owner(&app, "space");
    capture_shortcut_host::invalidate_all(&app);
    code_host::cancel_owner(&app, "space");
    video_host::cancel_all(&app);
    raster_edit_host::cancel_all();
    image_host::cancel_owner(&app, "space");
    pixel_sampler::invalidate(&app);
    pin_host::cancel_pending(&app, "space");
    recording::ensure_idle(&app)?;
    let window = app.get_webview_window("space").ok_or("窗口不存在")?;
    pin_window::capture_begin(&app);
    {
        let engine = host.lock()?;
        engine.ocr_jobs.cancel_owner("space");
        engine
            .translation_jobs
            .cancel_scene("space", &engine.store.snapshot().active_scene_id);
    }
    if let Err(error) = hide_for_capture(&window).await {
        show(&app);
        return Err(error);
    }
    if let Err(error) = work
        .wait_delay(preferences.capture_delay_seconds.seconds(), || {
            host.exit.ensure_new_operation()?;
            recording::ensure_idle(&app)
        })
        .await
    {
        if work.check().is_ok() {
            show(&app);
        }
        return Err(error);
    }
    let root = host.assets.clone();
    let capture_app = app.clone();
    let (work, _transition, captured) = tauri::async_runtime::spawn_blocking(move || {
        let captured = work.check().and_then(|_| {
            capture_app.state::<Host>().exit.ensure_new_operation()?;
            capture::capture_desktop_with_options(
                &root,
                capture::CaptureOptions {
                    include_cursor: preferences.include_cursor,
                },
            )
        });
        (work, transition, captured)
    })
    .await
    .map_err(|_| "截图任务中断".to_string())?;
    let result = async {
        let mut captured = captured?;
        work.check()?;
        host.exit.ensure_new_operation()?;
        expand_capture_space(&window, &captured).await?;
        let window_map = captured.window_map.take();
        let asset = Asset {
            id: captured.asset_id,
            name: "截图.png".into(),
            kind: AssetKind::Image,
            path: captured.path.to_string_lossy().into_owned(),
            width: Some(captured.width),
            height: Some(captured.height),
            origin_x: Some(captured.origin_x),
            origin_y: Some(captured.origin_y),
            scale_factor: Some(captured.scale_factor),
        };
        let mut engine = host.lock()?;
        work.check()?;
        host.exit.ensure_new_operation()?;
        let current = engine.store.snapshot();
        let active = current
            .scenes
            .iter()
            .find(|s| s.id == current.active_scene_id)
            .ok_or("场景不存在")?;
        if active.background.is_some()
            || !active.messages.is_empty()
            || !active.items.is_empty()
            || !active.draft.is_empty()
        {
            engine
                .store
                .apply(SceneCommand::NewScene)
                .map_err(|e| e.to_string())?;
        }
        let id = engine.store.snapshot().active_scene_id;
        let snapshot = engine
            .store
            .set_background(&id, asset.clone())
            .map_err(|e| e.to_string())?;
        if let Some(map) = window_map {
            // Metadata is optional: a rejected or unavailable map must never
            // discard the successfully saved screenshot or its new scene.
            let _ = host.window_snap.insert(&asset, map);
        }
        Ok(publish(&app, &snapshot))
    }
    .await;
    if work.check().is_ok() {
        show(&app);
    }
    result
}

#[tauri::command]
fn read_artifact(host: tauri::State<'_, Host>, asset_id: String) -> Result<String, String> {
    let asset = assets::resolve(&host.lock()?.store.snapshot(), &asset_id).ok_or("素材不存在")?;
    if !matches!(
        asset.kind,
        AssetKind::Html | AssetKind::Svg | AssetKind::Text
    ) {
        return Err("此素材不是文档".into());
    }
    assets::read_text(&asset, &host.assets)
}
#[tauri::command]
async fn export_snapshot(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_parent(&window)
            .add_filter("JSON", &["json"])
            .set_file_name("mewu-space.json")
            .save_file()
        {
            let host = app.state::<Host>();
            memory_export::write_to_path(&host.root.join("spaces.db"), &path)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "导出任务中断")?
}
#[tauri::command]
async fn export_region(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    region_id: String,
    clipboard: bool,
) -> Result<bool, String> {
    #[cfg(windows)]
    return image_host::export_region(app, window, scene_id, region_id, clipboard).await;
    #[cfg(not(windows))]
    {
        let host = app.state::<Host>();
        let root = host.assets.clone();
        let scene = host
            .lock()?
            .store
            .snapshot()
            .scenes
            .into_iter()
            .find(|s| s.id == scene_id)
            .ok_or("场景不存在")?;
        tauri::async_runtime::spawn_blocking(move || {
            let image = assets::crop(&scene, &region_id, &root)?;
            if clipboard {
                let image = image.into_rgba8();
                arboard::Clipboard::new()
                    .map_err(|_| "无法打开剪贴板")?
                    .set_image(arboard::ImageData {
                        width: image.width() as usize,
                        height: image.height() as usize,
                        bytes: std::borrow::Cow::Owned(image.into_raw()),
                    })
                    .map_err(|_| "无法复制图片")?;
            } else if let Some(path) = rfd::FileDialog::new()
                .set_parent(&window)
                .add_filter("PNG", &["png"])
                .set_file_name("截图.png")
                .save_file()
            {
                image
                    .save_with_format(path, image::ImageFormat::Png)
                    .map_err(|_| "无法保存图片")?;
            } else {
                return Ok(false);
            }
            Ok(true)
        })
        .await
        .map_err(|_| "导出任务中断")?
    }
}

#[tauri::command]
async fn send_message(
    app: AppHandle,
    host: tauri::State<'_, Host>,
    scene_id: String,
    visual_annotations: Option<mewu_core::VisualAnnotationGrant>,
) -> Result<HostSnapshot, String> {
    host.exit.ensure_new_operation()?;
    recording::ensure_idle(&app)?;
    speech_host::cancel_owner(&app, "space");
    if let Some(grant) = &visual_annotations {
        host.plugins
            .lock()
            .map_err(|_| "插件状态不可用")?
            .store
            .visual_annotations(grant)?;
    }
    let (scene, preflight) = {
        let engine = host.lock()?;
        let scene = engine
            .store
            .snapshot()
            .scenes
            .into_iter()
            .find(|s| s.id == scene_id)
            .ok_or("场景不存在")?;
        if visual_annotations.is_none() {
            ai::validate_scene_for_send(&scene)?;
        }
        (scene, preflight_run(&host, &engine, &scene_id)?)
    };
    let video_requested = scene.refs.iter().any(|reference| {
        reference.kind == mewu_core::ReferenceKind::Item
            && scene
                .items
                .iter()
                .any(|item| item.id == reference.id && item.asset.kind == AssetKind::Video)
    });
    let mut video_prepared = if video_requested {
        Some(
            video_input_host::prepare(
                &app,
                &scene,
                visual_annotations
                    .as_ref()
                    .ok_or("视频理解与批注插件未启用")?,
            )
            .await?,
        )
    } else {
        None
    };
    let attachments = if video_requested {
        None
    } else {
        Some(
            prepare_scene_attachments_with_visual(
                scene.clone(),
                host.assets.clone(),
                visual_annotations.is_some(),
            )
            .await?,
        )
    };
    recording::ensure_idle(&app)?;
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    if let Some(grant) = &visual_annotations {
        plugins.store.visual_annotations(grant)?;
    }
    let mut engine = host.lock()?;
    host.exit.ensure_new_operation()?;
    recording::ensure_idle(&app)?;
    if engine
        .store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        != Some(&scene)
        || engine
            .store
            .connection_for_scene(&scene_id)
            .map_err(|e| e.to_string())?
            != preflight.connection
    {
        return Err("会话或连接已变更，请重新发送".into());
    }
    let video = video_prepared
        .as_ref()
        .map(|prepared| {
            video_annotation_tool::RunScope::new(
                scene_id.clone(),
                visual_annotations
                    .as_ref()
                    .ok_or("视频理解与批注缺少授权")?
                    .clone(),
                prepared.input.clone(),
            )
            .map(Arc::new)
        })
        .transpose()?;
    let visual = visual_annotations
        .as_ref()
        .filter(|_| !video_requested)
        .map(|grant| {
            let input = attachments
                .as_ref()
                .ok_or("截图批注缺少输入")?
                .visual_input()
                .cloned()
                .ok_or("请引用需要处理的截图选区")?;
            visual_annotation_host::RunScope::new(scene_id.clone(), grant.clone(), input)
                .map(Arc::new)
        })
        .transpose()?;
    let memory_grant = memory_host::grant_for_scene(&engine.store, &plugins.store, &scene_id)?;
    if let Some(scope) = &video {
        mcp_host::preflight_video_catalog(
            &engine.store.snapshot(),
            &scene_id,
            scope.clone(),
            memory_grant.is_some(),
        )?;
    }
    host.exit.ensure_new_operation()?;
    let context = if let Some(prepared) = video_prepared.as_mut() {
        prepared.ensure_current()?;
        let input = mewu_core::VerifiedVideoRunInput::new(
            visual_annotations.ok_or("视频理解与批注缺少授权")?,
            prepared.input.clone(),
            prepared.assets.clone(),
        )
        .map_err(|e| e.to_string())?;
        let context = engine
            .store
            .begin_run_with_video_input(&scene_id, memory_grant, input)
            .map_err(|e| e.to_string())?;
        prepared.committed();
        context
    } else {
        engine
            .store
            .begin_run_with_memory(&scene_id, memory_grant)
            .map_err(|e| e.to_string())?
    };
    start_run_with_annotations(
        &app,
        &mut engine,
        context,
        preflight,
        attachments,
        visual,
        video,
    )
}

/// Host-only credentials for one already-validated connection. Never serialize
/// or expose these to manifests, plugin processes, window state, or diagnostics.
pub(crate) struct RunPreflight {
    transport: provider_transport::Transport,
    key: Zeroizing<String>,
    connection: ConnectionProfile,
}

pub(crate) async fn prepare_scene_attachments(
    scene: mewu_core::Scene,
    root: PathBuf,
) -> Result<ai::PreparedAttachments, String> {
    prepare_scene_attachments_with_visual(scene, root, false).await
}

async fn prepare_scene_attachments_with_visual(
    scene: mewu_core::Scene,
    root: PathBuf,
    visual: bool,
) -> Result<ai::PreparedAttachments, String> {
    static REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
    static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let request = REQUESTS
        .try_acquire()
        .map_err(|_| "引用正在处理，请稍后重试")?;
    let worker = WORKERS.acquire().await.map_err(|_| "引用处理不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        if visual {
            ai::prepare_scene_visual_attachments(
                &scene,
                &root,
                visual_annotation_input::VisualCoordinateProfile::ExplicitNormalized1000V2,
            )
        } else {
            ai::prepare_scene_attachments(&scene, &root)
        }
    })
    .await
    .map_err(|_| "准备引用中断")?
}

pub(crate) fn preflight_run(
    host: &Host,
    engine: &Engine,
    scene_id: &str,
) -> Result<RunPreflight, String> {
    host.exit.ensure_new_operation()?;
    let connection = engine
        .store
        .connection_for_scene(scene_id)
        .map_err(|e| e.to_string())?;
    connection_host::validate_request_options(&connection)?;
    let transport = connection_host::transport_for(&connection)?;
    if connection.model.trim().is_empty() {
        return Err("请先设置连接和模型".into());
    }
    let key = if connection.advanced.auth_mode == mewu_core::ConnectionAuthMode::None {
        Zeroizing::new(String::new())
    } else {
        credentials::read_profile(&host.root, &connection)?
    };
    if !key.is_empty() && reqwest::header::HeaderValue::from_str(key.as_str()).is_err() {
        return Err("连接密钥格式无效".into());
    }
    Ok(RunPreflight {
        transport,
        key,
        connection,
    })
}

pub(crate) fn start_run(
    app: &AppHandle,
    engine: &mut Engine,
    context: RunContext,
    preflight: RunPreflight,
    attachments: ai::PreparedAttachments,
) -> Result<HostSnapshot, String> {
    start_run_with_visual(app, engine, context, preflight, attachments, None)
}

fn start_run_with_visual(
    app: &AppHandle,
    engine: &mut Engine,
    context: RunContext,
    preflight: RunPreflight,
    attachments: ai::PreparedAttachments,
    visual: Option<Arc<visual_annotation_host::RunScope>>,
) -> Result<HostSnapshot, String> {
    start_run_with_annotations(
        app,
        engine,
        context,
        preflight,
        Some(attachments),
        visual,
        None,
    )
}

fn start_run_with_annotations(
    app: &AppHandle,
    engine: &mut Engine,
    context: RunContext,
    preflight: RunPreflight,
    attachments: Option<ai::PreparedAttachments>,
    visual: Option<Arc<visual_annotation_host::RunScope>>,
    video: Option<Arc<video_annotation_tool::RunScope>>,
) -> Result<HostSnapshot, String> {
    let scene_id = context.scene_id.clone();
    if !engine.store.run_is_active(&scene_id, &context.run_id) {
        engine.plugin_runs.remove(&context.run_id);
        return Err("运行已取消、结束或被替换".into());
    }
    if context.connection_profile != preflight.connection {
        engine.plugin_runs.remove(&context.run_id);
        let snapshot = engine
            .store
            .fail_run(&scene_id, &context.run_id, "连接已更改，请重新发送")
            .map_err(|e| e.to_string())?;
        return Ok(publish(app, &snapshot));
    }
    let RunPreflight { transport, key, .. } = preflight;
    let run_tools =
        match mcp_host::RunTools::with_annotations(&context, visual.clone(), video.clone()) {
            Ok(tools) => Arc::new(tools),
            Err(error) => {
                engine.plugin_runs.remove(&context.run_id);
                let snapshot = engine
                    .store
                    .fail_run(&scene_id, &context.run_id, error)
                    .map_err(|e| e.to_string())?;
                return Ok(publish(app, &snapshot));
            }
        };
    let snapshot = engine.store.snapshot();
    let plugin_origin = engine.plugin_runs.get(&context.run_id).cloned();
    let bindings = run_tools
        .definitions()
        .iter()
        .map(|tool| {
            let name = tool
                .pointer("/function/name")
                .and_then(serde_json::Value::as_str)
                .ok_or("工具缺少名称")?;
            let binding = run_tools
                .journal_binding(name)
                .or_else(|| agent_tools::journal_binding(&context, name))
                .ok_or("工具缺少来源记录")?;
            Ok((name.to_owned(), binding))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, &str>>();
    let bindings = match bindings {
        Ok(bindings) => bindings,
        Err(error) => {
            engine.plugin_runs.remove(&context.run_id);
            let snapshot = engine
                .store
                .fail_run(&scene_id, &context.run_id, error)
                .map_err(|e| e.to_string())?;
            return Ok(publish(app, &snapshot));
        }
    };
    let annotation_origin = visual
        .as_ref()
        .map(|scope| scope.origin.clone())
        .or_else(|| video.as_ref().map(|scope| scope.origin.clone()));
    let journal = Arc::new(
        journal_runtime::RunJournal::new(
            app.clone(),
            scene_id.clone(),
            context.run_id.clone(),
            bindings,
            plugin_origin.clone(),
        )
        .with_visual(annotation_origin.clone()),
    );
    let (cancel, receiver) = oneshot::channel();
    engine.runs.insert(context.run_id.clone(), cancel);
    if let Some(origin) = annotation_origin {
        engine.visual_runs.insert(context.run_id.clone(), origin);
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let run_id = context.run_id.clone();
        let scene_id = context.scene_id.clone();
        let mut public_reasoning = String::new();
        let result = {
            let work = async {
                let app3 = app2.clone();
                let tools3 = run_tools.clone();
                let origin3 = plugin_origin.clone();
                let visual3 = visual.clone();
                let video3 = video.clone();
                let memory_query = context
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == mewu_core::MessageRole::User)
                    .map(|message| message.text.clone())
                    .unwrap_or_default();
                let mut body =
                    tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
                        let host = app3.state::<Host>();
                        let mut context = context;
                        let snapshot = if let Some(scope) = &video3 {
                            let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
                            plugins.store.visual_annotations(&scope.origin.grant)?;
                            let engine = host.lock()?;
                            host.exit.ensure_background()?;
                            scope.check_engine(&engine, &context.scene_id, &context.run_id)?;
                            engine.store.snapshot()
                        } else {
                            let attachments = ai::persist_attachments(
                                attachments.ok_or("本次运行缺少输入")?,
                                &host.assets,
                            )?;
                            let attachment_paths: Vec<_> =
                                attachments.iter().map(|a| a.path.clone()).collect();
                            let attached = (|| -> Result<Snapshot, String> {
                                let _plugins = visual3
                                    .as_ref()
                                    .map(|scope| {
                                        let plugins =
                                            host.plugins.lock().map_err(|_| "插件状态不可用")?;
                                        plugins.store.visual_annotations(&scope.origin.grant)?;
                                        Ok::<_, String>(plugins)
                                    })
                                    .transpose()?;
                                let mut engine = host.lock()?;
                                if !run_is_authorized(
                                    &engine,
                                    &context.scene_id,
                                    &context.run_id,
                                    origin3.as_ref(),
                                ) {
                                    return Err("运行已取消、结束或插件已停用".into());
                                }
                                if let Some(scope) = &visual3 {
                                    if !scope.live(&engine, &context.scene_id, &context.run_id) {
                                        return Err("原位作答已取消或插件已停用".into());
                                    }
                                    engine
                                        .store
                                        .attach_run_visual_assets(
                                            &context.scene_id,
                                            &context.run_id,
                                            attachments,
                                            scope.origin.grant.clone(),
                                            scope.input.clone(),
                                        )
                                        .map_err(|e| e.to_string())
                                } else {
                                    engine
                                        .store
                                        .attach_run_assets(
                                            &context.scene_id,
                                            &context.run_id,
                                            attachments,
                                        )
                                        .map_err(|e| e.to_string())
                                }
                            })();
                            match attached {
                                Ok(snapshot) => snapshot,
                                Err(error) => {
                                    // These exact UUID paths were created by this request and never attached.
                                    for path in attachment_paths {
                                        let path = std::path::Path::new(&path);
                                        if path.parent() == Some(host.assets.as_path()) {
                                            let _ = std::fs::remove_file(path);
                                        }
                                    }
                                    return Err(error);
                                }
                            }
                        };
                        let current_scene = snapshot
                            .scenes
                            .iter()
                            .find(|s| s.id == context.scene_id)
                            .ok_or("场景不存在")?;
                        context.messages = current_scene
                            .messages
                            .iter()
                            .skip(current_scene.conversation_start.unwrap_or(0))
                            .cloned()
                            .collect();
                        let mut body = ai::request_body(&context, &host.assets)?;
                        if let Some(scope) = &visual3 {
                            ai::bind_visual_request_targets(&mut body, &scope.input)?;
                        }
                        if let Some(scope) = &video3 {
                            ai::bind_video_request_targets(&mut body, &scope.input)?;
                        }
                        connection_host::apply_parameters(&mut body, &context.connection_profile)?;
                        agent_tools::augment_request(&mut body, &context)?;
                        tools3.augment_request(&mut body)?;
                        Ok(body)
                    })
                    .await
                    .map_err(|_| "准备消息失败")??;
                memory_tools::augment_initial(&app2, &scene_id, &run_id, &memory_query, &mut body)
                    .await?;
                agent_loop::run_recorded_with_reasoning(
                    transport,
                    &key,
                    body,
                    run_tools.definitions(),
                    |text| {
                        let host = app2.state::<Host>();
                        if let Ok(engine) = host.lock() {
                            if run_is_authorized(&engine, &scene_id, &run_id, plugin_origin.as_ref()) {
                                let _ = app2.emit_to("space", "run-event", serde_json::json!({
                                    "sceneId": scene_id, "runId": run_id, "status": "running", "text": text
                                }));
                            }
                        };
                    },
                    |text| {
                        let host = app2.state::<Host>();
                        if let Ok(engine) = host.lock() {
                            if run_is_authorized(&engine, &scene_id, &run_id, plugin_origin.as_ref()) {
                                public_reasoning.push_str(text);
                                let _ = app2.emit_to("space", "run-event", serde_json::json!({
                                    "sceneId": scene_id, "runId": run_id, "status": "running", "reasoning": text
                                }));
                            }
                        };
                    },
                    journal,
                    |call, lease| {
                        let app = app2.clone();
                        let scene_id = scene_id.clone();
                        let run_id = run_id.clone();
                        let tools = run_tools.clone();
                        async move {
                            if agent_tools::label(&call.name).is_some() {
                                journal_tool_host::execute(app, scene_id, run_id, call, lease).await
                            } else {
                                tools.execute_recorded(&app, &scene_id, &run_id, call, lease).await
                            }
                        }
                    },
                ).await
            };
            tokio::select! {biased; _=receiver=>None,result=work=>Some(result)}
        };
        let Some(result) = result else {
            run_tools.abort().await;
            let host = app2.state::<Host>();
            if let Ok(mut engine) = host.lock() {
                engine.runs.remove(&run_id);
                engine.plugin_runs.remove(&run_id);
                engine.visual_runs.remove(&run_id);
            }
            return;
        };
        run_tools.close().await;
        let host = app2.state::<Host>();
        if let Ok(mut engine) = host.lock() {
            let authorized = run_is_authorized(&engine, &scene_id, &run_id, plugin_origin.as_ref());
            engine.runs.remove(&run_id);
            engine.plugin_runs.remove(&run_id);
            engine.visual_runs.remove(&run_id);
            if !authorized {
                return;
            }
            let updated = match result {
                Ok(answer) => match assets::from_code(&answer, &host.assets) {
                    Ok(artifacts) => engine.store.finish_run_with_reasoning_and_assets(
                        &scene_id,
                        &run_id,
                        answer,
                        if public_reasoning.is_empty() {
                            None
                        } else {
                            Some(public_reasoning)
                        },
                        artifacts,
                    ),
                    Err(error) => engine.store.fail_run(&scene_id, &run_id, error),
                },
                Err(error) => engine.store.fail_run(&scene_id, &run_id, error),
            };
            if let Ok(snapshot) = updated {
                publish(&app2, &snapshot);
                host.memory.wake.notify_one();
            }
        };
    });
    Ok(publish(app, &snapshot))
}

#[cfg(test)]
mod run_authorization_tests {
    use super::*;

    #[test]
    fn revoked_plugin_origin_rejects_late_effects_even_if_scene_is_still_running() {
        let mut engine = Engine {
            store: Store::open_in_memory().unwrap(),
            runs: HashMap::new(),
            plugin_runs: HashMap::new(),
            visual_runs: HashMap::new(),
            ocr_jobs: ocr_host::OcrRegistry::default(),
            translation_jobs: translation_host::TranslationRegistry::default(),
        };
        let scene_id = engine.store.snapshot().active_scene_id;
        engine
            .store
            .apply(SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "测试工作流".into(),
            })
            .unwrap();
        let run = engine.store.begin_run(&scene_id).unwrap();
        let (sender, _receiver) = oneshot::channel();
        engine.runs.insert(run.run_id.clone(), sender);
        let origin = (scene_id.clone(), "example.table".to_owned(), 3);
        engine
            .plugin_runs
            .insert(run.run_id.clone(), origin.clone());
        assert!(run_is_authorized(
            &engine,
            &scene_id,
            &run.run_id,
            Some(&origin)
        ));
        // A transport has been canceled, but a failed SQLite cancellation can
        // leave the durable run in Running. Neither prepared attachments, late
        // deltas nor a response already waiting for cleanup may pass this fence.
        engine.plugin_runs.remove(&run.run_id);
        assert!(engine.store.run_is_active(&scene_id, &run.run_id));
        assert!(!run_is_authorized(
            &engine,
            &scene_id,
            &run.run_id,
            Some(&origin)
        ));
        engine
            .plugin_runs
            .insert(run.run_id.clone(), (scene_id.clone(), origin.1.clone(), 4));
        assert!(!run_is_authorized(
            &engine,
            &scene_id,
            &run.run_id,
            Some(&origin)
        ));
        // Ordinary conversation does not depend on a plugin registry entry.
        assert!(run_is_authorized(&engine, &scene_id, &run.run_id, None));
        // External memory has no workflow origin. Removing the transport
        // registration still fences late effects if durable cancellation fails.
        engine.runs.remove(&run.run_id);
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        let (sender, _receiver) = oneshot::channel();
        engine.runs.insert(run.run_id.clone(), sender);
        engine
            .plugin_runs
            .insert(run.run_id.clone(), origin.clone());
        engine.store.cancel_run(&scene_id).unwrap();
        assert!(!run_is_authorized(
            &engine,
            &scene_id,
            &run.run_id,
            Some(&origin)
        ));
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
    }
}

#[tauri::command]
fn cancel_run(
    app: AppHandle,
    host: tauri::State<'_, Host>,
    scene_id: String,
) -> Result<HostSnapshot, String> {
    let mut engine = host.lock()?;
    video_host::cancel_scene(&app, &scene_id);
    let run_id = engine
        .store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.run.as_ref())
        .map(|r| r.id.clone());
    let snapshot = engine
        .store
        .cancel_run(&scene_id)
        .map_err(|e| e.to_string())?;
    if let Some(id) = run_id {
        engine.plugin_runs.remove(&id);
        engine.visual_runs.remove(&id);
        if let Some(sender) = engine.runs.remove(&id) {
            let _ = sender.send(());
        }
    }
    Ok(publish(&app, &snapshot))
}

fn start_capture(app: &AppHandle) {
    if !app
        .try_state::<Host>()
        .is_some_and(|host| host.exit.accepts_new_operations() && host.storage.is_ready())
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = capture_screen(app.clone()).await {
            if error != capture_delay_work::CANCELED {
                let _ = app.emit_to("space", "host-error", error);
            }
        }
    });
}

static EARLY_QUIT: AtomicBool = AtomicBool::new(false);

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--verify-update") {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() != 3 { std::process::exit(2); }
        let result = app_update::verify_cli(tauri::generate_context!(), &std::path::PathBuf::from(&args[2]));
        std::process::exit(if result.is_ok() { 0 } else { 2 });
    }
    if std::env::args().nth(1).as_deref() == Some("--storage-info") {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() != 3 {
            std::process::exit(2);
        }
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let result = storage_cli::paths(&context.config().identifier)
            .and_then(|paths| storage_cli::write_info(&paths, &std::path::PathBuf::from(&args[2])));
        std::process::exit(if result.is_ok() { 0 } else { 2 });
    }
    if std::env::args().nth(1).as_deref() == Some("--formula-layout-worker") {
        std::process::exit(formula_layout::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--video-clip-worker") {
        std::process::exit(video_clip_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--code-worker") {
        std::process::exit(code_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--speech-worker") {
        std::process::exit(speech_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--recording-worker") {
        std::process::exit(recording_worker::worker_main());
    }
    let quit_only = lifecycle::launch_action(&std::env::args().collect::<Vec<_>>())
        == lifecycle::LaunchAction::Quit;
    // A cold --quit may initialize the single-instance plugin, but never opens
    // user storage, starts a content listener, or constructs a WebView.
    let server = if quit_only {
        None
    } else {
        match content_server::bind() {
            Ok(server) => Some(server),
            Err(error) => {
                rfd::MessageDialog::new()
                    .set_title("Mewu")
                    .set_description(error)
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                return;
            }
        }
    };
    let origin = server
        .as_ref()
        .map(|server| server.origin())
        .unwrap_or_default();
    let mut context = tauri::generate_context!();
    if quit_only {
        context.config_mut().app.windows.clear();
    }
    context.config_mut().app.security.csp=Some(tauri::utils::config::Csp::Policy(format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' {origin} blob: data:; font-src 'self'; connect-src ipc: http://ipc.localhost; media-src {origin}; frame-src {origin}; object-src 'none'; base-uri 'none'; form-action 'none'")));
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(
            |app, args, _| match lifecycle::launch_action(&args) {
                lifecycle::LaunchAction::Quit => {
                    if app.try_state::<Host>().is_some() {
                        lifecycle::request_shutdown(app);
                    } else {
                        EARLY_QUIT.store(true, Ordering::Release);
                    }
                }
                lifecycle::LaunchAction::Capture => start_capture(app),
                lifecycle::LaunchAction::Settings => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = open_settings(app.clone()).await {
                            let _ = app.emit_to("space", "host-error", error);
                        }
                    });
                }
                lifecycle::LaunchAction::Ignore => {}
            },
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed
                        && capture_shortcut_host::route_pressed(app, shortcut)
                    {
                        start_capture(app);
                    }
                })
                .build(),
        )
        .on_permission_request(|_, _| tauri::webview::PermissionResponse::Deny)
        .on_page_load(|webview, _| {
            #[cfg(windows)]
            {
                // This applies to every frame, including generated HTML artifacts.
                // Application context menus continue to receive DOM events.
                let _ = webview.with_webview(|native| unsafe {
                    if let Ok(core) = native.controller().CoreWebView2() {
                        if let Ok(settings) = core.Settings() {
                            let _ = settings.SetAreDefaultContextMenusEnabled(false);
                        }
                    }
                });
            }
        })
        .invoke_handler({
            let handlers: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                get_snapshot,
                capture_shortcut_host::get_capture_shortcut,
                settings_host::get_system_preferences,
                settings_host::set_system_preferences,
                settings_host::get_capture_preferences,
                settings_host::set_capture_preferences,
                settings_host::settings_info,
                app_update::app_update_status,
                app_update::check_app_update,
                app_update::install_app_update,
                settings_host::open_data_directory,
                settings_host::read_license_document,
                storage_host::get_data_directory_state,
                storage_host::choose_data_directory,
                storage_host::cancel_data_directory_proposal,
                storage_host::migrate_data_directory,
                data_host::get_data_usage,
                data_host::clean_data,
                blackboard_text_host::save_blackboard_text,
                capture_shortcut_host::set_capture_shortcut,
                capture_shortcut_host::begin_capture_shortcut_edit,
                capture_shortcut_host::end_capture_shortcut_edit,
                lifecycle::begin_exit_preparation,
                lifecycle::finish_exit_preparation,
                memory_page,
                memory_entry,
                journal_commands::get_run_journal,
                journal_commands::get_run_journal_event,
                journal_commands::preview_run_continuation,
                journal_commands::continue_run_from_journal,
                memory_host::get_memory_provider_state,
                memory_host::probe_memory_provider,
                memory_host::cancel_memory_request,
                memory_host::create_memory_binding,
                memory_host::select_memory_provider,
                memory_host::set_memory_sync,
                memory_host::retire_memory_binding,
                memory_host::external_memory_page,
                memory_host::external_memory_entry,
                memory_host::queue_external_memory,
                memory_host::forget_external_memory,
                apply_scene_command,
                capture_screen,
                blackboard::create_blackboard,
            blackboard::finish_blackboard,
            blackboard::open_blackboard,
                asset_host::import_assets,
                web_link_host::open_web_link,
                ui_language::set_ui_language,
                asset_host::export_text_asset,
                read_artifact,
                connection_host::get_provider_presets,
                connection_host::save_connection_profile,
                connection_host::delete_connection_profile,
                connection_host::discover_connection_models,
                connection_host::test_connection,
                connection_host::cancel_connection_probe,
                mcp_host::discover_mcp_server,
                plugin_host::get_plugins,
                plugin_host::get_plugin_catalog,
                plugin_host::prepare_plugin,
                plugin_host::install_plugin,
                plugin_host::set_plugin_enabled,
                plugin_host::uninstall_plugin,
                plugin_host::rollback_plugin,
                plugin_host::reinstall_official_plugin,
                plugin_host::prepare_plugin_update,
                plugin_host::run_plugin_workflow,
                plugin_host::apply_plugin_drawing,
                mosaic_preview::get_mosaic_preview,
                pixel_sampler::get_pointer_sample,
                get_window_snap_map,
                pin_host::run_plugin_pin,
                pin_host::cancel_plugin_pin,
                pin_host::get_pin_state,
                pin_host::objects::get_pin_objects,
                pin_host::objects::control_pin_object,
                pin_host::objects::reference_pin_object,
                pin_host::pin_ready,
                pin_host::control_pin,
                pin_host::show_pin_menu,
                pin_host::export_pin,
                pin_host::start_pin_drag,
                table_host::get_message_tables,
                table_host::export_message_table,
                scroll_host::start_scroll_capture,
                scroll_host::control_scroll_capture,
                scroll_host::get_scroll_status,
                ocr_host::run_plugin_ocr,
                speech_host::get_voice_capabilities,
                speech_host::run_plugin_dictation,
                speech_host::cancel_plugin_dictation,
                code_host::scan_plugin_codes,
                code_host::cancel_plugin_code_scan,
                code_host::act_on_code,
                ocr_host::cancel_plugin_ocr,
                translation_host::run_plugin_translation,
                translation_host::cancel_plugin_translation,
                translation_host::clear_region_translation,
                send_message,
                cancel_run,
                hide_space,
                open_settings,
                rich_annotation_host::get_drawing_layout_preview,
                video_annotation_host::get_video_annotation_plan,
                video_annotation_host::get_video_annotation_preview,
                video_annotation_host::get_video_annotation_text,
                video_drawing_host::get_video_annotation_vector,
                video_drawing_host::apply_video_drawing,
                video_annotation_host::edit_video_annotation_text,
                video_annotation_host::apply_video_annotation_document,
                rich_annotation_host::copy_drawing_table,
                rich_annotation_host::apply_drawing_document,
                raster_edit_host::apply_raster_edit,
                close_scene_window,
                hide_frozen_widget,
                export_region,
                export_snapshot,
                runtime_info,
                freeze_space,
                restore_frozen_scene,
                get_frozen_scenes,
                resize_frozen_widget,
                recording::get_recording_status,
                recording_audio_host::get_recording_audio,
                recording_audio_host::set_recording_audio,
                recording::start_recording,
                recording::control_recording,
                video_host::get_video_info,
                video_host::apply_video_edit,
                video_host::cancel_video_request,
                video_host::export_video,
                video_host::copy_video
            ];
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
                let allowed = invoke
                    .message
                    .webview_ref()
                    .try_state::<Host>()
                    .is_none_or(|host| {
                        host.exit.allows_command(invoke.message.command())
                            && host.storage.allows_command(invoke.message.command())
                    });
                if !allowed {
                    invoke.resolver.reject("正在退出，请稍候");
                    return true;
                }
                handlers(invoke)
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" && matches!(event, tauri::WindowEvent::Destroyed) {
                storage_host::invalidate_owner(window.app_handle());
            }
            if window.label() == "settings"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(false) | tauri::WindowEvent::Destroyed
                )
            {
                capture_shortcut_host::invalidate_owner(window.app_handle(), window.label());
            }
            if pin_host::is_pin_label(window.label()) {
                if matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
                    let _ = window.app_handle().emit_to("space", "pin-objects-changed", ());
                }
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    pin_host::destroyed(window.app_handle(), window.label());
                }
                // Pin close requests destroy only this independent picture;
                // never route them into scene hiding or recording cancellation.
                return;
            }
            if window.label() == "space" && matches!(event, tauri::WindowEvent::Focused(true)) {
                pin_window::arrange_for_space(window.app_handle());
            }
            if window.label() == "space"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(false) | tauri::WindowEvent::Destroyed
                )
            {
                pixel_sampler::invalidate(window.app_handle());
                code_host::cancel_owner(window.app_handle(), "space");
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                image_host::cancel_owner(window.app_handle(), window.label());
                speech_host::cancel_owner(window.app_handle(), window.label());
                code_host::cancel_owner(window.app_handle(), window.label());
                if window.label() == "space" {
                    video_host::cancel_all(window.app_handle());
                    raster_edit_host::cancel_all();
                    pin_host::cancel_pending(window.app_handle(), "space");
                }
                if matches!(window.label(), "space" | scroll_host::LABEL) {
                    scroll_host::cancel(window.app_handle());
                }
                if let Some(host) = window.app_handle().try_state::<Host>() {
                    if let Ok(engine) = host.lock() {
                        engine.ocr_jobs.cancel_owner(window.label());
                        engine.translation_jobs.cancel_owner(window.label());
                    }
                    if let Ok(mut probes) = host.connection_probes.lock() {
                        probes.cancel_owner(window.label());
                    }
                    host.memory.cancel_owner(window.label());
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if scroll_host::busy(window.app_handle())
                    && matches!(window.label(), "space" | scroll_host::LABEL)
                {
                    api.prevent_close();
                    scroll_host::cancel(window.app_handle());
                    return;
                }
                if window.label() == "settings" {
                    return;
                }
                api.prevent_close();
                if window
                    .app_handle()
                    .try_state::<Host>()
                    .is_some_and(|host| !host.exit.accepts_new_operations())
                {
                    return;
                }
                if window.label() == frozen::LABEL {
                    let _ = hide_frozen_widget(window.app_handle().clone());
                    return;
                }
                if recording::busy(window.app_handle()) {
                    recording::request_stop(window.app_handle());
                    return;
                }
                if window.label() == "space" {
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let scene_id = app
                            .state::<Host>()
                            .lock()
                            .map(|engine| engine.store.snapshot().active_scene_id);
                        if let Ok(scene_id) = scene_id {
                            if let Err(error) = close_scene_window(app.clone(), scene_id).await {
                                let _ = app.emit_to("space", "host-error", error);
                            }
                        }
                    });
                } else {
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            if quit_only || EARLY_QUIT.load(Ordering::Acquire) {
                app.handle().exit(0);
                return Ok(());
            }
            let storage_paths = storage_cli::paths(&app.config().identifier)?;
            // The single-instance lock can be released during restart cleanup,
            // before the old process closes SQLite. This lifetime lock cannot.
            let storage_lock = storage_root::acquire_runtime_lock(
                &storage_paths,
                std::time::Duration::from_secs(15),
            )?;
            let resolved = storage_root::resolve_startup(&storage_paths)?;
            asset_locations::install_context(resolved.locations.clone())?;
            let storage = storage_host::StorageRuntime::new(storage_paths.clone(), &resolved);
            let root = resolved.root.clone();
            let assets = root.join("assets");
            std::fs::create_dir_all(&assets)?;
            recording_storage::prepare(&root)?;
            recording_storage::prepare_video(&root)?;
            clipboard_video_storage::prepare(&root)?;
            let mut store = Store::open(root.join("spaces.db"))?;
            #[cfg(windows)]
            let legacy_import_warning = match legacy_connections::import_into(
                &mut store,
                &root,
                &app.path().local_data_dir()?.join("MewuAI"),
            ) {
                Ok(report) => {
                    report.routing_interrupted
                        || report.entries.iter().any(|entry| {
                            matches!(
                                entry.outcome,
                                legacy_connections::Outcome::Interrupted {}
                                    | legacy_connections::Outcome::Skipped { .. }
                            )
                        })
                }
                Err(_) => true,
            };
            let plugins = plugin_host::PluginRuntime::open(&root)?;
            let system_preferences = system_preferences::SystemPreferencesActor::open(&root);
            network_policy::initialize(system_preferences.view());
            if let Ok(value) = system_preferences.view() { startup_windows::initialize(&value); }
            app.manage(Host {
                storage,
                _storage_lock: storage_lock,
                engine: Mutex::new(Engine {
                    store,
                    runs: HashMap::new(),
                    plugin_runs: HashMap::new(),
                    visual_runs: HashMap::new(),
                    ocr_jobs: ocr_host::OcrRegistry::default(),
                    translation_jobs: translation_host::TranslationRegistry::default(),
                }),
                plugins: Mutex::new(plugins),
                connection_probes: Arc::new(Mutex::new(connection_host::ProbeRegistry::default())),
                capture_preferences: capture_preferences::CapturePreferencesActor::open(&root),
                system_preferences,
                settings_resources: settings_info::BuildInfo::new(
                    env!("CARGO_PKG_VERSION"),
                    option_env!("MEWU_BUILD_COMMIT"),
                    if env!("CARGO_PKG_VERSION").contains('-') { settings_info::ReleaseChannel::Alpha } else { settings_info::ReleaseChannel::Stable },
                )
                .and_then(|build| settings_info::SettingsResources::open(&root, build)),
                capture_jobs: Arc::new(capture_delay_work::CaptureJobs::default()),
                root,
                assets,
                capturing: Arc::new(AtomicBool::new(false)),
                legacy_import_warning: AtomicBool::new({
                    #[cfg(windows)]
                    {
                        legacy_import_warning
                    }
                    #[cfg(not(windows))]
                    {
                        false
                    }
                }),
                exit: lifecycle::ExitState::default(),
                frozen_dismissed: AtomicBool::new(false),
                revision: AtomicU64::new(0),
                content_origin: origin,
                window_snap: window_snap::WindowSnapRegistry::default(),
                asset_transfers: asset_host::Transfers::default(),
                memory: memory_runtime::Runtime::default(),
                speech: speech_host::SpeechRegistry::default(),
                codes: code_host::CodeRegistry::default(),
                video: video_host::VideoRegistry::default(),
                image_exports: image_host::Registry::default(),
            });
            frozen::build(app)?;
            app.manage(recording::RecordingState::default());
            app.manage(app_update::UpdateState::default());
            app_update::start_background_checks(app.handle().clone());
            app.manage(scroll_host::ScrollState::default());
            app.manage(pixel_sampler::PixelSampler::default());
            app.manage(pin_host::PinRegistry::default());
            capture_shortcut_host::initialize(app.handle(), &app.state::<Host>().root)?;
            recording_audio_host::initialize(app.handle(), &app.state::<Host>().root)?;
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            let settings = MenuItem::with_id(
                app,
                "settings",
                ui_language::text("设置", "Settings"),
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(
                app,
                "quit",
                ui_language::text("退出", "Quit"),
                true,
                None::<&str>,
            )?;
            app.manage(ui_language::TrayLabels {
                settings: settings.clone(),
                quit: quit.clone(),
            });
            let menu = Menu::with_items(app, &[&settings, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Mewu 1.0")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        start_capture(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "settings" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = open_settings(app.clone()).await {
                                // Never show a native error dialog during capture: it
                                // could enter the screenshot after the hide handshake.
                                let _ = app.emit_to("space", "host-error", error);
                            }
                        });
                    }
                    "quit" => lifecycle::request_shutdown(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            // A failed trial stays Activated and fails closed/retries the same
            // target on next launch. Never reopen the old source while a Host
            // still owns the copied databases or has accepted user input.
            if let Some(trial) = &resolved.trial {
                storage_root::commit_startup(&storage_paths, trial)?;
            } else {
                storage_root::mark_initialized(&storage_paths, &resolved)?;
            }
            app.state::<Host>().storage.mark_ready();
            memory_worker::start(app.handle());
            let content_app = app.handle().clone();
            std::thread::Builder::new()
                .name("mewu-content".into())
                .spawn(move || {
                    if let Some(server) = server {
                        server.serve(content_app);
                    }
                })?;
            if lifecycle::launch_action(&std::env::args().collect::<Vec<_>>())
                == lifecycle::LaunchAction::Settings
            {
                let app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = open_settings(app.clone()).await {
                        let _ = app.emit_to("space", "host-error", error);
                    }
                });
            }
            Ok(())
        });
    let application = builder.build(context);
    if let Err(error) = application.as_ref() {
        // Startup/storage errors must not silently replace persisted user scenes.
        rfd::MessageDialog::new()
            .set_title("Mewu")
            .set_description(format!("无法启动：{error}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    if let Ok(application) = application {
        application.run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if app
                    .try_state::<Host>()
                    .is_some_and(|host| !host.exit.is_ready())
                {
                    api.prevent_exit();
                    lifecycle::request_shutdown(app);
                }
            }
        });
    }
}
