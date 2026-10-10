// SPDX-License-Identifier: MPL-2.0
use crate::{
    plugin_sources::{self, Catalog, CatalogEntry},
    plugins::{self, PluginManifest, PluginSnapshot, PluginSource, PluginState, PluginStore},
    Host, HostSnapshot,
};
use mewu_core::SceneCommand;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProposal {
    id: String,
    manifest: PluginManifest,
    source: PluginSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_revision: Option<u64>,
    sha256: String,
}
struct Pending {
    proposal: PluginProposal,
    bytes: Vec<u8>,
    created: Instant,
}
pub struct PluginRuntime {
    pub store: PluginStore,
    proposals: HashMap<String, Pending>,
    catalog_generation: u64,
}
impl PluginRuntime {
    pub fn open(root: &Path) -> Result<Self, String> {
        Ok(Self {
            store: PluginStore::open(root)?,
            proposals: HashMap::new(),
            catalog_generation: 0,
        })
    }
    fn prepare(
        &mut self,
        bytes: Vec<u8>,
        source: PluginSource,
        expected: Option<(String, u64)>,
    ) -> Result<PluginProposal, String> {
        let manifest = plugins::parse_manifest(&bytes)?;
        if manifest.id.starts_with("mewu.") {
            return Err("第三方包不能替换官方插件".into());
        }
        let installed = self
            .store
            .list()?
            .into_iter()
            .find(|p| p.manifest.id == manifest.id);
        if let Some((id, revision)) = expected {
            if manifest.id != id || installed.as_ref().is_none_or(|p| p.revision != revision) {
                return Err("插件已变更，请重新读取".into());
            }
            if installed
                .as_ref()
                .is_some_and(|p| p.manifest.version == manifest.version)
            {
                return Err("已是当前版本".into());
            }
        }
        self.proposals
            .retain(|_, p| p.created.elapsed() < Duration::from_secs(600));
        if self.proposals.len() >= 16 {
            return Err("待安装插件过多，请稍后重新读取".into());
        }
        let proposal = PluginProposal {
            id: uuid::Uuid::new_v4().to_string(),
            expected_revision: installed.map(|p| p.revision),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            manifest,
            source,
        };
        self.proposals.insert(
            proposal.id.clone(),
            Pending {
                proposal: proposal.clone(),
                bytes,
                created: Instant::now(),
            },
        );
        Ok(proposal)
    }
}
fn lock(host: &Host) -> Result<std::sync::MutexGuard<'_, PluginRuntime>, String> {
    host.plugins
        .lock()
        .map_err(|_| "插件存储需要重新启动".into())
}
fn publish(app: &AppHandle, snapshot: &PluginSnapshot) {
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
    engine.ocr_jobs.revoke_plugins(snapshot);
    engine.translation_jobs.revoke_plugins(snapshot);
    let mut ended = revoked_runs(&engine.plugin_runs, snapshot);
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
    let mut failure = None;
    for (run, scene) in ended {
        if let Some(sender) = engine.runs.remove(&run) {
            let _ = sender.send(());
        }
        engine.plugin_runs.remove(&run);
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
pub async fn get_plugin_catalog(
    app: AppHandle,
    source_url: Option<String>,
) -> Result<Catalog, String> {
    let (source, generation) = {
        let host = app.state::<Host>();
        let mut runtime = lock(&host)?;
        runtime.catalog_generation = runtime.catalog_generation.wrapping_add(1);
        (
            source_url.or(runtime.store.get_catalog_source()?),
            runtime.catalog_generation,
        )
    };
    let mut entries: Vec<_> = plugins::bundled_manifests()
        .into_iter()
        .map(|manifest| CatalogEntry {
            manifest,
            source: PluginSource::Official,
            installed: None,
        })
        .collect();
    let source = source.filter(|s| !s.trim().is_empty());
    let mut error = None;
    if let Some(url) = &source {
        let result = async {
            let parsed = plugin_sources::parse_url(url, "mewu-catalog.json")?;
            let (bytes, _) = plugin_sources::download(parsed, 2 * 1024 * 1024).await?;
            plugin_sources::parse_catalog(&bytes)
        }
        .await;
        match result {
            Ok(remote) => entries.extend(remote),
            Err(e) => error = Some(e),
        }
    }
    let host = app.state::<Host>();
    let mut runtime = lock(&host)?;
    host.exit.ensure_running()?;
    if runtime.catalog_generation == generation && error.is_none() {
        runtime.store.set_catalog_source(source.as_deref())?;
    }
    let installed = runtime.store.list()?;
    for entry in &mut entries {
        entry.installed = installed
            .iter()
            .find(|p| p.manifest.id == entry.manifest.id)
            .cloned();
    }
    Ok(Catalog {
        entries,
        source,
        error,
    })
}

#[tauri::command]
pub async fn prepare_plugin(
    app: AppHandle,
    window: tauri::WebviewWindow,
    url: Option<String>,
) -> Result<Option<PluginProposal>, String> {
    let (bytes, source) = if let Some(url) = url {
        plugin_sources::download(
            plugin_sources::parse_url(&url, "mewu-plugin.json")?,
            plugins::MAX_MANIFEST_BYTES,
        )
        .await?
    } else {
        let result = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
            let Some(path) = rfd::FileDialog::new()
                .set_parent(&window)
                .add_filter("Mewu 插件", &["json"])
                .pick_file()
            else {
                return Ok(None);
            };
            let bytes = plugins::read_validated_local(&path)?;
            let path = path
                .canonicalize()
                .map_err(|_| "无法读取插件路径")?
                .to_string_lossy()
                .into_owned();
            Ok(Some((bytes, PluginSource::Local { path })))
        })
        .await
        .map_err(|_| "读取插件中断")??;
        let Some(result) = result else {
            return Ok(None);
        };
        result
    };
    let host = app.state::<Host>();
    let result = lock(&host)?.prepare(bytes, source, None)?;
    Ok(Some(result))
}

