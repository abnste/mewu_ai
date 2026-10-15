// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createOptionalFeatureState } from '../optional-feature-state';
import * as extensions from '../extension-bridge';
import type { ExtensionRecord } from '../extension-contracts';
import { extensionServices, mergeExtensionRecords } from '../extension-state';
import { Database, Info, Link2, Puzzle, ScanLine, Settings2, UserRound, Wrench, X } from 'lucide-solid';
import type { AgentProfile, ConnectionProfile, DiscoverMcpServer, Snapshot } from '../contracts';
import type { DeleteMemory, SaveMemory } from './MemoryList';
import AgentsPanel from './AgentsPanel';
import ToolsPanel, { type McpSettingsCommand } from './ToolsPanel';
import PluginsPanel from './PluginsPanel';
import ConnectionsPanel from './ConnectionsPanel';
import CaptureSettings from './CaptureSettings';
import GeneralSettings from './GeneralSettings';
import { AboutSettings, createSettingsInformation, DataSettings } from './SettingsFacts';
import type { Preferences } from '../interface-preferences';
export type { Preferences } from '../interface-preferences';
import { t } from '../i18n';
import { nativeSelectOwnsEscape } from '../native-select-escape';
import '../settings-sections.css';

export type SettingsTab = 'general' | 'capture' | 'agent' | 'plugins' | 'data' | 'about';
interface Props {
  snapshot: Snapshot;
  initialTab: SettingsTab;
  preferences: Preferences;
  hotkey: string;
  draggable?: boolean;
  onPreferences: (value: Preferences) => void;
  onSaveAgent: (agent: AgentProfile) => Promise<void>;
  onSaveMemory: (value: SaveMemory) => Promise<void>;
  onDeleteMemory: (value: DeleteMemory) => Promise<void>;
  onSaveConnectionProfile: (profile: ConnectionProfile, expectedRevision?: number, apiKey?: string) => Promise<void>;
  onDeleteConnectionProfile: (id: string, expectedRevision: number) => Promise<void>;
  onSetDefaultConnection: (id: string | null) => Promise<void>;
  onDiscoverMcpServer: (value: DiscoverMcpServer) => Promise<void>;
  onMcpCommand: (value: McpSettingsCommand) => Promise<void>;
  onClose: () => void;
}

