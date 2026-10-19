// SPDX-License-Identifier: MPL-2.0
import { t } from "./i18n";
import { nativeSelectOwnsEscape } from './native-select-escape';
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { Check, CircleAlert, LoaderCircle, X } from 'lucide-solid';
import { controlScrollCapture, scrollNeedsKeep, scrollSurfaceReady, subscribeScroll, type ScrollAction, type ScrollStatus } from './scroll-bridge';
import { readPreferences } from './interface-preferences';
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
    <Show when={!props.status.hud}><rect x={rect().x} y={rect().y} width={rect().width} height={rect().height} fill="none" stroke="#3297f2" stroke-width="2" vector-effect="non-scaling-stroke" /></Show>
  </svg></div>;
}
const notices: Partial<Record<ScrollStatus['status'], string>> = { low_information: '内容不足', ambiguous: '无法对齐', lost_overlap: '滚动过快', limit_reached: '已达上限' };
const hudStyle = (rect: ScrollStatus['rect'] | null | undefined) => rect ? {
  left: `${rect.x * 100}%`, top: `${rect.y * 100}%`, width: `${rect.width * 100}%`, height: `${rect.height * 100}%`,
} : undefined;
export default function ScrollSurface() {
  document.documentElement.dataset.surface = 'scroll';
  const maskOpacity = readPreferences().maskOpacity;
  const [status, setStatus] = createSignal<ScrollStatus | null>(null), [pending, setPending] = createSignal(false), [error, setError] = createSignal('');
  let disposed = false, stop: (() => void) | undefined;
  let surface: HTMLDivElement | undefined;
  const preparingSurface = createMemo(() => status()?.phase === 'starting' && status()?.hud && !status()?.hud?.live ? status()!.id : undefined);
  createEffect(on(preparingSurface, id => {
    if (!id) return;
    let canceled = false, frame = 0;
    const ready = () => {
      if (canceled || disposed || status()?.id !== id || status()?.phase !== 'starting') return;
      if (document.hidden) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (canceled || disposed || document.hidden || status()?.id !== id || status()?.phase !== 'starting' || !surface?.isConnected) return;
        const box = surface.getBoundingClientRect();
        if (box.width <= 0 || box.height <= 0) return;
        // A visible layout acknowledgement is not an optical first-frame claim.
        void scrollSurfaceReady(id).catch(cause => {
          if (!canceled && !disposed && status()?.id === id) setError(cause instanceof Error ? cause.message : String(cause));
        });
      });
    };
    const visibility = () => { if (!document.hidden && !frame) ready(); };
    document.addEventListener('visibilitychange', visibility);
    ready();
    onCleanup(() => { canceled = true; if (frame) cancelAnimationFrame(frame); document.removeEventListener('visibilitychange', visibility); });
  }));
  onMount(async () => {
    try { stop = await subscribeScroll(value => { if (!disposed) { setStatus(value); setError(''); } }, 'scroll-controls'); if (disposed) stop(); }
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
  return <div ref={element => { surface = element; }} class="scroll-control-window" classList={{ 'scroll-compound-window': Boolean(status()?.hud) }}><Show when={status()} fallback={<Show when={error()}><div class="scroll-controls scroll-error" role="alert"><CircleAlert size={15} /><span>{error()}</span></div></Show>}>
    {current => <><Show when={current().hud}>{hud => <><Show when={hud().live && !hud().retainMain}><ScrollBackdrop status={current()} maskOpacity={maskOpacity} /></Show><div class="scroll-live-outline" style={hudStyle(hud().outline)} aria-label={t('长截图区域')} /></>}</Show>
    <Show when={!current().hud || current().hud?.preview}><Show when={current().preview}>{preview=><div class="scroll-preview" style={hudStyle(current().hud?.preview)}><img src={preview()} alt={t('长截图预览')} draggable={false} onLoad={event=>{const host=event.currentTarget.parentElement;if(host)host.scrollTop=host.scrollHeight;}}/></div>}</Show></Show>
    <Show when={!current().hud || current().hud?.controls}><div class="scroll-controls-slot" style={hudStyle(current().hud?.controls)}><div class="scroll-controls" role="toolbar" aria-label={t("长截图控制")}>
      <div class="scroll-measure"><output aria-label={t("长截图尺寸")}>{current().width} × {current().height}</output><Show when={error() || notices[current().status]}><span class="scroll-notice" title={error() || t(notices[current().status] ?? '')}>{error() || t(notices[current().status] ?? '')}</span></Show></div>
      <Show when={current().phase !== 'capturing'}><LoaderCircle size={15} class="spin" /></Show>
      <Show when={!current().automatic}><button classList={{ 'scroll-keep': scrollNeedsKeep(current()) }} disabled={pending() || current().phase !== 'capturing'} title={scrollNeedsKeep(current()) ? t('保存已拼部分') : t("完成{0}").replaceAll("{0}", () => String(current().stopHotkey.includes('F8') ? ' · F8' : ''))} aria-label={scrollNeedsKeep(current()) ? t('保存已拼部分') : t('完成长截图')} onClick={() => void act(scrollNeedsKeep(current()) ? 'keep' : 'finish')}><Check size={18} /><Show when={scrollNeedsKeep(current())}><span>{t("保存已拼部分")}</span></Show></button></Show>
      <button disabled={pending()} title={current().automatic ? [t('取消'), current().stopHotkey].filter(Boolean).join(' · ') : t("取消 · Esc")} aria-label={t("取消长截图")} onClick={() => void act('cancel')}><X size={18} /></button>
    </div></div></Show></>}
  </Show></div>;
}
