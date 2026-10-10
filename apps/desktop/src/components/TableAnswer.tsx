// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, untrack } from 'solid-js';
import { RotateCcw } from 'lucide-solid';
import type { TableMessage } from '../table-contracts';
import { getMessageTables } from '../table-bridge';
import { TableRequestGate } from '../table-resource';
import Markdown from './Markdown';
import ReplyTable from './ReplyTable';

export default function TableAnswer(props: { sceneId: string; messageId: string; text: string; onError?: (error: string) => void }) {
  let element!: HTMLDivElement;
  const [visible, setVisible] = createSignal(false), [result, setResult] = createSignal<TableMessage>();
  const [failed, setFailed] = createSignal(false), [retry, setRetry] = createSignal(0);
  const sceneId = createMemo(() => props.sceneId), messageId = createMemo(() => props.messageId), text = createMemo(() => props.text);
  const gate = new TableRequestGate();
  createEffect(on([sceneId, messageId, text], () => { gate.cancel(); setResult(undefined); setFailed(false); }));
  createEffect(on([sceneId, messageId, text, visible, retry], ([scene, message, content, shown]) => {
    if (!shown || untrack(result) || untrack(failed)) return;
    gate.cancel(); setFailed(false);
    void gate.load(() => getMessageTables(scene, message, content), setResult, error => { setFailed(true); props.onError?.(error instanceof Error ? error.message : String(error)); });
  }));
  onMount(() => {
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => setVisible(entries.some(entry => entry.isIntersecting)));
    observer.observe(element); onCleanup(() => observer.disconnect());
  });
  onCleanup(() => gate.dispose());
  const table = (index: number) => result()?.tables.find(table => table.index === index);
  return <div ref={element} class="table-answer">
    <Show when={result()?.tables.length} fallback={<Markdown text={text()} />}>
      <For each={result()?.blocks}>{block => <Show when={block.kind === 'table'} fallback={<Markdown text={block.kind === 'markdown' ? block.text : ''} />}><Show when={block.kind === 'table' && table(block.tableIndex)}>{value => <ReplyTable table={value()} target={{ sceneId: sceneId(), messageId: messageId(), tableIndex: value().index }} text={text()} onError={props.onError} />}</Show></Show>}</For>
    </Show>
    <Show when={failed()}><button class="table-load-retry" aria-label={t("重试读取表格")} title={t("重试读取表格")} onClick={() => { setFailed(false); setRetry(value => value + 1); }}><RotateCcw size={13} />{t("重试")}</button></Show>
  </div>;
}
