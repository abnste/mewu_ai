// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CodeAction, CodeScanRequest, CodeScanResult } from './code-contracts';
export const codeNative = isTauri();
export async function scanCodes(request: CodeScanRequest): Promise<CodeScanResult> {
  if (!codeNative) throw new Error('请在桌面版识别二维码与条码');
  return invoke('scan_plugin_codes', request as unknown as Record<string, unknown>);
}
export async function cancelCodeScan(requestId: string): Promise<void> {
  if (codeNative) await invoke('cancel_plugin_code_scan', { requestId });
}
export async function actOnCode(sourceToken: string, codeId: string, action: CodeAction): Promise<void> {
  if (!codeNative) throw new Error('请在桌面版操作识别结果');
  await invoke('act_on_code', { sourceToken, codeId, action });
}
export async function subscribeCodeInvalidated(accept: (requestId: string) => void): Promise<() => void> {
  if (!codeNative) return () => {};
  return listen<{ requestId: string }>('code-scan-invalidated', event => accept(event.payload.requestId), { target: 'space' });
}
