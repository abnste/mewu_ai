// SPDX-License-Identifier: MPL-2.0
import { createSignal } from 'solid-js';
import * as api from './extension-bridge';
import type { ExtensionCatalogEntry, ExtensionIcon } from './extension-contracts';

const STORAGE = 'mewu.extension-icons.v1', MAX_CACHE = 2 * 1024 * 1024;
const [icons, setIcons] = createSignal<Record<string, ExtensionIcon>>({});
let initialized = false, fetched = 0, pending: Promise<void> | undefined;
export const iconKey = (id: string, version: string, sha256: string) => JSON.stringify([id, version, sha256]);
// Only fixed SVG nodes/attributes are rendered. This also bounds untrusted
// localStorage; native package/catalog validation remains authoritative.
export function displayIcon(value: unknown): ExtensionIcon | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const icon = value as ExtensionIcon;
  if (!Array.isArray(icon.paths) || !icon.paths.length || icon.paths.length > 16
    || icon.paths.some(p => typeof p !== 'string' || !p.trim() || p.length > 4096)
    || icon.paths.reduce((n, p) => n + p.length, 0) > 8192
    || icon.color !== undefined && !/^#[\da-f]{6}$/i.test(icon.color)
    || icon.filled !== undefined && typeof icon.filled !== 'boolean') return;
  return { paths: icon.paths, ...(icon.color ? { color: icon.color } : {}), filled: icon.filled === true };
}
function initialize() {
  if (initialized) return; initialized = true;
  try {
    const text = localStorage.getItem(STORAGE); if (!text || text.length > MAX_CACHE) return;
    const rows: unknown = JSON.parse(text); if (!Array.isArray(rows) || rows.length > 256) return;
    const next: Record<string, ExtensionIcon> = {};
    for (const row of rows) {
      if (!Array.isArray(row) || row.length !== 2 || typeof row[0] !== 'string' || row[0].length > 512) continue;
      const icon = displayIcon(row[1]); if (icon) next[row[0]] = icon;
    }
    setIcons(next);
  } catch { /* Artwork is optional; no persistent configuration is overwritten. */ }
}
export function cachedExtensionIcon(id: string, version: string, sha256?: string) {
  initialize(); return sha256 ? icons()[iconKey(id, version, sha256)] : undefined;
}
export function rememberExtensionIcons(entries: ExtensionCatalogEntry[]) {
  initialize(); const next: Record<string, ExtensionIcon> = {};
  for (const entry of entries.slice(0, 256)) {
    const icon = displayIcon(entry.icon); if (icon) next[iconKey(entry.id, entry.version, entry.sha256)] = icon;
  }
  // Keep old installed-version artwork too, with a bounded cache and exact SHA.
  for (const [key, icon] of Object.entries(icons())) { if (Object.keys(next).length >= 256) break; if (!next[key]) next[key] = icon; }
  setIcons(next); fetched = Date.now();
  try { const text = JSON.stringify(Object.entries(next)); if (text.length <= MAX_CACHE) localStorage.setItem(STORAGE, text); } catch { /* Quota/private mode uses in-memory artwork. */ }
}
export function ensureExtensionIcons(): Promise<void> {
  initialize();
  if (!api.extensionNative || Date.now() - fetched < 10 * 60_000) return Promise.resolve();
  if (!pending) pending = Promise.resolve().then(() => api.getExtensionCatalog()).then(value => rememberExtensionIcons(value.plugins))
    .catch(() => { fetched = Date.now(); }).finally(() => { pending = undefined; });
  return pending;
}
