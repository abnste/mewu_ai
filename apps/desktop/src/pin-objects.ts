// SPDX-License-Identifier: MPL-2.0
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { native } from './bridge';
import type { Snapshot } from './contracts';
export interface PinObject {
  id: string; imageUrl: string; width: number; height: number; quarterTurns: number; opacity: number; topmost: boolean;
  attachmentAssetId: string | null; x: number; y: number; displayWidth: number; displayHeight: number;
}
export function validPinObject(value: PinObject): boolean {
  try {
    const url = new URL(value.imageUrl);
    return typeof value.id === 'string' && /^[0-9a-f-]{36}$/i.test(value.id) && url.protocol === 'http:' && url.hostname === '127.0.0.1' && !!url.port && !url.username && !url.password
      && [value.x, value.y, value.displayWidth, value.displayHeight].every(Number.isFinite) && value.displayWidth > 0 && value.displayHeight > 0
      && [value.width, value.height].every(n => Number.isSafeInteger(n) && n > 0) && Number.isInteger(value.quarterTurns) && value.quarterTurns >= 0 && value.quarterTurns < 4;
  } catch { return false; }
}
export async function controlPinObject(id: string, action: { type: 'close' } | { type: 'move'; x: number; y: number } | {type:'zoom';direction:1|-1}): Promise<void> {
  if (native) await invoke('control_pin_object', { id, action });
}
export async function referencePinObject(id: string | null, sceneId: string | null): Promise<Snapshot | null> {
  if (!native) throw Error('请在桌面版使用贴图');
  return invoke('reference_pin_object', { id, sceneId });
}
// Event notifications contain no object authority. Always read the live host
// registry; serials reject a slower response after a newer close/move refresh.
export async function subscribePinObjects(accept: (objects: PinObject[]) => void, error: (cause: unknown) => void): Promise<{ refresh: () => Promise<void>; stop: () => void }> {
  if (!native) return { refresh: async () => {}, stop: () => {} };
  let disposed = false, serial = 0;
  const refresh = async () => {
    const token = ++serial;
    try {
      const result = await invoke<PinObject[]>('get_pin_objects');
      if (!disposed && token === serial) {
        if (!Array.isArray(result) || !result.every(validPinObject)) throw Error('无法读取贴图对象');
        accept(result);
      }
    } catch (cause) { if (!disposed && token === serial) error(cause); }
  };
  const unlisten = await listen('pin-objects-changed', () => { void refresh(); });
  const resize = () => { void refresh(); };
  if(typeof window !== 'undefined') window.addEventListener('resize',resize);
  await refresh();
  return { refresh, stop: () => { disposed = true; serial++; unlisten(); if(typeof window !== 'undefined') window.removeEventListener('resize',resize); } };
}
