// SPDX-License-Identifier: MPL-2.0
import type { Reference, Scene } from './contracts';
import { isBlackboard } from './blackboard';

/** Only a current, explicitly referenced screen region can accompany a message. */
export function chatRegion(scene: Scene | undefined, references: Reference[], activeRegionId?: string): string | undefined {
  if (!scene?.background || scene.closed || scene.frozen || isBlackboard(scene)) return;
  const regions = scene.regions.filter(region => !region.imageOverride && references.some(reference => reference.kind === 'region' && reference.id === region.id));
  if (activeRegionId) return regions.find(region => region.id === activeRegionId)?.id;
  return regions.length === 1 ? regions[0].id : undefined;
}

export function chatRegionIdentity(scene: Scene | undefined, regionId: string | undefined): string | undefined {
  if (!scene?.background || !regionId) return;
  const region = scene.regions.find(region => region.id === regionId);
  if (!region || region.imageOverride) return;
  return JSON.stringify([scene.id, scene.background.id, region.id, region.x, region.y, region.width, region.height]);
}
