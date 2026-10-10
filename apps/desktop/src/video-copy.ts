// SPDX-License-Identifier: MPL-2.0
/** Bare C belongs to one active video; text/native controls retain their keys. */
export function videoCopyKey(event: Pick<KeyboardEvent, 'key' | 'keyCode' | 'defaultPrevented' | 'isComposing' | 'repeat' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>, scope: { active: boolean; blocked: boolean; focused: boolean; selectedText: boolean; target: Element | null; sceneId: string; itemId: string }): boolean {
  if (event.key.toLowerCase() !== 'c' || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.repeat || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || !scope.active || scope.blocked || !scope.focused || scope.selectedText) return false;
  const target = scope.target;
  if (target?.closest('input,textarea,select,button,a,[contenteditable]:not([contenteditable="false"]),.composer,.ocr-text-layer,.text-artifact,.selection-code-card,[data-run-journal],[role="menu"],.drawing-editor')) return false;
  const owner = target?.closest('[data-video-item]');
  if (owner && (owner.getAttribute('data-video-item') !== scope.itemId || owner.getAttribute('data-video-scene') !== scope.sceneId)) return false;
  if (target?.closest('.video-trim-bar[data-trim-interacting="true"]')) return false;
  return Boolean(owner || !target || target === target.ownerDocument.body || target.closest('.capture-surface') && !target.closest('.artifact-card'));
}
