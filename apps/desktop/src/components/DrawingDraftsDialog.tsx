// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { For, onCleanup, onMount, Show } from 'solid-js';
import type { DrawingKind } from '../contracts';
import type { PendingDrawingDraft } from './drawing-properties';
import './drawing.css';

const labels: Partial<Record<DrawingKind, string>> = { pen: '画笔', line: '直线', arrow: '箭头', rect: '矩形', ellipse: '椭圆', text: '文字', highlighter: '荧光笔', number: '序号', mosaic: '马赛克' };
export default function DrawingDraftsDialog(props: {
  entries: PendingDrawingDraft[];
  onDiscard: (entry: PendingDrawingDraft) => void;
  onClose: () => void;
}) {
  let dialog!: HTMLElement, back!: HTMLButtonElement;
  const previousFocus = document.activeElement;
  const focusable = () => [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled),textarea,[tabindex="0"]')];
  function keydown(event: KeyboardEvent) {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); props.onClose(); return; }
    if (event.key !== 'Tab') return;
    const elements = focusable(), first = elements[0], last = elements.at(-1);
    if (!elements.length) { event.preventDefault(); return; }
    if (event.shiftKey && (document.activeElement === first || !dialog.contains(document.activeElement))) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || !dialog.contains(document.activeElement))) { event.preventDefault(); first?.focus(); }
    event.stopPropagation();
  }
  const focusin = (event: FocusEvent) => { if (!dialog.closest('[inert]') && event.target instanceof Node && !dialog.contains(event.target)) back.focus(); };
  onMount(() => {
    back.focus(); document.addEventListener('keydown', keydown, true); document.addEventListener('focusin', focusin);
  });
  onCleanup(() => { document.removeEventListener('keydown', keydown, true); document.removeEventListener('focusin', focusin); if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus({ preventScroll: true }); });
  return <div class="modal-backdrop drawing-drafts-backdrop" onPointerDown={event => event.stopPropagation()} onKeyDown={event => event.stopPropagation()}>
    <section ref={dialog} class="drawing-drafts-dialog" role="dialog" aria-modal="true" aria-labelledby="drawing-drafts-title">
      <h2 id="drawing-drafts-title">{t("未保存标注")}</h2>
      <div class="drawing-drafts-list"><For each={props.entries}>{entry => {
        const value = () => entry.draft.drawing;
        return <article class="drawing-drafts-entry">
          <div class="drawing-drafts-heading"><strong>{t(labels[value().kind] ?? '')}</strong><button class="secondary-button" onClick={() => props.onDiscard(entry)}>{t("放弃修改")}</button></div>
          <Show when={value().kind === 'text'}><textarea aria-label={t("未保存的文字")} readOnly value={value().text ?? ''} /></Show>
          <div class="drawing-drafts-properties"><span class="drawing-drafts-swatch" style={{ background: value().color }} /><span>{value().color}</span><span>{value().fontSize ?? value().strokeWidth}px</span></div>
        </article>;
      }}</For></div>
      <footer><button ref={back} class="primary-button" onClick={props.onClose}>{t("返回")}</button></footer>
    </section>
  </div>;
}
