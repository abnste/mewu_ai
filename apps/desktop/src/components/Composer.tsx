// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Bot, Check, ChevronDown, ChevronUp, CircleAlert, Cpu, FolderOpen, History, Link2, LoaderCircle, MessageSquare, Minus, RotateCcw, Send, Square, SquarePen, X } from 'lucide-solid';
import type { AgentProfile, ConnectionProfile, Reference, Scene } from '../contracts';
import '../connections.css';
import Markdown from './Markdown';
import ToolProgress from './ToolProgress';
import TableAnswer from './TableAnswer';
import VoiceInputButton, { type VoiceControl } from './VoiceInputButton';
import RunJournal, { type ContinueJournal } from './RunJournal';
import { latestReply } from '../run-journal';
import VideoAnswerActions from './VideoAnswerActions';
import type { VideoAnswerAction } from '../video-annotation-contracts';
import ReasoningView from './ReasoningView';
import { observeConversationTail } from '../conversation-tail';
import { ReasoningDisclosure, splitReplyReasoning } from '../reply-reasoning';
import { t } from '../i18n';
import { currentTurnPrompt, thinkingGlowColor, thinkingGlowVisible } from '../conversation-presentation';
import '../conversation.css';

interface Props {
  scene: Scene; agents: AgentProfile[]; connections: ConnectionProfile[]; draft: string; refs: Reference[];
  stream?: { runId: string; text: string; reasoning?: string }; expanded: boolean; error?: string; sending: boolean; closing: boolean;
  position?: { x: number; y: number }; selectionActive: boolean; focusRequest: number;
  onPosition: (position?: { x: number; y: number }) => void;
  onDraft: (text: string) => void; onSend: () => void; onCancel: () => void; onImport: () => void;
  onFreeze: () => void; onNew: () => void; onCapture: () => void; onClose: () => void; onSessions: () => void;
  onRemoveRef: (reference: Reference) => void; onFocusRef: (reference: Reference) => void;
  referenceInsertion?:{sceneId:string;reference:Reference;serial:number};
  onReferenceInserted?:(serial:number)=>void;
  onExpanded: (expanded: boolean) => void; onAgent: (id: string) => void; onConnection: () => void;
  onSelectConnection: (id: string) => void;
  onError?: (error: string) => void;
  voice?: VoiceControl;
  onComposition?: (composing: boolean) => void;
  onContinueJournal: ContinueJournal;
  onVideoAnswer?: (action: VideoAnswerAction) => void;
  showButtonLabels?: boolean;
  thinkingGlowEnabled?: boolean;
  thinkingGlowColor?: string;
}

