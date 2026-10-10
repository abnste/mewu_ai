// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, Show } from 'solid-js';
import { ChevronDown, LoaderCircle } from 'lucide-solid';
import { t } from '../i18n';
import { ReasoningDisclosure, reasoningDisplayText } from '../reply-reasoning';
import Markdown from './Markdown';
import './reply-content.css';

export default function ReasoningView(props: { identity: string; text: string; running: boolean; disclosure?: ReasoningDisclosure }) {
  const localDisclosure = new ReasoningDisclosure();
  const disclosure = () => props.disclosure ?? localDisclosure;
  const [expanded, setExpanded] = createSignal(false);
  let panel: HTMLDivElement | undefined;
  createEffect(() => setExpanded(disclosure().update(props.identity, props.running, Boolean(props.text.trim()))));
  createEffect(() => { props.text; if (expanded() && props.running) queueMicrotask(() => { if (panel) panel.scrollTop = panel.scrollHeight; }); });
  return <Show when={props.text.trim()}><section class="reply-reasoning" data-reasoning-identity={props.identity}>
    <button class="reply-reasoning-toggle" aria-expanded={expanded()} onClick={() => setExpanded(disclosure().toggle())}>
      <Show when={props.running}><LoaderCircle size={12} class="spin" /></Show><span>{t(props.running ? '正在思考' : '思考过程')}</span><ChevronDown size={12} classList={{ expanded: expanded() }} />
    </button>
    <Show when={expanded()}><div ref={panel} class="reply-reasoning-body" tabIndex={0}><Markdown text={reasoningDisplayText(props.text)} /></div></Show>
  </section></Show>;
}
