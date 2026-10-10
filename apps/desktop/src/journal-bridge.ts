// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ContinuationDecision, JournalChanged, JournalEntryDetail, JournalEntryView, JournalIdentity, JournalPage, PreviewRunContinuation } from './journal-contracts';

let reads = 0;
const waiting: (() => void)[] = [];
async function read<T>(operation: () => Promise<T>): Promise<T> {
  if (reads >= 2) {
    if (waiting.length >= 16) throw new Error('记录读取繁忙，请稍后重试');
    await new Promise<void>(resolve => waiting.push(resolve));
  } else reads++;
  try { return await operation(); }
  finally { const next = waiting.shift(); if (next) next(); else reads--; }
}

export async function getRunJournal(input: JournalIdentity & { cursor?: string; limit: number }): Promise<JournalPage | null> {
  if (!isTauri()) return null;
  return read(() => invoke('get_run_journal', { sceneId: input.sceneId, runId: input.runId, cursor: input.cursor ?? null, limit: input.limit }));
}
export async function getRunJournalEvent(input: JournalIdentity & { eventId: string; expectedJournalRevision: number }): Promise<JournalEntryView> {
  if (!isTauri()) throw new Error('请在桌面版查看执行记录');
  const detail = await read(() => invoke<JournalEntryDetail>('get_run_journal_event', { ...input }));
  const content = detail.content;
  if (!content || !['text', 'json', 'omitted'].includes(content.type)) throw new Error('记录内容无效');
  const literal = content.type === 'text' ? content.text : content.type === 'json' ? JSON.stringify(content.value, null, 2) : '';
  if (typeof literal !== 'string') throw new Error('记录内容无效');
  return { runId: detail.runId, journalRevision: detail.journalRevision, entry: detail.entry, literal, unavailable: content.type === 'omitted' ? content.reason : null };
}
export async function previewRunContinuation(input: PreviewRunContinuation): Promise<ContinuationDecision> {
  if (!isTauri()) throw new Error('请在桌面版继续回答');
  return read(() => invoke('preview_run_continuation', { ...input }));
}
const listeners = new Set<(event: JournalChanged) => void>();
let subscription: Promise<() => void> | undefined;
export async function subscribeRunJournal(changed: (event: JournalChanged) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  listeners.add(changed);
  const pending = subscription ??= listen<JournalChanged>('run-journal-changed', event => { for (const callback of listeners) callback(event.payload); }, { target: 'space' });
  try {
    const stop = await pending;
    return () => { listeners.delete(changed); if (!listeners.size && subscription === pending) { subscription = undefined; stop(); } };
  } catch (error) { listeners.delete(changed); if (subscription === pending) subscription = undefined; throw error; }
}
