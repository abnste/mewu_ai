// SPDX-License-Identifier: MPL-2.0
// Native owns the immutable descriptor and PNG validation.
import type { Asset, Drawing, Region, RichDrawingRef } from './contracts';

export interface DrawingLayoutTarget { sceneId: string; regionId: string; drawingId: string; expectedRevision: number; reference: RichDrawingRef }
export interface DrawingLayoutPreview extends DrawingLayoutTarget { dataUrl: string }
export type DrawingTableFormat = 'table' | 'markdown' | 'csv' | 'tsv' | 'png';
export interface RichDrawingContext { sceneId: string; background: Asset; region: Region }
const assetIdentity = (asset: Asset) => [asset.id, asset.name, asset.kind, asset.path, asset.width, asset.height, asset.originX, asset.originY, asset.scaleFactor];
export function richSourceIdentity(context: RichDrawingContext): string {
  const region = context.region;
  return JSON.stringify([context.sceneId, region.id, assetIdentity(context.background), assetIdentity(region.imageOverride ?? context.background), region.x, region.y, region.width, region.height]);
}
export function richPreviewTarget(context: RichDrawingContext, drawing: Drawing): DrawingLayoutTarget | undefined {
  if (drawing.kind !== 'rich' || !drawing.rich) return;
  return { sceneId: context.sceneId, regionId: context.region.id, drawingId: drawing.id, expectedRevision: context.region.drawingRevision ?? 0, reference: { ...drawing.rich } };
}
const maxPngBytes = 8 * 1024 * 1024, prefix = 'data:image/png;base64,';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i, hash = /^[0-9a-f]{64}$/;
const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const exactKeys = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
export function validRichReference(value: unknown): value is RichDrawingRef {
  if (!record(value) || !exactKeys(value, ['layoutId', 'layoutSha256', 'rasterSha256', 'kind', 'width', 'height', ...(Object.hasOwn(value,'parentId')?['parentId']:[])])) return false;
  if(Object.hasOwn(value,'parentId')&&(value.kind!=='repair'||typeof value.parentId!=='string'||!uuid.test(value.parentId)))return false;
  return typeof value.layoutId === 'string' && uuid.test(value.layoutId) && typeof value.layoutSha256 === 'string' && hash.test(value.layoutSha256)
    && typeof value.rasterSha256 === 'string' && hash.test(value.rasterSha256) && ['table', 'formula', 'repair', 'extracted'].includes(String(value.kind))
    && Number.isSafeInteger(value.width) && Number.isSafeInteger(value.height) && Number(value.width) > 0 && Number(value.height) > 0
    && Number(value.width) <= 6000 && Number(value.height) <= 6000 && Number(value.width) * Number(value.height) <= 32 * 1024 * 1024;
}
export function validLayoutTarget(value: unknown): value is DrawingLayoutTarget {
  return record(value) && exactKeys(value, ['sceneId', 'regionId', 'drawingId', 'expectedRevision', 'reference'])
    && [value.sceneId, value.regionId, value.drawingId].every(id => typeof id === 'string' && uuid.test(id))
    && Number.isSafeInteger(value.expectedRevision) && Number(value.expectedRevision) >= 0 && validRichReference(value.reference);
}
const referenceKey = (value: RichDrawingRef) => [value.layoutId, value.layoutSha256, value.rasterSha256, value.kind, value.width, value.height,value.parentId??null];
export const sameRichReference = (left: RichDrawingRef, right: RichDrawingRef) => JSON.stringify(referenceKey(left)) === JSON.stringify(referenceKey(right));
export function layoutRequestKey(target: DrawingLayoutTarget, sourceIdentity: string): string {
  return JSON.stringify([sourceIdentity, target.sceneId, target.regionId, target.drawingId, target.expectedRevision, referenceKey(target.reference)]);
}
export const cloneLayoutTarget = (value: DrawingLayoutTarget): DrawingLayoutTarget => ({ ...value, reference: { ...value.reference } });

function pngMatchesReference(dataUrl: unknown, ref: RichDrawingRef): dataUrl is string {
  if (typeof dataUrl !== 'string' || !dataUrl.startsWith(prefix) || dataUrl.length > prefix.length + Math.ceil(maxPngBytes / 3) * 4) return false;
  const base64 = dataUrl.slice(prefix.length);
  if (base64.length < 80 || base64.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(base64)) return false;
  try {
    // Only decode the fixed PNG header here; native verifies the complete raster SHA/decode.
    const bytes = atob(base64.slice(0, 44));
    if (bytes.slice(0, 8) !== '\x89PNG\r\n\x1a\n' || bytes.slice(12, 16) !== 'IHDR') return false;
    const u32 = (at: number) => bytes.charCodeAt(at) * 0x1000000 + bytes.charCodeAt(at + 1) * 0x10000 + bytes.charCodeAt(at + 2) * 0x100 + bytes.charCodeAt(at + 3);
    return u32(8) === 13 && u32(16) === ref.width && u32(20) === ref.height;
  } catch { return false; }
}
export function validateLayoutPreview(value: unknown, expected: DrawingLayoutTarget): DrawingLayoutPreview {
  if (!record(value) || !exactKeys(value, ['sceneId', 'regionId', 'drawingId', 'expectedRevision', 'reference', 'dataUrl'])) throw Error('图中对象预览无效');
  const { dataUrl, ...target } = value;
  if (!validLayoutTarget(target) || layoutRequestKey(target, '') !== layoutRequestKey(expected, '') || !pngMatchesReference(dataUrl, expected.reference)) throw Error('图中对象预览无效');
  return { ...cloneLayoutTarget(expected), dataUrl };
}

