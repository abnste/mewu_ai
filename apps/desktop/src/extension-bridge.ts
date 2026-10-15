// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ExtensionBinding, ExtensionCatalog, ExtensionCatalogEntry, ExtensionControl, ExtensionInvocation, ExtensionProject, ExtensionProposal, ExtensionRecord } from './extension-contracts';

export const extensionNative = isTauri();
function desktop() { if (!extensionNative) throw new Error('请在桌面版管理插件'); }
export async function getExtensions(): Promise<ExtensionRecord[]> { desktop(); return invoke('get_extensions'); }
export async function getExtensionCatalog(): Promise<ExtensionCatalog> { desktop(); return invoke('get_extension_catalog'); }
export async function getExtensionProject(entry: ExtensionCatalogEntry): Promise<ExtensionProject | null> { desktop(); return invoke('get_extension_project', { entry }); }
export async function getExtensionProjectImage(entry: ExtensionCatalogEntry, index: number, expectedSourceCommit: string, expectedImageSha256: string): Promise<string> { desktop(); return invoke('get_extension_project_image', { entry, index, expectedSourceCommit, expectedImageSha256 }); }
export async function subscribeExtensions(accept: (records: ExtensionRecord[]) => void): Promise<() => void> {
  if (!extensionNative) return () => undefined;
  return listen<ExtensionRecord[]>('extension-state', event => accept(event.payload));
}
export async function prepareExtension(entry?: ExtensionCatalogEntry): Promise<ExtensionProposal | null> { desktop(); return invoke('prepare_extension', { entry: entry ?? null }); }
export async function installExtension(proposalId: string): Promise<ExtensionRecord[]> { desktop(); return invoke('install_extension', { proposalId }); }
export async function controlExtension(record: ExtensionRecord, action: ExtensionControl): Promise<ExtensionRecord[]> {
  desktop(); return invoke('control_extension', { pluginId: record.manifest.id, revision: record.revision, action });
}
export async function invokeExtension(invocation: ExtensionInvocation): Promise<unknown> { desktop(); return invoke('invoke_extension', { invocation }); }
export async function stopExtension(pluginId?: string): Promise<void> { desktop(); return invoke('stop_extension', { pluginId: pluginId ?? null }); }
export async function getExtensionBindings(): Promise<ExtensionBinding[]> { desktop(); return invoke('get_extension_bindings'); }
export async function subscribeExtensionBindings(accept: (bindings: ExtensionBinding[]) => void): Promise<() => void> {
  if (!extensionNative) return () => undefined;
  return listen<ExtensionBinding[]>('extension-bindings', event => accept(event.payload));
}
export async function setExtensionBinding(agentId: string, kind: 'agent' | 'memory', expectedRevision: number | undefined, choice?: { pluginId: string; serviceId: string }): Promise<ExtensionBinding[]> {
  desktop(); return invoke('set_extension_binding', { binding: { agentId, kind, pluginId: choice?.pluginId ?? null, serviceId: choice?.serviceId ?? null, expectedRevision: expectedRevision ?? null } });
}
