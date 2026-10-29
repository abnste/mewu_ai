// SPDX-License-Identifier: MPL-2.0
#![cfg_attr(not(test), windows_subsystem = "windows")]
mod agent_context;
mod agent_development;
mod agent_execution;
mod agent_loop;
mod agent_session;
mod agent_tools;
mod agent_turn_context;
mod ai;
mod app_control_host;
mod command_output_preview;
mod app_control_integration;
mod app_update;
mod asset_host;
mod asset_locations;
mod assets;
mod blackboard;
mod capture;
mod capture_cursor;
mod capture_delay_work;
#[cfg(target_os = "macos")]
mod capture_macos;
#[cfg(target_os = "macos")]
mod capture_permission_macos;
mod capture_preferences;
#[cfg(windows)]
mod capture_presentation;
mod capture_shortcut;
#[cfg(target_os = "linux")]
mod capture_shortcut_desktop_linux;
mod capture_shortcut_host;
#[cfg(target_os = "linux")]
mod capture_shortcut_portal_linux;
#[cfg(target_os = "linux")]
mod capture_shortcut_wayland_host;
mod capture_visibility;
#[cfg(target_os = "linux")]
mod capture_wayland_host;
#[cfg(target_os = "linux")]
mod capture_wayland_portal;
mod clipboard_video_storage;
mod clipboard_text;
mod code_decode;
mod code_host;
mod code_open;
#[cfg(target_os = "macos")]
mod code_open_macos;
mod code_worker;
mod connection_host;
mod content_server;
#[path = "../../../../tools/mewu-cli/cli.rs"]
mod control_cli;
mod control_run_stream;
mod credentials;
#[cfg(target_os = "linux")]
mod desktop_session_linux;
mod drawing_diagnostics;
mod drawing_render;
mod export_variant_dialog;
mod extension_agent;
mod extension_agent_authority;
mod extension_chat_region;
#[cfg(windows)]
mod extension_escape;
#[cfg(windows)]
mod extension_focus;
mod extension_host;
mod extension_input;
mod extension_market;
mod extension_model_lease;
mod extension_appearance;
#[cfg(test)]
mod extension_appearance_tests;
mod extension_objects_host;
mod extension_overlay_policy;
mod extension_process;
mod extension_project;
mod extension_project_host;
mod extension_region;
#[cfg(target_os = "linux")]
mod extension_region_a11y_linux;
#[cfg(any(target_os = "linux", test))]
mod extension_region_desktop;
#[cfg(target_os = "linux")]
mod extension_region_gnome;
mod extension_store;
#[cfg(test)]
mod extension_surface_tests;
mod extension_surface_window;
mod extension_surfaces;
mod extension_tools;
mod extension_ui;
mod external_process_env;
#[cfg(target_os = "macos")]
mod font_macos;
mod formula_layout;
mod frozen;
#[cfg(target_os = "linux")]
mod gtk_dialog_linux;
mod highlight_foreground;
mod highlight_preview;
mod hooks;
mod host_preferences;
mod image_export;
mod image_host;
mod image_save_dialog;
mod import_pin_host;
mod inline_reasoning;
mod interaction_performance;
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
mod mosaic_preview;
mod network_policy;
mod ocr;
mod ocr_host;
#[cfg(any(target_os = "macos", test))]
mod ocr_worker;
mod optional_authorizations;
mod optional_capabilities;
mod owned_process;
mod pin_host;
mod pin_window;
mod pixel_sampler;
mod plugin_host;
mod plugins;
mod provider_transport;
mod raster_compositor;
mod raster_edit_algorithm;
mod raster_edit_host;
mod recording;
#[cfg(target_os = "linux")]
mod recording_aac_linux;
mod recording_audio;
mod recording_audio_host;
mod recording_audio_timeline;
mod recording_backend;
#[cfg(target_os = "linux")]
mod recording_codec_linux;
#[cfg(target_os = "linux")]
mod recording_mp4_audio_linux;
#[cfg(target_os = "linux")]
mod recording_pipewire_linux;
#[cfg(target_os = "linux")]
mod recording_portal_linux;
mod recording_preferences;
mod recording_storage;
#[cfg(target_os = "linux")]
mod recording_system_audio_linux;
#[cfg(target_os = "linux")]
mod recording_tray_linux;
#[cfg(target_os = "linux")]
mod recording_wayland_host;
#[cfg(target_os = "linux")]
mod recording_wayland_worker;
mod recording_worker;
mod run_cancellation;
mod screen_entities;
#[cfg(windows)]
mod shared_capture_windows;
mod sse_budget;
mod toolbar_catalog;
#[cfg(any(target_os = "linux", test))]
mod window_actions_linux;
#[cfg(any(target_os = "linux", test))]
mod window_witness_linux;
#[cfg(target_os = "macos")]
mod window_witness_macos;
#[cfg(windows)]
mod windows_identity;

#[tauri::command]
async fn recording_wayland_host_prepare(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
) -> Result<Option<HostSnapshot>, String> {
    #[cfg(target_os = "linux")]
    {
        crate::recording_wayland_host::prepare_wayland_recording(app, window, scene_id).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, window, scene_id);
        Ok(None)
    }
}
mod application_snapshot;
#[cfg(all(windows, test))]
mod application_snapshot_probe;
#[cfg(windows)]
mod application_snapshot_windows;
mod application_snapshot_worker;
mod blackboard_text_host;
mod data_host;
mod data_storage;
#[cfg(windows)]
mod hold_key_windows;
mod hold_sequence;
mod mp3_file;
mod quick_capture;
#[cfg(windows)]
mod quick_images_windows;
mod rich_annotation_host;
mod runtime_platform;
mod save_dialog;
mod scroll_capture;
mod scroll_host;
mod scroll_stitch;
mod session_title;
mod settings_host;
mod settings_info;
mod settings_window;
mod snapshot_pin;
mod speech_backend;
mod speech_host;
mod speech_worker;
#[cfg(any(target_os = "linux", test))]
mod startup_webviews;
mod startup_windows;
mod storage_cli;
mod storage_dependencies;
mod storage_host;
#[cfg(test)]
mod storage_integration_tests;
mod storage_root;
mod system_capture_shortcut;
mod system_preferences;
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
#[cfg(target_os = "macos")]
mod video_clip_output_macos;
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
mod window_creation_error;
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
    optional: optional_capabilities::Authorizations,
    agent_runs: HashMap<String, (String, extension_agent_authority::RunAgentAuthority)>,
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
        && engine
            .agent_runs
            .get(run_id)
            .is_some_and(|(scene, grant)| scene == scene_id && grant.is_current(engine, scene_id))
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
    #[cfg(windows)]
    capture_presentation: Arc<capture_presentation::Registry>,
    capture_preferences: capture_preferences::CapturePreferencesActor,
    system_preferences: system_preferences::SystemPreferencesActor,
    session_titles: session_title::Runtime,
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
    _storage_lock: Arc<storage_root::RuntimeLock>,
}
/// Reads only the immutable map collected beside this background image. No OS
/// window enumeration occurs here. A capture publishes its new background while
/// still hidden, so visibility/capture guards would lose that first map load.
#[tauri::command]
async fn get_window_snap_map(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    background_id: String,
) -> Result<Option<window_snap::FrameSnapMap>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        get_window_snap_map_blocking(window, &host, scene_id, background_id)
    })
    .await
    .map_err(|_| "截图选区读取中断".to_string())?
}

fn get_window_snap_map_blocking(
    window: tauri::WebviewWindow,
    host: &Host,
    scene_id: String,
    background_id: String,
) -> Result<Option<window_snap::FrameSnapMap>, String> {
    use interaction_performance::{Kind, Stage, Trace};
    let mut timing = Trace::start(Kind::WindowSnap);
    timing.mark(Stage::SnapGet);
    let result = (|| {
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
    })();
    timing.mark(match &result {
        Ok(Some(_)) => Stage::SnapAvailable,
        Ok(None) => Stage::SnapMissing,
        Err(_) => Stage::SnapDenied,
    });
    timing.finish(matches!(&result, Ok(Some(_))));
    result
}
#[tauri::command]
async fn recording_surface_ready(
    app: AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    stage: String,
) -> Result<bool, String> {
    recording::surface_ready(&app, window, &id, &stage).await
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
// Internal invalidation only. Full Snapshot/get_snapshot contracts stay unchanged.
#[derive(Clone, Serialize)]
struct SnapshotNotice {
    revision: u64,
}
fn emit_run_event(app: &AppHandle, value: serde_json::Value) -> tauri::Result<()> {
    app_control_host::observe_run_event(app, &value);
    app.emit_to("space", "run-event", &value)?;
    app.emit_to(frozen::LABEL, "run-event", &value)
}
fn publish(app: &AppHandle, snapshot: &Snapshot) -> HostSnapshot {
    publish_owned(app, snapshot.clone())
}
fn publish_owned(app: &AppHandle, owned: Snapshot) -> HostSnapshot {
    let mut timing = interaction_performance::Trace::start(interaction_performance::Kind::Publish);
    let snapshot = &owned;
    hooks::observe_scene(app, &snapshot.active_scene_id);
    #[cfg(target_os = "linux")]
    recording_wayland_host::reconcile(snapshot);
    app.state::<Host>().session_titles.reconcile(snapshot);
    speech_host::reconcile(app, snapshot);
    quick_capture::reconcile(app, snapshot);
    code_host::reconcile(app, snapshot);
    video_host::reconcile(app, snapshot);
    image_host::reconcile(app, snapshot);
    pixel_sampler::reconcile(app, snapshot);
    pin_host::reconcile(app, snapshot);
    extension_surface_window::reconcile_snapshot(app, snapshot);
    timing.mark(interaction_performance::Stage::Validated);
    let revision = app.state::<Host>().revision.fetch_add(1, Ordering::AcqRel) + 1;
    let snapshot = HostSnapshot {
        snapshot: owned,
        revision,
    };
    let notice = SnapshotNotice { revision };
    let _ = app.emit_to("space", "snapshot-revision", &notice);
    if app.get_webview_window("settings").is_some() {
        let _ = app.emit_to("settings", "snapshot-revision", &notice);
    }
    // A completed background run updates the small widget without showing or focusing it.
    let _ = app.emit_to(
        frozen::LABEL,
        "frozen-scenes",
        frozen::snapshot(&snapshot.snapshot, revision),
    );
    frozen::reconcile(app, &snapshot.snapshot);
    timing.mark(interaction_performance::Stage::Emitted);
    timing.finish(true);
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
        if let Err(error) = capture_visibility::apply(&window) {
            let _ = app.emit_to("space", "host-error", error);
            return;
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        pin_window::arrange_for_space(app);
    }
}

#[tauri::command]
async fn get_snapshot(app: AppHandle) -> Result<HostSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        get_snapshot_blocking(app.clone(), &host)
    })
    .await
    .map_err(|_| "空间状态读取中断".to_string())?
}

