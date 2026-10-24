// SPDX-License-Identifier: MPL-2.0
/** Placement and lifecycle rules for transient pointer overlays. Pure and
 * JSX-free so the node test bench can load them without a bundler. */
export interface TransientPlacement { left: number; top: number }
/** Follow at a fixed offset below-right of the pointer, clamped into the
 * viewport with a margin; a viewport that cannot fit the frame centers it. */
export function clampTransient(x: number, y: number, width: number, height: number,
  viewportWidth: number, viewportHeight: number, margin = 16, offset = 12): TransientPlacement {
  const axis = (desired: number, size: number, viewport: number) =>
    viewport < size + 2 * margin
      ? Math.max(0, (viewport - size) / 2)
      : Math.max(margin, Math.min(desired, viewport - size - margin));
  return { left: axis(x + offset, width, viewportWidth), top: axis(y + offset, height, viewportHeight) };
}
export function isTransientEscape(event: { key?: unknown; isComposing?: unknown; keyCode?: unknown }): boolean {
  return event.key === 'Escape' && !event.isComposing && event.keyCode !== 229;
}
export function transientPointerSample(event: { clientX?: unknown; clientY?: unknown; buttons?: unknown }): { x: number; y: number; buttons: number } | undefined {
  const { clientX: x, clientY: y, buttons } = event;
  if (typeof x !== 'number' || typeof y !== 'number' || !Number.isFinite(x) || !Number.isFinite(y)) return;
  if (typeof buttons !== 'number' || !Number.isSafeInteger(buttons) || buttons < 0 || buttons > 31) return;
  return { x, y, buttons };
}
/** Minimal inbound bridge for a transient frame: only `escape` destroys it,
 * every other message is ignored. Pointer samples flow the other way. */
export function transientPortBridge(port: Pick<MessagePort, 'onmessage' | 'postMessage' | 'close'>, destroy: () => void) {
  port.onmessage = ({ data }) => {
    if (!data || typeof data !== 'object') return;
    if (data.type === 'escape') destroy();
  };
  return {
    pointer(value: { x: number; y: number; buttons: number }) { port.postMessage({ type: 'event', name: 'pointer', value }); },
    close() { port.onmessage = null; port.close(); }
  };
}
