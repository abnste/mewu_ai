// SPDX-License-Identifier: MPL-2.0
import type { Asset, Scene } from './contracts';
import type { PluginRecord } from './plugin-contracts';
import type { CodeScanRequest, CodeScanResult, CodeScanView, CodeSource } from './code-contracts';

const assetKey = (asset?: Asset) => asset ? [asset.id, asset.name, asset.kind, asset.path, asset.width ?? null, asset.height ?? null, asset.originX ?? null, asset.originY ?? null, asset.scaleFactor ?? null] : null;
/** Only persisted pixel contributors: never serialize a Scene or its messages. */
export function codeSource(scene: Scene | undefined, regionId: string, plugins: PluginRecord[]): CodeSource | undefined {
  const background = scene?.background, region = scene?.regions.find(value => value.id === regionId);
  if (!scene || !background || !region || scene.closed || scene.frozen) return;
  const choices = plugins.filter(value => value.state === 'enabled' && !value.error).flatMap(plugin => plugin.manifest.contributions.filter(value => value.kind === 'selection.codes').map(contribution => ({ plugin, contribution })));
  choices.sort((a, b) => {
    const official = (value: typeof a) => value.plugin.source.type === 'official' && value.plugin.manifest.id === 'mewu.codes' && value.contribution.id === 'recognize';
    return Number(official(b)) - Number(official(a)) || (a.plugin.manifest.id < b.plugin.manifest.id ? -1 : a.plugin.manifest.id > b.plugin.manifest.id ? 1 : 0) || (a.contribution.id < b.contribution.id ? -1 : a.contribution.id > b.contribution.id ? 1 : 0);
  });
  const entry = choices[0]; if (!entry) return;
  const target = { sceneId: scene.id, regionId, backgroundId: background.id, drawingRevision: region.drawingRevision ?? 0, x: region.x, y: region.y, width: region.width, height: region.height };
  const translation = region.translation;
  const request = { pluginId: entry.plugin.manifest.id, revision: entry.plugin.revision, contributionId: entry.contribution.id, target, translationId: translation?.overlay.id ?? null };
  const key = JSON.stringify([request.pluginId, request.revision, request.contributionId, scene.id, regionId, assetKey(background), assetKey(region.imageOverride), region.x, region.y, region.width, region.height, target.drawingRevision,
    (region.drawings ?? []).map(value => [value.id, value.kind, value.color, value.strokeWidth, value.points.map(point => [point.x, point.y]), value.text ?? null, value.fontSize ?? null]),
    translation ? [translation.backgroundId, translation.sourceId, translation.drawingRevision, translation.document.width, translation.document.height, assetKey(translation.overlay)] : null]);
  return { key, request };
}

export function validCodeResult(value: CodeScanResult, request: CodeScanRequest): boolean {
  if (!value || value.requestId !== request.requestId || value.sceneId !== request.target.sceneId || value.regionId !== request.target.regionId || typeof value.sourceToken !== 'string' || !value.sourceToken || value.sourceToken.length > 256 || !Array.isArray(value.codes) || value.codes.length > 12) return false;
  const ids = new Set<string>(); let total = 0;
  return value.codes.every(code => {
    if (!code || typeof code.id !== 'string' || !code.id || code.id.length > 256 || ids.has(code.id) || typeof code.format !== 'string' || code.format.length > 80 || typeof code.text !== 'string' || typeof code.canOpen !== 'boolean') return false;
    ids.add(code.id); const bytes = new TextEncoder().encode(code.text).length; total += bytes;
    return bytes > 0 && bytes <= 8192 && total <= 65536;
  });
}

