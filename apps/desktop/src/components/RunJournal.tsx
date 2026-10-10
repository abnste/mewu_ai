// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import * as journal from '../journal-bridge';
import { JournalReader, ProjectionPreview, type JournalState } from '../run-journal';
import type { ContinuationDecision, RunJournalSummary } from '../journal-contracts';
import RunJournalDetails from './RunJournalDetails';

export type ContinueJournal = (summary: RunJournalSummary, decision: ContinuationDecision, selected?: string[]) => Promise<boolean>;

export default function RunJournal(props: {
  sceneId: string; runId: string; history?: boolean; terminalLabel?: string;
  runPulse: string;
  unavailable: boolean; onContinue: ContinueJournal;
}) {
  const empty = (): JournalState => ({ open: false, loading: false, legacy: false });
  const [state, setState] = createSignal<JournalState>(empty());
  const [sourceState, setSourceState] = createSignal<JournalState>(empty());
  const [selectionMode, setSelectionMode] = createSignal(false);
  const [decision, setDecision] = createSignal<ContinuationDecision>();
  const [checking, setChecking] = createSignal(false);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal('');
  const [stale, setStale] = createSignal(false);
  let disposed = false, stop: (() => void) | undefined;
  const port = { page: journal.getRunJournal, detail: journal.getRunJournalEvent };
  const reader = new JournalReader(port, value => {
    const previous = state().page;
    setState(value);
    if (value.page && value.page !== previous) { setDecision(value.page.continuation); setStale(false); }
  });
  const sourceReader = new JournalReader(port, value => {
    const previous = sourceState().page; setSourceState(value);
    if (value.page && value.page !== previous) setStale(false);
  });
  const preview = new ProjectionPreview({ preview: journal.previewRunContinuation, changed: (value, error) => {
    if (disposed) return;
    if (value) { setDecision(value); setChecking(false); }
    else if (error) { setError(error); setChecking(false); }
  } });
  const identity = createMemo(() => [props.sceneId, props.runId, Boolean(props.history)] as const, undefined,
    { equals: (before, after) => before[0] === after[0] && before[1] === after[1] && before[2] === after[2] });
  const runPulse = createMemo(() => props.runPulse);
  createEffect(on(identity, ([sceneId, runId, history]) => {
    reader.target({ sceneId, runId }); sourceReader.target(undefined); preview.cancel();
    setDecision(undefined); setSelectionMode(false); setChecking(false); setError(''); setStale(false);
    if (!history) void reader.prefetch().then(() => { if (!disposed && reader.value().identity?.runId === runId) setDecision(reader.value().page?.continuation); });
  }));
  createEffect(on(runPulse, () => {
    preview.cancel(); setDecision(undefined); setChecking(false); setError(''); setStale(false);
    void reader.refresh(!props.history);
    void sourceReader.refresh();
  }, { defer: true }));
  void journal.subscribeRunJournal(event => {
    const page = reader.value().page;
    reader.invalidate(event, event.revision); sourceReader.invalidate(event, event.revision);
    if (event.sceneId === props.sceneId && (event.runId === props.runId || event.runId === decision()?.sourceRunId) && event.revision > (event.runId === props.runId ? page?.summary.revision ?? -1 : decision()?.journalRevision ?? -1)) {
      preview.cancel(); setDecision(undefined); setChecking(false); setStale(true);
    }
  }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(cause => { if (!disposed) setError(String(cause)); });
  onCleanup(() => { disposed = true; reader.dispose(); sourceReader.dispose(); preview.dispose(); stop?.(); });
  const currentReader = () => selectionMode() ? sourceReader : reader;
  async function open() {
    setError(''); await currentReader().open();
    if (!disposed && !selectionMode()) setDecision(reader.value().page?.continuation);
  }
  function close() { reader.close(); sourceReader.close(); preview.cancel(); setChecking(false); setError(''); setStale(false); }
  async function retry() {
    setError(''); preview.cancel(); setChecking(false); setSelectionMode(false);
    await reader.retry(); if (!disposed) setDecision(reader.value().page?.continuation);
  }
  function select(ids: string[]) {
    const value = decision(); if (!value || !value.selectionRequired) return;
    setChecking(true); setError('');
    preview.select({ sceneId: props.sceneId, sourceRunId: value.sourceRunId, expectedJournalRevision: value.journalRevision, expectedCheckpointSeq: value.checkpointSeq, selectedEventIds: ids });
  }
  async function continueAnswer(selected?: string[]) {
    if (pending() || props.unavailable || checking()) return;
    let value = decision();
    if (!value) { await reader.prefetch(); value = reader.value().page?.continuation; if (!disposed) setDecision(value); }
    const summary = reader.value().page?.summary;
    if (disposed || !summary || !value || !summary.canContinue) return;
    if (value.selectionRequired && !selectionMode()) {
      setSelectionMode(true); sourceReader.target({ sceneId: props.sceneId, runId: value.sourceRunId }); await sourceReader.open(); return;
    }
    if (!value.ready || (value.selectionRequired && !selected)) return;
    setPending(true); setError('');
    try { await props.onContinue(summary, value, selected); }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  const display = () => {
    const value = selectionMode() ? sourceState() : state();
    return { ...value, error: error() || value.error || (stale() ? '记录已更新，请重新载入' : undefined) };
  };
  return <RunJournalDetails summary={state().page?.summary} state={display()} decision={decision()} pending={pending()} checkingSelection={checking()} selecting={selectionMode()} history={props.history} terminalLabel={props.terminalLabel} unavailable={props.unavailable}
    onOpen={() => void open()} onClose={close} onRetry={() => void retry()} onMore={() => void currentReader().more()} onDetail={id => void currentReader().showDetail(id)} onSelection={select} onContinue={selected => void continueAnswer(selected)} />;
}
