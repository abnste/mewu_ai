// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingKind, DrawingPoint, Region } from '../contracts';

/** Match native drawing_render: synthesize oblique for every script, including CJK. */
export const DRAWING_TEXT_SKEW_DEGREES = -12;
export function drawingTextLineTransform(italic: boolean | undefined, baseline: number): string | undefined {
  return italic ? `translate(0 ${baseline}) skewX(${DRAWING_TEXT_SKEW_DEGREES}) translate(0 ${-baseline})` : undefined;
}

let textMeasureContext: CanvasRenderingContext2D | null | undefined;
/** Measure complete candidate lines with the native exporter's shared
 * exponential/binary search policy. Contextual shaping can change earlier
 * advances, so this is not a promise of first-overflow greedy equivalence. */
export function drawingTextLayout(drawing: Drawing, measureText?: (text: string) => number) {
  const size = drawing.fontSize ?? 20, lineHeight = size * 1.25;
  if (textMeasureContext === undefined && typeof document !== 'undefined') textMeasureContext = document.createElement('canvas').getContext('2d');
  const context = textMeasureContext;
  if (context) context.font = `normal ${drawing.textStyle?.bold ? '700' : '400'} ${size}px "Segoe UI", "Microsoft YaHei", sans-serif`;
  const widths = new Map<string, number>();
  const measure = (text: string) => {
    const cached = widths.get(text); if (cached !== undefined) return cached;
    const width = measureText?.(text) ?? context?.measureText(text.replaceAll('\t', '    ')).width ?? [...text].reduce((sum, char) => sum + (char.codePointAt(0)! > 255 ? size : size * .6), 0);
    widths.set(text, width); return width;
  };
  const frame = drawing.kind === 'text' && drawing.points.length === 2 ? { width: drawing.points[1].x - drawing.points[0].x, height: drawing.points[1].y - drawing.points[0].y } : undefined;
  const lines: string[] = [];
  const segmenter = frame ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : undefined;
  // Video's literal renderer expands TAB before finding wrap opportunities.
  // Expand only this display copy; the editable/stored source keeps its TABs.
  for (const paragraph of (drawing.text ?? '').replaceAll('\t', '    ').split('\n')) {
    if (!frame || !paragraph) { lines.push(paragraph); continue; }
    const ends = [...segmenter!.segment(paragraph)].map(part => part.index);
    ends.push(paragraph.length);
    const count = ends.length - 1;
    let start = 0;
    while (start < count) {
      // Match drawing_text_wrap::wrap_lines: an indivisible over-wide grapheme
      // occupies one line. Probe whole lines rather than shaping every prefix.
      let fit = start + 1, step = 1, limit = count;
      while (fit < count) {
        const probe = Math.min(start + step * 2, count);
        if (measure(paragraph.slice(ends[start], ends[probe])) > frame.width + .001) { limit = probe; break; }
        fit = probe; step *= 2;
      }
      while (fit + 1 < limit) {
        const middle = fit + Math.floor((limit - fit) / 2);
        if (measure(paragraph.slice(ends[start], ends[middle])) <= frame.width + .001) fit = middle;
        else limit = middle;
      }
      lines.push(paragraph.slice(ends[start], ends[fit]));
      start = fit;
    }
  }
  const contentWidth = Math.max(1, ...lines.map(measure));
  return { lines, size, lineHeight, contentWidth, width: frame?.width ?? contentWidth, height: Math.max(frame?.height ?? 0, lines.length * lineHeight), framed: !!frame };
}

