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
export async function listPluginProjects(): Promise<import('./extension-contracts').PluginDraft[]> { desktop(); return invoke('list_plugin_projects'); }
export async function preparePluginProject(projectId: string, expectedSha256: string): Promise<ExtensionProposal> { desktop(); return invoke('prepare_plugin_project', { projectId, expectedSha256 }); }
export async function subscribePluginProjects(accept: () => void): Promise<() => void> { if (!extensionNative) return () => undefined; return listen('plugin-projects-changed', accept); }
export async function installExtension(proposalId: string, sourceMigrationId?: string): Promise<ExtensionRecord[]> { desktop(); return invoke('install_extension', { proposalId, ...(sourceMigrationId ? { sourceMigrationId } : {}) }); }
export async function controlExtension(record: ExtensionRecord, action: ExtensionControl): Promise<ExtensionRecord[]> {
  desktop(); return invoke('control_extension', { pluginId: record.manifest.id, revision: record.revision, action });
}
export async function invokeExtension(invocation: ExtensionInvocation): Promise<unknown> { desktop(); return invoke('invoke_extension', { invocation }); }
export async function resolveExtensionAction(id: string, ok: boolean, reason?: string): Promise<void> { desktop(); return invoke('resolve_extension_action', { id, ok, reason: reason ?? null }); }
export async function openSurfaceWindow(surfaceId:string,x:number,y:number):Promise<void>{desktop();return invoke('open_extension_surface_window',{surfaceId,x,y});}
export async function releaseSurfaceWindow(surfaceId:string):Promise<void>{desktop();return invoke('release_extension_surface_window',{surfaceId});}
export async function requestCloseSurfaceWindow(surfaceId:string):Promise<void>{desktop();return invoke('request_close_extension_surface_window',{surfaceId});}
export async function subscribeSurfaceClosed(accept:(id:string)=>void):Promise<()=>void>{return listen<string>('extension-surface-closed',e=>accept(e.payload),{target:'space'});}
export async function subscribeSurfaceTargets(accept:(value:{surfaceId:string;targets:Array<{id:string;x:number;y:number;width:number;height:number}>})=>void):Promise<()=>void>{
  return listen('extension-surface-targets',e=>accept(e.payload as Parameters<typeof accept>[0]),{target:'space'});
}
export async function forwardSurfaceEvent(surfaceId:string,name:string,value:unknown):Promise<void>{desktop();return invoke('forward_extension_surface_event',{surfaceId,name,value});}
export async function stopExtension(pluginId?: string): Promise<void> { desktop(); return invoke('stop_extension', { pluginId: pluginId ?? null }); }
export async function getExtensionBindings(): Promise<ExtensionBinding[]> { desktop(); return invoke('get_extension_bindings'); }
export async function subscribeExtensionBindings(accept: (bindings: ExtensionBinding[]) => void): Promise<() => void> {
  if (!extensionNative) return () => undefined;
  return listen<ExtensionBinding[]>('extension-bindings', event => accept(event.payload));
}
export async function setExtensionBinding(agentId: string, kind: 'agent' | 'memory', expectedRevision: number | undefined, choice?: { pluginId: string; serviceId: string }): Promise<ExtensionBinding[]> {
  desktop(); return invoke('set_extension_binding', { binding: { agentId, kind, pluginId: choice?.pluginId ?? null, serviceId: choice?.serviceId ?? null, expectedRevision: expectedRevision ?? null } });
}

export async function executeExtensionAction(id: string): Promise<boolean> { desktop(); return invoke('execute_extension_action', { id }); }
