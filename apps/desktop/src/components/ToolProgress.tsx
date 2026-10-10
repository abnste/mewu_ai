// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createMemo, For, Show, type JSX } from 'solid-js';
import { Check, ChevronRight, CircleAlert, CircleHelp, LoaderCircle, Minus } from 'lucide-solid';
import type { ToolStep } from '../contracts';

export default function ToolProgress(props: { steps: ToolStep[]; running?: boolean; history?: boolean; records?: JSX.Element }) {
  const active = createMemo(() => props.running && !props.history ? props.steps.filter(step => step.status === 'running') : []);
  const finished = createMemo(() => props.steps.filter(step => !active().some(current => current.id === step.id)));
  const failed = () => finished().filter(step => step.status === 'failed').length;
  const unknown = () => finished().filter(step => step.status === 'unknown').length;
  const row = (step: ToolStep) => <div class="tool-step" classList={{ failed: step.status === 'failed' }}>
    <Show when={step.status !== 'unknown'} fallback={<CircleHelp size={12} />}><Show when={step.status === 'running'} fallback={<Show when={step.status === 'completed'} fallback={<CircleAlert size={12} />}><Check size={12} /></Show>}><Show when={props.running} fallback={<Minus size={12} />}><LoaderCircle size={12} class="spin" /></Show></Show></Show>
    <div><span>{step.label || step.name}</span><Show when={step.summary}><small>{step.summary}</small></Show></div>
  </div>;
  return <Show when={props.steps.length || props.records}><div class="tool-progress" aria-label={t("工具步骤")}>
    <div role="status"><For each={active()}>{row}</For></div>
    <Show when={finished().length}><details class="tool-completed"><summary><ChevronRight size={12} /><span>{finished().length}{t("个步骤")}<Show when={failed()}> · {failed()}{t("失败")}</Show><Show when={unknown()}> · {unknown()}{t("未确认")}</Show></span></summary><div class="tool-step-list"><For each={finished()}>{row}</For>{props.records}</div></details></Show>
    <Show when={!finished().length}>{props.records}</Show>
  </div></Show>;
}
