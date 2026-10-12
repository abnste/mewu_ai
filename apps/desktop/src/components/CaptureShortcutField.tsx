// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { Check, LoaderCircle, RotateCcw, X } from 'lucide-solid';
import * as bridge from '../shortcut-bridge';
import { acceptShortcutState, defaultCaptureShortcut, equalShortcut, formatShortcut, settleShortcutSave, shortcutFromKey, ShortcutLeaseController, type ShortcutDraft } from '../capture-shortcut-editor';
import type { CaptureShortcutState, ShortcutChord } from '../shortcut-contracts';
import { portalShortcutHasDisableAction, portalShortcutLabel, ShortcutPortalController, type ShortcutPortalView } from '../capture-shortcut-portal';
import '../capture-shortcut.css';

export default function CaptureShortcutField(props: { visible: boolean }) {
  const [state, setState] = createSignal<CaptureShortcutState>();
  const [draft, setDraft] = createSignal<ShortcutDraft>();
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false), [saving, setSaving] = createSignal(false);
  const [recording, setRecording] = createSignal<'idle' | 'arming' | 'armed'>('idle');
  const [portalView, setPortalView] = createSignal<ShortcutPortalView>({ canceling: false, configuring: false });
  let input!: HTMLInputElement, disposed = false, stopEvents: (() => void) | undefined;
  const isPortal = () => state()?.backend === 'wayland_portal';
  const editable = () => Boolean(state()?.editable && !loading() && !saving());
  const showError = (cause: unknown) => { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); };
  const lease = new ShortcutLeaseController({ active: () => !disposed && !isPortal() && props.visible && editable() && document.visibilityState !== 'hidden' && document.hasFocus() && document.activeElement === input, begin: bridge.beginCaptureShortcutEdit, end: bridge.endCaptureShortcutEdit, changed: setRecording, record: recordChord, error: showError });
  const portal = new ShortcutPortalController({ allowed: () => !disposed && props.visible && isPortal() && editable() && !state()?.configurationPending, configure: bridge.configureCaptureShortcutPortal, cancel: bridge.cancelCaptureShortcutPortal, accepted: accept, changed: setPortalView, error: showError });
  const portalBusy = () => Boolean(loading() || saving() || state()?.configurationPending || portalView().requestId);
  const portalCancellationId = () => portalView().requestId ?? state()?.configurationRequestId ?? undefined;
  const portalShowDisable = () => portalShortcutHasDisableAction(state());
  const portalCanDisable = () => !portalBusy() && state()?.status !== 'disabled' && (editable() || state()?.status === 'cleanup_pending');
  const value = () => draft()?.shortcut ?? (draft() ? null : state()?.configured ?? null);
  const conflict = () => Boolean(draft() && state() && draft()!.expectedRevision !== state()!.revision);
  function accept(next: CaptureShortcutState) {
    if (disposed) return;
    setState(old => acceptShortcutState(old, next));
    if (isPortal() || !state()?.editable) void lease.disarm().catch(showError);
  }
  async function initialize() {
    if (loading() || disposed) return;
    setLoading(true); setError('');
    try {
      if (!stopEvents) stopEvents = await bridge.subscribeCaptureShortcut({ state: accept, recorded: value => lease.record(value), ended: id => lease.ended(id) });
      if (disposed) { stopEvents(); return; }
      accept(await bridge.getCaptureShortcut());
    } catch (cause) { showError(cause); }
    finally { if (!disposed) setLoading(false); }
  }
  function recordChord(shortcut: ShortcutChord | null) {
    if (!editable() || !state()) return;
    setDraft(old => ({ expectedRevision: old?.expectedRevision ?? state()!.revision, shortcut: shortcut ? { ...shortcut } : null })); setError('');
  }
  function arm() {
    if (!state()) { void initialize(); return; }
    if (editable()) { setError(''); void lease.arm(); }
  }
  function disarm() { void lease.disarm().catch(showError); }
  function cancel() { setDraft(undefined); setError(''); disarm(); input?.blur(); }
  function restore() { disarm(); recordChord(defaultCaptureShortcut); }
  async function save() {
    const submitted = draft(); if (!submitted || !editable()) return;
    setSaving(true); setError('');
    try {
      await lease.flush();
      if (disposed) return;
      if (!state()?.editable) throw new Error('快捷键设置不可修改');
      const receipt = await bridge.setCaptureShortcut(submitted.expectedRevision, submitted.shortcut);
      if (disposed) return;
      accept(receipt);
      const settled = settleShortcutSave(submitted, draft(), receipt, state()!);
      setDraft(settled.draft);
      if (settled.conflict) setError('快捷键设置已变化');
    } catch (cause) { showError(cause); }
    finally { if (!disposed) setSaving(false); }
  }
  function configurePortal() { const current = state(); if (!current || portalBusy() || !editable()) return; setError(''); void portal.configure(current.revision); }
  async function modifyPortal() {
    if (!isPortal() || !state()?.systemSettingsAvailable || portalBusy() || !editable()) return;
    setSaving(true); setError('');
    try { await bridge.openCaptureShortcutSystemSettings(); } catch (cause) { showError(cause); }
    finally { if (!disposed) setSaving(false); }
  }
  async function disablePortal() {
    const current = state(); if (!current || !isPortal() || !portalCanDisable()) return;
    setSaving(true); setError('');
    try { accept(await bridge.setCaptureShortcut(current.revision, null)); } catch (cause) { showError(cause); }
    finally { if (!disposed) setSaving(false); }
  }
  const keyboard = (event: KeyboardEvent) => {
    if (event.key === 'Tab') { disarm(); return; }
    if (event.key === 'Escape' && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); event.stopPropagation(); cancel(); return; }
    if (event.isComposing || event.keyCode === 229) return;
    event.preventDefault(); event.stopPropagation();
    if (!lease.armed()) return;
    const chord = shortcutFromKey(event); if (chord !== undefined) recordChord(chord);
  };
  const visibility = () => { if (document.visibilityState === 'hidden') { disarm(); void portal.cancel().catch(showError); } };
  onMount(() => { window.addEventListener('blur', disarm); document.addEventListener('visibilitychange', visibility); void initialize(); });
  createEffect(on(() => props.visible, visible => { if (!visible) { disarm(); void portal.cancel().catch(showError); } }));
  onCleanup(() => { disposed = true; portal.dispose(); lease.dispose(); stopEvents?.(); window.removeEventListener('blur', disarm); document.removeEventListener('visibilitychange', visibility); });
  return <div class="capture-shortcut-field">
    <Show when={!isPortal()} fallback={<div class="capture-shortcut-controls">
      <span class="capture-shortcut-current" aria-label={t('截图快捷键')} title={portalShortcutLabel(state())} role="status">{portalShortcutLabel(state()) ?? t('未设置')}</span>
      <Show when={loading() || saving() || portalView().configuring || portalView().canceling || state()?.configurationPending && state()?.status !== 'cleanup_pending'}><LoaderCircle size={14} class="spin" aria-label={t('设置中…')} /></Show>
      <Show when={state()?.status !== 'active'}><button class="secondary-button" disabled={portalBusy() || !editable()} onClick={configurePortal}>{t('设置')}</button></Show>
      <Show when={state()?.status === 'active' && state()?.systemSettingsAvailable}><button class="secondary-button" disabled={portalBusy() || !editable()} onClick={() => void modifyPortal()}>{t('修改')}</button></Show>
      <Show when={portalShowDisable() && state()?.status !== 'disabled' && !portalCancellationId()}><button class="quiet-button" disabled={!portalCanDisable()} onClick={() => void disablePortal()}>{t('停用')}</button></Show>
      <Show when={portalCancellationId()}><button class="icon-button compact" aria-label={t('取消快捷键设置')} title={t('取消')} disabled={portalView().canceling} onClick={() => { const id = portalCancellationId(); if (!id) return; setError(''); void portal.cancelPublished(id).catch(showError); }}><X size={14} /></button></Show>
    </div>}><div class="capture-shortcut-controls">
      <input ref={input} classList={{ recording: recording() === 'armed' }} readOnly aria-label={t("截图快捷键")} aria-busy={recording() === 'arming' || loading()} value={state() ? formatShortcut(value()) : ''} placeholder={loading() ? t('加载中…') : t('重新读取')} title={recording() === 'armed' ? t('按组合键，Delete 清空') : t('编辑截图快捷键')} disabled={saving() || loading() || (state() !== undefined && !state()!.editable)} onFocus={arm} onClick={arm} onBlur={disarm} on:keydown={keyboard} />
      <button class="icon-button compact" aria-label={t("恢复默认截图快捷键")} title={t("恢复默认")} disabled={!editable()} onClick={restore}><RotateCcw size={14} /></button>
      <Show when={draft() || recording() !== 'idle'}><button class="icon-button compact" aria-label={t("保存截图快捷键")} title={t("保存")} disabled={!draft() || !editable()} onClick={() => void save()}><Show when={saving()} fallback={<Check size={15} />}><LoaderCircle size={14} class="spin" /></Show></button><button class="icon-button compact" aria-label={t("取消快捷键修改")} title={t("取消")} disabled={saving()} onClick={cancel}><X size={14} /></button></Show>
    </div></Show>
    <Show when={!isPortal() && state()?.active && state()?.status !== 'active' && !equalShortcut(state()!.configured, state()!.active)}><small class="capture-shortcut-actual">{t("当前：")}{formatShortcut(state()!.active)}</small></Show>
    <Show when={error() || state()?.message}><small class="capture-shortcut-error" role="alert">{error() || state()?.message}</small></Show>
    <Show when={!isPortal() && conflict()}><button class="capture-shortcut-reload" onClick={cancel} disabled={saving()}>{t("载入最新")}</button></Show>
  </div>;
}