export default function Composer(props: Props) {
  const reasoningViews = new Map<string, ReasoningDisclosure>();
  const disclosure = (identity: string) => { if (!reasoningViews.has(identity)) reasoningViews.set(identity, new ReasoningDisclosure()); return reasoningViews.get(identity)!; };
  const [picker, setPicker] = createSignal<'agent' | 'model'>();
  const [voiceMenu, setVoiceMenu] = createSignal(false);
  const [position, setPosition] = createSignal(props.position);
  const [dragging, setDragging] = createSignal(false);
  const [hidden, setHidden] = createSignal(false);
  const [dockHint, setDockHint] = createSignal(false);
  const [atEnd, setAtEnd] = createSignal(true);
  const [pickerBelow, setPickerBelow] = createSignal(false);
  const [pickerHeight, setPickerHeight] = createSignal(500);
  const [pickerLeft, setPickerLeft] = createSignal(-5);
  const [historyElement, setHistoryElement] = createSignal<HTMLDivElement>();
  let shell!: HTMLElement, input!: HTMLTextAreaElement, history!: HTMLDivElement, menu!: HTMLDivElement;
  let lastPointer = { x: 0, y: 0 }, focusAnchor: { x: number; y: number } | undefined;
  const running = () => props.scene.run?.status === 'running';
  const agent = () => props.agents.find(a => a.id === props.scene.agentId);
  const connection = () => props.connections.find(value => value.id === props.scene.connectionId);
  const referenceLabel = (ref: Reference) => ref.kind === 'region' ? `${t('图片')}${props.scene.regions.findIndex(r => r.id === ref.id) + 1}` : props.scene.items.find(item => item.id === ref.id)?.asset.name || t('文件');
  createEffect(on(()=>props.referenceInsertion,request=>{
    if(!request||request.sceneId!==props.scene.id||!input)return;
    const label=`@${referenceLabel(request.reference)} `;
    const start=input.selectionStart??props.draft.length,end=input.selectionEnd??start;
    input.setRangeText(label,start,end,'end');props.onDraft(input.value);
      props.onReferenceInserted?.(request.serial);
    setHidden(false);queueMicrotask(()=>input.focus({preventScroll:true}));
  }));
  const streaming = () => props.stream && props.scene.run?.id === props.stream.runId && !props.scene.messages.some(message => message.role === 'assistant' && message.runId === props.stream?.runId);
  const conversationMessages = createMemo(() => props.scene.messages.slice(props.scene.conversationStart ?? 0));
  const reply = createMemo(() => latestReply({ run: props.scene.run, messages: conversationMessages(), stream: props.stream }));
  const latestMessage = createMemo(() => props.scene.messages.find(message => message.id === reply().messageId));
  const currentPrompt = createMemo(() => currentTurnPrompt(conversationMessages(), props.scene.run?.id ?? latestMessage()?.runId));
  const replyParts = createMemo(() => splitReplyReasoning(reply().text, latestMessage()?.reasoning || (props.stream?.runId === props.scene.run?.id ? props.stream?.reasoning : '') || '', running()));
  const currentReasoning = createMemo(() => replyParts().reasoning);
  const latest = () => replyParts().text;
  const interrupted = () => props.scene.run?.status === 'failed' || props.scene.run?.status === 'canceled';
  const runPulse = () => `${props.scene.run?.id ?? ''}:${props.scene.run?.status ?? ''}`;
  const hasResponse = () => Boolean(latest() || currentReasoning() || running() || interrupted() || props.scene.run?.steps?.length);
  const historyVisible = createMemo(() => hasResponse() || Boolean(currentPrompt()) || (props.expanded && props.scene.messages.length > 0));
  const reveal = (focus = false) => {
    const wasHidden = hidden(); setHidden(false);
    if (focus || wasHidden) queueMicrotask(() => input?.focus({ preventScroll: true }));
  };
  const outside = (event: PointerEvent) => {
    if (!(event.target instanceof Element) || !event.target.closest('.channel-popover,.channel-trigger')) setPicker(undefined);
  };
  const pointer = (event: PointerEvent) => {
    lastPointer = { x: event.clientX, y: event.clientY };
    if (focusAnchor && Math.hypot(lastPointer.x - focusAnchor.x, lastPointer.y - focusAnchor.y) > 4) focusAnchor = undefined;
    if (position() || dragging() || picker() || voiceMenu() || props.voice?.pending || !shell || props.selectionActive) return;
    const target = event.target as HTMLElement, bounds = shell.getBoundingClientRect();
    const inBar = !hidden() && event.clientX >= bounds.left - 6 && event.clientX <= bounds.right + 6 && event.clientY >= bounds.top - 6 && event.clientY <= bounds.bottom + 6;
    const atEdge = event.clientY >= innerHeight - 24 && Math.abs(event.clientX - innerWidth / 2) <= shell.offsetWidth / 2 + 24;
    if (inBar || atEdge || target.closest('.session-history,.modal-backdrop')) { reveal(atEdge); return; }
    if (focusAnchor) return;
    const toolbar = document.querySelector('.capture-toolbar')?.getBoundingClientRect();
    const nearToolbar = toolbar && event.clientX >= toolbar.left - 24 && event.clientX <= toolbar.right + 24 && event.clientY >= toolbar.top - 24 && event.clientY <= toolbar.bottom + 24;
    const shouldHide = Boolean(target.closest('.capture-region') || nearToolbar);
    if (shouldHide && shell.contains(document.activeElement)) input?.blur();
    setHidden(shouldHide);
  };
  const escapePicker = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || !picker()) return;
    event.preventDefault(); event.stopImmediatePropagation();
    const trigger = shell.querySelector<HTMLButtonElement>(`[data-picker="${picker()}"]`);
    setPicker(undefined); trigger?.focus({ preventScroll: true });
  };
  document.addEventListener('pointerdown', outside); document.addEventListener('pointermove', pointer);
  document.addEventListener('keydown', escapePicker, true);
  onCleanup(() => { props.onComposition?.(false); document.removeEventListener('pointerdown', outside); document.removeEventListener('pointermove', pointer); document.removeEventListener('keydown', escapePicker, true); });
  createEffect(() => {
    props.draft;
    if (input) { input.style.height = 'auto'; input.style.height = `${Math.min(input.scrollHeight, 58)}px`; }
  });
  createEffect(() => {
    const host = historyElement();
    if (!host || !historyVisible()) return;
    onCleanup(observeConversationTail(host, atEnd));
  });
  createEffect(on(() => props.focusRequest, () => { focusAnchor = { ...lastPointer }; reveal(true); }));
  createEffect(() => { if (props.selectionActive && !position()) { setHidden(true); focusAnchor = undefined; input?.blur(); } });
  createEffect(() => {
    const text = latest(), count = props.scene.messages.length, expanded = props.expanded;
    if (atEnd()) queueMicrotask(() => { if (history && atEnd()) history.scrollTop = history.scrollHeight; });
    void text; void count; void expanded;
  });
  const placePicker = () => {
    const kind = picker();
    const button = kind && shell?.querySelector<HTMLButtonElement>(`[data-picker="${kind}"]`);
    if (!button || !menu) return;
    const trigger = button.getBoundingClientRect(), container = button.parentElement!.getBoundingClientRect();
    const above = container.top - 20, below = innerHeight - container.bottom - 20;
    const down = above < menu.scrollHeight && below > above;
    setPickerBelow(down); setPickerHeight(Math.max(80, down ? below : above));
    setPickerLeft(Math.max(8 - trigger.left, Math.min(-5, innerWidth - 8 - menu.offsetWidth - trigger.left)));
  };
  const resize = () => {
    placePicker();
    const current = position(); if (!current || !shell) return;
    const next = { x: Math.max(8, Math.min(innerWidth - shell.offsetWidth - 8, current.x)), y: Math.max(8, Math.min(innerHeight - shell.offsetHeight - 8, current.y)) };
    if (current.x !== next.x || current.y !== next.y) { setPosition(next); props.onPosition(next); }
  };
  onMount(() => { const observer = new ResizeObserver(resize); observer.observe(shell); onCleanup(() => observer.disconnect()); });
  createEffect(on(picker, open => {
    if (!open) return;
    queueMicrotask(() => {
      if (picker() !== open || !menu) return;
      placePicker();
      menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
    });
  }));
  window.addEventListener('resize', resize); onCleanup(() => window.removeEventListener('resize', resize));

  function drag(event: PointerEvent) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button,input,textarea,.conversation-scroll,.reference-chip')) return;
    event.preventDefault();
    const target = event.currentTarget as HTMLElement, bounds = shell.getBoundingClientRect();
    const original = position(), originalExpanded = props.expanded;
    const start = { x: event.clientX, y: event.clientY }; let detached = Boolean(original);
    target.setPointerCapture(event.pointerId); setDragging(true); setDockHint(true); setHidden(false);
    const move = (e: PointerEvent) => {
      const dx = e.clientX - start.x, dy = e.clientY - start.y;
      if (!detached && Math.hypot(dx, dy) >= 64) { detached = true; props.onExpanded(true); }
      const resistance = detached ? 1 : .25;
      setPosition({ x: Math.max(8, Math.min(innerWidth - shell.offsetWidth - 8, bounds.left + dx * resistance)), y: Math.max(8, Math.min(innerHeight - shell.offsetHeight - 8, bounds.top + dy * resistance)) });
    };
    const end = (e: PointerEvent) => {
      target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', end); target.removeEventListener('pointercancel', end);
      if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
      const dock = { x: (innerWidth - shell.offsetWidth) / 2, y: innerHeight - 24 - shell.offsetHeight };
      if (e.type === 'pointercancel') { setPosition(original); props.onExpanded(originalExpanded); }
      else if (!detached || (position() && Math.hypot(position()!.x - dock.x, position()!.y - dock.y) <= 40)) setPosition(undefined);
      props.onPosition(position()); setDragging(false); setDockHint(false);
    };
    target.addEventListener('pointermove', move); target.addEventListener('pointerup', end); target.addEventListener('pointercancel', end);
  }
  const send = () => {
    if (running() || props.sending || props.closing || (!props.draft.trim() && props.refs.length === 0)) return;
    setAtEnd(true); focusAnchor = { ...lastPointer }; reveal(); props.onSend();
  };
  const togglePicker = (kind: 'agent' | 'model') => setPicker(picker() === kind ? undefined : kind);
  const pickerKey = (event: KeyboardEvent, kind: 'agent' | 'model') => {
    if (event.key !== 'ArrowDown') return;
    event.preventDefault(); setPicker(kind);
    queueMicrotask(() => { if (picker() === kind) menu?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true }); });
  };
  const act = (action: () => void) => { setPicker(undefined); action(); };
  return <>
    <Show when={thinkingGlowVisible(props.scene, props.closing, props.thinkingGlowEnabled !== false)}><div class="space-thinking-glow" aria-hidden="true" style={{ '--thinking-glow-color': thinkingGlowColor(props.thinkingGlowColor) }} /></Show>
    <Show when={dockHint()}><div class="composer-dock-hint" style={{ height: `${shell?.offsetHeight || 72}px` }} /></Show>
    <section ref={shell} class="composer mewu-composer" inert={props.closing || (hidden() && !position())} classList={{ expanded: props.expanded, detached: Boolean(position()), dragging: dragging(), 'composer-hidden': hidden() && !position(), 'has-response': hasResponse() }} style={position() ? { left: `${position()!.x}px`, top: `${position()!.y}px`, bottom: 'auto', transform: 'none' } : {}} aria-label={t('对话条')} onPointerDown={drag}>
      <div class="composer-grip"><button title={t(props.expanded ? '收起历史' : '展开历史')} aria-label={t(props.expanded ? '收起历史' : '展开历史')} aria-expanded={props.expanded} onClick={() => { props.onExpanded(!props.expanded); setAtEnd(true); }}><Show when={props.expanded} fallback={<ChevronUp size={12} />}><ChevronDown size={12} /></Show></button></div>
      <Show when={props.expanded}><header class="conversation-heading">
        <MessageSquare size={15} /><span title={connection() ? `${connection()!.name} · ${connection()!.model}` : undefined}>{connection()?.model ? `${agent()?.name || 'Mewu'} · ${connection()!.model}` : agent()?.name || 'Mewu'}</span>
        <button class="icon-button compact" title={t('历史会话')} aria-label={t('历史会话')} onClick={props.onSessions}><History size={16} /></button>
        <button class="icon-button compact" title={t('最小化会话')} aria-label={t('最小化会话')} onClick={props.onFreeze}><Minus size={17} /></button>
        <button class="icon-button compact" title={t('关闭会话')} aria-label={t('关闭会话')} onClick={props.onClose}><X size={16} /></button>
      </header></Show>
      <Show when={historyVisible()}><div ref={node => { history = node; setHistoryElement(node); }} class="conversation-scroll" onWheel={event => { if (event.deltaY < 0) setAtEnd(false); }} onScroll={e => setAtEnd(e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight <= 24)}>
        <Show when={props.expanded} fallback={<><Show when={currentPrompt()}>{message => <article class="message user"><div class="message-text">{message().text}</div></article>}</Show><Show when={latest() || currentReasoning()}><article class="message assistant"><ReasoningView identity={`${props.scene.id}/${props.scene.run?.id || latestMessage()?.id || ''}`} disclosure={disclosure(`${props.scene.id}/${props.scene.run?.id || latestMessage()?.id || ''}`)} text={currentReasoning()} running={running()} /><Show when={latest()}><Show when={!streaming() && latestMessage()} fallback={<Markdown text={latest()} />}>{message => <><TableAnswer sceneId={props.scene.id} messageId={message().id} text={latest()} onError={props.onError} /><VideoAnswerActions scene={props.scene} runId={message().runId} disabled={props.sending || props.closing} onNavigate={props.onVideoAnswer} /></>}</Show></Show></article></Show></>}>
          <For each={props.scene.messages.map(message => message.id)}>{id => <Show when={props.scene.messages.find(message => message.id === id)}>{message => {
            const parts = createMemo(() => splitReplyReasoning(message().text, message().reasoning));
            return <article class="message" classList={{ user: message().role === 'user', assistant: message().role === 'assistant' }}><Show when={message().role === 'assistant'} fallback={<div class="message-text">{message().text}</div>}><><ReasoningView identity={`${props.scene.id}/${message().runId || message().id}`} disclosure={disclosure(`${props.scene.id}/${message().runId || message().id}`)} text={parts().reasoning} running={false} /><TableAnswer sceneId={props.scene.id} messageId={message().id} text={parts().text} onError={props.onError} /><VideoAnswerActions scene={props.scene} runId={message().runId} disabled={props.sending || props.closing} onNavigate={props.onVideoAnswer} /></></Show><Show when={message().role === 'user' && message().runId && (message().runId !== props.scene.run?.id || props.scene.run?.status === 'completed')}><ToolProgress steps={message().toolSteps ?? []} history records={<RunJournal runPulse={runPulse()} sceneId={props.scene.id} runId={message().runId!} history unavailable={props.sending || running() || props.closing} onContinue={props.onContinueJournal} />} /></Show></article>;
          }}</Show>}</For>
          <Show when={streaming() && (latest() || currentReasoning())}><article class="message assistant"><ReasoningView identity={`${props.scene.id}/${props.stream!.runId}`} disclosure={disclosure(`${props.scene.id}/${props.stream!.runId}`)} text={currentReasoning()} running={running()} /><Show when={latest()}><Markdown text={latest()} /></Show></article></Show>
        </Show>
        <Show when={running() && !currentReasoning() && !(streaming() && props.stream?.text) && !props.scene.run?.steps?.some(step => step.status === 'running')}><div class="thinking"><LoaderCircle size={13} class="spin" /><span>{t('正在思考')}</span></div></Show>
        <Show when={props.scene.run?.id} keyed>{_runId => <ToolProgress steps={props.scene.run?.steps ?? []} running={running()} />}</Show>
        <Show when={interrupted() && props.scene.run?.id} keyed>{runId => <RunJournal runPulse={runPulse()} sceneId={props.scene.id} runId={runId} terminalLabel={t(props.scene.run?.status === 'canceled' ? '已停止' : '未完成')} unavailable={props.sending || props.closing} onContinue={props.onContinueJournal} />}</Show>
      </div><Show when={!atEnd()}><button class="latest-answer" aria-label={t('回到最新回复')} title={t('回到最新回复')} onClick={() => { history.scrollTop = history.scrollHeight; setAtEnd(true); }}><ChevronDown size={13} /></button></Show></Show>
      <div class="composer-row" classList={{ 'with-button-labels': props.showButtonLabels !== false }}>
        <div class="channel-container composer-tool"><button class="composer-round channel-trigger" data-picker="agent" title={t('选择 Agent')} aria-label={t('选择 Agent')} aria-haspopup="dialog" aria-expanded={picker() === 'agent'} on:keydown={event => pickerKey(event, 'agent')} onClick={() => togglePicker('agent')}><Bot size={18} /></button><Show when={props.showButtonLabels !== false}><span class="composer-button-label">Agent</span></Show>
          <Show when={picker() === 'agent'}><div ref={menu} role="dialog" aria-label="Agent" class="channel-popover popover" classList={{ 'opens-down': pickerBelow() }} style={{ 'max-height': `${pickerHeight()}px`, left: `${pickerLeft()}px` }}>
            <For each={props.agents}>{entry => <button class="picker-entry" title={running() ? t('请求进行中') : props.scene.messages.length > 0 && props.scene.agentId !== entry.id ? t('已有对话，无法切换 Agent') : entry.name} disabled={running() || (props.scene.messages.length > 0 && props.scene.agentId !== entry.id)} onClick={() => act(() => props.onAgent(entry.id))}><MessageSquare size={15} /><span>{entry.name}</span><Show when={props.scene.agentId === entry.id}><Check size={14} /></Show></button>}</For>
            <div class="popover-divider" />
            <button class="picker-entry" onClick={() => act(props.onNew)}><SquarePen size={15} /><span>{t('新建对话')}</span></button>
            <button class="picker-entry" onClick={() => act(props.onSessions)}><History size={15} /><span>{t('历史会话')}</span></button>
            <button class="picker-entry" onClick={() => act(props.onFreeze)}><Minus size={15} /><span>{t('最小化会话')}</span></button>
            <button class="picker-entry" onClick={() => act(props.onClose)}><X size={15} /><span>{t('关闭会话')}</span></button>
            <Show when={position()}><button class="picker-entry" onClick={() => act(() => { setPosition(undefined); props.onPosition(undefined); })}><RotateCcw size={15} /><span>{t('归位')}</span></button></Show>
          </div></Show>
        </div>
        <div class="channel-container composer-tool"><button class="composer-round channel-trigger" data-picker="model" title={t('选择模型')} aria-label={t('选择模型')} aria-haspopup="dialog" aria-expanded={picker() === 'model'} on:keydown={event => pickerKey(event, 'model')} onClick={() => togglePicker('model')}><Cpu size={18} /></button><Show when={props.showButtonLabels !== false}><span class="composer-button-label">{t('模型')}</span></Show>
          <Show when={picker() === 'model'}><div ref={menu} role="dialog" aria-label={t('模型')} class="channel-popover popover" classList={{ 'opens-down': pickerBelow() }} style={{ 'max-height': `${pickerHeight()}px`, left: `${pickerLeft()}px` }}>
            <For each={props.connections.map(value => value.id)}>{id => <Show when={props.connections.find(value => value.id === id)}>{value => <button class="picker-entry" disabled={running() || props.sending} title={running() ? t('请求进行中') : `${value().name} · ${value().model}`} onClick={() => act(() => props.onSelectConnection(id))}><Link2 size={15} /><span class="channel-connection-label"><span>{value().name}</span><small>{value().model}</small></span><Show when={props.scene.connectionId === id}><Check size={14} /></Show></button>}</Show>}</For>
          </div></Show>
        </div>
        <div class="composer-field"><Show when={props.refs.length > 0}><div class="reference-list"><For each={props.refs}>{reference => <span class="reference-chip"><button class="reference-name" title={referenceLabel(reference)} onClick={() => props.onFocusRef(reference)}>@{referenceLabel(reference)}</button><button class="reference-remove" aria-label={`${t('移除引用')} ${referenceLabel(reference)}`} title={t('移除引用')} onClick={() => props.onRemoveRef(reference)}><X size={13} /></button></span>}</For></div></Show>
          <div class="composer-text-entry"><textarea ref={input} aria-label={t('输入消息')} placeholder={t(props.refs.length ? '询问选中内容…' : '输入消息…')} rows={1} value={props.draft} onFocus={() => setHidden(false)} onCompositionStart={() => props.onComposition?.(true)} onCompositionEnd={event => { props.onDraft(event.currentTarget.value); props.onComposition?.(false); }} onInput={event => props.onDraft(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); if (!running()) send(); } }} />
            <button class="composer-attach" title={t('添加文件')} aria-label={t('添加文件')} disabled={props.closing} onClick={props.onImport}><FolderOpen size={18} stroke-width={1.7} /></button>
          </div>
        </div>
        <Show when={props.voice}>{voice => <div class="composer-tool voice-tool"><VoiceInputButton {...voice()} onMenu={setVoiceMenu} /><Show when={props.showButtonLabels !== false}><span class="composer-button-label">{t('语音')}</span></Show></div>}</Show>
        <div class="composer-tool"><button class="composer-round composer-send" disabled={props.closing || (!running() && (props.sending || (!props.draft.trim() && props.refs.length === 0)))} aria-label={t(running() ? '停止' : '发送')} title={t(running() ? '停止' : '发送')} onClick={() => running() ? props.onCancel() : send()}><Show when={running()} fallback={<Show when={!props.sending} fallback={<LoaderCircle size={17} class="spin" />}><Send size={18} /></Show>}><Square size={13} fill="currentColor" /></Show></button><Show when={props.showButtonLabels !== false}><span class="composer-button-label">{t(running() ? '停止' : '发送')}</span></Show></div>
      </div>
      <Show when={props.error || props.scene.run?.error}><div class="composer-error"><CircleAlert size={13} /><span>{props.error || props.scene.run?.error}</span></div></Show>
    </section>
  </>;
}
