// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Snapshot } from './contracts';
import { sameVideoTarget, type VideoTarget, type VideoInfo, type VideoAction, type VideoGrant, type VideoExportProgress } from './video-contracts';
export const videoNative = isTauri();
function desktop() { if (!videoNative) throw new Error('请在桌面版处理视频'); }
export async function getVideoInfo(requestId: string, target: VideoTarget): Promise<VideoInfo> {
  desktop(); const value = await invoke<VideoInfo>('get_video_info', { requestId, target });
  const m = value.metadata;
  if (value.requestId !== requestId || !sameVideoTarget(value.target, target) || !Number.isSafeInteger(m.durationTicks) || m.durationTicks <= 0 || m.durationTicks > 18_000_000_000 || ![m.width, m.height, m.frameRateNumerator, m.frameRateDenominator].every(v => Number.isSafeInteger(v) && v > 0) || typeof m.hasAudio !== 'boolean') throw new Error('视频信息已变化');
  return value;
}
export async function cancelVideoRequest(requestId: string): Promise<void> { if (videoNative) await invoke('cancel_video_request', { requestId }); }
export async function applyVideoEdit(grant: VideoGrant, target: VideoTarget, action: VideoAction): Promise<Snapshot> { desktop(); return invoke('apply_video_edit', { ...grant, target, action }); }
export async function exportVideo(requestId: string, target: VideoTarget): Promise<void> { desktop(); await invoke('export_video', { requestId, target }); }
export async function copyVideo(requestId: string, target: VideoTarget): Promise<boolean> {
  desktop();
  const receipt = await invoke<unknown>('copy_video', { requestId, target });
  if (typeof receipt !== 'boolean') throw new Error('视频复制回执无效');
  return receipt;
}
export async function subscribeVideoProgress(accept: (value: VideoExportProgress) => void): Promise<() => void> {
  return videoNative ? listen<VideoExportProgress>('video-export-progress', event => { const v = event.payload; if (typeof v.requestId === 'string' && Number.isFinite(v.percent)) accept({ requestId: v.requestId, percent: Math.max(0, Math.min(100, v.percent)) }); }) : () => {};
}
