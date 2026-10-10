// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { VoiceCapabilities, VoiceProgress, VoiceRequest, VoiceResult } from './voice-contracts';
export const voiceNative = isTauri();
export async function getVoiceCapabilities(): Promise<VoiceCapabilities> {
  if (!voiceNative) return { supported: false, languages: [], unavailableReason: '请在桌面版使用语音输入' };
  return invoke('get_voice_capabilities');
}
export async function runDictation(request: VoiceRequest): Promise<VoiceResult> {
  if (!voiceNative) throw new Error('请在桌面版使用语音输入');
  return invoke('run_plugin_dictation', { ...request });
}
export async function cancelDictation(requestId: string): Promise<void> {
  if (voiceNative) await invoke('cancel_plugin_dictation', { requestId });
}
export async function subscribeVoice(accept: (progress: VoiceProgress) => void, invalidated: (requestId: string) => void): Promise<() => void> {
  if (!voiceNative) return () => {};
  const stopProgress = await listen<VoiceProgress>('voice-progress', event => accept(event.payload));
  try { const stopInvalidated = await listen<{ requestId: string }>('voice-invalidated', event => invalidated(event.payload.requestId)); return () => { stopProgress(); stopInvalidated(); }; }
  catch (error) { stopProgress(); throw error; }
}
