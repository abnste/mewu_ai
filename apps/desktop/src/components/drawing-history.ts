// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingStep } from '../contracts';

export interface DrawingHistoryState {
  source: string; revision: number; drawings: Drawing[];
  undo: DrawingStep[]; redo: DrawingStep[];
}
type HistoryOutcome = { ok: true } | { ok: false; error: unknown };
// History is a JSON contract. Shared nodes are common in local projections; compare
// separate host replies structurally without allocating whole-document JSON strings.
function sameValue(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (left === null || right === null || typeof left !== 'object' || typeof right !== 'object') return false;
  if (Array.isArray(left)) {
    if (!Array.isArray(right) || left.length !== right.length) return false;
    for (let index = 0; index < left.length; index++) if (!sameValue(left[index], right[index])) return false;
    return true;
  }
  if (Array.isArray(right)) return false;
  const a = left as Record<string, unknown>, b = right as Record<string, unknown>;
  let leftSize = 0, rightSize = 0;
  for (const key in a) {
    if (!Object.prototype.hasOwnProperty.call(a, key) || a[key] === undefined) continue;
    leftSize++;
    if (!Object.prototype.hasOwnProperty.call(b, key) || !sameValue(a[key], b[key])) return false;
  }
  for (const key in b) if (Object.prototype.hasOwnProperty.call(b, key) && b[key] !== undefined) rightSize++;
  return leftSize === rightSize;
}
/** The host still owns history. This projection changes only visible geometry. */
export function projectDrawingHistory(state: DrawingHistoryState, redo: boolean): DrawingHistoryState | undefined {
  const step = (redo ? state.redo : state.undo).at(-1); if (!step) return;
  const drawings = [...state.drawings], edits = 'batch' in step ? step.batch : [step];
  for (const edit of redo ? edits : [...edits].reverse()) {
    const from = redo ? edit.before : edit.after, to = redo ? edit.after : edit.before;
    if (!Number.isInteger(edit.index) || edit.index < 0 || edit.index > drawings.length) throw Error('绘制历史已变化');
    if (from) {
      const current = drawings[edit.index];
      if (!current || !sameValue(current, from)) throw Error('绘制历史已变化');
      drawings.splice(edit.index, 1);
    }
    if (to) drawings.splice(edit.index, 0, to);
  }
  return {
    source: state.source, revision: state.revision + 1, drawings,
    undo: redo ? [...state.undo, step] : state.undo.slice(0, -1),
    redo: redo ? state.redo.slice(0, -1) : [...state.redo, step],
  };
}

export class DrawingHistoryLane {
  private entries: { redo: boolean; before: DrawingHistoryState; after: DrawingHistoryState }[] = [];
  private flight?: Promise<HistoryOutcome>;
  private failure?: Extract<HistoryOutcome, { ok: false }>;
  private stopped = false;
  constructor(private hooks: {
    read: () => DrawingHistoryState; active: () => boolean;
    save: (redo: boolean, expectedRevision: number) => Promise<void>;
    changed: (preview: Drawing[] | undefined) => void;
    completed: (redo: boolean, step: DrawingStep) => void;
    failed: (error: unknown) => void;
  }, private limit = 16) {}
  pending() { return !!this.flight || this.entries.length > 0; }
  private state() { return this.entries.at(-1)?.after ?? this.hooks.read(); }
  available(redo: boolean) { return !this.stopped && (redo ? this.state().redo : this.state().undo).length > 0; }
  enqueue(redo: boolean) {
    if (this.stopped || this.entries.length >= this.limit || !this.hooks.active()) return false;
    if (!this.entries.length && !this.flight) this.failure = undefined;
    if (this.failure !== undefined) return false;
    const before = this.state(), after = projectDrawingHistory(before, redo); if (!after) return false;
    this.entries.push({ redo, before, after }); this.hooks.changed(after.drawings); this.pump(); return true;
  }
  private same(left: DrawingHistoryState, right: DrawingHistoryState) {
    return left.source === right.source && left.revision === right.revision
      && sameValue(left.drawings, right.drawings)
      && sameValue(left.undo, right.undo) && sameValue(left.redo, right.redo);
  }
  private pump() {
    if (this.flight || !this.entries.length) return;
    const flight = Promise.resolve().then(async () => {
      while (this.entries.length) {
        const entry = this.entries[0];
        if (this.stopped || !this.hooks.active() || !this.same(this.hooks.read(), entry.before)) throw Error('绘制来源或历史已变化');
        await this.hooks.save(entry.redo, entry.before.revision);
        if (this.stopped || !this.hooks.active() || !this.same(this.hooks.read(), entry.after)) throw Error('绘制历史回执不一致');
        const step = (entry.redo ? entry.before.redo : entry.before.undo).at(-1)!;
        this.entries.shift(); this.hooks.completed(entry.redo, step);
        this.hooks.changed(this.entries.at(-1)?.after.drawings);
      }
      return { ok: true } as const;
    }).catch(error => {
      const outcome = { ok: false, error } as const;
      this.failure = outcome; this.entries = []; this.hooks.changed(undefined); this.hooks.failed(error);
      return outcome;
    }).finally(() => { if (this.flight === flight) this.flight = undefined; });
    this.flight = flight;
  }
  async drain(active: () => boolean = () => true) {
    while (this.flight) {
      const flight = this.flight, outcome = await flight;
      if (!active()) throw Error('绘制编辑已切换');
      // Every waiter owns this settled result even if another waiter already
      // consumed the retained failure or a later user action starts a new flight.
      if (!outcome.ok) { if (this.failure === outcome) this.failure = undefined; throw outcome.error; }
    }
    if (this.failure !== undefined) { const failure = this.failure; this.failure = undefined; throw failure.error; }
  }
  stop() { this.stopped = true; }
}
