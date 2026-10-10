// SPDX-License-Identifier: MPL-2.0
import { createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, LoaderCircle } from 'lucide-solid';
import type { AgentProfile, ConnectionProfile, MemoryStats, Scene } from '../contracts';
import { t } from '../i18n';
import MemoryList, { type DeleteMemory, type SaveMemory } from './MemoryList';
import MemoryProviderSettings from './MemoryProviderSettings';
import { Row, SettingsAddButton, Toggle } from './SettingControls';
import '../connections.css';

interface Props {
  agents: AgentProfile[];
  connections: ConnectionProfile[];
  scenes: Scene[];
  memoryStats: MemoryStats[];
  active: boolean;
  onSelect: (id: string) => void;
  onSave: (agent: AgentProfile) => Promise<void>;
  onSaveMemory: (value: SaveMemory) => Promise<void>;
  onDeleteMemory: (value: DeleteMemory) => Promise<void>;
}

export default function AgentsPanel(props: Props) {
  const [drafts, setDrafts] = createSignal<Record<string, AgentProfile>>({});
  const [expanded, setExpanded] = createSignal('');
  const [opened, setOpened] = createSignal<Record<string, boolean>>({});
  const [pending, setPending] = createSignal('');
  const [saved, setSaved] = createSignal('');
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const profile = (id: string) => props.agents.find(agent => agent.id === id);
  const view = (id: string) => drafts()[id] ?? profile(id)!;
  const ids = createMemo(() => [...props.agents.map(agent => agent.id), ...Object.keys(drafts()).filter(id => !profile(id))]);
  const soul = (id: string) => view(id).instructions.trim().replace(/\s+/g, ' ');
  function clearError(id: string) {
    setErrors(old => { const next = { ...old }; delete next[id]; return next; });
  }
  function open(id: string) {
    setOpened(old => ({ ...old, [id]: true }));
    setExpanded(expanded() === id ? '' : id);
    props.onSelect(id);
  }
  function edit(id: string, patch: Partial<AgentProfile>) {
    setDrafts(old => ({ ...old, [id]: { ...view(id), ...patch } }));
    clearError(id); setSaved('');
  }
  function add() {
    const id = crypto.randomUUID();
    setDrafts(old => ({ ...old, [id]: { id, name: '', instructions: '', memory: '', memoryEnabled: true, defaultConnectionId: null } }));
    open(id);
    queueMicrotask(() => { if (!disposed) document.getElementById(`agent-name-${id}`)?.focus(); });
  }
  async function save(id: string) {
    if (pending()) return;
    const value = view(id);
    if (!value.name.trim()) { setErrors(old => ({ ...old, [id]: t('请输入名称') })); return; }
    setPending(id); clearError(id); setSaved('');
    try {
      await props.onSave({ ...value, name: value.name.trim() });
      if (disposed) return;
      // The saved row may be collapsed, or the user may already be editing it again.
      if (!drafts()[id] || drafts()[id] === value) {
        setDrafts(old => { const next = { ...old }; delete next[id]; return next; });
        setSaved(id);
      }
    } catch (cause) {
      if (!disposed) setErrors(old => ({ ...old, [id]: cause instanceof Error ? cause.message : String(cause) }));
    } finally { if (!disposed) setPending(''); }
  }
  return <section class="agents-panel" aria-label="Agent">
    <div class="settings-section-heading"><span>{t('我的 Agent')}</span><SettingsAddButton onClick={add} /></div>
    <For each={ids()}>{id => <article class="connection-card agent-card" classList={{ expanded: expanded() === id }}>
      <div class="connection-summary"><button class="connection-expand" aria-expanded={expanded() === id} aria-controls={`agent-editor-${id}`} onClick={() => open(id)}><Show when={expanded() === id} fallback={<ChevronRight size={15} />}><ChevronDown size={15} /></Show><span><strong>{view(id).name || t('新 Agent')}</strong><Show when={soul(id)}>{summary => <small title={summary()}>{summary()}</small>}</Show></span></button><Show when={drafts()[id]}><i class="connection-dirty" title={t('未保存')} aria-label={t('未保存')} /></Show></div>
      <Show when={opened()[id]}><div id={`agent-editor-${id}`} class="connection-editor agent-editor" hidden={expanded() !== id}>
        <label class="field-label">{t('名称')}<input id={`agent-name-${id}`} value={view(id).name} onInput={event => edit(id, { name: event.currentTarget.value })} maxlength={80} /></label>
        <label class="field-label">{t('灵魂设定')}<textarea rows={5} value={view(id).instructions} onInput={event => edit(id, { instructions: event.currentTarget.value })} /></label>
        <label class="field-label">{t('模型')}<select aria-label={t('Agent 模型')} value={view(id).defaultConnectionId ?? ''} onChange={event => edit(id, { defaultConnectionId: event.currentTarget.value || null })}><option value="">{t('使用默认模型')}</option><For each={props.connections.map(connection => connection.id)}>{connectionId => <option value={connectionId}>{(() => { const connection = props.connections.find(connection => connection.id === connectionId); return connection?.model ? `${connection.model} · ${connection.name}` : connection?.name; })()}</option>}</For></select></label>
        <Row title={t('长期记忆')}><Toggle label={t('长期记忆')} checked={view(id).memoryEnabled !== false} onChange={value => edit(id, { memoryEnabled: value })} /></Row>
        <Show when={errors()[id]}><p class="settings-error" role="alert">{errors()[id]}</p></Show>
        <div class="settings-actions"><Show when={saved() === id}><span class="saved-label"><Check size={13} />{t('已保存')}</span></Show><button class="primary-button" disabled={Boolean(pending())} onClick={() => void save(id)}><Show when={pending() === id}><LoaderCircle size={14} class="spin" /></Show>{t('保存')}</button></div>
        <MemoryProviderSettings agentId={id} agent={profile(id)} active={props.active && expanded() === id} scenes={props.scenes} />
        <MemoryList agentId={id} stats={props.memoryStats.find(stats => stats.agentId === id)} scenes={props.scenes} savedAgent={Boolean(profile(id))} active={props.active && expanded() === id} onSave={props.onSaveMemory} onDelete={props.onDeleteMemory} />
      </div></Show>
      <Show when={errors()[id] && expanded() !== id}><p class="settings-error connection-collapsed-error" role="alert">{errors()[id]}</p></Show>
    </article>}</For>
  </section>;
}