fn get_snapshot_blocking(app: AppHandle, host: &Host) -> Result<HostSnapshot, String> {
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
    // Normal commands need no extra Store read. A reserved exit may admit only
    // proven placement of an existing item; release this short borrow before
    // taking plugins, then repeat the proof under plugins -> Engine below.
    {
        let initial_engine = if host.exit.ensure_scene_command(&command).is_err()
            && matches!(&command, SceneCommand::UpdateItem { .. })
        {
            Some(host.lock()?)
        } else {
            None
        };
        ensure_scene_command_admission(
            &app,
            &host,
            window.label(),
            &command,
            initial_engine.as_ref().map(|engine| &engine.store),
        )?;
    }
    let command = match command {
        SceneCommand::FreezeScene { scene_id } => return freeze_space(app, scene_id).await,
        SceneCommand::CloseScene { scene_id } => return close_scene_window(app, scene_id).await,
        SceneCommand::AddDrawing { .. }
        | SceneCommand::UpdateDrawing { .. }
        | SceneCommand::UpdateDrawings { .. }
        | SceneCommand::DuplicateDrawings { .. }
        | SceneCommand::EraseDrawingStrokes { .. }
        | SceneCommand::RemoveDrawing { .. }
        | SceneCommand::RemoveDrawings { .. }
        | SceneCommand::UndoDrawing { .. }
        | SceneCommand::RedoDrawing { .. } => return Err("请从绘制工具操作".into()),
        command => command,
    };
    let _plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    ensure_scene_command_admission(&app, &host, window.label(), &command, Some(&engine.store))?;
    optional_capabilities::authoring_command(&engine, &command)?;
    let finishing_geometry = lifecycle::is_geometry_finish_command(&command);
    let disabled_memory = match &command {
        SceneCommand::SaveAgent { agent } if !agent.memory_enabled => Some(agent.id.clone()),
        SceneCommand::DeleteAgent { agent_id, .. } => Some(agent_id.clone()),
        _ => None,
    };
    let hook_event = match &command {
        SceneCommand::NewScene | SceneCommand::NewConversation { .. } => Some("scene.created"),
        _ => None,
    };
    let snapshot = engine.store.apply(command).map_err(|e| e.to_string())?;
    if let Some(event) = hook_event {
        hooks::lifecycle(
            &app,
            event,
            Some(&snapshot.active_scene_id),
            serde_json::Value::Null,
        );
    }
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
    Ok(publish_owned(&app, snapshot))
}

/// One durable capture operation, one full snapshot publication. The invoke
/// reply only identifies that publication instead of sending all history twice.
#[tauri::command]
async fn add_capture_region(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    background_id: String,
    region: mewu_core::Region,
    refs: Vec<mewu_core::Reference>,
) -> Result<u64, String> {
    if window.label() != "space" {
        return Err("此窗口不能添加截图选区".into());
    }
    let mut timing =
        interaction_performance::Trace::start(interaction_performance::Kind::AddRegion);
    let host = app.state::<Host>();
    let command = SceneCommand::AddRegion {
        scene_id: scene_id.clone(),
        region: region.clone(),
    };
    ensure_scene_command_admission(&app, &host, window.label(), &command, None)?;
    let _plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    let mut engine = host.lock()?;
    ensure_scene_command_admission(&app, &host, window.label(), &command, Some(&engine.store))?;
    optional_capabilities::authoring_command(&engine, &command)?;
    engine
        .store
        .active_background_for_preview(&scene_id, &background_id)
        .map_err(|error| error.to_string())?;
    timing.mark(interaction_performance::Stage::Validated);
    let snapshot = engine
        .store
        .add_region_with_refs(&scene_id, region, refs)
        .map_err(|error| error.to_string())?;
    timing.mark(interaction_performance::Stage::Applied);
    cancel_ended_runs(&mut engine, &snapshot);
    let published = publish_owned(&app, snapshot);
    timing.mark(interaction_performance::Stage::Published);
    timing.finish(true);
    Ok(published.revision)
}

fn ensure_scene_command_source(owner: &str, command: &SceneCommand) -> Result<(), String> {
    match command {
        SceneCommand::UpdateRegion { .. } => Err("选区更新方式已失效".into()),
        SceneCommand::EditImageAnnotations { .. }
        | SceneCommand::SetRegionGeometry { .. }
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
                | SceneCommand::EditImageAnnotations { .. }
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
    store: Option<&mewu_core::Store>,
) -> Result<(), String> {
    ensure_scene_command_source(owner, command)?;
    if let Some(store) = store {
        host.exit
            .ensure_scene_command_with_store(owner, command, store)?;
    } else {
        host.exit.ensure_scene_command(command)?;
    }
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
        engine.agent_runs.remove(&id);
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
    #[cfg(target_os = "linux")]
    recording_wayland_host::cancel_pending();
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
        snapshot.scenes.iter().any(|scene| {
            scene.frozen && scene.minimized && !scene.closed && frozen::has_content(scene)
        })
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
    let closed_scene_id = scene_id.clone();
    let host = app.state::<Host>();
    let _transition = begin_explicit_space_transition(&app, &host).await?;
    recording::ensure_idle(&app)?;
    let (was_active, has_frozen, published) = {
        let mut engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        let was_active = engine
            .store
            .active_scene()
            .map_err(|error| error.to_string())?
            .id
            == scene_id;
        let snapshot = engine
            .store
            .apply(SceneCommand::CloseScene { scene_id })
            .map_err(|error| error.to_string())?;
        cancel_ended_runs(&mut engine, &snapshot);
        let has_frozen = snapshot.scenes.iter().any(|scene| {
            scene.frozen && scene.minimized && !scene.closed && frozen::has_content(scene)
        });
        let published = publish_owned(&app, snapshot);
        (was_active, has_frozen, published)
    };
    quick_capture::scene_closed(&app, &closed_scene_id);
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
    hooks::lifecycle(
        &app,
        "scene.closed",
        Some(&closed_scene_id),
        serde_json::Value::Null,
    );
    Ok(published)
}

#[tauri::command]
async fn get_frozen_scenes(app: AppHandle) -> Result<frozen::FrozenSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        get_frozen_scenes_blocking(&host)
    })
    .await
    .map_err(|_| "会话列表读取中断".to_string())?
}

