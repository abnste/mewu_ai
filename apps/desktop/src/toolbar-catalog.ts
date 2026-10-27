// SPDX-License-Identifier: MPL-2.0
import catalog from '../ui/toolbar-catalog.json';
import { t } from './i18n';
export interface ToolbarItem { id: string; kind: 'button' | 'separator'; title: string; contextual?: boolean }
export function toolbarDefinition(surface: string): ToolbarItem[] {
  return (catalog.surfaces.find(value => value.id === surface)?.items ?? []).map(item => ({ ...item, kind: item.kind as ToolbarItem['kind'], title: t(item.title) }));
}
export function toolbarCaption(surface: string, id: string): string {
  const item = catalog.surfaces.find(value => value.id === surface)?.items.find(value => value.id === id);
  if (!item) throw Error(`Unknown toolbar item: ${surface}/${id}`);
  return t(item.title);
}
