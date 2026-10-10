// SPDX-License-Identifier: MPL-2.0
import type { Asset, Region } from './contracts';

export interface ImageBox { x: number; y: number; width: number; height: number }
export interface RegionImageProjection { region: Region; box: ImageBox; scale: number; width: number; height: number }
// Persistence keeps the screen selection. Only the renderer/editor uses this virtual region.
export function regionImageProjection(region: Region, box: ImageBox, background: Asset, scale: number): RegionImageProjection {
  const image = region.imageOverride;
  if (!image?.width || !image.height) return { region, box, scale, width: background.width ?? region.x + region.width, height: background.height ?? region.y + region.height };
  const fit = Math.min(box.width / image.width, box.height / image.height);
  const width = image.width * fit, height = image.height * fit;
  return { region: { ...region, x: 0, y: 0, width: image.width, height: image.height },
    box: { x: box.x + (box.width - width) / 2, y: box.y + (box.height - height) / 2, width, height },
    scale: fit, width: image.width, height: image.height };
}
