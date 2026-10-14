// SPDX-License-Identifier: MPL-2.0
//! In-memory revocation fences for external Agent runs. No Engine-held code
//! takes the extension SQLite lock; controllers revoke the same atomic lease.
use crate::{optional_capabilities::Grant, Engine};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct ExternalAgentAuthority {
    pub agent_id: String,
    pub plugin_id: String,
    pub service_id: String,
    pub plugin_revision: u64,
    pub binding_revision: u64,
    pub sha256: String,
    pub canceled: Arc<AtomicBool>,
}
#[derive(Debug, Clone)]
pub(crate) enum RunAgentAuthority {
    Core(Grant),
    Extension(ExternalAgentAuthority),
}
impl From<Grant> for RunAgentAuthority {
    fn from(value: Grant) -> Self {
        Self::Core(value)
    }
}
impl From<ExternalAgentAuthority> for RunAgentAuthority {
    fn from(value: ExternalAgentAuthority) -> Self {
        Self::Extension(value)
    }
}
impl RunAgentAuthority {
    pub(crate) fn is_current(&self, engine: &Engine, scene_id: &str) -> bool {
        match self {
            Self::Core(grant) => engine.optional.agent.as_ref() == Some(grant),
            Self::Extension(authority) => {
                !authority.canceled.load(Ordering::SeqCst)
                    && engine.store.scene(scene_id).is_ok_and(|scene| {
                        scene.agent_id == authority.agent_id
                            && !scene.closed
                            && scene
                                .run
                                .as_ref()
                                .and_then(|run| run.external_runtime.as_ref())
                                .is_some_and(|origin| {
                                    origin.plugin_id == authority.plugin_id
                                        && origin.service_id == authority.service_id
                                        && origin.revision == authority.plugin_revision
                                        && origin.sha256 == authority.sha256
                                })
                    })
            }
        }
    }
}
struct Lease {
    plugin: String,
    agent: String,
    dependencies: Vec<String>,
    canceled: Weak<AtomicBool>,
}
fn leases() -> &'static Mutex<HashMap<String, Lease>> {
    static LEASES: OnceLock<Mutex<HashMap<String, Lease>>> = OnceLock::new();
    LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(crate) fn register(
    authority: &ExternalAgentAuthority,
    dependencies: Vec<String>,
) -> Result<(), String> {
    let mut leases = leases().lock().map_err(|_| "Agent 扩展授权不可用")?;
    leases.retain(|_, lease| lease.canceled.strong_count() > 0);
    if leases.len() >= 128 {
        return Err("待运行的 Agent 扩展过多".into());
    }
    leases.insert(
        uuid::Uuid::new_v4().to_string(),
        Lease {
            plugin: authority.plugin_id.clone(),
            agent: authority.agent_id.clone(),
            dependencies,
            canceled: Arc::downgrade(&authority.canceled),
        },
    );
    Ok(())
}
fn cancel(predicate: impl Fn(&Lease) -> bool) {
    if let Ok(mut leases) = leases().lock() {
        leases.retain(|_, lease| {
            let Some(signal) = lease.canceled.upgrade() else {
                return false;
            };
            if predicate(lease) {
                signal.store(true, Ordering::SeqCst);
            }
            true
        });
    }
}
pub(crate) fn cancel_all() {
    cancel(|_| true)
}
pub(crate) fn cancel_plugin(id: &str) {
    cancel(|lease| {
        lease.plugin == id || lease.dependencies.iter().any(|dependency| dependency == id)
    })
}
pub(crate) fn cancel_agent(id: &str) {
    cancel(|lease| lease.agent == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authority() -> ExternalAgentAuthority {
        ExternalAgentAuthority {
            agent_id: uuid::Uuid::new_v4().to_string(),
            plugin_id: format!("community.{}", uuid::Uuid::new_v4()),
            service_id: "agent".into(),
            plugin_revision: 1,
            binding_revision: 1,
            sha256: "0".repeat(64),
            canceled: Arc::new(AtomicBool::new(false)),
        }
    }
    #[test]
    fn plugin_and_binding_changes_revoke_exact_live_agent_lease() {
        let first = authority();
        let other = authority();
        register(&first, vec![]).unwrap();
        register(&other, vec![]).unwrap();
        cancel_plugin(&first.plugin_id);
        assert!(first.canceled.load(Ordering::SeqCst));
        assert!(!other.canceled.load(Ordering::SeqCst));
        cancel_agent(&other.agent_id);
        assert!(other.canceled.load(Ordering::SeqCst));
    }
    #[test]
    fn dependency_removal_revokes_its_callers() {
        let first = authority();
        let dependency = format!("dep.{}", uuid::Uuid::new_v4());
        register(&first, vec![dependency.clone()]).unwrap();
        cancel_plugin(&dependency);
        assert!(first.canceled.load(Ordering::SeqCst));
    }
    #[test]
    fn external_agent_admission_works_without_legacy_agent_or_model_and_revokes_late_completion() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let snapshot = store.snapshot();
        let scene_id = snapshot.active_scene_id;
        let mut lease = authority();
        lease.agent_id = store.scene(&scene_id).unwrap().agent_id.clone();
        store
            .apply(mewu_core::SceneCommand::SetDraft {
                scene_id: scene_id.clone(),
                draft: "external agent task".into(),
            })
            .unwrap();
        let context = store
            .begin_external_run(
                &scene_id,
                mewu_core::ExternalRunOrigin {
                    plugin_id: lease.plugin_id.clone(),
                    service_id: lease.service_id.clone(),
                    revision: lease.plugin_revision,
                    sha256: lease.sha256.clone(),
                },
            )
            .unwrap();
        let mut engine = crate::Engine {
            store,
            optional: Default::default(),
            agent_runs: Default::default(),
            runs: Default::default(),
            plugin_runs: Default::default(),
            visual_runs: Default::default(),
            ocr_jobs: Default::default(),
            translation_jobs: Default::default(),
        };
        let (sender, _receiver) = tokio::sync::oneshot::channel();
        engine.runs.insert(context.run_id.clone(), sender);
        engine.agent_runs.insert(
            context.run_id.clone(),
            (scene_id.clone(), lease.clone().into()),
        );
        register(&lease, vec![]).unwrap();
        assert!(engine.optional.agent.is_none());
        assert!(crate::run_is_authorized(
            &engine,
            &scene_id,
            &context.run_id,
            None
        ));
        cancel_plugin(&lease.plugin_id);
        assert!(engine.store.run_is_active(&scene_id, &context.run_id));
        assert!(!crate::run_is_authorized(
            &engine,
            &scene_id,
            &context.run_id,
            None
        ));
    }
}