#[tauri::command]
pub async fn prepare_plugin_update(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<PluginProposal, String> {
    let record = {
        let host = app.state::<Host>();
        let runtime = lock(&host)?;
        runtime
            .store
            .list()?
            .into_iter()
            .find(|p| {
                p.manifest.id == id
                    && p.revision == expected_revision
                    && p.state != PluginState::Removed
            })
            .ok_or("插件已变更，请重新读取")?
    };
    let (bytes, source) = match record.source {
        PluginSource::Github {
            repository, path, ..
        } => {
            plugin_sources::download(
                plugin_sources::GithubFile {
                    repository,
                    reference: "HEAD".into(),
                    path,
                },
                plugins::MAX_MANIFEST_BYTES,
            )
            .await?
        }
        PluginSource::Local { path } => tauri::async_runtime::spawn_blocking(move || {
            let bytes = plugins::read_validated_local(Path::new(&path))?;
            Ok::<_, String>((bytes, PluginSource::Local { path }))
        })
        .await
        .map_err(|_| "读取插件中断")??,
        PluginSource::Official => return Err("官方插件随 Mewu 更新".into()),
    };
    let host = app.state::<Host>();
    let result = lock(&host)?.prepare(bytes, source, Some((id, expected_revision)))?;
    Ok(result)
}

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
#[tauri::command]
pub async fn install_plugin(app: AppHandle, proposal_id: String) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| {
        let pending = runtime
            .proposals
            .remove(&proposal_id)
            .ok_or("安装预览已失效，请重新读取")?;
        if pending.created.elapsed() >= Duration::from_secs(600) {
            return Err("安装预览已过期，请重新读取".into());
        }
        runtime.store.install_bytes(
            &pending.bytes,
            pending.proposal.source,
            pending.proposal.expected_revision,
        )?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn set_plugin_enabled(
    app: AppHandle,
    id: String,
    expected_revision: u64,
    enabled: bool,
) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| {
        runtime.store.set_enabled(&id, expected_revision, enabled)?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn uninstall_plugin(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| {
        runtime.store.uninstall(&id, expected_revision)?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn rollback_plugin(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| {
        runtime.store.rollback(&id, expected_revision)?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn reinstall_official_plugin(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<PluginSnapshot, String> {
    mutate(app, move |runtime| {
        runtime
            .store
            .install_official(&id, Some(expected_revision))?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn apply_plugin_drawing(
    app: AppHandle,
    host: tauri::State<'_, Host>,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    command: SceneCommand,
) -> Result<HostSnapshot, String> {
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
                K::Number => T::Number,
                K::Mosaic => T::Mosaic,
                K::Rich => return Err("富图层只能从文档入口编辑".into()),
            })
        }
        SceneCommand::RemoveDrawing { .. }
        | SceneCommand::UndoDrawing { .. }
        | SceneCommand::RedoDrawing { .. } => None,
        _ => return Err("此入口仅接受绘制操作".into()),
    };
    if tool.is_some_and(|tool| !tools.contains(&tool)) {
        return Err("此插件不提供该绘制工具".into());
    }
    let mut engine = host.lock()?;
    host.exit.ensure_scene_command(&command)?;
    crate::recording::ensure_idle(&app)?;
    let snapshot = engine.store.apply(command).map_err(|e| e.to_string())?;
    engine.ocr_jobs.reconcile(&snapshot);
    engine.translation_jobs.reconcile(&snapshot);
    Ok(crate::publish(&app, &snapshot))
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
    fn reviewed_bytes_are_immutable_and_proposals_bind_revision() {
        let root = root();
        let mut runtime = PluginRuntime::open(&root).unwrap();
        let mut manifest =
            plugins::parse_manifest(include_bytes!("../../plugins/official-table.json")).unwrap();
        manifest.id = "example.table".into();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(root.join("example.json"), &bytes).unwrap();
        let proposal = runtime
            .prepare(
                bytes.clone(),
                PluginSource::Local {
                    path: root.join("example.json").to_string_lossy().into_owned(),
                },
                None,
            )
            .unwrap();
        assert_eq!(proposal.sha256, format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(proposal.expected_revision, None);
        assert_eq!(runtime.proposals.get(&proposal.id).unwrap().bytes, bytes);
        runtime
            .store
            .install_bytes(&bytes, proposal.source.clone(), None)
            .unwrap();
        let pending = runtime.proposals.remove(&proposal.id).unwrap();
        assert!(runtime
            .store
            .install_bytes(
                &pending.bytes,
                pending.proposal.source,
                pending.proposal.expected_revision
            )
            .is_err());
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}
