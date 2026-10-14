// SPDX-License-Identifier: MPL-2.0
import type { Asset } from './contracts';
/** Original physical screenshot pixels map directly to native display pixels.
 * CSS innerWidth is integral, so a fractional Windows scale can otherwise make
 * object-fit:contain resample the entire image by a fraction of a pixel. */
export function captureDisplayGeometry(
  background: Asset | undefined,
  width: number,
  height: number,
  viewport: { width: number; height: number },
  deviceScale: number,
) {
  if (!(width > 0 && height > 0)) return { x: 0, y: 0, width: 0, height: 0, scale: 1, nativePixels: false };
  const coordinates = background?.captureCoordinates;
  const nativePixels = coordinates?.space === 'physicalPixels'
    && coordinates.width === width && coordinates.height === height
    && Number.isFinite(deviceScale) && deviceScale > 0
    // The CSS viewport rounds to integral DIP. At most half a DIP of rounding
    // identifies the same physical fullscreen; resized/imported images fall back.
    && Math.abs(viewport.width - width / deviceScale) <= .5
    && Math.abs(viewport.height - height / deviceScale) <= .5;
  const scale = nativePixels ? 1 / deviceScale : Math.min(viewport.width / width, viewport.height / height);
  return { x: nativePixels ? 0 : (viewport.width - width * scale) / 2,
    y: nativePixels ? 0 : (viewport.height - height * scale) / 2,
    width: width * scale, height: height * scale, scale, nativePixels };
}