export default function SettingsDialog(props: Props) {
  const optional = createOptionalFeatureState(() => props.snapshot.connections);
  // Presentation only: a pending authorization read must not destroy editors.
  // Execution continues to use the current, fail-closed capability accessor.
  const hostAgentVisible = createMemo(previous => {
    const value = optional(); return value === undefined ? previous : Boolean(value.agent);
  }, false);
  const [extensionRecords, setExtensionRecords] = createSignal<ExtensionRecord[]>([]);
  const externalAgentVisible = createMemo(() => extensionServices(extensionRecords(), 'agent').length > 0);
  const agentEnabled = () => hostAgentVisible() || externalAgentVisible();
  let extensionDisposed = false, extensionEvents = 0, stopExtensions: (() => void) | undefined;
  onMount(async () => {
    if (!extensions.extensionNative) return;
    const accept = (values: ExtensionRecord[]) => { if (!extensionDisposed) setExtensionRecords(old => mergeExtensionRecords(old, values)); };
    try {
      stopExtensions = await extensions.subscribeExtensions(values => { extensionEvents++; accept(values); });
      if (extensionDisposed) { stopExtensions(); return; }
      const beforeEvents = extensionEvents, installed = await extensions.getExtensions();
      if (beforeEvents === extensionEvents) accept(installed);
    } catch { /* Unavailable discovery does not grant execution authority. */ }
  });
  onCleanup(() => { extensionDisposed = true; stopExtensions?.(); });
  const [tab, setTab] = createSignal(props.initialTab);
  const facts = createSettingsInformation(() => tab() === 'general' || tab() === 'data' || tab() === 'about');
  const [agentId, setAgentId] = createSignal(props.snapshot.scenes.find(s => s.id === props.snapshot.activeSceneId)?.agentId || props.snapshot.agents[0]?.id || '');
  const [agentPane, setAgentPane] = createSignal<'identity' | 'connection' | 'tools'>(props.initialTab === 'agent' ? 'connection' : 'identity');
  createEffect(() => { if (!agentEnabled()) setAgentPane('connection'); });
  let dialog!: HTMLElement, closeButton!: HTMLButtonElement;
  const nav = [
    { id: 'general' as const, label: '通用', icon: Settings2 },
    { id: 'capture' as const, label: '截图与录屏', icon: ScanLine },
    { id: 'agent' as const, label: 'Agent 与连接', icon: UserRound },
    { id: 'plugins' as const, label: '插件', icon: Puzzle },
    { id: 'data' as const, label: '数据', icon: Database },
    { id: 'about' as const, label: '关于', icon: Info },
  ];
  onMount(() => closeButton.focus());
  const keyboard = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.target instanceof Element && event.target.closest('[data-settings-inner-dialog]')) return;
    // The recorder owns Escape, including IME and lease admission. Do not let
    // the dialog's document listener race Solid's delegated field events.
    if (event.key === 'Escape' && event.target instanceof Element && event.target.closest('.capture-shortcut-field input')) return;
    if (nativeSelectOwnsEscape(event)) return;
    if (event.key === 'Escape') { event.stopPropagation(); props.onClose(); }
    if (event.key !== 'Tab') return;
    const elements = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select,textarea,summary,[tabindex="0"]')].filter(el => el.getClientRects().length > 0);
    if (event.shiftKey && document.activeElement === elements[0]) { event.preventDefault(); elements.at(-1)?.focus(); }
    else if (!event.shiftKey && document.activeElement === elements.at(-1)) { event.preventDefault(); elements[0]?.focus(); }
  };
  document.addEventListener('keydown', keyboard);
  onCleanup(() => { document.removeEventListener('keydown', keyboard); });
  return <div class="modal-backdrop" onPointerDown={event => { if (event.target === event.currentTarget) props.onClose(); }}>
    <section ref={dialog} class="settings-dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <header class="settings-header" data-tauri-drag-region={props.draggable ? true : undefined}><h1 id="settings-title" data-tauri-drag-region={props.draggable ? true : undefined}>{t("设置")}</h1><button ref={closeButton} class="icon-button" title={t("关闭")} aria-label={t("关闭设置")} onClick={props.onClose}><X size={18} /></button></header>
      <div class="settings-body">
        <nav class="settings-nav" aria-label={t("设置分类")}><For each={nav}>{entry => <button classList={{ selected: tab() === entry.id }} onClick={() => setTab(entry.id)}><entry.icon size={16} /><span>{t(entry.id === 'agent' && !agentEnabled() ? '连接' : entry.label)}</span></button>}</For></nav>
        <div class="settings-content">
          <div hidden={tab() !== 'general'}><GeneralSettings active={tab() === 'general'} preferences={props.preferences} onPreferences={props.onPreferences} platform={facts.info()?.platform} /></div>
          <div hidden={tab() !== 'capture'}><CaptureSettings active={tab() === 'capture'} preferences={props.preferences} onPreferences={props.onPreferences} /></div>
          <div hidden={tab() !== 'agent'}><header class="settings-page-heading"><h2>{t(agentEnabled() ? "Agent 与连接" : "连接")}</h2></header>
            <div class="segmented"><Show when={agentEnabled()}><button aria-pressed={agentPane() === 'identity'} classList={{ selected: agentPane() === 'identity' }} onClick={() => setAgentPane('identity')}><UserRound size={13} />Agent</button></Show><button aria-pressed={agentPane() === 'connection'} classList={{ selected: agentPane() === 'connection' }} onClick={() => setAgentPane('connection')}><Link2 size={13} />{t("连接")}</button><Show when={agentEnabled()}><button aria-pressed={agentPane() === 'tools'} classList={{ selected: agentPane() === 'tools' }} onClick={() => setAgentPane('tools')}><Wrench size={13} />MCP</button></Show></div>
            <div hidden={agentPane() !== 'identity'}>
              <Show when={agentEnabled()}><AgentsPanel agents={props.snapshot.agents} connections={props.snapshot.connections} scenes={props.snapshot.scenes} memoryStats={props.snapshot.memoryStats} active={tab() === 'agent' && agentPane() === 'identity'} onSelect={setAgentId} onSave={props.onSaveAgent} onSaveMemory={props.onSaveMemory} onDeleteMemory={props.onDeleteMemory} /></Show>
            </div>
            <div hidden={agentPane() !== 'connection'}><ConnectionsPanel connections={props.snapshot.connections} defaultConnectionId={props.snapshot.defaultConnectionId} active={tab() === 'agent' && agentPane() === 'connection'} onSave={props.onSaveConnectionProfile} onDelete={props.onDeleteConnectionProfile} onDefault={props.onSetDefaultConnection} /></div>
            <div hidden={agentPane() !== 'tools'}><Show when={agentEnabled()}><ToolsPanel snapshot={props.snapshot} agentId={agentId()} onAgent={setAgentId} onDiscover={props.onDiscoverMcpServer} onCommand={props.onMcpCommand} /></Show></div>
          </div>
          <div hidden={tab() !== 'plugins'}><PluginsPanel active={tab() === 'plugins'} /></div>
          <div hidden={tab() !== 'data'}><DataSettings data={facts} active={tab() === 'data'} /></div>
          <div hidden={tab() !== 'about'}><AboutSettings data={facts} active={tab() === 'about'} /></div>
        </div>
      </div>
    </section>
  </div>;
}
