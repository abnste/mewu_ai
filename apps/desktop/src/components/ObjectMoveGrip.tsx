// SPDX-License-Identifier: MPL-2.0
import {createSignal,onCleanup,type JSX} from 'solid-js';
import {OBJECT_MOVE_WIDTH,OBJECT_MOVE_HEIGHT} from '../object-move-grip';
import './object-move-grip.css';
interface Props {label:string;disabled?:boolean;style?:JSX.CSSProperties;onPointerDown:(event:PointerEvent)=>void}
export default function ObjectMoveGrip(props:Props) {
  const [held,setHeld]=createSignal(false);
  let release:(()=>void)|undefined;
  const down=(event:PointerEvent)=>{
    if(event.button!==0||props.disabled)return;
    release?.();setHeld(true);
    const id=event.pointerId;
    const ended=(next:PointerEvent)=>{if(next.pointerId===id)release?.();};
    const blurred=()=>release?.();
    release=()=>{document.removeEventListener('pointerup',ended,true);document.removeEventListener('pointercancel',ended,true);window.removeEventListener('blur',blurred);release=undefined;setHeld(false);};
    document.addEventListener('pointerup',ended,true);document.addEventListener('pointercancel',ended,true);window.addEventListener('blur',blurred);
    try{props.onPointerDown(event);}catch(error){release?.();throw error;}
  };
  onCleanup(()=>release?.());
  return <button class="object-move-grip" classList={{'object-move-held':held()}} aria-label={props.label} title={props.label} disabled={props.disabled} style={{'--object-move-width':`${OBJECT_MOVE_WIDTH}px`,'--object-move-height':`${OBJECT_MOVE_HEIGHT}px`,...props.style}} onPointerDown={down} />;
}
