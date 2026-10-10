// SPDX-License-Identifier: MPL-2.0
import { createSignal } from 'solid-js';
import english from './locales/en-US.json';

export type UiLanguage = 'system' | 'zh-CN' | 'en-US';
export function resolveLanguage(value: UiLanguage, system = typeof navigator === 'object' ? navigator.language : 'en-US'): 'zh-CN' | 'en-US' {
  return value === 'system' ? (/^zh\b/i.test(system) ? 'zh-CN' : 'en-US') : value;
}
export function storedLanguage(): UiLanguage {
  try {
    const value = JSON.parse(localStorage.getItem('mewu.interface.v1') || '{}').uiLanguage;
    return value === 'zh-CN' || value === 'en-US' ? value : 'system';
  } catch { return 'system'; }
}
export const [uiLanguage, setUiLanguage] = createSignal<UiLanguage>(storedLanguage());
export function t(input: string): string {
  return resolveLanguage(uiLanguage()) === 'en-US' ? (english as Record<string, string>)[input] ?? input : input;
}
export function updateLanguage(value: UiLanguage = 'system') {
  setUiLanguage(value);
  document.documentElement.lang = resolveLanguage(value);
}
