// SPDX-License-Identifier: MPL-2.0
import type { ContinueFromRecord, ContinuationDecision, JournalEntryView, JournalIdentity, JournalPage, JournalReadPort, PreviewRunContinuation, RunJournalSummary } from './journal-contracts';

const same = (a: JournalIdentity | undefined, b: JournalIdentity | undefined) => a?.sceneId === b?.sceneId && a?.runId === b?.runId;
const textError = (error: unknown) => error instanceof Error ? error.message : String(error);
export interface JournalState {
  identity?: JournalIdentity; open: boolean; loading: boolean; legacy: boolean;
  page?: JournalPage; detail?: JournalEntryView; detailLoading?: string; error?: string;
}

/** Owned by an existing run row. Reads only on explicit expansion/retry. */
export class JournalReader {
  private epoch = 0;
  private disposed = false;
  private pageFlight?: Promise<void>;
  private pageEpoch?: number;
  private pageQueued = false;
  private detailFlight?: Promise<void>;
  private state: JournalState = { open: false, loading: false, legacy: false };
  constructor(private port: JournalReadPort, private changed: (state: JournalState) => void) {}
  value() { return this.state; }
  private set(patch: Partial<JournalState>) { this.state = { ...this.state, ...patch }; if (!this.disposed) this.changed(this.state); }
  target(identity: JournalIdentity | undefined) {
    if (same(identity, this.state.identity)) return;
    this.epoch++; this.pageQueued = false;
    this.state = { identity, open: false, loading: false, legacy: false };
    if (!this.disposed) this.changed(this.state);
  }
  close() {
    this.epoch++; this.pageQueued = false;
    this.set({ open: false, loading: false, detailLoading: undefined });
  }
  open(): Promise<void> {
    this.set({ open: true });
    return this.state.page || this.state.legacy ? Promise.resolve() : this.read(false);
  }
  retry(): Promise<void> { return this.read(false); }
  prefetch(): Promise<void> { return this.read(false); }
  refresh(readClosed = false): Promise<void> {
    this.epoch++; this.pageQueued = false;
    this.set({ page: undefined, detail: undefined, detailLoading: undefined, legacy: false, loading: false, error: undefined });
    return this.state.open || readClosed ? this.read(false) : Promise.resolve();
  }
  more(): Promise<void> { return this.state.page?.nextCursor ? this.read(true) : Promise.resolve(); }
  invalidate(identity: JournalIdentity, revision: number) {
    if (!same(identity, this.state.identity) || revision <= (this.state.page?.summary.revision ?? -1)) return;
    this.epoch++; this.pageQueued = false;
    this.set({ page: undefined, detail: undefined, detailLoading: undefined, legacy: false, loading: false });
    if (this.state.open) void this.read(false);
  }
  private read(append: boolean): Promise<void> {
    if (this.pageFlight) { if (this.pageEpoch !== this.epoch) this.pageQueued = true; return this.pageFlight; }
    const identity = this.state.identity, epoch = this.epoch, before = this.state.page;
    if (!identity || this.disposed) return Promise.resolve();
    this.set({ loading: true, error: undefined });
    const current = () => !this.disposed && epoch === this.epoch && same(identity, this.state.identity);
    const flight = this.port.page({ ...identity, limit: 25, ...(append && before?.nextCursor ? { cursor: before.nextCursor } : {}) }).then(page => {
      if (!current()) return;
      if (!page) { this.set({ legacy: true, page: undefined }); return; }
      if (!same(identity, { sceneId: page.summary.sceneId, runId: page.summary.runId })) throw Error('记录归属已变化');
      if (page.entries.length > 25 || new Set(page.entries.map(entry => entry.id)).size !== page.entries.length) throw Error('记录页无效');
      if (append && before && page.summary.revision !== before.summary.revision) throw Error('记录已更新，请重新载入');
      const entries = append && before ? [...before.entries, ...page.entries] : page.entries;
      if (entries.length > 64 || new Set(entries.map(entry => entry.id)).size !== entries.length || entries.some((entry, index) => index > 0 && entry.sequence <= entries[index - 1].sequence)) throw Error('记录页无效');
      this.set({ page: { ...page, entries }, legacy: false, detail: append ? this.state.detail : undefined });
    }).catch(error => { if (current()) this.set({ error: textError(error) }); }).finally(() => {
      if (this.pageFlight === flight) this.pageFlight = undefined;
      if (current()) this.set({ loading: false });
      if (this.pageQueued && this.state.open && !this.disposed) { this.pageQueued = false; void this.read(false); }
    });
    this.pageFlight = flight; this.pageEpoch = epoch; return flight;
  }
  showDetail(eventId: string): Promise<void> {
    // One detail at a time; max two native reads including a page. No full-run body cache.
    if (this.detailFlight) return this.detailFlight;
    const identity = this.state.identity, page = this.state.page, epoch = this.epoch;
    if (!identity || !page || !page.entries.some(entry => entry.id === eventId) || this.disposed) return Promise.resolve();
    this.set({ detailLoading: eventId, detail: undefined, error: undefined });
    const current = () => !this.disposed && epoch === this.epoch && same(identity, this.state.identity) && this.state.page?.summary.revision === page.summary.revision;
    const flight = this.port.detail({ ...identity, eventId, expectedJournalRevision: page.summary.revision }).then(detail => {
      if (!current()) return;
      if (detail.runId !== identity.runId || detail.journalRevision !== page.summary.revision || detail.entry.id !== eventId) throw Error('记录已更新，请重新载入');
      if (new TextEncoder().encode(detail.literal).byteLength > 2 * 1024 * 1024) throw Error('记录内容过大');
      this.set({ detail });
    }).catch(error => { if (current()) this.set({ error: textError(error) }); }).finally(() => {
      if (this.detailFlight === flight) this.detailFlight = undefined;
      if (current()) this.set({ detailLoading: undefined });
    });
    this.detailFlight = flight; return flight;
  }
  dispose() { this.disposed = true; this.epoch++; }
}

