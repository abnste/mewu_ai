// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { Check, ChevronLeft, ChevronRight, Copy, ExternalLink, QrCode } from 'lucide-solid';
import type { CodeAction, CodeScanResult } from '../code-contracts';
import { placeCodeCard, type CodeBox } from '../code-card-layout';
import './selection-code-card.css';

interface Props {
  result: CodeScanResult; region: CodeBox; toolbar?: CodeBox; hidden: boolean; disabled: boolean;
  onAction: (sourceToken: string, codeId: string, action: CodeAction) => Promise<boolean>;
}
export default function SelectionCodeCard(props: Props) {
  let element!: HTMLDivElement, disposed = false, copiedTimer: ReturnType<typeof setTimeout> | undefined;
  let observer: ResizeObserver | undefined, mutations: MutationObserver | undefined;
  const [selected, setSelected] = createSignal(''), [pending, setPending] = createSignal(false), [copied, setCopied] = createSignal('');
  const [size, setSize] = createSignal({ width: 245, height: 40 }, { equals: (a, b) => a.width === b.width && a.height === b.height });
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const [obstacles, setObstacles] = createSignal<CodeBox[]>([]);
  const index = createMemo(() => Math.max(0, props.result.codes.findIndex(value => value.id === selected())));
  const code = createMemo(() => props.result.codes[index()]);
  const location = createMemo(() => placeCodeCard(viewport(), props.region, size(), [...(props.toolbar ? [props.toolbar] : []), ...obstacles()]));
  createEffect(on(() => props.result.sourceToken, () => { setSelected(props.result.codes[0]?.id ?? ''); setCopied(''); }));
  function measure() {
    if (disposed || !element) return;
    setSize({ width: element.offsetWidth, height: element.offsetHeight });
    const next = [...document.querySelectorAll<HTMLElement>('.composer,.artifact-card,.ocr-actions,.translation-actions')].filter(node => node.getClientRects().length && getComputedStyle(node).visibility !== 'hidden').map(node => {
      const rect = node.getBoundingClientRect(); return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    });
    setObstacles(previous => JSON.stringify(previous) === JSON.stringify(next) ? previous : next);
  }
  const resize = () => { setViewport({ width: innerWidth, height: innerHeight }); measure(); };
  function navigate(delta: number) {
    if (pending() || props.disabled) return;
    const count = props.result.codes.length; if (count) setSelected(props.result.codes[(index() + delta + count) % count].id);
  }
  async function action(kind: CodeAction) {
    const current = code(), token = props.result.sourceToken;
    if (!current || pending() || props.disabled || props.hidden || !location()) return;
    setPending(true);
    try {
      const success = await props.onAction(token, current.id, kind);
      if (success && kind === 'copy' && !disposed && props.result.sourceToken === token && code()?.id === current.id) {
        setCopied(current.id); clearTimeout(copiedTimer); copiedTimer = setTimeout(() => setCopied(''), 1400);
      }
    } finally { if (!disposed) setPending(false); }
  }
  onMount(() => {
    observer = new ResizeObserver(measure); observer.observe(element);
    for (const node of document.querySelectorAll('.composer,.artifact-card')) observer.observe(node);
    mutations = new MutationObserver(measure);
    for (const node of document.querySelectorAll('.composer,.artifact-layer')) mutations.observe(node, { attributes: true, attributeFilter: ['style', 'class'], subtree: true, childList: true });
    window.addEventListener('resize', resize); measure();
  });
  onCleanup(() => { disposed = true; observer?.disconnect(); mutations?.disconnect(); window.removeEventListener('resize', resize); clearTimeout(copiedTimer); });
  return <div ref={element} class="selection-code-card" role="group" aria-label={t("二维码与条码")} aria-hidden={props.hidden || !location()} inert={props.hidden || !location()} style={{ left: `${location()?.x ?? 0}px`, top: `${location()?.y ?? 0}px`, visibility: props.hidden || !location() ? 'hidden' : 'visible' }} onPointerDown={event => event.stopPropagation()} onDblClick={event => event.stopPropagation()} on:keydown={event => {
    if (event.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); event.stopPropagation(); navigate(event.key === 'ArrowLeft' ? -1 : 1); }
  }}>
    <span class="selection-code-label" title={code()?.text}><QrCode size={15} /><span>{code()?.format.toUpperCase().includes('QR') ? t('二维码') : t('条码')}</span></span>
    <Show when={props.result.codes.length > 1}><div class="selection-code-navigation"><button class="icon-button compact" aria-label={t("上一个码")} title={t("上一个")} disabled={pending() || props.disabled} onClick={() => navigate(-1)}><ChevronLeft size={14} /></button><span>{index() + 1}/{props.result.codes.length}</span><button class="icon-button compact" aria-label={t("下一个码")} title={t("下一个")} disabled={pending() || props.disabled} onClick={() => navigate(1)}><ChevronRight size={14} /></button></div></Show>
    <button class="selection-code-action" aria-label={t("复制当前码")} title={code()?.text} disabled={pending() || props.disabled} onClick={() => void action('copy')}><Show when={copied() === code()?.id} fallback={<Copy size={13} />}><Check size={13} /></Show><span>{t("复制")}</span></button>
    <Show when={code()?.canOpen}><button class="selection-code-action" aria-label={t("打开当前链接")} title={code()?.text} disabled={pending() || props.disabled} onClick={() => void action('open')}><ExternalLink size={13} /><span>{t("打开")}</span></button></Show>
  </div>;
}
