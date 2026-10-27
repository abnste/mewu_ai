// SPDX-License-Identifier: MPL-2.0
use crate::{
    plugins::{self, PluginManifest, PluginSnapshot, PluginSource, PluginState, PluginStore},
    Host, HostSnapshot,
};
use mewu_core::SceneCommand;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::Path,
};
use tauri::{AppHandle, Emitter, Manager};

pub struct PluginRuntime { pub store: PluginStore }
impl PluginRuntime {
    pub fn open(root: &Path) -> Result<Self, String> { Self::open_with_blackboard(root, false) }
    pub fn open_with_blackboard(root: &Path, existing_blackboard: bool) -> Result<Self, String> {
        Ok(Self { store: PluginStore::open_with_blackboard(root, existing_blackboard)? })
    }
}
fn lock(host: &Host) -> Result<std::sync::MutexGuard<'_, PluginRuntime>, String> {
    host.plugins
        .lock()
        .map_err(|_| "插件存储需要重新启动".into())
}
fn publish(app: &AppHandle, snapshot: &PluginSnapshot) {
    crate::hooks::reconcile(app, snapshot);
    crate::hooks::lifecycle(app, "plugin.changed", None, serde_json::json!({"revision": snapshot.revision}));
    crate::video_host::revoke_plugins(app, snapshot);
    crate::recording::revoke_plugins(app, snapshot);
    crate::recording_audio_host::reconcile(app, snapshot);
    crate::speech_host::revoke_plugins(app, snapshot);
    crate::code_host::revoke_plugins(app, snapshot);
    crate::scroll_host::revoke_plugins(app, snapshot);
    crate::pin_host::revoke_plugins(app, snapshot);
    let _ = app.emit_to("space", "plugin-state", snapshot);
    let _ = app.emit_to("settings", "plugin-state", snapshot);
}
fn revoked_runs(
    origins: &HashMap<String, (String, String, u64)>,
    snapshot: &PluginSnapshot,
) -> Vec<(String, String)> {
    origins
        .iter()
        .filter(|(_, (_, id, revision))| {
            !snapshot.plugins.iter().any(|p| {
                &p.manifest.id == id
                    && p.revision == *revision
                    && p.state == PluginState::Enabled
                    && p.error.is_none()
            })
        })
        .map(|(run, (scene, _, _))| (run.clone(), scene.clone()))
        .collect()
}
fn settle_engine(engine: &mut crate::Engine, snapshot: &PluginSnapshot) -> Result<(), String> {
    let previous_agent = engine.optional.agent.clone();
    engine.optional = crate::optional_capabilities::Authorizations::from_snapshot(snapshot);
    if previous_agent != engine.optional.agent {
        engine.translation_jobs.cancel_all();
    }
    engine.ocr_jobs.revoke_plugins(snapshot);
    engine.translation_jobs.revoke_plugins(snapshot);
    let mut ended = revoked_runs(&engine.plugin_runs, snapshot);
    ended.extend(engine.agent_runs.iter()
        .filter(|(_, (scene, grant))| !grant.is_current(engine,scene))
        .map(|(run, (scene, _))| (run.clone(), scene.clone())));
    ended.extend(
        engine
            .visual_runs
            .iter()
            .filter(|(_, origin)| {
                !crate::plugins::visual_annotation_grant_is_current(&origin.grant, snapshot)
            })
            .map(|(run, origin)| (run.clone(), origin.scene_id.clone())),
    );
    for scene in engine.store.snapshot().scenes {
        if let Some(run) = scene.run {
            if let Some(mewu_core::RunMemoryAuthority::External { grant, .. }) =
                run.memory_authority
            {
                if !snapshot.plugins.iter().any(|plugin| {
                    plugin.manifest.id == grant.plugin_id
                        && plugin.revision == grant.plugin_revision
                        && plugin.state == PluginState::Enabled
                        && plugin.error.is_none()
                }) {
                    ended.push((run.id, scene.id));
                }
            }
        }
    }
    ended.sort();
    ended.dedup();
    let mut failure = None;
    for (run, scene) in ended {
        if let Some(sender) = engine.runs.remove(&run) {
            let _ = sender.send(());
        }
        engine.plugin_runs.remove(&run);
        engine.agent_runs.remove(&run);
        if let Some(origin) = engine.visual_runs.remove(&run) {
            let revoked = match origin.source_kind {
                crate::visual_annotation_host::SourceKind::Image => engine.store.revoke_visual_authority(&scene, &run),
                crate::visual_annotation_host::SourceKind::Video => engine.store.revoke_video_authority(&scene, &run),
            };
            if let Err(e) = revoked {
                failure = Some(e.to_string());
            }
        }
        if engine.store.run_is_active(&scene, &run) {
            if let Err(e) = engine.store.cancel_run(&scene) {
                failure = Some(e.to_string());
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
/// All plugin mutations and starts use plugin -> Engine lock order.
fn settle_runs(app: &AppHandle, host: &Host, snapshot: &PluginSnapshot) -> Result<(), String> {
    crate::hooks::reconcile(app, snapshot);
    crate::app_control_host::reconcile(app, snapshot);
    let next_agent = crate::optional_capabilities::Authorizations::from_snapshot(snapshot).agent;
    if host.lock()?.optional.agent != next_agent {
        host.session_titles.cancel_all();
        host.memory.cancel_where(|_| true);
        crate::speech_host::cancel_owner(app, "space");
    }
    crate::video_host::revoke_plugins(app, snapshot);
    crate::recording::revoke_plugins(app, snapshot);
    crate::recording_audio_host::reconcile(app, snapshot);
    crate::speech_host::revoke_plugins(app, snapshot);
    crate::code_host::revoke_plugins(app, snapshot);
    host.memory.cancel_where(|origin| {
        !snapshot.plugins.iter().any(|plugin| {
            plugin.manifest.id == origin.plugin_id
                && plugin.revision == origin.plugin_revision
                && plugin.state == PluginState::Enabled
                && plugin.error.is_none()
        })
    });
    crate::scroll_host::revoke_plugins(app, snapshot);
    crate::pin_host::revoke_plugins(app, snapshot);
    let mut engine = host.lock()?;
    let settled = settle_engine(&mut engine, snapshot);
    host.memory.wake.notify_one();
    crate::publish(app, &engine.store.snapshot());
    settled
}

/// The mutation has already committed. Even an unreadable catalog must revoke
/// cached origins; callers may not return early before this safety boundary.
fn finish_mutation(
    snapshot: Result<PluginSnapshot, String>,
    mut settle: impl FnMut(&PluginSnapshot) -> Result<(), String>,
    publish: impl FnOnce(&PluginSnapshot),
) -> Result<PluginSnapshot, String> {
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = settle(&PluginSnapshot {
                revision: 0,
                plugins: vec![],
            });
            return Err(error);
        }
    };
    let settled = settle(&snapshot);
    publish(&snapshot);
    settled?;
    Ok(snapshot)
}
#[tauri::command]
pub fn get_plugins(host: tauri::State<'_, Host>) -> Result<PluginSnapshot, String> {
    lock(&host)?.store.snapshot()
}

#[tauri::command]
pub fn get_official_plugins() -> Vec<PluginManifest> { plugins::bundled_manifests() }

async fn mutate(
    app: AppHandle,
    action: impl FnOnce(&mut PluginRuntime) -> Result<(), String> + Send + 'static,
) -> Result<PluginSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let mut runtime = lock(&host)?;
        host.exit.ensure_running()?;
        action(&mut runtime)?;
        finish_mutation(
            runtime.store.snapshot(),
            |snapshot| settle_runs(&app, &host, snapshot),
            |snapshot| publish(&app, snapshot),
        )
    })
    .await
    .map_err(|_| "插件操作中断".to_string())?
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfficialControl { Enable, Disable, Remove, Restore }
fn control_official_store(store: &mut PluginStore, id: &str, expected_revision: u64, action: OfficialControl) -> Result<(), String> {
        if !plugins::bundled_manifests().iter().any(|manifest| manifest.id == id) {
            return Err("未找到此官方模块".into());
        }
        let record = store.list()?.into_iter().find(|record| record.manifest.id == id).ok_or("未找到此官方模块")?;
        if record.source != PluginSource::Official { return Err("官方模块来源无效".into()); }
        match action {
            OfficialControl::Enable => { store.set_enabled(id, expected_revision, true)?; }
            OfficialControl::Disable => { store.set_enabled(id, expected_revision, false)?; }
            OfficialControl::Remove => { store.uninstall(id, expected_revision)?; }
            OfficialControl::Restore => { store.install_official(id, Some(expected_revision))?; }
        }
        Ok(())
}
#[tauri::command]
pub async fn control_official_plugin(app: AppHandle, id: String, expected_revision: u64, action: OfficialControl) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| control_official_store(&mut runtime.store, &id, expected_revision, action)).await
}

