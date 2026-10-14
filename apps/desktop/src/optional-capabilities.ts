// SPDX-License-Identifier: MPL-2.0
import type { AgentProfile, ConnectionProfile, Scene } from './contracts';
import type { PluginSnapshot } from './plugin-contracts';

export interface OptionalGrant { pluginId: string; revision: number; contributionId: string }
export function sameOptionalGrant(a: OptionalGrant | null | undefined, b: OptionalGrant | null | undefined): boolean {
  return Boolean(a && b && a.pluginId === b.pluginId && a.revision === b.revision && a.contributionId === b.contributionId);
}
export interface OptionalCapabilities {
  pluginRevision: number;
  connectionRevisions: Record<string, number>;
  agent: OptionalGrant | null;
  blackboard: OptionalGrant | null;
  readyConnectionIds: string[];
  readyAgentIds: string[];
  boundAgentIds: string[];
}

export interface OptionalAgentConfiguration {
  activeSceneId: string | null;
  activeScene?: Pick<Scene, 'id' | 'agentId' | 'connectionId'>;
  agents: Pick<AgentProfile, 'id' | 'defaultConnectionId'>[];
  defaultConnectionId: string | null;
}

export function optionalCapabilityKey(plugins: PluginSnapshot, connections: ConnectionProfile[], configuration?: OptionalAgentConfiguration): string {
  return JSON.stringify([plugins.revision, connections.map(value => [value.id, value.revision]).sort((a, b) => String(a[0]).localeCompare(String(b[0]))),
    configuration ? [configuration.activeSceneId, configuration.activeScene ? [configuration.activeScene.id, configuration.activeScene.agentId, configuration.activeScene.connectionId ?? null] : null,
      configuration.defaultConnectionId, configuration.agents.map(agent => [agent.id, agent.defaultConnectionId ?? null]).sort((a, b) => String(a[0]).localeCompare(String(b[0])))] : null]);
}

export function currentOptionalCapabilities(value: OptionalCapabilities, plugins: PluginSnapshot, connections: ConnectionProfile[], configuration?: OptionalAgentConfiguration): boolean {
  const grantShape = (grant: OptionalGrant | null) => grant === null || Boolean(grant && typeof grant.pluginId === 'string' && Number.isSafeInteger(grant.revision) && grant.revision > 0 && typeof grant.contributionId === 'string');
  const ids = (values: unknown): values is string[] => Array.isArray(values) && values.every(id => typeof id === 'string' && id.length > 0 && id.length <= 128) && new Set(values).size === values.length;
  if (!value || !Number.isSafeInteger(value.pluginRevision) || !value.connectionRevisions || typeof value.connectionRevisions !== 'object' || Array.isArray(value.connectionRevisions) || !ids(value.readyConnectionIds) || !ids(value.readyAgentIds) || !ids(value.boundAgentIds) || !grantShape(value.agent) || !grantShape(value.blackboard)) return false;
  if (value.pluginRevision !== plugins.revision || Object.keys(value.connectionRevisions).length !== connections.length) return false;
  if (!connections.every(connection => value.connectionRevisions[connection.id] === connection.revision)) return false;
  if (!value.readyConnectionIds.every(id => connections.some(connection => connection.id === id))) return false;
  if (!value.readyAgentIds.every(id => value.boundAgentIds.includes(id)) || (configuration && !value.boundAgentIds.every(id => configuration.agents.some(agent => agent.id === id)))) return false;
  return ([['agent.runtime', value.agent], ['space.blackboard', value.blackboard]] as const).every(([kind, grant]) => !grant || plugins.plugins.some(plugin => plugin.manifest.id === grant.pluginId && plugin.revision === grant.revision && plugin.state === 'enabled' && !plugin.error && plugin.manifest.contributions.some(contribution => contribution.id === grant.contributionId && contribution.kind === kind)));
}

export function optionalAgentReady(value: OptionalCapabilities | undefined, scene?: Pick<Scene, 'agentId' | 'connectionId'>): boolean {
  if (!value || !scene) return false;
  if (value.boundAgentIds.includes(scene.agentId)) return value.readyAgentIds.includes(scene.agentId);
  return Boolean(value.agent && scene.connectionId && value.readyConnectionIds.includes(scene.connectionId));
}
export function optionalAgentAvailable(value: OptionalCapabilities | undefined): boolean {
  return Boolean(value && (value.readyAgentIds.length || (value.agent && value.readyConnectionIds.length)));
}
