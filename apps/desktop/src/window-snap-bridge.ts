// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { SnapMapRequest } from './window-snap';

interface Pending { request: SnapMapRequest; resolve: (value: unknown) => void; reject: (cause: unknown) => void }
let active = false;
let latest: Pending | undefined;
function dispatch(pending: Pending) {
  active = true;
  let response: Promise<unknown>;
  try { response = invoke('get_window_snap_map', { sceneId: pending.request.sceneId, backgroundId: pending.request.backgroundId }); }
  catch (error) { response = Promise.reject(error); }
  void response.then(pending.resolve, pending.reject).finally(() => {
    active = false;
    const next = latest; latest = undefined;
    if (next) dispatch(next);
  });
}

/** Browser preview has no desktop map. The private fixture injects synthetic IPC only. */
export async function getWindowSnapMap(request: SnapMapRequest): Promise<unknown> {
  if (!isTauri()) return null;
  // A keyed scene change creates a new Canvas/loader. Keep the real IPC slot here
  // until it settles, so unmount cannot bypass the whole-space concurrency bound.
  return new Promise((resolve, reject) => {
    const pending: Pending = { request: { ...request }, resolve, reject };
    if (active) { latest?.resolve(null); latest = pending; }
    else dispatch(pending);
  });
}
