// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Check, ChevronDown, Copy, LoaderCircle } from 'lucide-solid';
import type { TableDocument, TableFormat, TableTarget } from '../table-contracts';
import { exportMessageTable } from '../table-bridge';
import { formulaCopyText } from '../reply-markdown';
import { replyTablePresentation, type ReplyCellPart } from '../reply-table-math';
import { ReplyFormulaView } from './Markdown';
import '../tables.css';

export function ReplyTableCell(props: { text: string; parts?: ReplyCellPart[] }) {
  return <span class="reply-table-cell"><Show when={props.parts} fallback={props.text}>{parts => <For each={parts()}>{part => <Show when={part.formula} fallback={part.text}>{formula => <ReplyFormulaView formula={formula()} text={part.text} />}</Show>}</For>}</Show></span>;
}

export default function ReplyTable(props: { table: TableDocument; target: TableTarget; text: string; onError?: (error: string) => void }) {
  const presentation = createMemo(() => replyTablePresentation(props.text, props.table));
  const [menu, setMenu] = createSignal<{ left: number; top: number }>();
  const [pending, setPending] = createSignal(false), [copied, setCopied] = createSignal(false), [error, setError] = createSignal('');
  let more!: HTMLButtonElement, element: HTMLDivElement | undefined, timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
  const options: { format: TableFormat; label: string }[] = [
    { format: 'markdown', label: '复制 Markdown' }, { format: 'csv', label: '复制 CSV' },
    { format: 'tsv', label: '复制制表符文本' }, { format: 'png', label: '保存图片' },
  ];
  const closeMenu = (focus = false) => { setMenu(undefined); if (focus && !disposed) more?.focus({ preventScroll: true }); };
  const toggleMenu = () => {
    if (menu()) { closeMenu(); return; }
    const rect = more.getBoundingClientRect(), height = 154, width = 166;
    setMenu({ left: Math.max(6, Math.min(innerWidth - width - 6, rect.right - width)), top: rect.bottom + height + 6 <= innerHeight ? rect.bottom + 5 : Math.max(6, rect.top - height - 5) });
    queueMicrotask(() => element?.querySelector<HTMLButtonElement>('button')?.focus({ preventScroll: true }));
  };
  async function output(format: TableFormat) {
    if (pending()) return;
    closeMenu(); clearTimeout(timer); setCopied(false); setError(''); setPending(true);
    const target = { ...props.target }, text = props.text;
    try {
      await exportMessageTable(target, format, text);
      if (!disposed && format !== 'png') { setCopied(true); timer = setTimeout(() => setCopied(false), 1800); }
    } catch (value) {
      if (!disposed) { const message = value instanceof Error ? value.message : String(value); setError(message); props.onError?.(message); }
    } finally { if (!disposed) setPending(false); }
  }
  const outside = (event: PointerEvent) => { if (menu() && event.target instanceof Node && !element?.contains(event.target) && !more.contains(event.target)) closeMenu(); };
  const key = (event: KeyboardEvent) => {
    if (!menu()) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); closeMenu(true); }
    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const buttons = [...(element?.querySelectorAll<HTMLButtonElement>('button') ?? [])], index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      event.preventDefault(); event.stopImmediatePropagation(); buttons[(index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    } else if (event.key === 'Tab') closeMenu();
  };
  const reposition = () => { if (menu()) closeMenu(); };
  document.addEventListener('pointerdown', outside); document.addEventListener('keydown', key, true);
  window.addEventListener('resize', reposition); document.addEventListener('scroll', reposition, true);
  onCleanup(() => { disposed = true; clearTimeout(timer); document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', key, true); window.removeEventListener('resize', reposition); document.removeEventListener('scroll', reposition, true); });
  return <section class="reply-table" aria-label={t("表格 {0}").replaceAll("{0}", () => String(props.table.index + 1))}>
    <div class="reply-table-actions"><button class="reply-table-copy" disabled={pending()} title={error() || t('复制表格')} aria-label={t("复制表格")} onClick={() => void output('table')}><Show when={pending()} fallback={<Show when={copied()} fallback={<Copy size={13} />}><Check size={13} /></Show>}><LoaderCircle size={13} class="spin" /></Show><span>{copied() ? t('已复制') : t('复制表格')}</span></button><button ref={more} disabled={pending()} title={t("更多表格操作")} aria-label={t("更多表格操作")} aria-haspopup="menu" aria-expanded={Boolean(menu())} onClick={toggleMenu}><ChevronDown size={13} /></button></div>
    <div class="reply-table-scroll" tabindex={0} onCopy={event => { const selection = window.getSelection(); if (!selection?.rangeCount || selection.isCollapsed || !event.clipboardData) return; const text = formulaCopyText(event.currentTarget, selection.getRangeAt(0)); if (text === undefined) return; event.preventDefault(); event.clipboardData.setData('text/plain', text); }}><table><thead><tr><For each={props.table.header}>{(cell, index) => <th scope="col" style={{ 'text-align': props.table.align[index()] ?? 'left' }}><ReplyTableCell text={cell} parts={presentation()?.[0]?.[index()]} /></th>}</For></tr></thead><tbody><For each={props.table.rows}>{(row, r) => <tr><For each={props.table.header}>{(_, index) => <td style={{ 'text-align': props.table.align[index()] ?? 'left' }}><ReplyTableCell text={row[index()] ?? ''} parts={presentation()?.[r() + 1]?.[index()]} /></td>}</For></tr>}</For></tbody></table></div>
    <Show when={error() && !props.onError}><span class="reply-table-error" role="alert">{error()}</span></Show>
    <Show when={menu()}>{position => <Portal><div ref={element} class="reply-table-menu" role="menu" aria-label={t("表格操作")} style={{ left: `${position().left}px`, top: `${position().top}px` }}><For each={options}>{option => <button role="menuitem" onClick={() => void output(option.format)}>{t(option.label)}</button>}</For></div></Portal>}</Show>
  </section>;
}
