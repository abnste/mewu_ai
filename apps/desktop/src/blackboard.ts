// SPDX-License-Identifier: MPL-2.0
import type { Scene } from './contracts';

export function isBlackboard(scene: Scene | undefined): boolean {
  const asset = scene?.background;
  if (!asset || asset.kind !== 'image' || asset.name !== '黑板.png') return false;
  return asset.path.replaceAll('\\', '/').endsWith(`/${asset.id}.board.png`)
    || asset.path.startsWith('data:image/png;');
}
