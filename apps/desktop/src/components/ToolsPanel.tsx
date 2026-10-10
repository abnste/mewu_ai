// SPDX-License-Identifier: MPL-2.0
import { t } from '../i18n';
import { createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, LoaderCircle, RefreshCw, Trash2 } from 'lucide-solid';
import type { DiscoverMcpServer, McpServer, SceneCommand, Snapshot } from '../contracts';
import { SettingsAddButton } from './SettingControls';
import '../connections.css';

export type McpSettingsCommand = Extract<SceneCommand, { type: 'set_mcp_grants' | 'set_mcp_enabled' | 'remove_mcp_server' }>;
interface Props {
  snapshot: Snapshot; agentId: string; onAgent: (id: string) => void;
  onDiscover: (value: DiscoverMcpServer) => Promise<void>;
  onCommand: (value: McpSettingsCommand) => Promise<void>;
}
interface ConfigDraft { label: string; executable: string; argsText: string; cwd: string; revision?: number; dirty: boolean }
interface GrantDraft { revision: number; toolNames: string[] }
const absolutePath = (value: string) => /^(?:[a-zA-Z]:[\\/]|\\\\|\/)/.test(value);
const fromServer = (server?: McpServer): ConfigDraft => ({ label: server?.label ?? '', executable: server?.command.executable ?? '', argsText: server?.command.args.join('\n') ?? '', cwd: server?.command.cwd ?? '', revision: server?.revision, dirty: false });
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export default function ToolsPanel(props: Props) {
  const [expanded, setExpanded] = createSignal('');
  const [opened, setOpened] = createSignal<Record<string, boolean>>({});
  const [configs, setConfigs] = createSignal<Record<string, ConfigDraft>>({});
  const [configErrors, setConfigErrors] = createSignal<Record<string, string>>({});
  const [discovering, setDiscovering] = createSignal<string>();
  const [grants, setGrants] = createSignal<Record<string, GrantDraft>>({});
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [pending, setPending] = createSignal<Record<string, boolean>>({});
  const [saved, setSaved] = createSignal<Record<string, number>>({});
  const [removeConfirmation, setRemoveConfirmation] = createSignal<{ id: string; revision: number }>();
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const agentId = createMemo(() => props.snapshot.agents.some(agent => agent.id === props.agentId) ? props.agentId : props.snapshot.agents[0]?.id ?? '');
  const server = (id: string) => props.snapshot.mcpServers.find(value => value.id === id);
  const ids = createMemo(() => [...props.snapshot.mcpServers.map(value => value.id), ...(configs().new ? ['new'] : [])]);
  const label = (id: string) => configs()[id]?.dirty ? configs()[id].label : server(id)?.label ?? configs()[id]?.label;
  const grantKey = (serverId: string, agent = agentId()) => serverId + '/' + agent;
  const selectedNames = (value: McpServer) => grants()[grantKey(value.id)]?.toolNames ?? value.grants.find(grant => grant.agentId === agentId())?.toolNames ?? [];
  const configStale = (id: string) => id !== 'new' && configs()[id]?.revision !== server(id)?.revision;
  const blocked = (id: string) => pending()[id] || discovering() === id;
  function ensureConfig(id: string, reload = false) {
    if (!configs()[id] || reload) setConfigs(old => ({ ...old, [id]: fromServer(server(id)) }));
    if (reload) setConfigErrors(old => ({ ...old, [id]: '' }));
  }
  function open(id: string) {
    ensureConfig(id);
    setOpened(old => ({ ...old, [id]: true }));
    setExpanded(expanded() === id ? '' : id);
  }
  function add() {
    ensureConfig('new'); setOpened(old => ({ ...old, new: true })); setExpanded('new');
    queueMicrotask(() => { if (!disposed) document.getElementById('mcp-name-new')?.focus(); });
  }
  function editConfig(id: string, patch: Partial<ConfigDraft>) {
    setConfigs(old => ({ ...old, [id]: { ...old[id], ...patch, dirty: true } }));
    setConfigErrors(old => ({ ...old, [id]: '' }));
  }
  async function discover(id: string) {
    const value = configs()[id];
    if (!value || discovering() || blocked(id)) return;
    const executable = value.executable.trim(), cwd = value.cwd.trim(), label = value.label.trim();
    const error = !label ? t('请输入名称') : !absolutePath(executable) ? t('程序需填写绝对路径') : cwd && !absolutePath(cwd) ? t('工作目录需填写绝对路径') : '';
    if (error) { setConfigErrors(old => ({ ...old, [id]: error })); return; }
    setDiscovering(id); setConfigErrors(old => ({ ...old, [id]: '' }));
    try {
      await props.onDiscover({ id: id === 'new' ? undefined : id, expectedRevision: value.revision, label, command: { executable, args: value.argsText.split(/\r?\n/).filter(line => line.length > 0), cwd: cwd || null } });
      if (disposed) return;
      if (configs()[id] === value) {
        if (id === 'new') {
          setConfigs(old => { const next = { ...old }; delete next[id]; return next; });
          setOpened(old => { const next = { ...old }; delete next[id]; return next; });
          if (expanded() === id) setExpanded('');
        } else setConfigs(old => ({ ...old, [id]: fromServer(server(id)) }));
      }
    } catch (error) { if (!disposed) setConfigErrors(old => ({ ...old, [id]: message(error) })); }
    finally { if (!disposed) setDiscovering(undefined); }
  }
  function choose(value: McpServer, name: string, checked: boolean) {
    const key = grantKey(value.id), draft = grants()[key] ?? { revision: value.revision, toolNames: [...selectedNames(value)] };
    const toolNames = checked ? [...new Set([...draft.toolNames, name])] : draft.toolNames.filter(item => item !== name);
    setGrants(old => ({ ...old, [key]: { ...draft, toolNames } }));
    setErrors(old => ({ ...old, [key]: '' })); setSaved(old => ({ ...old, [key]: 0 }));
  }
  function reloadGrants(value: McpServer) {
    const key = grantKey(value.id);
    setGrants(old => { const next = { ...old }; delete next[key]; return next; });
    setErrors(old => ({ ...old, [key]: '' })); setSaved(old => ({ ...old, [key]: 0 }));
  }
  async function saveGrants(value: McpServer) {
    const agent = agentId(), key = grantKey(value.id, agent), draft = grants()[key];
    if (!draft || !agent || blocked(value.id)) return;
    const elsewhere = props.snapshot.mcpServers.filter(server => server.id !== value.id).reduce((sum, server) => sum + (server.grants.find(grant => grant.agentId === agent)?.toolNames.length ?? 0), 0);
    if (elsewhere + draft.toolNames.length > 60) { setErrors(old => ({ ...old, [key]: t('每个 Agent 最多启用 60 个工具') })); return; }
    setPending(old => ({ ...old, [value.id]: true })); setErrors(old => ({ ...old, [key]: '' }));
    try {
      await props.onCommand({ type: 'set_mcp_grants', serverId: value.id, expectedRevision: draft.revision, agentId: agent, toolNames: [...draft.toolNames] });
      if (disposed) return;
      if (grants()[key] === draft) {
        setGrants(old => { const next = { ...old }; delete next[key]; return next; });
        setSaved(old => ({ ...old, [key]: server(value.id)?.revision ?? 0 }));
      }
    } catch (error) { if (!disposed) setErrors(old => ({ ...old, [key]: message(error) })); }
    finally { if (!disposed) setPending(old => ({ ...old, [value.id]: false })); }
  }
  async function changeServer(value: McpSettingsCommand) {
    const id = value.serverId;
    if (blocked(id)) return;
    setPending(old => ({ ...old, [id]: true })); setErrors(old => ({ ...old, [id]: '' }));
    try {
      await props.onCommand(value);
      if (disposed) return;
      if (value.type === 'remove_mcp_server') {
        setRemoveConfirmation(undefined);
        if (expanded() === id) setExpanded('');
        setConfigs(old => { const next = { ...old }; delete next[id]; return next; });
        setOpened(old => { const next = { ...old }; delete next[id]; return next; });
        setGrants(old => Object.fromEntries(Object.entries(old).filter(([key]) => !key.startsWith(id + '/'))));
      }
    } catch (error) { if (!disposed) setErrors(old => ({ ...old, [id]: message(error) })); }
    finally { if (!disposed) setPending(old => ({ ...old, [id]: false })); }
  }
  function discardNew() {
    if (blocked('new')) return;
    setConfigs(old => { const next = { ...old }; delete next.new; return next; });
    setOpened(old => { const next = { ...old }; delete next.new; return next; });
    setConfigErrors(old => { const next = { ...old }; delete next.new; return next; });
    if (expanded() === 'new') setExpanded('');
  }
  return <section class="tools-panel" aria-label="MCP">
    <div class="settings-section-heading"><span>{t('我的 MCP')}</span><SettingsAddButton disabled={props.snapshot.mcpServers.length >= 8 || Boolean(discovering())} onClick={add} /></div>
    <For each={ids()}>{id => <article class="connection-card mcp-card" classList={{ expanded: expanded() === id }} aria-label={label(id) || t('新 MCP')}>
      <div class="connection-summary"><button class="connection-expand" aria-expanded={expanded() === id} aria-controls={'mcp-editor-' + id} onClick={() => open(id)}><Show when={expanded() === id} fallback={<ChevronRight size={15} />}><ChevronDown size={15} /></Show><span><strong>{label(id) || t('新 MCP')}</strong><Show when={server(id)}>{value => <small>{value().tools.length}{t('个工具 ·')}{value().enabled ? t('已启用') : t('已停用')}</small>}</Show></span></button><Show when={configs()[id]?.dirty || Object.keys(grants()).some(key => key.startsWith(id + '/'))}><i class="connection-dirty" title={t('未保存')} aria-label={t('未保存')} /></Show></div>
      <Show when={opened()[id]}><div id={'mcp-editor-' + id} class="connection-editor mcp-editor" hidden={expanded() !== id}>
        <div class="mcp-server-actions"><Show when={server(id)}>{value => <button class="text-button" disabled={blocked(id)} title={value().enabled ? t('停用此程序的工具') : t('启用授权，下次请求时启动')} onClick={() => void changeServer({ type: 'set_mcp_enabled', serverId: id, expectedRevision: value().revision, enabled: !value().enabled })}>{value().enabled ? t('停用') : t('启用')}</button>}</Show><button class="icon-button compact" disabled={blocked(id)} title={id === 'new' ? t('移除草稿') : t('移除程序')} aria-label={t('移除 {0}').replaceAll('{0}', () => label(id) || t('新 MCP'))} onClick={() => id === 'new' ? discardNew() : setRemoveConfirmation({ id, revision: server(id)!.revision })}><Trash2 size={14} /></button></div>
        <Show when={removeConfirmation()?.id === id}><div class="mcp-remove"><span>{t('移除此程序和授权？')}</span><button class="text-button" disabled={blocked(id)} onClick={() => setRemoveConfirmation(undefined)}>{t('取消')}</button><button class="secondary-button" disabled={blocked(id)} onClick={() => void changeServer({ type: 'remove_mcp_server', serverId: id, expectedRevision: removeConfirmation()!.revision })}>{t('移除')}</button></div></Show>
        <label class="field-label">{t('名称')}<input id={'mcp-name-' + id} value={configs()[id]?.label ?? ''} disabled={blocked(id)} maxlength={80} onInput={event => editConfig(id, { label: event.currentTarget.value })} /></label>
        <label class="field-label">{t('程序绝对路径')}<textarea rows={2} spellcheck={false} value={configs()[id]?.executable ?? ''} disabled={blocked(id)} onInput={event => editConfig(id, { executable: event.currentTarget.value })} /></label>
        <label class="field-label">{t('参数（每行一个）')}<textarea rows={3} spellcheck={false} value={configs()[id]?.argsText ?? ''} disabled={blocked(id)} onInput={event => editConfig(id, { argsText: event.currentTarget.value })} /></label>
        <label class="field-label">{t('工作目录（可选）')}<textarea rows={1} spellcheck={false} value={configs()[id]?.cwd ?? ''} disabled={blocked(id)} onInput={event => editConfig(id, { cwd: event.currentTarget.value })} /></label>
        <p class="mcp-note">{t('程序具有当前用户权限。')}<Show when={id !== 'new'}>{t('重新读取后需重选工具。')}</Show></p>
        <div class="mcp-config-actions"><Show when={configStale(id)}><button class="text-button" disabled={blocked(id)} onClick={() => ensureConfig(id, true)}>{t('载入最新配置')}</button></Show><button class="primary-button" disabled={Boolean(discovering()) || blocked(id)} onClick={() => void discover(id)}><Show when={discovering() === id} fallback={<RefreshCw size={13} />}><LoaderCircle size={13} class="spin" /></Show>{t('启动并读取工具')}</button></div>
        <Show when={configErrors()[id]}><p class="settings-error" role="alert">{configErrors()[id]}</p></Show>
        <Show when={server(id)}>{value => <div class="mcp-permissions">
          <label class="field-label">Agent<select aria-label={t('MCP 所属 Agent')} value={agentId()} onChange={event => props.onAgent(event.currentTarget.value)}><For each={props.snapshot.agents.map(agent => agent.id)}>{agent => <option value={agent}>{props.snapshot.agents.find(value => value.id === agent)?.name}</option>}</For></select></label>
          <div class="mcp-tool-list"><For each={value().tools.map(tool => tool.name)}>{name => <Show when={value().tools.find(tool => tool.name === name)}>{tool => <div class="mcp-tool">
            <label><input type="checkbox" checked={selectedNames(value()).includes(name)} disabled={!value().enabled || !agentId() || blocked(id)} onChange={event => choose(value(), name, event.currentTarget.checked)} /><span>{name}</span></label>
            <details><summary><ChevronRight size={12} /><span>{t('详情')}</span></summary><Show when={tool().description}><p>{tool().description}</p></Show><pre>{JSON.stringify(tool().inputSchema, null, 2)}</pre></details>
          </div>}</Show>}</For></div>
          <Show when={value().tools.length}><div class="mcp-grant-actions"><span>{selectedNames(value()).length}{t('个已选')}</span><Show when={saved()[grantKey(id)] === value().revision && !grants()[grantKey(id)]}><span class="saved-label"><Check size={12} />{t('已保存')}</span></Show><Show when={grants()[grantKey(id)]?.revision !== undefined && grants()[grantKey(id)].revision !== value().revision}><button class="text-button" disabled={blocked(id)} onClick={() => reloadGrants(value())}>{t('载入最新')}</button></Show><button class="primary-button" disabled={!value().enabled || !agentId() || !grants()[grantKey(id)] || blocked(id)} onClick={() => void saveGrants(value())}><Show when={pending()[id]}><LoaderCircle size={12} class="spin" /></Show>{t('保存')}</button></div></Show>
        </div>}</Show>
        <Show when={errors()[id] || errors()[grantKey(id)]}><p class="settings-error" role="alert">{errors()[id] || errors()[grantKey(id)]}</p></Show>
      </div></Show>
      <Show when={expanded() !== id && (configErrors()[id] || errors()[id] || errors()[grantKey(id)])}><p class="settings-error connection-collapsed-error" role="alert">{configErrors()[id] || errors()[id] || errors()[grantKey(id)]}</p></Show>
    </article>}</For>
  </section>;
}
