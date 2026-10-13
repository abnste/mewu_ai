// SPDX-License-Identifier: MPL-2.0
//! Optional product capabilities are authorized by the host, independently of
//! WebView visibility. This is metadata only; no plugin receives credentials.
use crate::{plugins::{PluginContribution, PluginSnapshot, PluginSource, PluginState}, Engine, Host};
use mewu_core::{ConnectionAuthMode, ConnectionProfile, SceneCommand};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Grant {
    pub plugin_id: String,
    pub revision: u64,
    pub contribution_id: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Authorizations {
    pub revision: u64,
    pub agent: Option<Grant>,
    pub blackboard: Option<Grant>,
}
impl Authorizations {
    pub fn from_snapshot(snapshot: &PluginSnapshot) -> Self {
        let mut value = Self { revision: snapshot.revision, ..Self::default() };
        // An enabled community declaration may choose the same fixed host
        // implementation; an intact official declaration is preferred.
        for official in [true, false] {
            for record in &snapshot.plugins {
                if record.state != PluginState::Enabled || record.error.is_some()
                    || (record.source == PluginSource::Official) != official { continue; }
                for contribution in &record.manifest.contributions {
                    let (slot, id) = match contribution {
                        PluginContribution::AgentRuntime { id, .. } => (&mut value.agent, id),
                        PluginContribution::Blackboard { id, .. } => (&mut value.blackboard, id),
                        _ => continue,
                    };
                    if slot.is_none() {
                        *slot = Some(Grant { plugin_id: record.manifest.id.clone(), revision: record.revision, contribution_id: id.clone() });
                    }
                }
            }
        }
        value
    }
    pub fn require_agent(&self) -> Result<Grant, String> {
        self.agent.clone().ok_or("Agent 插件未启用".into())
    }
    pub fn require_blackboard(&self) -> Result<Grant, String> {
        self.blackboard.clone().ok_or("电子黑板插件未启用".into())
    }
    #[cfg(test)]
    pub fn test_defaults() -> Self {
        let manifests = crate::plugins::bundled_manifests();
        let snapshot = PluginSnapshot { revision: 1, plugins: manifests.into_iter().map(|manifest| crate::plugins::PluginRecord {
            manifest, source: PluginSource::Official, revision: 1, state: PluginState::Enabled, has_rollback: false, error: None,
        }).collect() };
        Self::from_snapshot(&snapshot)
    }
}

pub(crate) fn require_agent_current(engine: &Engine, grant: &Grant) -> Result<(), String> {
    if engine.optional.agent.as_ref() != Some(grant) {
        return Err("Agent 插件已变更".into());
    }
    Ok(())
}

pub(crate) fn scene_authoring(engine: &Engine, scene_id: &str) -> Result<Option<Grant>, String> {
    let scene = engine.store.scene(scene_id).map_err(|error| error.to_string())?;
    if is_blackboard(scene) {
        Ok(Some(engine.optional.require_blackboard()?))
    } else {
        Ok(None)
    }
}
pub(crate) fn is_blackboard(scene: &mewu_core::Scene) -> bool {
    scene.blackboard_link.is_some() || scene.background.as_ref().is_some_and(|asset|
        asset.kind == mewu_core::AssetKind::Image && asset.name == "黑板.png"
            && asset.path.replace('\\', "/").ends_with(&format!("/{}.board.png", asset.id)))
}
pub(crate) fn scene_authoring_current(engine: &Engine, scene_id: &str, grant: &Option<Grant>) -> Result<(), String> {
    if &scene_authoring(engine, scene_id)? != grant {
        return Err("电子黑板插件已变更".into());
    }
    Ok(())
}

/// Capture documents keep their core editing capabilities. A backing blackboard
/// needs its optional authoring grant; view, copy, close and finish stay usable.
pub(crate) fn authoring_command(engine: &Engine, command: &SceneCommand) -> Result<(), String> {
    use SceneCommand::*;
    let scene = match command {
        AddRegion { scene_id, .. } | UpdateRegion { scene_id, .. }
        | SetRegionGeometry { scene_id, .. } | UndoRegionGeometry { scene_id, .. }
        | RedoRegionGeometry { scene_id, .. } | RemoveRegion { scene_id, .. }
        | AddDrawing { scene_id, .. } | UpdateDrawing { scene_id, .. }
        | UpdateDrawings { scene_id, .. } | RemoveDrawing { scene_id, .. }
        | UndoDrawing { scene_id, .. } | RedoDrawing { scene_id, .. }
        | UpdateItem { scene_id, .. } | RemoveItem { scene_id, .. } => Some(scene_id),
        SaveAgent { .. } | SetAgent { .. } | SaveMemory { .. } | DeleteMemory { .. }
        | SetMcpGrants { .. } | SetMcpEnabled { enabled: true, .. } => {
            engine.optional.require_agent()?;
            None
        }
        // Disabling/removing tools and completing a pending geometry gesture
        // must remain possible after revocation.
        _ => None,
    };
    if let Some(scene) = scene { scene_authoring(engine, scene)?; }
    Ok(())
}

pub(crate) fn connection_ready(root: &Path, profile: &ConnectionProfile) -> bool {
    connection_ready_with(profile, || crate::credentials::read_profile(root, profile))
}
fn connection_ready_with(profile: &ConnectionProfile, key: impl FnOnce() -> Result<Zeroizing<String>, String>) -> bool {
    if profile.base_url.trim().is_empty() || profile.model.trim().is_empty()
        || mewu_core::validate_connection_profile(profile).is_err()
        || crate::connection_host::validate_request_options(profile).is_err()
        || crate::connection_host::transport_for(profile).is_err() { return false; }
    if profile.advanced.auth_mode == ConnectionAuthMode::None { return true; }
    if !profile.has_key || profile.credential_id.is_none() { return false; }
    key().is_ok_and(|value| !value.trim().is_empty() && crate::credentials::validate_key(&value).is_ok())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Capabilities {
    pub agent: Option<Grant>,
    pub blackboard: Option<Grant>,
    pub plugin_revision: u64,
    pub connection_revisions: BTreeMap<String, u64>,
    pub ready_connection_ids: Vec<String>,
}
#[tauri::command]
pub(crate) async fn get_optional_capabilities(app: AppHandle) -> Result<Capabilities, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        host.exit.ensure_running()?;
        let (connections, optional) = {
            let engine = host.lock()?;
            (engine.store.connection_profiles().to_vec(), engine.optional.clone())
        };
        // Secret Service can wait on the desktop keyring. Never hold the scene
        // store while checking credentials for settings or optional UI.
        let ready_connection_ids = connections.iter().filter(|profile| connection_ready(&host.root, profile)).map(|profile| profile.id.clone()).collect();
        Ok(Capabilities {
            agent: optional.agent,
            blackboard: optional.blackboard,
            plugin_revision: optional.revision,
            connection_revisions: connections.iter().map(|profile| (profile.id.clone(), profile.revision)).collect(),
            ready_connection_ids,
        })
    }).await.map_err(|_| "功能状态读取中断".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(auth_mode: ConnectionAuthMode) -> ConnectionProfile {
        serde_json::from_value(serde_json::json!({
            "id":"19172393-857b-471d-a182-19d555e653fc", "name":"test",
            "providerId":"custom", "revision":1, "baseUrl":"https://example.com/v1",
            "model":"test", "hasKey":false, "advanced":{"protocol":"chat_completions","authMode":auth_mode,"requestParameters":{}}
        })).unwrap()
    }
    #[test]
    fn initial_empty_connection_has_no_ai_and_explicit_no_auth_needs_no_secret() {
        let mut value = profile(ConnectionAuthMode::None);
        assert!(connection_ready_with(&value, || panic!("no-auth must not read credentials")));
        value.model.clear(); assert!(!connection_ready_with(&value, || panic!()));
        value.model = "test".into(); value.base_url.clear(); assert!(!connection_ready_with(&value, || panic!()));
    }
    #[test]
    fn authenticated_connections_require_real_nonempty_valid_credentials() {
        for mode in [ConnectionAuthMode::Bearer, ConnectionAuthMode::ApiKey] {
            let mut value = profile(mode);
            assert!(!connection_ready_with(&value, || panic!("a missing reference must not read secrets")));
            value.has_key=true; value.credential_id=Some("922cff56-b629-4dcb-91eb-3059de4be4b6".into());
            assert!(!connection_ready_with(&value, || Err("synthetic missing or decrypt failure".into())));
            for key in ["", " ", "bad\nheader"] {
                assert!(!connection_ready_with(&value, || Ok(Zeroizing::new(key.into()))));
            }
            assert!(connection_ready_with(&value, || Ok(Zeroizing::new("synthetic-only-key".into()))));
            assert!(!connection_ready_with(&value, || Ok(Zeroizing::new("x".repeat(16 * 1024 + 1)))));
        }
    }
    #[test]
    fn exact_optional_grants_change_with_disable_revision_and_integrity_failure() {
        let manifest = crate::plugins::bundled_manifests().into_iter().find(|m| m.id == "mewu.agent").unwrap();
        let mut snapshot = PluginSnapshot { revision: 1, plugins: vec![crate::plugins::PluginRecord {
            manifest, source: PluginSource::Official, revision: 1, state: PluginState::Enabled, has_rollback: false, error: None,
        }] };
        let first = Authorizations::from_snapshot(&snapshot).agent.unwrap();
        snapshot.plugins[0].revision=2; assert_ne!(Authorizations::from_snapshot(&snapshot).agent.as_ref(),Some(&first));
        snapshot.plugins[0].state=PluginState::Disabled; assert!(Authorizations::from_snapshot(&snapshot).agent.is_none());
        snapshot.plugins[0].state=PluginState::Enabled; snapshot.plugins[0].error=Some("synthetic integrity failure".into());
        assert!(Authorizations::from_snapshot(&snapshot).agent.is_none());
    }
    #[test]
    fn blackboard_authoring_revokes_without_disabling_core_capture_or_deleting_documents() {
        let mut engine = crate::Engine {
            store: mewu_core::Store::open_in_memory().unwrap(),
            optional: Authorizations::test_defaults(),
            agent_runs: Default::default(), runs: Default::default(), plugin_runs: Default::default(),
            visual_runs: Default::default(), ocr_jobs: Default::default(), translation_jobs: Default::default(),
        };
        let parent = engine.store.snapshot().active_scene_id;
        assert!(scene_authoring(&engine, &parent).unwrap().is_none());
        let id = uuid::Uuid::new_v4().to_string();
        let asset = mewu_core::Asset {
            id: id.clone(), name: "黑板.png".into(), kind: mewu_core::AssetKind::Image,
            path: std::env::temp_dir().join(format!("{id}.board.png")).to_string_lossy().into_owned(),
            width: Some(64), height: Some(64), origin_x: None, origin_y: None, scale_factor: None, capture_coordinates: None,
        };
        let snapshot = engine.store.create_blackboard_document(&parent, asset.clone()).unwrap();
        let board = snapshot.scenes.iter().find(|scene| scene.id == snapshot.active_scene_id).unwrap().clone();
        let accepted = scene_authoring(&engine, &board.id).unwrap();
        assert!(accepted.is_some());
        let agent_grant = engine.optional.require_agent().unwrap();
        engine.optional.agent.as_mut().unwrap().revision += 1;
        assert!(require_agent_current(&engine, &agent_grant).is_err());
        engine.optional.agent = None;
        assert!(require_agent_current(&engine, &agent_grant).is_err());
        engine.optional.blackboard = None;
        assert!(scene_authoring_current(&engine, &board.id, &accepted).is_err());
        assert!(scene_authoring(&engine, &parent).unwrap().is_none());
        assert!(authoring_command(&engine, &SceneCommand::UndoDrawing {scene_id:board.id.clone(),region_id:board.regions[0].id.clone(),background_id:asset.id.clone(),expected_revision:0}).is_err());
        assert_eq!(engine.store.snapshot(), snapshot);
        // Finish receives no new authored content. Its exact persisted source
        // can still be saved and returned to the parent after revocation.
        let preview_id = uuid::Uuid::new_v4().to_string();
        let preview = mewu_core::Asset { id:preview_id.clone(), path:std::env::temp_dir().join(format!("{preview_id}.png")).to_string_lossy().into_owned(), ..asset };
        let finished = engine.store.finish_blackboard_document(&board, preview).unwrap();
        assert_eq!(finished.active_scene_id, parent);
        assert!(finished.scenes.iter().any(|scene| scene.id == board.id && scene.regions == board.regions));
    }
}
