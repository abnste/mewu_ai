// SPDX-License-Identifier: MPL-2.0
import { compareVersions, validateStrict } from 'compare-versions';
import type { ExtensionCatalogEntry, ExtensionRecord, ExtensionSource } from './extension-contracts';

export function sameExtensionOrigin(a: ExtensionSource, b: ExtensionSource): boolean {
  return a.type === 'github' && b.type === 'github' ? a.entry.repository.toLowerCase() === b.entry.repository.toLowerCase()
    : a.type === 'local' && b.type === 'local' && a.path === b.path;
}
export function extensionUpdateEntry(installed: ExtensionRecord, entries: ExtensionCatalogEntry[]): ExtensionCatalogEntry | undefined {
  const entry = entries.find(entry => entry.id === installed.manifest.id);
  if (!entry || !validateStrict(entry.version) || !validateStrict(installed.manifest.version)) return;
  const order = compareVersions(entry.version, installed.manifest.version);
  if (order > 0 || order === 0 && !sameExtensionOrigin(installed.source, { type: 'github', entry })) return entry;
}
