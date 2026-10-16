// SPDX-License-Identifier: MPL-2.0
export type PluginPlatform = 'any' | 'windows-x86_64' | 'darwin-aarch64' | 'linux-x86_64';
export interface ExtensionManifest {
  schemaVersion: 2; hostApi: 2; runtime: 'javascript'; id: string; version: string;
  name: string; description: string; publisher?: string; license: string; entry: string;
  permissions: string[]; tags: string[]; dependencies: Record<string, string>;
  platforms?: PluginPlatform[];
}
export interface ExtensionCatalogEntry {
  id: string; name: string; description: string; tags: string[]; repository: string;
  version: string; releaseTag: string; assetName: string; sha256: string; size: number; hostApi: 2;
  platforms: PluginPlatform[]; manifestPath?: 'mewu-plugin.json';
}
export interface ExtensionCatalog { schemaVersion: 2; plugins: ExtensionCatalogEntry[] }
export interface ExtensionProjectScreenshot { path: string; sha256: string; size: number; width: number; height: number; caption?: string }
export interface ExtensionProject {
  id: string; version: string; packageSha256: string; sourceRepository: string; sourceCommit: string;
  screenshots: ExtensionProjectScreenshot[];
}
export type ExtensionSource = { type: 'local'; path: string } | { type: 'github'; entry: ExtensionCatalogEntry };
export type ExtensionServiceKind = 'command' | 'tool' | 'agent' | 'memory' | 'model' | 'channel' | 'renderer' | 'ui' | 'service';
export interface ExtensionService { kind: ExtensionServiceKind; id: string; title: string; description?: string; inputSchema?: unknown; metadata?: unknown }
export interface ExtensionRecord { manifest: ExtensionManifest; revision: number; state: 'enabled' | 'disabled' | 'removed'; source: ExtensionSource; sha256: string; services: ExtensionService[]; hasRollback: boolean; platformError?: string; unsupportedServices?: Record<string, string> }
export interface ExtensionProposal { id: string; manifest: ExtensionManifest; sha256: string; source: ExtensionSource; expectedRevision?: number }
export interface ExtensionInvocation { pluginId: string; revision: number; serviceId: string; input: unknown; sceneId?: string; regionId?: string }
export interface ExtensionBinding { agentId: string; kind: 'agent' | 'memory'; pluginId: string | null; serviceId: string | null; revision: number }
export type ExtensionControl = 'enable' | 'disable' | 'remove' | 'rollback';