fn get_frozen_scenes_blocking(host: &Host) -> Result<frozen::FrozenSnapshot, String> {
    let engine = host.lock()?;
    Ok(frozen::snapshot_scenes(
        engine.store.scenes(),
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
    /// One explicit close/freeze intent may wait out a passive native restore.
    /// Longer captures still report busy; no Engine or exit mutex crosses await.
    async fn acquire_explicit(
        active: &Arc<AtomicBool>,
        exit: &lifecycle::ExitState,
        timeout: std::time::Duration,
    ) -> Result<Self, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            exit.ensure_new_operation()?;
            if let Some(transition) = Self::try_begin(active) {
                exit.ensure_new_operation()?;
                return Ok(transition);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err("请先完成当前采集或空间切换".into());
            }
            tokio::time::sleep_until((now + std::time::Duration::from_millis(16)).min(deadline))
                .await;
        }
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

async fn begin_explicit_space_transition(
    app: &AppHandle,
    host: &Host,
) -> Result<SpaceTransition, String> {
    host.exit.ensure_new_operation()?;
    if !host.storage.is_ready() {
        return Err("正在启动，请稍候".into());
    }
    let transition = SpaceTransition::acquire_explicit(
        &host.capturing,
        &host.exit,
        std::time::Duration::from_secs(2),
    )
    .await?;
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

    #[tokio::test]
    async fn one_close_or_freeze_waits_for_the_real_restore_owner_then_persists() {
        for freeze in [false, true] {
            let active = Arc::new(AtomicBool::new(false));
            let exit = lifecycle::ExitState::default();
            let restore = SpaceTransition::try_begin(&active).unwrap();
            let acquire = SpaceTransition::acquire_explicit(
                &active,
                &exit,
                std::time::Duration::from_secs(2),
            );
            tokio::pin!(acquire);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(25), &mut acquire,)
                    .await
                    .is_err()
            );
            drop(restore);
            let explicit = tokio::time::timeout(std::time::Duration::from_secs(1), acquire)
                .await
                .unwrap()
                .unwrap();
            let mut store = mewu_core::Store::open_in_memory().unwrap();
            let scene_id = store.snapshot().active_scene_id;
            store
                .apply(if freeze {
                    SceneCommand::FreezeScene {
                        scene_id: scene_id.clone(),
                    }
                } else {
                    SceneCommand::CloseScene {
                        scene_id: scene_id.clone(),
                    }
                })
                .unwrap();
            let snapshot = store.snapshot();
            let scene = snapshot.scenes.iter().find(|s| s.id == scene_id).unwrap();
            assert!(if freeze {
                scene.minimized && scene.frozen
            } else {
                scene.closed
            });
            assert!(SpaceTransition::try_begin(&active).is_none());
            drop(explicit);
            assert!(SpaceTransition::try_begin(&active).is_some());
        }
    }

    #[tokio::test]
    async fn explicit_scene_transition_still_reports_a_long_capture_as_busy() {
        let active = Arc::new(AtomicBool::new(false));
        let exit = lifecycle::ExitState::default();
        let _capture = SpaceTransition::try_begin(&active).unwrap();
        assert!(SpaceTransition::acquire_explicit(
            &active,
            &exit,
            std::time::Duration::from_millis(25),
        )
        .await
        .is_err_and(|error| error == "请先完成当前采集或空间切换"));
        assert!(active.load(Ordering::Acquire));
    }
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
    let _transition = begin_explicit_space_transition(&app, &host).await?;
    pixel_sampler::invalidate(&app);
    pin_host::cancel_pending(&app, "space");
    recording::ensure_idle(&app)?;
    let previously_minimized = {
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
        let previously_minimized = scene.minimized;
        engine
            .store
            .apply(SceneCommand::FreezeScene {
                scene_id: scene_id.clone(),
            })
            .map_err(|error| error.to_string())?;
        previously_minimized
    };
    if let Err(error) = frozen::layout(&app, false, true).await {
        // Window operations cannot share SQLite's transaction. Compensate selection
        // if the native transition fails so the user does not see a blank new scene.
        if let Ok(mut engine) = host.lock().and_then(|engine| {
            host.exit.ensure_background()?;
            Ok(engine)
        }) {
            if let Ok(restored) = engine
                .store
                .compensate_failed_minimize(&scene_id, previously_minimized)
            {
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
    let background = {
        let engine = host.lock()?;
        let scene = engine.store.scene(&scene_id).map_err(|_| "会话不存在")?;
        if !scene.frozen {
            return Err("该会话已在空间中打开".into());
        }
        scene.background.clone()
    };
    frozen::prepare_restore(&app, background.as_ref()).await?;
    let mut engine = host.lock()?;
    host.exit.ensure_new_operation()?;
    let snapshot = engine
        .store
        .apply(SceneCommand::ActivateScene { scene_id })
        .map_err(|error| error.to_string())?;
    let snapshot = publish_owned(&app, snapshot);
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
    platform: runtime_platform::RuntimePlatform,
    hotkey: String,
    capture_backend: &'static str,
    content_origin: String,
    measure_interactions: bool,
}
#[tauri::command]
fn runtime_info(app: AppHandle, host: tauri::State<'_, Host>) -> Result<RuntimeInfo, String> {
    Ok(RuntimeInfo {
        platform: runtime_platform::RuntimePlatform::current(),
        hotkey: capture_shortcut_host::active_display(&app),
        capture_backend: capture::backend_name(),
        content_origin: host.content_origin.clone(),
        measure_interactions: interaction_performance::enabled(),
    })
}

/// A queued hide() from a worker is not proof that the native window is hidden.
/// Handshake with the main loop before acquiring desktop pixels.
async fn hide_for_capture(
    window: &tauri::WebviewWindow,
    #[cfg(target_os = "linux")] allowed: capture::CaptureCheck,
) -> Result<(), String> {
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
                #[cfg(target_os = "linux")]
                allowed()?;
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
                #[cfg(target_os = "linux")]
                allowed()?;
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

#[cfg(target_os = "linux")]
async fn linux_capture_witnesses(
    window: &tauri::WebviewWindow,
    work: &Arc<capture_delay_work::Work>,
) -> Result<Vec<window_witness_linux::CaptureWitness>, String> {
    let (send, receive) = oneshot::channel();
    let target = window.clone();
    let work = work.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                work.check()?;
                let app = target.app_handle();
                app.state::<Host>().exit.ensure_new_operation()?;
                let mut witnesses = vec![window_witness_linux::capture_witness(&target)?];
                if let Some(widget) = app.get_webview_window(frozen::LABEL) {
                    witnesses.push(window_witness_linux::capture_witness(&widget)?);
                }
                work.check()?;
                for witness in &witnesses {
                    window_witness_linux::current(witness)?;
                }
                Ok(witnesses)
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "无法检查截图窗口")?;
    receive.await.map_err(|_| "截图窗口检查中断")?
}

#[cfg(target_os = "macos")]
fn mac_capture_hidden(witnesses: &[window_witness_macos::CaptureWitness]) -> Result<bool, String> {
    for witness in witnesses {
        if !window_witness_macos::hidden(witness)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn wait_mac_capture_hidden(
    witnesses: &[window_witness_macos::CaptureWitness],
    work: &capture_delay_work::Work,
    app: &AppHandle,
) -> Result<(), String> {
    // WindowServer can lag the acknowledged main-loop orderOut. Poll its real
    // state for a bounded interval. A short pause only limits polling traffic;
    // each observation still proves hiding through Quartz and its generation.
    // This runs only on the blocking worker.
    let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        work.check()?;
        app.state::<Host>().exit.ensure_new_operation()?;
        let hidden = mac_capture_hidden(witnesses)?;
        work.check()?;
        app.state::<Host>().exit.ensure_new_operation()?;
        if hidden {
            return Ok(());
        }
        let remaining = until.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("截图窗口尚未隐藏，请重新截图".into());
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(5)));
    }
}

/// Complete the native layout before publishing the new screenshot or showing it.
/// Merely sizing a resizable borderless window leaves resize chrome and does not
/// tell the Windows shell that the taskbar must yield to a fullscreen surface.
async fn expand_capture_space(
    window: &tauri::WebviewWindow,
    captured: &capture::CapturedScreen,
    #[cfg(target_os = "linux")] allowed: capture::CaptureCheck,
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
                #[cfg(target_os = "linux")]
                allowed()?;
                #[cfg(not(windows))]
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
                        // A hidden fullscreen window already on this monitor
                        // needs no resize or shell/fullscreen transition. Repeating
                        // them invalidates the WebView composition surface.
                        #[cfg(windows)]
                        let already_placed =
                            target.is_fullscreen().map_err(|_| "无法确认全屏状态")?
                                && !target.is_minimized().map_err(|_| "无法确认空间状态")?
                                && target.inner_size().map_err(|_| "无法确认空间尺寸")?
                                    == tauri::PhysicalSize::new(
                                        captured.display_width,
                                        captured.display_height,
                                    )
                                && target.outer_position().map_err(|_| "无法确认空间位置")?
                                    == tauri::PhysicalPosition::new(
                                        captured.origin_x,
                                        captured.origin_y,
                                    )
                                && target
                                    .current_monitor()
                                    .map_err(|_| "无法定位显示器")?
                                    .is_some_and(|monitor| {
                                        monitor.position().x == captured.origin_x
                                            && monitor.position().y == captured.origin_y
                                            && monitor.size().width == captured.display_width
                                            && monitor.size().height == captured.display_height
                                    });
                        #[cfg(not(windows))]
                        let already_placed = false;
                        if !already_placed {
                            #[cfg(windows)]
                            if target.is_fullscreen().map_err(|_| "无法确认全屏状态")? {
                                target.set_fullscreen(false).map_err(|_| "无法调整空间")?;
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
                #[cfg(windows)]
                shared_capture_windows::author_space_region(
                    &target,
                    shared_capture_windows::Rect::new(
                        captured.origin_x,
                        captured.origin_y,
                        captured.display_width,
                        captured.display_height,
                    )?,
                )?;
                #[cfg(target_os = "linux")]
                allowed()?;
                Ok(())
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "无法展开空间")?;
    receive.await.map_err(|_| "空间展开中断")?
}

/// Only the first-party Space can acknowledge its current, newly captured image.
/// This command cannot initiate capture, switch scenes, or show a window.
#[tauri::command]
async fn capture_background_ready(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    background_id: String,
    width: u32,
    height: u32,
    success: bool,
    diagnostics: Option<serde_json::Value>,
) -> Result<bool, String> {
    let timing = interaction_performance::Trace::start(interaction_performance::Kind::CaptureAck);
    tauri::async_runtime::spawn_blocking(move || {
        capture_background_ready_blocking(
            app,
            window,
            scene_id,
            background_id,
            width,
            height,
            success,
            diagnostics,
            timing,
        )
    })
    .await
    .map_err(|_| "截图呈现确认中断".to_string())?
}

fn capture_background_ready_blocking(
    app: AppHandle,
    window: tauri::WebviewWindow,
    scene_id: String,
    background_id: String,
    width: u32,
    height: u32,
    success: bool,
    diagnostics: Option<serde_json::Value>,
    mut timing: interaction_performance::Trace,
) -> Result<bool, String> {
    timing.mark(interaction_performance::Stage::AckWorkerStarted);
    let result = (|| -> Result<bool, String> {
        if window.label() != "space" {
            return Err("此窗口不能确认截图呈现".into());
        }
        let host = app.state::<Host>();
        host.exit.ensure_new_operation()?;
        #[cfg(windows)]
        {
            if window.is_visible().map_err(|_| "无法检查截图窗口")? {
                return Ok(false);
            }
            timing.mark(interaction_performance::Stage::AckVisibilityChecked);
            let engine = host.lock()?;
            timing.mark(interaction_performance::Stage::AckLocked);
            let asset = engine
                .store
                .active_background_for_preview(&scene_id, &background_id)
                .map_err(|error| error.to_string())?;
            if asset.width != Some(width) || asset.height != Some(height) {
                return Ok(false);
            }
            timing.mark(interaction_performance::Stage::AckIdentityChecked);
            host.capture_presentation.acknowledge(
                &capture_presentation::Identity {
                    scene: scene_id,
                    background: background_id,
                    width,
                    height,
                },
                success,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (scene_id, background_id, width, height, success);
            Ok(false)
        }
    })();
    if matches!(result, Ok(true)) {
        timing.mark(interaction_performance::Stage::AckAccepted);
        if success {
            interaction_performance::record_frontend(diagnostics);
        }
        timing.finish(success);
    } else {
        timing.finish(false);
    }
    result
}

#[tauri::command]
async fn capture_screen(app: AppHandle) -> Result<HostSnapshot, String> {
    quick_capture::detach(&app);
    capture_screen_inner(app, None).await
}
async fn capture_screen_inner(
    app: AppHandle,
    quick_generation: Option<u64>,
) -> Result<HostSnapshot, String> {
    let quick_was_visible = app
        .get_webview_window("space")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let mut timing = interaction_performance::Trace::start(interaction_performance::Kind::Capture);
    recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let Some(transition) = SpaceTransition::try_begin(&host.capturing) else {
        if quick_generation.is_some() {
            return Err("截图正在切换".into());
        }
        // Match the original BeginCaptureAsync: ignore another request while
        // capture or a window transition owns the surface. This is not an error
        // or a progress notification, and must not release the current owner.
        return get_snapshot_blocking(app.clone(), &host);
    };
    #[cfg(target_os = "linux")]
    let transition = Arc::new(transition);
    host.system_preferences
        .snapshot_for_operation()
        .map_err(|error| error.to_string())?;
    let preferences = host
        .capture_preferences
        .snapshot_for_capture()
        .map_err(|e| e.to_string())?;
    let Some(work) = host.capture_jobs.begin()? else {
        if quick_generation.is_some() {
            return Err("截图正在切换".into());
        }
        return get_snapshot_blocking(app.clone(), &host);
    };
    let _invoke = work.cancel_on_drop();
    #[cfg(target_os = "linux")]
    let work = Arc::new(work);
    #[cfg(target_os = "macos")]
    {
        // Request only for an explicit capture, before hiding windows and
        // without holding Engine/plugin locks across the OS permission UI.
        tauri::async_runtime::spawn_blocking(capture_permission_macos::request)
            .await
            .map_err(|_| "无法检查屏幕录制权限")??;
        work.check()?;
        host.exit.ensure_new_operation()?;
        if quick_generation.is_some_and(|generation| !quick_capture::current(generation)) {
            return Err(capture_delay_work::CANCELED.into());
        }
    }
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
    #[cfg(target_os = "linux")]
    if capture_wayland_host::is_wayland(&window, work.clone(), transition.clone()).await? {
        return capture_wayland_host::capture(
            app.clone(),
            window,
            work,
            transition,
            preferences,
            timing,
        )
        .await;
    }
    #[cfg(target_os = "linux")]
    let capture_witnesses = linux_capture_witnesses(&window, &work).await?;
    #[cfg(target_os = "linux")]
    let capture_check: capture::CaptureCheck = {
        let capture_work = work.clone();
        let check_app = app.clone();
        let witnesses = capture_witnesses.clone();
        Arc::new(move || {
            capture_work.check()?;
            check_app.state::<Host>().exit.ensure_new_operation()?;
            recording::ensure_idle(&check_app)?;
            for witness in &witnesses {
                window_witness_linux::current(witness)?;
            }
            Ok(())
        })
    };
    #[cfg(target_os = "macos")]
    let capture_witnesses = {
        let mut witnesses = vec![window_witness_macos::capture_witness(&window)?];
        if let Some(widget) = app.get_webview_window(frozen::LABEL) {
            witnesses.push(window_witness_macos::capture_witness(&widget)?);
        }
        witnesses
    };
    pin_window::capture_begin(&app);
    timing.mark(interaction_performance::Stage::Admitted);
    {
        let engine = host.lock()?;
        engine.ocr_jobs.cancel_owner("space");
        let scene_id = &engine
            .store
            .active_scene()
            .map_err(|error| error.to_string())?
            .id;
        engine.translation_jobs.cancel_scene("space", scene_id);
    }
    if let Err(error) = hide_for_capture(
        &window,
        #[cfg(target_os = "linux")]
        capture_check.clone(),
    )
    .await
    {
        #[cfg(not(target_os = "linux"))]
        show(&app);
        #[cfg(target_os = "linux")]
        if capture_check().is_ok() {
            show(&app);
        }
        return Err(error);
    }
    timing.mark(interaction_performance::Stage::Hidden);
    if let Err(error) = work
        .wait_delay(preferences.capture_delay_seconds.seconds(), || {
            host.exit.ensure_new_operation()?;
            recording::ensure_idle(&app)
        })
        .await
    {
        if work.check().is_ok() {
            #[cfg(target_os = "linux")]
            let restore = capture_check().is_ok();
            #[cfg(not(target_os = "linux"))]
            let restore = true;
            if restore {
                show(&app);
            }
        }
        return Err(error);
    }
    let root = host.assets.clone();
    let capture_app = app.clone();
    #[cfg(target_os = "macos")]
    let work = Arc::new(work);
    #[cfg(target_os = "macos")]
    let capture_check: capture::CaptureCheck = {
        let capture_work = work.clone();
        let check_app = app.clone();
        let witnesses = capture_witnesses.clone();
        Arc::new(move || {
            capture_work.check()?;
            check_app.state::<Host>().exit.ensure_new_operation()?;
            if !mac_capture_hidden(&witnesses)? {
                return Err("截图窗口在捕获期间重新显示，请重新截图".into());
            }
            Ok(())
        })
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let native_capture_check = capture_check.clone();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let adoption_witnesses = capture_witnesses.clone();
    let (work, _transition, captured) = tauri::async_runtime::spawn_blocking(move || {
        let captured = work.check().and_then(|_| {
            capture_app.state::<Host>().exit.ensure_new_operation()?;
            #[cfg(target_os = "macos")]
            {
                wait_mac_capture_hidden(&capture_witnesses, &work, &capture_app)?;
                let mut exclusions = Vec::with_capacity(capture_witnesses.len());
                for witness in &capture_witnesses {
                    work.check()?;
                    if !window_witness_macos::hidden(witness)? {
                        return Err("截图窗口尚未隐藏，请重新截图".into());
                    }
                    if let Some(exclusion) = window_witness_macos::exclusion(witness)? {
                        exclusions.push(exclusion);
                    }
                }
                work.check()?;
                return capture::capture_desktop_with_exclusions(
                    &root,
                    capture::CaptureOptions {
                        include_cursor: preferences.include_cursor,
                    },
                    &exclusions,
                    native_capture_check,
                );
            }
            #[cfg(target_os = "linux")]
            return capture::capture_desktop_with_witnesses(
                &root,
                capture::CaptureOptions {
                    include_cursor: preferences.include_cursor,
                },
                &capture_witnesses,
                native_capture_check,
            );
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
    timing.mark(interaction_performance::Stage::Captured);
    #[cfg(windows)]
    let mut presentation_required = false;
    let result = async {
        #[cfg(target_os = "linux")]
        let mut pending = captured?;
        #[cfg(target_os = "linux")]
        let mut captured = pending.screen.clone();
        #[cfg(not(target_os = "linux"))]
        let mut captured = captured?;
        work.check()?;
        host.exit.ensure_new_operation()?;
        if quick_generation.is_some_and(|generation| !quick_capture::current(generation)) {
            return Err(capture_delay_work::CANCELED.into());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        capture_check()?;
        expand_capture_space(
            &window,
            &captured,
            #[cfg(target_os = "linux")]
            capture_check.clone(),
        )
        .await?;
        timing.mark(interaction_performance::Stage::Expanded);
        #[cfg(target_os = "linux")]
        {
            let native_check = pending.native_check.clone().ok_or("截图缺少窗口状态见证")?;
            let native_transition = _transition.clone();
            // No X11 socket waits on the UI thread or under Engine locks. If
            // the invoke is dropped, this worker still owns the transition and
            // the original capture Work until its bounded native check ends.
            tauri::async_runtime::spawn_blocking(move || {
                let _transition = native_transition;
                native_check()
            })
            .await
            .map_err(|_| "截图状态检查中断")??;
        }
        let window_map = captured.window_map.take();
        let asset = Asset {
            capture_coordinates: Some(captured.capture_coordinates),
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
        // Keep the synchronous adoption separate from the async render wait:
        // neither Engine nor its SQLite connection crosses an await.
        let (id, snapshot) = {
            let mut engine = host.lock()?;
            work.check()?;
            host.exit.ensure_new_operation()?;
            #[cfg(target_os = "macos")]
            for witness in &adoption_witnesses {
                window_witness_macos::current(witness)?;
            }
            #[cfg(target_os = "linux")]
            for witness in &adoption_witnesses {
                window_witness_linux::current(witness)?;
            }
            let snapshot = engine
                .store
                .adopt_capture_background(asset.clone())
                .map_err(|e| e.to_string())?;
            let id = snapshot.active_scene_id.clone();
            #[cfg(target_os = "linux")]
            pending.retain();
            if let Some(map) = window_map {
                // Metadata is optional: a rejected or unavailable map must never
                // discard the successfully saved screenshot or its new scene.
                let _ = host.window_snap.insert(&asset, map);
            }
            (id, snapshot)
        };
        timing.mark(interaction_performance::Stage::Adopted);
        #[cfg(windows)]
        {
            presentation_required = true;
        }
        #[cfg(windows)]
        let presentation = host
            .capture_presentation
            .begin(capture_presentation::Identity {
                scene: id.clone(),
                background: asset.id.clone(),
                width: captured.width,
                height: captured.height,
            })?;
        let published = publish_owned(&app, snapshot);
        timing.mark(interaction_performance::Stage::Published);
        #[cfg(windows)]
        presentation
            .wait(|| {
                work.check()?;
                host.exit.ensure_new_operation()?;
                recording::ensure_idle(&app)?;
                host.lock()?
                    .store
                    .active_background_for_preview(&id, &asset.id)
                    .map_err(|error| error.to_string())?;
                Ok(())
            })
            .await?;
        #[cfg(windows)]
        timing.mark(interaction_performance::Stage::Decoded);
        Ok(published)
    }
    .await;
    if work.check().is_ok() {
        #[cfg(target_os = "linux")]
        let restore = capture_check().is_ok();
        #[cfg(not(any(target_os = "linux", windows)))]
        let restore = true;
        #[cfg(windows)]
        let restore = !presentation_required || result.is_ok();
        if restore
            && (quick_generation.is_none()
                || quick_generation.is_some_and(quick_capture::current)
                || quick_was_visible)
        {
            show(&app);
            timing.mark(interaction_performance::Stage::Shown);
        }
    }
    timing.finish(result.is_ok());
    if let Ok(published) = &result {
        hooks::capture_opened(&app, &published.snapshot);
    }
    result
}

#[tauri::command]
async fn read_artifact(app: AppHandle, asset_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let asset = host
            .lock()?
            .store
            .asset(&asset_id)
            .cloned()
            .ok_or("素材不存在")?;
        if !matches!(
            asset.kind,
            AssetKind::Html | AssetKind::Svg | AssetKind::Text
        ) {
            return Err("此素材不是文档".into());
        }
        assets::read_text(&asset, &host.assets)
    })
    .await
    .map_err(|_| "文档读取中断".to_string())?
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
    variant: Option<image_host::ImageExportVariant>,
) -> Result<bool, String> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    return image_host::export_region(app, window, scene_id, region_id, clipboard, variant).await;
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
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
    window: tauri::WebviewWindow,
    host: tauri::State<'_, Host>,
    scene_id: String,
    visual_annotations: Option<mewu_core::VisualAnnotationGrant>,
    region_id: Option<String>,
    automation_origin: Option<String>,
) -> Result<HostSnapshot, String> {
    host.exit.ensure_new_operation()?;
    recording::ensure_idle(&app)?;
    speech_host::cancel_owner(&app, "space");
    if region_id.is_some() && window.label() != "space" {
        return Err("请从当前截图对话条发送区域操作".into());
    }
    let prepared_region = {
        let engine = host.lock()?;
        let scene = engine.store.scene(&scene_id).map_err(|e| e.to_string())?;
        extension_chat_region::PreparedChatRegion::capture(
            scene,
            &engine.store.snapshot().active_scene_id,
            region_id.as_deref(),
        )?
    };
    let hook_draft = {
        let engine = host.lock()?;
        let scene = engine.store.scene(&scene_id).map_err(|e| e.to_string())?;
        ai::validate_scene_history_for_send(scene)?;
        scene.draft.clone()
    };
    let hook_parent =
        hooks::take_prompt_origin(&app, automation_origin.as_deref(), &scene_id, &hook_draft)?;
    let hook_context = hooks::before_prompt(&app, &scene_id, &hook_draft, hook_parent).await?;
    if host
        .lock()?
        .store
        .scene(&scene_id)
        .map_err(|e| e.to_string())?
        .draft
        != hook_draft
    {
        return Err("会话内容已变化，请重新发送".into());
    }
    if let Some(provider) = extension_host::external_agent_for_scene(&app, &scene_id)? {
        return extension_agent::start_external_run(
            app.clone(),
            scene_id,
            provider,
            prepared_region,
            hook_context,
        )
        .await;
    }
    let extension_catalog = extension_host::tool_catalog(&app, &scene_id)?;
    if let Some(grant) = &visual_annotations {
        host.plugins
            .lock()
            .map_err(|_| "插件状态不可用")?
            .store
            .visual_annotations(grant)?;
    }
    let (scene, preflight) = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.require_agent()?;
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
    let preflight = resolve_run_credentials(host.root.clone(), preflight).await?;
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
                app.state::<pin_host::PinRegistry>()
                    .hidden_image_assets(&scene)?,
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
    if let Some(prepared) = &prepared_region {
        prepared.check_source(&scene, &engine.store.snapshot().active_scene_id)?;
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
        .zip(
            attachments
                .as_ref()
                .and_then(|prepared| prepared.visual_input()),
        )
        .map(|(grant, input)| {
            // Engine -> Registry. A pin published during preparation cannot
            // leave an invisible writable target in this new run.
            let registry = app.state::<pin_host::PinRegistry>();
            let pins = registry.state.lock().map_err(|_| "贴图状态不可用")?;
            pins.ensure_visual_targets_visible(
                engine.store.scene(&scene_id).map_err(|e| e.to_string())?,
                input,
            )?;
            let input = input.clone();
            visual_annotation_host::RunScope::new(scene_id.clone(), grant.clone(), input)
                .map(|scope| Arc::new(scope.for_connection(&preflight.connection)))
        })
        .transpose()?;
    // External memory is supplied only by live executable extensions. Historical
    // declarative bindings remain readable for migration, never grant execution.
    let memory_grant = None;
    if let Some(scope) = &video {
        mcp_host::preflight_video_catalog(
            &engine.store.snapshot(),
            &scene_id,
            scope.clone(),
            memory_grant.is_some(),
        )?;
    }
    host.exit.ensure_new_operation()?;
    optional_capabilities::require_agent_current(&engine, &preflight.agent_grant)?;
    hook_context.check()?;
    let mut context = if let Some(prepared) = video_prepared.as_mut() {
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
    if !hook_context.instructions.is_empty() {
        context.agent.instructions.push_str("\n\n");
        context
            .agent
            .instructions
            .push_str(&hook_context.instructions.join("\n\n"));
    }
    hooks::begin_run(&app, &scene_id, &context.run_id, &hook_context);
    let chat_region = prepared_region
        .map(|prepared| prepared.bind(context.run_id.clone(), Arc::new(AtomicBool::new(false))));
    start_run_with_extensions(
        &app,
        &mut engine,
        context,
        preflight,
        attachments,
        visual,
        video,
        extension_catalog,
        chat_region,
    )
}

/// Host-only credentials for one already-validated connection. Never serialize
/// or expose these to manifests, plugin processes, window state, or diagnostics.
pub(crate) struct RunPreflight {
    agent_grant: optional_capabilities::Grant,
    transport: provider_transport::Transport,
    key: Zeroizing<String>,
    connection: ConnectionProfile,
}

/// Captured under the state lock; keyring access must happen after releasing it.
pub(crate) struct RunPreparation {
    agent_grant: optional_capabilities::Grant,
    transport: provider_transport::Transport,
    connection: ConnectionProfile,
}

pub(crate) async fn resolve_run_credentials(
    root: PathBuf,
    prepared: RunPreparation,
) -> Result<RunPreflight, String> {
    tauri::async_runtime::spawn_blocking(move || resolve_run_credentials_blocking(&root, prepared))
        .await
        .map_err(|_| "读取连接凭据中断".to_string())?
}

pub(crate) fn resolve_run_credentials_blocking(
    root: &std::path::Path,
    prepared: RunPreparation,
) -> Result<RunPreflight, String> {
    resolve_run_credentials_with(prepared, |connection| {
        credentials::read_profile(root, connection)
    })
}

fn resolve_run_credentials_with(
    prepared: RunPreparation,
    read: impl FnOnce(&ConnectionProfile) -> Result<Zeroizing<String>, String>,
) -> Result<RunPreflight, String> {
    let RunPreparation {
        agent_grant,
        transport,
        connection,
    } = prepared;
    let key = if connection.advanced.auth_mode == mewu_core::ConnectionAuthMode::None {
        Zeroizing::new(String::new())
    } else {
        read(&connection)?
    };
    if connection.advanced.auth_mode != mewu_core::ConnectionAuthMode::None && key.trim().is_empty()
    {
        return Err("请先填写连接密钥".into());
    }
    credentials::validate_key(&key)?;
    Ok(RunPreflight {
        agent_grant,
        transport,
        key,
        connection,
    })
}

#[cfg(test)]
mod run_preflight_tests {
    use super::*;
    fn prepared(auth: mewu_core::ConnectionAuthMode) -> RunPreparation {
        let connection = ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "synthetic".into(),
            provider_id: "openai-compatible".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "synthetic".into(),
            has_key: true,
            credential_id: None,
            revision: 1,
            advanced: mewu_core::ConnectionAdvanced {
                auth_mode: auth,
                ..Default::default()
            },
        };
        RunPreparation {
            transport: connection_host::transport_for(&connection).unwrap(),
            connection,
            agent_grant: optional_capabilities::Authorizations::test_defaults()
                .require_agent()
                .unwrap(),
        }
    }
    #[test]
    fn native_preflight_no_auth_never_opens_keyring() {
        let result =
            resolve_run_credentials_with(prepared(mewu_core::ConnectionAuthMode::None), |_| {
                panic!("keyring read")
            });
        assert!(result.unwrap().key.is_empty());
    }
    #[test]
    fn native_preflight_credentials_remain_required_and_invalid_values_are_not_disclosed() {
        use mewu_core::ConnectionAuthMode::Bearer;
        for value in ["", " ", "private-secret\nheader"] {
            let error = resolve_run_credentials_with(prepared(Bearer), |_| {
                Ok(Zeroizing::new(value.into()))
            })
            .err()
            .unwrap();
            assert!(!error.contains("private-secret"));
        }
        let candidate = prepared(Bearer);
        let profile = candidate.connection.clone();
        let result = resolve_run_credentials_with(candidate, |observed| {
            assert_eq!(observed, &profile);
            Ok(Zeroizing::new("synthetic-key".into()))
        })
        .unwrap();
        assert_eq!(result.connection, profile);
        assert_eq!(result.key.as_str(), "synthetic-key");
    }
}

pub(crate) async fn prepare_scene_attachments(
    scene: mewu_core::Scene,
    root: PathBuf,
) -> Result<ai::PreparedAttachments, String> {
    prepare_scene_attachments_with_visual(scene, root, false, std::collections::HashSet::new())
        .await
}

async fn prepare_scene_attachments_with_visual(
    scene: mewu_core::Scene,
    root: PathBuf,
    visual: bool,
    hidden_image_assets: std::collections::HashSet<String>,
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
            ai::prepare_scene_visual_attachments_excluding_pins(
                &scene,
                &root,
                visual_annotation_input::VisualCoordinateProfile::ExplicitNormalized1000V2,
                &hidden_image_assets,
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
) -> Result<RunPreparation, String> {
    host.exit.ensure_new_operation()?;
    let agent_grant = engine.optional.require_agent()?;
    let connection = engine
        .store
        .connection_for_scene(scene_id)
        .map_err(|e| e.to_string())?;
    connection_host::validate_request_options(&connection)?;
    let transport = connection_host::transport_for(&connection)?;
    if connection.model.trim().is_empty() {
        return Err("请先设置连接和模型".into());
    }
    Ok(RunPreparation {
        agent_grant,
        transport,
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
    start_run_with_extensions(
        app,
        engine,
        context,
        preflight,
        attachments,
        visual,
        video,
        vec![],
        None,
    )
}
fn start_run_with_extensions(
    app: &AppHandle,
    engine: &mut Engine,
    context: RunContext,
    preflight: RunPreflight,
    attachments: Option<ai::PreparedAttachments>,
    visual: Option<Arc<visual_annotation_host::RunScope>>,
    video: Option<Arc<video_annotation_tool::RunScope>>,
    extension_catalog: Vec<extension_tools::ToolDescriptor>,
    chat_region: Option<Arc<extension_chat_region::ChatRegionScope>>,
) -> Result<HostSnapshot, String> {
    let scene_id = context.scene_id.clone();
    optional_capabilities::require_agent_current(engine, &preflight.agent_grant)?;
    if !engine.store.run_is_active(&scene_id, &context.run_id) {
        engine.plugin_runs.remove(&context.run_id);
        engine.agent_runs.remove(&context.run_id);
        return Err("运行已取消、结束或被替换".into());
    }
    if context.connection_profile != preflight.connection {
        engine.plugin_runs.remove(&context.run_id);
        engine.agent_runs.remove(&context.run_id);
        let snapshot = engine
            .store
            .fail_run(&scene_id, &context.run_id, "连接已更改，请重新发送")
            .map_err(|e| e.to_string())?;
        return Ok(publish(app, &snapshot));
    }
    let RunPreflight {
        agent_grant,
        transport,
        key,
        connection,
    } = preflight;
    let key = Arc::new(key);
    engine.agent_runs.insert(
        context.run_id.clone(),
        (scene_id.clone(), agent_grant.into()),
    );
    let title_transport = transport.clone();
    let run_tools = match mcp_host::RunTools::with_extensions(
        &context,
        visual.clone(),
        video.clone(),
        extension_catalog,
        chat_region.clone(),
    ) {
        Ok(tools) => Arc::new(tools.with_model_key(&context, key.clone())),
        Err(error) => {
            engine.plugin_runs.remove(&context.run_id);
            engine.agent_runs.remove(&context.run_id);
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
            engine.agent_runs.remove(&context.run_id);
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
    app_control_host::begin_run_stream(app, &context.scene_id, &context.run_id);
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
                let extension_agent_id = context.agent.id.clone();
                let extension_memory_enabled = context.agent.memory_enabled;
                let extension_custom_memory = extension_memory_enabled
                    && extension_host::has_binding(&app2, &extension_agent_id, "memory")?;
                let app3 = app2.clone();
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
                let context = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
                    let host = app3.state::<Host>();
                    let mut context = context;
                    if extension_custom_memory {
                        context.memories.clear();
                        context.memory_authority = None;
                        context.agent.memory_enabled = false;
                    }
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
                    Ok(context)
                })
                .await
                .map_err(|_| "准备消息失败")??;
                let prepared_context = agent_context::prepare(
                    &app2,
                    &context,
                    &transport,
                    &key,
                    &run_tools.definitions(),
                    &journal,
                )
                .await?;
                let app3 = app2.clone();
                let tools3 = run_tools.clone();
                let visual3 = visual.clone();
                let video3 = video.clone();
                let mut body =
                    tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
                        let host = app3.state::<Host>();
                        let mut body = ai::request_body_with_context(
                            &context,
                            &host.assets,
                            prepared_context.first_kept,
                            prepared_context.summary.as_deref(),
                        )?;
                        if let Some(scope) = &visual3 {
                            ai::bind_visual_request_targets(&mut body, &scope.input)?;
                        }
                        if let Some(scope) = &video3 {
                            ai::bind_video_request_targets(&mut body, &scope.input)?;
                        }
                        connection_host::apply_parameters(&mut body, &context.connection_profile)?;
                        agent_tools::augment_request(&mut body, &context)?;
                        agent_development::augment_request(&mut body, &host)?;
                        prepared_context.augment(&mut body)?;
                        tools3.augment_request(&mut body)?;
                        Ok(body)
                    })
                    .await
                    .map_err(|_| "准备消息失败")??;
                let extension_filtered_tools: Vec<_> = run_tools
                    .definitions()
                    .into_iter()
                    .filter(|tool| {
                        !extension_custom_memory
                            || tool
                                .pointer("/function/name")
                                .and_then(serde_json::Value::as_str)
                                .is_none_or(|name| agent_tools::label(name).is_none())
                    })
                    .collect();
                if extension_custom_memory {
                    body["tools"] = serde_json::Value::Array(extension_filtered_tools.clone());
                }
                if extension_memory_enabled {
                    if extension_agent::recall(
                        &app2,
                        &extension_agent_id,
                        &scene_id,
                        &memory_query,
                        &mut body,
                    )
                    .await
                    .is_err()
                    {
                        journal.active()?;
                        extension_agent::append_memory_unavailable(&mut body)?;
                    }
                }
                let answer = agent_loop::run_recorded_with_reasoning(
                    transport,
                    &key,
                    body,
                    extension_filtered_tools,
                    |text| {
                        let host = app2.state::<Host>();
                        if let Ok(engine) = host.lock() {
                            if run_is_authorized(&engine, &scene_id, &run_id, plugin_origin.as_ref()) {
                                let _ = emit_run_event(&app2, serde_json::json!({
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
                                let _ = emit_run_event(&app2, serde_json::json!({
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
                            let tool_name=call.name.clone();
                            let result=if agent_development::handles(&call.name) {
                                agent_development::execute(app.clone(), scene_id.clone(), run_id.clone(), call, lease).await
                            } else if agent_tools::label(&call.name).is_some() {
                                journal_tool_host::execute(app.clone(), scene_id.clone(), run_id.clone(), call, lease).await
                            } else {
                                tools.execute_recorded(&app, &scene_id, &run_id, call, lease).await
                            };
                            hooks::tool_settled(&app,&scene_id,&run_id,&tool_name,&result);
                            result
                        }
                    },
                ).await?;
                if extension_memory_enabled {
                    if let Err(error) = extension_agent::retain(
                        &app2,
                        &extension_agent_id,
                        &scene_id,
                        &memory_query,
                        &answer,
                    )
                    .await
                    {
                        eprintln!("[extension-memory] retain failed: {error}");
                    }
                }
                Ok::<String, String>(answer)
            };
            tokio::select! {biased; _=receiver=>None, _=extension_chat_region::canceled(&app2, chat_region.as_ref())=>None, result=work=>Some(result)}
        };
        let Some(result) = result else {
            hooks::forget_run(&app2, &run_id);
            run_tools.abort().await;
            let host = app2.state::<Host>();
            if let Ok(mut engine) = host.lock() {
                engine.runs.remove(&run_id);
                engine.plugin_runs.remove(&run_id);
                engine.agent_runs.remove(&run_id);
                engine.visual_runs.remove(&run_id);
                if engine.store.run_is_active(&scene_id, &run_id) {
                    if let Ok(snapshot) = engine.store.cancel_run(&scene_id) {
                        publish(&app2, &snapshot);
                    }
                }
            }
            return;
        };
        run_tools.close().await;
        let host = app2.state::<Host>();
        if let Ok(mut engine) = host.lock() {
            let authorized = run_is_authorized(&engine, &scene_id, &run_id, plugin_origin.as_ref());
            engine.runs.remove(&run_id);
            engine.plugin_runs.remove(&run_id);
            engine.agent_runs.remove(&run_id);
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
                hooks::run_finished(&app2, &scene_id, &run_id, &snapshot);
                publish(&app2, &snapshot);
                host.memory.wake.notify_one();
                if plugin_origin.is_none() {
                    session_title::schedule(
                        &app2,
                        &mut engine,
                        &scene_id,
                        &run_id,
                        connection,
                        title_transport,
                        key,
                    );
                }
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
            optional: optional_capabilities::Authorizations::test_defaults(),
            agent_runs: HashMap::new(),
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
        let agent_grant = engine.optional.require_agent().unwrap();
        engine.agent_runs.insert(
            run.run_id.clone(),
            (scene_id.clone(), agent_grant.clone().into()),
        );
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
        engine.agent_runs.remove(&run.run_id);
        assert!(engine.store.run_is_active(&scene_id, &run.run_id));
        assert!(!run_is_authorized(
            &engine,
            &scene_id,
            &run.run_id,
            Some(&origin)
        ));
        // Restore Agent admission so the next assertion isolates the workflow
        // origin revision rather than failing only on the missing Agent grant.
        engine.agent_runs.insert(
            run.run_id.clone(),
            (scene_id.clone(), agent_grant.clone().into()),
        );
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
        // Ordinary conversation requires its accepted Agent grant but does not
        // require a workflow origin. Missing admission never gains authority.
        assert!(run_is_authorized(&engine, &scene_id, &run.run_id, None));
        engine.agent_runs.remove(&run.run_id);
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        engine.agent_runs.insert(
            run.run_id.clone(),
            (scene_id.clone(), agent_grant.clone().into()),
        );
        engine.optional.agent = None;
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        let revised_agent = optional_capabilities::Grant {
            revision: agent_grant.revision + 1,
            ..agent_grant.clone()
        };
        engine.optional.agent = Some(revised_agent);
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        // Disable then re-enable under a newer revision cannot revive the old
        // admitted run even though the plugin and contribution IDs are equal.
        engine.optional.agent = None;
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        let reenabling_agent = optional_capabilities::Grant {
            revision: agent_grant.revision + 2,
            ..agent_grant
        };
        engine.optional.agent = Some(reenabling_agent.clone());
        assert!(!run_is_authorized(&engine, &scene_id, &run.run_id, None));
        assert!(engine.store.run_is_active(&scene_id, &run.run_id));
        // Give this synthetic run fresh matching admission to isolate the
        // remaining transport and durable-run cancellation fences below.
        engine.agent_runs.insert(
            run.run_id.clone(),
            (scene_id.clone(), reenabling_agent.into()),
        );
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
async fn cancel_run(
    app: AppHandle,
    scene_id: String,
    expected_run_id: String,
) -> Result<HostSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        cancel_run_blocking(app.clone(), &host, scene_id, expected_run_id)
    })
    .await
    .map_err(|_| "取消对话中断".to_string())?
}

fn cancel_run_blocking(
    app: AppHandle,
    host: &Host,
    scene_id: String,
    expected_run_id: String,
) -> Result<HostSnapshot, String> {
    let mut engine = host.lock()?;
    let snapshot =
        run_cancellation::cancel_current(&mut engine.store, &scene_id, &expected_run_id, || {
            video_host::cancel_scene(&app, &scene_id)
        })
        .map_err(|error| error.to_string())?;
    engine.plugin_runs.remove(&expected_run_id);
    engine.agent_runs.remove(&expected_run_id);
    engine.visual_runs.remove(&expected_run_id);
    if let Some(sender) = engine.runs.remove(&expected_run_id) {
        let _ = sender.send(());
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
        #[cfg(target_os = "linux")]
        capture_shortcut_portal_linux::restore_after_ready(&app);
    });
}

static EARLY_QUIT: AtomicBool = AtomicBool::new(false);

/// The keyboard hook only posts a request; existing capture admission and the
/// UI thread own presentation. A closed/replaced shortcut cancels queued work.
#[cfg(windows)]
pub(crate) fn capture_system_shortcut_pressed(app: &AppHandle, epoch: u64) {
    let target = app.clone();
    let _ = app.run_on_main_thread(move || {
        if system_capture_shortcut::dispatch_allowed(epoch)
            && capture_shortcut_host::system_capture_allowed(&target)
        {
            hooks::system_shortcut_pressed(&target);
        }
    });
}

// macOS development builds embed one Info.plist symbol per macro expansion.
// Share the expansion while constructing a fresh context for each CLI path.
#[cfg(target_os = "macos")]
fn macos_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

#[cfg(all(target_os = "linux", feature = "linux-ui-diagnostics"))]
fn enable_console_diagnostics(view: &webkit2gtk::WebView) {
    use gtk::glib::prelude::ObjectExt;
    use webkit2gtk::WebViewExt;
    // WebView's "settings" property is write-only; use its public getter.
    if let Some(settings) = view.settings() {
        settings.set_property("enable-write-console-messages-to-stdout", true);
    }
}

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--cli")) {
        let mut args: Vec<_> = std::env::args_os().collect();
        args.remove(1);
        let code = match std::env::current_exe() {
            Ok(host) => control_cli::run_for_host(args, &host),
            Err(_) => 3,
        };
        std::process::exit(code);
    }
    // A code extension has only piped protocol access; do not open user storage,
    // register the single-instance plugin, or create any WebView in this child.
    if std::env::args().nth(1).as_deref() == Some("--extension-worker") {
        std::process::exit(if mewu_extension::run_stdio_worker().is_ok() {
            0
        } else {
            1
        });
    }
    let measurement_enabled = std::env::args().nth(1).as_deref() == Some("--measure-interactions");
    interaction_performance::initialize(measurement_enabled);
    // Production admits only the fixed native recording worker. Other sealed
    // worker domains and the Code parent retain their explicit test admission.
    #[cfg(target_os = "linux")]
    {
        if let Some(code) = owned_process::linux_worker::bootstrap_entry() {
            std::process::exit(code);
        }
        #[cfg(feature = "linux-worker-tests")]
        if std::env::args().nth(1).as_deref() == Some("--mewu-linux-package-code-probe") {
            std::process::exit(code_worker::linux_package_probe_main());
        }
    }
    if std::env::args().nth(1).as_deref() == Some("--verify-update") {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() != 3 {
            std::process::exit(2);
        }
        #[cfg(target_os = "macos")]
        let context = macos_context();
        #[cfg(not(target_os = "macos"))]
        let context = tauri::generate_context!();
        let result = app_update::verify_cli(context, &std::path::PathBuf::from(&args[2]));
        std::process::exit(if result.is_ok() { 0 } else { 2 });
    }
    if std::env::args().nth(1).as_deref() == Some("--storage-info") {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() != 3 {
            std::process::exit(2);
        }
        #[cfg(target_os = "macos")]
        let context = macos_context();
        #[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--ocr-worker") {
        std::process::exit(ocr_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--application-snapshot-worker") {
        std::process::exit(application_snapshot_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--speech-worker") {
        std::process::exit(speech_worker::worker_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--recording-worker") {
        std::process::exit(recording_worker::worker_main());
    }
    #[cfg(windows)]
    if let Err(error) = windows_identity::initialize() {
        // A Shell grouping failure must not prevent capture or normal startup.
        eprintln!(
            "[windows-identity] initialization failed: 0x{:08X}",
            error.code().0 as u32
        );
    }
    #[cfg(target_os = "linux")]
    desktop_session_linux::initialize();
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
    #[cfg(target_os = "macos")]
    let mut context = macos_context();
    #[cfg(not(target_os = "macos"))]
    let mut context = tauri::generate_context!();
    if quit_only {
        context.config_mut().app.windows.clear();
    }
    // WebKit writes its browser data as soon as a webview is constructed.
    // Resolve the business data root before it can populate the legacy path.
    #[cfg(target_os = "linux")]
    let deferred_windows = startup_webviews::defer(context.config_mut());
    #[cfg(target_os = "linux")]
    let mut startup_capture =
        lifecycle::LinuxStartupCapture::new(&std::env::args().collect::<Vec<_>>());
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
                    if hooks::route_shortcut(app, shortcut, event.state() == ShortcutState::Pressed)
                    {
                        return;
                    }
                })
                .build(),
        )
        .on_permission_request(|_, _| tauri::webview::PermissionResponse::Deny)
        .on_page_load(|webview, payload| {
            if webview.label() == "space"
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Started)
            {
                app_control_host::renderer_invalidated(webview.app_handle());
            }
            #[cfg(target_os = "linux")]
            if webview.label() == "settings" {
                if let Some(window) = webview.app_handle().get_webview_window("settings") {
                    let _ = capture_shortcut_wayland_host::observe_settings(&window);
                }
            }
            #[cfg(all(target_os = "linux", feature = "linux-ui-diagnostics"))]
            {
                let _ = webview.with_webview(|native| {
                    enable_console_diagnostics(&native.inner());
                });
            }
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
                recording_wayland_host_prepare,
                get_snapshot,
                hooks::get_hooks,
                hooks::save_hook,
                hooks::remove_hook,
                hooks::check_hook,
                hooks::run_hook,
                hooks::cancel_hook,
                hooks::edit_hook_shortcut,
                app_control_host::register_control_renderer,
                app_control_host::confirm_control_renderer,
                app_control_host::claim_control_request,
                app_control_host::complete_control_request,
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
                capture_shortcut_host::configure_capture_shortcut_portal,
                capture_shortcut_host::cancel_capture_shortcut_portal,
                capture_shortcut_host::open_capture_shortcut_system_settings,
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
                memory_host::retire_memory_binding,
                memory_host::external_memory_page,
                memory_host::external_memory_entry,
                memory_host::forget_external_memory,
                apply_scene_command,
                add_capture_region,
                capture_screen,
                capture_background_ready,
                drawing_diagnostics::record_drawing_diagnostics,
                blackboard::create_blackboard,
                blackboard::finish_blackboard,
                blackboard::open_blackboard,
                asset_host::import_assets,
                asset_host::restore_message_artifact,
                web_link_host::open_web_link,
                ui_language::set_ui_language,
                asset_host::export_text_asset,
                image_host::export_image_item,
                image_host::export_control_image,
                read_artifact,
                connection_host::get_provider_presets,
                connection_host::save_connection_profile,
                connection_host::read_connection_key,
                connection_host::delete_connection_profile,
                connection_host::discover_connection_models,
                connection_host::test_connection,
                connection_host::cancel_connection_probe,
                mcp_host::discover_mcp_server,
                plugin_host::get_plugins,
                plugin_host::get_official_plugins,
                plugin_host::control_official_plugin,
                extension_host::get_extensions,
                extension_appearance::get_extension_appearance,
                toolbar_catalog::publish_toolbar_catalog,
                extension_host::get_extension_bindings,
                extension_host::set_extension_binding,
                extension_host::get_extension_catalog,
                extension_host::get_extension_project,
                extension_host::get_extension_project_image,
                extension_host::prepare_extension,
                agent_development::list_plugin_projects,
                agent_development::prepare_plugin_project,
                extension_host::install_extension,
                extension_host::control_extension,
                extension_host::invoke_extension,
                extension_host::actions::resolve_extension_action,
                extension_host::actions::execute_extension_action,
                extension_surface_window::open_extension_surface_window,
                extension_surface_window::get_extension_surface_window,
                extension_surface_window::control_extension_surface_window,
                extension_surface_window::release_extension_surface_window,
                extension_surface_window::request_close_extension_surface_window,
                extension_surface_window::extension_surface_viewport_point,
                extension_surface_window::publish_extension_surface_targets,
                extension_surface_window::forward_extension_surface_event,
                extension_host::stop_extension,
                plugin_host::run_plugin_workflow,
                plugin_host::apply_plugin_drawing,
                mosaic_preview::get_mosaic_preview,
                highlight_preview::get_highlight_preview,
                pixel_sampler::get_pointer_sample,
                get_window_snap_map,
                pin_host::run_plugin_pin,
                pin_host::media::pin_scene_item,
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
                scroll_host::scroll_surface_ready,
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
                optional_capabilities::get_optional_capabilities,
                cancel_run,
                hide_space,
                quick_capture::quick_interaction_state,
                quick_capture::quick_interaction_open,
                quick_capture::quick_interaction_recognize,
                quick_capture::quick_interaction_images,
                quick_capture::quick_interaction_touch,
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
                clipboard_text::write_clipboard_text,
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
                recording_surface_ready,
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
            if let Some(id) = extension_surface_window::id(window.label()) {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    extension_surface_window::request_close(window.app_handle(), id);
                }
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    extension_surface_window::destroyed(window.app_handle(), id);
                }
                return;
            }
            if matches!(window.label(), "space" | "settings")
                && matches!(event, tauri::WindowEvent::Destroyed)
            {
                if let Some(extensions) = window
                    .app_handle()
                    .try_state::<extension_host::ExtensionHost>()
                {
                    extensions.window_destroyed(window.label());
                }
            }
            if window.label() == "space" && matches!(event, tauri::WindowEvent::Destroyed) {
                app_control_host::renderer_invalidated(window.app_handle());
            }
            if window.label() == frozen::LABEL && matches!(event, tauri::WindowEvent::Destroyed) {
                frozen::destroyed(window.app_handle());
            }
            #[cfg(target_os = "linux")]
            if matches!(event, tauri::WindowEvent::Destroyed) {
                // Revoke before any Engine locks or async cancellation work.
                window_witness_linux::revoke_owner(window.label());
                capture_wayland_host::revoke_owner(window.label());
                window_actions_linux::revoke_owner(window.label());
                if matches!(window.label(), "space" | frozen::LABEL) {
                    if let Some(host) = window.app_handle().try_state::<Host>() {
                        host.capture_jobs.cancel();
                    }
                }
            }
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window_witness_macos::revoke_owner(window.label());
                if matches!(window.label(), "space" | frozen::LABEL) {
                    if let Some(host) = window.app_handle().try_state::<Host>() {
                        host.capture_jobs.cancel();
                    }
                }
            }
            if window.label() == "settings" && matches!(event, tauri::WindowEvent::Destroyed) {
                hooks::window_closed(window.app_handle(), window.label());
                storage_host::invalidate_owner(window.app_handle());
            }
            let wayland_shortcut = {
                #[cfg(target_os = "linux")]
                {
                    window
                        .app_handle()
                        .try_state::<capture_shortcut_portal_linux::Runtime>()
                        .is_some()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    false
                }
            };
            if window.label() == "settings"
                && capture_shortcut_host::owner_event_invalidates(&event, wayland_shortcut)
            {
                capture_shortcut_host::invalidate_owner(window.app_handle(), window.label());
            }
            if pin_host::is_pin_label(window.label()) {
                if matches!(
                    event,
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                ) {
                    let _ = window
                        .app_handle()
                        .emit_to("space", "pin-objects-changed", ());
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
            #[cfg(target_os = "linux")]
            desktop_session_linux::observe_display();
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
            #[cfg(target_os = "macos")]
            storage_root::prepare_macos_storage_identity(&storage_paths, &storage_lock)?;
            let storage_lock = Arc::new(storage_lock);
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
            if measurement_enabled {
                store.set_persistence_observer(Some(interaction_performance::record_persistence));
            }
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
            let existing_blackboard = store
                .scenes()
                .iter()
                .any(optional_capabilities::is_blackboard);
            let plugins =
                plugin_host::PluginRuntime::open_with_blackboard(&root, existing_blackboard)?;
            let optional =
                optional_capabilities::Authorizations::from_snapshot(&plugins.store.snapshot()?);
            let system_preferences = system_preferences::SystemPreferencesActor::open(&root);
            network_policy::initialize(system_preferences.view());
            if let Ok(value) = system_preferences.view() {
                startup_windows::initialize(&value);
            }
            app.manage(extension_host::ExtensionHost::open(
                &root,
                storage_lock.clone(),
            )?);
            app.manage(Host {
                storage,
                _storage_lock: storage_lock,
                engine: Mutex::new(Engine {
                    store,
                    optional,
                    agent_runs: HashMap::new(),
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
                session_titles: session_title::Runtime::default(),
                settings_resources: settings_info::BuildInfo::new(
                    env!("CARGO_PKG_VERSION"),
                    option_env!("MEWU_BUILD_COMMIT"),
                    if env!("CARGO_PKG_VERSION").contains('-') {
                        settings_info::ReleaseChannel::Alpha
                    } else {
                        settings_info::ReleaseChannel::Stable
                    },
                )
                .and_then(|build| settings_info::SettingsResources::open(&root, build)),
                capture_jobs: Arc::new(capture_delay_work::CaptureJobs::default()),
                #[cfg(windows)]
                capture_presentation: Arc::new(capture_presentation::Registry::default()),
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
            app.manage(frozen::Lifecycle::default());
            app.manage(app_control_host::Runtime::default());
            #[cfg(target_os = "linux")]
            for config in &deferred_windows {
                tauri::WebviewWindowBuilder::from_config(app.handle(), config)?.build()?;
            }
            app.manage(recording::RecordingState::default());
            app.manage(app_update::UpdateState::default());
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            app_update::start_background_checks(app.handle().clone());
            app.manage(scroll_host::ScrollState::default());
            app.manage(pixel_sampler::PixelSampler::default());
            app.manage(pin_host::PinRegistry::default());
            capture_shortcut_host::initialize(app.handle(), &app.state::<Host>().root)?;
            recording_audio_host::initialize(app.handle(), &app.state::<Host>().root)?;
            {
                let host = app.state::<Host>();
                let snapshot = host
                    .plugins
                    .lock()
                    .map_err(|_| "插件状态不可用")?
                    .store
                    .snapshot()?;
                hooks::Runtime::initialize(app.handle(), &host.root, &snapshot)?;
            }
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            // AppIndicator does not report tray left-click events on Linux.
            // Keep a native menu entry into the same capture admission path.
            #[cfg(target_os = "linux")]
            let capture = MenuItem::with_id(
                app,
                "capture",
                ui_language::text("截图", "Capture"),
                true,
                None::<&str>,
            )?;
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
            #[cfg(target_os = "linux")]
            let menu = Menu::with_items(app, &[&capture, &settings, &quit])?;
            #[cfg(not(target_os = "linux"))]
            let menu = Menu::with_items(app, &[&settings, &quit])?;
            app.manage(ui_language::TrayLabels {
                #[cfg(target_os = "linux")]
                capture: capture.clone(),
                #[cfg(target_os = "linux")]
                recording_controls: recording_tray_linux::Controls::new(menu.clone()),
                settings: settings.clone(),
                quit: quit.clone(),
            });
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("MewuAI")
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
                    #[cfg(target_os = "linux")]
                    "capture" => start_capture(app),
                    #[cfg(target_os = "linux")]
                    id if recording_tray_linux::handle(app, id) => {}
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
            {
                let snapshot = app
                    .state::<Host>()
                    .plugins
                    .lock()
                    .map_err(|_| "插件状态不可用")?
                    .store
                    .snapshot()?;
                app_control_host::reconcile(app.handle(), &snapshot);
            }
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
                    let result = open_settings(app.clone()).await;
                    // open_settings has dropped its SpaceTransition now. A
                    // Ready-time restore would see capturing=true and remain
                    // deferred with no subsequent idle notification.
                    #[cfg(target_os = "linux")]
                    capture_shortcut_portal_linux::restore_after_ready(&app);
                    if let Err(error) = result {
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
        application.run(move |app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                hooks::ready(app);
            }
            #[cfg(target_os = "linux")]
            if matches!(event, tauri::RunEvent::Ready) {
                if startup_capture.take_ready(&event) {
                    start_capture(app);
                } else if startup_capture.restore_at_ready() {
                    capture_shortcut_portal_linux::restore_after_ready(app);
                }
            }
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
