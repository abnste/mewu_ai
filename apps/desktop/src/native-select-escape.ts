// SPDX-License-Identifier: MPL-2.0
/** Native select pickers own Escape before window/editor cancellation listeners. */
export function nativeSelectOwnsEscape(event: Pick<KeyboardEvent, 'key' | 'target'>): boolean {
  if (event.key !== 'Escape' || typeof document === 'undefined') return false;
  const target = typeof Element !== 'undefined' && event.target instanceof Element ? event.target : document.activeElement;
  if (target?.closest('select,option,optgroup') || document.activeElement?.closest('select,option,optgroup')) return true;
  try { return Boolean(document.querySelector('select:open')); }
  catch { return false; } // Older WebViews still use the event/focus target above.
}
