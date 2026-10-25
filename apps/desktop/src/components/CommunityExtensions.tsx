// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronRight, Download, LoaderCircle, RefreshCw, Trash2, Undo2, X } from 'lucide-solid';
import * as api from '../extension-bridge';
import type { ExtensionCatalogEntry, ExtensionControl, ExtensionManifest, ExtensionProposal, ExtensionRecord, ExtensionSource } from '../extension-contracts';
import { mergeExtensionRecords } from '../extension-state';
import { extensionUpdateEntry } from '../extension-updates';
import { extensionPlatformError, installedExtensionError, type HostPlatform } from '../plugin-platform';
import { pluginTagLabels } from '../plugin-categories';
import type { PluginTag } from '../plugin-contracts';
import { t } from '../i18n';
import ExtensionProjectGallery from './ExtensionProjectGallery';
import PluginIcon from './PluginIcon';
import { rememberExtensionIcons } from '../extension-icons';

interface Props {
  active: boolean; pane: 'discover' | 'installed'; query: string; filter: string;
  addRequest: number; refreshRequest: number; onDetail: (open: boolean) => void;
  platform?: HostPlatform;
}
const sourceText = (source: ExtensionSource) => source.type === 'github' ? `${source.entry.repository}\n${source.entry.releaseTag}\n${source.entry.assetName}` : source.path;
const tagText = (tag: string) => t(pluginTagLabels[tag as PluginTag] ?? tag);

