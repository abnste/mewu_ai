// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { translationLanguages, type TranslationLanguage } from '../translation-request';
import { t } from '../i18n';
import { LoaderCircle, ScanLine, Video } from 'lucide-solid';
import CaptureShortcutField from './CaptureShortcutField';
import type { Preferences } from './SettingsDialog';
import { CapturePreferencesController, type CapturePreferencesView } from '../capture-preferences';
import { settingsApi } from '../settings-bridge';
import type { SettingsClient } from '../settings-client';
import { Row, Toggle } from './SettingControls';
import RecordingAudioSettings from './RecordingAudioSettings';

export default function CaptureSettings(props: { active: boolean; preferences: Preferences; onPreferences: (value: Preferences) => void; api?: SettingsClient }) {
  const api = props.api ?? settingsApi;
  const [pane, setPane] = createSignal<'screenshot' | 'recording'>('screenshot');
  const [view, setView] = createSignal<CapturePreferencesView>({ pending: false, conflict: false, error: '' });
  const [loading, setLoading] = createSignal(false);
  const controller = new CapturePreferencesController(api.saveCapturePreferences, setView);
  let disposed = false, stop: (() => void) | undefined, started = false;
  async function initialize() {
    if (loading() || disposed) return;
    setLoading(true);
    try {
      if (!stop) {
        const unsubscribe = await api.watchCapturePreferences(state => controller.loaded(state));
        if (disposed) unsubscribe(); else stop = unsubscribe;
      } else controller.loaded(await api.getCapturePreferences());
    } catch (error) { controller.failed(error); }
    finally { if (!disposed) setLoading(false); }
  }
  async function loadLatest() {
    if (loading() || view().pending || disposed) return;
    const draft = view().draft;
    setLoading(true);
    try { const state = await api.getCapturePreferences(); if (!disposed) controller.loadLatest(state, draft); }
    catch (error) { controller.failed(error); }
    finally { if (!disposed) setLoading(false); }
  }
  createEffect(on(() => props.active && pane() === 'screenshot', active => { if (active && !started) { started = true; void initialize(); } }));
  onCleanup(() => { disposed = true; stop?.(); controller.dispose(); });
  return <div class="capture-settings">
    <header class="settings-page-heading"><h2>{t("截图与录屏")}</h2></header>
    <div class="segmented"><button aria-pressed={pane() === 'screenshot'} classList={{ selected: pane() === 'screenshot' }} onClick={() => setPane('screenshot')}><ScanLine size={13} />{t('截图')}</button><button aria-pressed={pane() === 'recording'} classList={{ selected: pane() === 'recording' }} onClick={() => setPane('recording')}><Video size={13} />{t('录屏')}</button></div>
    <div hidden={pane() !== 'screenshot'}>
    <Row title={t("截图快捷键")}><CaptureShortcutField visible={props.active && pane() === 'screenshot'} /></Row>
    <Row title={t('翻译目标语言')}><select aria-label={t('翻译目标语言')} value={props.preferences.translationLanguage ?? 'zh-Hans'} onChange={event => props.onPreferences({ ...props.preferences, translationLanguage: event.currentTarget.value as TranslationLanguage })}><For each={translationLanguages}>{([language, label]) => <option value={language}>{t(label)}</option>}</For></select></Row>
    <Row title={t("选区外暗度")}><div class="range-field"><input aria-label={t("选区外暗度")} type="range" min=".3" max=".8" step=".01" value={props.preferences.maskOpacity} onInput={event => props.onPreferences({ ...props.preferences, maskOpacity: Number(event.currentTarget.value) })} /><output>{Math.round(props.preferences.maskOpacity * 100)}%</output></div></Row>
    <Show when={view().values} fallback={<Show when={loading()}><LoaderCircle size={16} class="spin" aria-label={t("加载截图设置")} /></Show>}>
      <Row title={t("延时截图")}><select aria-label={t("延时截图")} value={view().values!.captureDelaySeconds} onChange={event => controller.edit({ captureDelaySeconds: Number(event.currentTarget.value) as 0 | 3 | 5 })}><option value="0">{t("无")}</option><option value="3">{t("3 秒")}</option><option value="5">{t("5 秒")}</option></select></Row>
      <Row title={t("默认图片格式")}><select aria-label={t("默认图片格式")} value={view().values!.defaultImageFormat} onChange={event => controller.edit({ defaultImageFormat: event.currentTarget.value as 'png' | 'jpeg' })}><option value="png">PNG</option><option value="jpeg">JPEG</option></select></Row>
      <Row title={t("截图包含鼠标指针")}><Toggle label={t("截图包含鼠标指针")} checked={view().values!.includeCursor} onChange={value => controller.edit({ includeCursor: value })} /></Row>
    </Show>
    <Show when={view().error || view().conflict}><p class="settings-error" role="alert">{view().error || t('截图设置已变化，请载入最新设置')}</p></Show>
    <Show when={view().draft || view().error}><div class="settings-actions">
      <Show when={view().draft}><button class="secondary-button" disabled={view().pending || loading()} onClick={() => view().conflict || view().error ? void loadLatest() : controller.discard()}>{view().conflict || view().error ? t('载入最新') : t('取消')}</button><button class="primary-button" disabled={view().pending || view().conflict || loading()} onClick={() => void controller.saveDraft()}><Show when={view().pending}><LoaderCircle size={13} class="spin" /></Show>{t("保存")}</button></Show>
      <Show when={!view().draft && view().error}><button class="secondary-button" disabled={loading()} onClick={() => void initialize()}>{t("重试")}</button></Show>
    </div></Show>
    </div>
    <div hidden={pane() !== 'recording'}><RecordingAudioSettings active={props.active && pane() === 'recording'} /></div>
  </div>;
}
