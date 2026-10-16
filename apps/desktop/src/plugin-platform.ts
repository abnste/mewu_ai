// SPDX-License-Identifier: MPL-2.0
import type { PluginPlatform, ExtensionCatalogEntry, ExtensionManifest, ExtensionRecord, ExtensionSource } from './extension-contracts';
import type { PluginManifest } from './plugin-contracts';
export type HostPlatform = 'windows' | 'macos' | 'linux' | 'other';
const targets = { windows: 'windows-x86_64', macos: 'darwin-aarch64', linux: 'linux-x86_64' } as const;
export function pluginPlatformError(platforms: PluginPlatform[] | undefined, platform?: HostPlatform): string | undefined {
  if (!platform) return '正在读取系统信息';
  if (platform === 'other') return '此插件不支持当前系统';
  if (platforms === undefined) return;
  if (!Array.isArray(platforms) || !platforms.length || platforms.length > 3 || new Set(platforms).size !== platforms.length
    || platforms.some(value => value !== 'any' && !Object.values(targets).includes(value as typeof targets[keyof typeof targets]))
    || (platforms.includes('any') && platforms.length !== 1)
    || !platforms.some(value => value === 'any' || value === targets[platform])) return '此插件不支持当前系统';
}
export function extensionPlatformError(manifest: ExtensionManifest | ExtensionCatalogEntry, source: ExtensionSource | undefined, platform?: HostPlatform): string | undefined {
  return pluginPlatformError(manifest.platforms, platform)
    || (source?.type === 'github' ? pluginPlatformError(source.entry.platforms, platform) : undefined);
}
export function installedExtensionError(record: ExtensionRecord, platform?: HostPlatform): string | undefined {
  return record.platformError || extensionPlatformError(record.manifest, record.source, platform);
}
export function declarativePluginError(manifest: PluginManifest, platform?: HostPlatform): string | undefined {
  const error = pluginPlatformError(manifest.platforms, platform); if (error) return error;
  for (const value of manifest.contributions) {
    if (value.kind === 'selection.ocr' && ({ windows: 'windows', 'apple.vision': 'macos', 'linux.tesseract': 'linux' } as const)[value.engine] !== platform)
      return '此插件的本机能力不支持当前系统';
    if (value.kind === 'input.speech-to-text' && platform !== 'windows') return '此插件的本机能力不支持当前系统';
    if (value.kind === 'selection.codes' && platform === 'linux') return '此插件的本机能力不支持当前系统';
  }
}
