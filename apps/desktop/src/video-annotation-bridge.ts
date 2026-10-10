// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Snapshot, SpaceItem } from './contracts';
import type { VideoAnnotationAction, VideoAnnotationPreviewTarget, VideoAnnotationTarget, VideoTextContent, VideoTextLayoutRef, VideoTextRead } from './video-annotation-contracts';
import { validateVideoTextContent, validateVideoTextRead } from './video-manual-edit';
import { validateVideoAnnotationPlan, validVideoAnnotationTarget, VideoAnnotationReadBroker } from './video-annotation-preview';
import { videoAnnotationTarget } from './video-annotations';
const broker = new VideoAnnotationReadBroker();
const desktop = () => { if (!isTauri()) throw Error('请在桌面版读取视频标注'); };
export function getVideoAnnotationPlan(sceneId: string, item: SpaceItem) {
  desktop(); const captured = structuredClone(item), target = videoAnnotationTarget(sceneId, captured);
  if (!validVideoAnnotationTarget(target)) return Promise.reject(Error('视频标注已变化'));
  return broker.read(async () => validateVideoAnnotationPlan(await invoke('get_video_annotation_plan', { target }), sceneId, captured));
}
export function getVideoAnnotationPreview(request: VideoAnnotationPreviewTarget, sourceIdentity: string) {
  desktop(); const captured = structuredClone(request);
  return broker.preview(captured, sourceIdentity, () => invoke('get_video_annotation_preview', { ...captured }));
}
export async function applyVideoAnnotationDocument(target: VideoAnnotationTarget, action: VideoAnnotationAction): Promise<Snapshot> {
  desktop();
  if (!validVideoAnnotationTarget(target) || !['remove', 'undo', 'redo', 'move'].includes(action.type)) throw Error('视频标注已变化');
  return invoke('apply_video_annotation_document', { target: { ...target }, action: { ...action } });
}
export async function getVideoAnnotationText(requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef): Promise<VideoTextRead> {
  desktop();
  if (!validVideoAnnotationTarget(target)) throw Error('视频标注已变化');
  const captured = structuredClone({ requestId, target, annotationId, reference });
  return validateVideoTextRead(await invoke<VideoTextRead>('get_video_annotation_text', captured), requestId, target, annotationId, reference);
}
export async function editVideoAnnotationText(requestId: string, target: VideoAnnotationTarget, annotationId: string, reference: VideoTextLayoutRef, content: VideoTextContent): Promise<Snapshot> {
  desktop();
  if (!validVideoAnnotationTarget(target) || !validateVideoTextContent(content)) throw Error('视频文字无效');
  return invoke('edit_video_annotation_text', structuredClone({ requestId, target, annotationId, reference, content }));
}
