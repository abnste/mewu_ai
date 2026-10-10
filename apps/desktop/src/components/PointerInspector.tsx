// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, Show } from 'solid-js';
import type { Asset } from '../contracts';
import { getPointerSample } from '../pointer-bridge';
import { PointerInspector as SampleGate, PointerSampleCanceledError, pointerImagePoint, pointerInspectorSize, pointerPopupPosition, type PointerImageBox, type PointerSample, type PointerSampleRequest } from '../pointer-inspector';
import './pointer-inspector.css';

export interface PointerInspectorHandle { copy: () => boolean }
interface Props {
  sceneId: string; background: Asset; imageBox: PointerImageBox;
  dimensions?: string;
  disabled: boolean; blockedAt: (x: number, y: number) => boolean;
  onReady: (handle: PointerInspectorHandle | undefined) => void;
  onError: (message: string) => void;
}
const blockedElements = '.video-trim-popover,.selection-code-card,.composer,.artifact-card,.capture-image-override,.capture-toolbar,.ocr-text-layer,.ocr-selection-menu,.ocr-actions,.translation-actions,.space-utilities,button,a,input,textarea,select,[contenteditable]:not([contenteditable="false"])';
const modalElements = '.modal-backdrop,.history-backdrop,.channel-popover,.reply-table-menu,[role="menu"],.composer.dragging';

