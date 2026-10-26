// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, For, Match, Show, Switch } from 'solid-js';
import { Trash2 } from 'lucide-solid';
import type { HookAction, HookSnapshot } from '../hooks-contracts';
import { newHookAction } from '../hooks-contracts';
import { Row } from './SettingControls';
import { t } from '../i18n';
import HookInputEditor from './HookInputEditor';
import operationCatalog from '../../control/catalog.json';
const operations=operationCatalog.commands.filter(command=>command.execution==='operation'&&command.method!=='chat.send');
const kinds: [HookAction['kind'], string][] = [['injectPrompt', '注入提示词'], ['sendPrompt', '发送提示词'], ['mcp', 'MCP'], ['command', '执行命令'], ['program', '运行程序'], ['openApp', '打开软件'], ['openUrl', '打开网站'], ['input', '键盘与鼠标'], ['script', 'JavaScript'], ['emit', '触发事件'], ['app', 'Mewu 操作'], ['delay', '等待']];
function JsonField(props: { label: string; value: unknown; onChange: (value: unknown) => void; onInvalid: (value: boolean) => void }) {
  const [raw, setRaw] = createSignal(JSON.stringify(props.value, null, 2));
  let accepted = JSON.stringify(props.value);
  createEffect(() => { const next = JSON.stringify(props.value); if (next !== accepted) { accepted = next; setRaw(JSON.stringify(props.value, null, 2)); } });
  return <label class="field-label">{t(props.label)}<textarea class="hooks-code" rows={4} value={raw()} spellcheck={false} onInput={e => {
    const text = e.currentTarget.value; setRaw(text);
    try { const value: unknown = JSON.parse(text); accepted = JSON.stringify(value); e.currentTarget.setCustomValidity(''); props.onInvalid(false); props.onChange(value); }
    catch { e.currentTarget.setCustomValidity(t('JSON 格式无效')); props.onInvalid(true); }
  }}/></label>;
}
export default function HookActionEditor(props: { action: HookAction; index: number; prompt: boolean; data?: HookSnapshot; onChange: (value: HookAction) => void; onRemove: () => void; onInvalid: (value: boolean) => void }) {
  const action = <K extends HookAction['kind']>(kind: K) => props.action as Extract<HookAction, {kind:K}>;
  const edit = (patch: object) => props.onChange({...props.action, ...patch} as HookAction);
  const field = (label: string, key: string, value: () => string, rows = 1) => <label class="field-label">{t(label)}
    <Show when={rows > 1} fallback={<input aria-label={t(label)} value={value()} onInput={e => edit({[key]: key === 'cwd' ? e.currentTarget.value || null : e.currentTarget.value})}/>}>
      <textarea rows={rows} value={value()} spellcheck={false} classList={{'hooks-code':key==='code'}} onInput={e => edit({[key]:e.currentTarget.value})}/>
    </Show></label>;
  return <section class="hooks-action">
    <div class="hooks-action-heading"><select aria-label={t('动作类型')} value={props.action.kind} onChange={e => {props.onInvalid(false);props.onChange(newHookAction(e.currentTarget.value as HookAction['kind'],props.data?.platform));}}>
      <For each={kinds.filter(([kind]) => props.prompt ? kind !== 'sendPrompt' && kind !== 'app' : kind !== 'injectPrompt')}>{([kind,title]) => <option value={kind}>{t(title)}</option>}</For>
    </select><button class="icon-button compact" title={t('删除动作')} aria-label={t('删除动作')} onClick={props.onRemove}><Trash2 size={14}/></button></div>
    <Switch>
      <Match when={props.action.kind==='injectPrompt'||props.action.kind==='sendPrompt'}>{field('提示词','text',()=>(props.action as {text:string}).text,4)}</Match>
      <Match when={props.action.kind==='command'}>
        <Row title={t('运行环境')}><select value={action('command').shell} onChange={e=>edit({shell:e.currentTarget.value})}><option value="powershell">PowerShell</option><Show when={props.data?.platform==='windows'}><option value="cmd">CMD</option></Show><Show when={props.data?.platform!=='windows'}><option value="sh">Shell</option></Show></select></Row>
        {field('命令','code',()=>action('command').code,4)}{field('工作目录','cwd',()=>action('command').cwd||'')}
      </Match>
      <Match when={props.action.kind==='program'}>
        {field('程序路径','executable',()=>action('program').executable)}
        <JsonField label="参数" value={action('program').arguments} onInvalid={props.onInvalid} onChange={v=>{if(Array.isArray(v)&&v.every(x=>typeof x==='string'))edit({arguments:v});else props.onInvalid(true);}}/>
        {field('工作目录','cwd',()=>action('program').cwd||'')}
      </Match>
      <Match when={props.action.kind==='openUrl'}>{field('网址','url',()=>action('openUrl').url)}</Match>
      <Match when={props.action.kind==='openApp'}>{field('应用路径','path',()=>action('openApp').path)}</Match>
      <Match when={props.action.kind==='script'}>{field('JavaScript','code',()=>action('script').code,6)}</Match>
      <Match when={props.action.kind==='input'}><HookInputEditor steps={action('input').steps} onChange={steps=>edit({steps})}/></Match>
      <Match when={props.action.kind==='mcp'}>
        <Row title="MCP"><select value={action('mcp').serverId} onChange={e=>edit({serverId:e.currentTarget.value,tool:''})}><option value=""/><For each={props.data?.mcpServers||[]}>{server=><option value={server.id}>{server.name}</option>}</For></select></Row>
        <Row title={t('工具')}><select value={action('mcp').tool} onChange={e=>edit({tool:e.currentTarget.value})}><option value=""/><For each={props.data?.mcpServers.find(s=>s.id===action('mcp').serverId)?.tools||[]}>{tool=><option value={tool.name}>{tool.name}</option>}</For></select></Row>
        <JsonField label="参数" value={action('mcp').arguments} onInvalid={props.onInvalid} onChange={v=>{if(v&&typeof v==='object'&&!Array.isArray(v))edit({arguments:v});else props.onInvalid(true);}}/>
      </Match>
      <Match when={props.action.kind==='emit'}>
        {field('事件名称','name',()=>action('emit').name)}<JsonField label="事件数据" value={action('emit').payload} onInvalid={props.onInvalid} onChange={v=>edit({payload:v})}/>
      </Match>
      <Match when={props.action.kind==='app'}>
        <label class="field-label">{t('操作')}<select aria-label={t('Mewu 操作')} value={action('app').method} onChange={e=>{props.onInvalid(false);edit({method:e.currentTarget.value,params:{}});}}><Show when={!operations.some(c=>c.method===action('app').method)}><option value={action('app').method}>{action('app').method}</option></Show><For each={operations}>{command=><option value={command.method}>{t(command.title||command.method)}</option>}</For></select></label>
        <Show when={Object.keys(operations.find(c=>c.method===action('app').method)?.inputSchema.properties||{}).some(key=>key!=='context')||Object.keys(action('app').params).length}><JsonField label="参数" value={action('app').params} onInvalid={props.onInvalid} onChange={v=>{if(v&&typeof v==='object'&&!Array.isArray(v))edit({params:v});else props.onInvalid(true);}}/></Show>
      </Match>
      <Match when={props.action.kind==='delay'}><Row title={t('等待毫秒')}><input type="number" min="0" max="10000" value={action('delay').milliseconds} onInput={e=>edit({milliseconds:e.currentTarget.valueAsNumber})}/></Row></Match>
    </Switch>
  </section>;
}
