// SPDX-License-Identifier: MPL-2.0
import { TrimCanceled } from './video-trim';
type VideoRequestKind = 'info' | 'export' | 'copy' | 'text-read' | 'text-edit';
const replaceable = (kind: VideoRequestKind) => kind === 'info' || kind === 'text-read';
interface Ticket<T> { id: string; kind: VideoRequestKind; run: (id: string) => Promise<T>; resolve: (value: T) => void; reject: (error: unknown) => void; canceled: boolean; succeeded?: boolean; cleanup?: Promise<void>; cleanupError?: unknown; done?: Promise<void> }
export interface VideoRequest<T> { id: string; promise: Promise<T>; cancel: () => Promise<void> }
/** One real worker/cleanup slot across card lifetimes, plus one replaceable info intent. */
export class VideoRequestLane {
  private active?: Ticket<unknown>;
  private latest?: Ticket<unknown>;
  private disposed = false;
  constructor(private cancelNative: (id: string) => Promise<void>) {}
  request<T>(kind: VideoRequestKind, run: (id: string) => Promise<T>, id = crypto.randomUUID()): VideoRequest<T> {
    let resolve!: (value: T) => void, reject!: (error: unknown) => void;
    const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
    const ticket: Ticket<T> = { id, kind, run, resolve, reject, canceled: false };
    if (this.disposed || (this.active && !replaceable(this.active.kind)) || (this.latest && !replaceable(this.latest.kind))) { reject(new Error('视频处理进行中')); return { id, promise, cancel: async () => {} }; }
    if (this.latest) { this.latest.canceled = true; this.latest.reject(new TrimCanceled()); }
    this.latest = ticket as Ticket<unknown>;
    if (this.active) void this.cancel(this.active).catch(() => {});
    this.drain();
    return { id, promise, cancel: () => this.cancel(ticket as Ticket<unknown>) };
  }
  private async cancel(ticket: Ticket<unknown>) {
    ticket.canceled = true;
    if (this.latest === ticket) { this.latest = undefined; ticket.reject(new TrimCanceled()); return; }
    if (this.active !== ticket) return;
    if (!ticket.cleanup) ticket.cleanup = Promise.resolve().then(() => this.cancelNative(ticket.id)).catch(error => { ticket.cleanupError = error; });
    await ticket.done;
    if (ticket.cleanupError !== undefined && !(!replaceable(ticket.kind) && ticket.succeeded)) throw ticket.cleanupError;
  }
  private drain() {
    if (this.active || !this.latest || this.disposed) return;
    const ticket = this.latest; this.latest = undefined; this.active = ticket;
    ticket.done = (async () => {
      let value: unknown, failure: unknown;
      try { if (ticket.canceled) throw new TrimCanceled(); value = await ticket.run(ticket.id); }
      catch (error) { failure = error; }
      ticket.succeeded = failure === undefined && (ticket.kind !== 'copy' || value === true);
      if (ticket.cleanup) await ticket.cleanup;
      if (!replaceable(ticket.kind) && ticket.succeeded) ticket.resolve(value);
      else if (ticket.cleanupError !== undefined) ticket.reject(ticket.cleanupError);
      else if (ticket.kind === 'copy' && failure === undefined && value === false) ticket.resolve(false);
      else if (ticket.kind === 'copy' && failure !== undefined) ticket.reject(failure);
      else if (ticket.canceled) ticket.reject(new TrimCanceled());
      else if (failure !== undefined) ticket.reject(failure);
      else ticket.resolve(value);
      if (this.active === ticket) this.active = undefined;
      this.drain();
    })();
  }
  async cancelAll() {
    if (this.latest) { const old = this.latest; this.latest = undefined; old.canceled = true; old.reject(new TrimCanceled()); }
    if (this.active) await this.cancel(this.active);
  }
  dispose() { this.disposed = true; return this.cancelAll(); }
}

export type RegisterVideoFlush = (flush: (active: () => boolean) => Promise<void>) => () => void;
export class VideoFlushRegistry {
  private entries = new Set<(active: () => boolean) => Promise<void>>();
  register: RegisterVideoFlush = flush => { this.entries.add(flush); return () => { this.entries.delete(flush); }; };
  async flush(active: () => boolean) {
    for (const entry of [...this.entries]) { if (!active()) throw new TrimCanceled(); await entry(active); }
    if (!active()) throw new TrimCanceled();
  }
}

/** Every mounted player registers, independent of active card, metadata or plugin. */
export class VideoPauseRegistry {
  private players = new Set<() => void>();
  register = (pause: () => void) => { this.players.add(pause); return () => { this.players.delete(pause); }; };
  pauseAll() { for (const pause of [...this.players]) pause(); }
}
