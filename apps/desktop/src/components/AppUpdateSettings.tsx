// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { LoaderCircle } from 'lucide-solid';
import { t } from '../i18n';
import { Row } from './SettingControls';

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
  update_install_failed: '更新未完成，请重试', update_save_failed: '保存未完成，已取消更新',
  update_not_available: '请先检查更新', update_busy: '正在更新，请稍候', update_too_large: '更新文件过大',
};
export function AppUpdateSettings(props: { active: boolean }) {
  const [status, setStatus] = createSignal<UpdateStatus>(), [error, setError] = createSignal(''), [pending, setPending] = createSignal(false);
  let disposed = false, reading = false;
  const busy = () => pending() || ['checking', 'downloading', 'installing'].includes(status()?.phase ?? '');
  async function refresh() {
    if (!isTauri() || disposed || reading || !props.active) return;
    reading = true;
    try { const value = await invoke<UpdateStatus>('app_update_status'); if (!disposed) setStatus(value); }
    catch { /* A closed settings window does not interrupt host-owned downloads. */ }
    finally { reading = false; }
  }
  async function perform(command: 'check_app_update' | 'install_app_update') {
    if (busy() || disposed || !isTauri()) return;
    setPending(true); setError('');
    try {
      await invoke(command);
      await refresh();
    } catch (cause) {
      if (!disposed) setError(errors[String(cause)] ?? '更新未完成，请重试');
    } finally { if (!disposed) setPending(false); }
  }
  createEffect(on(() => props.active, active => { if (active) void refresh(); }));
  const timer = setInterval(() => void refresh(), 1000);
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
  return <><Row title={t('软件更新')}>
    <span class="settings-value" aria-live="polite">{label()}</span>
    <Show when={busy()}><LoaderCircle size={14} class="spin" /></Show>
    <button class="quiet-button" disabled={busy() || !isTauri()} onClick={() => void perform('check_app_update')}>{t('检查更新')}</button>
    <Show when={status()?.availableVersion}><button class="secondary-button" disabled={busy()} onClick={() => void perform('install_app_update')}>{t('更新并重启')}</button></Show>
  </Row><Show when={error()}><p class="settings-error" role="alert">{t(error())}</p></Show></>;
}
