// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { OcrTarget } from './contracts';
import { PinRevisionGate } from './pin-interaction';
export interface PinViewState { id: string; revision: number; imageUrl: string; width: number; height: number; quarterTurns: number; topmost: boolean; opacity: number; scaleFactor: number; shadowPadding: number; dragThresholdX: number; dragThresholdY: number }
export interface PinRequest { requestId: string; pluginId: string; revision: number; contributionId: string; target: OcrTarget; translationId: string | null }
export type PinAction = { type: 'close' | 'restore' | 'escape' } | { type: 'zoom'; direction: 1 | -1 } | { type: 'rotate'; quarterTurns: 1 | -1 } | { type: 'set_topmost'; enabled: boolean } | { type: 'set_opacity'; opacity: .8 | 1 };
export const pinNative = isTauri();
function desktop() { if (!pinNative) throw new Error('请在桌面版使用贴图'); }
export async function runPin(request: PinRequest): Promise<{ id: string }> { desktop(); return invoke('run_plugin_pin', { requestId: request.requestId, pluginId: request.pluginId, revision: request.revision, contributionId: request.contributionId, target: request.target, translationId: request.translationId }); }
export async function cancelPin(requestId: string): Promise<void> { if (pinNative) await invoke('cancel_plugin_pin', { requestId }); }
export async function pinReady(success: boolean): Promise<void> { desktop(); await invoke('pin_ready', { success }); }
export async function controlPin(action: PinAction): Promise<void> { desktop(); await invoke('control_pin', { action }); }
export async function exportPin(clipboard: boolean): Promise<void> { desktop(); await invoke('export_pin', { clipboard }); }
export async function showPinMenu(): Promise<void> { desktop(); await invoke('show_pin_menu'); }
export async function startPinDrag(): Promise<void> { desktop(); await invoke('start_pin_drag'); }
export async function subscribePinEscape(accept: () => void): Promise<() => void> { return pinNative ? listen('pin-escape', accept, { target: 'space' }) : () => undefined; }
export function validPinState(value: PinViewState): boolean {
  try {
    const url = new URL(value.imageUrl);
    return url.protocol === 'http:' && url.hostname === '127.0.0.1' && Boolean(url.port) && !url.username && !url.password && !url.hash
      && Number.isSafeInteger(value.width) && value.width > 0 && Number.isSafeInteger(value.height) && value.height > 0
      && Number.isInteger(value.quarterTurns) && Number.isFinite(value.scaleFactor) && value.scaleFactor > 0
      && Number.isFinite(value.shadowPadding) && value.shadowPadding >= 0 && [value.dragThresholdX, value.dragThresholdY].every(number => Number.isFinite(number) && number >= 0)
      && (value.opacity === .8 || value.opacity === 1);
  } catch { return false; }
}
export async function subscribePin(accept: (value: PinViewState) => void, error: (message: string) => void): Promise<() => void> {
  desktop(); const gate = new PinRevisionGate(), stops: (() => void)[] = [];
  const state = (value: PinViewState) => { if (!validPinState(value)) { error('无法读取贴图'); return; } if (gate.accept(value)) accept(value); };
  let disposed = false;
  const stop = () => { disposed = true; gate.dispose(); stops.forEach(value => value()); };
  try {
    stops.push(await listen<PinViewState>('pin-state', event => { if (!disposed) state(event.payload); }));
    stops.push(await listen<{ message: string }>('pin-error', event => { if (!disposed && typeof event.payload?.message === 'string') error(event.payload.message); }));
    state(await invoke<PinViewState>('get_pin_state'));
    return stop;
  } catch (cause) { stop(); throw cause; }
}
