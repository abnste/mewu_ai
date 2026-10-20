// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import * as extensions from '../extension-bridge';
import { surfaceExtensionServices, mergeExtensionRecords, type ExtensionServiceChoice } from '../extension-state';
import type { ExtensionRecord } from '../extension-contracts';
import { registerObjectDrop, type ObjectPoint, type ObjectSource } from '../extension-object-drag';
import { surfaceBridge, type DropTarget } from '../extension-surface-bridge';
import { surfacePointerDrag } from '../extension-surface-pointer';
import PluginIcon from './PluginIcon';
import './extension-surfaces.css';

interface Props { sceneId:string;disabled:boolean;onBeforeSave:()=>Promise<void>;onError:(message:string)=>void }
interface Mounted {surfaceId:string;url:string;width:number;height:number}
export default function ExtensionSurfaces(props:Props) {
  const [records,setRecords]=createSignal<ExtensionRecord[]>([]);
  const [widths,setWidths]=createSignal<Record<string,number>>({});
  let disposed=false,serial=0,stop:(()=>void)|undefined;
  if(extensions.extensionNative)void(async()=>{
    try{
      stop=await extensions.subscribeExtensions(value=>{serial++;if(!disposed)setRecords(old=>mergeExtensionRecords(old,value));});
      if(disposed){stop();return;}
      const before=serial,value=await extensions.getExtensions();
      if(!disposed&&before===serial)setRecords(value);
    }catch(error){if(!disposed)props.onError(String(error));}
  })();
  onCleanup(()=>{disposed=true;stop?.();});
  const choices=createMemo(()=>surfaceExtensionServices(records()));
  const key=(choice:ExtensionServiceChoice)=>`${choice.record.manifest.id}/${choice.service.id}`;
  // Registry snapshots allocate new records. Identity must follow the service
  // generation, not the containing object, or changing one plugin closes all.
  const generation=(choice:ExtensionServiceChoice)=>JSON.stringify([choice.record.manifest.id,choice.service.id,choice.record.revision]);
  const generations=createMemo(()=>choices().map(generation));
  const offset=(choice:ExtensionServiceChoice)=>{
    let x=16;for(const entry of choices()){if(key(entry)===key(choice))break;const width=widths()[key(entry)];if(width)x+=width+12;}return x;
  };
  return <For each={generations()}>{id=>{
    const choice=choices().find(value=>generation(value)===id)!;
    return <Show when={props.sceneId} keyed>{_identity=> <Surface {...props} choice={choice} left={offset(choice)}
      onWidth={width=>setWidths(old=>({...old,[key(choice)]:width}))}/>}</Show>;
  }}</For>;
}
function Surface(props:Props&{choice:ExtensionServiceChoice;left:number;onWidth:(width:number)=>void}) {
  const [open,setOpen]=createSignal(false),[mounted,setMounted]=createSignal<Mounted>(),[dragging,setDragging]=createSignal(false);
  let frame:HTMLIFrameElement|undefined,disposed=false,opening:Promise<void>|undefined;
  let trigger:HTMLButtonElement|undefined;
  let connection:ReturnType<typeof surfaceBridge>|undefined,targets:DropTarget[]=[];
  let busy=false;
  let pointerGesture:ReturnType<typeof surfacePointerDrag>|undefined;
  const identity={pluginId:props.choice.record.manifest.id,revision:props.choice.record.revision,serviceId:props.choice.service.id,sceneId:props.sceneId};
  const report=(error:unknown)=>{if(!disposed)props.onError(error instanceof Error?error.message:String(error));};
  const request=async(input:unknown,objectSource?:ObjectSource)=>{
    const surface=mounted();
    if(disposed||props.disabled||!open()||!surface)throw Error('插件界面已关闭');
    if(busy)throw Error('插件正在处理');
    busy=true;
    try{
      // Flush only once on actions, never poll the drawing/selection hot path.
      await Promise.resolve();await props.onBeforeSave();
      if(disposed||props.disabled||!open()||mounted()?.surfaceId!==surface.surfaceId)throw Error('插件界面已变化');
      const result=await extensions.invokeExtension({...identity,surface:{action:'call',id:surface.surfaceId},input,...(objectSource?{objectSource}:{})});
      return result;
    }finally{busy=false;}
  };
  function close(){
    pointerGesture?.cancel();pointerGesture=undefined;
    connection?.close();connection=undefined;targets=[];
    const value=mounted();setMounted(undefined);setOpen(false);
    props.onWidth(0);
    if(value)void extensions.invokeExtension({...identity,surface:{action:'close',id:value.surfaceId},input:null}).catch(()=>{});
  }
  async function show(){
    if(disposed||props.disabled)return;
    setOpen(true);
    if(mounted()||opening)return opening;
    opening=(async()=>{
      try{
        const result=await extensions.invokeExtension({...identity,surface:{action:'mount'},input:{type:'mount'}}) as Mounted;
        if(!result||typeof result.surfaceId!=='string'||typeof result.url!=='string'
          ||!/^http:\/\/127\.0\.0\.1:\d+\/[a-f0-9-]{36}$/.test(result.url)
          ||!Number.isInteger(result.width)||!Number.isInteger(result.height))throw Error('插件界面返回无效');
        if(disposed||!open()){
          void extensions.invokeExtension({...identity,surface:{action:'close',id:result.surfaceId},input:null}).catch(()=>{});return;
        }
        setMounted(result);
        props.onWidth(Math.min(result.width,innerWidth-32));
      }catch(error){close();report(error);}finally{opening=undefined;}
    })();
    return opening;
  }
  const hit=(point:ObjectPoint)=>{
    const rect=frame?.getBoundingClientRect();
    if(!open()||!rect||point.x<rect.left||point.x>rect.right||point.y<rect.top||point.y>rect.bottom)return;
    const x=point.x-rect.left,y=point.y-rect.top;
    return [...targets].reverse().find(t=>x>=t.x&&y>=t.y&&x<t.x+t.width&&y<t.y+t.height)?.id;
  };
  const unregister=registerObjectDrop({begin(source){
    let live=true;
    setDragging(true);
    return {
      move(point){
        if(!live)return;
        // Hovering a specific entry may reveal it. Moving any canvas object
        // must not start every installed UI plugin.
        const rect=trigger?.getBoundingClientRect();
        if(!open()&&rect&&point.x>=rect.left&&point.x<rect.right&&point.y>=rect.top&&point.y<rect.bottom)void show();
        connection?.event('drag',{target:hit(point)??null,kind:source.source.kind});
      },
      finish(point){
        const generation=mounted()?.surfaceId;
        const target=hit(point);const accepts=live&&!disposed&&!props.disabled&&!busy&&!!target;
        live=false;setDragging(false);connection?.event('drag',null);
        if(!accepts)return false;
        void request({type:'drop',target,kind:source.source.kind},source.source).then(async result=>{
          if(!disposed&&!props.disabled&&open()&&generation===mounted()?.surfaceId&&result&&typeof result==='object'&&(result as {removeSource?:boolean}).removeSource===true)await source.remove();
          connection?.event('change',result);
        }).catch(report);
        return true;
      },
      cancel(){live=false;setDragging(false);connection?.event('drag',null);}
    };
  }});
  createEffect(()=>{if(props.disabled)close();});
  onCleanup(()=>{disposed=true;unregister();close();});
  function loaded(){
    connection?.close();
    if(!frame?.contentWindow||!mounted()||disposed)return;
    const channel=new MessageChannel();
    connection=surfaceBridge(channel.port1,async input=>{
      // Coordinate conversion is host mechanism, never a plugin business rule.
      if(input&&typeof input==='object'&&(input as {type?:string}).type==='viewport-point'){
        const {x,y}=input as {x:number;y:number};const rect=frame!.getBoundingClientRect();
        if(!Number.isFinite(x)||!Number.isFinite(y))throw Error('无效坐标');
        return {x:Math.max(0,Math.min(.95,(rect.left+x)/innerWidth)),y:Math.max(0,Math.min(.95,(rect.top+y)/innerHeight))};
      }
      return request(input);
    },value=>{targets=value;},async()=>{
      if(disposed||props.disabled||!open()||!frame||pointerGesture)throw Error('拖动不可用');
      const gesture=surfacePointerDrag(frame);pointerGesture=gesture;
      try{return await gesture.done;}finally{if(pointerGesture===gesture)pointerGesture=undefined;}
    });
    frame.contentWindow.postMessage({type:'mewu-connect'},'*',[channel.port2]);
  }
  return <>
    <button ref={trigger} class="icon-button" classList={{selected:open()}} title={props.choice.service.title} aria-label={props.choice.service.title} aria-expanded={open()} disabled={props.disabled}
      onPointerDown={e=>e.stopPropagation()} onClick={()=>open()?close():void show()}><PluginIcon plugin={props.choice.record.manifest} sha256={props.choice.record.sha256} source={props.choice.record.source} size={17}/></button>
    <Show when={mounted()}>{value=><Portal><iframe ref={frame} class="extension-surface-frame" classList={{'receiving-object':dragging()}} sandbox="allow-scripts" allow="" referrerpolicy="no-referrer"
      onPointerDown={e=>e.stopPropagation()} onWheel={e=>e.stopPropagation()}
      title={props.choice.service.title} src={value().url} style={{width:`${value().width}px`,height:`${value().height}px`,left:`${Math.max(16,Math.min(props.left,innerWidth-value().width-16))}px`}} onLoad={loaded}/></Portal>}</Show>
  </>;
}
