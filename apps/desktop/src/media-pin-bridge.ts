// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';

export interface SceneItemPinTarget {
  sceneId: string;
  itemId: string;
  expectedAssetId: string;
}

export async function pinSceneItem(target: SceneItemPinTarget): Promise<{ id: string }> {
  if (!isTauri()) throw new Error('请在桌面版使用贴图');
  return invoke('pin_scene_item', { sceneId: target.sceneId, itemId: target.itemId, expectedAssetId: target.expectedAssetId });
}
