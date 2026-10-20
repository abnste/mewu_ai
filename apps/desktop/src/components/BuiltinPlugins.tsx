// SPDX-License-Identifier: MPL-2.0
// Fixed host modules only. Community code uses the executable plugin runtime.
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronRight, Download, LoaderCircle, Trash2 } from 'lucide-solid';
import * as bridge from '../plugin-bridge';
import type { PluginManifest, PluginRecord, PluginSnapshot, PluginTag } from '../plugin-contracts';
import { pluginModuleTags, pluginTagLabels } from '../plugin-categories';
import { declarativePluginError, type HostPlatform } from '../plugin-platform';
import { isCoreReplacementPlugin } from '../plugin-core';
import { t } from '../i18n';
import PluginIcon from './PluginIcon';

interface Props { active: boolean; platform?: HostPlatform; pane: 'discover' | 'installed'; query: string; filter: string; refreshRequest: number; onDetail: (open: boolean) => void }
export default function BuiltinPlugins(props: Props) {
  const [snapshot, setSnapshot] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const [catalog, setCatalog] = createSignal<PluginManifest[]>([]);
  const [selected, setSelected] = createSignal(''), [busy, setBusy] = createSignal(''), [error, setError] = createSignal(''), [confirm, setConfirm] = createSignal(false);
  let disposed = false, initialized = false, loading = false, events = 0, unsubscribe: (() => void) | undefined;
  const accept = (value: PluginSnapshot) => { if (!disposed && value.revision >= snapshot().revision) setSnapshot(value); };
  const record = (id: string) => snapshot().plugins.find(value => value.source.type === 'official' && value.manifest.id === id);
  const entries = createMemo(() => catalog().filter(value => !isCoreReplacementPlugin(value.id)
    && (props.pane === 'discover' || Boolean(record(value.id) && record(value.id)!.state !== 'removed'))
    && (!props.filter || pluginModuleTags(value).includes(props.filter as PluginTag))
    && (!props.query.trim() || [value.id, value.name, value.description, ...pluginModuleTags(value).map(tag => t(pluginTagLabels[tag]))].join(' ').toLocaleLowerCase().includes(props.query.trim().toLocaleLowerCase()))));
  const detail = () => catalog().find(value => value.id === selected());
  const unsupported = (manifest: PluginManifest) => declarativePluginError(manifest, props.platform);
  const status = (value?: PluginRecord) => !value || value.state === 'removed' ? t('未安装') : value.state === 'enabled' ? t('已启用') : t('已停用');
  createEffect(() => props.onDetail(Boolean(detail())));
  async function refresh() {
    if (loading || disposed) return; loading = true; setError('');
    try {
      if (!unsubscribe) unsubscribe = await bridge.subscribePlugins(value => { events++; accept(value); }, 'settings');
      if (disposed) { unsubscribe?.(); return; }
      const before = events, values = await Promise.all([bridge.getPlugins(), bridge.getOfficialPlugins()]);
      if (!disposed) { if (events === before) accept(values[0]); setCatalog(values[1]); initialized = true; }
    } catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { loading = false; }
  }
  createEffect(on(() => props.active, active => { if (active && !initialized) void refresh(); }));
  createEffect(on(() => props.refreshRequest, () => { if (props.active) void refresh(); }, { defer: true }));
  createEffect(on(() => props.pane, () => { setSelected(''); setConfirm(false); }, { defer: true }));
  onCleanup(() => { disposed = true; unsubscribe?.(); props.onDetail(false); });
  async function control(manifest: PluginManifest, action: 'enable' | 'disable' | 'remove' | 'restore') {
    const current = record(manifest.id); if (busy() || !current) return;
    setBusy(manifest.id); setError('');
    try { accept(await bridge.controlOfficialPlugin(manifest.id, current.revision, action)); if (!disposed) { setConfirm(false); if (action === 'remove') setSelected(''); } }
    catch (cause) { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setBusy(''); }
  }
  const action = (manifest: PluginManifest) => {
    const current = record(manifest.id), removed = !current || current.state === 'removed';
    return <button class="secondary-button" aria-pressed={current?.state === 'enabled'} title={unsupported(manifest) && t(unsupported(manifest)!)} disabled={!bridge.pluginNative || Boolean(busy()) || (current?.state !== 'enabled' && Boolean(unsupported(manifest)))} onClick={() => void control(manifest, removed ? 'restore' : current!.state === 'enabled' ? 'disable' : 'enable')}><Show when={removed} fallback={<Show when={current?.state === 'enabled'}><Check size={13}/></Show>}><Download size={13}/></Show>{removed ? t('安装') : current!.state === 'enabled' ? t('已启用') : t('启用')}</button>;
  };
  return <div class="builtin-plugins" onKeyDown={event => { if (event.key === 'Escape' && detail()) { event.preventDefault(); event.stopPropagation(); setSelected(''); setConfirm(false); } }}>
    <Show when={error()}><div class="plugin-error" role="alert">{error()}</div></Show>
    <Show when={detail()} fallback={<div class="plugin-list"><For each={entries()}>{manifest => <article class="plugin-row"><span class="plugin-glyph"><PluginIcon plugin={{ ...manifest, tags: pluginModuleTags(manifest) }} kind={manifest.contributions[0]?.kind} size={24}/></span><div class="plugin-summary"><button class="plugin-name" onClick={() => { setSelected(manifest.id); setConfirm(false); }}><span>{manifest.name}</span><ChevronRight size={13}/></button><p class="plugin-description">{manifest.description}</p><div class="plugin-tags"><span>{manifest.version}</span><span>{t('官方')}</span><For each={pluginModuleTags(manifest)}>{tag => <span>{t(pluginTagLabels[tag])}</span>}</For></div></div><div class="plugin-row-actions">{action(manifest)}<span class="plugin-status"><Show when={busy() === manifest.id} fallback={unsupported(manifest) ? t(unsupported(manifest)!) : status(record(manifest.id))}><LoaderCircle size={12} class="spin"/></Show></span></div></article>}</For></div>}>{manifest => <div class="plugin-detail">
      <button class="text-button plugin-detail-back" onClick={() => { setSelected(''); setConfirm(false); }}><ArrowLeft size={14}/>{t('返回')}</button>
      <div class="plugin-detail-heading"><span class="plugin-glyph"><PluginIcon plugin={{ ...manifest(), tags: pluginModuleTags(manifest()) }} kind={manifest().contributions[0]?.kind} size={26}/></span><div><h3>{manifest().name}</h3><div class="plugin-tags"><span>{manifest().version}</span><span>{t('官方')}</span></div></div></div>
      <p class="plugin-description">{manifest().description}</p><div class="plugin-detail-actions"><span class="plugin-status">{status(record(manifest().id))}</span>{action(manifest())}<Show when={record(manifest().id)?.state !== 'removed'}><button class="icon-button" aria-label={t('卸载插件')} disabled={Boolean(busy())} onClick={() => setConfirm(true)}><Trash2 size={15}/></button></Show></div>
      <Show when={confirm()}><div class="plugin-confirm"><span>{t('卸载')}{manifest().name}？</span><button class="text-button" onClick={() => setConfirm(false)}>{t('取消')}</button><button class="secondary-button" disabled={Boolean(busy())} onClick={() => void control(manifest(), 'remove')}>{t('卸载')}</button></div></Show>
      <dl class="plugin-metadata"><dt>{t('来源')}</dt><dd>{t('官方')}</dd><dt>{t('许可')}</dt><dd>{manifest().license}</dd><dt>{t('标识')}</dt><dd>{manifest().id}</dd></dl><section class="plugin-detail-section"><h4>{t('能力')}</h4><ul><For each={manifest().contributions}>{value => <li>{value.title}</li>}</For></ul></section>
    </div>}</Show>
  </div>;
}