export function drawingBounds(drawing: Drawing) {
  const points = drawing.kind === 'arrow' ? [...drawing.points, ...(arrowWings(drawing.points) ?? [])] : drawing.points;
  const xs = points.map(point => point.x), ys = points.map(point => point.y);
  let x = Math.min(...xs), y = Math.min(...ys), width = Math.max(...xs) - x, height = Math.max(...ys) - y;
  if (drawing.kind === 'text') {
    const layout = drawingTextLayout(drawing), size = layout.size;
    width = Math.max(layout.width, layout.contentWidth); height = layout.height;
    // Oblique glyphs extend right above the baseline and slightly left below it.
    if (drawing.textStyle?.italic) { const overhang = Math.tan(-DRAWING_TEXT_SKEW_DEGREES * Math.PI / 180) * size; x -= overhang * .25; width += overhang * 1.25; }
  }
  if (drawing.kind === 'number') width = height = drawing.fontSize ?? 28;
  const margin = ['number', 'mosaic', 'rich', 'mark'].includes(drawing.kind) ? 0 : drawing.strokeWidth / 2;
  return { x: x - margin, y: y - margin, width: Math.max(1, width) + margin * 2, height: Math.max(1, height) + margin * 2 };
}
/** Disjoint fill/border paths: original Mark's 84/255 and 190/255 alpha never stack. */
export function markPaths(drawing: Drawing) {
  const [a, b] = drawing.points;
  const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y), width = Math.abs(a.x - b.x), height = Math.abs(a.y - b.y);
  const radius = Math.min(3, width / 2, height / 2), borderWidth = Math.min(drawing.strokeWidth, width / 2, height / 2);
  const path = (x: number, y: number, w: number, h: number, r: number) => w > 0 && h > 0
    ? `M${x + r} ${y}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}v${h - 2 * r}a${r} ${r} 0 0 1 ${-r} ${r}h${2 * r - w}a${r} ${r} 0 0 1 ${-r} ${-r}v${2 * r - h}a${r} ${r} 0 0 1 ${r} ${-r}z` : '';
  const outer = path(x, y, width, height, radius), fill = path(x + borderWidth, y + borderWidth, width - 2 * borderWidth, height - 2 * borderWidth, Math.max(0, radius - borderWidth));
  return { outer, fill, border: outer + fill };
}
export function constrainedEnd(kind: DrawingKind, start: DrawingPoint, end: DrawingPoint, shift: boolean, region: Pick<Region, 'x' | 'y' | 'width' | 'height'>): DrawingPoint {
  if (!shift || !['line', 'arrow', 'rect', 'ellipse', 'mark'].includes(kind)) return end;
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
  if (drawing.kind === 'text' && (drawing.points.length === 1 || drawing.points.length === 2)) {
    const first = drawing.points[0], layout = drawingTextLayout(drawing);
    if (![first.x, first.y, layout.width, layout.height].every(Number.isFinite) || layout.width <= 0 || layout.height <= 0) return [];
    // Keep the text origin fixed. Side handles adjust width/height independently;
    // the lower-right handle adjusts both, without scaling the font.
    return [{ x: first.x + layout.width, y: first.y + layout.height / 2 }, { x: first.x + layout.width / 2, y: first.y + layout.height }, { x: first.x + layout.width, y: first.y + layout.height }];
  }
  if (drawing.points.length !== 2 || !['line', 'arrow', 'rect', 'ellipse', 'mark'].includes(drawing.kind)) return [];
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
  if (drawing.kind === 'text') {
    const layout = drawingTextLayout(drawing), first = drawing.points[0];
    if (first.x >= region.x + region.width || first.y >= region.y + region.height) return;
    const right = handle === 1 ? first.x + layout.width : Math.max(first.x + 1, end.x);
    const bottom = handle === 0 ? first.y + layout.height : Math.max(first.y + 1, end.y);
    // The box is a minimum height. Wrapped text always remains in the document
    // and is painted at its natural height, even when the box is shortened.
    return [{ ...first }, { x: clamp(right, first.x + 1, region.x + region.width), y: clamp(bottom, first.y + 1, region.y + region.height) }];
  }
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

export function arrowWings(points: DrawingPoint[]): [DrawingPoint, DrawingPoint] | undefined {
  const [a, b] = points;
  if (!a || !b || ![a.x, a.y, b.x, b.y].every(Number.isFinite)) return;
  const dx = b.x - a.x, dy = b.y - a.y, distance = Math.hypot(dx, dy);
  if (!distance || !Number.isFinite(distance)) return;
  // Original Mewu: each wing is 1/4 of the shaft, capped at 24, at ±30°.
  const length = Math.min(24, distance * .25), wing = length / 2, back = length * Math.sqrt(3) / 2;
  const ux = dx / distance, uy = dy / distance, x = b.x - ux * back, y = b.y - uy * back;
  return [{x:x - uy * wing, y:y + ux * wing}, {x:x + uy * wing, y:y - ux * wing}];
}
export function arrowHead(points: DrawingPoint[]) {
  const wings = arrowWings(points);
  return wings ? [wings[0], points[1], wings[1]].map(point => `${point.x},${point.y}`).join(' ') : '';
}
export const penPath = (points: DrawingPoint[]) => points.map((point, index) => `${index ? 'L' : 'M'}${point.x},${point.y}`).join(' ');
export function drawingMoveDelta(delta: number, minimum: number, maximum: number, regionStart: number, regionEnd: number, backgroundEnd: number) {
  // Resizing a crop can leave a saved object larger than the crop. Keep those
  // coordinates valid without squeezing the object or changing its shape.
  const low = regionStart - minimum, high = regionEnd - maximum;
  return low <= high ? Math.max(low, Math.min(high, delta)) : Math.max(-minimum, Math.min(backgroundEnd - maximum, delta));
}
