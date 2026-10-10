// SPDX-License-Identifier: MPL-2.0
import type { UiLanguage } from './i18n';
import { translationLanguages, type TranslationLanguage } from './translation-request';
export interface Preferences {
  textSize: 'small' | 'default' | 'large';
  reduceMotion: boolean;
  maskOpacity: number;
  uiLanguage?: UiLanguage;
  translationLanguage?: TranslationLanguage;
  showButtonLabels?: boolean;
  thinkingGlowEnabled?: boolean;
  thinkingGlowColor?: string;
}
export const preferenceKey = 'mewu.interface.v1';
export function readPreferences(): Preferences {
  const defaults: Preferences = { textSize: 'default', reduceMotion: false, maskOpacity: .58, uiLanguage: 'system', translationLanguage: 'zh-Hans', showButtonLabels: true, thinkingGlowEnabled: true, thinkingGlowColor: '#A7C7FF' };
  try {
    const value = JSON.parse(localStorage.getItem(preferenceKey) || '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return defaults;
    return {
      textSize: ['small', 'default', 'large'].includes(value.textSize) ? value.textSize : defaults.textSize,
      reduceMotion: value.reduceMotion === true,
      maskOpacity: typeof value.maskOpacity === 'number' && Number.isFinite(value.maskOpacity) ? Math.min(.8, Math.max(.3, value.maskOpacity)) : defaults.maskOpacity,
      uiLanguage: ['system', 'zh-CN', 'en-US'].includes(value.uiLanguage) ? value.uiLanguage : defaults.uiLanguage,
      translationLanguage: translationLanguages.some(([language]) => language === value.translationLanguage) ? value.translationLanguage : defaults.translationLanguage,
      showButtonLabels: value.showButtonLabels !== false,
      thinkingGlowEnabled: value.thinkingGlowEnabled !== false,
      thinkingGlowColor: typeof value.thinkingGlowColor === 'string' && /^#[0-9a-f]{6}$/i.test(value.thinkingGlowColor.trim()) ? value.thinkingGlowColor.trim().toUpperCase() : defaults.thinkingGlowColor,
    };
  } catch { return defaults; }
}
