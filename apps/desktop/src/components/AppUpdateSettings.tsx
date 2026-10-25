// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, on, onCleanup, Show, type Accessor } from 'solid-js';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { LoaderCircle } from 'lucide-solid';
import { t } from '../i18n';
import { Row } from './SettingControls';
import { settingsApi } from '../settings-bridge';
import type { SettingsInfo } from '../settings-contracts';

interface UpdateStatus {
  phase: 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'installing' | 'failed';
  currentVersion: string;
  availableVersion: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
}
const errors: Record<string, string> = {
  update_proxy: '网络代理设置不可用', update_invalid: '更新信息无效',
  update_check_failed: '检查更新失败', update_download_failed: '下载或签名验证失败',
  update_feed_unavailable: '暂时无法获取发行版本', update_network: '无法连接 GitHub',
  update_timeout: '检查更新超时', update_rate_limited: 'GitHub 请求过于频繁，请稍后重试',
  update_install_failed: '更新未完成，请重试', update_save_failed: '保存未完成，已取消更新',
  update_not_available: '请先检查更新', update_busy: '正在更新，请稍候', update_too_large: '更新文件过大',
  update_unsupported: '此平台尚未配置在线更新',
  update_install_location: '请将 MewuAI 移到应用程序文件夹后重试',
  update_appimage_location: '请使用可写目录中的 AppImage 更新',
};
type UpdateCommand = 'check_app_update' | 'install_app_update';
interface UpdateApi {
  native: () => boolean;
  info: () => Promise<SettingsInfo>;
  status: () => Promise<UpdateStatus>;
  perform: (command: UpdateCommand) => Promise<unknown>;
}
const nativeUpdates: UpdateApi = {
  native: isTauri, info: () => settingsApi.info(),
  status: () => invoke<UpdateStatus>('app_update_status'),
  perform: command => invoke(command),
};
export function createAppUpdateSettings(active: Accessor<boolean>, api: UpdateApi = nativeUpdates, options: { checkOnOpen?: boolean } = {}) {
  const [information, setInformation] = createSignal<SettingsInfo>();
  const [status, setStatus] = createSignal<UpdateStatus>(), [error, setError] = createSignal(''), [pending, setPending] = createSignal(false);
  let disposed = false, reading = false, checkedOnOpen = false;
  const mode = () => {
    const info = information();
    if (!info) return 'loading';
    if (['macos', 'linux'].includes(info.platform) && info.updateSupport === 'manual_only') return 'manual';
    return (info.platform === 'windows' && info.updateSupport === 'signed_installer')
      || (info.platform === 'macos' && info.updateSupport === 'signed_bundle')
      || (info.platform === 'linux' && info.updateSupport === 'signed_appimage') ? 'signed' : 'unsupported';
  };
  const busy = () => pending() || ['checking', 'downloading', 'installing'].includes(status()?.phase ?? '');
  async function refresh() {
    if (!api.native() || disposed || reading || !active()) return;
    reading = true;
    try {
      if (!information()) {
        const value = await api.info();
        if (disposed) return;
        setInformation(value); setError('');
      }
      if (mode() !== 'signed') return;
      const value = await api.status(); if (!disposed) setStatus(value);
    }
    catch (cause) {
      if (!disposed && !information()) setError(cause instanceof Error ? cause.message : String(cause));
      // A closed settings window does not interrupt host-owned downloads.
    }
    finally { reading = false; }
  }
  async function perform(command: UpdateCommand) {
    if (mode() !== 'signed' || busy() || disposed || !api.native()) return;
    setPending(true); setError('');
    try {
      await api.perform(command);
      await refresh();
    } catch (cause) {
      if (!disposed) setError(errors[String(cause)] ?? '更新未完成，请重试');
    } finally { if (!disposed) setPending(false); }
  }
  createEffect(on(active, visible => {
    if (visible) void (async () => {
      await refresh();
      if (disposed || !active() || !options.checkOnOpen || checkedOnOpen || mode() !== 'signed') return;
      checkedOnOpen = true;
      if (!busy()) await perform('check_app_update');
    })();
  }));
  // Poll only a host-owned operation in progress, never idle settings pages.
  const timer = setInterval(() => { if (['checking', 'downloading', 'installing'].includes(status()?.phase ?? '')) void refresh(); }, 1000);
  onCleanup(() => { disposed = true; clearInterval(timer); });
  const label = () => {
    const value = status();
    if (!value) return '';
    if (value.phase === 'current') return t('已是最新版本');
    if (value.phase === 'checking') return t('正在检查');
    if (value.phase === 'downloading') {
      const percent = value.totalBytes ? Math.min(100, Math.floor(value.downloadedBytes / value.totalBytes * 100)) : 0;
      return t('正在下载') + (percent ? ` ${percent}%` : '');
    }
    if (value.phase === 'installing') return t('正在保存并更新');
    if (value.phase === 'failed') return t(errors[value.error ?? ''] ?? '检查更新失败');
    return value.availableVersion ?? '';
  };
  return { status, error, busy, mode, refresh, perform, label };
}
export type AppUpdateController = ReturnType<typeof createAppUpdateSettings>;
export function AppUpdateSettings(props: { active: boolean; controller?: AppUpdateController }) {
  const { status, error, busy, mode, perform, label } = props.controller ?? createAppUpdateSettings(() => props.active);
  return <><Row title={t('软件更新')}>
    <Show when={mode() === 'manual'}><span class="settings-value">{t('手动更新：请退出 MewuAI，使用新版安装包替换应用。')}</span></Show>
    <Show when={mode() === 'unsupported'}><span class="settings-value">{t(errors.update_unsupported)}</span></Show>
    <Show when={mode() === 'signed'}>
    <span class="settings-value" aria-live="polite">{label()}</span>
    <Show when={busy()}><LoaderCircle size={14} class="spin" /></Show>
    <button class="quiet-button" disabled={busy() || !isTauri()} onClick={() => void perform('check_app_update')}>{t('检查更新')}</button>
    <Show when={status()?.availableVersion}><button class="secondary-button" disabled={busy()} onClick={() => void perform('install_app_update')}>{t('更新并重启')}</button></Show>
    </Show>
  </Row><Show when={error()}><p class="settings-error" role="alert">{t(error())}</p></Show></>;
}
