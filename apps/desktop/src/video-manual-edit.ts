// SPDX-License-Identifier: MPL-2.0
import type { SpaceItem } from './contracts';
import type { TimedVideoAnnotation, VideoAnnotationAction, VideoAnnotationTarget, VideoPixelPoint, VideoPixelRect, VideoTextContent, VideoTextLayoutRef, VideoTextRead } from './video-annotation-contracts';
import { videoAnnotationBounds } from './video-annotations';
import type { VideoRequest } from './video-requests';
import { TrimCanceled } from './video-trim';

export interface VideoManualAuthority { key: string; target: VideoAnnotationTarget; item: SpaceItem; editable: boolean }
export interface VideoTextDraft { key: string; target: VideoAnnotationTarget; annotationId: string; reference: VideoTextLayoutRef; original: VideoTextContent; content: VideoTextContent; error?: string }
export interface VideoManualView { move?: { annotationId: string; bounds: VideoPixelRect }; text?: VideoTextDraft; reading: boolean; pending: boolean }
const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const changed = (draft: VideoTextDraft) => !equal(draft.original, draft.content);
const point = (value: VideoPixelPoint) => Number.isFinite(value.x) && Number.isFinite(value.y);
const draftKey = (value: VideoTextDraft) => JSON.stringify([value.target.sceneId, value.target.itemId, value.target.sourceId, value.annotationId]);

