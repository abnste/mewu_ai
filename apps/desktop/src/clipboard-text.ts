// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';

/** Desktop text uses the same OS-owned clipboard as image exports. WebKit's
 * navigator.clipboard is not a reliable publication channel in a native view. */
export async function writeClipboardText(text: string): Promise<void> {
  if (!text || text.includes('\0') || new TextEncoder().encode(text).length > 2 * 1024 * 1024) throw new Error('复制文本为空、过长或包含空字符');
  if (isTauri()) await invoke<void>('write_clipboard_text', { text });
  else {
    if (!navigator.clipboard) throw new Error('剪贴板不可用');
    await navigator.clipboard.writeText(text);
  }
}
