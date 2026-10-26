// SPDX-License-Identifier: MPL-2.0
import {createEffect,createMemo,createSignal,For,onCleanup,onMount,Show} from 'solid-js';
import {Check,LoaderCircle,X} from 'lucide-solid';
import * as bridge from '../hooks-bridge';
import * as shortcut from '../shortcut-bridge';
import {formatShortcut} from '../capture-shortcut-editor';
import type {HookRule,HookSnapshot} from '../hooks-contracts';
import {formatHookShortcut,hookAccelerator,newHookRule} from '../hooks-contracts';
import {t} from '../i18n';
import '../capture-shortcut.css';
/** Inline editing of ordinary rules, with the same CAS and native recording
 * lease as Hooks. No second preference, default identity or execution route. */
export default function HookShortcutSetting(props:{active:boolean;enabled:boolean;method:string;label:string}){
  const [view,setView]=createSignal<HookSnapshot>(),[error,setError]=createSignal('');
  const [draft,setDraft]=createSignal<{base:number;rule:HookRule;value:string}>();
  const [recording,setRecording]=createSignal(''),[pending,setPending]=createSignal(false);
  let disposed=false,stop:(()=>void)|undefined,stopShortcuts:(()=>void)|undefined,flight=false,dirty=false,leaseId:string|undefined;
  const available=()=>props.active&&props.enabled&&Boolean(view()?.enabled)&&!pending();
  const accept=(v:HookSnapshot)=>{if(!disposed)setView(old=>!old||v.revision>=old.revision?v:old);};
  async function reload(){if(disposed||!props.active||!props.enabled)return;if(flight){dirty=true;return;}flight=true;try{do{dirty=false;accept(await bridge.getHooks());}while(dirty&&!disposed);}catch(e){if(!disposed)setError(String(e));}finally{flight=false;}}
  const rules=createMemo(()=>view()?.rules.filter(r=>r.trigger.kind==='shortcut'&&r.actions.some(a=>a.kind==='app'&&a.method===props.method))||[]);
  // Empty is a local placeholder, never an automatically persisted rule.
  const ids=createMemo(()=>rules().length?rules().map(r=>r.id):['']);
  const rule=(id:string)=>rules().find(r=>r.id===id);
  const editing=(id:string)=>Boolean(draft()&&(draft()!.rule.id===id||(!id&&draft()!.rule.revision===0)));
  const value=(id:string)=>editing(id)?draft()!.value:(rule(id)?.trigger as {accelerator?:string}|undefined)?.accelerator||'';
  const displayed=(id:string)=>formatHookShortcut(editing(id)?value(id):view()?.shortcuts[id]?.label||value(id),view()?.platform);
  function record(value:string){if(available()&&recording()&&draft())setDraft(d=>d?{...d,value}:d);}
  async function endLease(id:string){await Promise.allSettled([bridge.editHookShortcut(id,false),shortcut.endCaptureShortcutEdit(id)]);}
  async function disarm(){const id=leaseId;leaseId=undefined;setRecording('');if(id)await endLease(id);}
  async function arm(id:string){
    if(!available())return;
    // Invalidate synchronously; a late begin receipt must not rearm this field.
    void disarm();
    if(!editing(id)){
      const r=rule(id)||{...newHookRule('shortcut',view()?.platform),name:props.label,actions:[{kind:'app' as const,method:props.method,params:{}}],timeoutMs:30000};
      setDraft({base:view()!.revision,rule:r,value:r.trigger.kind==='shortcut'?r.trigger.accelerator:''});
    }
    const lease=crypto.randomUUID();leaseId=lease;
    try{
      await shortcut.beginCaptureShortcutEdit(lease);
      if(disposed||!available()||leaseId!==lease){await endLease(lease);return;}
      await bridge.editHookShortcut(lease,true);
      if(disposed||!available()||leaseId!==lease){await endLease(lease);return;}
      setRecording(draft()!.rule.id);
    }catch(e){if(leaseId===lease)await disarm();else await endLease(lease);if(!disposed)setError(String(e));}
  }
  function cancel(){void disarm();setDraft(undefined);setError('');}
  async function save(){
    const d=draft();if(!d||!available())return;
    setPending(true);setError('');
    try{
      await disarm();if(disposed||!props.active||!props.enabled||!view()?.enabled)return;
      if(!d.value){if(d.rule.revision)accept(await bridge.removeHook(d.base,d.rule.id,d.rule.revision));}
      else accept(await bridge.saveHook(d.base,{...d.rule,trigger:{kind:'shortcut',accelerator:d.value}}));
      if(!disposed&&draft()===d)setDraft(undefined);
    }catch(e){if(!disposed){setError(String(e));void reload();}}finally{if(!disposed)setPending(false);}
  }
  function keyboard(e:KeyboardEvent){
    if(e.isComposing)return;
    if(e.key==='Tab'){void disarm();return;}
    e.preventDefault();e.stopPropagation();
    if(e.key==='Escape'){cancel();(e.currentTarget as HTMLInputElement).blur();return;}
    if(!recording())return;
    if((e.key==='Delete'||e.key==='Backspace')&&!e.ctrlKey&&!e.altKey&&!e.metaKey&&!e.shiftKey){record('');return;}
    const key=hookAccelerator(e);if(key)record(key);
  }
  const received=(id:string,key:string)=>{if(id===leaseId&&document.activeElement?.matches('[data-operation-shortcut]'))record(key);};
  const blur=()=>void disarm();
  onMount(async()=>{try{
    stop=await bridge.subscribeHooks(()=>void reload(),received);if(disposed){stop();return;}
    stopShortcuts=await shortcut.subscribeCaptureShortcut({state:()=>{},recorded:r=>received(r.leaseId,formatShortcut(r.shortcut).replace(/Ctrl/g,'Control')),ended:id=>{if(id===leaseId)void disarm();}});
    if(disposed){stopShortcuts();return;}void reload();
  }catch(e){if(!disposed)setError(String(e));}});
  onMount(()=>window.addEventListener('blur',blur));
  createEffect(()=>{if(props.active&&props.enabled)void reload();else cancel();});
  createEffect(()=>{if(view()&&!view()!.enabled)cancel();});
  onCleanup(()=>{disposed=true;void disarm();stop?.();stopShortcuts?.();window.removeEventListener('blur',blur);});
  return <div class="hook-shortcut-setting capture-shortcut-field"><For each={ids()}>{id=><div class="capture-shortcut-controls">
    <input data-operation-shortcut readOnly aria-label={t(props.label)} classList={{recording:Boolean(recording())&&editing(id),'hook-shortcut-disabled':rule(id)?.enabled===false}} value={displayed(id)} placeholder={t(props.enabled?'未设置':'插件未启用')} title={view()?.shortcuts[id]?.message||t('按组合键，Delete 清空')} disabled={!available()} onFocus={()=>void arm(id)} onBlur={()=>void disarm()} onKeyDown={keyboard}/>
    <Show when={editing(id)}><button class="icon-button compact" aria-label={t('保存快捷键')} title={t('保存')} disabled={pending()} onClick={()=>void save()}><Show when={pending()} fallback={<Check size={15}/>}><LoaderCircle size={14} class="spin"/></Show></button><button class="icon-button compact" aria-label={t('取消快捷键修改')} title={t('取消')} disabled={pending()} onClick={cancel}><X size={14}/></button></Show>
  </div>}</For><For each={rules()}>{r=><Show when={view()?.shortcuts[r.id]?.message}><span class="capture-shortcut-error" role="status">{view()!.shortcuts[r.id].message}</span></Show>}</For><Show when={error()&&props.enabled}><span class="capture-shortcut-error" role="alert">{error()}</span></Show></div>;
}
