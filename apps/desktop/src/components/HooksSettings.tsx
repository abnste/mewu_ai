// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, LoaderCircle, Play, Plus, Square, Trash2 } from 'lucide-solid';
import { Row, SettingsAddButton, Toggle } from './SettingControls';
import CaptureShortcutField from './CaptureShortcutField';
import SystemCaptureShortcutSetting from './SystemCaptureShortcutSetting';
import { settingsApi } from '../settings-bridge';
import { acceptShortcutState } from '../capture-shortcut-editor';
import type { CaptureShortcutState } from '../shortcut-contracts';
import HookActionEditor from './HookActionEditor';
import * as bridge from '../hooks-bridge';
import * as shortcut from '../shortcut-bridge';
import { formatShortcut } from '../capture-shortcut-editor';
import type { HookCategory, HookRule, HookSnapshot, HookTrigger } from '../hooks-contracts';
import { hookAccelerator, newHookAction, newHookRule } from '../hooks-contracts';
import { t } from '../i18n';
import '../connections.css';
import '../hooks.css';

const tabs: [HookCategory,string][] = [['cron','定时触发'],['prompt','提示词触发'],['reply','AI 回复触发'],['shortcut','快捷键触发'],['event','事件触发']];
const statusText: Record<string,string> = {running:'运行中',completed:'已完成',failed:'失败',canceled:'已停止',skipped:'条件未满足',interrupted:'运行中断'};
export default function HooksSettings(props:{active:boolean}) {
  const [data,setData]=createSignal<HookSnapshot>();
  const [tab,setTab]=createSignal<HookCategory>('cron');
  const [drafts,setDrafts]=createSignal<Record<string,HookRule>>({});
  const [bases,setBases]=createSignal<Record<string,number>>({});
  const [expanded,setExpanded]=createSignal('');
  const [pending,setPending]=createSignal('');
  const [error,setError]=createSignal('');
  const [checked,setChecked]=createSignal('');
  const [invalid,setInvalid]=createSignal<Record<string,boolean>>({});
  const [testText,setTestText]=createSignal<Record<string,string>>({});
  const [recording,setRecording]=createSignal('');
  const [captureState,setCaptureState]=createSignal<CaptureShortcutState>();
  const acceptCapture=(value:CaptureShortcutState)=>setCaptureState(old=>acceptShortcutState(old,value));
  let disposed=false,refreshing=false,dirty=false,stops:(()=>void)|undefined,stopShortcuts:(()=>void)|undefined;
  let editLease:string|undefined;
  const ids=createMemo(()=>[...new Set([...(data()?.rules||[]).filter(r=>r.trigger.kind===tab()).map(r=>r.id),...Object.values(drafts()).filter(r=>r.trigger.kind===tab()).map(r=>r.id)])]);
  const rule=(id:string)=>drafts()[id]||data()?.rules.find(r=>r.id===id);
  function edit(id:string,patch:Partial<HookRule>){const current=rule(id);if(!current)return;if(!(id in bases()))setBases(v=>({...v,[id]:data()?.revision||0}));setDrafts(v=>({...v,[id]:{...current,...patch}}));setChecked('');}
  function discard(id:string){setDrafts(v=>{const n={...v};delete n[id];return n;});setBases(v=>{const n={...v};delete n[id];return n;});setInvalid(v=>Object.fromEntries(Object.entries(v).filter(([key])=>!key.startsWith(id+':'))));}
  async function refresh(){if(disposed)return;if(refreshing){dirty=true;return;}refreshing=true;try{do{dirty=false;const value=await bridge.getHooks();if(!disposed)setData(v=>!v||value.revision>=v.revision?value:v);}while(dirty&&!disposed);}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}finally{refreshing=false;}}
  function received(leaseId:string,accelerator:string){const id=recording();if(leaseId===editLease&&id&&document.activeElement?.matches('[data-hook-shortcut]'))edit(id,{trigger:{kind:'shortcut',accelerator}});}
  function recordingEnded(leaseId:string){if(leaseId!==editLease)return;void disarm();const field=document.activeElement as HTMLElement|null;if(field?.matches('[data-hook-shortcut]'))field.blur();}
  async function endLease(id:string){await Promise.allSettled([bridge.editHookShortcut(id,false),shortcut.endCaptureShortcutEdit(id)]);}
  async function disarm(){const id=editLease;editLease=undefined;setRecording('');if(id)await endLease(id);}
  async function arm(id:string){
    await disarm();if(disposed||!props.active)return;
    const lease=crypto.randomUUID();editLease=lease;
    try{
      await shortcut.beginCaptureShortcutEdit(lease);
      if(editLease!==lease||disposed||!props.active){await endLease(lease);return;}
      await bridge.editHookShortcut(lease,true);
      if(editLease!==lease||disposed||!props.active){await endLease(lease);return;}
      setRecording(id);
    }catch(e){if(editLease===lease)await disarm();else await endLease(lease);if(!disposed)setError(e instanceof Error?e.message:String(e));}
  }
  onMount(async()=>{try{stops=await bridge.subscribeHooks(()=>{if(props.active)void refresh();else dirty=true;},received);if(disposed){stops();return;}stopShortcuts=await shortcut.subscribeCaptureShortcut({state:acceptCapture,recorded:value=>received(value.leaseId,formatShortcut(value.shortcut).replace(/Ctrl/g,'Control')),ended:recordingEnded});if(disposed){stopShortcuts();return;}await refresh();if(!disposed)acceptCapture(await shortcut.getCaptureShortcut());}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}});
  const blur=()=>void disarm();onMount(()=>window.addEventListener('blur',blur));
  // MCP discovery/enablement changes outside the Hooks rule revision. Reload
  // metadata on each entry while preserving unsaved rule drafts.
  createEffect(()=>{if(!props.active)void disarm();else void refresh();});
  onCleanup(()=>{disposed=true;void disarm();stops?.();stopShortcuts?.();window.removeEventListener('blur',blur);});
  function add(){const next=newHookRule(tab(),data()?.platform);setDrafts(v=>({...v,[next.id]:next}));setBases(v=>({...v,[next.id]:data()?.revision||0}));setExpanded(next.id);setError('');}
  async function save(id:string,override?:HookRule){const r=override||drafts()[id];if(!r||pending())return;setPending(id);setError('');try{await disarm();const value=await bridge.saveHook(bases()[id]??data()?.revision??0,r);if(!disposed){setData(value);discard(id);}}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}finally{if(!disposed)setPending('');}}
  async function remove(id:string){const r=data()?.rules.find(r=>r.id===id);if(!r){discard(id);setExpanded('');return;}if(pending())return;setPending(id);setError('');try{const value=await bridge.removeHook(data()!.revision,id,r.revision);if(!disposed){setData(value);discard(id);setExpanded('');}}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}finally{if(!disposed)setPending('');}}
  async function run(id:string){const r=data()?.rules.find(r=>r.id===id);if(!r||drafts()[id]||pending())return;setPending(id);setError('');try{await bridge.runHook(id,r.revision,testText()[id]||'');await refresh();}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}finally{if(!disposed)setPending('');}}
  async function check(id:string){const r=rule(id);if(!r)return;setPending(id);setError('');try{await bridge.checkHook(r);if(!disposed)setChecked(id);}catch(e){if(!disposed)setError(e instanceof Error?e.message:String(e));}finally{if(!disposed)setPending('');}}
  async function removeCapture(){setPending('capture');setError('');try{const s=await shortcut.getCaptureShortcut();acceptCapture(await shortcut.setCaptureShortcut(s.revision,null));}catch(e){setError(e instanceof Error?e.message:String(e));}finally{setPending('');}}
  const description=(r:HookRule)=>{const x=r.trigger;if(x.kind==='cron')return `${x.expression} · ${x.timezone}`;if('matcher' in x)return x.matcher.value;if(x.kind==='shortcut')return data()?.shortcuts[r.id]?.label||x.accelerator;return x.name;};
  return <div class="hooks-settings">
    <header class="settings-page-heading"><h2>{t('事件触发器')}</h2><SettingsAddButton disabled={!data()?.enabled||Boolean(pending())} onClick={add}/></header>
    <div class="segmented"><For each={tabs}>{([id,title])=><button aria-pressed={tab()===id} classList={{selected:tab()===id}} onClick={()=>{void disarm();setTab(id);setExpanded('');}}>{t(title)}</button>}</For></div>
    <Show when={error()}><p class="settings-error" role="alert">{error()}</p></Show>
    <Show when={tab()==='shortcut'}><div class="connection-list"><article class="connection-card"><div class="connection-summary"><button class="connection-expand" aria-expanded={expanded()==='capture'} onClick={()=>setExpanded(expanded()==='capture'?'':'capture')}><Show when={expanded()==='capture'} fallback={<ChevronRight size={14}/>}><ChevronDown size={14}/></Show><span><strong>{t('唤起截图')}</strong><small>{captureState()?.actualTriggerDescription||(captureState()?.configured?formatShortcut(captureState()!.configured!):'')}</small></span></button><button class="icon-button compact" aria-label={t('删除截图快捷键')} disabled={Boolean(pending())} onClick={()=>void removeCapture()}><Trash2 size={14}/></button></div><Show when={expanded()==='capture'}><div class="connection-editor"><CaptureShortcutField visible={props.active&&tab()==='shortcut'&&expanded()==='capture'}/></div></Show></article></div></Show>
    <Show when={tab()==='shortcut'&&data()?.platform==='windows'}><SystemCaptureShortcutSetting active={props.active&&tab()==='shortcut'} api={settingsApi}/></Show>
    <div class="connection-list"><For each={ids()}>{id=>{const current=()=>rule(id)!;const invalidRule=()=>Object.entries(invalid()).some(([key,value])=>key.startsWith(id+':')&&value);const runs=()=>data()?.runs.filter(r=>r.ruleId===id)||[];const active=()=>runs().find(r=>r.status==='running');
      const setTrigger=(value:HookTrigger)=>edit(id,{trigger:value});return <article class="connection-card hooks-rule" classList={{expanded:expanded()===id}}>
      <div class="connection-summary"><button class="connection-expand" aria-expanded={expanded()===id} onClick={()=>{void disarm();setExpanded(expanded()===id?'':id);}}><Show when={expanded()===id} fallback={<ChevronRight size={14}/>}><ChevronDown size={14}/></Show><span><strong>{current().name||t('新建事件')}</strong><small>{description(current())}</small></span></button><Toggle checked={current().enabled} label={t('启用规则')} disabled={Boolean(pending())} onChange={enabled=>{if(drafts()[id])edit(id,{enabled});else void save(id,{...current(),enabled});}}/></div>
      <Show when={expanded()===id}><fieldset class="connection-editor" disabled={Boolean(pending())}>
        <label class="field-label">{t('名称')}<input value={current().name} maxLength={120} onInput={e=>edit(id,{name:e.currentTarget.value})}/></label>
        <Show when={current().trigger.kind==='cron'}><div class="connection-advanced-row"><label class="field-label">Cron<input value={(current().trigger as Extract<HookTrigger,{kind:'cron'}>).expression} onInput={e=>setTrigger({...current().trigger,expression:e.currentTarget.value} as HookTrigger)}/></label><label class="field-label">{t('时区')}<input list="hooks-timezones" value={(current().trigger as Extract<HookTrigger,{kind:'cron'}>).timezone} onInput={e=>setTrigger({...current().trigger,timezone:e.currentTarget.value} as HookTrigger)}/><datalist id="hooks-timezones"><option value="local">{t('系统时区')}</option><option value="UTC"/><option value="Asia/Shanghai"/><option value="Asia/Singapore"/></datalist></label></div><Show when={data()?.nextRuns[id]}><small class="settings-muted">{t('下次执行')} {new Date(data()!.nextRuns[id]).toLocaleString()}</small></Show></Show>
        <Show when={current().trigger.kind==='prompt'||current().trigger.kind==='reply'}><Row title={t('匹配方式')}><select value={(current().trigger as Extract<HookTrigger,{matcher:unknown}>).matcher.mode} onChange={e=>{const tr=current().trigger as Extract<HookTrigger,{matcher:unknown}>;setTrigger({...tr,matcher:{...tr.matcher,mode:e.currentTarget.value as 'contains'|'exact'|'regex'}});}}><option value="contains">{t('包含')}</option><option value="exact">{t('完全匹配')}</option><option value="regex">{t('正则表达式')}</option></select></Row><label class="field-label">{t('匹配内容')}<input value={(current().trigger as Extract<HookTrigger,{matcher:unknown}>).matcher.value} onInput={e=>{const tr=current().trigger as Extract<HookTrigger,{matcher:unknown}>;setTrigger({...tr,matcher:{...tr.matcher,value:e.currentTarget.value}});}}/></label><Row title={t('忽略大小写')}><Toggle checked={(current().trigger as Extract<HookTrigger,{matcher:unknown}>).matcher.ignoreCase} label={t('忽略大小写')} onChange={ignoreCase=>{const tr=current().trigger as Extract<HookTrigger,{matcher:unknown}>;setTrigger({...tr,matcher:{...tr.matcher,ignoreCase}});}}/></Row></Show>
        <Show when={current().trigger.kind==='shortcut'}><label class="field-label">{t('全局快捷键')}<input data-hook-shortcut readOnly value={(current().trigger as Extract<HookTrigger,{kind:'shortcut'}>).accelerator} placeholder={t('按下快捷键')} onFocus={()=>void arm(id)} onBlur={()=>void disarm()} onKeyDown={e=>{if(e.isComposing)return;if(e.key==='Tab'){void disarm();return;}e.preventDefault();e.stopPropagation();if(e.key==='Escape'){void disarm();e.currentTarget.blur();return;}const accelerator=hookAccelerator(e);if(accelerator)setTrigger({kind:'shortcut',accelerator});}}/></label><Show when={data()?.shortcuts[id]?.message}><p class="settings-error">{data()!.shortcuts[id].message}</p></Show></Show>
        <Show when={current().trigger.kind==='event'}><label class="field-label">{t('事件')}<input list={`hook-events-${id}`} value={(current().trigger as Extract<HookTrigger,{kind:'event'}>).name} onInput={e=>setTrigger({kind:'event',name:e.currentTarget.value})}/><datalist id={`hook-events-${id}`}><For each={data()?.events}>{event=><option value={event.name}>{t(event.title)}</option>}</For></datalist></label></Show>
        <details class="hooks-condition"><summary>{t('附加条件')}</summary><textarea rows={3} class="hooks-code" value={current().condition} placeholder="return event.text.includes('？');" spellcheck={false} onInput={e=>edit(id,{condition:e.currentTarget.value})}/></details>
        <div class="settings-section-heading"><h3>{t('动作')}</h3><button class="icon-button compact" title={t('添加动作')} aria-label={t('添加动作')} disabled={current().actions.length>=16} onClick={()=>edit(id,{actions:[...current().actions,newHookAction(current().trigger.kind==='prompt'?'injectPrompt':'command',data()?.platform)]})}><Plus size={14}/></button></div>
        <For each={current().actions.map((_,i)=>i)}>{index=><HookActionEditor action={current().actions[index]} index={index} prompt={current().trigger.kind==='prompt'} data={data()} onChange={value=>edit(id,{actions:current().actions.map((a,i)=>i===index?value:a)})} onRemove={()=>{edit(id,{actions:current().actions.filter((_,i)=>i!==index)});setInvalid(values=>Object.fromEntries(Object.entries(values).flatMap(([key,value])=>{if(!key.startsWith(id+':'))return [[key,value]];const position=Number(key.slice(id.length+1));return position===index?[]:[[(id+':'+(position>index?position-1:position)),value]];})));}} onInvalid={value=>setInvalid(v=>({...v,[`${id}:${index}`]:value}))}/>}</For>
        <details><summary>{t('执行选项')}</summary><Row title={t('冷却毫秒')}><input type="number" min="0" max="86400000" value={current().cooldownMs} onInput={e=>edit(id,{cooldownMs:e.currentTarget.valueAsNumber})}/></Row><Row title={t('超时毫秒')}><input type="number" min="100" max="60000" value={current().timeoutMs} onInput={e=>edit(id,{timeoutMs:e.currentTarget.valueAsNumber})}/></Row><Row title={t('失败后停止')}><Toggle label={t('失败后停止')} checked={current().stopOnError} onChange={stopOnError=>edit(id,{stopOnError})}/></Row></details>
        <div class="settings-actions"><button class="icon-button compact" aria-label={t('删除规则')} title={t('删除规则')} onClick={()=>void remove(id)}><Trash2 size={14}/></button><span class="hooks-actions-spacer"/><Show when={checked()===id}><Check size={14}/></Show><button class="secondary-button" disabled={invalidRule()} onClick={()=>void check(id)}>{t('校验')}</button><Show when={drafts()[id]}><button class="secondary-button" onClick={()=>discard(id)}>{t('取消')}</button><button class="primary-button" disabled={invalidRule()||!current().name.trim()||current().actions.length===0} onClick={()=>void save(id)}><Show when={pending()===id}><LoaderCircle size={13} class="spin"/></Show>{t('保存')}</button></Show></div>
      </fieldset><Show when={!drafts()[id]}><div class="hooks-test"><Show when={current().trigger.kind==='prompt'||current().trigger.kind==='reply'}><input aria-label={t('测试内容')} placeholder={t('测试内容')} value={testText()[id]||''} onInput={e=>setTestText(v=>({...v,[id]:e.currentTarget.value}))}/></Show><Show when={active()} fallback={<button class="secondary-button" disabled={Boolean(pending())||!current().enabled} onClick={()=>void run(id)}><Play size={13}/>{t('运行一次')}</button>}>{running=><button class="secondary-button" onClick={()=>void bridge.cancelHook(running().id).catch(e=>setError(String(e)))}><Square size={13}/>{t('停止')}</button>}</Show></div></Show><Show when={runs().length}><details class="hooks-runs"><summary>{t('最近运行')}</summary><For each={runs().slice(0,8)}>{run=><div class="hooks-run"><time>{new Date(run.startedAt).toLocaleTimeString()}</time><span>{t(statusText[run.status]||run.status)}</span><small>{run.completedActions}/{current().actions.length}</small><Show when={run.error}><p class="settings-error">{run.error}</p></Show></div>}</For></details></Show></Show>
    </article>;}}</For></div>
  </div>;
}
