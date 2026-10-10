// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Check, ChevronLeft, ChevronRight, LoaderCircle, Plus, RefreshCw, Trash2, X } from 'lucide-solid';
import type { AgentMemoryStatus, AgentProfile, MemoryBindingView, MemoryEvidenceDetail, MemoryEvidencePage, MemoryEvidenceView, MemoryProviderProbe, MemoryWriteReceipt, Scene } from '../contracts';
import type { PluginRecord, PluginSnapshot } from '../plugin-contracts';
import * as api from '../memory-provider-bridge';
import { getPlugins, subscribePlugins } from '../plugin-bridge';
import { manualMemoryText, memoryBindingLabels, memoryEvidenceLabel, memoryPolicySettled, memoryReasonLabel, memorySelectionSettled, memoryWriteLabel, mergeMemoryStatus, MemoryRequestScope, normalizeMemoryEndpoint } from '../memory-provider-state';
import '../memory-provider.css';

interface Props { agentId: string; agent?: AgentProfile; active: boolean; scenes: Scene[] }
interface ConfigDraft { endpoint: string; key: string; expectedRevision: number }
interface SourceDraft { id: string | null; revision: number }
interface SyncDraft { value: boolean; revision: number }
interface TextDraft { text: string; requestId: string; receipt?: MemoryWriteReceipt }
interface Foreground { requestId: string; agentId: string; kind: 'probe' | 'create'; draft: ConfigDraft }
const message = (cause: unknown) => cause instanceof Error ? cause.message : String(cause);
const originLabels = { manual: '手动记录', user_quote: '对话中记录', completed_turn: '完整对话' };

