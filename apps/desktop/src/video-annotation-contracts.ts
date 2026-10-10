// SPDX-License-Identifier: MPL-2.0
import type { VideoRange } from './contracts';
import type { VideoVectorLayoutRef } from './video-drawing-contracts';

export interface VideoPixelPoint { x: number; y: number }
export interface VideoPixelRect extends VideoPixelPoint { width: number; height: number }
export interface VideoTextLayoutRef { layoutId: string; layoutSha256: string; rasterSha256: string; width: number; height: number }
export type VideoAnnotationPrimitive =
  | { kind: 'rect'; bounds: VideoPixelRect; style: { color: string; strokeWidth: number; opacity: number } }
  | { kind: 'text'; topLeft: VideoPixelPoint; layout: VideoTextLayoutRef }
  | { kind: 'vector'; topLeft: VideoPixelPoint; layout: VideoVectorLayoutRef };
export interface VideoAnnotationOrigin { runId: string; userMessageId: string; toolEventId: string; groupId: string; targetHandle: string; manifestSha256: string; createdSha256: string }
export interface TimedVideoAnnotation { id: string; interval: VideoRange; primitive: VideoAnnotationPrimitive; origin?: VideoAnnotationOrigin }
export interface VideoAnnotationStep { id: string; before: TimedVideoAnnotation[]; after: TimedVideoAnnotation[] }
export interface VideoAnnotationDocument {
  version: 1; clock: 'sourcePlaybackTicks'; sourceId: string; sourceDurationTicks: number; sourceWidth: number; sourceHeight: number;
  revision: number; objects: TimedVideoAnnotation[]; undo: VideoAnnotationStep[]; redo: VideoAnnotationStep[];
}
export interface VideoAnnotationTarget { sceneId: string; itemId: string; sourceId: string; expectedRangeRevision: number; expectedAnnotationRevision: number }
export interface VideoAnnotationRasterRef { rasterKey: string; pngSha256: string; width: number; height: number }
export interface VideoAnnotationPlanEntry { annotationId: string; interval: VideoRange; bounds: VideoPixelRect; reference: VideoAnnotationRasterRef }
export interface VideoAnnotationPlan {
  target: VideoAnnotationTarget; documentSha256: string; clock: 'sourcePlaybackTicks'; sourceDurationTicks: number;
  sourceWidth: number; sourceHeight: number; range: VideoRange; entries: VideoAnnotationPlanEntry[];
}
export interface VideoAnnotationPreviewTarget { target: VideoAnnotationTarget; documentSha256: string; annotationId: string; reference: VideoAnnotationRasterRef }
export interface VideoAnnotationPreview extends VideoAnnotationPreviewTarget { dataUrl: string }
export type VideoAnnotationAction = { type: 'remove'; annotationId: string } | { type: 'undo' | 'redo'; operationId: string } | { type: 'move'; annotationId: string; fromTopLeft: VideoPixelPoint; toTopLeft: VideoPixelPoint };
export interface VideoTextContent { version: 1; text: string; color: string; fontSize: number }
export interface VideoTextRead { requestId: string; target: VideoAnnotationTarget; annotationId: string; reference: VideoTextLayoutRef; content: VideoTextContent }
export interface VideoAnswerAction { target: VideoAnnotationTarget; annotationId: string; runId: string; toolEventId: string; interval: VideoRange }
export interface VideoPlayerPort { sourceIdentity: string; jump: (action: VideoAnswerAction, active: () => boolean) => Promise<void> }
export type RegisterVideoPlayer = (sceneId: string, itemId: string, port: VideoPlayerPort) => () => void;
