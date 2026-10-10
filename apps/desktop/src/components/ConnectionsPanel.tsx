// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { nativeSelectOwnsEscape } from '../native-select-escape';
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, CircleAlert, LoaderCircle, MoreHorizontal, Plus, RefreshCw, Search } from 'lucide-solid';
import type { ConnectionAdvanced, ConnectionProfile, ConnectionTestResult, ProviderPreset } from '../contracts';
import { cancelConnectionProbe, discoverConnectionModels, getProviderPresets, testConnection } from '../bridge';
import { normalizeConnectionPath, parseConnectionParameters, validateConnectionProfile } from '../connection-policy';
import { subscribePlugins } from '../plugin-bridge';
import { profileFromProviderPreset } from '../provider-presets';
import { connectionDraft as fromProfile, settleConnectionSave, type ConnectionDraft as Draft } from './connection-draft';
import { SettingsAddButton } from './SettingControls';
import '../connections.css';

interface Props {
  connections: ConnectionProfile[]; defaultConnectionId: string | null; active: boolean;
  onSave: (profile: ConnectionProfile, expectedRevision?: number, apiKey?: string) => Promise<void>;
  onDelete: (id: string, expectedRevision: number) => Promise<void>;
  onDefault: (id: string | null) => Promise<void>;
}
interface Probe { id: string; requestId: string; kind: 'models' | 'test'; draft: Draft }
const message = (cause: unknown) => cause instanceof Error ? cause.message : String(cause);
const groupLabels = { china: '国内', global: '国际', custom: '自定义' };

