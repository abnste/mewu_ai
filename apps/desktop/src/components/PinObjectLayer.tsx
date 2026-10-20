// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Link2, X } from 'lucide-solid';
import { t } from '../i18n';
import { controlPinObject, type PinObject } from '../pin-objects';
import './pin-objects.css';
import {moveImageObject} from '../image-object-geometry';
import { beginObjectDrag, type ObjectDrag } from '../extension-object-drag';
import PinVideo from './PinVideo';
interface Props { aiAvailable?: boolean; objects: PinObject[]; busy: boolean; exiting?: boolean; referenced: (object: PinObject) => boolean; onReference: (object: PinObject) => void; onError: (error: unknown) => void }
function ImageObject(props: Props & { object: PinObject }) {
  const [position, setPosition] = createSignal({ x: props.object.x, y: props.object.y });
  let moving = false, disposed=false, zooming=false,zoomSteps=0, intentGeneration=0, cleanup: (() => void) | undefined;
  createEffect(() => { const x=props.object.x,y=props.object.y; if(!moving)setPosition({x,y}); });
  function cancelPendingIntent() {
    intentGeneration++;zoomSteps=0;cleanup?.();moving=false;
    if(!disposed)setPosition({x:props.object.x,y:props.object.y});
  }
  createEffect(on(() => Boolean(props.exiting), exiting => { if(exiting)cancelPendingIntent(); }));
  onCleanup(() => {disposed=true;cancelPendingIntent();});
  async function drainZoom() {
    if(zooming||disposed||props.exiting)return;zooming=true;
    const generation=intentGeneration;
    try {while(zoomSteps&&!disposed&&!props.exiting&&generation===intentGeneration){const direction=zoomSteps>0?1:-1;zoomSteps-=direction;await controlPinObject(props.object.id,{type:'zoom',direction});}}
    catch(cause){if(generation===intentGeneration){zoomSteps=0;if(!disposed)props.onError(cause);}}
    finally{zooming=false;if(zoomSteps&&!disposed&&!props.exiting)void drainZoom();}
  }
  function wheel(event:WheelEvent) {
    if(props.busy||props.exiting||disposed||moving||!event.deltaY)return;
    event.preventDefault();event.stopPropagation();
    zoomSteps=Math.max(-8,Math.min(8,zoomSteps+(event.deltaY<0?1:-1)));
    void drainZoom();
  }
  async function closeObject() {
    if(props.busy||props.exiting||disposed)return;
    const generation=intentGeneration;
    try{await controlPinObject(props.object.id,{type:'close'});}
    catch(cause){if(!disposed&&generation===intentGeneration)props.onError(cause);}
  }
  function down(event: PointerEvent, grip = false) {
    if (props.busy || props.exiting || disposed || event.button !== 0 || !grip && (event.target as Element).closest('button,.pin-video-content')) return;
    event.preventDefault();event.stopPropagation();cleanup?.();
    const target=event.currentTarget as HTMLElement,start=position(),x=event.clientX,y=event.clientY,vw=innerWidth,vh=innerHeight,generation=intentGeneration;
    let changed=false;let collection:ObjectDrag|undefined;moving=true;target.setPointerCapture(event.pointerId);
    const move=(e:PointerEvent)=>{changed=true;collection??=beginObjectDrag({source:{kind:'pin',id:props.object.id},remove:closeObject});collection?.move({x:e.clientX,y:e.clientY});const next=moveImageObject({...start,width:props.object.displayWidth,height:props.object.displayHeight},(e.clientX-x)/vw,(e.clientY-y)/vh,vw,vh);setPosition({x:next.x,y:next.y});};
    const stop=()=>{target.removeEventListener('pointermove',move);target.removeEventListener('pointerup',end);target.removeEventListener('pointercancel',end);if(target.hasPointerCapture(event.pointerId))target.releasePointerCapture(event.pointerId);cleanup=undefined;};
    const end=async(e:PointerEvent)=>{stop();const collected=e.type==='pointerup'&&collection?.finish({x:e.clientX,y:e.clientY});collection?.cancel();try{if(changed&&!collected&&e.type!=='pointercancel'&&!disposed&&!props.exiting&&generation===intentGeneration)await controlPinObject(props.object.id,{type:'move',x:position().x,y:position().y});}catch(cause){if(!disposed&&generation===intentGeneration){setPosition({x:props.object.x,y:props.object.y});props.onError(cause);}}finally{if(!disposed&&generation===intentGeneration){moving=false;if(collected||!changed||e.type==='pointercancel'||props.object.x!==start.x||props.object.y!==start.y)setPosition({x:props.object.x,y:props.object.y});}}};
    cleanup=()=>{collection?.cancel();stop();};target.addEventListener('pointermove',move);target.addEventListener('pointerup',end);target.addEventListener('pointercancel',end);
  }
  return <section class="space-pin-object" tabindex={0} aria-label={t('贴图')} onPointerDown={down} onWheel={wheel} style={{left:`${position().x*100}%`,top:`${position().y*100}%`,width:`${props.object.displayWidth*100}%`,height:`${props.object.displayHeight*100}%`,opacity:props.object.opacity}}>
    <Show when={props.object.media} fallback={<img src={props.object.imageUrl} alt={t('贴图')} draggable={false} style={{width:props.object.quarterTurns%2?`${props.object.displayHeight/props.object.displayWidth*innerHeight/innerWidth*100}%`:'100%',height:props.object.quarterTurns%2?`${props.object.displayWidth/props.object.displayHeight*innerWidth/innerHeight*100}%`:'100%',transform:`translate(-50%,-50%) rotate(${props.object.quarterTurns*90}deg)`}}/>}><PinVideo source={props.object.imageUrl} media={props.object.media!} width={props.object.width} height={props.object.height} onError={props.onError} onDrag={event => down(event, true)} /></Show>
    <div class="image-object-tools" role="toolbar" aria-label={t('图片工具')} onPointerDown={e=>e.stopPropagation()}>
      <Show when={props.aiAvailable !== false && !props.object.media}><button class="icon-button compact" classList={{selected:props.referenced(props.object)}} title={props.referenced(props.object)?t('取消引用'):t('引用')} aria-label={props.referenced(props.object)?t('取消引用'):t('引用')} disabled={props.busy} onClick={()=>props.onReference(props.object)}><Link2 size={15}/></button></Show>
      <button class="icon-button compact" title={t('关闭贴图')} aria-label={t('关闭贴图')} disabled={props.busy||props.exiting} onClick={()=>void closeObject()}><X size={15}/></button>
    </div>
  </section>;
}
export default function PinObjectLayer(props: Props) {
  const index=createMemo(()=>new Map(props.objects.map(object=>[object.id,object])));
  return <div class="space-pin-layer"><For each={props.objects.map(object=>object.id)}>{id=><ImageObject {...props} object={index().get(id)!}/>}</For></div>;
}
