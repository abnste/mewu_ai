// SPDX-License-Identifier: MPL-2.0
import {createSignal,onMount,onCleanup,Show} from 'solid-js';
import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {Pin,X} from 'lucide-solid';
import ObjectMoveGrip from './components/ObjectMoveGrip';
import {surfaceBridge,type DropTarget} from './extension-surface-bridge';
import {surfacePointerDrag} from './extension-surface-pointer';
import './extension-object.css';
interface State{surfaceId:string;url:string;width:number;height:number;pinned:boolean;owner:{plugin:string;service:string;scene:string;revision:number}}
export default function ExtensionObjectView(){
  const [state,setState]=createSignal<State>(),[error,setError]=createSignal('');
  let frame:HTMLIFrameElement|undefined,connection:ReturnType<typeof surfaceBridge>|undefined,gesture:ReturnType<typeof surfacePointerDrag>|undefined;
  let disposed=false;
  let stopClose:(()=>void)|undefined;
  const stops:Array<()=>void>=[];
  let targets:DropTarget[]=[];
  const publishTargets=()=>{
    if(!frame||disposed)return;
    const rect=frame.getBoundingClientRect();
    void invoke('publish_extension_surface_targets',{targets:targets.map(t=>({...t,x:t.x+rect.left,y:t.y+rect.top}))}).catch(report);
  };
  const report=(cause:unknown)=>setError(cause instanceof Error?cause.message:String(cause));
  const control=(action:string)=>invoke<void>('control_extension_surface_window',{action});
  const close=()=>connection?connection.event('close-request',null):void control('close').catch(report);
  onMount(async()=>{try{
    stopClose=await listen('extension-surface-close-request',close);
    if(disposed){stopClose();return;}
    for(const name of ['tauri://move','tauri://resize','tauri://scale-change']){
      const stop=await listen(name,publishTargets);if(disposed){stop();return;}stops.push(stop);
    }
    const stop=await listen<{name:string;value:unknown}>('extension-surface-event',event=>connection?.event(event.payload.name,event.payload.value));
    if(disposed){stop();return;}stops.push(stop);
    const value=await invoke<State>('get_extension_surface_window');if(!disposed)setState(value);
  }catch(e){report(e);}});
  function loaded(){
    const value=state();if(!frame?.contentWindow||!value||disposed)return;
    connection?.close();const channel=new MessageChannel();
    connection=surfaceBridge(channel.port1,input=>{
      if(input&&typeof input==='object'&&(input as {type?:string}).type==='viewport-point'){
        const {x,y}=input as {x:number;y:number},rect=frame!.getBoundingClientRect();
        if(!Number.isFinite(x)||!Number.isFinite(y))throw Error('无效坐标');
        return invoke('extension_surface_viewport_point',{x:rect.left+x,y:rect.top+y});
      }
      const owner=value.owner;
      return invoke('invoke_extension',{invocation:{pluginId:owner.plugin,revision:owner.revision,serviceId:owner.service,sceneId:owner.scene,surface:{action:'call',id:value.surfaceId},input}});
    },value=>{targets=value;publishTargets();},async input=>{
      if(gesture||!frame)throw Error('拖动不可用');
      const current=surfacePointerDrag(frame,input);gesture=current;
      try{return await current.done;}finally{if(gesture===current)gesture=undefined;}
    },()=>{void control('close').catch(report);},report);
    frame.contentWindow.postMessage({type:'mewu-connect'},'*',[channel.port2]);
    void control('ready').catch(report);
  }
  const key=(e:KeyboardEvent)=>{if(e.key==='Escape'&&!e.isComposing&&e.keyCode!==229&&!e.repeat){e.preventDefault();close();}};
  window.addEventListener('keydown',key);
  onCleanup(()=>{disposed=true;stopClose?.();stops.forEach(stop=>stop());connection?.close();gesture?.cancel();window.removeEventListener('keydown',key);});
  return <main class="extension-object-shell">
    <Show when={state()}>{value=><section class="extension-object-body">
      <iframe ref={frame} src={value().url} sandbox="allow-scripts" allow="" referrerpolicy="no-referrer" title="插件对象" onLoad={loaded}/>
      <ObjectMoveGrip label="移动" onPointerDown={()=>void control('drag').catch(report)}/>
      <div class="extension-object-controls">
        <button title={value().pinned?'取消置顶':'置顶'} aria-label={value().pinned?'取消置顶':'置顶'} classList={{selected:value().pinned}} onClick={async()=>{try{const pinned=!value().pinned;await control(pinned?'pin':'unpin');setState({...value(),pinned});}catch(e){report(e);}}}><Pin size={15}/></button>
        <button title="关闭" aria-label="关闭" onClick={close}><X size={15}/></button>
      </div>
    </section>}</Show>
    <Show when={error()}><div class="extension-object-error" role="status">{error()}</div></Show>
  </main>;
}
