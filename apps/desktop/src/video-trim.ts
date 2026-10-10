// SPDX-License-Identifier: MPL-2.0
// Source time is integer 100ns ticks from a trusted native probe.
export const TICKS_PER_SECOND = 10_000_000;
export const MIN_RANGE_TICKS = 1_000_000;
export interface VideoRange { startTicks: number; endTicks: number }
export type TrimPart = 'start' | 'end' | 'seek';
export interface TrimAuthority { identity: string; durationTicks: number; revision: number; range: VideoRange | null; editable: boolean }
export interface TrimEdit { identity: string; durationTicks: number; expectedRevision: number; from: VideoRange | null; to: VideoRange | null }
export interface TrimReceipt { identity: string; durationTicks: number; revision: number; range: VideoRange | null }
export interface TrimView { range: VideoRange; positionTicks: number; part?: TrimPart; pending: boolean }
export class TrimCanceled extends Error { constructor() { super('视频目标已变化'); } }
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
export function assertDuration(duration: number) { if (!Number.isSafeInteger(duration) || duration <= 0) throw Error('无效视频时长'); }
export function fullRange(duration: number): VideoRange { assertDuration(duration); return { startTicks: 0, endTicks: duration }; }
export function sameRange(a: VideoRange | null, b: VideoRange | null) { return a === null || b === null ? a === b : a.startTicks === b.startTicks && a.endTicks === b.endTicks; }
export function checkedRange(duration: number, range: VideoRange | null): VideoRange {
  assertDuration(duration); const value = range ?? fullRange(duration);
  if (![value.startTicks, value.endTicks].every(Number.isSafeInteger) || value.startTicks < 0 || value.endTicks > duration || value.endTicks - value.startTicks < Math.min(MIN_RANGE_TICKS, duration)) throw Error('无效视频范围');
  return { ...value };
}
export function canonicalRange(duration: number, range: VideoRange): VideoRange | null { const value = checkedRange(duration, range); return value.startTicks === 0 && value.endTicks === duration ? null : value; }
export function clampPosition(position: number, range: VideoRange) { if (!Number.isFinite(position)) throw Error('无效播放位置'); return clamp(Math.round(position), range.startTicks, range.endTicks); }
export function moveBoundary(duration: number, range: VideoRange, part: Exclude<TrimPart, 'seek'>, ticks: number): VideoRange {
  checkedRange(duration, range); if (!Number.isFinite(ticks)) throw Error('无效裁切位置'); const minimum = Math.min(MIN_RANGE_TICKS, duration);
  return part === 'start' ? { startTicks: clamp(Math.round(ticks), 0, range.endTicks - minimum), endTicks: range.endTicks } : { startTicks: range.startTicks, endTicks: clamp(Math.round(ticks), range.startTicks + minimum, duration) };
}
export function positionFromClient(clientX: number, railLeft: number, railWidth: number, duration: number) {
  assertDuration(duration); if (![clientX, railLeft, railWidth].every(Number.isFinite) || railWidth <= 0) throw Error('无效时间轴');
  return Math.round(clamp((clientX - railLeft) / railWidth, 0, 1) * duration);
}
export function keyboardPosition(key: string, shift: boolean, current: number, duration: number): number | undefined {
  assertDuration(duration); const step = shift ? TICKS_PER_SECOND : MIN_RANGE_TICKS;
  if (key === 'Home') return 0; if (key === 'End') return duration;
  if (key === 'ArrowLeft' || key === 'ArrowDown') return Math.max(0, current - step);
  if (key === 'ArrowRight' || key === 'ArrowUp') return Math.min(duration, current + step);
}
export function formatTicks(value: number) {
  if (!Number.isFinite(value)) return '00:00.0'; const tenths = Math.floor(Math.max(0, value) / 1_000_000), seconds = Math.floor(tenths / 10), minutes = Math.floor(seconds / 60), hours = Math.floor(minutes / 60);
  return `${hours ? `${hours}:` : ''}${String(minutes % 60).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}.${tenths % 10}`;
}
interface Gesture { authority: TrimAuthority; part: TrimPart; mode: 'pointer' | 'keyboard' | 'action'; range: VideoRange; beforePosition: number; position: number; wasPlaying: boolean }
interface Hooks {
  current: () => TrimAuthority | undefined;
  position: () => number;
  playing: () => boolean;
  pause: () => void;
  seek: (position: number) => void;
  resume: (position: number) => void;
  commit: (edit: TrimEdit) => Promise<TrimReceipt>;
  changed: (view: TrimView | undefined) => void;
  error: (error: unknown) => void;
}
/** One local gesture and one acknowledged range commit, never an unbounded command queue. */
export class VideoTrimGesture {
  private gesture?: Gesture;
  private accepted?: Gesture;
  private flight?: Promise<boolean>;
  private disposed = false;
  private held = new Set<string>();
  constructor(private hooks: Hooks) {}
  isInteracting() { return Boolean(this.gesture); }
  isPending() { return Boolean(this.flight); }
  private sameTarget(before: TrimAuthority, revision = true) {
    const now = this.hooks.current();
    return Boolean(now && now.identity === before.identity && now.durationTicks === before.durationTicks && (!revision || now.revision === before.revision));
  }
  private publish() {
    if (this.disposed) return; const state = this.hooks.current();
    if (!state) { this.hooks.changed(undefined); return; }
    const gesture = this.gesture ?? (this.accepted && this.sameTarget(this.accepted.authority) ? this.accepted : undefined);
    this.hooks.changed({ range: gesture ? { ...gesture.range } : checkedRange(state.durationTicks, state.range), positionTicks: gesture?.position ?? clampPosition(this.hooks.position(), checkedRange(state.durationTicks, state.range)), part: gesture?.part, pending: Boolean(this.flight) });
  }
  begin(part: TrimPart, mode: Gesture['mode'] = 'pointer'): boolean {
    const state = this.hooks.current();
    if (this.disposed || this.gesture || this.flight || !state || (part !== 'seek' && !state.editable)) return false;
    const range = checkedRange(state.durationTicks, state.range), position = clampPosition(this.hooks.position(), range);
    this.gesture = { authority: { ...state, range: state.range && { ...state.range } }, part, mode, range, beforePosition: position, position, wasPlaying: this.hooks.playing() };
    this.hooks.pause(); this.publish(); return true;
  }
  move(ticks: number): boolean {
    const value = this.gesture;
    if (!value || !this.sameTarget(value.authority) || (value.part !== 'seek' && !this.hooks.current()?.editable)) { this.cancel(false); return false; }
    if (value.part === 'seek') value.position = clampPosition(ticks, value.range);
    else { value.range = moveBoundary(value.authority.durationTicks, value.range, value.part, ticks); value.position = value.part === 'start' ? value.range.startTicks : value.range.endTicks; }
    this.hooks.seek(value.position); this.publish(); return true;
  }
  keyDown(part: TrimPart, key: string, shift: boolean): boolean {
    const state = this.hooks.current(); if (!state || keyboardPosition(key, shift, 0, state.durationTicks) === undefined) return false;
    if (!this.gesture && !this.begin(part, 'keyboard')) return false;
    const value = this.gesture!; if (value.mode !== 'keyboard' || value.part !== part) return false;
    this.held.add(key); const current = part === 'start' ? value.range.startTicks : part === 'end' ? value.range.endTicks : value.position;
    return this.move(keyboardPosition(key, shift, current, state.durationTicks)!);
  }
  keyUp(key: string): Promise<boolean> | undefined { if (!this.held.delete(key) || this.held.size || this.gesture?.mode !== 'keyboard') return; return this.complete(); }
  async setBoundary(part: Exclude<TrimPart, 'seek'>) { if (!this.begin(part, 'action')) return false; this.move(this.gesture!.beforePosition); return this.complete(); }
  async reset() {
    if (!this.begin('start', 'action')) return false; const value = this.gesture!;
    value.range = fullRange(value.authority.durationTicks); value.position = clampPosition(value.position, value.range); this.publish(); return this.complete();
  }
  complete(): Promise<boolean> {
    const value = this.gesture; if (!value || this.disposed) return Promise.resolve(false);
    this.gesture = undefined; this.held.clear();
    if (!this.sameTarget(value.authority) || (value.part !== 'seek' && !this.hooks.current()?.editable)) { this.publish(); return Promise.resolve(false); }
    const to = canonicalRange(value.authority.durationTicks, value.range);
    if (value.part === 'seek' || sameRange(to, value.authority.range)) { this.publish(); return Promise.resolve(true); }
    const edit: TrimEdit = { identity: value.authority.identity, durationTicks: value.authority.durationTicks, expectedRevision: value.authority.revision, from: value.authority.range, to };
    let committed = false;
    // Keep ownership through the actual commit Promise. A canceled view cannot cancel a saved transaction.
    this.accepted = value;
    const flight = Promise.resolve().then(() => {
      if (this.disposed || !this.sameTarget(value.authority) || !this.hooks.current()?.editable) throw new TrimCanceled();
      return this.hooks.commit(edit);
    }).then(receipt => {
      if (receipt.identity !== edit.identity || receipt.durationTicks !== edit.durationTicks || receipt.revision !== edit.expectedRevision + 1 || !sameRange(receipt.range, edit.to)) throw Error('视频范围回执不一致');
      committed = true;
      return !this.disposed && this.sameTarget(value.authority, false);
    }).catch(error => { if (!(error instanceof TrimCanceled) && !this.disposed && this.sameTarget(value.authority, false)) this.hooks.error(error); return false; }).finally(() => {
      if (this.flight !== flight) return;
      this.flight = undefined; this.accepted = undefined;
      if (!this.disposed) {
        const current = this.hooks.current();
        if (current && this.sameTarget(value.authority, false)) {
          const ownCommit = committed && current.revision === edit.expectedRevision + 1 && sameRange(current.range, edit.to);
          const ownFailure = !committed && current.revision === edit.expectedRevision;
          if (ownCommit || ownFailure) this.hooks.seek(clampPosition(ownCommit ? value.position : value.beforePosition, checkedRange(current.durationTicks, current.range)));
        }
        this.publish();
      }
    });
    this.flight = flight; this.publish(); return flight;
  }
  cancel(restorePlayback = true) {
    const value = this.gesture; this.gesture = undefined; this.held.clear();
    if (value && this.sameTarget(value.authority) && !this.disposed) { this.hooks.seek(value.beforePosition); if (restorePlayback && value.wasPlaying) this.hooks.resume(value.beforePosition); }
    this.publish();
  }
  reconcile() {
    const value = this.gesture;
    if (value && (!this.sameTarget(value.authority) || (value.part !== 'seek' && !this.hooks.current()?.editable))) this.cancel(false); else this.publish();
  }
  /** Transition/exit cancels an unfinished gesture, but must wait for an accepted commit. */
  async flush(active: () => boolean = () => true) {
    this.cancel(false); const flight = this.flight;
    const success = !flight || await flight;
    if (!active() || this.disposed) throw new TrimCanceled();
    if (!success) throw Error('视频范围未保存');
  }
  dispose() { if (this.disposed) return; this.cancel(false); this.disposed = true; }
}

