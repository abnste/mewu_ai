// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { isCoreReplacementPlugin } from '../plugin-core';
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronRight, Download, FolderOpen, LoaderCircle, Pencil, Puzzle, RefreshCw, Search, Trash2, Undo2, X } from 'lucide-solid';
import * as bridge from '../plugin-bridge';
import type { CatalogEntry, PluginContribution, PluginManifest, PluginProposal, PluginRecord, PluginSnapshot, PluginSource, PluginTag } from '../plugin-contracts';
import { pluginModuleTags, pluginTagLabels, pluginTags } from '../plugin-categories';
import { SettingsAddButton } from './SettingControls';
import '../plugins.css';

interface Props { active: boolean }
type Kind = PluginContribution['kind'];
interface Pending { key: string; label: string }
const kindLabels: Record<Kind, string> = { 'model.connection': '模型接入', 'selection.workflow': '图像处理', 'selection.drawing-tools': '绘制', 'selection.ocr': '文字识别', 'selection.scroll': '长截图', 'selection.translation': '翻译', 'selection.pin': '贴图', 'selection.codes': '二维码与条码', 'selection.recording': '屏幕录制', 'artifact.video-trim': '视频裁剪', 'artifact.video-drawing-tools': '视频绘制', 'artifact.video-gif': 'GIF 导出', 'recording.audio': '声源', 'memory.provider': '长期记忆', 'input.speech-to-text': '语音输入', 'agent.visual-annotations': '原位作答' };
const toolLabels = { pen: '画笔', line: '直线', arrow: '箭头', rect: '矩形', ellipse: '椭圆', text: '文字', highlighter: '荧光笔', number: '序号', mosaic: '马赛克' };
const sourceLabel = (source: PluginSource) => source.type === 'official' ? t('官方') : source.type === 'local' ? t('本地开发包') : 'GitHub';
const sourceText = (source: PluginSource) => source.type === 'official' ? t('Mewu 随包插件') : source.type === 'local' ? source.path : `${source.repository}\n${source.path}\n${source.commit}`;
const githubUrl = (source: Extract<PluginSource, { type: 'github' }>) => `https://github.com/${source.repository}/blob/${encodeURIComponent(source.commit)}/${source.path.split('/').map(encodeURIComponent).join('/')}`;
const kinds = (manifest: PluginManifest) => [...new Set(manifest.contributions.map(value => value.kind))];

function ErrorNote(props: { value?: string; onClose?: () => void }) {
  return <Show when={props.value}><div class="plugin-error" role="alert"><span>{props.value}</span><Show when={props.onClose}><button class="icon-button compact" title={t("关闭提示")} aria-label={t("关闭插件提示")} onClick={() => props.onClose?.()}><X size={13} /></button></Show></div></Show>;
}

