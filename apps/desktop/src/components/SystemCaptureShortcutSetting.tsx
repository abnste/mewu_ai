// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { t } from '../i18n';
import { SystemPreferencesController, type SystemPreferencesView } from '../system-preferences';
import type { SettingsClient } from '../settings-client';
import { Row, Toggle } from './SettingControls';

export default function SystemCaptureShortcutSetting(props: { active: boolean; api: SettingsClient }) {
  const [view, setView] = createSignal<SystemPreferencesView>({ pending: false, conflict: false, error: '' });
  const [loading, setLoading] = createSignal(false);
  const controller = new SystemPreferencesController(props.api.saveSystemPreferences, setView);
  let disposed = false, stop: (() => void) | undefined, started = false;
  async function initialize() {
    if (loading() || view().pending || disposed) return;
    const draft = view().draft;
    setLoading(true);
    try {
      if (!stop) {
        const unsubscribe = await props.api.watchSystemPreferences(state => controller.loaded(state));
        if (disposed) unsubscribe(); else stop = unsubscribe;
      } else {
        const state = await props.api.getSystemPreferences();
        if (!disposed) controller.loadLatest(state, draft);
      }
    } catch (error) { controller.failed(error); }
    finally { if (!disposed) setLoading(false); }
  }
  function change(value: boolean) {
    if (loading() || view().pending || view().conflict || disposed) return;
    controller.edit({ replaceSystemCaptureShortcut: value });
    void controller.saveDraft();
  }
  createEffect(on(() => props.active, active => { if (active && !started) { started = true; void initialize(); } }));
  onCleanup(() => { disposed = true; stop?.(); controller.dispose(); });
  const warning = () => view().state?.systemCaptureShortcutWarning || '';
  const checked = () => view().draft?.value.replaceSystemCaptureShortcut ?? (view().state?.replaceSystemCaptureShortcut === true && !warning());
  return <>
    <Row title={t('使用 Win+Shift+S 截图')}><Toggle label={t('使用 Win+Shift+S 截图')} checked={checked()} disabled={!view().state || loading() || view().pending || view().conflict} onChange={change} /></Row>
    <Show when={view().error || view().conflict || warning()}>
      <p class="settings-error" role="alert">{t(view().error || (view().conflict ? '系统设置已变化，请载入最新设置' : warning()))}</p>
      <Show when={view().error || view().conflict}>
      <div class="settings-actions">
        <Show when={view().conflict} fallback={<button class="secondary-button" disabled={loading() || view().pending} onClick={() => view().draft ? void controller.saveDraft() : void initialize()}>{t('重试')}</button>}><button class="secondary-button" disabled={loading() || view().pending} onClick={() => void initialize()}>{t('载入最新设置')}</button></Show>
        <Show when={view().draft}><button class="secondary-button" disabled={view().pending} onClick={() => controller.discard()}>{t('取消')}</button></Show>
      </div>
      </Show>
    </Show>
  </>;
}
