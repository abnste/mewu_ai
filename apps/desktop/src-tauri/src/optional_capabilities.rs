// SPDX-License-Identifier: MPL-2.0
//! Optional product capabilities are authorized by the host, independently of
//! WebView visibility. This is metadata only; no plugin receives credentials.
pub(crate) use crate::optional_authorizations::{Authorizations, Grant};
use crate::{Engine, Host};
use mewu_core::{ConnectionAuthMode, ConnectionProfile, SceneCommand};
use serde::Serialize;
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager};

pub(crate) fn require_agent_current(engine: &Engine, grant: &Grant) -> Result<(), String> {
    if engine.optional.agent.as_ref() != Some(grant) {
        return Err("Agent 插件已变更".into());
    }
    Ok(())
}

pub(crate) fn scene_authoring(engine: &Engine, scene_id: &str) -> Result<Option<Grant>, String> {
    let scene = engine
        .store
        .scene(scene_id)
        .map_err(|error| error.to_string())?;
    if is_blackboard(scene) {
        Ok(Some(engine.optional.require_blackboard()?))
    } else {
        Ok(None)
    }
}
pub(crate) fn is_blackboard(scene: &mewu_core::Scene) -> bool {
    scene.blackboard_link.is_some()
        || scene.background.as_ref().is_some_and(|asset| {
            asset.kind == mewu_core::AssetKind::Image
                && asset.name == "黑板.png"
                && asset
                    .path
                    .replace('\\', "/")
                    .ends_with(&format!("/{}.board.png", asset.id))
        })
}
pub(crate) fn scene_authoring_current(
    engine: &Engine,
    scene_id: &str,
    grant: &Option<Grant>,
) -> Result<(), String> {
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
        EditImageAnnotations { scene_id, .. }
        | AddRegion { scene_id, .. }
        | UpdateRegion { scene_id, .. }
        | SetRegionGeometry { scene_id, .. }
        | UndoRegionGeometry { scene_id, .. }
        | RedoRegionGeometry { scene_id, .. }
        | RemoveRegion { scene_id, .. }
        | AddDrawing { scene_id, .. }
        | UpdateDrawing { scene_id, .. }
        | UpdateDrawings { scene_id, .. }
        | EraseDrawingStrokes { scene_id, .. }
        | RemoveDrawing { scene_id, .. }
        | UndoDrawing { scene_id, .. }
        | RedoDrawing { scene_id, .. }
        | UpdateItem { scene_id, .. }
        | RemoveItem { scene_id, .. } => Some(scene_id),
        // Soul/role selection and manually edited local facts are user data,
        // independent of which executable Agent provider the user has enabled.
        // Core still validates identity, context compatibility and memory CAS.
        SaveAgent { .. } | SetAgent { .. } | SaveMemory { .. } | DeleteMemory { .. } => None,
        SetMcpGrants { .. } | SetMcpEnabled { enabled: true, .. } => {
            engine.optional.require_agent()?;
            None
        }
        // Disabling/removing tools and completing a pending geometry gesture
        // must remain possible after revocation.
        _ => None,
    };
    if let Some(scene) = scene {
        scene_authoring(engine, scene)?;
    }
    Ok(())
}

