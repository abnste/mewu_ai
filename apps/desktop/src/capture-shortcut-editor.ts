// SPDX-License-Identifier: MPL-2.0
import type { CaptureShortcutState, ShortcutChord, ShortcutEditLease, ShortcutLetter, ShortcutRecorded } from './shortcut-contracts';
export const defaultCaptureShortcut: ShortcutChord = { code: 'KeyS', ctrl: false, shift: true, alt: true };
export interface ShortcutDraft { shortcut: ShortcutChord | null; expectedRevision: number }
export function equalShortcut(a: ShortcutChord | null, b: ShortcutChord | null): boolean {
  return a === b || Boolean(a && b && a.code === b.code && a.ctrl === b.ctrl && a.shift === b.shift && a.alt === b.alt);
}
export function validShortcut(value: unknown): value is ShortcutChord {
  if (!value || typeof value !== 'object') return false;
  const chord = value as ShortcutChord;
  return /^Key[A-Z]$/.test(chord.code) && [chord.ctrl, chord.shift, chord.alt].every(value => typeof value === 'boolean') && (chord.ctrl || chord.shift || chord.alt);
}
export function formatShortcut(value: ShortcutChord | null): string {
  return value ? [value.ctrl && 'Ctrl', value.shift && 'Shift', value.alt && 'Alt', value.code.slice(3)].filter(Boolean).join(' + ') : '未设置';
}
/** DOM code is physical. Native's KeyA enum means VK_A on Windows. */
export function shortcutFromKey(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey' | 'repeat' | 'isComposing' | 'keyCode'>): ShortcutChord | null | undefined {
  if (event.repeat || event.isComposing || event.keyCode === 229 || event.metaKey) return;
  if (event.key === 'Delete') return null;
  if (!/^[a-z]$/i.test(event.key) || (!event.ctrlKey && !event.shiftKey && !event.altKey)) return;
  return { code: `Key${event.key.toUpperCase() as ShortcutLetter}`, ctrl: event.ctrlKey, shift: event.shiftKey, alt: event.altKey };
}
export function acceptShortcutState(current: CaptureShortcutState | undefined, next: CaptureShortcutState): CaptureShortcutState {
  return current && (next.sequence < current.sequence || next.revision < current.revision) ? current : next;
}
/** A successful commit may be overtaken by another window; never adopt its CAS. */
export function settleShortcutSave(submitted: ShortcutDraft, currentDraft: ShortcutDraft | undefined, receipt: CaptureShortcutState, latest: CaptureShortcutState): { draft?: ShortcutDraft; conflict: boolean } {
  if (currentDraft !== submitted) return { draft: currentDraft, conflict: Boolean(currentDraft && currentDraft.expectedRevision !== latest.revision) };
  if (latest.revision > receipt.revision || !equalShortcut(receipt.configured, submitted.shortcut)) return { draft: { ...submitted, expectedRevision: receipt.revision }, conflict: true };
  return { conflict: false };
}

interface Cleanup { promise: Promise<void>; retryId?: string }
interface Token { id: string; ready: Promise<ShortcutEditLease>; lease?: ShortcutEditLease; failedCleanup?: Cleanup }
/** One owner lease; late begin/end/recorded events cannot take over a new editor. */
export class ShortcutLeaseController {
  private token?: Token;
  private cleanup: Cleanup = { promise: Promise.resolve() };
  private flushFlight?: Promise<void>;
  private disposed = false;
  constructor(private hooks: { active: () => boolean; begin: (leaseId: string) => Promise<ShortcutEditLease>; end: (leaseId: string) => Promise<void>; changed: (state: 'idle' | 'arming' | 'armed') => void; record: (shortcut: ShortcutChord) => void; error: (error: unknown) => void; id?: () => string }) {}
  armed(): boolean { return Boolean(this.token?.lease && !this.disposed && this.hooks.active()); }
  async arm(): Promise<void> {
    if (this.disposed || this.token || !this.hooks.active()) return;
    const token: Token = { id: this.hooks.id?.() ?? crypto.randomUUID(), ready: undefined! };
    const previous = this.cleanup;
    this.token = token; this.hooks.changed('arming');
    token.ready = (this.flushFlight ?? this.clean(previous, previous.promise)).catch(error => {
      token.failedCleanup = previous; throw error;
    }).then(() => {
      if (this.disposed || this.token !== token || !this.hooks.active()) throw new Error('快捷键录入已结束');
      return this.hooks.begin(token.id);
    });
    try {
      const lease = await token.ready;
      if (this.disposed || this.token !== token) return; // disarm owns the matching end.
      if (!this.hooks.active()) { await this.disarm(); return; }
      if (lease.leaseId !== token.id || !Number.isFinite(lease.expiresAt)) throw new Error('快捷键录入状态无效');
      token.lease = lease;
      this.hooks.changed('armed');
    } catch (error) {
      if (!this.disposed && this.token === token) { void this.disarm().catch(this.hooks.error); this.hooks.error(error); }
    }
  }
  record(value: ShortcutRecorded) { if (this.token?.id === value.leaseId && this.armed() && validShortcut(value.shortcut)) this.hooks.record({ ...value.shortcut }); }
  ended(leaseId: string) { if (this.token?.id === leaseId) void this.disarm().catch(this.hooks.error); }
  disarm(): Promise<void> {
    const token = this.token;
    if (!token) return this.cleanup.promise;
    this.token = undefined; if (!this.disposed) this.hooks.changed('idle');
    const cleanup: Cleanup = { promise: undefined! };
    cleanup.promise = token.ready.then(() => {
      cleanup.retryId = token.id; return this.hooks.end(token.id);
    }, error => {
      // A canceled queued token never owned a lease. Its predecessor still may:
      // preserve that exact cleanup debt, even through several focus changes.
      if (token.failedCleanup) { cleanup.retryId = token.failedCleanup.retryId; throw error; }
    });
    this.cleanup = cleanup; void cleanup.promise.catch(() => {});
    return cleanup.promise;
  }
  private clean(cleanup: Cleanup, observed: Promise<void>): Promise<void> {
    return observed.catch(error => {
      // Another explicit focus/save may already be retrying the same debt.
      if (cleanup.promise !== observed) return cleanup.promise;
      if (!cleanup.retryId) throw error;
      const id = cleanup.retryId;
      cleanup.promise = Promise.resolve().then(() => this.hooks.end(id));
      void cleanup.promise.catch(() => {});
      return cleanup.promise;
    });
  }
  flush(): Promise<void> {
    if (this.flushFlight) return this.flushFlight;
    const pending = this.disarm(), cleanup = this.cleanup;
    const result = this.clean(cleanup, pending).finally(() => { if (this.flushFlight === result) this.flushFlight = undefined; });
    this.flushFlight = result; return result;
  }
  dispose() { if (this.disposed) return; const ended = this.disarm(); this.disposed = true; void ended.catch(() => {}); }
}
