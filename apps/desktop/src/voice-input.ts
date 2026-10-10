// SPDX-License-Identifier: MPL-2.0
import type { VoiceCapabilities, VoiceLanguage, VoicePending, VoiceProgress, VoiceRequest, VoiceResult } from './voice-contracts';

export interface VoiceScope { sceneId: string; agentId: string; backgroundId: string | null; runId: string | null; pluginId: string; revision: number; contributionId: string }
interface Ticket { request: VoiceRequest; scope: VoiceScope; phase: VoiceProgress['phase']; canceled: boolean; runSettled: boolean; cancelSettled: boolean; cancelError?: unknown; result?: VoiceResult; release: Promise<void>; resolve: () => void; reject: (error: unknown) => void }
export function sameVoiceScope(a: VoiceScope | undefined, b: VoiceScope): boolean {
  return Boolean(a && a.sceneId === b.sceneId && a.agentId === b.agentId && a.backgroundId === b.backgroundId && a.runId === b.runId && a.pluginId === b.pluginId && a.revision === b.revision && a.contributionId === b.contributionId);
}
export function appendDictation(draft: string, recognized: string): string {
  if ([...recognized].length > 8000 || new TextEncoder().encode(recognized).length > 32768 || recognized.includes('\0')) throw new Error('识别文字超出范围');
  const text = recognized.trim();
  if (!text) throw new Error('未识别到文字');
  return draft ? `${draft}${/\s$/u.test(draft) ? '' : ' '}${text}` : text;
}
export function voiceLanguages(capabilities: VoiceCapabilities): Array<{ value: VoiceLanguage; label: string }> {
  if (!capabilities.supported) return [];
  const options: Array<{ value: VoiceLanguage; label: string }> = [{ value: 'system', label: '系统语言' }];
  for (const [value, family] of [['zh-CN', 'zh'], ['en-US', 'en']] as const) {
    const language = capabilities.languages.find(item => item.tag.toLowerCase() === value.toLowerCase()) ?? capabilities.languages.find(item => item.tag.toLowerCase().split('-')[0] === family);
    if (language) options.push({ value, label: language.name || language.tag });
  }
  return options;
}

/** One real request remains owned until both its run and cancellation settle. */
export class VoiceInput {
  private ticket?: Ticket;
  private disposed = false;
  constructor(private hooks: {
    current: () => VoiceScope | undefined;
    run: (request: VoiceRequest) => Promise<VoiceResult>;
    cancel: (requestId: string) => Promise<void>;
    composing: () => boolean;
    draft: () => string;
    setDraft: (text: string) => void;
    changed: (pending?: VoicePending) => void;
    error: (error: unknown) => void;
    id?: () => string;
  }) {}
  private valid(ticket: Ticket): boolean { return !this.disposed && this.ticket === ticket && !ticket.canceled && sameVoiceScope(this.hooks.current(), ticket.scope); }
  private publish(ticket: Ticket) {
    if (!this.disposed && this.ticket === ticket) this.hooks.changed({ requestId: ticket.request.requestId, sceneId: ticket.scope.sceneId, phase: ticket.phase, awaitingComposition: Boolean(ticket.result) });
  }
  private release(ticket: Ticket) {
    if (!ticket.runSettled || (ticket.canceled && !ticket.cancelSettled) || (!ticket.canceled && ticket.result)) return;
    if (this.ticket === ticket) { this.ticket = undefined; if (!this.disposed) this.hooks.changed(undefined); }
    if (ticket.cancelError) ticket.reject(ticket.cancelError); else ticket.resolve();
  }
  start(language: VoiceLanguage): boolean {
    const scope = this.hooks.current(); if (this.disposed || this.ticket || !scope) return false;
    let resolve!: () => void, reject!: (error: unknown) => void;
    const release = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
    void release.catch(() => {});
    const request = { requestId: this.hooks.id?.() ?? crypto.randomUUID(), pluginId: scope.pluginId, revision: scope.revision, contributionId: scope.contributionId, sceneId: scope.sceneId, language };
    const ticket: Ticket = { request, scope, phase: 'starting', canceled: false, runSettled: false, cancelSettled: false, release, resolve, reject };
    this.ticket = ticket; this.publish(ticket); void this.execute(ticket); return true;
  }
  private async execute(ticket: Ticket) {
    try {
      const result = await this.hooks.run(ticket.request);
      if (!this.valid(ticket)) return;
      if (result.requestId !== ticket.request.requestId || result.sceneId !== ticket.scope.sceneId || !['recognized', 'candidate'].includes(result.confidence)) throw new Error('语音结果已失效');
      appendDictation('', result.text); // Validate before retaining a bounded IME result.
      ticket.result = result;
      if (this.hooks.composing()) { ticket.phase = 'stopping'; this.publish(ticket); }
      else this.insert(ticket);
    } catch (error) { if (this.valid(ticket)) this.hooks.error(error); }
    finally { ticket.runSettled = true; this.release(ticket); }
  }
  private insert(ticket: Ticket) {
    if (!ticket.result || !this.valid(ticket) || this.hooks.composing()) return;
    const text = appendDictation(this.hooks.draft(), ticket.result.text);
    ticket.result = undefined;
    this.hooks.setDraft(text); // Synchronous read/merge/write of the latest draft.
    this.release(ticket);
  }
  compositionEnded() {
    const ticket = this.ticket; if (!ticket?.result) return;
    queueMicrotask(() => { if (!this.valid(ticket)) return; try { this.insert(ticket); } catch (error) { ticket.result = undefined; this.hooks.error(error); this.release(ticket); } });
  }
  progress(value: VoiceProgress) {
    const ticket = this.ticket;
    if (!ticket || ticket.request.requestId !== value.requestId || ticket.scope.sceneId !== value.sceneId) return;
    const order = { starting: 0, listening: 1, stopping: 2 };
    if (!(value.phase in order) || order[value.phase] < order[ticket.phase]) return;
    ticket.phase = value.phase; this.publish(ticket);
  }
  isActive(requestId: string): boolean { return Boolean(this.ticket && this.ticket.request.requestId === requestId && this.valid(this.ticket)); }
  invalidate(requestId: string) { if (this.ticket?.request.requestId === requestId) void this.cancel().catch(this.hooks.error); }
  reconcile() { const ticket = this.ticket; if (ticket && !ticket.canceled && !sameVoiceScope(this.hooks.current(), ticket.scope)) void this.cancel().catch(this.hooks.error); }
  cancel(): Promise<void> {
    const ticket = this.ticket; if (!ticket) return Promise.resolve();
    if (ticket.canceled) return ticket.release;
    // Revoke insertion before any IPC/microtask/exit flush can run.
    ticket.canceled = true; ticket.result = undefined; ticket.phase = 'stopping'; this.publish(ticket);
    void Promise.resolve().then(() => this.hooks.cancel(ticket.request.requestId)).catch(error => { ticket.cancelError = error; }).finally(() => { ticket.cancelSettled = true; this.release(ticket); });
    return ticket.release;
  }
  dispose() { if (this.disposed) return; const canceled = this.cancel(); this.disposed = true; void canceled.catch(() => {}); }
}
