// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Copy, RotateCw, X } from 'lucide-solid';
import type { OcrDocument } from '../contracts';
import { nearestOcrGlyph, ocrLayout, unrotateOcrPoint, type OcrGlyph } from '../ocr-selection';
import './ocr.css';

interface Props { document: OcrDocument; active: boolean; label?: string; repeatLabel?: string; onClose: () => void; onRepeat?: () => void; onError: (message: string) => void; onSelecting?: (value: boolean) => void }

export default function OcrTextLayer(props: Props) {
  let surface!: HTMLDivElement, textHost!: HTMLSpanElement;
  let pointer: number | undefined, anchor = -1;
  const key = createMemo(() => JSON.stringify(props.document));
  const layout = createMemo(on(key, () => ocrLayout(props.document)));
  const [range, setRange] = createSignal<[number, number]>();
  const [menu, setMenu] = createSignal<{ x: number; y: number }>();
  const highlights = createMemo(() => {
    const selected = range(), boxes: OcrGlyph[] = [];
    if (!selected) return boxes;
    for (const glyph of layout().glyphs) {
      if (glyph.end <= selected[0] || glyph.start >= selected[1]) continue;
      const previous = boxes[boxes.length - 1];
      if (previous && Math.abs(previous.y - glyph.y) < 1 && Math.abs(previous.height - glyph.height) < 1 && glyph.x >= previous.x && glyph.x - previous.x - previous.width <= Math.max(3, glyph.height * .35)) {
        previous.width = glyph.x + glyph.width - previous.x; previous.end = glyph.end;
      } else boxes.push({ ...glyph });
    }
    return boxes;
  });
  function ownsSelection() { const selection = window.getSelection(); return Boolean(selection?.anchorNode && textHost?.contains(selection.anchorNode) && selection.focusNode && textHost.contains(selection.focusNode)); }
  function clearSelection() { if (ownsSelection()) window.getSelection()?.removeAllRanges(); setRange(undefined); }
  function select(start: number, end: number) {
    const node = textHost.firstChild;
    if (!node || node.nodeType !== Node.TEXT_NODE) return;
    const from = Math.min(start, end), to = Math.max(start, end);
    const value = document.createRange(); value.setStart(node, from); value.setEnd(node, to);
    const selection = window.getSelection(); selection?.removeAllRanges(); selection?.addRange(value);
    setRange(from === to ? undefined : [from, to]);
  }
  function hit(event: PointerEvent) {
    const box = surface.getBoundingClientRect(), doc = props.document;
    const point = unrotateOcrPoint((event.clientX - box.left) * doc.width / box.width, (event.clientY - box.top) * doc.height / box.height, doc.width, doc.height, doc.textAngle ?? 0);
    return nearestOcrGlyph(layout().glyphs, point.x, point.y);
  }
  function move(event: PointerEvent) {
    if (event.pointerId !== pointer) return;
    const index = hit(event), glyphs = layout().glyphs;
    if (index < 0 || anchor < 0) return;
    select(glyphs[Math.min(anchor, index)].start, glyphs[Math.max(anchor, index)].end);
  }
  function finish(event?: PointerEvent) {
    if (pointer === undefined || (event && event.pointerId !== pointer)) return;
    const id = pointer; pointer = undefined; props.onSelecting?.(false);
    if (surface.hasPointerCapture(id)) surface.releasePointerCapture(id);
  }
  function begin(event: PointerEvent) {
    if (event.shiftKey) return;
    event.stopPropagation();
    if (event.button !== 0 || (event.target as Element).closest('button,.ocr-context-menu')) return;
    event.preventDefault(); setMenu(undefined); surface.focus({ preventScroll: true }); anchor = hit(event);
    if (anchor < 0) return;
    pointer = event.pointerId; props.onSelecting?.(true);
    try { surface.setPointerCapture(pointer); move(event); } catch { finish(); }
  }
  function selectedText() { return ownsSelection() ? window.getSelection()?.toString() ?? '' : ''; }
  async function copy(all: boolean) {
    const text = all ? layout().text : selectedText(); setMenu(undefined);
    if (!text) return;
    try { await navigator.clipboard.writeText(text); }
    catch { props.onError('无法复制文字'); }
  }
  function clipboard(event: ClipboardEvent) {
    const text = selectedText();
    event.stopPropagation();
    if (text && event.clipboardData) { event.preventDefault(); event.clipboardData.setData('text/plain', text); }
  }
  function keydown(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); if (menu()) setMenu(undefined); else if (range()) clearSelection(); else props.onClose(); }
    else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); select(0, layout().text.length); }
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      event.preventDefault(); const selected = range(), glyphs = layout().glyphs;
      if (!glyphs.length) return;
      const index = selected ? glyphs.findIndex(glyph => glyph.end >= selected[1]) : -1;
      const next = Math.max(0, Math.min(glyphs.length - 1, index + (event.key === 'ArrowRight' ? 1 : -1)));
      select(event.shiftKey && selected ? selected[0] : glyphs[next].start, glyphs[next].end);
    }
  }
  const changed = () => { if (!ownsSelection()) setRange(undefined); };
  document.addEventListener('selectionchange', changed);
  createEffect(on(key, () => { finish(); clearSelection(); setMenu(undefined); }));
  createEffect(() => { if (!props.active) { finish(); clearSelection(); setMenu(undefined); } });
  onCleanup(() => { finish(); clearSelection(); document.removeEventListener('selectionchange', changed); });
  return <div ref={surface} class="ocr-text-layer" classList={{ 'ocr-active': props.active }} tabIndex={0} role="textbox" aria-label={props.label ?? t('识别文字')} aria-readonly="true" aria-multiline="true" onPointerDown={begin} onPointerMove={move} onPointerUp={event => { move(event); finish(event); }} onPointerCancel={finish} onLostPointerCapture={finish} onKeyDown={keydown} onCopy={clipboard} onContextMenu={event => { event.preventDefault(); event.stopPropagation(); surface.focus({ preventScroll: true }); const box = surface.getBoundingClientRect(); setMenu({ x: Math.max(0, Math.min(event.clientX - box.x, box.width - 112)), y: Math.max(0, Math.min(event.clientY - box.y, box.height - 138)) }); }}>
    <span ref={textHost} class="ocr-readable-text">{layout().text}</span>
    <div class="ocr-highlights-clip" aria-hidden="true"><svg viewBox={`0 0 ${props.document.width} ${props.document.height}`} preserveAspectRatio="none"><g transform={`rotate(${props.document.textAngle ?? 0} ${props.document.width / 2} ${props.document.height / 2})`}><For each={highlights()}>{box => <rect x={box.x} y={box.y} width={box.width} height={box.height} />}</For></g></svg></div>
    <Show when={props.active}><div class="ocr-text-actions" onPointerDown={event => { event.preventDefault(); event.stopPropagation(); }}><button title={t("复制全文")} aria-label={props.label ? t("复制{0}全文").replaceAll("{0}", () => String(props.label)) : t('复制识别全文')} onClick={() => void copy(true)}><Copy size={14} /></button><Show when={props.onRepeat}><button title={props.repeatLabel ?? t('重新识别')} aria-label={props.repeatLabel ?? t('重新识别文字')} onClick={() => props.onRepeat?.()}><RotateCw size={14} /></button></Show><button title={t("退出选字")} aria-label={t("退出选字")} onClick={props.onClose}><X size={14} /></button></div></Show>
    <Show when={menu()}>{position => <div class="ocr-context-menu" role="menu" style={{ left: `${position().x}px`, top: `${position().y}px` }} onPointerDown={event => { event.preventDefault(); event.stopPropagation(); }}><button role="menuitem" disabled={!range()} onClick={() => void copy(false)}>{t("复制")}</button><button role="menuitem" onClick={() => void copy(true)}>{t("复制全文")}</button><button role="menuitem" onClick={() => { select(0, layout().text.length); setMenu(undefined); }}>{t("全选")}</button><button role="menuitem" onClick={props.onClose}>{t("退出选字")}</button></div>}</Show>
  </div>;
}