export default function ConnectionsPanel(props: Props) {
  const [drafts, setDrafts] = createSignal<Record<string, Draft>>({});
  const [selected, setSelected] = createSignal('');
  const [presets, setPresets] = createSignal<ProviderPreset[]>([]);
  const [presetLoading, setPresetLoading] = createSignal(false), [presetError, setPresetError] = createSignal('');
  const [adding, setAdding] = createSignal(false), [query, setQuery] = createSignal('');
  const [menu, setMenu] = createSignal(''), [deleting, setDeleting] = createSignal('');
  const [pending, setPending] = createSignal(''), [probe, setProbe] = createSignal<Probe>();
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [models, setModels] = createSignal<Record<string, string[]>>({});
  const [modelStatus, setModelStatus] = createSignal<Record<string, string>>({});
  const [testResults, setTestResults] = createSignal<Record<string, ConnectionTestResult>>({});
  const [saved, setSaved] = createSignal('');
  let disposed = false, initialized = false, presetEpoch = 0, presetsDirty = true;
  let stopPlugins: (() => void) | undefined;
  const active = createMemo(() => props.active);
  const savedProfile = (id: string) => props.connections.find(value => value.id === id);
  const form = (id: string) => drafts()[id];
  const ids = createMemo(() => [...props.connections.map(value => value.id), ...Object.entries(drafts()).filter(([id, value]) => !savedProfile(id) && (value.isNew || value.dirty)).map(([id]) => id)].sort((a, b) => Number(b === props.defaultConnectionId) - Number(a === props.defaultConnectionId)));
  const view = (id: string) => drafts()[id]?.profile ?? savedProfile(id)!;
  const providerName = (id: string) => presets().find(value => value.id === id)?.name ?? (id.startsWith('plugin.') ? t('模型接入') : id);
  const stale = (id: string) => { const value = form(id); return value && !value.isNew && savedProfile(id)?.revision !== value.expectedRevision; };
  const matchingPresets = () => {
    const text = query().trim().toLocaleLowerCase();
    return presets().filter(value => !text || [value.name, value.id, value.searchTerms ?? '', value.baseUrl].join(' ').toLocaleLowerCase().includes(text));
  };
  function clearError(id: string) { setErrors(old => { const next = { ...old }; delete next[id]; return next; }); }
  function cancelProbe() { const value = probe(); if (!value) return; setProbe(undefined); void cancelConnectionProbe(value.requestId).catch(() => undefined); }
  function invalidate(id: string, clearModels = true) {
    if (probe()?.id === id) cancelProbe();
    setSaved(''); clearError(id);
    setTestResults(old => { const next = { ...old }; delete next[id]; return next; });
    setModelStatus(old => { const next = { ...old }; delete next[id]; return next; });
    if (clearModels) setModels(old => { const next = { ...old }; delete next[id]; return next; });
  }
  function ensure(id: string) { if (!form(id) && savedProfile(id)) setDrafts(old => ({ ...old, [id]: fromProfile(savedProfile(id)!) })); }
  function open(id: string) { cancelProbe(); ensure(id); setSelected(selected() === id ? '' : id); setMenu(''); setDeleting(''); setAdding(false); }
  function edit(id: string, patch: Partial<Draft>, keepModels = false) {
    ensure(id); const current = form(id); if (!current) return;
    invalidate(id, !keepModels); setDrafts(old => ({ ...old, [id]: { ...current, ...patch, dirty: true } }));
  }
  function field(id: string, patch: Partial<ConnectionProfile>, keepModels = false) { edit(id, { profile: { ...form(id).profile, ...patch } }, keepModels); }
  function advanced(id: string, patch: Partial<ConnectionAdvanced>) { field(id, { advanced: { ...form(id).profile.advanced, ...patch } }); }
  async function loadPresets() {
    if (presetLoading()) return; setPresetLoading(true); setPresetError('');
    const epoch = presetEpoch; presetsDirty = false;
    try { const values = await getProviderPresets(); if (!disposed && epoch === presetEpoch) { setPresets(values); initialized = true; } }
    catch (error) { if (!disposed) setPresetError(message(error)); }
    finally { if (!disposed) { setPresetLoading(false); if (presetsDirty && props.active) void loadPresets(); } }
  }
  void subscribePlugins(() => {
    if (disposed) return; presetEpoch++; presetsDirty = true; setPresets([]);
    if (props.active) void loadPresets();
  }).then(stop => { if (disposed) stop(); else { stopPlugins = stop; presetEpoch++; presetsDirty = true; if (props.active) void loadPresets(); } })
    .catch(error => { if (!disposed) setPresetError(message(error)); });
  createEffect(on(active, value => { if (!value) { cancelProbe(); setMenu(''); } else if (!initialized || presetsDirty) void loadPresets(); }));
  const revisions = createMemo(() => props.connections.map(value => `${value.id}:${value.revision}`).join('|'));
  createEffect(on(revisions, () => {
    setDrafts(old => Object.fromEntries(Object.entries(old).map(([id, value]) => [id, !value.dirty && !value.isNew && savedProfile(id) && savedProfile(id)!.revision !== value.expectedRevision ? fromProfile(savedProfile(id)!) : value])));
    const current = probe(); if (current && !current.draft.isNew && savedProfile(current.id)?.revision !== current.draft.expectedRevision) cancelProbe();
  }));
  const outsideMenu = (event: PointerEvent) => { if (!(event.target instanceof Element) || !event.target.closest('.connection-menu,.connection-summary .icon-button')) setMenu(''); };
  document.addEventListener('pointerdown', outsideMenu);
  onCleanup(() => { disposed = true; stopPlugins?.(); cancelProbe(); setDrafts({}); document.removeEventListener('pointerdown', outsideMenu); });
  function add(preset: ProviderPreset) {
    if (!presets().includes(preset) || presetLoading()) return;
    cancelProbe(); const id = crypto.randomUUID(); let name = preset.name, suffix = 2;
    while (ids().some(id => view(id).name === name)) name = `${preset.name} ${suffix++}`;
    const profile = profileFromProviderPreset(preset, id, name);
    setDrafts(old => ({ ...old, [id]: { ...fromProfile(profile), expectedRevision: undefined, isNew: true, dirty: true } }));
    setSelected(id); setAdding(false); setMenu(''); setQuery('');
  }
  function reload(id: string) { const profile = savedProfile(id); if (!profile) return; cancelProbe(); invalidate(id); setDrafts(old => ({ ...old, [id]: fromProfile(profile) })); }
  function prepared(value: Draft, requireModel = true) {
    const profile = { ...value.profile, name: value.profile.name.trim(), baseUrl: value.profile.baseUrl.trim(), model: value.profile.model.trim(), advanced: { ...value.profile.advanced, requestPath: normalizeConnectionPath(value.profile.advanced.requestPath), requestParameters: parseConnectionParameters(value.parametersText) } };
    validateConnectionProfile(profile, requireModel); return profile;
  }
  const keyArgument = (value: Draft) => value.clearKey ? '' : value.key || undefined;
  async function save(id: string) {
    const value = form(id); if (!value || pending()) return;
    cancelProbe(); setPending(id); setSaved(''); clearError(id);
    try {
      await props.onSave(prepared(value), value.expectedRevision, keyArgument(value));
      if (disposed) return;
      const current = form(id); if (!current) return;
      const settled = settleConnectionSave(value, current, savedProfile(id));
      setDrafts(old => old[id] === current ? { ...old, [id]: settled } : old);
      if (!settled.dirty) setSaved(id);
    } catch (error) { if (!disposed) setErrors(old => ({ ...old, [id]: message(error) })); }
    finally { if (!disposed) setPending(''); }
  }
  async function runProbe(id: string, kind: Probe['kind']) {
    const value = form(id); if (!value || pending()) return; cancelProbe(); clearError(id);
    let profile: ConnectionProfile;
    try { profile = prepared(value, kind === 'test'); } catch (error) { setErrors(old => ({ ...old, [id]: message(error) })); return; }
    const operation: Probe = { id, draft: value, kind, requestId: crypto.randomUUID() }; setProbe(operation);
    const args = { requestId: operation.requestId, profile, expectedRevision: value.expectedRevision, apiKey: keyArgument(value) };
    const current = () => !disposed && probe() === operation && selected() === id && form(id) === value && props.active;
    try {
      if (kind === 'models') { const result = await discoverConnectionModels(args); if (current()) { setModels(old => ({ ...old, [id]: result.models })); setModelStatus(old => ({ ...old, [id]: result.models.length ? t("{0} 个模型").replaceAll("{0}", () => String(result.models.length)) : '未返回模型，可手动输入' })); } }
      else { const result = await testConnection(args); if (current()) setTestResults(old => ({ ...old, [id]: result })); }
    } catch (error) { if (current()) setErrors(old => ({ ...old, [id]: message(error) })); }
    finally { if (!disposed && probe() === operation) setProbe(undefined); }
  }
  async function setDefault(id: string) {
    if (pending() || !savedProfile(id)) return; setPending(id); clearError(id); setMenu('');
    try { await props.onDefault(id); } catch (error) { if (!disposed) setErrors(old => ({ ...old, [id]: message(error) })); }
    finally { if (!disposed) setPending(''); }
  }
  async function remove(id: string) {
    if (pending()) return; cancelProbe(); const value = savedProfile(id);
    if (value) {
      setPending(id); clearError(id);
      try { await props.onDelete(id, value.revision); } catch (error) { if (!disposed) setErrors(old => ({ ...old, [id]: message(error) })); return; }
      finally { if (!disposed) setPending(''); }
    }
    if (disposed) return;
    setDrafts(old => { const next = { ...old }; delete next[id]; return next; }); setDeleting(''); setMenu(''); if (selected() === id) setSelected('');
  }
  function keyboard(event: KeyboardEvent) {
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key !== 'Escape') return;
    if (menu()) setMenu(''); else if (deleting()) setDeleting(''); else if (adding()) setAdding(false); else if (probe()) cancelProbe(); else return;
    event.preventDefault(); event.stopPropagation();
  }
  return <section class="connections-panel" aria-label={t("API 连接")} onKeyDown={keyboard}>
    <div class="settings-section-heading"><span>{t("我的连接")}</span><SettingsAddButton onClick={() => { setAdding(!adding()); setMenu(''); cancelProbe(); }} /></div>
    <Show when={adding()}><div class="connection-presets"><label class="connections-search"><Search size={14} /><input type="search" aria-label={t("搜索服务商")} placeholder={t("搜索服务商")} value={query()} onInput={event => setQuery(event.currentTarget.value)} /></label><Show when={presetLoading()}><p class="connection-note"><LoaderCircle size={13} class="spin" />{t("加载中")}</p></Show><Show when={presetError()}><p class="settings-error" role="alert">{presetError()}<button class="text-button" onClick={() => void loadPresets()}>{t("重试")}</button></p></Show><For each={['china', 'global', 'custom'] as const}>{group => <Show when={matchingPresets().some(value => value.group === group)}><h4>{t(groupLabels[group])}</h4><div class="connection-preset-grid"><For each={matchingPresets().filter(value => value.group === group)}>{preset => <button onClick={() => add(preset)}>{preset.name}</button>}</For></div></Show>}</For></div></Show>
    <For each={ids()}>{id => <article class="connection-card" classList={{ expanded: selected() === id }}>
      <div class="connection-summary"><button class="connection-expand" aria-expanded={selected() === id} onClick={() => open(id)}><Show when={selected() === id} fallback={<ChevronRight size={15} />}><ChevronDown size={15} /></Show><span><strong>{view(id).name || t('新连接')}</strong><small>{providerName(view(id).providerId)}<Show when={view(id).model}> · {view(id).model}</Show></small></span></button><Show when={props.defaultConnectionId === id}><span class="connection-default">{t("默认")}</span></Show><Show when={form(id)?.dirty}><i class="connection-dirty" title={t("未保存")} aria-label={t("未保存")} /></Show><button class="icon-button compact" aria-label={t("连接操作 {0}").replaceAll("{0}", () => String(view(id).name))} title={t("连接操作")} onClick={() => setMenu(menu() === id ? '' : id)}><MoreHorizontal size={17} /></button>
        <Show when={menu() === id}><div class="connection-menu"><button disabled={Boolean(pending()) || !savedProfile(id) || props.defaultConnectionId === id} onClick={() => void setDefault(id)}>{t("设为默认")}</button><button onClick={() => { ensure(id); setSelected(id); setMenu(''); queueMicrotask(() => document.getElementById(`connection-name-${id}`)?.focus()); }}>{t("重命名")}</button><button disabled={Boolean(pending())} onClick={() => { setDeleting(id); setMenu(''); }}>{form(id)?.isNew ? t('移除草稿') : t('删除连接')}</button></div></Show>
      </div>
      <Show when={deleting() === id}><div class="connection-remove"><span>{t("删除")}{view(id).name}？</span><button class="text-button" disabled={Boolean(pending())} onClick={() => setDeleting('')}>{t("取消")}</button><button class="secondary-button" disabled={Boolean(pending())} onClick={() => void remove(id)}>{t("删除")}</button></div></Show>
      <Show when={selected() === id && form(id)}>{value => <div class="connection-editor">
        <Show when={stale(id)}><div class="connection-conflict"><CircleAlert size={14} /><span>{savedProfile(id) ? t('连接已更改，草稿已保留') : t('连接已被删除，草稿已保留')}</span><Show when={savedProfile(id)}><button class="text-button" onClick={() => reload(id)}>{t("载入最新")}</button></Show></div></Show>
        <label class="field-label">{t("连接名称")}<input id={`connection-name-${id}`} value={value().profile.name} maxlength={80} onInput={event => field(id, { name: event.currentTarget.value }, true)} /></label>
        <Show when={value().profile.providerId.toLowerCase() === 'custom'}><label class="field-label">{t("API 地址")}<input type="url" aria-label={t("API 地址")} value={value().profile.baseUrl} placeholder="https://…/v1" spellcheck={false} onInput={event => field(id, { baseUrl: event.currentTarget.value })} /></label></Show>
        <label class="field-label">API Key<input type="password" aria-label="API Key" value={value().key} autocomplete="off" spellcheck={false} disabled={value().clearKey} placeholder={value().clearKey ? t('保存后清除') : value().profile.hasKey ? t('已保存') : ''} onInput={event => edit(id, { key: event.currentTarget.value })} /></label>
        <Show when={value().profile.hasKey}><button class="text-button connection-clear-key" onClick={() => edit(id, { clearKey: !value().clearKey })}>{value().clearKey ? t('撤销清除') : t('清除已保存密钥')}</button></Show>
        <label class="field-label">{t("模型")}<div class="connection-model"><input aria-label={t("模型")} list={`connection-models-${id}`} value={value().profile.model} spellcheck={false} placeholder={t("选择或输入模型 ID")} onInput={event => field(id, { model: event.currentTarget.value }, true)} /><datalist id={`connection-models-${id}`}><For each={models()[id] ?? []}>{model => <option value={model} />}</For></datalist><button class="icon-button" title={t("获取模型")} aria-label={t("获取模型")} disabled={Boolean(pending()) || probe()?.id === id} onClick={() => void runProbe(id, 'models')}><RefreshCw size={17} classList={{ spin: probe()?.id === id && probe()?.kind === 'models' }} /></button></div></label>
        <Show when={modelStatus()[id]}><p class="connection-note">{modelStatus()[id]}</p></Show>
        <details class="settings-details connection-advanced"><summary>{t("高级")}<ChevronRight size={13} /></summary>
          <Show when={value().profile.providerId.toLowerCase() !== 'custom'}><label class="field-label">{t("API 地址")}<input type="url" aria-label={t("API 地址")} value={value().profile.baseUrl} spellcheck={false} onInput={event => field(id, { baseUrl: event.currentTarget.value })} /></label></Show>
          <div class="connection-advanced-row"><label class="field-label">{t("API 格式")}<select aria-label={t("API 格式")} value={value().profile.advanced.protocol} onChange={event => advanced(id, { protocol: event.currentTarget.value as ConnectionAdvanced['protocol'] })}><option value="chat_completions">OpenAI Chat</option><option value="anthropic_messages">Anthropic Messages</option><option value="openai_responses">OpenAI Responses</option></select></label><label class="field-label">{t("认证")}<select aria-label={t("认证方式")} value={value().profile.advanced.authMode} onChange={event => advanced(id, { authMode: event.currentTarget.value as ConnectionAdvanced['authMode'] })}><option value="bearer">Bearer</option><option value="api_key">API Key</option><option value="none">{t("无")}</option></select></label></div>
          <label class="field-label">{t("请求路径")}<input aria-label={t("请求路径")} value={value().profile.advanced.requestPath ?? ''} placeholder={t("可选，如 /v1/messages")} title={t("相对域名根路径；留空按 API 地址自动补全")} spellcheck={false} onInput={event => advanced(id, { requestPath: event.currentTarget.value })} /></label>
          <label class="field-label">{t("请求参数 JSON")}<textarea aria-label={t("请求参数 JSON")} rows={3} maxlength={16384} spellcheck={false} value={value().parametersText} onInput={event => edit(id, { parametersText: event.currentTarget.value })} /></label>
        </details>
        <div class="connection-test"><button class="secondary-button" disabled={Boolean(pending()) || Boolean(probe())} onClick={() => void runProbe(id, 'test')}><Show when={probe()?.id === id && probe()?.kind === 'test'}><LoaderCircle size={13} class="spin" /></Show>{t("测试连接")}</button><Show when={probe()?.id === id} fallback={<Show when={testResults()[id]} fallback={<small>{t("会发送测试请求")}</small>}>{result => <span class="saved-label"><Check size={13} />{t("连接正常 ·")}{Math.round(result().latencyMs)} ms</span>}</Show>}><button class="text-button" onClick={cancelProbe}>{t("取消")}</button></Show></div>
        <Show when={errors()[id]}><p class="settings-error" role="alert">{errors()[id]}</p></Show>
        <div class="settings-actions"><Show when={saved() === id}><span class="saved-label"><Check size={13} />{t("已保存")}</span></Show><button class="primary-button" disabled={Boolean(pending()) || stale(id)} onClick={() => void save(id)}><Show when={pending() === id}><LoaderCircle size={14} class="spin" /></Show>{t("保存")}</button></div>
      </div>}</Show>
      <Show when={selected() !== id && errors()[id]}><p class="settings-error connection-collapsed-error" role="alert">{errors()[id]}</p></Show>
    </article>}</For>
    <Show when={!ids().length && !adding()}><button class="connection-empty" onClick={() => setAdding(true)}><Plus size={17} />{t("添加连接")}</button></Show>
  </section>;
}