export default function MemoryProviderSettings(props: Props) {
  const [states, setStates] = createSignal<Record<string, AgentMemoryStatus>>({});
  const [plugins, setPlugins] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const [configDrafts, setConfigDrafts] = createSignal<Record<string, ConfigDraft>>({});
  const [sourceDrafts, setSourceDrafts] = createSignal<Record<string, SourceDraft>>({});
  const [syncDrafts, setSyncDrafts] = createSignal<Record<string, SyncDraft>>({});
  const [textDrafts, setTextDrafts] = createSignal<Record<string, TextDraft>>({});
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [configOpen, setConfigOpen] = createSignal<Record<string, boolean>>({});
  const [expanded, setExpanded] = createSignal('');
  const [recordsOpen, setRecordsOpen] = createSignal(false);
  const [foreground, setForeground] = createSignal<Foreground>();
  const [verified, setVerified] = createSignal<{ draft: ConfigDraft; result: MemoryProviderProbe }>();
  const [pending, setPending] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [page, setPage] = createSignal<MemoryEvidencePage>();
  const [pageLoading, setPageLoading] = createSignal(false);
  const [pageIndex, setPageIndex] = createSignal(0);
  const [pageCursors, setPageCursors] = createSignal<(string | undefined)[]>([undefined]);
  const [bindingCursors, setBindingCursors] = createSignal<(string | undefined)[]>([undefined]);
  const [bindingPage, setBindingPage] = createSignal(0);
  const [detailId, setDetailId] = createSignal('');
  const [detail, setDetail] = createSignal<MemoryEvidenceDetail>();
  const [detailLoading, setDetailLoading] = createSignal(false);
  const scope = new MemoryRequestScope();
  let disposed = false, stateEpoch = 0, pageEpoch = 0, detailEpoch = 0;
  let stateFlight = false, stateAgain = false, listening: Promise<void> | undefined;
  let stopState: (() => void) | undefined, stopPlugins: (() => void) | undefined;
  const agentId = createMemo(() => props.agentId);
  const visible = createMemo(() => props.active && Boolean(props.agent));
  const state = () => states()[props.agentId];
  const selection = () => state()?.selection ?? props.agent?.memoryProvider ?? { revision: 0, bindingId: null };
  const bindings = () => state()?.bindings ?? [];
  const binding = (id: string) => bindings().find(value => value.id === id);
  const source = () => sourceDrafts()[props.agentId] ?? { id: selection().bindingId, revision: selection().revision };
  const config = () => configDrafts()[props.agentId];
  const errorKey = (part: string) => `${props.agentId}/${part}`;
  const error = (part: string) => errors()[errorKey(part)] ?? '';
  const setError = (id: string, part: string, text: string) => setErrors(old => ({ ...old, [`${id}/${part}`]: text }));
  const availablePlugin = (value?: MemoryBindingView): PluginRecord | undefined => plugins().plugins.find(record => record.manifest.id === (value?.pluginId ?? 'mewu.memory-hindsight') && record.state === 'enabled' && !record.error && record.manifest.contributions.some(contribution => contribution.id === (value?.contributionId ?? 'hindsight') && contribution.kind === 'memory.provider' && contribution.adapter === 'hindsight'));
  const selectable = (value?: MemoryBindingView) => Boolean(value && availablePlugin(value) && value.configured && (value.status === 'ready' || value.status === 'suspended'));
  const authorized = (value?: MemoryBindingView) => Boolean(value && value.enabled && value.status === 'ready' && availablePlugin(value)?.revision === value.pluginRevision);
  const sync = (value: MemoryBindingView) => syncDrafts()[value.id] ?? { value: value.policy.completedTurnSync, revision: value.revision };
  const activeBinding = createMemo(() => binding(expanded()));
  const ledgerKey = createMemo(() => `${props.agentId}/${expanded()}/${activeBinding()?.ledgerRevision ?? -1}/${visible() && recordsOpen()}`);
  const textDraft = () => textDrafts()[expanded()];
  const canWrite = (value: MemoryBindingView) => Boolean(authorized(value) && selection().bindingId === value.id && props.agent?.memoryEnabled && value.policy.explicitRetain);

  function accept(id: string, next: AgentMemoryStatus) {
    if (disposed) return;
    setStates(old => ({ ...old, [id]: mergeMemoryStatus(old[id], next) }));
  }
  function cancelForeground() {
    scope.invalidate();
    const operation = foreground(); setForeground(undefined); setVerified(undefined);
    if (operation) void api.cancelMemoryRequest(operation.requestId).catch(() => undefined);
  }
  async function subscribe() {
    if (listening) return listening;
    listening = (async () => {
      stopState = await api.subscribeMemoryProvider(id => { if (visible() && props.agentId === id) refreshState(); });
      if (disposed) { stopState(); return; }
      stopPlugins = await subscribePlugins(next => {
        if (disposed || next.revision < plugins().revision) return;
        setPlugins(next); cancelForeground(); if (visible()) refreshState();
      });
      if (disposed) { stopPlugins(); return; }
      const next = await getPlugins();
      if (!disposed && next.revision >= plugins().revision) setPlugins(next);
    })().catch(cause => { stopState?.(); stopPlugins?.(); stopState = undefined; stopPlugins = undefined; listening = undefined; throw cause; });
    return listening;
  }
  async function loadState(cursor?: string, destination = 0) {
    if (stateFlight || !visible() || !api.memoryProviderNative) { if (stateFlight) stateAgain = true; return; }
    const id = props.agentId, epoch = ++stateEpoch;
    stateFlight = true; setLoading(true); setError(id, 'load', '');
    try {
      await subscribe();
      if (disposed || epoch !== stateEpoch || !visible() || id !== props.agentId) return;
      const next = await api.getMemoryProviderState({ agentId: id, cursor, limit: 25 });
      if (disposed || epoch !== stateEpoch || !visible() || id !== props.agentId) return;
      accept(id, next); setBindingPage(destination); setBindingCursors(old => [...old.slice(0, destination), cursor]);
    } catch (cause) { if (!disposed && epoch === stateEpoch && id === props.agentId) setError(id, 'load', message(cause)); }
    finally {
      stateFlight = false;
      if (!disposed) {
        setLoading(false);
        if (stateAgain) { stateAgain = false; if (visible()) void loadState(); }
      }
    }
  }
  function refreshState() { stateEpoch++; if (stateFlight) stateAgain = true; else void loadState(); }
  createEffect(on([agentId, visible], () => {
    cancelForeground(); stateEpoch++; pageEpoch++; detailEpoch++;
    setExpanded(''); setRecordsOpen(false); setDetail(undefined); setDetailId(''); setPage(undefined);
    setBindingCursors([undefined]); setBindingPage(0);
    if (visible()) refreshState();
  }));
  const providerRevision = createMemo(() => props.agent?.memoryProvider?.revision ?? 0);
  createEffect(on(providerRevision, (_, previous) => { if (previous !== undefined && visible()) refreshState(); }));
  onCleanup(() => { disposed = true; stateEpoch++; pageEpoch++; detailEpoch++; cancelForeground(); scope.dispose(); stopState?.(); stopPlugins?.(); setConfigDrafts({}); });

  function toggleConfig() {
    cancelForeground(); const id = props.agentId;
    if (!configDrafts()[id]) setConfigDrafts(old => ({ ...old, [id]: { endpoint: '', key: '', expectedRevision: selection().revision } }));
    setConfigOpen(old => ({ ...old, [id]: !old[id] }));
  }
  function editConfig(patch: Partial<ConfigDraft>) {
    cancelForeground(); setError(props.agentId, 'config', '');
    setConfigDrafts(old => ({ ...old, [props.agentId]: { ...old[props.agentId], ...patch } }));
  }
  async function configure(kind: Foreground['kind']) {
    const value = config(), plugin = availablePlugin(), id = props.agentId;
    if (!value || !plugin || foreground() || pending()) return;
    if (kind === 'create' && verified()?.draft !== value) return;
    const requestId = crypto.randomUUID(), operation = { requestId, agentId: id, kind, draft: value };
    scope.invalidate(); const currentScope = scope.capture();
    setForeground(operation); setError(id, 'config', '');
    const current = () => currentScope() && visible() && props.agentId === id && configDrafts()[id] === value && foreground() === operation;
    try {
      const args = { requestId, agentId: id, pluginId: plugin.manifest.id, pluginRevision: plugin.revision, contributionId: 'hindsight', endpoint: normalizeMemoryEndpoint(value.endpoint), apiKey: value.key };
      if (kind === 'probe') {
        const result = await api.probeMemoryProvider(args); if (current()) setVerified({ draft: value, result });
      } else {
        const next = await api.createMemoryBinding({ ...args, expectedProviderRevision: value.expectedRevision });
        if (disposed) return;
        if (props.agentId === id) { stateEpoch++; setBindingPage(0); setBindingCursors([undefined]); }
        accept(id, next);
        if (current()) {
          setConfigDrafts(old => ({ ...old, [id]: { endpoint: '', key: '', expectedRevision: next.selection.revision } }));
          setConfigOpen(old => ({ ...old, [id]: false })); setVerified(undefined);
        }
      }
    } catch (cause) { if (current()) { setError(id, 'config', message(cause)); if (kind === 'create') refreshState(); } }
    finally { if (!disposed && foreground() === operation) setForeground(undefined); }
  }
  async function mutate(key: string, operation: () => Promise<AgentMemoryStatus>, settled?: (next: AgentMemoryStatus) => void) {
    if (pending() || foreground()) return;
    const id = props.agentId; setPending(key); setError(id, key, ''); stateEpoch++;
    try {
      const next = await operation();
      if (disposed) return;
      if (props.agentId === id) { stateEpoch++; setBindingPage(0); setBindingCursors([undefined]); }
      accept(id, next); settled?.(next);
    } catch (cause) { if (!disposed) { setError(id, key, message(cause)); if (props.agentId === id && visible()) refreshState(); } }
    finally { if (!disposed) setPending(''); }
  }
  function useSource(id = source().id) {
    const agent = props.agentId, value = source(), target = id ? binding(id) : undefined;
    if (id && !target) return;
    const expectedProviderRevision = sourceDrafts()[agent]?.revision ?? selection().revision;
    void mutate('source', () => api.selectMemoryProvider({ agentId: agent, bindingId: id, expectedProviderRevision, expectedBindingRevision: target?.revision ?? null }), next => {
      const current = states()[agent]?.selection;
      if (memorySelectionSettled(current, next.selection) && (sourceDrafts()[agent] === value || !sourceDrafts()[agent])) setSourceDrafts(old => { const copy = { ...old }; delete copy[agent]; return copy; });
      if (props.agentId === agent && id) setExpanded(id);
    });
  }
  function saveSync(value: MemoryBindingView) {
    const draft = sync(value), id = props.agentId;
    void mutate(`sync/${value.id}`, () => api.setMemorySync({ agentId: id, bindingId: value.id, expectedBindingRevision: draft.revision, completedTurnSync: draft.value }), next => {
      const own = next.bindings.find(item => item.id === value.id);
      const latest = states()[id]?.bindings.find(item => item.id === value.id);
      if (syncDrafts()[value.id] === draft && memoryPolicySettled(draft.revision, draft.value, own, latest)) setSyncDrafts(old => { const copy = { ...old }; delete copy[value.id]; return copy; });
    });
  }
  function openBinding(id: string) { setExpanded(expanded() === id ? '' : id); setRecordsOpen(false); setDetailId(''); setDetail(undefined); }

  async function loadPage(cursor?: string, destination = 0) {
    const id = props.agentId, target = expanded(), epoch = ++pageEpoch;
    if (!target || !recordsOpen() || !visible()) return;
    setPageLoading(true); setError(id, 'records', '');
    try {
      const next = await api.externalMemoryPage({ agentId: id, bindingId: target, cursor, limit: 25 });
      if (disposed || epoch !== pageEpoch || id !== props.agentId || target !== expanded() || !visible()) return;
      setPage(next); setPageIndex(destination); setPageCursors(old => [...old.slice(0, destination), cursor]);
    } catch (cause) { if (!disposed && epoch === pageEpoch) setError(id, 'records', message(cause)); }
    finally { if (!disposed && epoch === pageEpoch) setPageLoading(false); }
  }
  createEffect(on(ledgerKey, () => {
    pageEpoch++; detailEpoch++; setPage(undefined); setPageIndex(0); setPageCursors([undefined]); setPageLoading(false); setDetailId(''); setDetail(undefined); setDetailLoading(false);
    if (visible() && recordsOpen() && activeBinding()) void loadPage();
  }));
  async function openDetail(value: MemoryEvidenceView) {
    if (detailId() === value.id) { detailEpoch++; setDetailId(''); setDetail(undefined); setDetailLoading(false); return; }
    const id = props.agentId, target = expanded(), epoch = ++detailEpoch;
    setDetailId(value.id); setDetail(undefined); setDetailLoading(true); setError(id, 'detail', '');
    try {
      const next = await api.externalMemoryEntry({ agentId: id, bindingId: target, evidenceId: value.id });
      if (disposed || epoch !== detailEpoch || id !== props.agentId || target !== expanded() || !visible()) return;
      if (next) setDetail(next); else setError(id, 'detail', '记录已不可用');
    } catch (cause) { if (!disposed && epoch === detailEpoch) setError(id, 'detail', message(cause)); }
    finally { if (!disposed && epoch === detailEpoch) setDetailLoading(false); }
  }
  async function addEvidence(value: MemoryBindingView) {
    const draft = textDrafts()[value.id], id = props.agentId, key = `text/${value.id}`;
    if (!draft || pending() || !canWrite(value)) return;
    setPending(key); setError(id, key, '');
    try {
      const receipt = await api.queueExternalMemory({ agentId: id, bindingId: value.id, expectedBindingRevision: value.revision, requestId: draft.requestId, text: manualMemoryText(draft.text) });
      if (disposed) return;
      if (textDrafts()[value.id] === draft) setTextDrafts(old => ({ ...old, [value.id]: { ...draft, text: ['queued', 'accepted', 'committed'].includes(receipt.state) ? '' : draft.text, receipt } }));
      if (id === props.agentId && visible()) { refreshState(); void loadPage(); }
    } catch (cause) { if (!disposed) setError(id, key, message(cause)); }
    finally { if (!disposed) setPending(''); }
  }
  function forget(value: MemoryEvidenceView) {
    const id = props.agentId;
    void mutate(`forget/${value.id}`, () => api.forgetExternalMemory({ agentId: id, bindingId: value.bindingId, evidenceId: value.id, expectedRevision: value.revision }), () => { if (id === props.agentId && visible()) void loadPage(); });
  }
  function sourceName(value: MemoryEvidenceView) { return props.scenes.find(scene => scene.id === value.source.sceneId)?.title ?? t(originLabels[value.source.origin]); }
  function reloadSource() { setSourceDrafts(old => { const next = { ...old }; delete next[props.agentId]; return next; }); setError(props.agentId, 'source', ''); refreshState(); }
  const disabled = () => Boolean(pending() || foreground());
  return <section class="memory-provider" aria-label={t("记忆来源")}>
    <div class="memory-provider-source"><label class="field-label">{t("记忆来源")}<select aria-label={t("记忆来源")} disabled={!props.agent || !api.memoryProviderNative || disabled()} value={source().id ?? ''} onChange={event => setSourceDrafts(old => ({ ...old, [props.agentId]: { id: event.currentTarget.value || null, revision: selection().revision } }))}>
      <option value="">{t("本地")}</option>
      <Show when={selection().bindingId && !binding(selection().bindingId!)}><option value={selection().bindingId!}>Hindsight</option></Show>
      <For each={bindings().map(value => value.id)}>{id => <option value={id} disabled={!selectable(binding(id)!)}>Hindsight · {binding(id)?.endpoint}</option>}</For>
    </select></label>
      <button class="secondary-button" disabled={disabled() || !api.memoryProviderNative || !props.agent || source().revision !== selection().revision || (source().id !== null && !selectable(binding(source().id!)!)) || (source().id === selection().bindingId && (!source().id || authorized(binding(source().id!))))} onClick={() => useSource()}>{t("使用")}</button>
    </div>
    <Show when={source().revision !== selection().revision || error('source')}><div class="memory-provider-conflict"><span role="alert">{error('source') || t('配置已更改，草稿已保留')}</span><button class="text-button" disabled={disabled()} onClick={reloadSource}>{t("载入最新")}</button></div></Show>
    <div class="memory-provider-heading"><Show when={props.agent && availablePlugin()}><button class="text-button" disabled={disabled()} onClick={toggleConfig}><Plus size={13} />{t("连接 Hindsight")}</button></Show><Show when={loading()}><LoaderCircle size={13} class="spin" aria-label={t("加载记忆来源")} /></Show></div>
    <Show when={error('load')}><div class="memory-provider-conflict"><span role="alert">{error('load')}</span><button class="text-button" onClick={refreshState}>{t("重试")}</button></div></Show>
    <Show when={configOpen()[props.agentId] && config()}>{value => <div class="memory-provider-config">
      <label class="field-label">{t("服务地址")}<input aria-label={t("记忆服务地址")} type="url" value={value().endpoint} placeholder="https://…" spellcheck={false} onInput={event => editConfig({ endpoint: event.currentTarget.value })} /></label>
      <label class="field-label">{t("独立密钥")}<input aria-label={t("记忆服务独立密钥")} type="password" value={value().key} autocomplete="off" spellcheck={false} onInput={event => editConfig({ key: event.currentTarget.value })} /></label>
      <div class="memory-provider-config-actions"><button class="secondary-button" disabled={disabled() || !availablePlugin()} onClick={() => void configure('probe')}><Show when={foreground()?.kind === 'probe'}><LoaderCircle size={13} class="spin" /></Show>{t("验证连接")}</button>
        <Show when={verified()?.draft === value()}><span class="saved-label"><Check size={13} />{t("服务可用")}</span></Show>
        <Show when={foreground()}><button class="text-button" onClick={() => { cancelForeground(); refreshState(); }}>{t("取消等待")}</button></Show>
        <button class="primary-button" disabled={disabled() || !availablePlugin() || verified()?.draft !== value() || value().expectedRevision !== selection().revision} onClick={() => void configure('create')}><Show when={foreground()?.kind === 'create'}><LoaderCircle size={13} class="spin" /></Show>{t("创建记忆库")}</button>
      </div>
      <Show when={value().expectedRevision !== selection().revision}><div class="memory-provider-conflict"><span>{t("配置已更改，草稿已保留")}</span><button class="text-button" disabled={disabled()} onClick={() => editConfig({ expectedRevision: selection().revision })}>{t("载入最新")}</button></div></Show>
      <Show when={error('config')}><p class="settings-error" role="alert">{error('config')}</p></Show>
    </div>}</Show>
    <div class="memory-provider-bindings"><For each={bindings().map(value => value.id)}>{id => <Show when={binding(id)}>{value => <article class="memory-binding">
      <button class="memory-binding-heading" aria-expanded={expanded() === id} onClick={() => openBinding(id)}><ChevronRight size={14} classList={{ expanded: expanded() === id }} /><span><strong>Hindsight</strong><small>{value().endpoint}</small></span><span class="memory-binding-status">{!availablePlugin(value()) ? t('插件不可用') : t(memoryBindingLabels[value().status])}<Show when={selection().bindingId === id}>{t("· 当前")}</Show></span></button>
      <Show when={expanded() === id}><div class="memory-binding-body">
        <div class="memory-sync"><label><input type="checkbox" aria-label={t("同步之后完成的对话")} checked={sync(value()).value} disabled={disabled() || value().status === 'retired' || (!availablePlugin(value()) && !value().policy.completedTurnSync)} onChange={event => setSyncDrafts(old => ({ ...old, [id]: { value: event.currentTarget.checked, revision: old[id]?.revision ?? value().revision } }))} /><span>{t("同步之后完成的对话")}</span></label><button class="text-button" disabled={disabled() || !syncDrafts()[id] || sync(value()).revision !== value().revision} onClick={() => saveSync(value())}>{t("保存")}</button></div>
        <p class="memory-provider-note">{t("仅之后完成的对话文字，不补传历史。")}</p>
        <Show when={sync(value()).revision !== value().revision || error(`sync/${id}`)}><div class="memory-provider-conflict"><span role="alert">{error(`sync/${id}`) || t('配置已更改，草稿已保留')}</span><button class="text-button" disabled={disabled()} onClick={() => { setSyncDrafts(old => { const next = { ...old }; delete next[id]; return next; }); setError(props.agentId, `sync/${id}`, ''); }}>{t("载入最新")}</button></div></Show>
        <div class="memory-binding-actions"><Show when={selectable(value()) && (selection().bindingId !== id || !authorized(value()))}><button class="secondary-button" disabled={disabled()} onClick={() => { setSourceDrafts(old => ({ ...old, [props.agentId]: { id, revision: selection().revision } })); useSource(id); }}>{t("使用此记忆库")}</button></Show><Show when={value().status !== 'retired'}><button class="text-button" disabled={disabled()} onClick={() => void mutate(`retire/${id}`, () => api.retireMemoryBinding({ agentId: props.agentId, bindingId: id, expectedBindingRevision: value().revision }))}>{t("停用连接")}</button></Show></div>
        <Show when={error(`retire/${id}`)}><p class="settings-error" role="alert">{error(`retire/${id}`)}</p></Show>
        <header class="memory-heading"><button class="memory-expand text-button" aria-expanded={recordsOpen()} onClick={() => setRecordsOpen(!recordsOpen())}><ChevronRight size={13} classList={{ expanded: recordsOpen() }} />{t("同步记录")}</button><span>{value().evidenceCount}{t("条")}<Show when={value().pendingCount}> · {value().pendingCount}{t("待处理")}</Show><Show when={value().deletionPendingCount}> · {value().deletionPendingCount}{t("待删除")}</Show></span></header>
        <Show when={recordsOpen()}>
          <div class="memory-provider-record-actions"><button class="text-button" disabled={pageLoading()} onClick={() => void loadPage()}><RefreshCw size={12} />{t("刷新")}</button><button class="secondary-button" disabled={!canWrite(value()) || disabled()} onClick={() => setTextDrafts(old => ({ ...old, [id]: old[id] ?? { text: '', requestId: crypto.randomUUID() } }))}><Plus size={12} />{t("添加")}</button></div>
          <Show when={textDraft()}>{draft => <div class="memory-editor"><label class="field-label">{t("添加记录")}<textarea aria-label={t("添加外接记忆记录")} rows={3} value={draft().text} onInput={event => { const text = event.currentTarget.value; setTextDrafts(old => ({ ...old, [id]: { text, requestId: crypto.randomUUID() } })); }} /></label><div class="memory-editor-actions"><Show when={draft().receipt}><span role="status">{t(memoryWriteLabel(draft().receipt!))}<Show when={draft().receipt!.reason}> · {t(memoryReasonLabel(draft().receipt!.reason))}</Show></span></Show><button class="text-button" disabled={disabled()} onClick={() => setTextDrafts(old => { const next = { ...old }; delete next[id]; return next; })}>{t("关闭")}</button><button class="primary-button" disabled={disabled() || !canWrite(value()) || !draft().text.trim()} onClick={() => void addEvidence(value())}>{t("保存")}</button></div><Show when={error(`text/${id}`)}><p class="settings-error" role="alert">{error(`text/${id}`)}</p></Show></div>}</Show>
          <Show when={error('records')}><div class="memory-provider-conflict"><span role="alert">{error('records')}</span><button class="text-button" onClick={() => void loadPage()}>{t("重新查看")}</button></div></Show>
          <div class="memory-list" aria-busy={pageLoading()}><For each={page()?.entries.map(entry => entry.id) ?? []}>{entryId => <Show when={page()?.entries.find(entry => entry.id === entryId)}>{entry => <article class="memory-entry">
            <div class="memory-entry-row"><p>{entry().preview}</p><button class="icon-button compact" title={t("忘记")} aria-label={t("忘记记录")} disabled={disabled() || Boolean(entry().forgotten?.localSuppressed)} onClick={() => forget(entry())}><Trash2 size={13} /></button></div>
            <div class="memory-record-meta"><span role="status">{t(memoryEvidenceLabel(entry()))}</span><Show when={entry().reason}><span>{t(memoryReasonLabel(entry().reason))}</span></Show><button class="text-button" aria-expanded={detailId() === entryId} onClick={() => void openDetail(entry())}>{t("查看原文")}</button></div>
            <Show when={error(`forget/${entryId}`)}><p class="settings-error" role="alert">{error(`forget/${entryId}`)}</p></Show>
            <Show when={detailId() === entryId}><div class="memory-evidence-detail"><span>{sourceName(entry())} · {new Date(entry().createdAt).toLocaleString()}</span><Show when={detailLoading()}><LoaderCircle size={13} class="spin" /></Show><Show when={detail()?.summary.id === entryId}><Show when={detail()?.attributedText !== null} fallback={<p>{t("不再使用")}</p>}><pre>{detail()?.attributedText}</pre></Show></Show><Show when={error('detail')}><p class="settings-error" role="alert">{error('detail')}</p></Show></div></Show>
          </article>}</Show>}</For></div>
          <footer class="memory-pagination"><span><Show when={pageLoading()} fallback={page() ? t("{0} 条 · 第 {1} 页").replaceAll("{0}", () => String(page()!.total)).replaceAll("{1}", () => String(pageIndex() + 1)) : ''}><LoaderCircle size={13} class="spin" /></Show></span><button class="icon-button compact" title={t("上一页记录")} aria-label={t("上一页记录")} disabled={pageLoading() || !pageIndex()} onClick={() => void loadPage(pageCursors()[pageIndex() - 1], pageIndex() - 1)}><ChevronLeft size={15} /></button><button class="icon-button compact" title={t("下一页记录")} aria-label={t("下一页记录")} disabled={pageLoading() || !page()?.nextCursor} onClick={() => void loadPage(page()!.nextCursor!, pageIndex() + 1)}><ChevronRight size={15} /></button></footer>
        </Show>
      </div></Show>
    </article>}</Show>}</For></div>
    <Show when={bindingPage() > 0 || state()?.nextCursor}><footer class="memory-pagination"><span>{t("记忆连接")}</span><button class="icon-button compact" title={t("上一页连接")} aria-label={t("上一页记忆连接")} disabled={loading() || !bindingPage()} onClick={() => void loadState(bindingCursors()[bindingPage() - 1], bindingPage() - 1)}><ChevronLeft size={15} /></button><button class="icon-button compact" title={t("下一页连接")} aria-label={t("下一页记忆连接")} disabled={loading() || !state()?.nextCursor} onClick={() => void loadState(state()!.nextCursor!, bindingPage() + 1)}><ChevronRight size={15} /></button></footer></Show>
  </section>;
}
