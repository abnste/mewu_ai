// SPDX-License-Identifier: MPL-2.0
import type { ExtensionRecord } from './extension-contracts';
import { extensionServices } from './extension-state';
export function toolbarLayoutServices(records: ExtensionRecord[], surface: string) {
  return extensionServices(records, 'service').filter(({ record, service }) => {
    const m = service.metadata as { surface?: unknown; targets?: unknown } | undefined;
    return record.manifest.permissions.includes('ui.render') && m?.surface === 'toolbar-layout'
      && Array.isArray(m.targets) && m.targets.includes(surface);
  }).sort((a, b) => `${a.record.manifest.id}/${a.service.id}`.localeCompare(`${b.record.manifest.id}/${b.service.id}`, 'en'));
}
export function reorderToolbar<T>(items: readonly T[], id: (item: T) => string | undefined, result: unknown): T[] {
  if (!result || typeof result !== 'object') throw Error('工具条布局返回格式无效');
  const order = (result as { order?: unknown }).order;
  if (!Array.isArray(order) || order.length > 128 || order.some(v => typeof v !== 'string' || !v || v.length > 128) || new Set(order).size !== order.length) throw Error('工具条排序无效');
  const ranks = new Map(order.map((key, index) => [key, index]));
  return items.map((item, index) => ({ item, index, rank: ranks.get(id(item) ?? '') ?? order.length + index }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index).map(v => v.item);
}