interface Entry { key: string; target: DrawingLayoutTarget; source: string; load: () => Promise<unknown>; promise: Promise<DrawingLayoutPreview>; resolve: (value: DrawingLayoutPreview) => void; reject: (error: unknown) => void; settled: boolean; bytes: number }
// One module-wide broker must be used by all mounted/just-unmounted layers.
// A renderer being disposed does not release a running native read early.
export class DrawingLayoutPreviewCache {
  private entries = new Map<string, Entry>();
  private waiting: Entry[] = [];
  private running = 0;
  constructor(private concurrency = 2, private maximumPending = 256, private capacity = 8, private maxBase64Bytes = 16 * 1024 * 1024) {}
  get(target: DrawingLayoutTarget, sourceIdentity: string, load: () => Promise<unknown>): Promise<DrawingLayoutPreview> {
    if (!validLayoutTarget(target)) return Promise.reject(Error('图中对象预览无效'));
    const copy = cloneLayoutTarget(target), key = layoutRequestKey(copy, sourceIdentity), existing = this.entries.get(key);
    if (existing) { this.entries.delete(key); this.entries.set(key, existing); return existing.promise; }
    if ([...this.entries.values()].filter(value => !value.settled).length >= this.maximumPending) return Promise.reject(Error('图中对象预览繁忙'));
    let resolve!: Entry['resolve'], reject!: Entry['reject'];
    const promise = new Promise<DrawingLayoutPreview>((yes, no) => { resolve = yes; reject = no; });
    const entry: Entry = { key, target: copy, source: sourceIdentity, load, promise, resolve, reject, settled: false, bytes: 0 };
    this.entries.set(key, entry); this.waiting.push(entry); this.pump(); return promise;
  }
  private pump() {
    while (this.running < this.concurrency && this.waiting.length) {
      const entry = this.waiting.shift()!; this.running++;
      void Promise.resolve().then(entry.load).then(raw => validateLayoutPreview(raw, entry.target)).then(value => {
        entry.settled = true; entry.bytes = value.dataUrl.length;
        this.trim(); entry.resolve(value);
      }, error => { if (this.entries.get(entry.key) === entry) this.entries.delete(entry.key); entry.reject(error); }).finally(() => { this.running--; this.pump(); });
    }
  }
  private trim() {
    let ready = [...this.entries.values()].filter(value => value.settled), bytes = ready.reduce((sum, value) => sum + value.bytes, 0);
    for (const entry of ready) {
      if (ready.length <= this.capacity && bytes <= this.maxBase64Bytes) break;
      this.entries.delete(entry.key); bytes -= entry.bytes; ready = ready.filter(value => value !== entry);
    }
  }
}
export interface DrawingLayoutReadState { preview?: DrawingLayoutPreview; loading: boolean; error?: string }
export class DrawingLayoutReader {
  private generation = 0;
  private disposed = false;
  private visible?: DrawingLayoutPreview;
  private source = '';
  constructor(private read: (target: DrawingLayoutTarget, sourceIdentity: string) => Promise<DrawingLayoutPreview>, private publish: (value: DrawingLayoutReadState) => void) {}
  select(target?: DrawingLayoutTarget, sourceIdentity = '') {
    const generation = ++this.generation; if (this.disposed) return;
    const previous = this.visible;
    const keep = target && previous && sourceIdentity === this.source
      && target.sceneId === previous.sceneId && target.regionId === previous.regionId
      && target.drawingId === previous.drawingId && sameRichReference(target.reference, previous.reference);
    if (!keep) this.visible = undefined;
    this.source = sourceIdentity;
    this.publish({ loading: Boolean(target), ...(keep ? { preview: previous } : {}) });
    if (!target) return;
    void Promise.resolve().then(() => this.read(cloneLayoutTarget(target), sourceIdentity)).then(preview => {
      if (!this.disposed && generation === this.generation) { this.visible = preview; this.publish({ loading: false, preview }); }
    }, cause => { if (!this.disposed && generation === this.generation) { this.visible = undefined; this.publish({ loading: false, error: cause instanceof Error ? cause.message : String(cause) }); } });
  }
  imageFailed() { if (!this.disposed) { this.generation++; this.visible = undefined; this.publish({ loading: false, error: '无法读取图中对象' }); } }
  dispose() { this.disposed = true; this.generation++; }
}
