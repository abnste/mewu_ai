// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { OcrTarget, Snapshot } from './contracts';
import type { TranslationProgress, TranslationRequest } from './translation-request';
export async function runTranslation(request: TranslationRequest): Promise<Snapshot> {
  if (!isTauri()) throw new Error('请在桌面版翻译');
  return invoke('run_plugin_translation', { requestId: request.requestId, pluginId: request.pluginId, revision: request.revision, contributionId: request.contributionId, target: request.target, language: request.language });
}
export async function cancelTranslation(requestId: string): Promise<void> {
  if (isTauri()) await invoke('cancel_plugin_translation', { requestId });
}
export async function clearTranslation(target: OcrTarget): Promise<Snapshot> {
  if (!isTauri()) throw new Error('请在桌面版移除译文');
  return invoke('clear_region_translation', { target });
}
export async function subscribeTranslation(accept: (progress: TranslationProgress) => void): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<TranslationProgress>('translation-progress', event => accept(event.payload));
}
