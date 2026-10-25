// SPDX-License-Identifier: MPL-2.0
import { createMemo, For, onMount, Show } from 'solid-js';
import { Bot, BrainCircuit, Brush, Captions, FileText, GalleryVerticalEnd, Globe, Languages, MessageCircle, Mic, MousePointer2, Palette, Pin, Presentation, Ruler, ScanText, ShieldCheck, TerminalSquare, Video, Workflow, Wrench } from 'lucide-solid';
import type { ExtensionCatalogEntry, ExtensionManifest, ExtensionSource } from '../extension-contracts';
import type { PluginContribution } from '../plugin-contracts';
import { cachedExtensionIcon, displayIcon, ensureExtensionIcons } from '../extension-icons';
import './plugin-icon.css';

type Identity = Pick<ExtensionManifest, 'id' | 'name' | 'tags' | 'icon'> & Partial<Pick<ExtensionCatalogEntry, 'version' | 'sha256'>>;
const builtins = { 'app.hooks': Workflow, 'app.cli': TerminalSquare, 'agent.runtime': Bot, 'space.blackboard': Presentation, 'selection.drawing-tools': Brush,
  'selection.ocr': ScanText, 'selection.scroll': GalleryVerticalEnd, 'selection.translation': Languages, 'selection.pin': Pin,
  'selection.recording': Video, 'agent.visual-annotations': Captions, 'input.speech-to-text': Mic, 'memory.provider': BrainCircuit };
export default function PluginIcon(props: { plugin: Identity; kind?: PluginContribution['kind']; size?: number; sha256?: string; source?: ExtensionSource }) {
  onMount(() => {
    if (!props.kind && props.source?.type !== 'local' && !displayIcon(props.plugin.icon)
      && !displayIcon(props.source?.type === 'github' ? props.source.entry.icon : undefined)) void ensureExtensionIcons();
  });
  const icon = createMemo(() => displayIcon(props.plugin.icon) ?? displayIcon(props.source?.type === 'github' ? props.source.entry.icon : undefined)
    ?? cachedExtensionIcon(props.plugin.id, props.plugin.version ?? '', props.sha256 ?? props.plugin.sha256 ?? (props.source?.type === 'github' ? props.source.entry.sha256 : undefined)));
  const fallback = createMemo(() => {
    if (props.kind && props.kind in builtins) return builtins[props.kind as keyof typeof builtins];
    const tags = props.plugin.tags ?? [];
    if (tags.includes('memory')) return BrainCircuit;
    if (tags.includes('ui-enhancement') || tags.includes('ui') || tags.includes('rendering')) return Ruler;
    if (tags.includes('agi') || tags.includes('agents')) return tags.includes('vision') ? MousePointer2 : Bot;
    if (tags.includes('themes') || tags.includes('theme')) return Palette;
    if (tags.includes('audio') || tags.includes('speech')) return Mic;
    if (tags.includes('workflow') || tags.includes('automation')) return Workflow;
    if (tags.includes('security') || tags.includes('privacy')) return ShieldCheck;
    if (tags.includes('documents')) return FileText;
    if (tags.includes('messaging') || tags.includes('channels')) return MessageCircle;
    if (tags.includes('model-providers') || tags.includes('models') || tags.includes('network')) return Globe;
    if (tags.includes('drawing')) return Brush;
    if (tags.includes('tools')) return Wrench;
    return undefined;
  });
  return <span class="plugin-icon" aria-hidden="true" style={{ width: `${props.size ?? 20}px`, height: `${props.size ?? 20}px`, color: icon()?.color }}>
    <Show when={icon()} fallback={<Show when={fallback()} fallback={<span class="plugin-icon-initial">{Array.from(props.plugin.name || props.plugin.id)[0]?.toLocaleUpperCase()}</span>}>{Icon => { const Glyph = Icon(); return <Glyph size={props.size ?? 20} stroke-width={1.75}/>; }}</Show>}>{art =>
      <svg viewBox="0 0 24 24" width={props.size ?? 20} height={props.size ?? 20} fill={art().filled ? 'currentColor' : 'none'} stroke={art().filled ? 'none' : 'currentColor'} stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><For each={art().paths}>{path => <path d={path}/>}</For></svg>
    }</Show>
  </span>;
}
