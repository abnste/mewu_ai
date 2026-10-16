// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import * as api from '../extension-bridge';
import type { ExtensionBinding, ExtensionRecord } from '../extension-contracts';
import { extensionServiceKey, extensionServices, mergeExtensionBindings, mergeExtensionRecords } from '../extension-state';
import { Row } from './SettingControls';
import { t } from '../i18n';

interface Props { agentId: string; active: boolean; saved: boolean; onMemoryBinding?: (external: boolean | undefined) => void }
export default function ExtensionProviders(props: Props) {
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]), [bindings, setBindings] = createSignal<ExtensionBinding[]>([]);
  const [pending, setPending] = createSignal(''), [error, setError] = createSignal('');
  const [memoryKnownFor, setMemoryKnownFor] = createSignal<string>();
  const agentId = createMemo(() => props.agentId);
  let disposed = false, epoch = 0, initialized = false, eventSerial = 0, unsubscribe: (() => void) | undefined, unsubscribeBindings: (() => void) | undefined, listening: Promise<void> | undefined;
  const accept = (value: ExtensionRecord[]) => { if (!disposed) setRecords(old => mergeExtensionRecords(old, value)); };
  const agents = createMemo(() => extensionServices(records(), 'agent', true)), memories = createMemo(() => extensionServices(records(), 'memory', true));
  const choices = (kind: 'agent' | 'memory') => kind === 'agent' ? agents() : memories();
  const binding = (kind: 'agent' | 'memory') => bindings().find(binding => binding.agentId === props.agentId && binding.kind === kind);
  // Native acknowledgements/events contain the complete binding set. Retain a
  // known selection through refreshes, but never reuse another Agent's state.
  const acceptBindings = (value: ExtensionBinding[]) => { if (!disposed) { setBindings(old => mergeExtensionBindings(old, value)); setMemoryKnownFor(agentId()); } };
  const selected = (kind: 'agent' | 'memory') => { const value = binding(kind); return value?.pluginId && value?.serviceId ? JSON.stringify([value.pluginId, value.serviceId]) : ''; };
  const unavailable = (kind: 'agent' | 'memory') => Boolean(selected(kind)) && !choices(kind).some(choice => extensionServiceKey(choice) === selected(kind));
  const externalMemory = createMemo(() => !api.extensionNative || !props.saved ? false : memoryKnownFor() === agentId() ? Boolean(selected('memory')) : undefined);
  createEffect(() => props.onMemoryBinding?.(externalMemory()));
  async function ensureListening() {
    if (!listening) listening = (async () => {
      const stop = await api.subscribeExtensions(value => { eventSerial++; accept(value); });
      if (disposed) { stop(); return; } unsubscribe = stop;
      const stopBindings = await api.subscribeExtensionBindings(acceptBindings);
      if (disposed) stopBindings(); else unsubscribeBindings = stopBindings;
    })().catch(error => { unsubscribe?.(); unsubscribe = undefined; listening = undefined; throw error; });
    await listening;
  }
  async function refresh() {
    if (!api.extensionNative || !props.saved) return;
    const current = ++epoch; setError('');
    try {
      await ensureListening(); if (disposed) return;
      const beforeEvents = eventSerial;
      const [values, linked] = await Promise.all([api.getExtensions(), api.getExtensionBindings()]);
      if (!disposed && epoch === current) { if (eventSerial === beforeEvents) accept(values); acceptBindings(linked); initialized = true; }
    } catch (cause) { if (!disposed && epoch === current) setError(cause instanceof Error ? cause.message : String(cause)); }
  }
  createEffect(on(() => [props.active, props.saved, props.agentId] as const, ([active]) => { if (active && (!initialized || props.saved)) void refresh(); }));
  onCleanup(() => { disposed = true; epoch++; unsubscribe?.(); unsubscribeBindings?.(); });
  async function select(kind: 'agent' | 'memory', key: string) {
    if (pending() || !props.saved) return;
    const choice = choices(kind).find(choice => extensionServiceKey(choice) === key);
    if (key && (!choice || choice.unavailableReason)) return;
    const agentId = props.agentId, current = epoch; setPending(kind); setError('');
    try {
      const value = await api.setExtensionBinding(agentId, kind, binding(kind)?.revision, choice ? { pluginId: choice.record.manifest.id, serviceId: choice.service.id } : undefined);
      if (!disposed && current === epoch && agentId === props.agentId) acceptBindings(value);
    } catch (cause) { if (!disposed && current === epoch) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) setPending(''); }
  }
  return <Show when={api.extensionNative && props.saved}>
    <For each={['agent', 'memory'] as const}>{kind => <Row title={t(kind === 'agent' ? 'Agent 引擎' : '记忆提供方')}><select aria-label={t(kind === 'agent' ? 'Agent 引擎' : '记忆提供方')} value={selected(kind)} disabled={Boolean(pending())} onChange={event => void select(kind, event.currentTarget.value)}>
      <option value="">{t(kind === 'agent' ? 'Mewu 默认' : '当前记忆设置')}</option>
      <Show when={unavailable(kind)}><option value={selected(kind)} disabled>{binding(kind)?.pluginId} · {t('不可用')}</option></Show>
      <For each={choices(kind)}>{choice => <option disabled={Boolean(choice.unavailableReason)} value={extensionServiceKey(choice)}>{choice.service.title} · {choice.unavailableReason ? t(choice.unavailableReason) : choice.record.manifest.name}</option>}</For>
    </select></Row>}</For>
    <Show when={error()}><p class="settings-error" role="alert">{error()}</p></Show>
  </Show>;
}
