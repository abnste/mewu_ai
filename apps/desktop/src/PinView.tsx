// SPDX-License-Identifier: MPL-2.0
import { t } from "./i18n";
import { nativeSelectOwnsEscape } from './native-select-escape';
import { createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import * as pin from './pin-bridge';
import { referencePinObject } from './pin-objects';
import { Copy, Download, Link2, Pin, RotateCcw, X } from 'lucide-solid';
import { PinDragGesture, pinImageLayout } from './pin-interaction';
import './pin.css';

export default function PinView() {
  document.documentElement.dataset.surface = 'pin';
  const [state, setState] = createSignal<pin.PinViewState>(), [error, setError] = createSignal('');
  const [referencing, setReferencing] = createSignal(false);
  async function reference() { if (closing || disposed || referencing()) return; setReferencing(true); try { await referencePinObject(null,null); } catch(cause){report(cause);} finally{if(!disposed)setReferencing(false);} }
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  let disposed = false, acknowledged = false, closing = false, dragging = false, exporting = false, zooming = false, zoomSteps = 0;
  let stop: (() => void) | undefined, errorTimer: ReturnType<typeof setTimeout> | undefined;
  const gesture = new PinDragGesture();
  const padding = () => state() ? state()!.shadowPadding / state()!.scaleFactor : 12;
  const scale = () => state()?.scaleFactor ?? 1;
  const layout = createMemo(() => { const value = state(); return value && pinImageLayout(value.width, value.height, value.quarterTurns, viewport().width - padding() * 2, viewport().height - padding() * 2); });
  function report(cause: unknown) { if (disposed) return; setError(cause instanceof Error ? cause.message : String(cause)); clearTimeout(errorTimer); errorTimer = setTimeout(() => setError(''), 5000); }
  function failed() { if (acknowledged || disposed) return; acknowledged = true; void pin.pinReady(false).catch(report); }
  async function loaded(image: HTMLImageElement) {
    if (acknowledged || disposed) return;
    try { await image.decode(); if (disposed || acknowledged) return; acknowledged = true; await pin.pinReady(true); }
    catch (cause) { report(cause); acknowledged = true; if (!disposed) void pin.pinReady(false).catch(() => {}); }
  }
  onMount(async () => {
    try { stop = await pin.subscribePin(value => { if (!disposed) setState(value); }, text => { report(text); if (!state()) failed(); }); if (disposed) stop(); }
    catch (cause) { report(cause); failed(); }
  });
  async function close() { if (closing || disposed) return; closing = true; zoomSteps = 0; gesture.end(); try { await pin.controlPin({ type: 'close' }); } catch (cause) { closing = false; report(cause); } }
  async function exportImage(copy: boolean) { if (exporting || closing || disposed || !state()) return; exporting = true; try { await pin.exportPin(copy); } catch (cause) { report(cause); } finally { exporting = false; } }
  function down(event: PointerEvent) { if (event.button !== 0 || closing || !state() || (event.target as Element).closest('button')) return; event.preventDefault(); gesture.begin(event.clientX, event.clientY, event.pointerId); }
  function move(event: PointerEvent) {
    const value = state(); if (!value || closing || dragging) return;
    if (gesture.move(event.clientX, event.clientY, event.pointerId, event.buttons, value.dragThresholdX, value.dragThresholdY)) { dragging = true; void pin.startPinDrag().catch(report).finally(() => { dragging = false; }); }
  }
  const end = () => gesture.end();
  async function drainZoom() {
    if (zooming) return; zooming = true;
    try { while (zoomSteps && !closing && !disposed) { const direction = zoomSteps > 0 ? 1 : -1; zoomSteps -= direction; await pin.controlPin({ type: 'zoom', direction }); } }
    catch (cause) { zoomSteps = 0; report(cause); }
    finally { zooming = false; }
  }
  function wheel(event: WheelEvent) { event.preventDefault(); if (!state() || closing || !event.deltaY) return; gesture.end(); zoomSteps = Math.max(-8, Math.min(8, zoomSteps + (event.deltaY < 0 ? 1 : -1))); void drainZoom(); }
  function keyboard(event: KeyboardEvent) {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.isComposing || event.repeat || closing) return;
    if (event.key === 'Escape') { event.preventDefault(); gesture.end(); void pin.controlPin({ type: 'escape' }).catch(report); }
    else if ((event.ctrlKey || event.metaKey) && !event.altKey && ['c', 's'].includes(event.key.toLowerCase())) { event.preventDefault(); void exportImage(event.key.toLowerCase() === 'c'); }
  }
  const resize = () => { gesture.end(); setViewport({ width: innerWidth, height: innerHeight }); };
  window.addEventListener('pointermove', move); window.addEventListener('pointerup', end); window.addEventListener('pointercancel', end); window.addEventListener('blur', end); window.addEventListener('resize', resize); document.addEventListener('keydown', keyboard);
  document.addEventListener('wheel', wheel, { passive: false });
  onCleanup(() => { disposed = true; gesture.end(); zoomSteps = 0; stop?.(); clearTimeout(errorTimer); window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', end); window.removeEventListener('pointercancel', end); window.removeEventListener('blur', end); window.removeEventListener('resize', resize); document.removeEventListener('keydown', keyboard); document.removeEventListener('wheel', wheel); });
  return <main class="pin-surface" tabindex={0} style={{ '--pin-padding': `${padding()}px`, '--pin-outline': `${1 / scale()}px`, '--pin-shadow-y': `${2 / scale()}px`, '--pin-shadow-blur': `${6 / scale()}px`, '--pin-radius': `${10 / scale()}px`, opacity: state()?.opacity ?? 1 }} onPointerDown={down} onDblClick={event => { if (event.button === 0 && !(event.target as Element).closest('button')) { event.preventDefault(); void close(); } }} onContextMenu={event => { event.preventDefault(); gesture.end(); if (!closing) void pin.showPinMenu().catch(report); }}>
    <Show when={state()}>{value => <div class="pin-frame"><img class="pin-image" src={value().imageUrl} alt={t("贴图")} draggable={false} style={{ width: `${layout()?.width ?? 0}px`, height: `${layout()?.height ?? 0}px`, transform: `translate(-50%,-50%) rotate(${layout()?.angle ?? 0}deg)` }} onLoad={event => void loaded(event.currentTarget)} onError={failed} /></div>}</Show>
    <Show when={state()}><div class="pin-hover-tools" role="toolbar" aria-label={t('图片工具')}>
      <button title={t('引用')} aria-label={t('引用')} disabled={referencing()} onClick={()=>void reference()}><Link2 size={15}/></button>
      <button title={t('复制')} aria-label={t('复制')} onClick={()=>void exportImage(true)}><Copy size={15}/></button>
      <button title={t('保存')} aria-label={t('保存')} onClick={()=>void exportImage(false)}><Download size={15}/></button>
      <button title={t('还原')} aria-label={t('还原')} onClick={()=>void pin.controlPin({type:'restore'}).catch(report)}><RotateCcw size={15}/></button>
      <button title={t('置顶')} aria-label={t('置顶')} classList={{selected:state()?.topmost}} onClick={()=>void pin.controlPin({type:'set_topmost',enabled:!state()?.topmost}).catch(report)}><Pin size={15}/></button>
      <button title={t('关闭贴图')} aria-label={t('关闭贴图')} onClick={()=>void close()}><X size={15}/></button>
    </div></Show>
    <Show when={error()}><div class="pin-error" role="alert"><span>{error()}</span><button title={t("关闭贴图")} aria-label={t("关闭贴图")} onClick={() => void close()}>×</button></div></Show>
  </main>;
}