#[tauri::command]
pub async fn apply_plugin_drawing(
    app: AppHandle,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    command: SceneCommand,
) -> Result<u64, String> {
    // Drawing validation, durable storage and snapshot serialization must not
    // block the native window thread or an async runtime worker.
    tauri::async_runtime::spawn_blocking(move || {
        apply_plugin_drawing_blocking(app, plugin_id, plugin_revision, contribution_id, command)
    })
    .await
    .map_err(|_| "绘制更新中断")?
}

fn apply_plugin_drawing_blocking(
    app: AppHandle,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    command: SceneCommand,
) -> Result<u64, String> {
    let host = app.state::<Host>();
    crate::recording::ensure_idle(&app)?;
    let runtime = lock(&host)?;
    let tools = runtime
        .store
        .drawing_tools(&plugin_id, plugin_revision, &contribution_id)?;
    use mewu_core::DrawingKind as K;
    use plugins::DrawingTool as T;
    let tool = match &command {
        SceneCommand::AddDrawing { drawing, .. } | SceneCommand::UpdateDrawing { drawing, .. } => {
            Some(match drawing.kind {
                K::Pen => T::Pen,
                K::Line => T::Line,
                K::Arrow => T::Arrow,
                K::Rect => T::Rect,
                K::Ellipse => T::Ellipse,
                K::Text => T::Text,
                K::Highlighter => T::Highlighter,
                K::Mark => T::Mark,
                K::Number => T::Number,
                K::Mosaic => T::Mosaic,
                K::Rich => return Err("富图层只能从文档入口编辑".into()),
            })
        }
        SceneCommand::RemoveDrawing { .. }
        | SceneCommand::RemoveDrawings { .. }
        | SceneCommand::EraseDrawingStrokes { .. }
        | SceneCommand::UndoDrawing { .. }
        | SceneCommand::RedoDrawing { .. } => None,
        _ => return Err("此入口仅接受绘制操作".into()),
    };
    if tool.is_some_and(|tool| !tools.contains(&tool)) {
        return Err("此插件不提供该绘制工具".into());
    }
    let mut engine = host.lock()?;
    host.exit.ensure_scene_command(&command)?;
    crate::optional_capabilities::authoring_command(&engine, &command)?;
    crate::recording::ensure_idle(&app)?;
    let snapshot = engine.store.apply(command).map_err(|e| e.to_string())?;
    engine.ocr_jobs.reconcile(&snapshot);
    engine.translation_jobs.reconcile(&snapshot);
    Ok(crate::publish_owned(&app, snapshot).revision)
}

