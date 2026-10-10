// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { DrawingCommand, Snapshot } from './contracts';
import { previewDrawingCommand } from './bridge';
import { cloneLayoutTarget, DrawingLayoutPreviewCache, validLayoutTarget, type DrawingLayoutTarget, type DrawingTableFormat } from './drawing-layout-preview';
const cache = new DrawingLayoutPreviewCache();
export async function getDrawingLayoutPreview(target: DrawingLayoutTarget, sourceIdentity: string) {
  if (!isTauri()) throw Error('请在桌面版读取图中对象');
  const expected = cloneLayoutTarget(target);
  return cache.get(expected, sourceIdentity, () => invoke('get_drawing_layout_preview', { ...expected }));
}
export async function copyDrawingTable(target: DrawingLayoutTarget, format: DrawingTableFormat): Promise<void> {
  if (!isTauri()) throw Error('请在桌面版复制表格');
  if (!validLayoutTarget(target) || target.reference.kind !== 'table' || !['table', 'markdown', 'csv', 'tsv', 'png'].includes(format)) throw Error('表格已更新');
  await invoke('copy_drawing_table', { ...cloneLayoutTarget(target), format });
}
export async function applyDrawingDocument(command: DrawingCommand): Promise<Snapshot> {
  if (!isTauri()) return previewDrawingCommand(command);
  if (command.type === 'add_drawing') throw Error('无法新增此对象');
  return invoke('apply_drawing_document', { command });
}
