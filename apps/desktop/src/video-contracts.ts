// SPDX-License-Identifier: MPL-2.0
import type { VideoRange } from './contracts';
export interface VideoTarget { sceneId: string; itemId: string; sourceId: string; expectedRevision: number }
export interface VideoMetadata { durationTicks: number; width: number; height: number; frameRateNumerator: number; frameRateDenominator: number; hasAudio: boolean }
export interface VideoInfo { requestId: string; target: VideoTarget; metadata: VideoMetadata }
export type VideoAction = { type: 'set'; from: VideoRange | null; to: VideoRange | null } | { type: 'undo' | 'redo'; from: VideoRange | null; operationId: string };
export interface VideoGrant { pluginId: string; revision: number; contributionId: string }
export interface VideoExportProgress { requestId: string; percent: number }
export interface VideoExportState { kind?: 'save' | 'copy'; requestId: string; sceneId: string; itemId: string; percent?: number; canceling: boolean }
export function sameVideoTarget(a: VideoTarget, b: VideoTarget) { return a.sceneId === b.sceneId && a.itemId === b.itemId && a.sourceId === b.sourceId && a.expectedRevision === b.expectedRevision; }