export interface ContinuationScope {
  sceneId: string; agentId: string; viewedRunId: string; sourceRunId: string; currentRunId: string | null;
  connectionId: string; connectionRevision: number; journalRevision: number; checkpointSeq: number;
}

/** Only used after the host says the default evidence set exceeds its budget. */
export class ProjectionPreview {
  private current?: { version: number; input: PreviewRunContinuation };
  private queued?: { version: number; input: PreviewRunContinuation };
  private flight?: Promise<void>;
  private version = 0;
  private disposed = false;
  constructor(private hooks: {
    preview: (input: PreviewRunContinuation) => Promise<ContinuationDecision>;
    changed: (decision: ContinuationDecision | undefined, error?: string) => void;
  }) {}
  select(input: PreviewRunContinuation) {
    if (this.disposed) return;
    const ticket = { version: ++this.version, input: { ...input, selectedEventIds: input.selectedEventIds ? [...input.selectedEventIds] : null } };
    this.current = ticket; this.queued = ticket; this.hooks.changed(undefined);
    this.drain();
  }
  private drain() {
    if (this.flight || !this.queued || this.disposed) return;
    const ticket = this.queued; this.queued = undefined;
    const current = () => !this.disposed && this.current === ticket;
    const flight = this.hooks.preview(ticket.input).then(decision => {
      if (!current()) return;
      if (decision.journalRevision !== ticket.input.expectedJournalRevision || decision.checkpointSeq !== ticket.input.expectedCheckpointSeq || JSON.stringify(decision.selectedEventIds) !== JSON.stringify(ticket.input.selectedEventIds)) throw Error('记录已更新，请重新载入');
      this.hooks.changed(decision);
    }).catch(error => { if (current()) this.hooks.changed(undefined, textError(error)); }).finally(() => {
      if (this.flight === flight) this.flight = undefined;
      this.drain();
    });
    this.flight = flight;
  }
  cancel() { this.version++; this.current = undefined; this.queued = undefined; if (!this.disposed) this.hooks.changed(undefined); }
  dispose() { this.disposed = true; this.cancel(); }
}
const sameScope = (left: ContinuationScope, right: ContinuationScope | undefined) => right &&
  (Object.keys(left) as (keyof ContinuationScope)[]).every(key => left[key] === right[key]);

