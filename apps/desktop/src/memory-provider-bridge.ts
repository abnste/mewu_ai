// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AgentMemoryStatus, MemoryEvidenceDetail, MemoryEvidencePage, MemoryProviderProbe, MemoryWriteReceipt } from './contracts';

export interface MemoryProviderConfig { requestId: string; agentId: string; pluginId: string; pluginRevision: number; contributionId: string; endpoint: string; apiKey: string }
export interface MemoryBindingTarget { agentId: string; bindingId: string; expectedBindingRevision: number }
export const memoryProviderNative = isTauri();
function desktop() { if (!memoryProviderNative) throw new Error('请在桌面版连接记忆服务'); }

export async function getMemoryProviderState(args: { agentId: string; cursor?: string; limit?: number }): Promise<AgentMemoryStatus> {
  desktop(); return invoke('get_memory_provider_state', { ...args, cursor: args.cursor ?? null, limit: args.limit ?? 25 });
}
export async function probeMemoryProvider(args: MemoryProviderConfig): Promise<MemoryProviderProbe> {
  desktop(); return invoke('probe_memory_provider', { ...args });
}
export async function createMemoryBinding(args: MemoryProviderConfig & { expectedProviderRevision: number }): Promise<AgentMemoryStatus> {
  desktop(); return invoke('create_memory_binding', { ...args });
}
export async function selectMemoryProvider(args: { agentId: string; expectedProviderRevision: number; bindingId: string | null; expectedBindingRevision: number | null }): Promise<AgentMemoryStatus> {
  desktop(); return invoke('select_memory_provider', { ...args });
}
export async function setMemorySync(args: MemoryBindingTarget & { completedTurnSync: boolean }): Promise<AgentMemoryStatus> {
  desktop(); return invoke('set_memory_sync', { ...args });
}
export async function retireMemoryBinding(args: MemoryBindingTarget): Promise<AgentMemoryStatus> {
  desktop(); return invoke('retire_memory_binding', { ...args });
}
export async function externalMemoryPage(args: { agentId: string; bindingId: string; query?: string; cursor?: string; limit?: number }): Promise<MemoryEvidencePage> {
  desktop(); return invoke('external_memory_page', { ...args, query: args.query ?? '', cursor: args.cursor ?? null, limit: args.limit ?? 25 });
}
export async function externalMemoryEntry(args: { agentId: string; bindingId: string; evidenceId: string }): Promise<MemoryEvidenceDetail | null> {
  desktop(); return invoke('external_memory_entry', { ...args });
}
export async function queueExternalMemory(args: MemoryBindingTarget & { requestId: string; text: string }): Promise<MemoryWriteReceipt> {
  desktop(); return invoke('queue_external_memory', { ...args });
}
export async function forgetExternalMemory(args: { agentId: string; bindingId: string; evidenceId: string; expectedRevision: number }): Promise<AgentMemoryStatus> {
  desktop(); return invoke('forget_external_memory', { ...args });
}
export async function cancelMemoryRequest(requestId: string): Promise<void> {
  if (memoryProviderNative) await invoke('cancel_memory_request', { requestId });
}
export async function subscribeMemoryProvider(invalidate: (agentId: string) => void): Promise<() => void> {
  if (!memoryProviderNative) return () => undefined;
  let stopped = false;
  const stop = await listen<{ agentId: string }>('memory-state', event => {
    if (!stopped && typeof event.payload?.agentId === 'string') invalidate(event.payload.agentId);
  });
  return () => { stopped = true; stop(); };
}
