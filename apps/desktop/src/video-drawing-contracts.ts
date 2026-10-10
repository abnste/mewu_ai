// SPDX-License-Identifier: MPL-2.0
import type { VideoAnnotationTarget, VideoPixelPoint, VideoPixelRect, VideoTextContent } from './video-annotation-contracts';

export type VideoVectorTool = 'pen' | 'line' | 'arrow' | 'rect' | 'ellipse' | 'number';
export type VideoDrawingTool = VideoVectorTool | 'text';
export interface VideoDrawingGrant { pluginId: string; revision: number; contributionId: string; tools: readonly VideoDrawingTool[] }
export interface VideoVectorLayoutRef { layoutId: string; layoutSha256: string; rasterSha256: string; width: number; height: number; geometryBounds: VideoPixelRect }
export type VideoSourceVector =
  | { kind: 'pen' | 'line' | 'arrow' | 'rect' | 'ellipse'; version: 1; sourcePoints: VideoPixelPoint[]; color: string; strokeWidth: number }
  | { kind: 'number'; version: 1; sourcePoints: [VideoPixelPoint]; color: string; number: number; diameter: number };
export type VideoSourceDrawing = VideoSourceVector | { kind: 'text'; topLeft: VideoPixelPoint; content: VideoTextContent };
export type VideoLocalVectorContent =
  | { kind: 'pen' | 'line' | 'arrow' | 'rect' | 'ellipse'; version: 1; localPoints: VideoPixelPoint[]; color: string; strokeWidth: number }
  | { kind: 'number'; version: 1; localPoints: [VideoPixelPoint]; color: string; number: number; diameter: number };
export interface VideoVectorRead { requestId: string; target: VideoAnnotationTarget; annotationId: string; reference: VideoVectorLayoutRef; content: VideoLocalVectorContent }
export type VideoDrawingAction = { type: 'add'; content: VideoSourceDrawing } | { type: 'update'; annotationId: string; reference: VideoVectorLayoutRef; content: VideoSourceVector };
export interface VideoDrawingNativeArgs { requestId: string; target: VideoAnnotationTarget; pluginId: string; pluginRevision: number; contributionId: string; action: VideoDrawingAction }
