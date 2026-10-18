// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingStep } from '../contracts';

export interface DrawingHistoryState {
  source: string; revision: number; drawings: Drawing[];
  undo: DrawingStep[]; redo: DrawingStep[];
}
type HistoryOutcome = { ok: true } | { ok: false; error: unknown };
type DrawingIntent = { redo: boolean } | { drawing: Drawing };
interface DrawingEntry { intent: DrawingIntent; before: DrawingHistoryState; after: DrawingHistoryState }
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
  private entries: DrawingEntry[] = [];
  private flight?: Promise<HistoryOutcome>;
  private failure?: Extract<HistoryOutcome, { ok: false }>;
  private faulted = false;
  private stopped = false;
  private ownRevisions = 0;
  private saving?: { entry: DrawingEntry; acknowledged?: DrawingHistoryState };
  constructor(private hooks: {
    read: () => DrawingHistoryState; active: () => boolean;
    save: (redo: boolean, expectedRevision: number) => Promise<void>;
    changed: (preview: Drawing[] | undefined, additions?: Drawing[]) => void;
    completed: (redo: boolean, step: DrawingStep) => void;
    failed: (error: unknown) => void;
    add?: {
      save: (drawing: Drawing, expectedRevision: number) => Promise<void>;
      remember: (drawing: Drawing, expectedRevision: number) => void;
      forget: (drawing: Drawing) => void;
    };
  }, private limit = 16) {}
  pending() { return !!this.flight || this.entries.length > 0; }
  hasHistory() { return this.entries.some(entry => 'redo' in entry.intent); }
  failed() { return this.faulted; }
  capacity() { return !this.stopped && !this.failed() && this.entries.length < this.limit; }
  private capture(state: DrawingHistoryState) { return { ...state, drawings: [...state.drawings], undo: [...state.undo], redo: [...state.redo] }; }
  private state() { return this.entries.at(-1)?.after ?? this.capture(this.hooks.read()); }
  private project(before: DrawingHistoryState, intent: DrawingIntent) {
    if ('redo' in intent) return projectDrawingHistory(before, intent.redo);
    if (before.drawings.some(value => value.id === intent.drawing.id)) throw Error('绘制对象已存在');
    const step: DrawingStep = { index: before.drawings.length, before: null, after: intent.drawing };
    return { ...before, revision: before.revision + 1, drawings: [...before.drawings, intent.drawing], undo: [...before.undo, step].slice(-128), redo: [] };
  }
  private publish() { this.hooks.changed(this.entries.at(-1)?.after.drawings, this.entries.flatMap(entry => 'drawing' in entry.intent ? [entry.intent.drawing] : [])); }
  available(redo: boolean) { return !this.stopped && (redo ? this.state().redo : this.state().undo).length > 0; }
  enqueue(redo: boolean) {
    if (this.stopped || this.entries.length >= this.limit || !this.hooks.active()) return false;
    if (!this.entries.length && !this.flight) { this.failure = undefined; this.faulted = false; }
    if (this.failure !== undefined) return false;
    const before = this.state(), intent = { redo }, after = this.project(before, intent); if (!after) return false;
    this.entries.push({ intent, before, after }); this.publish(); this.pump(); return true;
  }
  enqueueAdd(drawing: Drawing, revisionIdentity: number) {
    if (!this.hooks.add || !this.capacity() || !this.hooks.active() || this.revisionIdentity() !== revisionIdentity) return false;
    const intent = { drawing: structuredClone(drawing) }, before = this.state(), after = this.project(before, intent)!;
    this.hooks.add.remember(intent.drawing, this.hooks.read().revision);
    this.entries.push({ intent, before, after }); this.publish(); this.pump(); return true;
  }
  private same(left: DrawingHistoryState, right: DrawingHistoryState) {
    return left.source === right.source && left.revision === right.revision
      && sameValue(left.drawings, right.drawings)
      && sameValue(left.undo, right.undo) && sameValue(left.redo, right.redo);
  }
  private acknowledged(actual: DrawingHistoryState, entry: DrawingEntry) {
    if ('redo' in entry.intent) return this.same(actual, entry.after);
    // Add can legally compact only an old undo prefix (128 steps / byte budget).
    // Prove the exact append and retained suffix, then replay later intents from
    // that authoritative history instead of guessing Rust's serialized byte size.
    const after = entry.after;
    if (actual.source !== after.source || actual.revision !== after.revision || !sameValue(actual.drawings, after.drawings)
      || !sameValue(actual.redo, after.redo) || !actual.undo.length || actual.undo.length > after.undo.length) return false;
    const offset = after.undo.length - actual.undo.length;
    return actual.undo.every((step, index) => sameValue(step, after.undo[offset + index]));
  }
  private observe() {
    const saving = this.saving;
    if (saving && !saving.acknowledged) {
      const actual = this.hooks.read();
      if (this.acknowledged(actual, saving.entry)) { saving.acknowledged = this.capture(actual); this.ownRevisions++; }
    }
  }
  revisionIdentity() {
    this.observe();
    const actual = this.hooks.read(), expected = this.saving?.acknowledged ?? this.entries[0]?.before;
    if (expected && !this.same(actual, expected)) return NaN;
    return actual.revision - this.ownRevisions;
  }
  private pump() {
    if (this.flight || !this.entries.length) return;
    const flight = Promise.resolve().then(async () => {
      while (this.entries.length) {
        const entry = this.entries[0];
        if (this.stopped || !this.hooks.active() || !this.same(this.hooks.read(), entry.before)) throw Error('绘制来源或历史已变化');
        const saving = { entry, acknowledged: undefined as DrawingHistoryState | undefined }; this.saving = saving;
        try {
          if ('drawing' in entry.intent) {
            this.hooks.add!.remember(entry.intent.drawing, entry.before.revision);
            await this.hooks.add!.save(entry.intent.drawing, entry.before.revision);
          } else await this.hooks.save(entry.intent.redo, entry.before.revision);
          this.observe();
          if (this.stopped || !this.hooks.active() || !saving.acknowledged || !this.same(this.hooks.read(), saving.acknowledged)) throw Error('绘制历史回执不一致');
        } catch (error) {
          this.observe();
          if ('drawing' in entry.intent && saving.acknowledged) this.hooks.add!.forget(entry.intent.drawing);
          throw error;
        } finally { this.saving = undefined; }
        this.entries.shift();
        if ('drawing' in entry.intent) this.hooks.add!.forget(entry.intent.drawing);
        else this.hooks.completed(entry.intent.redo, (entry.intent.redo ? entry.before.redo : entry.before.undo).at(-1)!);
        let before = saving.acknowledged!;
        for (const next of this.entries) {
          const after = this.project(before, next.intent); if (!after) throw Error('绘制历史已变化');
          next.before = before; next.after = after; before = after;
        }
        this.publish();
      }
      return { ok: true } as const;
    }).catch(error => {
      const outcome = { ok: false, error } as const;
      this.failure = outcome; this.faulted = true; this.entries = []; this.publish(); this.hooks.failed(error);
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