/// UI eligibility is a configuration projection, not proof that a credential can
/// currently be unlocked. Merely selecting a scene must not read a secret or
/// prompt the desktop keyring. Actual model calls still read and validate it.
pub(crate) fn connection_configured(profile: &ConnectionProfile) -> bool {
    if profile.base_url.trim().is_empty()
        || profile.model.trim().is_empty()
        || mewu_core::validate_connection_profile(profile).is_err()
        || crate::connection_host::validate_request_options(profile).is_err()
        || crate::connection_host::transport_for(profile).is_err()
    {
        return false;
    }
    if profile.advanced.auth_mode == ConnectionAuthMode::None {
        return true;
    }
    profile.has_key && profile.credential_id.is_some()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Capabilities {
    pub agent: Option<Grant>,
    pub blackboard: Option<Grant>,
    pub plugin_revision: u64,
    pub connection_revisions: BTreeMap<String, u64>,
    /// Internal desktop UI metadata; execution still performs credential IO.
    pub configured_connection_ids: Vec<String>,
    pub ready_agent_ids: Vec<String>,
    pub bound_agent_ids: Vec<String>,
}
fn external_agent_ready(
    record: &crate::extension_store::Record,
    service_id: &str,
    dependencies_valid: bool,
) -> bool {
    // A declared capability is an upper bound, not a requirement that every
    // invocation use it. The selected handler decides whether it needs a model;
    // model.invoke enforces actual connection/key authority at the call boundary.
    record.state == "enabled"
        && record.platform_error.is_none()
        && !record.unsupported_services.contains_key(service_id)
        && dependencies_valid
        && record.services.iter().any(|service| {
            service.id == service_id && service.kind == mewu_extension::ServiceKind::Agent
        })
}
#[tauri::command]
pub(crate) async fn get_optional_capabilities(app: AppHandle) -> Result<Capabilities, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        host.exit.ensure_running()?;
        let (connections, optional, agent_ids) = {
            let engine = host.lock()?;
            let snapshot = engine.store.snapshot();
            let agent_ids = snapshot
                .agents
                .into_iter()
                .map(|agent| agent.id)
                .collect::<Vec<_>>();
            (
                engine.store.connection_profiles().to_vec(),
                engine.optional.clone(),
                agent_ids,
            )
        };
        // Pure metadata only. A failed/deferred keychain unlock must neither
        // hide Composer nor be triggered by every active-scene change.
        let configured_connection_ids: Vec<String> = connections
            .iter()
            .filter(|profile| connection_configured(profile))
            .map(|profile| profile.id.clone())
            .collect();
        let (ready_agent_ids, bound_agent_ids) = {
            let extensions = app.state::<crate::extension_host::ExtensionHost>();
            let store = extensions.store()?;
            let mut ids = vec![];
            let mut bound = vec![];
            for agent_id in agent_ids {
                let Some(binding) = store.binding(&agent_id, "agent")? else {
                    continue;
                };
                let (Some(plugin), Some(service)) = (binding.plugin_id, binding.service_id) else {
                    continue;
                };
                bound.push(agent_id.clone());
                let Ok(record) = store.get(&plugin) else {
                    continue;
                };
                if external_agent_ready(
                    &record,
                    &service,
                    store.validate_dependencies(&record.manifest).is_ok(),
                ) {
                    ids.push(agent_id);
                }
            }
            (ids, bound)
        };
        Ok(Capabilities {
            agent: optional.agent,
            blackboard: optional.blackboard,
            plugin_revision: optional.revision,
            connection_revisions: connections
                .iter()
                .map(|profile| (profile.id.clone(), profile.revision))
                .collect(),
            configured_connection_ids,
            ready_agent_ids,
            bound_agent_ids,
        })
    })
    .await
    .map_err(|_| "功能状态读取中断".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginSnapshot, PluginSource, PluginState};
    fn profile(auth_mode: ConnectionAuthMode) -> ConnectionProfile {
        serde_json::from_value(serde_json::json!({
            "id":"19172393-857b-471d-a182-19d555e653fc", "name":"test",
            "providerId":"custom", "revision":1, "baseUrl":"https://example.com/v1",
            "model":"test", "hasKey":false, "advanced":{"protocol":"chat_completions","authMode":auth_mode,"requestParameters":{}}
        })).unwrap()
    }
    #[test]
    fn declared_model_permission_does_not_require_a_host_key_for_external_agent_admission() {
        let manifest:mewu_extension::ExtensionManifest=serde_json::from_value(serde_json::json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"community.independent-agent","version":"1.0.0","name":"Independent","description":"","license":"MPL-2.0","entry":"index.js","permissions":["model.invoke","network.fetch"]})).unwrap();
        let mut record = crate::extension_store::Record {
            manifest,
            revision: 1,
            state: "enabled".into(),
            source: crate::extension_store::Source::Local {
                path: "plugin.zip".into(),
            },
            sha256: "0".repeat(64),
            services: vec![mewu_extension::ServiceRegistration {
                kind: mewu_extension::ServiceKind::Agent,
                id: "agent".into(),
                title: "Agent".into(),
                description: None,
                input_schema: None,
                metadata: None,
            }],
            has_rollback: false,
            platform_error: None,
            unsupported_services: Default::default(),
        };
        // No configured key: direct host model use remains unavailable, while an
        // independent handler may choose its network/service/local fallback.
        assert!(!connection_configured(&profile(ConnectionAuthMode::Bearer)));
        assert!(external_agent_ready(&record, "agent", true));
        assert!(!external_agent_ready(&record, "agent", false));
        assert!(!external_agent_ready(&record, "not-registered", true));
        record.state = "disabled".into();
        assert!(!external_agent_ready(&record, "agent", true));
    }
    #[test]
    fn local_soul_and_memory_configuration_remain_usable_without_a_legacy_agent_grant() {
        let mut engine = crate::Engine {
            store: mewu_core::Store::open_in_memory().unwrap(),
            optional: Default::default(),
            agent_runs: Default::default(),
            runs: Default::default(),
            plugin_runs: Default::default(),
            visual_runs: Default::default(),
            ocr_jobs: Default::default(),
            translation_jobs: Default::default(),
        };
        let snapshot = engine.store.snapshot();
        let scene_id = snapshot.active_scene_id;
        let mut agent = snapshot.agents[0].clone();
        agent.id = uuid::Uuid::new_v4().to_string();
        agent.name = "Independent Agent role".into();
        agent.instructions = "A user-defined soul".into();
        let save = SceneCommand::SaveAgent {
            agent: agent.clone(),
        };
        authoring_command(&engine, &save).unwrap();
        engine.store.apply(save).unwrap();
        let choose = SceneCommand::SetAgent {
            scene_id: scene_id.clone(),
            agent_id: agent.id.clone(),
        };
        authoring_command(&engine, &choose).unwrap();
        engine.store.apply(choose).unwrap();
        assert_eq!(engine.store.scene(&scene_id).unwrap().agent_id, agent.id);
        assert_eq!(
            engine.store.agent_profile(&agent.id).unwrap().instructions,
            "A user-defined soul"
        );
        let save = SceneCommand::SaveMemory {
            agent_id: agent.id.clone(),
            id: None,
            text: "A manually edited local fact".into(),
            expected_revision: None,
        };
        authoring_command(&engine, &save).unwrap();
        engine.store.apply(save).unwrap();
        let page = engine.store.memory_page(&agent.id, "", None, 10).unwrap();
        assert_eq!(page.total, 1);
        let entry = &page.entries[0];
        let stale = SceneCommand::DeleteMemory {
            agent_id: agent.id.clone(),
            id: entry.id.clone(),
            expected_revision: entry.revision + 1,
        };
        authoring_command(&engine, &stale).unwrap();
        assert!(engine.store.apply(stale).is_err());
        let delete = SceneCommand::DeleteMemory {
            agent_id: agent.id.clone(),
            id: entry.id.clone(),
            expected_revision: entry.revision,
        };
        authoring_command(&engine, &delete).unwrap();
        engine.store.apply(delete).unwrap();
        assert_eq!(
            engine
                .store
                .memory_page(&agent.id, "", None, 10)
                .unwrap()
                .total,
            0
        );
        assert!(engine.optional.require_agent().is_err());
        for command in [
            SceneCommand::SetMcpGrants {
                server_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 1,
                agent_id: agent.id.clone(),
                tool_names: vec!["click".into()],
            },
            SceneCommand::SetMcpEnabled {
                server_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 1,
                enabled: true,
            },
        ] {
            assert!(authoring_command(&engine, &command).is_err());
        }
    }
    #[test]
    fn initial_empty_connection_has_no_ai_and_explicit_no_auth_needs_no_secret() {
        let mut value = profile(ConnectionAuthMode::None);
        assert!(connection_configured(&value));
        value.model.clear();
        assert!(!connection_configured(&value));
        value.model = "test".into();
        value.base_url.clear();
        assert!(!connection_configured(&value));
    }
    #[test]
    fn authenticated_ui_configuration_requires_valid_claims_not_an_unlocked_secret() {
        for mode in [ConnectionAuthMode::Bearer, ConnectionAuthMode::ApiKey] {
            let mut value = profile(mode);
            assert!(!connection_configured(&value));
            value.has_key = true;
            assert!(!connection_configured(&value));
            value.credential_id = Some("922cff56-b629-4dcb-91eb-3059de4be4b6".into());
            assert!(connection_configured(&value));
            value.credential_id = Some("invalid-reference".into());
            assert!(!connection_configured(&value));
            value.credential_id = Some("922cff56-b629-4dcb-91eb-3059de4be4b6".into());
            value.has_key = false;
            assert!(!connection_configured(&value));
        }
    }
    #[test]
    fn configured_ui_rejects_invalid_transport_and_request_options_without_io() {
        let mut value = profile(ConnectionAuthMode::None);
        value.advanced.request_path = Some("../outside".into());
        assert!(!connection_configured(&value));
        value.advanced.request_path = None;
        value.base_url = "https://user:password@example.com".into();
        assert!(!connection_configured(&value));
        value.base_url = "https://example.com/v1".into();
        value.model = "invalid model".into();
        assert!(!connection_configured(&value));
    }
    #[cfg(windows)]
    #[test]
    fn configured_ui_without_a_blob_stays_visible_but_actual_credential_read_fails() {
        let root =
            std::env::temp_dir().join(format!("mewu-optional-capability-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut value = profile(ConnectionAuthMode::Bearer);
        value.has_key = true;
        value.credential_id = Some("922cff56-b629-4dcb-91eb-3059de4be4b6".into());
        // No root or reader is passed to the UI projection. It cannot open a
        // scoped blob or prompt a platform keyring, even when none is usable.
        assert!(connection_configured(&value));
        // This is the same production reader called by main::preflight_run and
        // actual model requests, not an injected success or a network request.
        assert!(crate::credentials::read_profile(&root, &value).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn exact_optional_grants_change_with_disable_revision_and_integrity_failure() {
        let manifest = crate::plugins::bundled_manifests()
            .into_iter()
            .find(|m| m.id == "mewu.agent")
            .unwrap();
        let mut snapshot = PluginSnapshot {
            revision: 1,
            plugins: vec![crate::plugins::PluginRecord {
                manifest,
                source: PluginSource::Official,
                revision: 1,
                state: PluginState::Enabled,
                has_rollback: false,
                error: None,
            }],
        };
        let first = Authorizations::from_snapshot(&snapshot).agent.unwrap();
        snapshot.plugins[0].revision = 2;
        assert_ne!(
            Authorizations::from_snapshot(&snapshot).agent.as_ref(),
            Some(&first)
        );
        snapshot.plugins[0].state = PluginState::Disabled;
        assert!(Authorizations::from_snapshot(&snapshot).agent.is_none());
        snapshot.plugins[0].state = PluginState::Enabled;
        snapshot.plugins[0].error = Some("synthetic integrity failure".into());
        assert!(Authorizations::from_snapshot(&snapshot).agent.is_none());
    }
    #[test]
    fn blackboard_authoring_revokes_without_disabling_core_capture_or_deleting_documents() {
        let mut engine = crate::Engine {
            store: mewu_core::Store::open_in_memory().unwrap(),
            optional: Authorizations::test_defaults(),
            agent_runs: Default::default(),
            runs: Default::default(),
            plugin_runs: Default::default(),
            visual_runs: Default::default(),
            ocr_jobs: Default::default(),
            translation_jobs: Default::default(),
        };
        let parent = engine.store.snapshot().active_scene_id;
        assert!(scene_authoring(&engine, &parent).unwrap().is_none());
        let id = uuid::Uuid::new_v4().to_string();
        let asset = mewu_core::Asset {
            id: id.clone(),
            name: "黑板.png".into(),
            kind: mewu_core::AssetKind::Image,
            path: std::env::temp_dir()
                .join(format!("{id}.board.png"))
                .to_string_lossy()
                .into_owned(),
            width: Some(64),
            height: Some(64),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
            capture_coordinates: None,
        };
        let snapshot = engine
            .store
            .create_blackboard_document(&parent, asset.clone())
            .unwrap();
        let board = snapshot
            .scenes
            .iter()
            .find(|scene| scene.id == snapshot.active_scene_id)
            .unwrap()
            .clone();
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
        assert!(authoring_command(
            &engine,
            &SceneCommand::UndoDrawing {
                scene_id: board.id.clone(),
                region_id: board.regions[0].id.clone(),
                background_id: asset.id.clone(),
                expected_revision: 0
            }
        )
        .is_err());
        assert_eq!(engine.store.snapshot(), snapshot);
        // Finish receives no new authored content. Its exact persisted source
        // can still be saved and returned to the parent after revocation.
        let preview_id = uuid::Uuid::new_v4().to_string();
        let preview = mewu_core::Asset {
            id: preview_id.clone(),
            path: std::env::temp_dir()
                .join(format!("{preview_id}.png"))
                .to_string_lossy()
                .into_owned(),
            ..asset
        };
        let finished = engine
            .store
            .finish_blackboard_document(&board, preview)
            .unwrap();
        assert_eq!(finished.active_scene_id, parent);
        assert!(finished
            .scenes
            .iter()
            .any(|scene| scene.id == board.id && scene.regions == board.regions));
    }
}
