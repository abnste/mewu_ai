// SPDX-License-Identifier: MPL-2.0
import type { OcrTarget, Snapshot } from './contracts';
import type { PluginRecord } from './plugin-contracts';
import { sameOcrTarget } from './ocr-selection';

export const translationLanguages = [
  ['zh-Hans', '简体中文'], ['en', '英语'], ['ja', '日语'], ['ko', '韩语'], ['de', '德语'], ['fr', '法语'], ['es', '西班牙语'],
] as const;
export type TranslationLanguage = typeof translationLanguages[number][0];
export interface TranslationRequest { requestId: string; pluginId: string; revision: number; contributionId: string; target: OcrTarget; language: TranslationLanguage }
export interface TranslationProgress { requestId: string; sceneId: string; regionId: string; phase: 'recognizing' | 'translating' | 'rendering'; completed: number; total: number }
export interface TranslationPending { request: TranslationRequest; progress?: TranslationProgress }
const phases = { recognizing: 0, translating: 1, rendering: 2 };

export function translationScopeValid(request: TranslationRequest, snapshot: Snapshot, plugins: PluginRecord[], connection: string): boolean {
  const scene = snapshot.scenes.find(value => value.id === request.target.sceneId);
  const region = scene?.regions.find(value => value.id === request.target.regionId);
  const profile = snapshot.connections.find(value => value.id === scene?.connectionId);
  return Boolean(scene && !scene.closed && region && scene.background && profile && `${profile.id}:${profile.revision}` === connection &&
    sameOcrTarget(request.target, { sceneId: scene.id, regionId: region.id, backgroundId: scene.background.id, drawingRevision: region.drawingRevision ?? 0, x: region.x, y: region.y, width: region.width, height: region.height }) &&
    plugins.some(value => value.manifest.id === request.pluginId && value.revision === request.revision && value.state === 'enabled' && !value.error && value.manifest.contributions.some(item => item.id === request.contributionId && item.kind === 'selection.translation')));
}

// Owned by App, not the visible Canvas: freezing must not dispose a background task.
export class TranslationRequests<T> {
  private entries = new Map<string, TranslationPending>();
  private disposed = false;
  constructor(private hooks: { run: (request: TranslationRequest) => Promise<T>; cancel: (id: string) => Promise<void>; changed: (values: TranslationPending[]) => void; result: (value: T, request: TranslationRequest) => void; error: (message: string, request: TranslationRequest) => void }) {}
  private publish() { if (!this.disposed) this.hooks.changed([...this.entries.values()]); }
  async start(request: TranslationRequest) {
    if (this.disposed) return;
    this.cancelScene(request.target.sceneId);
    const entry: TranslationPending = { request }; this.entries.set(request.target.sceneId, entry); this.publish();
    const current = () => !this.disposed && this.entries.get(request.target.sceneId) === entry;
    try { const value = await this.hooks.run(request); if (current()) this.hooks.result(value, request); }
    catch (error) { if (current()) this.hooks.error(error instanceof Error ? error.message : String(error), request); }
    finally { if (current()) { this.entries.delete(request.target.sceneId); this.publish(); } }
  }
  progress(value: TranslationProgress) {
    const entry = this.entries.get(value.sceneId);
    if (!entry || entry.request.requestId !== value.requestId || entry.request.target.regionId !== value.regionId || !Object.hasOwn(phases, value.phase) || !Number.isInteger(value.completed) || !Number.isInteger(value.total) || value.completed < 0 || value.total < value.completed) return;
    const old = entry.progress;
    if (old && (phases[value.phase] < phases[old.phase] || (value.phase === old.phase && value.completed < old.completed))) return;
    entry.progress = value; this.publish();
  }
  cancelScene(sceneId: string) {
    const entry = this.entries.get(sceneId); if (!entry) return;
    this.entries.delete(sceneId); this.publish(); void this.hooks.cancel(entry.request.requestId).catch(() => undefined);
  }
  retain(valid: (request: TranslationRequest) => boolean) { for (const value of [...this.entries.values()]) if (!valid(value.request)) this.cancelScene(value.request.target.sceneId); }
  has(requestId: string) { return [...this.entries.values()].some(value => value.request.requestId === requestId); }
  cancelAll() { for (const id of [...this.entries.keys()]) this.cancelScene(id); }
  dispose() { this.disposed = true; this.cancelAll(); }
}
