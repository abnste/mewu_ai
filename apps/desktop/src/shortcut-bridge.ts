// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CaptureShortcutState, ShortcutChord, ShortcutEditLease, ShortcutRecorded } from './shortcut-contracts';
export const shortcutNative = isTauri();
const unsupported = () => new Error('请在桌面版设置截图快捷键');
export async function getCaptureShortcut(): Promise<CaptureShortcutState> {
  if (!shortcutNative) return { revision: 0, sequence: 0, configured: null, active: null, status: 'unavailable', editable: false, message: unsupported().message };
  return invoke('get_capture_shortcut');
}
export async function setCaptureShortcut(expectedRevision: number, shortcut: ShortcutChord | null): Promise<CaptureShortcutState> {
  if (!shortcutNative) throw unsupported();
  return invoke('set_capture_shortcut', { expectedRevision, shortcut });
}
export async function beginCaptureShortcutEdit(leaseId: string): Promise<ShortcutEditLease> {
  if (!shortcutNative) throw unsupported();
  return invoke('begin_capture_shortcut_edit', { leaseId });
}
export async function endCaptureShortcutEdit(leaseId: string): Promise<void> {
  if (shortcutNative) await invoke('end_capture_shortcut_edit', { leaseId });
}
export async function subscribeCaptureShortcut(hooks: { state: (value: CaptureShortcutState) => void; recorded: (value: ShortcutRecorded) => void; ended: (leaseId: string) => void }): Promise<() => void> {
  if (!shortcutNative) return () => {};
  const stops: (() => void)[] = []; let disposed = false;
  try {
    stops.push(await listen<CaptureShortcutState>('capture-shortcut-state', event => { if (!disposed) hooks.state(event.payload); }, { target: 'settings' }));
    stops.push(await listen<ShortcutRecorded>('capture-shortcut-recorded', event => { if (!disposed) hooks.recorded(event.payload); }, { target: 'settings' }));
    stops.push(await listen<{ leaseId: string }>('capture-shortcut-edit-ended', event => { if (!disposed) hooks.ended(event.payload.leaseId); }, { target: 'settings' }));
  } catch (error) { disposed = true; for (const stop of stops) stop(); throw error; }
  return () => { disposed = true; for (const stop of stops) stop(); };
}
