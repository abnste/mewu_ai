// SPDX-License-Identifier: MPL-2.0
import type { ExtensionBinding, ExtensionRecord, ExtensionService, ExtensionServiceKind } from './extension-contracts';

export interface ExtensionServiceChoice { record: ExtensionRecord; service: ExtensionService }
export function mergeExtensionRecords(previous: ExtensionRecord[], incoming: ExtensionRecord[]): ExtensionRecord[] {
  const records = new Map(previous.map(record => [record.manifest.id, record]));
  for (const record of incoming) {
    const old = records.get(record.manifest.id);
    if (!old || record.revision >= old.revision) records.set(record.manifest.id, record);
  }
  return [...records.values()].sort((a, b) => a.manifest.id < b.manifest.id ? -1 : a.manifest.id > b.manifest.id ? 1 : 0);
}
export function extensionServices(records: ExtensionRecord[], kind: ExtensionServiceKind): ExtensionServiceChoice[] {
  return records.filter(record => record.state === 'enabled').flatMap(record => record.services.filter(service => service.kind === kind).map(service => ({ record, service })));
}
export function mergeExtensionBindings(previous: ExtensionBinding[], incoming: ExtensionBinding[]): ExtensionBinding[] {
  const key = (value: ExtensionBinding) => JSON.stringify([value.agentId, value.kind]);
  const bindings = new Map(previous.map(binding => [key(binding), binding]));
  for (const binding of incoming) {
    const old = bindings.get(key(binding));
    if (!old || binding.revision >= old.revision) bindings.set(key(binding), binding);
  }
  return [...bindings.values()];
}
export function selectionExtensionServices(records: ExtensionRecord[]): ExtensionServiceChoice[] {
  return extensionServices(records, 'command').filter(({ service }) => service.metadata !== null && typeof service.metadata === 'object'
    && !Array.isArray(service.metadata) && (service.metadata as Record<string, unknown>).placement === 'selection-toolbar');
}
export const extensionServiceKey = (choice: ExtensionServiceChoice): string => JSON.stringify([choice.record.manifest.id, choice.service.id]);
export function extensionResultText(result: unknown): string {
  if (typeof result === 'string') return result;
  if (result && typeof result === 'object' && !Array.isArray(result)) {
    for (const field of ['text', 'answer', 'message']) {
      const value = (result as Record<string, unknown>)[field];
      if (typeof value === 'string') return value;
    }
  }
  return '';
}
