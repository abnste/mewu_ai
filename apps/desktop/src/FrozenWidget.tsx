// SPDX-License-Identifier: MPL-2.0
import { t } from "./i18n";
import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Check, ChevronDown, CircleAlert, LoaderCircle, MessageSquare, X } from 'lucide-solid';
import './frozen.css';

interface FrozenScene {
  id: string;
  title: string;
  preview: string;
  status: 'idle' | 'running' | 'completed' | 'failed' | 'canceled';
  updatedAt: number;
}
interface FrozenSnapshot { revision: number; scenes: FrozenScene[] }

export default function FrozenWidget() {
  document.documentElement.dataset.surface = 'frozen';
  const [snapshot, setSnapshot] = createSignal<FrozenSnapshot>({ revision: -1, scenes: [] });
  const [expanded, setExpanded] = createSignal(false);
  const [restoring, setRestoring] = createSignal(false);
  const [closing, setClosing] = createSignal<string>();
  const [hiding, setHiding] = createSignal(false);
  const [resizing, setResizing] = createSignal(false);
  const [error, setError] = createSignal('');
  const primary = createMemo(() => snapshot().scenes[0]);
  const busy = () => restoring() || Boolean(closing()) || hiding();
  let disposed = false;
  let press: { x: number; y: number; pointerId: number; sceneId?: string; dragging: boolean; element: HTMLElement } | undefined;
  let suppressClick = false;
  let closePressedId: string | undefined;
  const stops: (() => void)[] = [];

  const accept = (next: FrozenSnapshot) => {
    if (!disposed && next.revision >= snapshot().revision) setSnapshot(next);
  };
  onMount(async () => {
    if (!isTauri()) return;
    try {
      for (const subscribe of [
        () => listen<FrozenSnapshot>('frozen-scenes', event => accept(event.payload)),
        () => listen<boolean>('frozen-widget-layout', event => setExpanded(event.payload)),
      ]) {
        const stop = await subscribe();
        if (disposed) stop(); else stops.push(stop);
      }
      accept(await invoke<FrozenSnapshot>('get_frozen_scenes'));
    } catch (cause) { if (!disposed) setError(String(cause)); }
  });
  onCleanup(() => { disposed = true; cancelPress(); stops.forEach(stop => stop()); });

  async function toggle() {
    if (busy() || resizing()) return;
    setResizing(true);
    try {
      setError('');
      const next = !expanded();
      await invoke('resize_frozen_widget', { expanded: next });
      setExpanded(next);
    } catch (cause) { setError(String(cause)); }
    finally { setResizing(false); }
  }
  async function restore(sceneId: string) {
    if (busy()) return;
    setRestoring(true); setError('');
    try { await invoke('restore_frozen_scene', { sceneId }); }
    catch (cause) { setError(String(cause)); }
    finally { setRestoring(false); }
  }
  function cancelPress() {
    if (press?.element.hasPointerCapture(press.pointerId)) press.element.releasePointerCapture(press.pointerId);
    press = undefined;
  }
  function closePointerDown(event: PointerEvent, sceneId?: string) {
    event.stopPropagation(); cancelPress(); closePressedId = sceneId;
  }
  function closeClick(event: MouseEvent, sceneId?: string) {
    event.stopPropagation();
    const target = event.detail === 0 ? sceneId : closePressedId ?? sceneId;
    closePressedId = undefined;
    if (target) void closeScene(target);
  }
  async function closeScene(sceneId: string) {
    if (busy()) return;
    cancelPress(); setClosing(sceneId); setError('');
    try { await invoke('close_scene_window', { sceneId }); }
    catch (cause) { setError(String(cause)); }
    finally { setClosing(undefined); }
  }
  async function hideWidget() {
    if (busy()) return;
    cancelPress(); setHiding(true); setError('');
    try { await invoke('hide_frozen_widget'); }
    catch (cause) { setError(String(cause)); }
    finally { setHiding(false); }
  }
  function pointerDown(event: PointerEvent) {
    if (event.button !== 0 || busy() || (event.target as Element).closest('.frozen-count,.frozen-close')) return;
    const element = (event.target as Element).closest('.frozen-restore') as HTMLElement | null;
    if (!element) return;
    suppressClick = false;
    press = { x: event.clientX, y: event.clientY, pointerId: event.pointerId, sceneId: primary()?.id, dragging: false, element };
    element.setPointerCapture(event.pointerId);
  }
  async function pointerMove(event: PointerEvent) {
    if (!press || press.pointerId !== event.pointerId || press.dragging || !(event.buttons & 1)) return;
    if (Math.hypot(event.clientX - press.x, event.clientY - press.y) < 5) return;
    press.dragging = true; suppressClick = true;
    const target = press.element;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    try { await getCurrentWindow().startDragging(); }
    catch (cause) { setError(String(cause)); }
    finally { press = undefined; }
  }
  function primaryClick(event: MouseEvent) {
    // Native drag may deliver a click after its pointer loop ends. Suppress it
    // until a fresh pointer press, while keeping keyboard activation accessible.
    if (event.detail !== 0 && suppressClick) { event.preventDefault(); return; }
    const sceneId = press?.sceneId ?? primary()?.id;
    press = undefined;
    if (sceneId) void restore(sceneId);
  }
  const status = (scene: FrozenScene) => {
    if (scene.status === 'running') return t('正在回复…');
    if (scene.status === 'failed') return t('回复失败');
    if (scene.status === 'canceled') return t('已停止');
    if (scene.status === 'completed') return t('回答已完成');
    return scene.preview || t('点击恢复');
  };
  const symbol = (scene: FrozenScene) => <>
    <Show when={scene.status === 'running'}><LoaderCircle size={13} class="frozen-spin" /></Show>
    <Show when={scene.status === 'completed'}><Check size={13} class="frozen-complete" /></Show>
    <Show when={scene.status === 'failed'}><CircleAlert size={13} class="frozen-failed" /></Show>
  </>;

  return <main class="frozen-widget" classList={{ 'is-expanded': expanded() }}
    aria-label={t("冻结的会话")} onKeyDown={event => { if (event.key === 'Escape' && expanded()) void toggle(); }}>
    <Show when={expanded()}>
      <section class="frozen-list" aria-label={t("选择要恢复的会话")}>
        <header><span>{t("冻结的会话")}</span><span class="frozen-total">{snapshot().scenes.length}</span><button class="frozen-hide" title={t("隐藏浮窗")} aria-label={t("隐藏浮窗")} disabled={busy()} onClick={() => void hideWidget()}><X size={14} /></button></header>
        <div class="frozen-list-scroll">
          <For each={snapshot().scenes.map(scene => scene.id)}>{id => <Show when={snapshot().scenes.find(scene => scene.id === id)}>{scene => <div class="frozen-row">
            <button class="frozen-row-restore" disabled={busy()} onClick={() => void restore(id)} title={t("恢复：{0}").replaceAll("{0}", () => String(scene().title))}>
              <MessageSquare size={16} />
              <span class="frozen-copy"><strong>{scene().title}</strong><span>{status(scene())}</span></span>
              <span class="frozen-state" aria-label={scene().status}>{symbol(scene())}</span>
            </button>
            <button class="frozen-close frozen-row-close" disabled={busy()} title={t("关闭会话")} aria-label={t("关闭会话：{0}").replaceAll("{0}", () => String(scene().title))} onPointerDown={event => closePointerDown(event, id)} onPointerCancel={() => { closePressedId = undefined; }} onClick={event => closeClick(event, id)}><X size={13} /></button>
          </div>}</Show>}</For>
        </div>
      </section>
    </Show>
    <div class="frozen-pill" onPointerDown={pointerDown} onPointerMove={event => void pointerMove(event)}
      onPointerCancel={() => { suppressClick = true; press = undefined; }}>
      <button class="frozen-restore" disabled={!primary() || busy()}
        title={error() || t("恢复：{0}").replaceAll("{0}", () => String(primary()?.title ?? t('会话')))}
        onClick={primaryClick}>
        <span class="frozen-icon"><MessageSquare size={15} /></span>
        <span class="frozen-copy"><strong>{primary()?.title || t('冻结的会话')}</strong>
          <span classList={{ 'frozen-failed': Boolean(error()) }}>{error() || (primary() ? status(primary()!) : t('暂无会话'))}</span>
        </span>
      </button>
      <div class="frozen-primary-actions">
        <span class="frozen-primary-state frozen-state">{primary() && symbol(primary()!)}</span>
        <button class="frozen-close frozen-primary-close" title={t("关闭会话")} aria-label={t("关闭会话：{0}").replaceAll("{0}", () => String(primary()?.title ?? ''))} disabled={!primary() || busy()} onPointerDown={event => closePointerDown(event, primary()?.id)} onPointerCancel={() => { closePressedId = undefined; }} onClick={event => closeClick(event, primary()?.id)}><X size={13} /></button>
      <Show when={snapshot().scenes.length > 0}>
        <button class="frozen-count" disabled={busy() || resizing()} title={t("选择会话")} aria-expanded={expanded()} aria-label={t("选择 {0} 个冻结会话").replaceAll("{0}", () => String(snapshot().scenes.length))} onClick={() => void toggle()}>
          <span>{snapshot().scenes.length}</span><ChevronDown size={11} classList={{ turned: expanded() }} />
        </button>
      </Show>
      </div>
    </div>
  </main>;
}
