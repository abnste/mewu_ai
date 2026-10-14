// SPDX-License-Identifier: MPL-2.0
//! Pure API 1 authorization projection shared by desktop and catalog validator.
//! No UI, Engine, filesystem, network, or credential access belongs here.
use crate::plugins::{PluginContribution, PluginSnapshot, PluginSource, PluginState};
use serde::Serialize;

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
        let mut value = Self {
            revision: snapshot.revision,
            ..Self::default()
        };
        // An enabled community declaration may choose the same fixed host
        // implementation; an intact official declaration is preferred.
        for official in [true, false] {
            for record in &snapshot.plugins {
                if record.state != PluginState::Enabled
                    || record.error.is_some()
                    || (record.source == PluginSource::Official) != official
                {
                    continue;
                }
                for contribution in &record.manifest.contributions {
                    let (slot, id) = match contribution {
                        PluginContribution::AgentRuntime { id, .. } => (&mut value.agent, id),
                        PluginContribution::Blackboard { id, .. } => (&mut value.blackboard, id),
                        _ => continue,
                    };
                    if slot.is_none() {
                        *slot = Some(Grant {
                            plugin_id: record.manifest.id.clone(),
                            revision: record.revision,
                            contribution_id: id.clone(),
                        });
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
        let snapshot = PluginSnapshot {
            revision: 1,
            plugins: manifests
                .into_iter()
                .map(|manifest| crate::plugins::PluginRecord {
                    manifest,
                    source: PluginSource::Official,
                    revision: 1,
                    state: PluginState::Enabled,
                    has_rollback: false,
                    error: None,
                })
                .collect(),
        };
        Self::from_snapshot(&snapshot)
    }
}
