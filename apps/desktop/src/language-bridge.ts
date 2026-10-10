// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { resolveLanguage, type UiLanguage } from './i18n';
export async function syncNativeLanguage(language: UiLanguage = 'system') {
  if (isTauri()) await invoke('set_ui_language', { locale: resolveLanguage(language) });
}
