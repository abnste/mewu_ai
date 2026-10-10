// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingKind, DrawingPoint, Region } from '../contracts';

export function drawingBounds(drawing: Drawing) {
  const xs = drawing.points.map(point => point.x), ys = drawing.points.map(point => point.y);
  let x = Math.min(...xs), y = Math.min(...ys), width = Math.max(...xs) - x, height = Math.max(...ys) - y;
  if (drawing.kind === 'text') {
    const lines = (drawing.text ?? '').split('\n'), size = drawing.fontSize ?? 20;
    width = Math.max(...lines.map(line => [...line].reduce((sum, char) => sum + (char.charCodeAt(0) > 255 ? 1 : .6), 0))) * size;
    height = Math.max(1, lines.length) * size * 1.25;
  }
  if (drawing.kind === 'number') width = height = drawing.fontSize ?? 28;
  const margin = ['number', 'mosaic', 'rich'].includes(drawing.kind) ? 0 : drawing.strokeWidth / 2;
  return { x: x - margin, y: y - margin, width: Math.max(1, width) + margin * 2, height: Math.max(1, height) + margin * 2 };
}
export function constrainedEnd(kind: DrawingKind, start: DrawingPoint, end: DrawingPoint, shift: boolean, region: Pick<Region, 'x' | 'y' | 'width' | 'height'>): DrawingPoint {
  if (!shift || !['line', 'arrow', 'rect', 'ellipse'].includes(kind)) return end;
  const dx = end.x - start.x, dy = end.y - start.y, ax = Math.abs(dx), ay = Math.abs(dy);
  let ux = dx < 0 ? -1 : 1, uy = dy < 0 ? -1 : 1, distance = Math.max(ax, ay);
  if (kind === 'line' || kind === 'arrow') {
    const threshold = Math.SQRT2 - 1;
    if (ay <= ax * threshold) { uy = 0; distance = ax; }
    else if (ax <= ay * threshold) { ux = 0; distance = ay; }
    else distance = (ax + ay) / 2;
  }
  if (ux) distance = Math.min(distance, ux > 0 ? region.x + region.width - start.x : start.x - region.x);
  if (uy) distance = Math.min(distance, uy > 0 ? region.y + region.height - start.y : start.y - region.y);
  return { x: start.x + ux * distance, y: start.y + uy * distance };
}
export function drawingResizeHandles(drawing: Drawing): DrawingPoint[] {
  if (drawing.points.length !== 2 || !['line', 'arrow', 'rect', 'ellipse'].includes(drawing.kind)) return [];
  const [a, b] = drawing.points;
  if (![a.x, a.y, b.x, b.y].every(Number.isFinite)) return [];
  if (drawing.kind === 'line' || drawing.kind === 'arrow') return [a, b];
  const left = Math.min(a.x, b.x), right = Math.max(a.x, b.x), top = Math.min(a.y, b.y), bottom = Math.max(a.y, b.y);
  return [{ x: left, y: top }, { x: right, y: top }, { x: right, y: bottom }, { x: left, y: bottom }];
}
export function resizedDrawingPoints(drawing: Drawing, handle: number, pointer: DrawingPoint, shift: boolean, region: Pick<Region, 'x' | 'y' | 'width' | 'height'>): DrawingPoint[] | undefined {
  const handles = drawingResizeHandles(drawing);
  if (!Number.isInteger(handle) || !handles[handle] || ![pointer.x, pointer.y, region.x, region.y, region.width, region.height].every(Number.isFinite) || region.width <= 0 || region.height <= 0) return;
  const [a, b] = drawing.points;
  const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
  const end = { x: clamp(pointer.x, region.x, region.x + region.width), y: clamp(pointer.y, region.y, region.y + region.height) };
  if (drawing.kind === 'line' || drawing.kind === 'arrow') {
    const anchor = handle === 0 ? b : a;
    const adjusted = constrainedEnd(drawing.kind, anchor, shift ? pointer : end, shift, region);
    if (Math.hypot(adjusted.x - anchor.x, adjusted.y - anchor.y) < 1) return;
    return handle === 0 ? [adjusted, { ...b }] : [{ ...a }, adjusted];
  }
  const left = handle === 0 || handle === 3, top = handle === 0 || handle === 1;
  const anchor = handles[(handle + 2) % 4];
  let x = left ? Math.min(end.x, Math.max(region.x, anchor.x - 2)) : Math.max(end.x, Math.min(region.x + region.width, anchor.x + 2));
  let y = top ? Math.min(end.y, Math.max(region.y, anchor.y - 2)) : Math.max(end.y, Math.min(region.y + region.height, anchor.y + 2));
  if (shift) {
    // Preserve the original corner's quadrant; crossing the fixed anchor never flips it.
    const dx = left ? -1 : 1, dy = top ? -1 : 1;
    const limit = Math.min(left ? anchor.x - region.x : region.x + region.width - anchor.x, top ? anchor.y - region.y : region.y + region.height - anchor.y);
    if (limit <= 0) return;
    const side = clamp(Math.max((pointer.x - anchor.x) * dx, (pointer.y - anchor.y) * dy), Math.min(2, limit), limit);
    x = anchor.x + dx * side; y = anchor.y + dy * side;
  }
  const lowX = Math.min(x, anchor.x), highX = Math.max(x, anchor.x), lowY = Math.min(y, anchor.y), highY = Math.max(y, anchor.y);
  if (highX - lowX < 1 || highY - lowY < 1) return;
  // Keep the two original control points' orientation as well as the fixed corner.
  return [a, b].map(point => ({ x: point.x <= (a.x + b.x) / 2 ? lowX : highX, y: point.y <= (a.y + b.y) / 2 ? lowY : highY }));
}
export const rasterDrawing = (drawing:Drawing) => drawing.rich?.kind === 'repair' || drawing.rich?.kind === 'extracted';
export const drawingOrder = (drawings: Drawing[]) => [...drawings.filter(value => value.kind === 'mosaic'), ...drawings.filter(rasterDrawing), ...drawings.filter(value => value.kind !== 'mosaic' && !rasterDrawing(value))];
export function validNextNumber(value: string) { return /^[1-9]\d{0,3}$/.test(value) ? Number(value) : undefined; }

export function arrowHead(points: DrawingPoint[], strokeWidth: number) {
  const [a, b] = points;
  if (!a || !b) return '';
  const dx = b.x - a.x, dy = b.y - a.y, distance = Math.hypot(dx, dy);
  if (!distance) return '';
  const length = Math.min(distance, Math.max(strokeWidth * 4, 12)), wing = length * .48;
  const ux = dx / distance, uy = dy / distance, x = b.x - ux * length, y = b.y - uy * length;
  return `${b.x},${b.y} ${x - uy * wing},${y + ux * wing} ${x + uy * wing},${y - ux * wing}`;
}
export const penPath = (points: DrawingPoint[]) => points.map((point, index) => `${index ? 'L' : 'M'}${point.x},${point.y}`).join(' ');
export function drawingMoveDelta(delta: number, minimum: number, maximum: number, regionStart: number, regionEnd: number, backgroundEnd: number) {
  // Resizing a crop can leave a saved object larger than the crop. Keep those
  // coordinates valid without squeezing the object or changing its shape.
  const low = regionStart - minimum, high = regionEnd - maximum;
  return low <= high ? Math.max(low, Math.min(high, delta)) : Math.max(-minimum, Math.min(backgroundEnd - maximum, delta));
}
