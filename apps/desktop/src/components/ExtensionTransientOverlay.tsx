// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { listen } from '@tauri-apps/api/event';
import * as extensions from '../extension-bridge';
import type { ExtensionRecord } from '../extension-contracts';
import { mergeExtensionRecords } from '../extension-state';
import { clampTransient, isTransientEscape, transientPointerSample, transientPortBridge } from '../extension-transient-overlay';
import './extension-surfaces.css';

interface Props { sceneId: string; disabled: boolean; onError: (message: string) => void }
interface TransientSurface { surfaceId: string; url: string; width: number; height: number; pluginId: string; revision: number; serviceId: string; sceneId: string }
/** A gesture-scoped overlay follows the pointer with pointer-events:none, so
 * selection drags underneath keep working; pointerup, Escape or blur destroys
 * the frame and closes the server-side document. */
export default function ExtensionTransientOverlay(props: Props) {
  const [mounted, setMounted] = createSignal<TransientSurface>();
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]);
  let disposed = false, ready = false, failed = false;
  const subscriptions: Array<() => void> = [];
  let frame: HTMLIFrameElement | undefined;
  let connection: ReturnType<typeof transientPortBridge> | undefined;
  let latest: { x: number; y: number; buttons: number } = { x: innerWidth / 2, y: innerHeight / 2, buttons: 0 };
  let frameRequest = 0, attached = false;
  const report = (error: unknown) => { if (!disposed) props.onError(error instanceof Error ? error.message : String(error)); };
  // Subscribe before reading the catalog. A late initial read must not undo a
  // newer revoke event. Until authority is known, reject and release documents.
  const track = async (subscription: Promise<() => void>) => {
    const stop = await subscription;
    if (disposed || failed) stop(); else subscriptions.push(stop);
  };
  const update = (incoming: ExtensionRecord[]) => {
    if (!disposed && !failed) setRecords(previous => mergeExtensionRecords(previous, incoming));
  };
  if (extensions.extensionNative) void (async () => {
    try {
      await Promise.all([
        track(extensions.subscribeExtensions(update)),
        track(extensions.subscribeSurfaceClosed(id => { if (mounted()?.surfaceId === id) destroy(); })),
        track(listen<TransientSurface>('extension-transient-surface', event => accept(event.payload), { target: 'space' })),
      ]);
      if (disposed || failed) return;
      update(await extensions.getExtensions());
      if (!disposed && !failed) ready = true;
    } catch (error) {
      failed = true; ready = false;
      subscriptions.splice(0).forEach(stop => stop());
      destroy(); report(error);
    }
  })();
  const authorized = (value: TransientSurface) => records().some(record =>
    record.manifest.id === value.pluginId && record.revision === value.revision && record.state === 'enabled'
    && record.manifest.permissions.includes('ui.overlay') && !record.platformError
    && !record.unsupportedServices?.[value.serviceId] && record.services.some(service => service.id === value.serviceId));
  // The overlay must appear at the pointer, whose position predates the popup.
  const onPointer = (event: PointerEvent) => {
    const sample = transientPointerSample(event);
    if (!sample) return;
    latest = sample;
    if (mounted() && !frameRequest) frameRequest = requestAnimationFrame(follow);
  };
  const onKey = (event: KeyboardEvent) => {
    if (!isTransientEscape(event)) return;
    // Both this handler and hold triggers run at window capture. Do not use
    // stopImmediatePropagation: the same Escape must also cancel an armed hold,
    // regardless of registration order, before reaching screenshot handlers.
    event.preventDefault(); event.stopPropagation();
    destroy();
  };
  const onEnd = () => destroy();
  function attach() {
    if (attached) return;
    attached = true;
    addEventListener('keydown', onKey, true);
    addEventListener('pointerup', onEnd, true);
    addEventListener('pointercancel', onEnd, true);
    addEventListener('blur', onEnd);
  }
  function detach() {
    if (!attached) return;
    attached = false;
    removeEventListener('keydown', onKey, true);
    removeEventListener('pointerup', onEnd, true);
    removeEventListener('pointercancel', onEnd, true);
    removeEventListener('blur', onEnd);
  }
  function follow() {
    frameRequest = 0;
    const value = mounted();
    if (!value) return;
    const placement = clampTransient(latest.x, latest.y, value.width, value.height, innerWidth, innerHeight);
    if (frame) { frame.style.left = `${placement.left}px`; frame.style.top = `${placement.top}px`; }
    connection?.pointer(latest);
  }
  function accept(value: TransientSurface) {
    if (!value || typeof value.surfaceId !== 'string' || typeof value.url !== 'string') return;
    if (disposed || failed || !ready || props.disabled || value.sceneId !== props.sceneId || !authorized(value)) {
      release(value);
      return;
    }
    if (mounted()?.surfaceId === value.surfaceId) return;
    destroy();
    setMounted(value);
    // Reactive scene/disabled changes may have destroyed it during setMounted.
    if (mounted() === value) attach();
  }
  function release(value: TransientSurface) {
    void extensions.invokeExtension({ pluginId: value.pluginId, revision: value.revision, serviceId: value.serviceId, sceneId: value.sceneId, surface: { action: 'close', id: value.surfaceId }, input: null }).catch(() => {});
  }
  function destroy() {
    detach();
    connection?.close(); connection = undefined;
    if (frameRequest) { cancelAnimationFrame(frameRequest); frameRequest = 0; }
    frame = undefined;
    const value = mounted();
    setMounted(undefined);
    if (value) release(value);
  }
  function loaded() {
    connection?.close(); connection = undefined;
    if (!frame?.contentWindow || !mounted() || disposed) return;
    const channel = new MessageChannel();
    connection = transientPortBridge(channel.port1, destroy);
    frame.contentWindow.postMessage({ type: 'mewu-connect' }, '*', [channel.port2]);
  }
  createEffect(() => {
    const value = mounted();
    if (value && (props.disabled || value.sceneId !== props.sceneId || !authorized(value))) destroy();
  });
  addEventListener('pointermove', onPointer, true);
  onCleanup(() => {
    disposed = true;
    removeEventListener('pointermove', onPointer, true);
    ready = false;
    subscriptions.splice(0).forEach(stop => stop());
    destroy();
  });
  return <Show when={mounted()} keyed>{value => {
    const initial = clampTransient(latest.x, latest.y, value.width, value.height, innerWidth, innerHeight);
    return <Portal><iframe ref={frame} class="extension-transient-frame" sandbox="allow-scripts" allow="" referrerpolicy="no-referrer"
      title="插件瞬态弹层" src={value.url} onLoad={loaded}
      style={{ width: `${value.width}px`, height: `${value.height}px`, left: `${initial.left}px`, top: `${initial.top}px` }} /></Portal>;
  }}</Show>;
}
