// SPDX-License-Identifier: MPL-2.0
import {createEffect,createMemo,createSignal,For,onCleanup,onMount,Show} from 'solid-js';
import {getHooks,subscribeHooks} from '../hooks-bridge';
import type {HookSnapshot} from '../hooks-contracts';
import {formatHookShortcut} from '../hooks-contracts';
import {t} from '../i18n';
/** A settings link into ordinary rules that invoke an operation. No alternate
 * preference, rule identity, default restoration, or execution route exists. */
export default function HookShortcutSetting(props:{active:boolean;enabled:boolean;method:string;onOpen:(id?:string)=>void}){
  const [view,setView]=createSignal<HookSnapshot>(),[error,setError]=createSignal('');
  let disposed=false,stop:(()=>void)|undefined,flight=false;
  const reload=async()=>{if(disposed||!props.active||!props.enabled||flight)return;flight=true;try{const v=await getHooks();if(!disposed){setView(v);setError('');}}catch(e){if(!disposed)setError(String(e));}finally{flight=false;}};
  const rules=createMemo(()=>view()?.rules.filter(r=>r.trigger.kind==='shortcut'&&r.actions.some(a=>a.kind==='app'&&a.method===props.method))||[]);
  onMount(async()=>{try{const off=await subscribeHooks(()=>void reload(),()=>{});if(disposed)off();else stop=off;}catch(e){if(!disposed)setError(String(e));}});
  createEffect(()=>{if(props.active&&props.enabled)void reload();});
  onCleanup(()=>{disposed=true;stop?.();});
  return <div class="hook-shortcut-setting"><Show when={rules().length&&props.enabled} fallback={<button class="text-button" disabled={!props.enabled} onClick={()=>props.onOpen()}>{t(props.enabled?'未设置':'插件未启用')}</button>}><For each={rules()}>{rule=><button class="text-button" classList={{'hook-shortcut-disabled':!rule.enabled}} onClick={()=>props.onOpen(rule.id)}>{formatHookShortcut(view()?.shortcuts[rule.id]?.label||(rule.trigger.kind==='shortcut'?rule.trigger.accelerator:''),view()?.platform)}</button>}</For></Show><Show when={error()&&props.enabled}><span class="settings-error">{error()}</span></Show></div>;
}
