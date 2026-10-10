// SPDX-License-Identifier: MPL-2.0
import { syncNativeLanguage } from './language-bridge';
import { t } from "./i18n";
import { preferenceKey, readPreferences } from './interface-preferences';
import { updateLanguage } from './i18n';
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { CircleAlert, LoaderCircle, X } from 'lucide-solid';
import * as bridge from './bridge';
import type { SceneCommand, Snapshot } from './contracts';
import SettingsDialog, { type Preferences } from './components/SettingsDialog';
import './settings-window.css';



export default function SettingsSurface() {
  document.documentElement.dataset.surface = 'settings';
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [preferences, setPreferences] = createSignal(readPreferences());
  const [hotkey, setHotkey] = createSignal('');
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  let disposed = false;
  let stopEvents: (() => void) | undefined;
  let commands: Promise<void> = Promise.resolve();
  const accept = (next: Snapshot) => {
    if (disposed) return;
    const previous = snapshot();
    if (previous?.revision !== undefined && next.revision !== undefined && next.revision < previous.revision) return;
    setSnapshot(next);
  };
  const showError = (cause: unknown) => {
    if (!disposed) setError(cause instanceof Error ? cause.message : String(cause));
  };
  async function initialize() {
    await syncNativeLanguage(preferences().uiLanguage).catch(showError);
    if (loading() || disposed) return;
    setLoading(true); setError('');
    try {
      if (!stopEvents) stopEvents = await bridge.subscribe(accept, () => undefined, showError);
      if (disposed) { stopEvents(); return; }
      accept(await bridge.getSnapshot());
      const info = await bridge.runtimeInfo();
      if (!disposed) setHotkey(info.hotkey);
    } catch (cause) { showError(cause); }
    finally { if (!disposed) setLoading(false); }
  }
  function save(operation: () => Promise<Snapshot>): Promise<void> {
    const next = commands.then(async () => {
      if (disposed) throw new Error('设置窗口已关闭');
      accept(await operation());
    });
    commands = next.catch(() => undefined);
    return next;
  }
  const command = (value: SceneCommand) => save(() => bridge.applyCommand(value));
  function changePreferences(value: Preferences) {
    document.documentElement.dataset.buttonLabels = value.showButtonLabels === false ? 'hide' : 'show';
    updateLanguage(value.uiLanguage);
    void syncNativeLanguage(value.uiLanguage).catch(showError);
    setPreferences(value);
    try { localStorage.setItem(preferenceKey, JSON.stringify(value)); }
    catch { /* Keep this window usable if local storage is unavailable. */ }
  }
  function syncPreferences(event: StorageEvent) {
    if (event.key === preferenceKey || event.key === null) { const next = readPreferences(); updateLanguage(next.uiLanguage); setPreferences(next); }
  }
  async function close() {
    if (!bridge.native) { location.search = ''; return; }
    try { await getCurrentWindow().close(); }
    catch { showError('无法关闭设置窗口'); }
  }
  onMount(() => { window.addEventListener('storage', syncPreferences); void initialize(); });
  onCleanup(() => { disposed = true; stopEvents?.(); window.removeEventListener('storage', syncPreferences); });
  return <main class="settings-surface" classList={{ 'reduce-motion': preferences().reduceMotion, 'text-small': preferences().textSize === 'small', 'text-large': preferences().textSize === 'large' }}>
    <Show when={snapshot()} fallback={<div class="settings-window-loading">
      <header class="settings-header" data-tauri-drag-region><h1 data-tauri-drag-region>{t("设置")}</h1><button class="icon-button" aria-label={t("关闭设置")} title={t("关闭")} onClick={() => void close()}><X size={18} /></button></header>
      <div class="settings-window-state"><Show when={loading()} fallback={<><CircleAlert size={20} /><p role="alert">{error()}</p><button class="secondary-button" onClick={() => void initialize()}>{t("重试")}</button></>}><LoaderCircle size={20} class="spin" aria-label={t("加载设置")} /></Show></div>
    </div>}>
      <SettingsDialog snapshot={snapshot()!} initialTab="general" preferences={preferences()} hotkey={hotkey()} draggable
        onPreferences={changePreferences}
        onSaveAgent={agent => command({ type: 'save_agent', agent })}
        onSaveMemory={value => command({ type: 'save_memory', ...value })}
        onDeleteMemory={value => command({ type: 'delete_memory', ...value })}
        onSaveConnectionProfile={(profile, revision, key) => save(() => bridge.saveConnectionProfile(profile, revision, key))}
        onDeleteConnectionProfile={(id, revision) => save(() => bridge.deleteConnectionProfile(id, revision))}
        onSetDefaultConnection={connectionId => command({ type: 'set_default_connection', connectionId })}
        onDiscoverMcpServer={value => save(() => bridge.discoverMcpServer(value))}
        onMcpCommand={command}
        onClose={() => void close()} />
      <Show when={error()}><div class="settings-window-alert" role="alert"><CircleAlert size={15} /><span>{error()}</span><button class="icon-button compact" aria-label={t("关闭提示")} onClick={() => setError('')}><X size={14} /></button></div></Show>
    </Show>
  </main>;
}