export default function PluginsPanel(props: Props) {
  const [snapshot, setSnapshot] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const [catalog, setCatalog] = createSignal<CatalogEntry[]>([]);
  const [catalogSource, setCatalogSource] = createSignal('');
  const [catalogDraft, setCatalogDraft] = createSignal('');
  const [pane, setPane] = createSignal<'discover' | 'installed'>('discover');
  const [query, setQuery] = createSignal('');
  const [filter, setFilter] = createSignal<PluginTag | ''>('');
  const [selected, setSelected] = createSignal('');
  const [proposal, setProposal] = createSignal<PluginProposal>();
  const [adding, setAdding] = createSignal(false);
  const [sourceEditor, setSourceEditor] = createSignal(false);
  const [url, setUrl] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [catalogLoading, setCatalogLoading] = createSignal(false);
  const [pending, setPending] = createSignal<Pending>();
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [confirmRemoval, setConfirmRemoval] = createSignal('');
  let disposed = false, initialized = false, catalogSerial = 0, navigationSerial = 0, stopEvents: (() => void) | undefined;
  const active = createMemo(() => props.active);
  const record = (id: string) => snapshot().plugins.find(value => value.manifest.id === id);
  const clearError = (key: string) => setErrors(previous => { const next = { ...previous }; delete next[key]; return next; });
  const report = (key: string, cause: unknown) => { if (!disposed) setErrors(previous => ({ ...previous, [key]: cause instanceof Error ? cause.message : String(cause) })); };
  const accept = (value: PluginSnapshot) => { if (!disposed && value.revision >= snapshot().revision) setSnapshot(value); };
  const busy = () => Boolean(pending());
  const disabled = () => busy() || !bridge.pluginNative;
  const sourceEntries = createMemo(() => pane() === 'discover' ? catalog().filter(value => !isCoreReplacementPlugin(value.manifest.id)) : snapshot().plugins.filter(value => value.state !== 'removed' && !isCoreReplacementPlugin(value.manifest.id)).map(value => ({ manifest: value.manifest, source: value.source, installed: value })));
  const entries = createMemo(() => {
    const text = query().trim().toLocaleLowerCase();
    return sourceEntries().filter(value => (!filter() || pluginModuleTags(value.manifest).includes(filter() as PluginTag)) && (!text || [value.manifest.name, value.manifest.id, value.manifest.description, value.manifest.publisher ?? '', ...pluginModuleTags(value.manifest).map(tag => t(pluginTagLabels[tag]))].join(' ').toLocaleLowerCase().includes(text)));
  });
  const entry = (id: string) => entries().find(value => value.manifest.id === id);
  const detail = createMemo(() => {
    const prepared = proposal();
    if (prepared) return { manifest: prepared.manifest, source: prepared.source };
    const installed = record(selected());
    if (installed && installed.state !== 'removed') return { manifest: installed.manifest, source: installed.source };
    return catalog().find(value => value.manifest.id === selected()) ?? (installed ? { manifest: installed.manifest, source: installed.source } : undefined);
  });
  const installedDetail = () => detail() ? record(detail()!.manifest.id) : undefined;
  const officialVersion = () => catalog().find(value => value.source.type === 'official' && value.manifest.id === installedDetail()?.manifest.id)?.manifest.version;

  async function refreshCatalog(sourceUrl?: string) {
    const serial = ++catalogSerial; setCatalogLoading(true); clearError('catalog');
    try {
      const value = await bridge.getPluginCatalog(sourceUrl);
      if (disposed || serial !== catalogSerial) return;
      setCatalog(value.entries); setCatalogSource(value.source ?? '');
      if (!value.error && (!sourceEditor() || sourceUrl !== undefined)) setCatalogDraft(value.source ?? '');
      if (value.error) report('catalog', value.error);
      else if (sourceUrl !== undefined) setSourceEditor(false);
    } catch (cause) { if (serial === catalogSerial) report('catalog', cause); }
    finally { if (!disposed && serial === catalogSerial) setCatalogLoading(false); }
  }
  async function initialize() {
    if (loading() || disposed) return;
    setLoading(true); clearError('load');
    try {
      if (!stopEvents) stopEvents = await bridge.subscribePlugins(accept);
      if (disposed) { stopEvents(); return; }
      accept(await bridge.getPlugins());
      await refreshCatalog();
      initialized = true;
    } catch (cause) { report('load', cause); }
    finally { if (!disposed) setLoading(false); }
  }
  createEffect(on(active, value => { if (value && !initialized) void initialize(); }));
  onCleanup(() => { disposed = true; catalogSerial++; stopEvents?.(); });

  function open(id: string) { navigationSerial++; setSelected(id); setProposal(undefined); setConfirmRemoval(''); setAdding(false); }
  function back() { navigationSerial++; setSelected(''); setProposal(undefined); setConfirmRemoval(''); }
  async function operation(key: string, label: string, action: () => Promise<void>) {
    if (busy()) return;
    setPending({ key, label }); clearError(key);
    try { await action(); } catch (cause) { report(key, cause); }
    finally { if (!disposed) setPending(undefined); }
  }
  async function prepare(source?: string, key = 'add') {
    const navigation = navigationSerial;
    await operation(key, '读取中', async () => {
      const value = await bridge.preparePlugin(source);
      if (disposed || !value || navigation !== navigationSerial) return;
      setProposal(value); setSelected(value.manifest.id); setAdding(false); setConfirmRemoval('');
    });
  }
  async function commit() {
    const value = proposal(); if (!value) return;
    await operation(value.manifest.id, '安装中', async () => {
      accept(await bridge.installPlugin(value.id));
      if (!disposed && proposal()?.id === value.id) { setProposal(undefined); setSelected(value.manifest.id); }
    });
  }
  async function change(value: PluginRecord, action: 'enable' | 'disable' | 'remove' | 'rollback' | 'reinstall') {
    const id = value.manifest.id;
    const labels = { enable: '启用中', disable: '停用中', remove: '卸载中', rollback: '恢复中', reinstall: '安装中' };
    await operation(id, labels[action], async () => {
      const result = action === 'remove' ? await bridge.uninstallPlugin(id, value.revision)
        : action === 'rollback' ? await bridge.rollbackPlugin(id, value.revision)
          : action === 'reinstall' ? await bridge.reinstallOfficialPlugin(id, value.revision)
            : await bridge.setPluginEnabled(id, value.revision, action === 'enable');
      accept(result); if (!disposed) setConfirmRemoval('');
    });
  }
  async function update(value: PluginRecord) {
    const navigation = navigationSerial;
    await operation(value.manifest.id, '检查更新', async () => {
      const next = await bridge.preparePluginUpdate(value.manifest.id, value.revision);
      if (!disposed && navigation === navigationSerial) { setProposal(next); setSelected(next.manifest.id); setConfirmRemoval(''); }
    });
  }
  async function installEntry(value: CatalogEntry) {
    open(value.manifest.id);
    const installed = record(value.manifest.id);
    if (value.source.type === 'official') {
      if (installed) await change(installed, 'reinstall');
      else report(value.manifest.id, '插件状态尚未就绪，请刷新');
    } else if (value.source.type === 'github') await prepare(githubUrl(value.source), value.manifest.id);
    else await prepare(undefined, value.manifest.id);
  }
  const status = (value?: PluginRecord) => !value || value.state === 'removed' ? t('未安装') : value.error ? t('加载失败') : value.state === 'enabled' ? t('已启用') : t('已停用');

  function keyboard(event: KeyboardEvent) {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key !== 'Escape') return;
    if (confirmRemoval()) setConfirmRemoval('');
    else if (detail()) back();
    else if (adding()) { navigationSerial++; setAdding(false); }
    else if (sourceEditor()) setSourceEditor(false);
    else return;
    event.preventDefault(); event.stopPropagation();
  }
  return <section class="plugins-panel" aria-label={t("插件")} onKeyDown={keyboard}>
    <header class="settings-page-heading"><h2>{t("插件")}</h2><Show when={!detail()}><SettingsAddButton disabled={disabled()} title={bridge.pluginNative ? undefined : t('请在桌面版添加插件')} onClick={() => { setAdding(!adding()); setSourceEditor(false); }} /></Show></header>
    <Show when={!detail()}><div class="segmented"><button aria-pressed={pane() === 'discover'} classList={{ selected: pane() === 'discover' }} onClick={() => setPane('discover')}>{t("插件市场")}</button><button aria-pressed={pane() === 'installed'} classList={{ selected: pane() === 'installed' }} onClick={() => setPane('installed')}>{t("已安装")}</button></div></Show>
    <Show when={!bridge.pluginNative}><p class="plugin-empty">{t("预览 · 安装仅限桌面版")}</p></Show>
    <ErrorNote value={errors().load} onClose={() => clearError('load')} />
    <Show when={errors().load}><button class="text-button" disabled={loading()} onClick={() => void initialize()}>{t("重试")}</button></Show>
    <Show when={!detail()} fallback={<Show when={detail()}>{value => <div class="plugin-detail">
      <button class="text-button plugin-detail-back" onClick={back}><ArrowLeft size={14} />{t("返回")}</button>
      <div class="plugin-detail-heading"><span class="plugin-glyph"><Show when={kinds(value().manifest).includes('selection.drawing-tools')} fallback={<Puzzle size={20} />}><Pencil size={20} /></Show></span><div><h3>{value().manifest.name}</h3><div class="plugin-tags"><span>{value().manifest.version}</span><span>{sourceLabel(value().source)}</span><For each={pluginModuleTags(value().manifest)}>{tag => <span>{t(pluginTagLabels[tag])}</span>}</For></div></div></div>
      <p class="plugin-description">{value().manifest.description}</p>
      <div class="plugin-detail-actions">
        <span class="plugin-status" classList={{ error: Boolean(installedDetail()?.error) }}><Show when={pending()?.key === value().manifest.id} fallback={proposal() ? t('待确认') : status(installedDetail())}><LoaderCircle size={13} class="spin" />{t(pending()!.label)}</Show></span>
        <Show when={proposal()} fallback={<Show when={installedDetail() && installedDetail()!.state !== 'removed'} fallback={<button class="primary-button" disabled={disabled()} onClick={() => void installEntry(value())}><Download size={13} />{t("安装")}</button>}>
          <button class="secondary-button" disabled={disabled()} onClick={() => void change(installedDetail()!, installedDetail()!.state === 'enabled' ? 'disable' : 'enable')}>{installedDetail()?.state === 'enabled' ? t('停用') : t('启用')}</button>
          <Show when={installedDetail()?.source.type !== 'official'}><button class="secondary-button" disabled={disabled()} onClick={() => void update(installedDetail()!)}><RefreshCw size={13} />{t("检查更新")}</button></Show>
          <Show when={installedDetail()?.source.type === 'official' && officialVersion() && officialVersion() !== installedDetail()?.manifest.version}><button class="secondary-button" disabled={disabled()} onClick={() => void change(installedDetail()!, 'reinstall')}><RefreshCw size={13} />{t("安装")}{officialVersion()}</button></Show>
          <Show when={installedDetail()?.hasRollback}><button class="secondary-button" disabled={disabled()} onClick={() => void change(installedDetail()!, 'rollback')}><Undo2 size={13} />{t("恢复上一版")}</button></Show>
          <button class="icon-button" title={t("卸载插件")} aria-label={t("卸载插件")} disabled={disabled()} onClick={() => setConfirmRemoval(value().manifest.id)}><Trash2 size={15} /></button>
        </Show>}><button class="secondary-button" disabled={busy()} onClick={() => setProposal(undefined)}>{t("取消")}</button><button class="primary-button" disabled={disabled()} onClick={() => void commit()}><Download size={13} />{installedDetail() && installedDetail()!.state !== 'removed' ? t('安装更新') : t('安装')}</button></Show>
      </div>
      <Show when={confirmRemoval() === value().manifest.id}><div class="plugin-confirm"><span>{t("卸载")}{value().manifest.name}？</span><button class="text-button" disabled={busy()} onClick={() => setConfirmRemoval('')}>{t("取消")}</button><button class="secondary-button" disabled={disabled()} onClick={() => void change(installedDetail()!, 'remove')}>{t("卸载")}</button></div></Show>
      <ErrorNote value={errors()[value().manifest.id] ?? installedDetail()?.error} onClose={errors()[value().manifest.id] ? () => clearError(value().manifest.id) : undefined} />
      <dl class="plugin-metadata"><dt>{t("来源")}</dt><dd>{sourceText(value().source)}</dd><Show when={value().manifest.publisher}><dt>{t("发布者")}</dt><dd>{value().manifest.publisher}</dd></Show><dt>{t("许可")}</dt><dd>{value().manifest.license}</dd><dt>{t("标识")}</dt><dd>{value().manifest.id}</dd><Show when={proposal()}><dt>SHA-256</dt><dd>{proposal()!.sha256}</dd></Show></dl>
      <section class="plugin-detail-section"><h4>{t("操作")}</h4><ul><For each={value().manifest.contributions}>{contribution => <li>{contribution.title}<Show when={contribution.kind === 'selection.drawing-tools'}><span class="plugin-description"> · {(contribution as Extract<PluginContribution, { kind: 'selection.drawing-tools' }>).tools.map(tool => t(toolLabels[tool])).join('、')}</span></Show></li>}</For></ul><Show when={kinds(value().manifest).includes('selection.workflow')}><p class="plugin-description">{t("使用当前选区图片和连接")}</p></Show></section>
      <For each={value().manifest.contributions.filter((contribution): contribution is Extract<PluginContribution, { kind: 'selection.workflow' }> => contribution.kind === 'selection.workflow')}>{contribution => <details class="plugin-detail-section"><summary>{t("提示词")}<ChevronRight size={12} /></summary><p class="plugin-description">{contribution.prompt}</p></details>}</For>
      <For each={value().manifest.contributions.filter((contribution): contribution is Extract<PluginContribution, { kind: 'model.connection' }> => contribution.kind === 'model.connection')}>{contribution => <section class="plugin-detail-section"><h4>{contribution.title}</h4><dl class="plugin-metadata"><dt>{t("API 地址")}</dt><dd>{contribution.template.baseUrl || t('自定义')}</dd><dt>{t("API 格式")}</dt><dd>{contribution.template.advanced.protocol === 'chat_completions' ? 'Chat Completions' : contribution.template.advanced.protocol === 'openai_responses' ? 'Responses' : 'Anthropic Messages'}</dd><dt>{t("认证方式")}</dt><dd>{contribution.template.advanced.authMode === 'api_key' ? 'x-api-key' : contribution.template.advanced.authMode === 'bearer' ? 'Bearer' : t('无')}</dd><Show when={contribution.template.model}><dt>{t("模型")}</dt><dd>{contribution.template.model}</dd></Show><Show when={contribution.template.advanced.requestPath}><dt>{t("请求路径")}</dt><dd>{contribution.template.advanced.requestPath}</dd></Show><Show when={Object.keys(contribution.template.advanced.requestParameters).length}><dt>{t("请求参数")}</dt><dd>{JSON.stringify(contribution.template.advanced.requestParameters)}</dd></Show></dl></section>}</For>
    </div>}</Show>}>
      <Show when={adding()}><div class="plugin-add"><header><h3>{t("添加插件")}</h3><button class="icon-button compact" title={t("关闭")} aria-label={t("关闭添加插件")} onClick={() => { navigationSerial++; setAdding(false); }}><X size={14} /></button></header><label class="field-label">{t("GitHub 插件地址")}<input type="url" aria-label={t("GitHub 插件地址")} placeholder="https://github.com/…/mewu-plugin.json" value={url()} spellcheck={false} disabled={busy()} onInput={event => setUrl(event.currentTarget.value)} /></label><ErrorNote value={errors().add} onClose={() => clearError('add')} /><div class="plugin-add-actions"><button class="secondary-button" disabled={disabled()} onClick={() => void prepare()}><FolderOpen size={14} />{t("选择本地包")}</button><button class="primary-button" disabled={disabled() || !url().trim()} onClick={() => void prepare(url().trim())}><Show when={pending()?.key === 'add'} fallback={<Download size={13} />}><LoaderCircle size={13} class="spin" /></Show>{t("读取")}</button></div></div></Show>
      <div class="plugins-toolbar"><label class="plugins-search"><Search size={14} /><input type="search" aria-label={t("搜索插件")} placeholder={t("搜索插件")} value={query()} onInput={event => setQuery(event.currentTarget.value)} /></label><select aria-label={t("插件类型")} value={filter()} onChange={event => setFilter(event.currentTarget.value as PluginTag | '')}><option value="">{t("全部类型")}</option><For each={pluginTags}>{tag => <option value={tag}>{t(pluginTagLabels[tag])}</option>}</For></select><button class="icon-button" title={t("刷新插件")} aria-label={t("刷新插件")} disabled={loading() || catalogLoading() || busy()} onClick={() => void initialize()}><RefreshCw size={15} classList={{ spin: catalogLoading() || loading() }} /></button></div>
      <Show when={pane() === 'discover'}><button class="text-button" disabled={!bridge.pluginNative || busy()} onClick={() => { setSourceEditor(!sourceEditor()); setCatalogDraft(catalogSource()); setAdding(false); }}>{t("目录来源")}</button><Show when={sourceEditor()}><div class="plugin-add"><label class="field-label">{t("GitHub 目录地址")}<input aria-label={t("插件目录地址")} type="url" placeholder="https://github.com/…/catalog.json" value={catalogDraft()} spellcheck={false} disabled={catalogLoading()} onInput={event => setCatalogDraft(event.currentTarget.value)} /></label><div class="plugin-add-actions"><button class="text-button" disabled={catalogLoading()} onClick={() => setSourceEditor(false)}>{t("取消")}</button><button class="primary-button" disabled={catalogLoading() || busy()} onClick={() => void refreshCatalog(catalogDraft().trim())}>{t("读取")}</button></div></div></Show><ErrorNote value={errors().catalog} onClose={() => clearError('catalog')} /></Show>
      <Show when={loading() && snapshot().revision < 0}><div class="plugin-loading"><LoaderCircle size={14} class="spin" />{t("加载中")}</div></Show>
      <div class="plugin-list"><For each={entries().map(value => value.manifest.id)}>{id => <Show when={entry(id)}>{value => <article class="plugin-row"><span class="plugin-glyph"><Show when={kinds(value().manifest).includes('selection.drawing-tools')} fallback={<Puzzle size={18} />}><Pencil size={18} /></Show></span><div class="plugin-summary"><button class="plugin-name" onClick={() => open(id)}><span>{value().manifest.name}</span><ChevronRight size={13} /></button><p class="plugin-description">{value().manifest.description}</p><div class="plugin-tags"><span>{value().manifest.version}</span><span>{sourceLabel(value().source)}</span><For each={pluginModuleTags(value().manifest)}>{tag => <span>{t(pluginTagLabels[tag])}</span>}</For></div><ErrorNote value={errors()[id]} onClose={() => clearError(id)} /></div><div class="plugin-row-actions"><Show when={record(id) && record(id)!.state !== 'removed'} fallback={<button class="secondary-button" disabled={disabled()} onClick={() => void installEntry(value())}><Download size={12} />{t("安装")}</button>}><button class="secondary-button" title={record(id)?.state === 'enabled' ? t('停用插件') : t('启用插件')} aria-pressed={record(id)?.state === 'enabled'} disabled={disabled()} onClick={() => void change(record(id)!, record(id)!.state === 'enabled' ? 'disable' : 'enable')}><Show when={record(id)?.state === 'enabled'}><Check size={12} /></Show>{record(id)?.state === 'enabled' ? t('已启用') : t('启用')}</button></Show><span class="plugin-status" classList={{ error: Boolean(record(id)?.error) }}><Show when={pending()?.key === id} fallback={status(record(id))}><LoaderCircle size={12} class="spin" />{t(pending()!.label)}</Show></span></div></article>}</Show>}</For></div>
      <Show when={!loading() && !catalogLoading() && entries().length === 0 && !errors().load && !errors().catalog}><p class="plugin-empty">{query().trim() || filter() ? t('无匹配插件') : pane() === 'installed' ? t('没有已安装插件') : t('目录为空')}</p></Show>
    </Show>
  </section>;
}
