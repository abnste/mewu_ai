// SPDX-License-Identifier: MPL-2.0
import { For, Match, Switch } from 'solid-js';
import { Plus, Trash2 } from 'lucide-solid';
import type { HookInput } from '../hooks-contracts';
import { t } from '../i18n';

const kinds: [HookInput['kind'], string][] = [
  ['key', '按键'], ['text', '输入文字'], ['move', '移动鼠标'], ['click', '点击鼠标'],
  ['button', '按住或释放鼠标'], ['scroll', '滚动'], ['delay', '等待'],
];
function initial(kind: HookInput['kind']): HookInput {
  switch (kind) {
    case 'key': return { kind, key: 'Enter' };
    case 'text': return { kind, text: '' };
    case 'move': return { kind, x: 0, y: 0 };
    case 'click': return { kind, button: 'left', count: 1 };
    case 'button': return { kind, button: 'left', direction: 'press' };
    case 'scroll': return { kind, x: 0, y: 1 };
    case 'delay': return { kind, milliseconds: 500 };
  }
}
export default function HookInputEditor(props: { steps: HookInput[]; onChange: (steps: HookInput[]) => void }) {
  const change = (index: number, value: HookInput) => props.onChange(props.steps.map((step, i) => i === index ? value : step));
  return <div class="hooks-input-steps">
    <For each={props.steps.map((_, i) => i)}>{index => {
      const step = () => props.steps[index];
      const edit = (patch: object) => change(index, { ...step(), ...patch } as HookInput);
      const number = (label: string, key: string, value: () => number, min?: number, max?: number) =>
        <label class="field-label">{t(label)}<input type="number" value={value()} min={min} max={max} step="1" onInput={e => edit({ [key]: e.currentTarget.valueAsNumber })}/></label>;
      return <div class="hooks-input-step">
        <div class="hooks-action-heading"><select aria-label={t('输入操作')} value={step().kind} onChange={e => change(index, initial(e.currentTarget.value as HookInput['kind']))}>
          <For each={kinds}>{([kind, title]) => <option value={kind}>{t(title)}</option>}</For>
        </select><button class="icon-button compact" aria-label={t('删除操作')} onClick={() => props.onChange(props.steps.filter((_, i) => i !== index))}><Trash2 size={14}/></button></div>
        <Switch>
          <Match when={step().kind === 'key'}><label class="field-label">{t('按键组合')}<input value={(step() as Extract<HookInput, {kind:'key'}>).key} placeholder="Control+Shift+S" onInput={e => edit({ key: e.currentTarget.value })}/></label></Match>
          <Match when={step().kind === 'text'}><label class="field-label">{t('输入文字')}<textarea rows={3} value={(step() as Extract<HookInput, {kind:'text'}>).text} onInput={e => edit({ text: e.currentTarget.value })}/></label></Match>
          <Match when={step().kind === 'move' || step().kind === 'scroll'}><div class="connection-advanced-row">
            {number('X', 'x', () => (step() as {x:number}).x, step().kind === 'scroll' ? -1000 : undefined, step().kind === 'scroll' ? 1000 : undefined)}
            {number('Y', 'y', () => (step() as {y:number}).y, step().kind === 'scroll' ? -1000 : undefined, step().kind === 'scroll' ? 1000 : undefined)}
          </div></Match>
          <Match when={step().kind === 'click' || step().kind === 'button'}><div class="connection-advanced-row">
            <label class="field-label">{t('鼠标按钮')}<select value={(step() as {button:string}).button} onChange={e => edit({ button: e.currentTarget.value })}><option value="left">{t('左键')}</option><option value="right">{t('右键')}</option><option value="middle">{t('中键')}</option></select></label>
            <Switch><Match when={step().kind === 'click'}>{number('点击次数', 'count', () => (step() as {count:number}).count, 1, 3)}</Match>
              <Match when={step().kind === 'button'}><label class="field-label">{t('操作')}<select value={(step() as {direction:string}).direction} onChange={e => edit({ direction: e.currentTarget.value })}><option value="press">{t('按住')}</option><option value="release">{t('释放')}</option></select></label></Match></Switch>
          </div></Match>
          <Match when={step().kind === 'delay'}>{number('等待毫秒', 'milliseconds', () => (step() as {milliseconds:number}).milliseconds, 0, 10000)}</Match>
        </Switch>
      </div>;
    }}</For>
    <button class="secondary-button" disabled={props.steps.length >= 128} onClick={() => props.onChange([...props.steps, initial('key')])}><Plus size={14}/>{t('添加操作')}</button>
  </div>;
}
