// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { LoaderCircle } from 'lucide-solid';
import { t, type UiLanguage } from '../i18n';
import type { Preferences } from '../interface-preferences';
import { SystemPreferencesController, type SystemPreferencesView } from '../system-preferences';
import { settingsApi } from '../settings-bridge';
import type { SettingsClient } from '../settings-client';
import { Row, Toggle } from './SettingControls';

export default function GeneralSettings(props: { active: boolean; preferences: Preferences; onPreferences: (value: Preferences) => void; api?: SettingsClient }) {
  const api = props.api ?? settingsApi;
  const [view, setView] = createSignal<SystemPreferencesView>({ pending: false, conflict: false, error: '' });
  const [loading, setLoading] = createSignal(false);
  const controller = new SystemPreferencesController(api.saveSystemPreferences, setView);
  let disposed = false, stop: (() => void) | undefined, started = false;
  async function initialize() {
    if (loading() || disposed) return;
    setLoading(true);
    try {
      if (!stop) {
        const unsubscribe = await api.watchSystemPreferences(state => controller.loaded(state));
        if (disposed) unsubscribe(); else stop = unsubscribe;
      } else controller.loaded(await api.getSystemPreferences());
    } catch (error) { controller.failed(error); }
    finally { if (!disposed) setLoading(false); }
  }
  async function loadLatest() {
    if (loading() || view().pending || disposed) return;
    const draft = view().draft; setLoading(true);
    try { const state = await api.getSystemPreferences(); if (!disposed) controller.loadLatest(state, draft); }
    catch (error) { controller.failed(error); }
    finally { if (!disposed) setLoading(false); }
  }
  createEffect(on(() => props.active, active => { if (active && !started) { started = true; void initialize(); } }));
  onCleanup(() => { disposed = true; stop?.(); controller.dispose(); });
  const startup = () => view().draft?.value.launchAtStartup ?? view().state?.startupRegistered ?? false;
  return <div class="general-settings">
    <header class="settings-page-heading"><h2>{t('通用')}</h2></header>
    <Row title={t('界面语言')}><select aria-label={t('界面语言')} value={props.preferences.uiLanguage ?? 'system'} onChange={event => props.onPreferences({ ...props.preferences, uiLanguage: event.currentTarget.value as UiLanguage })}><option value="system">{t('跟随系统')}</option><option value="zh-CN">{t('简体中文')}</option><option value="en-US">English</option></select></Row>
    <Row title={t('文字大小')}><select aria-label={t('文字大小')} value={props.preferences.textSize} onChange={event => props.onPreferences({ ...props.preferences, textSize: event.currentTarget.value as Preferences['textSize'] })}><option value="small">{t('小')}</option><option value="default">{t('默认')}</option><option value="large">{t('大')}</option></select></Row>
    <Row title={t('显示按钮功能文字')}><Toggle label={t('显示按钮功能文字')} checked={props.preferences.showButtonLabels !== false} onChange={value => props.onPreferences({ ...props.preferences, showButtonLabels: value })} /></Row>
    <Row title={t('AI 思考时显示底部呼吸光效')}><div class="thinking-glow-setting"><input type="color" aria-label={t('呼吸光效颜色')} value={props.preferences.thinkingGlowColor ?? '#A7C7FF'} onInput={event => props.onPreferences({ ...props.preferences, thinkingGlowColor: event.currentTarget.value.toUpperCase() })} /><Toggle label={t('AI 思考时显示底部呼吸光效')} checked={props.preferences.thinkingGlowEnabled !== false} onChange={value => props.onPreferences({ ...props.preferences, thinkingGlowEnabled: value })} /></div></Row>
    <Row title={t('减少动态效果')}><Toggle label={t('减少动态效果')} checked={props.preferences.reduceMotion} onChange={value => props.onPreferences({ ...props.preferences, reduceMotion: value })} /></Row>
    <Show when={view().values} fallback={<Show when={loading()}><LoaderCircle size={16} class="spin" aria-label={t('加载系统设置')} /></Show>}>
      <Row title={t('登录 Windows 后自动启动')}><Toggle label={t('登录 Windows 后自动启动')} checked={startup()} disabled={view().pending} onChange={value => controller.edit({ launchAtStartup: value })} /></Row>
      <Show when={view().state?.startupWarning}><p class="settings-error" role="alert">{t(view().state!.startupWarning!)}</p></Show>
      <Row title={t('允许屏幕共享看到框选和标注')}><Toggle label={t('允许屏幕共享看到框选和标注')} checked={view().values!.allowScreenShare} disabled={view().pending} onChange={value => controller.edit({ allowScreenShare: value })} /></Row>
      <Row title={t('网络代理')}><select aria-label={t('网络代理')} value={view().values!.networkProxyMode} onChange={event => controller.edit({ networkProxyMode: event.currentTarget.value as 'system' | 'direct' | 'custom' })}><option value="system">{t('跟随系统')}</option><option value="direct">{t('直接连接')}</option><option value="custom">{t('自定义代理')}</option></select></Row>
      <Show when={view().values!.networkProxyMode === 'custom'}><label class="field-label">{t('代理地址')}<input aria-label={t('代理地址')} placeholder="http://127.0.0.1:7890" value={view().values!.networkProxyUrl} maxlength={2048} onInput={event => controller.edit({ networkProxyUrl: event.currentTarget.value })} /></label></Show>
      <Show when={view().draft}><div class="settings-actions"><button class="secondary-button" disabled={view().pending} onClick={() => controller.discard()}>{t('取消')}</button><Show when={view().conflict}><button class="secondary-button" disabled={loading() || view().pending} onClick={() => void loadLatest()}>{t('载入最新设置')}</button></Show><button class="primary-button" disabled={view().pending || view().conflict} onClick={() => void controller.saveDraft()}><Show when={view().pending}><LoaderCircle size={14} class="spin" /></Show>{t('保存')}</button></div></Show>
    </Show>
    <Show when={view().error}><p class="settings-error" role="alert">{t(view().error)}</p><Show when={!view().state}><button class="secondary-button" disabled={loading()} onClick={() => void initialize()}>{t('重试')}</button></Show></Show>
  </div>;
}
