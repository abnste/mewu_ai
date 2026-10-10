// SPDX-License-Identifier: MPL-2.0
import type { SpaceItem } from './contracts';
import {validVideoVectorReference} from './video-drawing-wire';
import {visibleVideoVectorBounds} from './video-drawing-session';
import type { VideoAnnotationPlan, VideoAnnotationPreview, VideoAnnotationPreviewTarget, VideoAnnotationRasterRef, VideoAnnotationTarget } from './video-annotation-contracts';
import { sameVideoAnnotationTarget, videoAnnotationBounds, videoAnnotationTarget, videoRetainedRange } from './video-annotations';

const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const keys = (value: Record<string, unknown>, expected: string[]) => Object.keys(value).length === expected.length && expected.every(key => Object.hasOwn(value, key));
const uuid = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
const hash = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const revision = (value: unknown) => Number.isSafeInteger(value) && Number(value) >= 0;
const dimensions = (width: unknown, height: unknown) => Number.isSafeInteger(width) && Number.isSafeInteger(height) && Number(width) > 0 && Number(height) > 0 && Number(width) <= 8192 && Number(height) <= 8192 && Number(width) * Number(height) <= 4 * 1024 * 1024;
const sameRange = (value: unknown, expected: { startTicks: number; endTicks: number }) => record(value) && keys(value, ['startTicks', 'endTicks']) && value.startTicks === expected.startTicks && value.endTicks === expected.endTicks;
const sameBounds = (value: unknown, expected: { x: number; y: number; width: number; height: number }) => record(value) && keys(value, ['x', 'y', 'width', 'height']) && value.x === expected.x && value.y === expected.y && value.width === expected.width && value.height === expected.height;
export function validVideoAnnotationTarget(value: unknown): value is VideoAnnotationTarget {
  return record(value) && keys(value, ['sceneId', 'itemId', 'sourceId', 'expectedRangeRevision', 'expectedAnnotationRevision']) && [value.sceneId, value.itemId, value.sourceId].every(uuid) && revision(value.expectedRangeRevision) && revision(value.expectedAnnotationRevision);
}
function validRaster(value: unknown): value is VideoAnnotationRasterRef {
  return record(value) && keys(value, ['rasterKey', 'pngSha256', 'width', 'height']) && hash(value.rasterKey) && hash(value.pngSha256) && dimensions(value.width, value.height);
}
export function validateVideoAnnotationPlan(raw: unknown, sceneId: string, item: SpaceItem): VideoAnnotationPlan {
  const doc = item.videoAnnotations, range = videoRetainedRange(item);
  if (!doc || !range || !record(raw) || !keys(raw, ['target', 'documentSha256', 'clock', 'sourceDurationTicks', 'sourceWidth', 'sourceHeight', 'range', 'entries']) || !validVideoAnnotationTarget(raw.target) || !sameVideoAnnotationTarget(raw.target, videoAnnotationTarget(sceneId, item)) || !hash(raw.documentSha256) || raw.clock !== 'sourcePlaybackTicks' || doc.clock !== raw.clock || raw.sourceDurationTicks !== doc.sourceDurationTicks || raw.sourceWidth !== doc.sourceWidth || raw.sourceHeight !== doc.sourceHeight || !dimensions(raw.sourceWidth, raw.sourceHeight) || !sameRange(raw.range, range) || !Array.isArray(raw.entries) || raw.entries.length !== doc.objects.length || raw.entries.length > 48) throw Error('视频标注已变化');
  for (let i = 0; i < raw.entries.length; i++) {
    const entry = raw.entries[i], object = doc.objects[i], bounds = object.primitive.kind === 'vector' ? visibleVideoVectorBounds(object.primitive.topLeft,object.primitive.layout,{width:doc.sourceWidth,height:doc.sourceHeight}) : videoAnnotationBounds(object);
    if(object.primitive.kind==='vector'&&!validVideoVectorReference(object.primitive.layout))throw Error('视频绘制预览无效');
    if (!record(entry) || !keys(entry, ['annotationId', 'interval', 'bounds', 'reference']) || entry.annotationId !== object.id || !sameRange(entry.interval, object.interval) || !sameBounds(entry.bounds, bounds) || !validRaster(entry.reference) || entry.reference.width !== Math.ceil(bounds.width) || entry.reference.height !== Math.ceil(bounds.height)) throw Error('视频标注预览无效');
    if (object.primitive.kind === 'text' && entry.reference.pngSha256 !== object.primitive.layout.rasterSha256) throw Error('视频文字预览无效');
  }
  return structuredClone(raw) as unknown as VideoAnnotationPlan;
}
export function validVideoPreviewTarget(value: unknown): value is VideoAnnotationPreviewTarget {
  return record(value) && keys(value, ['target', 'documentSha256', 'annotationId', 'reference']) && validVideoAnnotationTarget(value.target) && hash(value.documentSha256) && uuid(value.annotationId) && validRaster(value.reference);
}
export function videoPreviewKey(value: VideoAnnotationPreviewTarget): string {
  const t = value.target, r = value.reference;
  return JSON.stringify([t.sceneId, t.itemId, t.sourceId, t.expectedRangeRevision, t.expectedAnnotationRevision, value.documentSha256, value.annotationId, r.rasterKey, r.pngSha256, r.width, r.height]);
}
function pngMatches(value: unknown, reference: VideoAnnotationRasterRef): value is string {
  const prefix = 'data:image/png;base64,';
  if (typeof value !== 'string' || !value.startsWith(prefix) || value.length > prefix.length + Math.ceil(8 * 1024 * 1024 / 3) * 4) return false;
  const data = value.slice(prefix.length);
  if (data.length < 80 || data.length % 4 || !/^[A-Za-z0-9+/]+={0,2}$/.test(data)) return false;
  try {
    const header = atob(data.slice(0, 44));
    const u32 = (at: number) => header.charCodeAt(at) * 0x1000000 + header.charCodeAt(at + 1) * 0x10000 + header.charCodeAt(at + 2) * 0x100 + header.charCodeAt(at + 3);
    return header.slice(0, 8) === '\x89PNG\r\n\x1a\n' && header.slice(12, 16) === 'IHDR' && u32(8) === 13 && u32(16) === reference.width && u32(20) === reference.height;
  } catch { return false; }
}
export function validateVideoAnnotationPreview(raw: unknown, expected: VideoAnnotationPreviewTarget): VideoAnnotationPreview {
  if (!record(raw)) throw Error('视频标注预览无效');
  const { dataUrl, ...target } = raw;
  if (!validVideoPreviewTarget(target) || videoPreviewKey(target) !== videoPreviewKey(expected) || !pngMatches(dataUrl, expected.reference)) throw Error('视频标注预览无效');
  return { ...structuredClone(expected), dataUrl };
}

