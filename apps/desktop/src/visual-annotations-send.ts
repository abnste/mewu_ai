// SPDX-License-Identifier: MPL-2.0
import type { Reference, Scene, VisualAnnotationGrant } from './contracts';
import type { PluginRecord } from './plugin-contracts';

export const coreAnnotationGrant: VisualAnnotationGrant = { pluginId: 'mewu.core.annotations', pluginRevision: 1, contributionId: 'annotate' };
export function annotationGrant(_plugins: PluginRecord[]): VisualAnnotationGrant {
  return { ...coreAnnotationGrant };
}
export function hasAnnotationTarget(scene: Scene | undefined, references: Reference[]): boolean {
  if (!scene) return false;
  const videos = references.filter(reference => reference.kind === 'item' && scene.items.some(item => item.id === reference.id && item.asset.kind === 'video'));
  if (videos.length) return videos.length === 1;
  return Boolean(scene.background && references.some(reference => reference.kind === 'region' && scene.regions.some(region => region.id === reference.id)));
}
export function annotationSendIdentity(scene: Scene, references?: Reference[]): string {
  const sources = references?.map(reference => {
    if (reference.kind === 'region') return [reference.kind, reference.id, scene.regions.find(region => region.id === reference.id) ?? null];
    const item = scene.items.find(item => item.id === reference.id);
    return [reference.kind, reference.id, item?.asset ?? null, item?.videoEdit ?? null, item?.videoAnnotations ?? null];
  });
  return JSON.stringify([scene.id, scene.agentId, scene.background, scene.connectionId, scene.run?.id ?? null, ...(sources ? [sources] : [])]);
}
export function assertAnnotationSend(input: { identity: string; sourceIdentity?: string; grant: VisualAnnotationGrant; scene?: Scene; references: Reference[]; draft: string; plugins: PluginRecord[]; active: boolean }): void {
  const current = input.scene;
  if (!input.active || !current || current.closed || current.frozen || current.run?.status === 'running' || annotationSendIdentity(current) !== input.identity) throw new Error('会话已变化');
  if (input.sourceIdentity !== undefined && annotationSendIdentity(current, input.references) !== input.sourceIdentity) throw new Error('引用内容已变化');
  const available = input.grant.pluginId === coreAnnotationGrant.pluginId && input.grant.pluginRevision === coreAnnotationGrant.pluginRevision && input.grant.contributionId === coreAnnotationGrant.contributionId;
  if (!available) throw new Error('标注能力已变化，请重试');
  if (!hasAnnotationTarget(current, input.references)) throw new Error('请引用截图选区或视频');
}
