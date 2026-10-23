// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { listen } from '@tauri-apps/api/event';
import * as extensions from '../extension-bridge';
import { clampTransient, isTransientEscape, transientPointerSample, transientPortBridge } from '../extension-transient-overlay';
import './extension-surfaces.css';

interface Props { sceneId: string; disabled: boolean; onError: (message: string) => void }
interface TransientSurface { surfaceId: string; url: string; width: number; height: number; pluginId: string; revision: number; serviceId: string; sceneId: string }
/** A gesture-scoped overlay follows the pointer with pointer-events:none, so
 * selection drags underneath keep working; pointerup, Escape or blur destroys
 * the frame and closes the server-side document. */
export default function ExtensionTransientOverlay(props: Props) {
  const [mounted, setMounted] = createSignal<TransientSurface>();
  let disposed = false, stopEvents: (() => void) | undefined;
  let frame: HTMLIFrameElement | undefined;
  let connection: ReturnType<typeof transientPortBridge> | undefined;
  let latest: { x: number; y: number; buttons: number } = { x: innerWidth / 2, y: innerHeight / 2, buttons: 0 };
  let frameRequest = 0, attached = false;
  const report = (error: unknown) => { if (!disposed) props.onError(error instanceof Error ? error.message : String(error)); };
  if (extensions.extensionNative) void listen<TransientSurface>('extension-transient-surface', event => accept(event.payload), { target: 'space' })
    .then(stop => { if (disposed) stop(); else stopEvents = stop; }).catch(report);
  // The overlay must appear at the pointer, whose position predates the popup.
  const onPointer = (event: PointerEvent) => {
    const sample = transientPointerSample(event);
    if (!sample) return;
    latest = sample;
    if (mounted() && !frameRequest) frameRequest = requestAnimationFrame(follow);
  };
  const onKey = (event: KeyboardEvent) => {
    if (!isTransientEscape(event)) return;
    event.preventDefault(); event.stopImmediatePropagation();
    destroy();
  };
  const onEnd = () => destroy();
  function attach() {
    if (attached) return;
    attached = true;
    // Register before other Escape consumers, exactly like a pending pin: a
    // pointer-tied frame is the topmost authority for its whole short life.
    document.addEventListener('keydown', onKey, true);
    addEventListener('pointerup', onEnd, true);
    addEventListener('pointercancel', onEnd, true);
    addEventListener('blur', onEnd);
  }
  function detach() {
    if (!attached) return;
    attached = false;
    document.removeEventListener('keydown', onKey, true);
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
    if (disposed || !value || typeof value.surfaceId !== 'string' || typeof value.url !== 'string') return;
    if (value.sceneId !== props.sceneId) {
      // A stale scene never shows; still release its server-side document.
      void extensions.invokeExtension({ pluginId: value.pluginId, revision: value.revision, serviceId: value.serviceId, sceneId: value.sceneId, surface: { action: 'close', id: value.surfaceId }, input: null }).catch(() => {});
      return;
    }
    destroy();
    setMounted(value);
    attach();
  }
  function destroy() {
    detach();
    connection?.close(); connection = undefined;
    if (frameRequest) { cancelAnimationFrame(frameRequest); frameRequest = 0; }
    frame = undefined;
    const value = mounted();
    setMounted(undefined);
    if (value) void extensions.invokeExtension({ pluginId: value.pluginId, revision: value.revision, serviceId: value.serviceId, sceneId: value.sceneId, surface: { action: 'close', id: value.surfaceId }, input: null }).catch(() => {});
  }
  function loaded() {
    connection?.close(); connection = undefined;
    if (!frame?.contentWindow || !mounted() || disposed) return;
    const channel = new MessageChannel();
    connection = transientPortBridge(channel.port1, destroy);
    frame.contentWindow.postMessage({ type: 'mewu-connect' }, '*', [channel.port2]);
  }
  createEffect(() => { if (props.disabled) destroy(); });
  addEventListener('pointermove', onPointer, true);
  onCleanup(() => {
    disposed = true;
    removeEventListener('pointermove', onPointer, true);
    stopEvents?.();
    destroy();
  });
  return <Show when={mounted()} keyed>{value => {
    const initial = clampTransient(latest.x, latest.y, value.width, value.height, innerWidth, innerHeight);
    return <Portal><iframe ref={frame} class="extension-transient-frame" sandbox="allow-scripts" allow="" referrerpolicy="no-referrer"
      title="插件瞬态弹层" src={value.url} onLoad={loaded}
      style={{ width: `${value.width}px`, height: `${value.height}px`, left: `${initial.left}px`, top: `${initial.top}px` }} /></Portal>;
  }}</Show>;
}
