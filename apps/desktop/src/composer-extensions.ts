// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import * as api from './extension-bridge';
import type { ExtensionBinding, ExtensionRecord } from './extension-contracts';
import { extensionServiceKey, extensionServices, mergeExtensionBindings } from './extension-state';

export interface AgentModel { id: string; name: string }
export interface AgentModelCatalog { models: AgentModel[]; selectedModelId: string; selection: unknown }
export function parseAgentModelCatalog(value: unknown): AgentModelCatalog {
  if (!value || typeof value !== 'object') throw Error('模型列表无效');
  const data = value as Record<string, unknown>;
  if (data.ok === false) throw Error(typeof data.error === 'string' ? data.error.slice(0, 500) : '无法读取模型');
  if (data.ok !== true || !Array.isArray(data.models) || data.models.length > 512 || typeof data.selectedModelId !== 'string' || data.selectedModelId.length > 300) throw Error('模型列表无效');
  const models: AgentModel[] = [], ids = new Set<string>();
  for (const model of data.models) {
    if (!model || typeof model.id !== 'string' || !model.id || model.id.length > 300 || typeof model.name !== 'string' || model.name.length > 300 || ids.has(model.id)) throw Error('模型列表无效');
    ids.add(model.id); models.push({ id: model.id, name: model.name });
  }
  return { models, selectedModelId: data.selectedModelId, selection: data.selection ?? null };
}

/** Provider-neutral UI. Authentication, discovery and model persistence stay in the plugin. */
export function createComposerExtensions(props: { sceneId: () => string; agentId: () => string; records: () => ExtensionRecord[]; busy: () => boolean }) {
  const [bindings, setBindings] = createSignal<ExtensionBinding[]>([]);
  const [known, setKnown] = createSignal(false), [pending, setPending] = createSignal(false), [error, setError] = createSignal('');
  const [catalog, setCatalog] = createSignal<AgentModelCatalog>();
  let disposed = false, epoch = 0, stop: (() => void) | undefined;
  const engines = createMemo(() => extensionServices(props.records(), 'agent', true));
  const binding = createMemo(() => bindings().find(v => v.agentId === props.agentId() && v.kind === 'agent'));
  const selected = () => { const b = binding(); return b?.pluginId && b.serviceId ? JSON.stringify([b.pluginId, b.serviceId]) : ''; };
  const engine = createMemo(() => engines().find(v => extensionServiceKey(v) === selected()));
  const modelsService = createMemo(() => {
    const choice = engine(), id = (choice?.service.metadata as Record<string, unknown> | undefined)?.modelsService;
    return choice && !choice.unavailableReason && typeof id === 'string' && choice.record.services.some(s => s.kind === 'service' && s.id === id) && !choice.record.unsupportedServices?.[id] ? id : undefined;
  });
  const identity = createMemo(() => JSON.stringify([props.sceneId(), props.agentId(), selected(), binding()?.revision, engine()?.record.revision, modelsService()]));
  createEffect(on(identity, () => { epoch++; setCatalog(undefined); setError(''); setPending(false); }));
  const accept = (values: ExtensionBinding[]) => { if (!disposed) { setBindings(old => mergeExtensionBindings(old, values)); setKnown(true); } };
  onMount(async () => {
    if (!api.extensionNative) { setKnown(true); return; }
    try { const unsubscribe = await api.subscribeExtensionBindings(accept); if (disposed) { unsubscribe(); return; } stop = unsubscribe; accept(await api.getExtensionBindings()); }
    catch (cause) { if (!disposed) setError(String(cause)); }
  });
  onCleanup(() => { disposed = true; epoch++; stop?.(); });
  async function selectEngine(key: string) {
    if (!known() || pending() || props.busy()) return false;
    const choice = engines().find(v => extensionServiceKey(v) === key);
    if (key && (!choice || choice.unavailableReason)) return false;
    const current = epoch, agentId = props.agentId(); setPending(true); setError('');
    try {
      const values = await api.setExtensionBinding(agentId, 'agent', binding()?.revision, choice ? { pluginId: choice.record.manifest.id, serviceId: choice.service.id } : undefined);
      if (disposed || epoch !== current) return false;
      accept(values); return true;
    } catch (cause) { if (!disposed && epoch === current) setError(cause instanceof Error ? cause.message : String(cause)); return false; }
    finally { if (!disposed && epoch === current) setPending(false); }
  }
  async function models(modelId?: string) {
    const choice = engine(), serviceId = modelsService();
    if (!choice || !serviceId || pending() || props.busy()) return false;
    const current = epoch; setPending(true); setError('');
    try {
      const value = await api.invokeExtension({ pluginId: choice.record.manifest.id, revision: choice.record.revision, serviceId, sceneId: props.sceneId(), input: modelId === undefined ? { type: 'list' } : { type: 'select', modelId, expected: catalog()?.selection ?? null } });
      if (disposed || epoch !== current) return false;
      setCatalog(parseAgentModelCatalog(value)); return true;
    } catch (cause) { if (!disposed && epoch === current) setError(cause instanceof Error ? cause.message : String(cause)); return false; }
    finally { if (!disposed && epoch === current) setPending(false); }
  }
  return { engines, selected, engine, modelsService, known, pending, error, catalog, selectEngine, refreshModels: () => models(), selectModel: (id: string) => models(id) };
}