interface Ticket { source: CodeSource; request: CodeScanRequest; canceled: boolean; runDone: boolean; cancelDone: boolean; cancelError?: unknown; finished: Promise<void>; finish: () => void; fail: (error: unknown) => void }
/** Space-owned, including completed action authority. Slot survives Canvas remount. */
export class CodeScanController {
  private intent?: CodeSource;
  private current?: Ticket;
  private result?: CodeScanView;
  private paused = false;
  private invalidated = false;
  private disposed = false;
  private timer?: ReturnType<typeof setTimeout>;
  constructor(private hooks: { run: (request: CodeScanRequest) => Promise<CodeScanResult>; cancel: (requestId: string) => Promise<void>; changed: (view: CodeScanView | undefined) => void; matches: (source: CodeSource) => boolean; id?: () => string; delay?: number }) {}
  setSource(source: CodeSource | undefined, paused = false) {
    if (this.disposed) return;
    const changed = source?.key !== this.intent?.key;
    this.paused = paused;
    if (changed) {
      this.intent = source; this.invalidated = false; this.clearTimer(); this.publish(undefined); this.revoke();
    }
    if (paused) this.clearTimer(); else this.schedule();
  }
  isCurrent(sourceToken: string): boolean {
    const ticket = this.current;
    return Boolean(!this.disposed && !this.paused && !this.invalidated && ticket && !ticket.canceled && this.result?.result.sourceToken === sourceToken && this.intent?.key === ticket.source.key && this.hooks.matches(ticket.source));
  }
  invalidate(requestId: string) {
    if (this.current?.request.requestId !== requestId) return;
    this.invalidated = true; this.clearTimer(); this.publish(undefined); this.revoke();
  }
  cancel(): Promise<void> {
    this.intent = undefined; this.invalidated = false; this.clearTimer(); this.publish(undefined);
    const ticket = this.current; this.revoke(); return ticket?.finished ?? Promise.resolve();
  }
  dispose() { if (this.disposed) return; void this.cancel().catch(() => {}); this.disposed = true; }
  private publish(view: CodeScanView | undefined) { this.result = view; if (!this.disposed) this.hooks.changed(view); }
  private clearTimer() { if (this.timer !== undefined) clearTimeout(this.timer); this.timer = undefined; }
  private revoke() {
    const ticket = this.current; if (!ticket || ticket.canceled) return;
    ticket.canceled = true;
    void Promise.resolve().then(() => this.hooks.cancel(ticket.request.requestId)).catch(error => { ticket.cancelError = error; }).finally(() => { ticket.cancelDone = true; this.settle(ticket); });
  }
  private settle(ticket: Ticket) {
    if (this.current !== ticket || !ticket.canceled || !ticket.runDone || !ticket.cancelDone) return;
    this.current = undefined;
    if (ticket.cancelError) ticket.fail(ticket.cancelError); else ticket.finish();
    this.schedule();
  }
  private schedule() {
    if (this.disposed || this.paused || this.invalidated || this.current || this.timer !== undefined || !this.intent) return;
    this.timer = setTimeout(() => { this.timer = undefined; this.start(); }, this.hooks.delay ?? 150);
  }
  private start() {
    const source = this.intent;
    if (this.disposed || this.paused || this.invalidated || this.current || !source || !this.hooks.matches(source)) return;
    let finish!: () => void, fail!: (error: unknown) => void;
    const finished = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; }); void finished.catch(() => {});
    const ticket: Ticket = { source, request: { ...source.request, requestId: this.hooks.id?.() ?? crypto.randomUUID() }, canceled: false, runDone: false, cancelDone: false, finished, finish, fail };
    this.current = ticket;
    void Promise.resolve().then(() => ticket.canceled ? Promise.reject(new Error('扫码已取消')) : this.hooks.run(ticket.request)).then(value => {
      if (!this.disposed && this.current === ticket && !ticket.canceled && !this.invalidated && this.intent?.key === source.key && this.hooks.matches(source) && validCodeResult(value, ticket.request)) this.publish({ key: source.key, result: value });
    }).catch(() => { /* Automatic scans have no status/toast; retry on a new intent. */ }).finally(() => { ticket.runDone = true; this.settle(ticket); });
  }
}