// Shared across cards and disposed readers. A dropped UI subscription does not
// release an actual IPC read early. Rejected reads are not cached or retried.
export class VideoAnnotationReadBroker {
  private running = 0;
  private waiting: (() => void)[] = [];
  private previews = new Map<string, { promise: Promise<VideoAnnotationPreview>; settled: boolean; size: number }>();
  constructor(private concurrency = 2, private maxPending = 128, private capacity = 8, private maxBytes = 16 * 1024 * 1024) {}
  read<T>(work: () => Promise<T>): Promise<T> {
    if (this.running + this.waiting.length >= this.maxPending) return Promise.reject(Error('视频标注读取繁忙'));
    return new Promise((resolve, reject) => {
      this.waiting.push(() => { this.running++; void Promise.resolve().then(work).then(resolve, reject).finally(() => { this.running--; this.pump(); }); });
      this.pump();
    });
  }
  private pump() { while (this.running < this.concurrency && this.waiting.length) this.waiting.shift()!(); }
  preview(target: VideoAnnotationPreviewTarget, sourceIdentity: string, load: () => Promise<unknown>): Promise<VideoAnnotationPreview> {
    if (!validVideoPreviewTarget(target)) return Promise.reject(Error('视频标注预览无效'));
    const key = JSON.stringify([sourceIdentity, videoPreviewKey(target)]), found = this.previews.get(key);
    if (found) { this.previews.delete(key); this.previews.set(key, found); return found.promise; }
    const copy = structuredClone(target), entry = { promise: undefined! as Promise<VideoAnnotationPreview>, settled: false, size: 0 };
    entry.promise = this.read(load).then(raw => validateVideoAnnotationPreview(raw, copy)).then(value => {
      entry.settled = true; entry.size = value.dataUrl.length; this.trim(); return value;
    }, error => { if (this.previews.get(key) === entry) this.previews.delete(key); throw error; });
    this.previews.set(key, entry); return entry.promise;
  }
  private trim() {
    let ready = [...this.previews].filter(([, value]) => value.settled), bytes = ready.reduce((sum, [, value]) => sum + value.size, 0);
    for (const [key, entry] of ready) {
      if (ready.length <= this.capacity && bytes <= this.maxBytes) break;
      this.previews.delete(key); bytes -= entry.size; ready = ready.filter(([id]) => id !== key);
    }
  }
}
