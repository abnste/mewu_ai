// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { replyWebLink } from './reply-content';

/** Native opens the system browser. Browser previews keep normal anchor behavior. */
export async function openReplyLink(source: string): Promise<void> {
  const url = replyWebLink(source);
  if (!url) throw new Error('链接不可用');
  if (!isTauri()) return;
  await invoke<void>('open_web_link', { url });
}
export const nativeReplyLinks = isTauri;
