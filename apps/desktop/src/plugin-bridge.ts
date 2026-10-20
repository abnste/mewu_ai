// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PluginManifest, PluginSnapshot } from './plugin-contracts';
import type { DrawingCommand, OcrTarget, Snapshot } from './contracts';
import { previewDrawingCommand } from './bridge';
import type { OptionalCapabilities } from './optional-capabilities';

export const pluginNative = isTauri();
const bundled = import.meta.glob('../plugins/official-*.json', { eager: true, import: 'default' }) as Record<string, PluginManifest>;
function desktop() { if (!pluginNative) throw new Error('请在桌面版管理插件'); }

export async function getPlugins(): Promise<PluginSnapshot> {
  return pluginNative ? invoke('get_plugins') : { revision: 0, plugins: [] };
}
export async function getOptionalCapabilities(): Promise<OptionalCapabilities> {
  if (!pluginNative) return { pluginRevision: 0, connectionRevisions: {}, agent: null, blackboard: null, configuredConnectionIds: [], readyAgentIds: [], boundAgentIds: [] };
  return invoke('get_optional_capabilities');
}
export async function getOfficialPlugins(): Promise<PluginManifest[]> {
  return pluginNative ? invoke('get_official_plugins') : Object.values(bundled);
}
export async function subscribePlugins(accept: (value: PluginSnapshot) => void, surface: 'space' | 'settings' = 'space'): Promise<() => void> {
  if (!pluginNative) return () => undefined;
  return listen<PluginSnapshot>('plugin-state', event => accept(event.payload), { target: surface });
}
export async function controlOfficialPlugin(id: string, expectedRevision: number, action: 'enable' | 'disable' | 'remove' | 'restore'): Promise<PluginSnapshot> {
  desktop(); return invoke('control_official_plugin', { id, expectedRevision, action });
}
export async function applyPluginDrawing(pluginId: string, pluginRevision: number, contributionId: string, command: DrawingCommand): Promise<number | Snapshot> {
  if(!pluginNative&&pluginId==='mewu.core.drawing'&&pluginRevision===1&&contributionId==='drawing-tools')return previewDrawingCommand(command);
  desktop(); return invoke<number>('apply_plugin_drawing', { pluginId, pluginRevision, contributionId, command });
}
export async function runPluginWorkflow(pluginId: string, pluginRevision: number, contributionId: string, sceneId: string, regionId: string): Promise<Snapshot> {
  desktop(); return invoke('run_plugin_workflow', { pluginId, pluginRevision, contributionId, sceneId, regionId });
}
export async function runPluginOcr(requestId: string, pluginId: string, revision: number, contributionId: string, target: OcrTarget): Promise<Snapshot> {
  if (!pluginNative) throw new Error('请在桌面版识别文字');
  return invoke('run_plugin_ocr', { requestId, pluginId, revision, contributionId, target });
}
export async function cancelPluginOcr(requestId: string): Promise<void> {
  if (pluginNative) await invoke('cancel_plugin_ocr', { requestId });
}
