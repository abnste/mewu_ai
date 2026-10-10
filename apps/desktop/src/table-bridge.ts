// SPDX-License-Identifier: MPL-2.0
import { invoke } from '@tauri-apps/api/core';
import { native } from './bridge';
import type { TableFormat, TableMessage, TableTarget } from './table-contracts';
import { TableMessageCache, tableTextHash } from './table-resource';
import { previewMessageTables, tablePlainText } from './table-preview';

const cache = new TableMessageCache();
export async function getMessageTables(sceneId: string, messageId: string, text: string): Promise<TableMessage> {
  const hash = await tableTextHash(text);
  return cache.get(`${sceneId}:${messageId}:${hash}`, async () => {
    const value = native ? await invoke<TableMessage>('get_message_tables', { sceneId, messageId }) : previewMessageTables(text, hash);
    if (value.sourceHash !== hash) throw new Error('回答已更新，请重试');
    return value;
  });
}
export async function exportMessageTable(target: TableTarget, format: TableFormat, text: string): Promise<void> {
  if (native) return invoke('export_message_table', { ...target, format });
  if (format === 'table' || format === 'png') throw new Error(format === 'table' ? '请在桌面版复制 Excel 表格' : '请在桌面版保存表格图片');
  const message = await getMessageTables(target.sceneId, target.messageId, text), table = message.tables.find(value => value.index === target.tableIndex);
  if (!table) throw new Error('表格已不存在');
  await navigator.clipboard.writeText(tablePlainText(table, format));
}