export interface SeekIntent { identity: string; ticks: number }
/** Serialized adapter seam; work resolves only after the actual owned seek completes. */
export class LatestVideoSeek {
  private latest?: { value: SeekIntent; epoch: number };
  private flight?: Promise<void>;
  private epoch = 0;
  private cancelEpoch = 0;
  private disposed = false;
  private error?: unknown;
  constructor(private hooks: { currentIdentity: () => string | undefined; work: (value: SeekIntent) => Promise<number>; presented: (ticks: number) => void; error: (error: unknown) => void }) {}
  request(value: SeekIntent) {
    if (this.disposed || value.identity !== this.hooks.currentIdentity() || !Number.isSafeInteger(value.ticks) || value.ticks < 0) return false;
    this.latest = { value: { ...value }, epoch: ++this.epoch }; this.error = undefined; this.drain(); return true;
  }
  private drain() {
    const request = this.latest; if (this.disposed || this.flight || !request) return;
    this.latest = undefined;
    const flight = Promise.resolve().then(() => !this.disposed && request.epoch === this.epoch && request.value.identity === this.hooks.currentIdentity() ? this.hooks.work(request.value) : Promise.reject(new TrimCanceled())).then(actual => {
      if (!this.disposed && request.epoch === this.epoch && request.value.identity === this.hooks.currentIdentity()) this.hooks.presented(actual);
    }).catch(error => {
      if (!this.disposed && request.epoch === this.epoch && request.value.identity === this.hooks.currentIdentity()) { this.error = error; this.hooks.error(error); }
    }).finally(() => { if (this.flight === flight) { this.flight = undefined; this.drain(); } });
    this.flight = flight;
  }
  cancel() { this.cancelEpoch++; this.epoch++; this.latest = undefined; this.error = undefined; }
  async flush(active: () => boolean = () => true) {
    const generation = this.cancelEpoch;
    while (this.flight || this.latest) { if (this.disposed || !active() || this.cancelEpoch !== generation) throw new TrimCanceled(); const flight = this.flight; if (flight) await flight; else this.drain(); }
    if (this.disposed || !active() || this.cancelEpoch !== generation) throw new TrimCanceled(); if (this.error !== undefined) throw this.error;
  }
  dispose() { this.cancel(); this.disposed = true; }
}
