/* SPDX-License-Identifier: MPL-2.0 */
import {createEffect,createMemo,createSignal,onCleanup,Show} from 'solid-js';
import {Move} from 'lucide-solid';
import type {Drawing,DrawingPoint} from '../contracts';
import type {EditorBox} from '../drawing-editor-port';
import type {RegisterDrawingFlush} from '../drawing-flush';
import {blackboardImage,imageAt,zoomBlackboardImage} from '../blackboard-image';
import {t} from '../i18n';
import './blackboard-image.css';
interface Props {
  surface:()=>SVGSVGElement;box:EditorBox;frame:()=>EditorBox;drawings:()=>Drawing[];preview:()=>Drawing|undefined;
  revision:()=>number;source:()=>string;disabled:()=>boolean;gestureActive:()=>boolean;
  onMove:(event:PointerEvent,drawing:Drawing)=>void;
  onUpdate:(drawing:Drawing,revision:number,active:()=>boolean)=>Promise<boolean>;
  onPreview:(drawing:Drawing|undefined)=>void;
  onZooming:(value:boolean)=>void;
  onRegisterFlush?:RegisterDrawingFlush;onError:(message:string)=>void;
}
export default function BlackboardImageControls(props:Props){
  let controls!:HTMLDivElement,disposed=false,timer:ReturnType<typeof setTimeout>|undefined;
  let zoom:{value:Drawing;revision:number;source:string}|undefined,flight:Promise<boolean>|undefined;
  const [hovered,setHovered]=createSignal(''),[zooming,setZooming]=createSignal(false);
  // The image and its corner control must use the same live gesture geometry.
  const value=createMemo(()=>{const id=hovered(),preview=props.preview();return preview?.id===id?preview:props.drawings().find(value=>value.id===id);});
  const coordinate=(event:{clientX:number;clientY:number}):DrawingPoint=>{const box=props.surface().getBoundingClientRect(),frame=props.frame();return{x:frame.x+(event.clientX-box.left)/box.width*frame.width,y:frame.y+(event.clientY-box.top)/box.height*frame.height};};
  const cancel=()=>{if(timer!==undefined)clearTimeout(timer);timer=undefined;zoom=undefined;setZooming(false);props.onZooming(false);props.onPreview(undefined);};
  async function flush(active:()=>boolean=()=>true):Promise<boolean>{
    if(timer!==undefined)clearTimeout(timer);timer=undefined;
    if(flight)return (await flight)&&active();
    const pending=zoom;if(!pending)return true;zoom=undefined;
    const valid=()=>!disposed&&active()&&props.source()===pending.source&&props.drawings().some(value=>value.id===pending.value.id);
    if(!valid()){cancel();return false;}
    const request=props.onUpdate(pending.value,pending.revision,valid).catch(error=>{if(valid())props.onError(error instanceof Error?error.message:String(error));return false;});
    flight=request;
    const saved=await request;
    if(flight===request)flight=undefined;
    if(!disposed){setZooming(false);props.onZooming(false);props.onPreview(undefined);}
    return saved;
  }
  const hover=(event:PointerEvent)=>{
    if(disposed||event.buttons||props.disabled()||props.gestureActive()||zooming())return;
    setHovered(imageAt(props.drawings(),coordinate(event))?.id??'');
  };
  const leave=(event:PointerEvent)=>{if(!controls?.contains(event.relatedTarget as Node)&&!zooming())setHovered('');};
  const wheel=(event:WheelEvent)=>{
    if(disposed||props.disabled()||props.gestureActive()||flight)return;
    const point=coordinate(event),image=zoom?.value??imageAt(props.drawings(),point);
    if(!image||!blackboardImage(image))return;
    const delta=event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?props.box.height:1);
    const next=zoomBlackboardImage(image,props.frame(),point,delta);if(!next)return;
    event.preventDefault();event.stopPropagation();
    if(!zoom)zoom={value:next,revision:props.revision(),source:props.source()};else zoom.value=next;
    setHovered(image.id);setZooming(true);props.onZooming(true);props.onPreview(next);
    if(timer!==undefined)clearTimeout(timer);timer=setTimeout(()=>void flush(),140);
  };
  const beforeStroke=(event:PointerEvent)=>{if(zooming()){event.preventDefault();event.stopImmediatePropagation();void flush();}};
  createEffect(()=>{const surface=props.surface();if(!surface)return;surface.addEventListener('pointermove',hover);surface.addEventListener('pointerleave',leave);surface.addEventListener('pointerdown',beforeStroke,true);surface.addEventListener('wheel',wheel,{passive:false});onCleanup(()=>{surface.removeEventListener('pointermove',hover);surface.removeEventListener('pointerleave',leave);surface.removeEventListener('pointerdown',beforeStroke,true);surface.removeEventListener('wheel',wheel);});});
  createEffect(()=>{const source=props.source();if(zoom&&zoom.source!==source)cancel();});
  const unregister=props.onRegisterFlush?.(async active=>{if(!(await flush(active)))throw Error('图片尺寸尚未保存');});
  const position=()=>{const drawing=value();if(!drawing)return{};const frame=props.frame(),a=drawing.points[0],b=drawing.points[1],x=props.box.x+(b.x-frame.x)/frame.width*props.box.width-32,y=props.box.y+(a.y-frame.y)/frame.height*props.box.height+4;return{left:`${Math.max(4,Math.min(innerWidth-32,x))}px`,top:`${Math.max(4,Math.min(innerHeight-32,y))}px`};};
  onCleanup(()=>{disposed=true;unregister?.();cancel();});
  return <Show when={value()}>{drawing=><div ref={controls} class="blackboard-image-controls" role="toolbar" aria-label={t('图片工具')} style={position()} onPointerDown={event=>event.stopPropagation()} onPointerLeave={event=>{const surface=props.surface();if(!surface.contains(event.relatedTarget as Node)&&!zooming())setHovered('');}}>
    <button aria-label={t('移动图片')} title={t('移动图片')} disabled={props.disabled()||zooming()} onPointerDown={event=>props.onMove(event,drawing())}><Move size={18} strokeWidth={1.7}/></button>
  </div>}</Show>;
}
