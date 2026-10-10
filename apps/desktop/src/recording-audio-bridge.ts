// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { recordingAudioGrants, validRecordingAudioState, type RecordingAudioState, type RecordingAudioMode, type RecordingAudioGrant } from './recording-audio';
export const recordingAudioNative = isTauri();
function checked(value: unknown): RecordingAudioState { if (!validRecordingAudioState(value)) throw new Error('声音设置状态无效'); return value; }
export async function getRecordingAudio(): Promise<RecordingAudioState> {
  if (!recordingAudioNative) return { revision: 0, sequence: 0, mode: 'system', grant: recordingAudioGrants()[0], editable: false, message: '仅桌面版可录屏' };
  return checked(await invoke('get_recording_audio'));
}
export async function setRecordingAudio(expectedRevision: number, mode: RecordingAudioMode, grant: RecordingAudioGrant | null): Promise<RecordingAudioState> {
  if (!recordingAudioNative) throw new Error('仅桌面版可录屏');
  return checked(await invoke('set_recording_audio', { expectedRevision, mode, grant }));
}
export async function subscribeRecordingAudio(accept: (state: RecordingAudioState) => void): Promise<() => void> {
  if (!recordingAudioNative) { accept(await getRecordingAudio()); return () => {}; }
  let sequence = -1, stopped = false;
  const receive = (state: RecordingAudioState) => { if (!stopped && state.sequence >= sequence) { sequence = state.sequence; accept(state); } };
  const stop = await listen<unknown>('recording-audio-state', event => { if (validRecordingAudioState(event.payload)) receive(event.payload); });
  try { receive(await getRecordingAudio()); }
  catch (error) { stopped = true; stop(); throw error; }
  return () => { stopped = true; stop(); };
}