export class ContinuationController<TSnapshot> {
  private flight?: Promise<boolean>;
  private disposed = false;
  constructor(private hooks: {
    current: () => ContinuationScope | undefined; allowed: () => boolean;
    beginOperation: () => () => void; flush: () => Promise<void>;
    enqueue: (operation: () => Promise<TSnapshot>) => Promise<TSnapshot>;
    start: (input: ContinueFromRecord) => Promise<TSnapshot>;
    accept: (snapshot: TSnapshot) => void; pending: (pending: boolean) => void;
    error: (message: string) => void;
  }) {}
  continue(summary: RunJournalSummary, decision: ContinuationDecision, selected?: readonly string[]): Promise<boolean> {
    if (this.flight) return this.flight; // Explicit double-click does not dispatch twice.
    const scope = this.hooks.current();
    if (this.disposed || !scope || !this.hooks.allowed() || !summary.canContinue || summary.status === 'running' || summary.status === 'completed') return Promise.resolve(false);
    if (scope.sceneId !== summary.sceneId || scope.agentId !== summary.agentId || scope.viewedRunId !== summary.runId || scope.sourceRunId !== decision.sourceRunId || scope.journalRevision !== decision.journalRevision || scope.checkpointSeq !== decision.checkpointSeq) return Promise.resolve(false);
    // The host decides the complete projection budget. Never infer it from raw content bytes.
    if (!decision.ready || (decision.selectionRequired && !selected)) return Promise.resolve(false);
    const chosen = selected ?? (summary.kind === 'continuation' ? decision.selectedEventIds : undefined);
    const ids = chosen ? [...new Set(chosen)] : null;
    if (ids && (!ids.length || ids.some(id => !decision.defaultEventIds.includes(id)) || ids.length !== decision.selectedEventIds.length || ids.some((id, index) => id !== decision.selectedEventIds[index]))) return Promise.resolve(false);
    if (!ids && (decision.selectedEventIds.length !== decision.defaultEventIds.length || decision.selectedEventIds.some((id, index) => id !== decision.defaultEventIds[index]))) return Promise.resolve(false);
    const active = () => !this.disposed && this.hooks.allowed() && sameScope(scope, this.hooks.current());
    const finish = this.hooks.beginOperation(); this.hooks.pending(true);
    const input: ContinueFromRecord = {
      sceneId: scope.sceneId, sourceRunId: scope.sourceRunId,
      expectedJournalRevision: scope.journalRevision, expectedCheckpointSeq: scope.checkpointSeq,
      expectedConnectionId: scope.connectionId, expectedConnectionRevision: scope.connectionRevision,
      selectedEventIds: ids,
    };
    const flight = (async () => {
      try {
        // Caller flushes accepted voice/drawing/video/geometry/draft operations,
        // never SpaceOperations.drain (which would wait for this operation itself).
        await this.hooks.flush();
        if (!active()) return false;
        const snapshot = await this.hooks.enqueue(async () => {
          if (!active()) throw Error('会话或连接已变化');
          return this.hooks.start(input);
        });
        // An accepted native run is not canceled by unmounting a reading row.
        // Global Snapshot.revision handling belongs to App.accept, not this controller.
        this.hooks.accept(snapshot); return true;
      } catch (error) { if (!this.disposed) this.hooks.error(textError(error)); return false; }
      finally { finish(); if (!this.disposed) this.hooks.pending(false); }
    })().finally(() => { if (this.flight === flight) this.flight = undefined; });
    this.flight = flight; return flight;
  }
  drain() { return this.flight ?? Promise.resolve(false); }
  dispose() { this.disposed = true; }
}

/** Replaces only Composer's latest-answer choice, not Markdown/history rendering. */
export function latestReply(input: {
  run?: { id: string; status: 'running' | 'completed' | 'failed' | 'canceled' };
  messages: { id: string; role: string; runId?: string; text: string }[];
  stream?: { runId: string; text: string };
}) {
  const run = input.run;
  const persisted = [...input.messages].reverse().find(message => message.role === 'assistant' && (!run || message.runId === run.id));
  if (persisted) return { text: persisted.text, messageId: persisted.id, incomplete: false };
  if (run && input.stream?.runId === run.id && input.stream.text) return { text: input.stream.text, messageId: undefined, incomplete: run.status !== 'running' };
  return { text: '', messageId: undefined, incomplete: Boolean(run && run.status !== 'running' && run.status !== 'completed') };
}
