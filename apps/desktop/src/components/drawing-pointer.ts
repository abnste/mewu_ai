// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingPoint } from '../contracts';

export interface DrawingQueueSnapshot { source: string; revision: number; drawings: Drawing[] }
/** Only new vector strokes use this lane. Existing-object edits keep their CAS. */
export class DrawingAddQueue {
  private entries: Drawing[] = [];
  private expected?: DrawingQueueSnapshot;
  private saving?: { drawing: Drawing; before: DrawingQueueSnapshot; acknowledged: boolean };
  private flight?: Promise<void>;
  private failure: unknown;
  private stopped = false;
  private ownRevisions = 0;
  constructor(private hooks: {
    read: () => DrawingQueueSnapshot;
    active: () => boolean;
    save: (drawing: Drawing, expectedRevision: number) => Promise<void>;
    remember: (drawing: Drawing, expectedRevision: number) => void;
    forget: (drawing: Drawing) => void;
    changed: (drawings: Drawing[]) => void;
    failed: (error: unknown) => void;
  }, private limit = 16) {}
  private same(left: DrawingQueueSnapshot, right: DrawingQueueSnapshot) {
    return left.source === right.source && left.revision === right.revision && JSON.stringify(left.drawings) === JSON.stringify(right.drawings);
  }
  private observe() {
    const saving = this.saving;
    if (!saving || saving.acknowledged) return;
    const after = { ...saving.before, revision: saving.before.revision + 1, drawings: [...saving.before.drawings, saving.drawing] };
    // A publish may arrive before its IPC reply. Only this exact own append can
    // advance a gesture's revision token; unrelated edits still cancel it.
    if (this.same(this.hooks.read(), after)) {
      saving.acknowledged = true; this.expected = after; this.ownRevisions++;
    }
  }
  revisionIdentity() { this.observe(); return this.hooks.read().revision - this.ownRevisions; }
  pending() { return this.entries.length > 0 || !!this.flight; }
  failed() { return this.failure !== undefined; }
  capacity() { return !this.stopped && !this.failed() && this.entries.length < this.limit; }
  enqueue(drawing: Drawing, revisionIdentity: number): boolean {
    if (!this.capacity() || !this.hooks.active() || this.revisionIdentity() !== revisionIdentity) return false;
    const current = this.hooks.read();
    if (!this.entries.length && !this.saving) this.expected = { ...current, drawings: structuredClone(current.drawings) };
    if (!this.expected || current.source !== this.expected.source) return false;
    const value = structuredClone(drawing);
    try { this.hooks.remember(value, this.expected.revision); }
    catch (error) { this.failure = error; this.hooks.failed(error); return false; }
    this.entries.push(value); this.hooks.changed([...this.entries]); this.pump(); return true;
  }
  private pump() {
    if (this.flight || this.stopped || this.failed() || !this.entries.length) return;
    const flight = Promise.resolve().then(async () => {
      while (this.entries.length && !this.stopped) {
        if (!this.hooks.active() || !this.expected || !this.same(this.hooks.read(), this.expected)) throw Error('绘制来源或笔迹已变化');
        const drawing = this.entries[0], before = this.expected;
        this.hooks.remember(drawing, before.revision);
        const saving = { drawing, before, acknowledged: false }; this.saving = saving;
        try {
          await this.hooks.save(drawing, before.revision);
          this.observe();
          if (!saving.acknowledged) throw Error('绘制保存回执不一致');
        } catch (error) {
          // An authoritative publish can prove persistence even if a later IPC
          // delivery fails. Never offer that same ID as an unsaved new stroke.
          this.observe();
          if (saving.acknowledged) { this.hooks.forget(drawing); this.entries.shift(); }
          throw error;
        } finally { this.saving = undefined; }
        this.hooks.forget(drawing); this.entries.shift(); this.hooks.changed([...this.entries]);
      }
      if (this.stopped && this.entries.length) throw Error('绘制编辑已切换');
    }).catch(error => {
      this.failure = error; this.hooks.changed([...this.entries]); this.hooks.failed(error);
    }).finally(() => { if (this.flight === flight) this.flight = undefined; });
    this.flight = flight;
  }
  async drain(active: () => boolean = () => true) {
    while (this.flight) { const flight = this.flight; await flight; if (!active()) throw Error('绘制编辑已切换'); }
    if (this.failure !== undefined) throw this.failure;
    if (this.entries.length) throw Error('标注尚未保存');
  }
  stop() { this.stopped = true; }
}

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
    get textStyle() { return read().textStyle; },
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
    let reusable: PenSegment[] | undefined;
    if (source !== points || points.length < start + tailLength || (endpoint && points[start + tailLength - 1] !== endpoint)) {
      // A native JSON reply owns a new points array. Rebuild its real paths,
      // then retain objects only when their immutable SVG d is exactly equal.
      // Do not compare old points: callers may have mutated that array already.
      reusable = tail ? [...completed, tail] : completed;
      source = points; completed = []; tail = undefined; tailLength = 0; endpoint = undefined;
    }
    let offset = completed.length * (segmentLength - 1);
    const path = (from: number, until: number) => {
      let value = `M${points[from].x},${points[from].y}`;
      for (let i = from + 1; i < until; i++) value += ` L${points[i].x},${points[i].y}`;
      const previous = reusable?.[completed.length];
      return previous?.path === value ? previous : { path: value };
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
