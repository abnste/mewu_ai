// SPDX-License-Identifier: MPL-2.0
import type { Scene, SpaceItem, VideoRange } from './contracts';
import type { TimedVideoAnnotation, VideoAnnotationTarget, VideoAnswerAction, VideoPixelRect, VideoPlayerPort } from './video-annotation-contracts';
import { TrimCanceled } from './video-trim';

export function videoAnnotationTarget(sceneId: string, item: SpaceItem): VideoAnnotationTarget {
  return { sceneId, itemId: item.id, sourceId: item.asset.id, expectedRangeRevision: item.videoEdit?.revision ?? 0, expectedAnnotationRevision: item.videoAnnotations?.revision ?? 0 };
}
export function sameVideoAnnotationTarget(a: VideoAnnotationTarget, b: VideoAnnotationTarget): boolean {
  return a.sceneId === b.sceneId && a.itemId === b.itemId && a.sourceId === b.sourceId && a.expectedRangeRevision === b.expectedRangeRevision && a.expectedAnnotationRevision === b.expectedAnnotationRevision;
}
export const videoSourceIdentity = (sceneId: string, item: SpaceItem) => JSON.stringify([sceneId, item.id, item.asset]);
export const videoAnnotationIdentity = (sceneId: string, item: SpaceItem) => JSON.stringify([videoSourceIdentity(sceneId, item), item.videoEdit ?? null, item.videoAnnotations ?? null]);
export function videoAnnotationBounds(object: TimedVideoAnnotation): VideoPixelRect {
  return object.primitive.kind === 'rect' ? { ...object.primitive.bounds } : { ...object.primitive.topLeft, width: object.primitive.layout.width, height: object.primitive.layout.height };
}
export function videoRetainedRange(item: SpaceItem): VideoRange | undefined {
  const duration = item.videoEdit?.sourceDurationTicks ?? item.videoAnnotations?.sourceDurationTicks;
  return duration ? item.videoEdit?.range ?? { startTicks: 0, endTicks: duration } : undefined;
}
export function intersectVideoInterval(a: VideoRange, b: VideoRange): VideoRange | undefined {
  const startTicks = Math.max(a.startTicks, b.startTicks), endTicks = Math.min(a.endTicks, b.endTicks);
  return startTicks < endTicks ? { startTicks, endTicks } : undefined;
}
export const videoAnnotationVisible = (interval: VideoRange, range: VideoRange, sourcePlaybackTicks?: number) => sourcePlaybackTicks !== undefined && Number.isSafeInteger(sourcePlaybackTicks) && sourcePlaybackTicks >= Math.max(interval.startTicks, range.startTicks) && sourcePlaybackTicks < Math.min(interval.endTicks, range.endTicks);
// Links only come from committed document provenance, never model Markdown URLs.
export function videoAnswerActions(scene: Scene, runId?: string): VideoAnswerAction[] {
  if (!runId) return [];
  return scene.items.filter(item => item.asset.kind === 'video').flatMap(item => {
    const range = videoRetainedRange(item), document = item.videoAnnotations;
    if (!range || !document || document.sourceId !== item.asset.id) return [];
    return document.objects.flatMap(object => {
      const interval = intersectVideoInterval(object.interval, range);
      return object.origin?.runId === runId && interval ? [{ target: videoAnnotationTarget(scene.id, item), annotationId: object.id, runId, toolEventId: object.origin.toolEventId, interval }] : [];
    });
  }).slice(0, 24);
}
export function currentVideoAnswer(scene: Scene | undefined, action: VideoAnswerAction): SpaceItem | undefined {
  if (!scene || scene.closed || scene.frozen || scene.id !== action.target.sceneId) return;
  const item = scene.items.find(item => item.id === action.target.itemId && item.asset.kind === 'video');
  if (!item || !sameVideoAnnotationTarget(videoAnnotationTarget(scene.id, item), action.target)) return;
  const object = item.videoAnnotations?.objects.find(value => value.id === action.annotationId), range = videoRetainedRange(item);
  if (!object || !range || object.origin?.runId !== action.runId || object.origin.toolEventId !== action.toolEventId) return;
  const interval = intersectVideoInterval(object.interval, range);
  if (interval?.startTicks !== action.interval.startTicks || interval.endTicks !== action.interval.endTicks) return;
  return item;
}
export class VideoPlayerRegistry {
  private entries = new Map<string, VideoPlayerPort>();
  register = (sceneId: string, itemId: string, port: VideoPlayerPort): (() => void) => {
    const key = JSON.stringify([sceneId, itemId]); this.entries.set(key, port);
    return () => { if (this.entries.get(key) === port) this.entries.delete(key); };
  };
  async jump(action: VideoAnswerAction, sourceIdentity: string, active: () => boolean): Promise<void> {
    const key = JSON.stringify([action.target.sceneId, action.target.itemId]), port = this.entries.get(key);
    if (!active() || !port || port.sourceIdentity !== sourceIdentity) throw new TrimCanceled();
    const current = () => active() && this.entries.get(key) === port && port.sourceIdentity === sourceIdentity;
    await port.jump(action, current);
    if (!current()) throw new TrimCanceled();
  }
}
