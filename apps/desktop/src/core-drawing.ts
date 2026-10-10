// SPDX-License-Identifier: MPL-2.0
import type { ManualDrawingKind } from './contracts';
import type { VideoDrawingGrant, VideoDrawingTool } from './video-drawing-contracts';

export const coreDrawingGrant = { pluginId: 'mewu.core.drawing', revision: 1, contributionId: 'drawing-tools' } as const;
export const coreDrawingTools: readonly ManualDrawingKind[] = ['pen', 'line', 'arrow', 'rect', 'ellipse', 'text', 'highlighter', 'number', 'mosaic'];
export const coreVideoDrawingTools: readonly VideoDrawingTool[] = ['pen', 'line', 'arrow', 'rect', 'ellipse', 'text', 'number'];
export function videoDrawingGrant(): VideoDrawingGrant {
  return { pluginId: 'mewu.core.drawing', revision: 1, contributionId: 'video-drawing-tools', tools: [...coreVideoDrawingTools] };
}
export function validVideoDrawingGrant(value: VideoDrawingGrant): boolean {
  return value.pluginId === 'mewu.core.drawing' && value.revision === 1 && value.contributionId === 'video-drawing-tools'
    && value.tools.length === coreVideoDrawingTools.length && coreVideoDrawingTools.every((tool, index) => value.tools[index] === tool);
}
