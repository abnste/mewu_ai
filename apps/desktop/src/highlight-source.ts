// SPDX-License-Identifier: MPL-2.0
import type { Asset, Drawing, Region } from './contracts';
export interface HighlightTarget {
  regionId: string; sourceId: string; x: number; y: number; width: number; height: number; rasterDrawings: Drawing[];
}
export const rasterDrawing = (drawing: Drawing) => drawing.kind === 'mosaic' || drawing.kind === 'rich' && ['repair', 'extracted'].includes(drawing.rich?.kind ?? '');
export function highlightTarget(background: Asset, region?: Region): HighlightTarget | undefined {
  if (!region) return;
  return { regionId: region.id, sourceId: (region.imageOverride ?? background).id, x: region.x, y: region.y,
    width: region.width, height: region.height, rasterDrawings: structuredClone((region.drawings ?? []).filter(rasterDrawing)) };
}
export function highlightSourceIdentity(sceneId: string, background: Asset, region?: Region): string {
  // The typed footprint excludes editable ink, history, translation and revision.
  return JSON.stringify([sceneId, background, region?.imageOverride ?? null, highlightTarget(background, region)]);
}
export function highlightCrop(background: Asset, region?: Region) {
  const source = region?.imageOverride ?? background, width = source.width ?? 0, height = source.height ?? 0;
  if (!region || region.imageOverride) return { x: 0, y: 0, width, height };
  return { x: Math.round(region.x), y: Math.round(region.y), width: Math.round(region.width), height: Math.round(region.height) };
}
