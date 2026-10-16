// SPDX-License-Identifier: MPL-2.0
import type { ExtensionBinding, ExtensionRecord, ExtensionService, ExtensionServiceKind } from './extension-contracts';

export interface ExtensionServiceChoice { record: ExtensionRecord; service: ExtensionService; unavailableReason?: string }
export function mergeExtensionRecords(previous: ExtensionRecord[], incoming: ExtensionRecord[]): ExtensionRecord[] {
  const records = new Map(previous.map(record => [record.manifest.id, record]));
  for (const record of incoming) {
    const old = records.get(record.manifest.id);
    if (!old || record.revision >= old.revision) records.set(record.manifest.id, record);
  }
  return [...records.values()].sort((a, b) => a.manifest.id < b.manifest.id ? -1 : a.manifest.id > b.manifest.id ? 1 : 0);
}
export function extensionServices(records: ExtensionRecord[], kind: ExtensionServiceKind, includeUnsupported = false): ExtensionServiceChoice[] {
  return records.filter(record => record.state === 'enabled').flatMap(record => record.services.filter(service => service.kind === kind)
    .map(service => ({ record, service, ...(record.platformError || record.unsupportedServices?.[service.id] ? { unavailableReason: record.platformError || record.unsupportedServices?.[service.id] } : {}) }))
    .filter(choice => includeUnsupported || !choice.unavailableReason));
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
  return placedExtensionServices(records, 'selection-toolbar');
}
export function spaceExtensionServices(records: ExtensionRecord[]): ExtensionServiceChoice[] {
  return placedExtensionServices(records, 'space-toolbar');
}
function placedExtensionServices(records: ExtensionRecord[], placement: string): ExtensionServiceChoice[] {
  return extensionServices(records, 'command', true).filter(({ service }) => service.metadata !== null && typeof service.metadata === 'object'
    && !Array.isArray(service.metadata) && (service.metadata as Record<string, unknown>).placement === placement);
}
export function extensionCommandInput(service: ExtensionService): Record<string, unknown> {
  const metadata = service.metadata;
  if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) return {};
  const input = (metadata as Record<string, unknown>).input;
  return input !== null && typeof input === 'object' && !Array.isArray(input) ? input as Record<string, unknown> : {};
}
export const extensionServiceKey = (choice: ExtensionServiceChoice): string => JSON.stringify([choice.record.manifest.id, choice.service.id]);
export function extensionResultText(result: unknown): string {
  if (typeof result === 'string') return result;
  if (result && typeof result === 'object' && !Array.isArray(result)) {
    for (const field of ['text', 'answer', 'message', 'content']) {
      const value = (result as Record<string, unknown>)[field];
      if (typeof value === 'string') return value;
    }
  }
  return '';
}