/** Renderer-only recovery; dirty input is never evicted or rebased onto another CAS. */
export class VideoTextDraftCache {
  private values = new Map<string, VideoTextDraft>();
  constructor(private limit = 16, private maxBytes = 128 * 1024) {}
  remember(draft: VideoTextDraft) {
    const key = draftKey(draft);
    if (!changed(draft)) { this.values.delete(key); return; }
    const values = [...this.values].filter(([id]) => id !== key); values.push([key, draft]);
    if (values.length > this.limit || new TextEncoder().encode(JSON.stringify(values)).length > this.maxBytes) throw Error('未保存视频文字过多');
    this.values.set(key, draft);
  }
  find(target: VideoAnnotationTarget, annotationId?: string) { return [...this.values.values()].find(value => value.target.sceneId === target.sceneId && value.target.itemId === target.itemId && value.target.sourceId === target.sourceId && (!annotationId || value.annotationId === annotationId)); }
  recover(target: VideoAnnotationTarget) { return [...this.values.values()].find(value => value.target.sceneId === target.sceneId && value.target.itemId === target.itemId); }
  list(sceneId?: string) { return [...this.values.values()].filter(value => sceneId === undefined || value.target.sceneId === sceneId); }
  remove(draft: VideoTextDraft) { if (this.values.get(draftKey(draft)) === draft) this.values.delete(draftKey(draft)); }
}
export const videoTextDrafts = new VideoTextDraftCache();
export function assertVideoTextDraftsSaved(sceneId?: string) { if (videoTextDrafts.list(sceneId).length) throw Error('还有未保存的视频文字'); }
export function validateVideoTextContent(value: unknown): value is VideoTextContent {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  return Object.keys(v).length === 4 && v.version === 1 && typeof v.text === 'string' && v.text.trim().length > 0 && [...v.text].length <= 500 && new TextEncoder().encode(v.text).length <= 4096 && !/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(v.text) && typeof v.color === 'string' && /^#[0-9a-f]{6}$/i.test(v.color) && typeof v.fontSize === 'number' && Number.isFinite(v.fontSize) && v.fontSize >= 8 && v.fontSize <= 128;
}
export function validateVideoTextRead(raw: VideoTextRead, requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef): VideoTextRead {
  if (!raw || Object.keys(raw).length !== 5 || raw.requestId !== requestId || !equal(raw.target, target) || raw.annotationId !== annotationId || !equal(raw.reference, reference) || !validateVideoTextContent(raw.content)) throw Error('视频文字回执不一致');
  return structuredClone(raw);
}
export function videoPoint(client: VideoPixelPoint, box: { left: number; top: number; width: number; height: number }, source: { width: number; height: number }): VideoPixelPoint {
  if (!point(client) || ![box.left, box.top, box.width, box.height, source.width, source.height].every(Number.isFinite) || box.width <= 0 || box.height <= 0 || source.width <= 0 || source.height <= 0) throw Error('视频尺寸无效');
  return { x: (client.x - box.left) / box.width * source.width, y: (client.y - box.top) / box.height * source.height };
}
export function movedVideoObject(object: TimedVideoAnnotation, position: VideoPixelPoint): TimedVideoAnnotation {
  return { ...structuredClone(object), primitive: object.primitive.kind === 'rect' ? { ...object.primitive, bounds: { ...object.primitive.bounds, ...position } } : { ...object.primitive, topLeft: { ...position } } };
}
interface Hooks {
  authority: () => VideoManualAuthority;
  move: (target: VideoAnnotationTarget, action: VideoAnnotationAction) => Promise<SpaceItem>;
  read: (target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef) => VideoRequest<VideoTextRead>;
  edit: (draft: VideoTextDraft) => VideoRequest<SpaceItem>;
  pause: () => void;
  publish: (view: VideoManualView) => void;
  cache?: VideoTextDraftCache;
}
/** A pointer gesture is one immutable source-space CAS. Text waits real native receipts. */
export class VideoManualEditor {
  private gesture?: { authority: VideoManualAuthority; object: TimedVideoAnnotation; start: VideoPixelPoint; from: VideoPixelPoint; to: VideoPixelPoint };
  private text?: VideoTextDraft;
  private reading?: VideoRequest<VideoTextRead>;
  private readingKey?: string;
  private pending?: Promise<void>;
  private editing?: VideoRequest<SpaceItem>;
  private disposed = false;
  private cache: VideoTextDraftCache;
  constructor(private hooks: Hooks) { this.cache = hooks.cache ?? videoTextDrafts; const authority = hooks.authority(); this.text = this.cache.recover(authority.target); if (this.text && this.text.key !== authority.key) { this.text = { ...this.text, error: '视频标注已变化' }; this.cache.remember(this.text); } this.publish(); }
  private publish() { if (!this.disposed) this.hooks.publish({ move: this.gesture ? { annotationId: this.gesture.object.id, bounds: { ...videoAnnotationBounds(this.gesture.object), ...this.gesture.to } } : undefined, text: this.text, reading: !!this.reading, pending: !!this.pending }); }
  private valid(key: string) { return !this.disposed && this.hooks.authority().key === key; }
  reconcile() { if (this.gesture && !this.valid(this.gesture.authority.key)) this.gesture = undefined; if (this.reading && (!this.hooks.authority().editable || !this.valid(this.readingKey!))) { const old = this.reading; this.reading = undefined; this.readingKey = undefined; void old.cancel().catch(() => {}); } if (this.text && !this.valid(this.text.key) && !this.pending) { this.text = { ...this.text, error: '视频标注已变化' }; this.cache.remember(this.text); } this.publish(); }
  begin(annotationId: string, at: VideoPixelPoint): boolean {
    const authority = this.hooks.authority(), object = authority.item.videoAnnotations?.objects.find(value => value.id === annotationId);
    if (this.disposed || !authority.editable || this.pending || this.reading || this.text || !object || !point(at)) return false;
    this.hooks.pause(); const bounds = videoAnnotationBounds(object);
    this.gesture = { authority: structuredClone(authority), object: structuredClone(object), start: { ...at }, from: { x: bounds.x, y: bounds.y }, to: { x: bounds.x, y: bounds.y } }; this.publish(); return true;
  }
  move(at: VideoPixelPoint) {
    const g = this.gesture, doc = g?.authority.item.videoAnnotations;
    if (!g || !doc || !point(at) || !this.valid(g.authority.key) || !this.hooks.authority().editable) { this.cancelMove(); return; }
    const bounds = videoAnnotationBounds(g.object);
    if (g.object.primitive.kind === 'vector') {
      const geometry = g.object.primitive.layout.geometryBounds;
      g.to = {
        x: Math.max(Math.ceil(-geometry.x), Math.min(Math.floor(doc.sourceWidth - geometry.x - geometry.width), Math.floor(g.from.x + at.x - g.start.x + .5))),
        y: Math.max(Math.ceil(-geometry.y), Math.min(Math.floor(doc.sourceHeight - geometry.y - geometry.height), Math.floor(g.from.y + at.y - g.start.y + .5))),
      };
      this.publish(); return;
    }
    g.to = { x: Math.max(0, Math.min(doc.sourceWidth - bounds.width, g.from.x + at.x - g.start.x)), y: Math.max(0, Math.min(doc.sourceHeight - bounds.height, g.from.y + at.y - g.start.y)) }; this.publish();
  }
  cancelMove() { this.gesture = undefined; this.publish(); }
  completeMove(): Promise<void> {
    const g = this.gesture; this.gesture = undefined;
    if (!g || !this.valid(g.authority.key) || !this.hooks.authority().editable || equal(g.from, g.to)) { this.publish(); return Promise.resolve(); }
    this.pending = (async () => {
      const returned = await this.hooks.move(g.authority.target, { type: 'move', annotationId: g.object.id, fromTopLeft: g.from, toTopLeft: g.to });
      const expected = g.authority.item.videoAnnotations!.objects.map(value => value.id === g.object.id ? movedVideoObject(value, g.to) : value);
      if (!equal(returned.asset, g.authority.item.asset) || !equal(returned.videoEdit, g.authority.item.videoEdit) || returned.videoAnnotations?.revision !== g.authority.target.expectedAnnotationRevision + 1 || !equal(returned.videoAnnotations.objects, expected)) throw Error('视频移动回执不一致');
    })().finally(() => { this.pending = undefined; this.publish(); }); this.publish(); return this.pending;
  }
  async open(annotationId: string) {
    if (this.pending || this.disposed || !this.hooks.authority().editable) return;
    await this.flush(() => !this.disposed);
    const authority = this.hooks.authority(), object = authority.item.videoAnnotations?.objects.find(value => value.id === annotationId);
    if (!authority.editable || object?.primitive.kind !== 'text') return;
    this.hooks.pause(); this.cancelMove();
    const restored = this.cache.find(authority.target, annotationId);
    if (restored) { this.text = restored; this.publish(); return; }
    const reference = structuredClone(object.primitive.layout), request = this.hooks.read(authority.target, annotationId, reference); this.reading = request; this.readingKey = authority.key; this.publish();
    try {
      const raw = validateVideoTextRead(await request.promise, request.id, authority.target, annotationId, reference);
      if (this.reading !== request || !this.valid(authority.key) || !this.hooks.authority().editable) throw new TrimCanceled();
      this.text = { key: authority.key, target: structuredClone(authority.target), annotationId, reference, original: raw.content, content: structuredClone(raw.content) };
    } finally { if (this.reading === request) { this.reading = undefined; this.readingKey = undefined; } this.publish(); }
  }
  input(value: string) {
    if (!this.text || this.pending || this.disposed || !this.hooks.authority().editable) return;
    const next = { ...this.text, content: { ...this.text.content, text: value }, error: undefined }; this.cache.remember(next); this.text = next; this.publish();
  }
  async save(active: () => boolean = () => true, flushing = false) {
    if (this.pending) { await this.pending; if (!active()) throw new TrimCanceled(); }
    const draft = this.text; if (!draft) return;
    if (!flushing && !this.hooks.authority().editable) throw new TrimCanceled();
    if (!changed(draft)) { this.cache.remove(draft); this.text = undefined; this.publish(); return; }
    if (!active() || !this.valid(draft.key) || !validateVideoTextContent(draft.content)) { const error = !validateVideoTextContent(draft.content) ? '视频文字无效' : '视频标注已变化'; this.text = { ...draft, error }; this.cache.remember(this.text); this.publish(); throw Error(error); }
    const before = structuredClone(this.hooks.authority().item), request = this.hooks.edit(draft); this.editing = request;
    this.pending = (async () => {
      try {
        const returned = await request.promise, object = returned.videoAnnotations?.objects.find(value => value.id === draft.annotationId), original = before.videoAnnotations!.objects.find(value => value.id === draft.annotationId)!;
        if (!equal(returned.asset, before.asset) || !equal(returned.videoEdit, before.videoEdit) || returned.videoAnnotations?.revision !== draft.target.expectedAnnotationRevision + 1 || object?.primitive.kind !== 'text' || original.primitive.kind !== 'text' || !equal(object.interval, original.interval) || !equal(object.origin, original.origin) || !equal(object.primitive.topLeft, original.primitive.topLeft) || returned.videoAnnotations.objects.length !== before.videoAnnotations!.objects.length || !equal(returned.videoAnnotations.objects.filter(value => value.id !== draft.annotationId), before.videoAnnotations!.objects.filter(value => value.id !== draft.annotationId))) throw Error('视频文字回执不一致');
        // Native commit is authoritative even if the component was disposed while rendering.
        this.cache.remove(draft); if (this.text === draft) this.text = undefined;
      } catch (error) {
        if (this.text === draft) { this.text = { ...draft, error: error instanceof Error ? error.message : String(error) }; this.cache.remember(this.text); }
        throw error;
      }
    })().finally(() => { this.pending = undefined; this.editing = undefined; this.publish(); }); this.publish(); await this.pending;
    if (!active()) throw new TrimCanceled();
  }
  async cancelText() {
    this.cancelMove();
    if (this.reading) { const old = this.reading; this.reading = undefined; await old.cancel(); }
    if (this.editing) { await this.editing.cancel(); try { await this.pending; } catch (error) { if (!(error instanceof TrimCanceled)) throw error; } }
    if (this.text) { this.cache.remove(this.text); this.text = undefined; } this.publish();
  }
  interrupt() { this.cancelMove(); if (this.reading) { const old = this.reading; this.reading = undefined; this.readingKey = undefined; void old.cancel().catch(() => {}); } this.publish(); }
  async flush(active: () => boolean) { this.cancelMove(); if (this.reading) { const old = this.reading; this.reading = undefined; this.readingKey = undefined; await old.cancel(); } await this.save(active, true); }
  dispose() { this.interrupt(); this.disposed = true; /* dirty drafts and real in-flight receipts retain ownership */ }
}