#[tauri::command]
pub async fn run_plugin_workflow(
    app: AppHandle,
    host: tauri::State<'_, Host>,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    scene_id: String,
    region_id: String,
) -> Result<HostSnapshot, String> {
    crate::recording::ensure_idle(&app)?;
    let (workflow, original, preflight) = {
        let runtime = lock(&host)?;
        let workflow = runtime
            .store
            .workflow(&plugin_id, plugin_revision, &contribution_id)?;
        let engine = host.lock()?;
        let preflight = crate::preflight_run(&host, &engine, &scene_id)?;
        let scene = engine
            .store
            .snapshot()
            .scenes
            .into_iter()
            .find(|s| s.id == scene_id)
            .ok_or("场景不存在")?;
        (workflow, scene, preflight)
    };
    let preflight = crate::resolve_run_credentials(host.root.clone(), preflight).await?;
    let mut scene = original.clone();
    // The user's unsent references are unrelated to this explicit selection workflow.
    scene.refs = vec![mewu_core::Reference {
        kind: mewu_core::ReferenceKind::Region,
        id: region_id.clone(),
    }];
    crate::ai::validate_scene_for_send(&scene)?;
    let attachments = crate::prepare_scene_attachments(scene, host.assets.clone()).await?;
    crate::recording::ensure_idle(&app)?;
    let runtime = lock(&host)?;
    runtime
        .store
        .workflow(&plugin_id, plugin_revision, &contribution_id)?;
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    crate::recording::ensure_idle(&app)?;
    if engine
        .store
        .snapshot()
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        != Some(&original)
        || engine
            .store
            .connection_for_scene(&scene_id)
            .map_err(|e| e.to_string())?
            != preflight.connection
    {
        return Err("会话或连接已变更，请重试".into());
    }
    crate::optional_capabilities::require_agent_current(&engine, &preflight.agent_grant)?;
    let mut context = engine
        .store
        .begin_workflow(&scene_id, &workflow.prompt, &region_id)
        .map_err(|e| e.to_string())?;
    context.agent.memory_enabled = false;
    context.memories.clear();
    context.mcp_servers.clear();
    engine.plugin_runs.insert(
        context.run_id.clone(),
        (scene_id, plugin_id, plugin_revision),
    );
    crate::start_run(&app, &mut engine, context, preflight, attachments)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn active_engine(
        store: mewu_core::Store,
        plugin: &plugins::PluginRecord,
    ) -> (
        crate::Engine,
        String,
        String,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let mut engine = crate::Engine {
            store,
            optional: crate::optional_capabilities::Authorizations::test_defaults(),
            agent_runs: HashMap::new(),
            runs: HashMap::new(),
            plugin_runs: HashMap::new(),
            visual_runs: HashMap::new(),
            ocr_jobs: crate::ocr_host::OcrRegistry::default(),
            translation_jobs: crate::translation_host::TranslationRegistry::default(),
        };
        let scene = engine.store.snapshot().active_scene_id;
        engine
            .store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "故障测试".into(),
            })
            .unwrap();
        let run = engine.store.begin_run(&scene).unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        engine.runs.insert(run.run_id.clone(), send);
        engine.agent_runs.insert(run.run_id.clone(), (scene.clone(), engine.optional.require_agent().unwrap().into()));
        engine.plugin_runs.insert(
            run.run_id.clone(),
            (scene.clone(), plugin.manifest.id.clone(), plugin.revision),
        );
        (engine, scene, run.run_id, receive)
    }
    fn remove_test_root(root: &Path) {
        let resolved = root.canonicalize().unwrap();
        assert_eq!(
            resolved.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mewu-proposals-"));
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn agent_disable_revokes_plain_runs_and_keeps_history() {
        let root = root();
        let mut runtime = PluginRuntime::open(&root).unwrap();
        let agent = runtime.store.list().unwrap().into_iter().find(|record| record.manifest.id == "mewu.agent").unwrap();
        let (mut engine, scene, run, mut receiver) = active_engine(mewu_core::Store::open_in_memory().unwrap(), &agent);
        engine.plugin_runs.clear(); // Ordinary sends have no workflow origin.
        let before = engine.store.snapshot().scenes.into_iter().find(|value| value.id == scene).unwrap();
        assert!(crate::run_is_authorized(&engine, &scene, &run, None));
        runtime.store.set_enabled(&agent.manifest.id, agent.revision, false).unwrap();
        settle_engine(&mut engine, &runtime.store.snapshot().unwrap()).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), ());
        assert!(!crate::run_is_authorized(&engine, &scene, &run, None));
        assert!(engine.agent_runs.is_empty());
        let after = engine.store.snapshot().scenes.into_iter().find(|value| value.id == scene).unwrap();
        assert_eq!(after.messages.len(), before.messages.len());
        assert_eq!(after.background, before.background);
        assert_eq!(after.items, before.items);
        assert!(!engine.store.run_is_active(&scene, &run));
        drop(engine); drop(runtime); remove_test_root(&root);
    }
    #[test]
    fn committed_disable_still_revokes_when_another_registry_row_breaks_snapshot() {
        let root = root();
        let mut runtime = PluginRuntime::open(&root).unwrap();
        let plugin = runtime.store.list().unwrap().into_iter().next().unwrap();
        let (mut engine, scene, run, mut receiver) =
            active_engine(mewu_core::Store::open_in_memory().unwrap(), &plugin);
        let changed = runtime
            .store
            .set_enabled(&plugin.manifest.id, plugin.revision, false)
            .unwrap();
        assert_eq!(changed.state, PluginState::Disabled);
        // Inject a real catalog-read failure after the target mutation committed.
        let db = rusqlite::Connection::open(root.join("plugins/plugins.db")).unwrap();
        assert!(
            db.execute(
                "UPDATE registry SET payload='not json' WHERE id<>?1",
                [&plugin.manifest.id]
            )
            .unwrap()
                > 0
        );
        let catalog = runtime.store.snapshot();
        let expected_error = catalog.as_ref().unwrap_err().clone();
        let result = finish_mutation(
            catalog,
            |snapshot| settle_engine(&mut engine, snapshot),
            |_| panic!("unreadable catalog must not be published"),
        );
        assert_eq!(result.unwrap_err(), expected_error);
        assert_eq!(receiver.try_recv().unwrap(), ());
        assert!(engine.plugin_runs.is_empty());
        assert!(engine.runs.is_empty());
        assert!(!engine.store.run_is_active(&scene, &run));
        let stored: String = db
            .query_row(
                "SELECT payload FROM registry WHERE id=?1",
                [&plugin.manifest.id],
                |r| r.get(0),
            )
            .unwrap();
        let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored["state"], "disabled");
        assert_eq!(stored["revision"], changed.revision);
        drop(db);
        drop(runtime);
        drop(engine);
        remove_test_root(&root);
    }
    #[test]
    fn scene_write_failure_cannot_keep_revoked_origin_or_transport_alive() {
        let root = root();
        let mut runtime = PluginRuntime::open(&root).unwrap();
        let plugin = runtime.store.list().unwrap().into_iter().next().unwrap();
        let (mut engine, scene, run, mut receiver) = active_engine(
            mewu_core::Store::open(root.join("scenes.db")).unwrap(),
            &plugin,
        );
        let origin = engine.plugin_runs[&run].clone();
        runtime
            .store
            .set_enabled(&plugin.manifest.id, plugin.revision, false)
            .unwrap();
        let db = rusqlite::Connection::open(root.join("scenes.db")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_cancel BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'injected scene write failure'); END;").unwrap();
        let mut published = false;
        let result = finish_mutation(
            runtime.store.snapshot(),
            |snapshot| settle_engine(&mut engine, snapshot),
            |_| published = true,
        );
        assert!(result.is_err());
        assert!(published); // Plugin state is already durable despite scene failure.
        assert_eq!(receiver.try_recv().unwrap(), ());
        assert!(engine.plugin_runs.is_empty());
        assert!(engine.runs.is_empty());
        assert!(engine.store.run_is_active(&scene, &run));
        assert!(!crate::run_is_authorized(
            &engine,
            &scene,
            &run,
            Some(&origin)
        ));
        drop(db);
        drop(runtime);
        drop(engine);
        remove_test_root(&root);
    }
    #[test]
    fn revocation_filter_keeps_only_exact_enabled_and_intact_revisions() {
        let manifest = plugins::bundled_manifests().into_iter().next().unwrap();
        let record = plugins::PluginRecord {
            manifest,
            source: PluginSource::Official,
            revision: 4,
            state: PluginState::Enabled,
            has_rollback: false,
            error: None,
        };
        let origins = HashMap::from([(
            "run".into(),
            ("scene".into(), record.manifest.id.clone(), 4),
        )]);
        let mut snapshot = PluginSnapshot {
            revision: 1,
            plugins: vec![record.clone()],
        };
        assert!(revoked_runs(&origins, &snapshot).is_empty());
        for broken in [
            plugins::PluginRecord {
                revision: 5,
                ..record.clone()
            },
            plugins::PluginRecord {
                state: PluginState::Disabled,
                ..record.clone()
            },
            plugins::PluginRecord {
                state: PluginState::Removed,
                ..record.clone()
            },
            plugins::PluginRecord {
                error: Some("package missing".into()),
                ..record.clone()
            },
        ] {
            snapshot.plugins = vec![broken];
            assert_eq!(
                revoked_runs(&origins, &snapshot),
                vec![("run".into(), "scene".into())]
            );
        }
        snapshot.plugins.clear();
        assert_eq!(revoked_runs(&origins, &snapshot).len(), 1);
    }
    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("mewu-proposals-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        root
    }
    #[test]
    fn retired_declarations_keep_their_data_but_never_appear_or_execute() {
        let root = root();
        let before;
        let bytes;
        {
            let mut runtime = PluginRuntime::open(&root).unwrap();
            let mut manifest = plugins::parse_manifest(include_bytes!("../../plugins/official-table.json")).unwrap();
            manifest.id = "example.retired".into();
            bytes = serde_json::to_vec(&manifest).unwrap();
            let file = root.join("retired.json"); std::fs::write(&file, &bytes).unwrap();
            let record = runtime.store.install_bytes(&bytes, PluginSource::Local { path: file.to_string_lossy().into_owned() }, None).unwrap();
            before = serde_json::to_vec(&record).unwrap();
            assert!(!runtime.store.snapshot().unwrap().plugins.iter().any(|record| record.manifest.id == manifest.id));
            let error = runtime.store.workflow(&manifest.id, record.revision, "recognize-table").unwrap_err();
            assert!(error.contains("停止支持"));
        }
        let runtime = PluginRuntime::open(&root).unwrap();
        let retained = runtime.store.list().unwrap().into_iter().find(|record| record.manifest.id == "example.retired").unwrap();
        assert_eq!(serde_json::to_vec(&retained).unwrap(), before);
        assert_eq!(std::fs::read(root.join("retired.json")).unwrap(), bytes);
        assert!(!runtime.store.snapshot().unwrap().plugins.iter().any(|record| record.manifest.id == "example.retired"));
        drop(runtime); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn official_controls_keep_saved_choices_and_cas_without_a_package_installer() {
        let root = root(); let mut store = PluginStore::open(&root).unwrap();
        assert!(control_official_store(&mut store, "example.retired", 1, OfficialControl::Enable).is_err());
        control_official_store(&mut store, "mewu.agent", 1, OfficialControl::Disable).unwrap();
        assert!(store.require_agent().is_err());
        assert!(control_official_store(&mut store, "mewu.agent", 1, OfficialControl::Enable).is_err());
        control_official_store(&mut store, "mewu.agent", 2, OfficialControl::Remove).unwrap();
        drop(store); let mut store = PluginStore::open(&root).unwrap();
        assert!(store.require_agent().is_err());
        control_official_store(&mut store, "mewu.agent", 3, OfficialControl::Restore).unwrap();
        assert!(store.require_agent().is_ok());
        assert!(serde_json::from_str::<OfficialControl>("\"rollback\"").is_err());
        drop(store); std::fs::remove_dir_all(root).unwrap();
    }
}
