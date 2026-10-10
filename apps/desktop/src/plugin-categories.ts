// SPDX-License-Identifier: MPL-2.0
import type { PluginContribution, PluginManifest, PluginTag } from './plugin-contracts';

export function pluginCategory(kind: PluginContribution['kind']): PluginContribution['kind'] {
  return kind === 'recording.audio' || kind === 'artifact.video-trim' || kind === 'artifact.video-gif' ? 'selection.recording' : kind;
}
export const pluginCategories = (manifest: PluginManifest) => [...new Set(manifest.contributions.map(value => pluginCategory(value.kind)))];

export const pluginTagLabels: Record<PluginTag, string> = {
  agi:'AGI', 'ui-enhancement':'UI 增强', billing:'用量计费', themes:'主题外观',
  'model-providers':'模型与渠道接入', messaging:'通讯渠道接入', memory:'记忆', tools:'工具与能力',
  system:'系统操作', vision:'视觉与多模态', audio:'语音与音频', documents:'文档与渲染',
  skills:'技能包', automation:'工作流与自动化', privacy:'安全与隐私权限', entertainment:'娱乐',
};
export const pluginTags = Object.keys(pluginTagLabels) as PluginTag[];
const legacyModule: Record<PluginContribution['kind'], PluginTag[]> = {
  'model.connection':['model-providers'],
  'selection.workflow':['tools','automation','vision'], 'selection.drawing-tools':['vision','tools'],
  'artifact.video-drawing-tools':['vision','tools'], 'selection.ocr':['vision','documents'],
  'selection.scroll':['vision','tools'], 'selection.translation':['vision','documents'],
  'selection.pin':['ui-enhancement','vision'], 'selection.codes':['vision','tools'],
  'selection.recording':['vision','audio'], 'artifact.video-trim':['vision','tools'],
  'artifact.video-gif':['vision','tools'], 'input.speech-to-text':['audio'],
  'agent.visual-annotations':['vision','tools'], 'recording.audio':['audio'], 'memory.provider':['memory'],
};
export function pluginModuleTags(manifest: PluginManifest): PluginTag[] {
  return [...new Set(manifest.tags?.length ? manifest.tags : manifest.contributions.flatMap(value => legacyModule[value.kind]))];
}
