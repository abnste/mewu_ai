// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Snapshot } from './contracts';
import type { VideoAnnotationTarget } from './video-annotation-contracts';
import type { VideoDrawingAction, VideoDrawingGrant, VideoVectorLayoutRef, VideoVectorRead } from './video-drawing-contracts';
import { validVideoDrawingTarget, validVideoVectorReference, validateVideoVectorRead, videoDrawingNativeArgs, videoDrawingUuid } from './video-drawing-wire';
const desktop = () => { if (!isTauri()) throw Error('请在桌面版编辑视频绘制'); };
export async function getVideoAnnotationVector(requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoVectorLayoutRef): Promise<VideoVectorRead> {
  desktop();
  if (!videoDrawingUuid(requestId) || !validVideoDrawingTarget(target) || !videoDrawingUuid(annotationId) || !validVideoVectorReference(reference)) throw Error('视频绘制对象已变化');
  const expected = structuredClone({ requestId, target, annotationId, reference });
  return validateVideoVectorRead(await invoke('get_video_annotation_vector', expected), expected);
}
export async function applyVideoDrawing(requestId: string, target: VideoAnnotationTarget, grant: VideoDrawingGrant, action: VideoDrawingAction): Promise<Snapshot> {
  desktop(); return invoke('apply_video_drawing', { ...videoDrawingNativeArgs(requestId, target, grant, action) });
}
