// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, onCleanup } from 'solid-js';
import { Link2, X } from 'lucide-solid';
import { t } from '../i18n';
import { controlPinObject, type PinObject } from '../pin-objects';
import './pin-objects.css';
import {moveImageObject} from '../image-object-geometry';
interface Props { objects: PinObject[]; busy: boolean; referenced: (object: PinObject) => boolean; onReference: (object: PinObject) => void; onError: (error: unknown) => void }
function ImageObject(props: Props & { object: PinObject }) {
  const [position, setPosition] = createSignal({ x: props.object.x, y: props.object.y });
  let moving = false, disposed=false, zooming=false,zoomSteps=0, cleanup: (() => void) | undefined;
  createEffect(() => { const x=props.object.x,y=props.object.y; if(!moving)setPosition({x,y}); });
  onCleanup(() => {disposed=true;zoomSteps=0;cleanup?.();});
  async function wheel(event:WheelEvent) {
    if(props.busy||moving||!event.deltaY)return;
    event.preventDefault();event.stopPropagation();
    zoomSteps=Math.max(-8,Math.min(8,zoomSteps+(event.deltaY<0?1:-1)));
    if(zooming)return;zooming=true;
    try {while(zoomSteps&&!disposed){const direction=zoomSteps>0?1:-1;zoomSteps-=direction;await controlPinObject(props.object.id,{type:'zoom',direction});}}
    catch(cause){zoomSteps=0;if(!disposed)props.onError(cause);}finally{zooming=false;}
  }
  function down(event: PointerEvent) {
    if (props.busy || event.button !== 0 || (event.target as Element).closest('button')) return;
    event.preventDefault();event.stopPropagation();cleanup?.();
    const target=event.currentTarget as HTMLElement,start=position(),x=event.clientX,y=event.clientY,vw=innerWidth,vh=innerHeight;
    let changed=false;moving=true;target.setPointerCapture(event.pointerId);
    const move=(e:PointerEvent)=>{changed=true;const next=moveImageObject({...start,width:props.object.displayWidth,height:props.object.displayHeight},(e.clientX-x)/vw,(e.clientY-y)/vh,vw,vh);setPosition({x:next.x,y:next.y});};
    const stop=()=>{target.removeEventListener('pointermove',move);target.removeEventListener('pointerup',end);target.removeEventListener('pointercancel',end);if(target.hasPointerCapture(event.pointerId))target.releasePointerCapture(event.pointerId);cleanup=undefined;};
    const end=async(e:PointerEvent)=>{stop();try{if(changed&&e.type!=='pointercancel')await controlPinObject(props.object.id,{type:'move',x:position().x,y:position().y});}catch(cause){setPosition({x:props.object.x,y:props.object.y});props.onError(cause);}finally{moving=false;if(!changed||e.type==='pointercancel'||props.object.x!==start.x||props.object.y!==start.y)setPosition({x:props.object.x,y:props.object.y});}};
    cleanup=stop;target.addEventListener('pointermove',move);target.addEventListener('pointerup',end);target.addEventListener('pointercancel',end);
  }
  return <section class="space-pin-object" tabindex={0} aria-label={t('贴图')} onPointerDown={down} onWheel={wheel} style={{left:`${position().x*100}%`,top:`${position().y*100}%`,width:`${props.object.displayWidth*100}%`,height:`${props.object.displayHeight*100}%`,opacity:props.object.opacity}}>
    <img src={props.object.imageUrl} alt={t('贴图')} draggable={false} style={{width:props.object.quarterTurns%2?`${props.object.displayHeight/props.object.displayWidth*innerHeight/innerWidth*100}%`:'100%',height:props.object.quarterTurns%2?`${props.object.displayWidth/props.object.displayHeight*innerWidth/innerHeight*100}%`:'100%',transform:`translate(-50%,-50%) rotate(${props.object.quarterTurns*90}deg)`}}/>
    <div class="image-object-tools" role="toolbar" aria-label={t('图片工具')} onPointerDown={e=>e.stopPropagation()}>
      <button class="icon-button compact" classList={{selected:props.referenced(props.object)}} title={props.referenced(props.object)?t('取消引用'):t('引用')} aria-label={props.referenced(props.object)?t('取消引用'):t('引用')} disabled={props.busy} onClick={()=>props.onReference(props.object)}><Link2 size={15}/></button>
      <button class="icon-button compact" title={t('关闭贴图')} aria-label={t('关闭贴图')} disabled={props.busy} onClick={()=>void controlPinObject(props.object.id,{type:'close'}).catch(props.onError)}><X size={15}/></button>
    </div>
  </section>;
}
export default function PinObjectLayer(props: Props) {
  const index=createMemo(()=>new Map(props.objects.map(object=>[object.id,object])));
  return <div class="space-pin-layer"><For each={props.objects.map(object=>object.id)}>{id=><ImageObject {...props} object={index().get(id)!}/>}</For></div>;
}
