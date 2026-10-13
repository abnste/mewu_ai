// SPDX-License-Identifier: MPL-2.0
import type { ConnectionProfile } from './contracts';
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
}

export function optionalCapabilityKey(plugins: PluginSnapshot, connections: ConnectionProfile[]): string {
  return JSON.stringify([plugins.revision, connections.map(value => [value.id, value.revision]).sort((a, b) => String(a[0]).localeCompare(String(b[0])))]);
}

export function currentOptionalCapabilities(value: OptionalCapabilities, plugins: PluginSnapshot, connections: ConnectionProfile[]): boolean {
  const grantShape = (grant: OptionalGrant | null) => grant === null || Boolean(grant && typeof grant.pluginId === 'string' && Number.isSafeInteger(grant.revision) && grant.revision > 0 && typeof grant.contributionId === 'string');
  if (!value || !Number.isSafeInteger(value.pluginRevision) || !value.connectionRevisions || typeof value.connectionRevisions !== 'object' || Array.isArray(value.connectionRevisions) || !Array.isArray(value.readyConnectionIds) || !grantShape(value.agent) || !grantShape(value.blackboard)) return false;
  if (value.pluginRevision !== plugins.revision || Object.keys(value.connectionRevisions).length !== connections.length) return false;
  if (!connections.every(connection => value.connectionRevisions[connection.id] === connection.revision)) return false;
  if (!value.readyConnectionIds.every(id => connections.some(connection => connection.id === id))) return false;
  return ([['agent.runtime', value.agent], ['space.blackboard', value.blackboard]] as const).every(([kind, grant]) => !grant || plugins.plugins.some(plugin => plugin.manifest.id === grant.pluginId && plugin.revision === grant.revision && plugin.state === 'enabled' && !plugin.error && plugin.manifest.contributions.some(contribution => contribution.id === grant.contributionId && contribution.kind === kind)));
}
