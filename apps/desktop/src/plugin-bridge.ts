// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PluginCatalog, PluginManifest, PluginProposal, PluginSnapshot } from './plugin-contracts';
import type { DrawingCommand, OcrTarget, Snapshot } from './contracts';
import { previewDrawingCommand } from './bridge';

export const pluginNative = isTauri();
const bundled = import.meta.glob('../plugins/official-*.json', { eager: true, import: 'default' }) as Record<string, PluginManifest>;
function desktop() { if (!pluginNative) throw new Error('请在桌面版管理插件'); }

export async function getPlugins(): Promise<PluginSnapshot> {
  return pluginNative ? invoke('get_plugins') : { revision: 0, plugins: [] };
}
export async function getPluginCatalog(sourceUrl?: string): Promise<PluginCatalog> {
  if (pluginNative) return invoke('get_plugin_catalog', { sourceUrl: sourceUrl ?? null });
  if (sourceUrl) throw new Error('请在桌面版读取插件目录');
  return { entries: Object.values(bundled).map(manifest => ({ manifest, source: { type: 'official' as const } })) };
}
export async function subscribePlugins(accept: (value: PluginSnapshot) => void): Promise<() => void> {
  if (!pluginNative) return () => undefined;
  return listen<PluginSnapshot>('plugin-state', event => accept(event.payload));
}
export async function preparePlugin(url?: string): Promise<PluginProposal | null> {
  desktop(); return invoke('prepare_plugin', { url: url ?? null });
}
export async function installPlugin(proposalId: string): Promise<PluginSnapshot> {
  desktop(); return invoke('install_plugin', { proposalId });
}
export async function setPluginEnabled(id: string, expectedRevision: number, enabled: boolean): Promise<PluginSnapshot> {
  desktop(); return invoke('set_plugin_enabled', { id, expectedRevision, enabled });
}
export async function uninstallPlugin(id: string, expectedRevision: number): Promise<PluginSnapshot> {
  desktop(); return invoke('uninstall_plugin', { id, expectedRevision });
}
export async function rollbackPlugin(id: string, expectedRevision: number): Promise<PluginSnapshot> {
  desktop(); return invoke('rollback_plugin', { id, expectedRevision });
}
export async function reinstallOfficialPlugin(id: string, expectedRevision: number): Promise<PluginSnapshot> {
  desktop(); return invoke('reinstall_official_plugin', { id, expectedRevision });
}
export async function preparePluginUpdate(id: string, expectedRevision: number): Promise<PluginProposal> {
  desktop(); return invoke('prepare_plugin_update', { id, expectedRevision });
}
export async function applyPluginDrawing(pluginId: string, pluginRevision: number, contributionId: string, command: DrawingCommand): Promise<Snapshot> {
  if(!pluginNative&&pluginId==='mewu.core.drawing'&&pluginRevision===1&&contributionId==='drawing-tools')return previewDrawingCommand(command);
  desktop(); return invoke('apply_plugin_drawing', { pluginId, pluginRevision, contributionId, command });
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
