// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { PointerSample, PointerSampleRequest } from './pointer-inspector';

export const pointerNative = isTauri();
export async function getPointerSample(request: PointerSampleRequest): Promise<PointerSample> {
  if (!pointerNative) throw new Error('请在桌面版取色');
  return invoke('get_pointer_sample', { sceneId: request.sceneId, backgroundId: request.backgroundId, x: request.x, y: request.y });
}
