// SPDX-License-Identifier: MPL-2.0
import type { SpaceItem } from './contracts';

interface Box { x: number; y: number; width: number; height: number }
const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));

export function projectBackgroundItem(item: SpaceItem, background: Box, viewport: { width: number; height: number }): SpaceItem {
  return { ...item,
    x: (background.x + item.x * background.width) / viewport.width,
    y: (background.y + item.y * background.height) / viewport.height,
    width: item.width * background.width / viewport.width,
    height: item.height * background.height / viewport.height,
  };
}

export function persistBackgroundItem(item: SpaceItem, background: Box, viewport: { width: number; height: number }): SpaceItem {
  const projectedWidth = item.width * viewport.width / background.width;
  const projectedHeight = item.height * viewport.height / background.height;
  // Core permits independent media outside the captured frame. The old [0,1]
  // clamp made every large wheel zoom or edge crossing snap back on save.
  const scale = Math.min(1, 8 / Math.max(projectedWidth, projectedHeight));
  const width = projectedWidth * scale, height = projectedHeight * scale;
  return { ...item,
    x: clamp((item.x * viewport.width - background.x) / background.width + (projectedWidth - width) / 2, -16, 16),
    y: clamp((item.y * viewport.height - background.y) / background.height + (projectedHeight - height) / 2, -16, 16),
    width, height,
  };
}