export default function PointerInspector(props: Props) {
  let popup: HTMLDivElement | undefined, observer: ResizeObserver | undefined;
  let pointer: { x: number; y: number } | undefined, request: PointerSampleRequest | undefined;
  let disposed = false, version = 0, copying = false;
  const [sample, setSample] = createSignal<PointerSample>();
  const [anchor, setAnchor] = createSignal({ x: 0, y: 0 });
  const [size, setSize] = createSignal(pointerInspectorSize(false));
  const [viewport, setViewport] = createSignal({ width: innerWidth, height: innerHeight });
  const gate = new SampleGate(getPointerSample, value => { if (!disposed) setSample(value); });
  const coordinates = () => sample() ? `X ${sample()!.globalX} Y ${sample()!.globalY}` : '';
  const coordinateWidth = () => Math.max(82, coordinates().length * 6.1);
  const dimensionWidth = () => Math.max(82, (props.dimensions?.length ?? 0) * 6.1);
  const location = createMemo(() => pointerPopupPosition(anchor().x, anchor().y, viewport().width, viewport().height, { width: size().width, height: Math.max(size().height, pointerInspectorSize(Boolean(props.dimensions)).height) }));
  function allowed(point: { x: number; y: number }): boolean {
    if (disposed || props.disabled || !document.hasFocus() || props.blockedAt(point.x, point.y) || document.querySelector(modalElements)) return false;
    const target = document.elementFromPoint(point.x, point.y);
    return Boolean(target?.closest('.capture-surface') && !target.closest(blockedElements));
  }
  function hide() { if (!pointer && !request && !sample()) return; version++; pointer = undefined; request = undefined; gate.cancel(); }
  function move(event: PointerEvent) {
    const point = { x: event.clientX, y: event.clientY };
    if (!allowed(point)) { hide(); return; }
    const pixel = pointerImagePoint(point.x, point.y, props.imageBox, { width: props.background.width ?? 0, height: props.background.height ?? 0 });
    if (!pixel) { hide(); return; }
    if (!pointer || pointer.x !== point.x || pointer.y !== point.y) { version++; pointer = point; setAnchor(point); }
    request = { sceneId: props.sceneId, backgroundId: props.background.id, ...pixel };
    gate.request(request);
  }
  function copy(): boolean {
    const target = request, point = pointer;
    if (!target || !point || !allowed(point)) { hide(); return false; }
    if (copying) return true;
    const token = version; copying = true;
    const current = () => !disposed && version === token && request?.sceneId === target.sceneId && request?.backgroundId === target.backgroundId && request?.x === target.x && request?.y === target.y && props.sceneId === target.sceneId && props.background.id === target.backgroundId && allowed(point);
    void gate.sampleExact(target).then(async value => { if (current()) await navigator.clipboard.writeText(value.hex); }).catch(error => {
      if (current() && !(error instanceof PointerSampleCanceledError)) props.onError(error instanceof Error ? error.message : String(error));
    }).finally(() => { copying = false; });
    return true;
  }
  const recheck = () => { if (pointer && !allowed(pointer)) hide(); };
  const resize = () => { hide(); setViewport({ width: innerWidth, height: innerHeight }); };
  const focus = (event: FocusEvent) => { if (event.target instanceof Element && event.target.closest(blockedElements)) hide(); };
  const visibility = () => { if (document.hidden) hide(); };
  const identity = createMemo(() => `${props.sceneId}:${props.background.id}:${props.imageBox.x}:${props.imageBox.y}:${props.imageBox.width}:${props.imageBox.height}:${props.disabled}`);
  createEffect(on(identity, hide, { defer: true }));
  const visible = createMemo(() => Boolean(sample()));
  createEffect(on(visible, showing => {
    if (!showing) { observer?.disconnect(); observer = undefined; return; }
    queueMicrotask(() => {
      if (disposed || !sample() || !popup) return;
      observer?.disconnect(); observer = new ResizeObserver(() => { if (!disposed && popup && sample()) setSize({ width: popup.offsetWidth, height: popup.offsetHeight }); }); observer.observe(popup);
    });
  }));
  const mutations = new MutationObserver(recheck);
  mutations.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'hidden'] });
  window.addEventListener('pointermove', move); window.addEventListener('pointerdown', recheck, true); window.addEventListener('blur', hide); window.addEventListener('resize', resize);
  document.documentElement.addEventListener('pointerleave', hide); document.addEventListener('focusin', focus); document.addEventListener('visibilitychange', visibility);
  props.onReady({ copy });
  onCleanup(() => {
    disposed = true; version++; gate.dispose(); observer?.disconnect(); mutations.disconnect(); props.onReady(undefined);
    window.removeEventListener('pointermove', move); window.removeEventListener('pointerdown', recheck, true); window.removeEventListener('blur', hide); window.removeEventListener('resize', resize);
    document.documentElement.removeEventListener('pointerleave', hide); document.removeEventListener('focusin', focus); document.removeEventListener('visibilitychange', visibility);
  });
  return <Show when={sample() && location()}><div ref={popup} class="pointer-inspector" style={{ left: `${location()!.left}px`, top: `${location()!.top}px` }} aria-hidden="true">
    <div class="pointer-color"><i style={{ background: sample()!.hex }} /><span>{sample()!.hex}</span></div>
    <div class="pointer-magnifier"><img src={sample()!.dataUrl} alt="" draggable={false} onError={hide} /><svg viewBox="0 0 90 90"><path class="pointer-cross-outline" d="M0 45H41.2 M48.8 45H90 M45 0V41.2 M45 48.8V90" /><path class="pointer-cross" d="M0 45H41.2 M48.8 45H90 M45 0V41.2 M45 48.8V90" /><rect class="pointer-pixel-outline" x="43.2" y="43.2" width="3.6" height="3.6" /><rect class="pointer-pixel" x="43.2" y="43.2" width="3.6" height="3.6" /></svg></div>
    <div class="pointer-coordinate-footer">
      <div class="pointer-coordinates"><svg viewBox={`0 0 ${coordinateWidth()} 16`} preserveAspectRatio="xMidYMid meet"><text x={coordinateWidth() / 2} y="12" text-anchor="middle">{coordinates()}</text></svg></div>
      <Show when={props.dimensions}><div class="pointer-dimensions"><svg viewBox={`0 0 ${dimensionWidth()} 16`} preserveAspectRatio="xMidYMid meet"><text x={dimensionWidth() / 2} y="12" text-anchor="middle">{props.dimensions}</text></svg></div></Show>
    </div>
  </div></Show>;
}
