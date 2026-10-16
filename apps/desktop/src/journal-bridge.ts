// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ArgumentDiagnostic, ContinuationDecision, JournalChanged, JournalEntryDetail, JournalEntryView, JournalIdentity, JournalPage, PreviewRunContinuation } from './journal-contracts';

function argumentDiagnostic(detail: JournalEntryDetail): ArgumentDiagnostic | null {
  if (detail.entry.phase !== 'not_sent' || detail.notSentReason !== 'invalid_arguments'
    || !['video_annotation', 'visual_annotation'].includes(detail.binding?.type ?? '')) return null;
  const value = detail.argumentDiagnostic;
  if (!value || typeof value !== 'object' || Object.keys(value).some(key => !['fieldPath', 'reason', 'expectedType'].includes(key))
    || typeof value.fieldPath !== 'string' || value.fieldPath.length > 256
    || !['required', 'type', 'additional_properties', 'const', 'enum', 'minimum', 'maximum', 'exclusive_minimum', 'exclusive_maximum', 'min_length', 'max_length', 'min_items', 'max_items', 'one_of', 'pattern', 'syntax', 'eof', 'data'].includes(value.reason)) return null;
  if (value.expectedType != null && (value.reason !== 'type' || !['null', 'boolean', 'number', 'integer', 'string', 'array', 'object'].includes(value.expectedType))) return null;
  const parts = value.fieldPath.split('/');
  if (parts.shift() !== '$' || parts.length > 16) return null;
  let previous = '', array = '', numericDepth = 0;
  for (const part of parts) {
    if (/^[0-9]+$/.test(part)) {
      const limit = previous === 'groups' ? 6 : previous === 'objects' ? 48 : previous === 'points' ? 128
        : previous === 'header' || previous === 'align' ? 32 : previous === 'rows' ? 199 : array === 'rows' && numericDepth === 1 ? 32 : 0;
      if (!/^(0|[1-9][0-9]{0,2})$/.test(part) || Number(part) >= limit) return null;
      numericDepth++;
    } else {
      if (!['coordinateSpace', 'styleUnit', 'timeSpace', 'objects', 'targetHandle', 'interval', 'startFrameHandle', 'endHandle', 'primitive', 'kind', 'bounds', 'x', 'y', 'width', 'height', 'style', 'color', 'strokeWidth', 'opacity', 'topLeft', 'anchor', 'text', 'fontSize', 'groups', 'target', 'points', 'table', 'version', 'header', 'rows', 'align', 'tex'].includes(part)) return null;
      array = part; numericDepth = 0;
    }
    previous = part;
  }
  return { fieldPath: value.fieldPath, reason: value.reason, ...(value.expectedType == null ? {} : { expectedType: value.expectedType }) };
}

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
  return { runId: detail.runId, journalRevision: detail.journalRevision, entry: detail.entry, literal, unavailable: content.type === 'omitted' ? content.reason : null, notSentReason: detail.entry.phase === 'not_sent' ? detail.notSentReason ?? null : null, argumentDiagnostic: argumentDiagnostic(detail) };
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
