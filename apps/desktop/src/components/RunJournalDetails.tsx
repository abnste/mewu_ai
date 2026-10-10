// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { For, Show, createMemo, createSignal } from 'solid-js';
import type { ContinuationDecision, JournalEntrySummary, RunJournalSummary } from '../journal-contracts';
import type { JournalState } from '../run-journal';
import '../journal.css';

const phaseText = (entry: JournalEntrySummary) => entry.phase === 'unknown' ? t('结果未确认')
  : entry.phase === 'not_sent' ? t('未执行') : entry.phase === 'prepared' ? t('待执行')
  : entry.phase === 'dispatched' ? t('等待结果') : entry.returnedError || entry.responseKind === 'rpc_error' ? t('返回错误')
  : entry.responseKind === 'input_required' ? t('需要输入') : entry.responseKind === 'task' ? t('远端处理中') : t('已返回');
const terminalText = (summary: RunJournalSummary) => summary.status === 'interrupted' ? t('已中断') : summary.status === 'canceled' ? t('已停止') : t('未完成');

export interface RunJournalProps {
  summary?: RunJournalSummary;
  state: JournalState;
  decision?: ContinuationDecision;
  pending: boolean;
  checkingSelection?: boolean;
  selecting?: boolean;
  history?: boolean;
  terminalLabel?: string;
  unavailable?: boolean;
  onOpen: () => void; onClose: () => void; onRetry: () => void; onMore: () => void;
  onDetail: (id: string) => void;
  onSelection: (ids: string[]) => void;
  onContinue: (selected?: string[]) => void;
}

/** Small child of existing reply area; no new window or normal-reply banner. */
export default function RunJournalDetails(props: RunJournalProps) {
  const [selection, setSelection] = createSignal<{ key: string; ids: string[] }>();
  const needsSelection = () => (props.decision ?? props.state.page?.continuation)?.selectionRequired === true;
  const eligibleIds = () => (props.decision ?? props.state.page?.continuation)?.defaultEventIds ?? [];
  const selectionKey = () => { const value = props.decision ?? props.state.page?.continuation; return value ? `${value.sourceRunId}:${value.journalRevision}:${value.checkpointSeq}` : ''; };
  const selected = createMemo(() => selection()?.key === selectionKey() ? selection()!.ids : eligibleIds());
  const canContinue = () => props.summary?.canContinue && props.summary.status !== 'running' && props.summary.status !== 'completed' && !props.pending && !props.unavailable;
  const toggle = (id: string, checked: boolean) => {
    if (!props.summary || !eligibleIds().includes(id)) return;
    const ids = eligibleIds().filter(value => value === id ? checked : selected().includes(value));
    setSelection({ key: selectionKey(), ids }); props.onSelection(ids);
  };
  const continueAnswer = () => {
    if (needsSelection() && !props.selecting) { props.onContinue(); return; }
    if (needsSelection() && (props.checkingSelection || !props.decision?.ready || JSON.stringify(props.decision.selectedEventIds) !== JSON.stringify(selected()))) return;
    props.onContinue(needsSelection() ? [...selected()] : undefined);
  };
  const keyboard = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && props.state.open && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); props.onClose(); }
    // Native element listener precedes parent document bubbling. Canvas capture
    // listeners must additionally exclude closest('[data-run-journal]') in integration.
    event.stopPropagation();
  };
  const entry = (value: JournalEntrySummary) => <div class="journal-entry">
    <Show when={needsSelection() && value.selectable && eligibleIds().includes(value.id)}>
      <input type="checkbox" aria-label={t("包含 {0}").replaceAll("{0}", () => String(value.label))} checked={selected().includes(value.id)} onChange={event => toggle(value.id, event.currentTarget.checked)} />
    </Show>
    <button class="journal-entry-open" onClick={() => props.onDetail(value.id)} disabled={Boolean(props.state.detailLoading)}><span>{value.label}</span><small>{phaseText(value)}</small></button>
    <Show when={props.state.detail?.entry.id === value.id}><div class="journal-detail">
      <Show when={!props.state.detail?.unavailable} fallback={<span>{props.state.detail?.unavailable === 'memory_policy' || props.state.detail?.unavailable === 'history_policy' ? t('未保留原文') : t('内容不可用')}</span>}><pre>{props.state.detail?.literal}</pre></Show>
    </div></Show>
  </div>;
  return <div class="run-journal tool-progress" data-run-journal on:keydown={keyboard} onPointerDown={event => event.stopPropagation()}>
    <Show when={!props.history && (props.terminalLabel || (props.summary && props.summary.status !== 'running' && props.summary.status !== 'completed'))}>
      <div class="journal-terminal"><span>{props.summary ? terminalText(props.summary) : props.terminalLabel}</span><button onClick={() => props.state.open ? props.onClose() : props.onOpen()} aria-expanded={props.state.open}>{t("记录")}</button>
        <Show when={canContinue()}><button title={t("根据记录继续，不调用工具")} onClick={continueAnswer} disabled={needsSelection() && props.selecting && (props.checkingSelection || !props.decision?.ready || selected().length === 0)}>{t("继续回答")}</button></Show>
      </div>
    </Show>
    <Show when={props.history || props.summary?.status === 'completed'}><button class="journal-record-toggle" aria-expanded={props.state.open} onClick={() => props.state.open ? props.onClose() : props.onOpen()}>{t("记录")}</button></Show>
    <Show when={props.state.open}>
      <div class="journal-body">
        <Show when={props.state.legacy}><span>{t("未保存执行记录")}</span></Show>
        <Show when={props.decision?.blockedReason === 'mandatory_context_too_large'}><span>{t("原始上下文过大，无法继续")}</span></Show>
        <For each={props.state.page?.entries.map(value => value.id) ?? []}>{id => <Show when={props.state.page?.entries.find(value => value.id === id)}>{value => entry(value())}</Show>}</For>
        <Show when={props.state.loading}><span role="status">{t("读取中")}</span></Show>
        <Show when={props.state.error}><div class="journal-error" role="alert"><span>{props.state.error}</span><button onClick={props.onRetry}>{t("重试")}</button></div></Show>
        <Show when={props.state.page?.nextCursor}><button onClick={props.onMore} disabled={props.state.loading}>{t("更多")}</button></Show>
        <Show when={props.history && canContinue()}><button onClick={continueAnswer} disabled={needsSelection() && props.selecting && (props.checkingSelection || !props.decision?.ready || selected().length === 0)}>{t("继续回答")}</button></Show>
      </div>
    </Show>
  </div>;
}