export default function CommunityExtensions(props: Props) {
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]), [catalog, setCatalog] = createSignal<ExtensionCatalogEntry[]>([]);
  const [selected, setSelected] = createSignal(''), [proposal, setProposal] = createSignal<ExtensionProposal>();
  const [busy, setBusy] = createSignal(''), [error, setError] = createSignal(''), [loading, setLoading] = createSignal(false);
  const [confirmRemoval, setConfirmRemoval] = createSignal(false);
  let disposed = false, serial = 0, navigation = 0, eventSerial = 0, unsubscribe: (() => void) | undefined, listening: Promise<void> | undefined;
  const accept = (value: ExtensionRecord[]) => { if (!disposed) setRecords(old => mergeExtensionRecords(old, value)); };
  const record = (id: string) => records().find(record => record.manifest.id === id);
  const selectedRecord = () => record(proposal()?.manifest.id ?? selected());
  const selectedCatalog = () => catalog().find(entry => entry.id === selected());
  const unsupported = (manifest: ExtensionManifest | ExtensionCatalogEntry, source?: ExtensionSource) => extensionPlatformError(manifest, source, props.platform);
  const selectedError = () => detail() ? unsupported(detail()!.manifest, detail()!.source) : undefined;
  const entryError = (entry: ExtensionManifest | ExtensionCatalogEntry) => record(entry.id) ? installedExtensionError(record(entry.id)!, props.platform) : unsupported(entry);
  const detail = createMemo((): { manifest: ExtensionManifest | ExtensionCatalogEntry; source: ExtensionSource } | undefined => {
    const prepared = proposal(); if (prepared) return { manifest: prepared.manifest, source: prepared.source };
    const installed = selectedRecord(); if (installed) return { manifest: installed.manifest, source: installed.source };
    const found = selectedCatalog(); if (found) return { manifest: found, source: { type: 'github', entry: found } };
  });
  const entries = createMemo(() => {
    const text = props.query.trim().toLocaleLowerCase(), known = new Set(catalog().map(entry => entry.id));
    const source = props.pane === 'discover' ? catalog().filter(entry => !record(entry.id) || record(entry.id)!.state === 'removed')
      : records().filter(record => record.state !== 'removed' && (record.source.type === 'github' || known.has(record.manifest.id))).map(record => ({ ...record.manifest, repository: record.source.type === 'github' ? record.source.entry.repository : '' }));
    return source.filter(entry => (!props.filter || entry.tags.includes(props.filter))
      && (!text || [entry.name, entry.id, entry.description, ...entry.tags.map(tagText)].join(' ').toLocaleLowerCase().includes(text)));
  });
  createEffect(() => props.onDetail(Boolean(detail())));
  async function ensureListening() {
    if (!listening) listening = (async () => {
      const stop = await api.subscribeExtensions(value => { eventSerial++; accept(value); });
      if (disposed) stop(); else unsubscribe = stop;
    })().catch(error => { listening = undefined; throw error; });
    await listening;
  }
  async function refresh() {
    if (!api.extensionNative) return;
    const current = ++serial; setLoading(true); setError('');
    try {
      await ensureListening(); if (disposed) return;
      const beforeEvents = eventSerial, installed = await api.getExtensions();
      if (beforeEvents === eventSerial) accept(installed);
      const value = await api.getExtensionCatalog();
      if (!disposed && current === serial) { rememberExtensionIcons(value.plugins); setCatalog(value.plugins); }
    } catch (cause) { if (!disposed && current === serial) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed && current === serial) setLoading(false); }
  }
  createEffect(on(() => [props.active, props.pane] as const, ([active]) => { if (active) void refresh(); }));
  createEffect(on(() => props.refreshRequest, () => { if (props.active) void refresh(); }, { defer: true }));
  createEffect(on(() => props.addRequest, () => { if (props.active) void prepare(); }, { defer: true }));
  createEffect(on(() => props.pane, () => back(), { defer: true }));
  onCleanup(() => { disposed = true; serial++; navigation++; unsubscribe?.(); props.onDetail(false); });
  function back() { navigation++; setSelected(''); setProposal(undefined); setConfirmRemoval(false); setError(''); }
  function open(id: string) { navigation++; setSelected(id); setProposal(undefined); setConfirmRemoval(false); setError(''); }
  async function operate(id: string, action: () => Promise<void>) {
    if (busy()) return; setBusy(id); setError('');
    try { await action(); } catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setBusy(''); }
  }
  async function prepare(entry?: ExtensionCatalogEntry) {
    if (entry && unsupported(entry)) return;
    const current = navigation;
    await operate(entry?.id ?? 'local', async () => {
      const prepared = await api.prepareExtension(entry);
      if (!disposed && prepared && current === navigation) { setProposal(prepared); setSelected(prepared.manifest.id); setConfirmRemoval(false); }
    });
  }
  async function install() {
    const value = proposal(); if (!value) return;
    if (unsupported(value.manifest, value.source)) return;
    if (value.sourceMigration && (value.sourceMigration.schemaVersion !== 1 || !['migrate', 'migrate-and-install'].includes(value.sourceMigration.operation))) { setError(t('插件迁移协议版本不受支持')); return; }
    await operate(value.manifest.id, async () => { accept(await api.installExtension(value.id, value.sourceMigration?.id)); if (!disposed && proposal()?.id === value.id) setProposal(undefined); });
  }
  async function control(value: ExtensionRecord, action: ExtensionControl) {
    if (action !== 'disable' && action !== 'remove' && installedExtensionError(value, props.platform)) return;
    await operate(value.manifest.id, async () => { accept(await api.controlExtension(value, action)); if (!disposed) { setConfirmRemoval(false); if (action === 'remove') back(); } });
  }
  const status = (value?: ExtensionRecord) => !value || value.state === 'removed' ? t('未安装') : value.state === 'enabled' ? t('已启用') : t('已停用');
  const availableUpdate = (id: string) => { const installed = record(id); return installed ? extensionUpdateEntry(installed, catalog()) : undefined; };
  const updateEntry = () => availableUpdate(selected());
  const updateLabel = (id: string) => availableUpdate(id)?.version === record(id)?.manifest.version ? t('迁移来源') : t('更新');
  return <div class="community-extensions" onKeyDown={event => { if (event.key === 'Escape' && detail()) { event.preventDefault(); event.stopPropagation(); back(); } }}>
    <Show when={!api.extensionNative}><p class="plugin-empty">{t('请在桌面版管理插件')}</p></Show>
    <Show when={error()}><div class="plugin-error" role="alert"><span>{error()}</span><button class="icon-button compact" aria-label={t('关闭提示')} onClick={() => setError('')}><X size={13}/></button></div></Show>
    <Show when={loading()}><div class="plugin-loading"><LoaderCircle size={14} class="spin"/>{t('加载中')}</div></Show>
    <Show when={detail()} fallback={<>
      <div class="plugin-list"><For each={entries()}>{entry => <article class="plugin-row">
        <span class="plugin-glyph"><PluginIcon plugin={entry} source={record(entry.id)?.source} sha256={record(entry.id)?.sha256} size={24}/></span><div class="plugin-summary">
          <button class="plugin-name" onClick={() => open(entry.id)}><span>{entry.name}</span><ChevronRight size={13}/></button>
          <p class="plugin-description">{entry.description}</p><div class="plugin-tags"><span>{entry.version}</span><span>{entry.repository || t('本地开发包')}</span><For each={entry.tags}>{tag => <span>{tagText(tag)}</span>}</For></div>
        </div><div class="plugin-row-actions">
          <Show when={record(entry.id) && record(entry.id)!.state !== 'removed'} fallback={<button class="secondary-button" title={entryError(entry) && t(entryError(entry)!)} disabled={Boolean(busy()) || !api.extensionNative || Boolean(entryError(entry))} onClick={() => void prepare(entry as ExtensionCatalogEntry)}><Download size={13}/>{t('安装')}</button>}>
            <Show when={availableUpdate(entry.id)} fallback={
            <button class="secondary-button" title={entryError(entry) && t(entryError(entry)!)} disabled={Boolean(busy()) || (record(entry.id)?.state !== 'enabled' && Boolean(entryError(entry)))} aria-pressed={record(entry.id)?.state === 'enabled'} onClick={() => void control(record(entry.id)!, record(entry.id)!.state === 'enabled' ? 'disable' : 'enable')}><Show when={record(entry.id)?.state === 'enabled'}><Check size={13}/></Show>{record(entry.id)?.state === 'enabled' ? t('已启用') : t('启用')}</button>
            }>{update => <button class="secondary-button" disabled={Boolean(busy()) || Boolean(unsupported(update()))} onClick={() => void prepare(update())}><RefreshCw size={13}/>{updateLabel(entry.id)}</button>}</Show>
          </Show><span class="plugin-status"><Show when={busy() === entry.id} fallback={entryError(entry) ? t(entryError(entry)!) : status(record(entry.id))}><LoaderCircle size={12} class="spin"/></Show></span>
        </div>
      </article>}</For></div>
      <Show when={api.extensionNative && !loading() && !error() && !entries().length && props.pane === 'discover'}><p class="plugin-empty">{props.query.trim() || props.filter ? t('无匹配插件') : t('目录为空')}</p></Show>
    </>}>{value => <div class="plugin-detail">
      <button class="text-button plugin-detail-back" onClick={back}><ArrowLeft size={14}/>{t('返回')}</button>
      <div class="plugin-detail-heading"><span class="plugin-glyph"><PluginIcon plugin={value().manifest} source={value().source} size={26}/></span><div><h3>{value().manifest.name}</h3><div class="plugin-tags"><span>{value().manifest.version}</span><span>{value().source.type === 'github' ? 'GitHub' : t('本地开发包')}</span><For each={value().manifest.tags}>{tag => <span>{tagText(tag)}</span>}</For></div></div></div>
      <p class="plugin-description">{value().manifest.description}</p>
      <ExtensionProjectGallery active={props.active} entry={value().source.type === 'github' ? (value().source as Extract<ExtensionSource, { type: 'github' }>).entry : undefined} name={value().manifest.name}/>
      <div class="plugin-detail-actions"><span class="plugin-status"><Show when={busy()} fallback={selectedError() ? t(selectedError()!) : proposal() ? t('待确认') : status(selectedRecord())}><LoaderCircle size={13} class="spin"/>{t('处理中')}</Show></span>
        <Show when={proposal()} fallback={<Show when={selectedRecord() && selectedRecord()!.state !== 'removed'} fallback={<button class="primary-button" disabled={Boolean(busy()) || Boolean(selectedError())} onClick={() => void prepare(selectedCatalog())}><Download size={13}/>{t('安装')}</button>}>
          <button class="secondary-button" title={selectedError() && t(selectedError()!)} disabled={Boolean(busy()) || (selectedRecord()?.state !== 'enabled' && Boolean(selectedError()))} onClick={() => void control(selectedRecord()!, selectedRecord()!.state === 'enabled' ? 'disable' : 'enable')}>{selectedRecord()?.state === 'enabled' ? t('停用') : t('启用')}</button>
          <Show when={updateEntry()}>{entry => <button class="secondary-button" title={unsupported(entry()) && t(unsupported(entry())!)} disabled={Boolean(busy()) || Boolean(unsupported(entry()))} onClick={() => void prepare(entry())}><RefreshCw size={13}/>{updateLabel(entry().id)}</button>}</Show>
          <Show when={selectedRecord()?.hasRollback}><button class="secondary-button" disabled={Boolean(busy()) || Boolean(selectedError())} onClick={() => void control(selectedRecord()!, 'rollback')}><Undo2 size={13}/>{t('恢复上一版')}</button></Show>
          <button class="icon-button" disabled={Boolean(busy())} aria-label={t('卸载插件')} title={t('卸载插件')} onClick={() => setConfirmRemoval(true)}><Trash2 size={15}/></button>
        </Show>}><button class="secondary-button" disabled={Boolean(busy())} onClick={back}>{t('取消')}</button><button class="primary-button" disabled={Boolean(busy()) || Boolean(selectedError())} onClick={() => void install()}><Download size={13}/>{proposal()?.sourceMigration ? proposal()?.sourceMigration?.operation === 'migrate-and-install' ? t('迁移并安装') : t('确认迁移') : proposal()?.expectedRevision && selectedRecord()?.state !== 'removed' ? t('更新') : t('安装')}</button></Show>
      </div>
      <Show when={confirmRemoval()}><div class="plugin-confirm"><span>{t('卸载')}{value().manifest.name}？</span><button class="text-button" disabled={Boolean(busy())} onClick={() => setConfirmRemoval(false)}>{t('取消')}</button><button class="secondary-button" disabled={Boolean(busy())} onClick={() => void control(selectedRecord()!, 'remove')}>{t('卸载')}</button></div></Show>
      <Show when={proposal()?.sourceMigration}>{migration => <section class="plugin-detail-section"><h4>{t('来源迁移')}</h4><dl class="plugin-metadata"><dt>{t('原来源')}</dt><dd>{sourceText(migration().from)}</dd><dt>{t('新来源')}</dt><dd>{sourceText(value().source)}</dd></dl><p class="plugin-description">{migration().operation === 'migrate-and-install' ? t('保留配置、凭据和数据，从新来源重新安装。') : t('保留配置、凭据和数据，启停状态不变。')}{t('新来源将接管后续更新及此插件的数据访问。')}</p></section>}</Show>
      <dl class="plugin-metadata"><Show when={value().source.type === 'local'}><dt>{t('来源')}</dt><dd>{sourceText(value().source)}</dd></Show><dt>{t('标识')}</dt><dd>{value().manifest.id}</dd>
        <Show when={'license' in value().manifest}><dt>{t('许可')}</dt><dd>{(value().manifest as ExtensionManifest).license}</dd></Show>
        <Show when={proposal() || selectedRecord()}><dt>SHA-256</dt><dd>{proposal()?.sha256 ?? selectedRecord()?.sha256}</dd></Show></dl>
      <Show when={proposal()}>{prepared => <section class="plugin-detail-section"><h4>{t('权限')}</h4><ul><For each={prepared().manifest.permissions}>{permission => <li><code>{permission}</code></li>}</For></ul><Show when={!prepared().manifest.permissions.length}><p class="plugin-description">{t('无')}</p></Show></section>}</Show>
      <Show when={!proposal() && selectedRecord()?.services.length}><section class="plugin-detail-section"><h4>{t('能力')}</h4><ul><For each={selectedRecord()?.services}>{service => <li>{service.title}<Show when={selectedRecord()?.unsupportedServices?.[service.id]} fallback={<Show when={service.description}><span class="plugin-description"> · {service.description}</span></Show>}>{reason => <span class="plugin-status"> · {t(reason())}</span>}</Show></li>}</For></ul></section></Show>
    </div>}</Show>
  </div>;
}
