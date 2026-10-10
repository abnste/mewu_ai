// SPDX-License-Identifier: MPL-2.0
import { t } from "./i18n";
import { nativeSelectOwnsEscape } from './native-select-escape';
import { createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Check, CircleAlert, LoaderCircle, X } from 'lucide-solid';
import { controlScrollCapture, scrollNeedsKeep, subscribeScroll, type ScrollAction, type ScrollStatus } from './scroll-bridge';
import './scroll.css';

export function ScrollBackdrop(props: { status: ScrollStatus; maskOpacity: number }) {
  const previous = document.documentElement.dataset.surface;
  document.documentElement.dataset.surface = 'scroll-live';
  onCleanup(() => { if (previous) document.documentElement.dataset.surface = previous; else delete document.documentElement.dataset.surface; });
  const rect = createMemo(() => {
    const value = props.status.rect, x = Math.max(0, Math.min(1, value.x)), y = Math.max(0, Math.min(1, value.y));
    return { x, y, width: Math.max(0, Math.min(1 - x, value.width)), height: Math.max(0, Math.min(1 - y, value.height)) };
  });
  return <div class="scroll-live-mask" aria-label={t("长截图区域")}><svg viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden="true">
    <defs><mask id="scroll-cutout" maskUnits="userSpaceOnUse" x="0" y="0" width="1" height="1"><rect width="1" height="1" fill="white" /><rect {...rect()} fill="black" /></mask></defs>
    <rect width="1" height="1" fill="#000" fill-opacity={props.maskOpacity} mask="url(#scroll-cutout)" />
    <rect {...rect()} fill="none" stroke="#3297f2" stroke-width="2" vector-effect="non-scaling-stroke" />
  </svg></div>;
}
const notices: Partial<Record<ScrollStatus['status'], string>> = { low_information: '内容不足', ambiguous: '无法对齐', lost_overlap: '滚动过快', limit_reached: '已达上限' };
export default function ScrollSurface() {
  document.documentElement.dataset.surface = 'scroll';
  const [status, setStatus] = createSignal<ScrollStatus | null>(null), [pending, setPending] = createSignal(false), [error, setError] = createSignal('');
  let disposed = false, stop: (() => void) | undefined;
  onMount(async () => {
    try { stop = await subscribeScroll(value => { if (!disposed) { setStatus(value); setError(''); } }); if (disposed) stop(); }
    catch (cause) { if (!disposed) setError(String(cause)); }
  });
  async function act(action: ScrollAction) {
    const current = status(); if (!current || pending() || (current.phase === 'finishing' && action !== 'cancel')) return;
    setPending(true); setError('');
    try { await controlScrollCapture(current.id, action); }
    catch (cause) { if (!disposed && status()?.id === current.id) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  const keyboard = (event: KeyboardEvent) => {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.repeat || !['Escape', 'F8'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation(); void act(event.key === 'Escape' ? 'cancel' : 'finish');
  };
  document.addEventListener('keydown', keyboard);
  onCleanup(() => { disposed = true; stop?.(); document.removeEventListener('keydown', keyboard); });
  return <div class="scroll-control-window"><Show when={status()} fallback={<Show when={error()}><div class="scroll-controls scroll-error" role="alert"><CircleAlert size={15} /><span>{error()}</span></div></Show>}>
    {current => <><Show when={current().preview}>{preview=><div class="scroll-preview"><img src={preview()} alt={t('长截图预览')} draggable={false} onLoad={event=>{const host=event.currentTarget.parentElement;if(host)host.scrollTop=host.scrollHeight;}}/></div>}</Show><div class="scroll-controls" role="toolbar" aria-label={t("长截图控制")}>
      <div class="scroll-measure"><output aria-label={t("长截图尺寸")}>{current().width} × {current().height}</output><Show when={error() || notices[current().status]}><span class="scroll-notice" title={error() || t(notices[current().status] ?? '')}>{error() || t(notices[current().status] ?? '')}</span></Show></div>
      <Show when={current().phase !== 'capturing'}><LoaderCircle size={15} class="spin" /></Show>
      <button classList={{ 'scroll-keep': scrollNeedsKeep(current()) }} disabled={pending() || current().phase !== 'capturing'} title={scrollNeedsKeep(current()) ? t('保存已拼部分') : t("完成{0}").replaceAll("{0}", () => String(current().stopHotkey.includes('F8') ? ' · F8' : ''))} aria-label={scrollNeedsKeep(current()) ? t('保存已拼部分') : t('完成长截图')} onClick={() => void act(scrollNeedsKeep(current()) ? 'keep' : 'finish')}><Check size={18} /><Show when={scrollNeedsKeep(current())}><span>{t("保存已拼部分")}</span></Show></button>
      <button disabled={pending()} title={t("取消 · Esc")} aria-label={t("取消长截图")} onClick={() => void act('cancel')}><X size={18} /></button>
    </div></>}
  </Show></div>;
}
