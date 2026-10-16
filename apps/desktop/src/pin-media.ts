// SPDX-License-Identifier: MPL-2.0
import type { VideoRange } from './contracts';
export interface PinVideoMedia {
  kind: 'video'; sourceId: string; sourceDurationTicks: number | null; range: VideoRange | null; annotationRevision: number;
  annotations: { id: string; interval: VideoRange; bounds: { x: number; y: number; width: number; height: number }; imageUrl: string }[];
}
const interval = (value: VideoRange) => value && Number.isSafeInteger(value.startTicks) && Number.isSafeInteger(value.endTicks) && value.startTicks >= 0 && value.endTicks > value.startTicks;
export function validPinMedia(value: PinVideoMedia | undefined, sourceUrl: string): boolean {
  if (value === undefined) return true;
  try {
    const origin = new URL(sourceUrl).origin;
    return value?.kind === 'video' && typeof value.sourceId === 'string' && /^[a-f0-9-]{36}$/.test(value.sourceId)
      && (value.sourceDurationTicks === null || Number.isSafeInteger(value.sourceDurationTicks) && value.sourceDurationTicks > 0)
      && (value.range === null || interval(value.range) && value.sourceDurationTicks !== null && value.range.endTicks <= value.sourceDurationTicks)
      && Number.isSafeInteger(value.annotationRevision) && value.annotationRevision >= 0 && Array.isArray(value.annotations) && value.annotations.length <= 48 && new Set(value.annotations.map(entry => entry.id)).size === value.annotations.length
      && value.annotations.every(entry => {
        const url = new URL(entry.imageUrl), bounds = entry.bounds;
        return typeof entry.id === 'string' && interval(entry.interval) && (value.sourceDurationTicks === null || entry.interval.endTicks <= value.sourceDurationTicks) && url.origin === origin && !url.username && !url.password && !url.search && !url.hash
          && [bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite) && bounds.width > 0 && bounds.height > 0;
      });
  } catch { return false; }
}
