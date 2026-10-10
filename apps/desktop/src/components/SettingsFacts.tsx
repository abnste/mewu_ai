// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createSignal, on, onCleanup, onMount, Show, type Accessor } from 'solid-js';
import { LoaderCircle, X } from 'lucide-solid';
import type { DataDirectoryProposal, DataDirectoryState, LicenseDocument, LicenseDocumentKind, SettingsInfo } from '../settings-contracts';
import { AppUpdateSettings } from './AppUpdateSettings';
import { settingsApi } from '../settings-bridge';
import type { SettingsClient } from '../settings-client';
import { Row } from './SettingControls';
import { createDataCleanup } from '../data-cleanup';
import { DataCleanupSettings } from './DataCleanupSettings';

export function createSettingsInformation(active: Accessor<boolean>, api: SettingsClient = settingsApi) {
  const [info, setInfo] = createSignal<SettingsInfo>(), [error, setError] = createSignal(''), [pending, setPending] = createSignal(false);
  let disposed = false, started = false;
  async function load() {
    if (disposed || pending()) return;
    setPending(true); setError('');
    try { const value = await api.info(); if (!disposed) setInfo(value); }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  createEffect(on(active, visible => { if (visible && !started) { started = true; void load(); } }));
  onCleanup(() => { disposed = true; });
  return { info, error, pending, load, api };
}
export type SettingsInformation = ReturnType<typeof createSettingsInformation>;
export function InformationState(props: { data: SettingsInformation }) {
  return <><Show when={props.data.pending()}><LoaderCircle size={16} class="spin" aria-label={t("加载软件信息")} /></Show><Show when={props.data.error()}><p class="settings-error" role="alert">{props.data.error()}</p><button class="secondary-button" disabled={props.data.pending()} onClick={() => void props.data.load()}>{t("重试")}</button></Show></>;
}
export function createDataDirectorySettings(api: SettingsClient = settingsApi, active: Accessor<boolean> = () => true) {
  const [state, setState] = createSignal<DataDirectoryState>(), [proposal, setProposal] = createSignal<DataDirectoryProposal>(), [error, setError] = createSignal(''), [pending, setPending] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; const selected = proposal(); if (selected) void api.cancelDataDirectoryProposal(selected.proposalId).catch(() => {}); });
  async function load() {
    if (pending() || disposed) return;
    setPending(true); setError('');
    try { const value = await api.dataDirectoryState(); if (!disposed) setState(value); }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  async function perform(action: () => Promise<void>) {
    if (pending() || disposed) return;
    setPending(true); setError('');
    try { await action(); }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(false); }
  }
  async function choose() {
    if (!state()?.canChange) return;
    await perform(async () => {
      const old = proposal();
      if (old) { await api.cancelDataDirectoryProposal(old.proposalId); if (disposed) return; setProposal(undefined); }
      const value = await api.chooseDataDirectory();
      if (disposed) { if (value) await api.cancelDataDirectoryProposal(value.proposalId); return; }
      if (value && value.generation !== state()?.generation) { await api.cancelDataDirectoryProposal(value.proposalId); throw new Error('数据目录已变化，请重新读取'); }
      setProposal(value ?? undefined);
    });
  }
  async function cancel() {
    const selected = proposal();
    if (selected) await perform(async () => { await api.cancelDataDirectoryProposal(selected.proposalId); if (!disposed && proposal()?.proposalId === selected.proposalId) setProposal(undefined); });
  }
  async function migrate() {
    const selected = proposal();
    if (!selected || !state()?.canChange || selected.generation !== state()?.generation) return;
    await perform(async () => { await api.migrateDataDirectory(selected); if (!disposed) setProposal(undefined); });
  }
  createEffect(on(active, visible => { if (visible) void load(); }));
  return { state, proposal, error, pending, load, choose, cancel, migrate, open: () => perform(() => api.openDataDirectory()) };
}
export function DataSettings(props: { data: SettingsInformation; active: boolean }) {
  const directory = createDataDirectorySettings(props.data.api, () => false);
  const cleanup = createDataCleanup(props.data.api, () => props.active);
  const refresh = async () => { if (props.active) { await directory.load(); if (props.active) await cleanup.load(); } };
  createEffect(on(() => props.active, visible => { if (visible) void refresh(); }));
  onMount(() => {
    const focus = () => { if (props.active && !cleanup.review() && !cleanup.pending()) void refresh(); };
    window.addEventListener('focus', focus);
    onCleanup(() => window.removeEventListener('focus', focus));
  });
  return <><header class="settings-page-heading"><h2>{t("数据")}</h2></header><InformationState data={props.data} /><div class="settings-data-path"><span class="field-label">{t("数据目录")}</span><Show when={directory.state()} fallback={<Show when={props.data.info()}>{info => <code>{info().dataDirectory}</code>}</Show>}>{state => <code>{state().path}</code>}</Show><div class="settings-data-actions"><button class="secondary-button" disabled={directory.pending() || !props.data.info()} onClick={() => void directory.open()}>{t("打开")}</button><button class="secondary-button" disabled={directory.pending() || cleanup.pending() || !!cleanup.review() || !directory.state()?.canChange} onClick={() => void directory.choose()}>{t("更改")}</button></div><Show when={directory.proposal()}>{proposal => <><code>{proposal().path}</code><div class="settings-data-actions"><button class="secondary-button" disabled={directory.pending() || cleanup.pending() || !!cleanup.review() || !directory.state()?.canChange} onClick={() => void directory.migrate()}>{t("迁移并重启")}</button><button class="quiet-button" disabled={directory.pending()} onClick={() => void directory.cancel()}>{t("取消")}</button></div></>}</Show><Show when={directory.pending()}><LoaderCircle size={16} class="spin" aria-label={t("处理数据目录")} /></Show></div><Show when={directory.error()}><p class="settings-error" role="alert">{directory.error()}</p><button class="quiet-button" disabled={directory.pending()} onClick={() => void directory.load()}>{t("重新读取")}</button></Show><DataCleanupSettings data={cleanup} disabled={directory.pending() || !!directory.proposal()} /></>;
}
function LicenseDialog(props: { value: LicenseDocument; onClose: () => void }) {
  const [limit, setLimit] = createSignal(65536);
  let root!: HTMLElement, button!: HTMLButtonElement;
  const previous = document.activeElement as HTMLElement | null;
  onMount(() => button.focus());
  onCleanup(() => previous?.isConnected && previous.focus());
  function keydown(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); props.onClose(); }
    if (event.key === 'Tab') {
      const controls = [...root.querySelectorAll<HTMLElement>('button:not(:disabled),[tabindex="0"]')];
      if (event.shiftKey && document.activeElement === controls[0]) { event.preventDefault(); controls.at(-1)?.focus(); }
      else if (!event.shiftKey && document.activeElement === controls.at(-1)) { event.preventDefault(); controls[0]?.focus(); }
    }
  }
  return <div class="settings-document-backdrop" onPointerDown={event => { if (event.target === event.currentTarget) props.onClose(); }}><section ref={root} data-settings-inner-dialog role="dialog" aria-modal="true" aria-label={props.value.title} class="settings-document" on:keydown={keydown}>
    <header><h2>{props.value.title}</h2><button ref={button} class="icon-button" aria-label={t("关闭文档")} onClick={props.onClose}><X size={16} /></button></header><pre tabIndex={0}>{props.value.text.slice(0, limit())}</pre><Show when={props.value.text.length > limit()}><button class="secondary-button" onClick={() => setLimit(value => value + 65536)}>{t("显示更多")}</button></Show>
  </section></div>;
}
export function AboutSettings(props: { data: SettingsInformation; active: boolean }) {
  const [documentValue, setDocument] = createSignal<LicenseDocument>(), [error, setError] = createSignal(''), [pending, setPending] = createSignal(false);
  let disposed = false, generation = 0;
  async function read(kind: LicenseDocumentKind) {
    if (pending() || disposed) return;
    const current = ++generation;
    setPending(true); setError('');
    try { const value = await props.data.api.readLicenseDocument(kind); if (!disposed && props.active && current === generation) setDocument(value); }
    catch (cause) { if (!disposed && current === generation) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed && current === generation) setPending(false); }
  }
  createEffect(on(() => props.active, active => { if (!active) { generation++; setPending(false); setDocument(undefined); } }));
  onCleanup(() => { disposed = true; generation++; });
  return <><header class="settings-page-heading"><h2>{t("关于")}</h2></header><InformationState data={props.data} /><Show when={props.data.info()}>{info => <>
    <Row title={info().application}><span class="settings-value">{info().version}</span></Row>
    <Show when={info().commit}><Row title={t("构建")}><code class="settings-build">{info().commit}</code></Row></Show>
    <AppUpdateSettings active={props.active} />
    <Row title={t("开源许可")}><button class="quiet-button" disabled={pending()} onClick={() => void read('license')}>{info().license}</button></Row>
    <Show when={info().noticesAvailable}><Row title={t("第三方许可")}><button class="quiet-button" disabled={pending()} onClick={() => void read('notices')}>{t("查看")}</button></Row></Show>
  </>}</Show><Show when={error()}><p class="settings-error" role="alert">{error()}</p></Show><Show when={documentValue()}>{value => <LicenseDialog value={value()} onClose={() => setDocument(undefined)} />}</Show></>;
}
