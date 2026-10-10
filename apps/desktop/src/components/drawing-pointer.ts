// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingPoint } from '../contracts';

/** Keep input samples; only the visual publication is limited to a display frame. */
export function drawingFrame(publish: () => void) {
  let frame: number | undefined;
  return {
    request() {
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => { frame = undefined; publish(); });
    },
    cancel() { if (frame !== undefined) cancelAnimationFrame(frame); frame = undefined; },
  };
}

export function drawingPointerSamples(event: PointerEvent): PointerEvent[] {
  // Older WebViews and synthetic events may not expose coalesced input.
  const samples = event.type === 'pointermove' && typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : [];
  if (!samples.length) return [event];
  const last = samples[samples.length - 1];
  // A host may leave the aggregate endpoint out of its underlying samples.
  return last.clientX === event.clientX && last.clientY === event.clientY && last.pressure === event.pressure ? samples : [...samples, event];
}

/** Port render callbacks must receive live properties, rather than a Show snapshot. */
export function liveDrawing(id: string, read: () => Drawing): Drawing {
  return {
    id,
    get kind() { return read().kind; }, get color() { return read().color; },
    get strokeWidth() { return read().strokeWidth; }, get points() { return read().points; },
    get text() { return read().text; }, get fontSize() { return read().fontSize; },
    get origin() { return read().origin; }, get rich() { return read().rich; },
  };
}

export interface PenSegment { path: string }
/** Completed paths keep their identity and SVG d; only a bounded tail is rebuilt. */
export function penSegments(segmentLength = 128) {
  if (!Number.isInteger(segmentLength) || segmentLength < 2) throw Error('Invalid pen segment length');
  let source: DrawingPoint[] | undefined, completed: PenSegment[] = [];
  let tail: PenSegment | undefined, tailLength = 0, endpoint: DrawingPoint | undefined;
  return (points: DrawingPoint[]): PenSegment[] => {
    const start = completed.length * (segmentLength - 1);
    if (source !== points || points.length < start + tailLength || (endpoint && points[start + tailLength - 1] !== endpoint)) {
      source = points; completed = []; tail = undefined; tailLength = 0; endpoint = undefined;
    }
    let offset = completed.length * (segmentLength - 1);
    const path = (from: number, until: number) => {
      let value = `M${points[from].x},${points[from].y}`;
      for (let i = from + 1; i < until; i++) value += ` L${points[i].x},${points[i].y}`;
      return { path: value };
    };
    while (points.length - offset > segmentLength) {
      completed.push(tail && tailLength === segmentLength ? tail : path(offset, offset + segmentLength));
      offset += segmentLength - 1; tail = undefined; tailLength = 0;
    }
    const length = points.length - offset;
    if (length < 2) return completed;
    if (!tail || tailLength !== length || endpoint !== points[points.length - 1]) tail = path(offset, points.length);
    tailLength = length; endpoint = points[points.length - 1];
    return [...completed, tail];
  };
}
