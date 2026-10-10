// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, ChevronDown, LoaderCircle, Mic, Square } from 'lucide-solid';
import type { VoiceCapabilities, VoiceLanguage, VoicePending } from '../voice-contracts';
import { voiceLanguages } from '../voice-input';
import '../voice-input.css';

export interface VoiceControl {
  capabilities?: VoiceCapabilities;
  language: VoiceLanguage;
  pending?: VoicePending;
  disabled: boolean;
  readingCapabilities?: boolean;
  retryCapabilities?: boolean;
  onLanguage: (language: VoiceLanguage) => void;
  onToggle: () => void;
}
export default function VoiceInputButton(props: VoiceControl & { onMenu: (open: boolean) => void }) {
  let host!: HTMLDivElement, trigger!: HTMLButtonElement;
  const [open, setOpen] = createSignal(false), [below, setBelow] = createSignal(false);
  const options = () => props.capabilities ? voiceLanguages(props.capabilities) : [];
  const available = () => options().some(option => option.value === props.language);
  const title = () => props.pending ? props.pending.awaitingComposition ? t('等待输入完成') : { starting: t('正在启动'), listening: t('停止语音输入'), stopping: t('正在停止') }[props.pending.phase] : props.readingCapabilities || !props.capabilities ? t('正在读取语音识别器') : props.retryCapabilities ? t('重新读取语音识别器') : !props.capabilities.supported ? props.capabilities.unavailableReason || t('语音输入不可用') : !available() ? t('当前语音语言未安装') : t('语音输入');
  function close() { setOpen(false); props.onMenu(false); }
  function menu() { if (open()) { close(); return; } setBelow(trigger.getBoundingClientRect().top < 135); setOpen(true); props.onMenu(true); }
  const outside = (event: PointerEvent) => { if (event.target instanceof Node && !host.contains(event.target)) close(); };
  const escape = (event: KeyboardEvent) => { if (event.key === 'Escape' && open()) { event.preventDefault(); event.stopImmediatePropagation(); close(); trigger.focus(); } };
  document.addEventListener('pointerdown', outside); document.addEventListener('keydown', escape, true);
  onCleanup(() => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape, true); props.onMenu(false); });
  return <div ref={host} class="voice-input-control">
    <button class="composer-round voice-trigger" classList={{ listening: props.pending?.phase === 'listening' }} title={title()} aria-label={props.pending ? title() : props.retryCapabilities ? t('重新读取语音识别器') : t('语音输入')} disabled={props.pending ? props.pending.phase === 'stopping' && !props.pending.awaitingComposition : props.disabled || props.readingCapabilities || (!available() && !props.retryCapabilities)} onClick={() => { close(); props.onToggle(); }}>
      <Show when={props.pending} fallback={<Mic size={18} />}>{pending => <Show when={pending().phase === 'listening' || pending().awaitingComposition} fallback={<LoaderCircle size={17} class="spin" />}><Square size={12} fill="currentColor" /></Show>}</Show>
    </button>
    <button ref={trigger} class="voice-language-trigger" aria-label={t("选择语音语言")} title={t("语音语言")} aria-expanded={open()} disabled={props.disabled || !!props.pending || !options().length} onClick={menu}><ChevronDown size={10} /></button>
    <Show when={open()}><div class="voice-language-menu popover" role="menu" classList={{ 'opens-down': below() }}><For each={options()}>{option => <button role="menuitemradio" aria-checked={props.language === option.value} onClick={() => { props.onLanguage(option.value); close(); }}><span>{t(option.label)}</span><Show when={props.language === option.value}><Check size={13} /></Show></button>}</For></div></Show>
  </div>;
}
