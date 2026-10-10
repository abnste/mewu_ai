// SPDX-License-Identifier: MPL-2.0
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export async function subscribeImportPins(onError: (message: string) => void) {
  if (!isTauri()) return () => undefined;
  let disposed = false;
  const stop = await listen<{ error?: unknown }>('import-pin-result', event => {
    if (!disposed && typeof event.payload?.error === 'string' && event.payload.error) onError(event.payload.error);
  }, { target: 'space' });
  return () => { disposed = true; stop(); };
}
