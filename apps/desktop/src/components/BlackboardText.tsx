// SPDX-License-Identifier: MPL-2.0
import { createEffect, onCleanup, untrack } from 'solid-js';
import { readArtifact, saveBlackboardText } from '../bridge';
import { createBlackboardText } from '../blackboard-text';
import type { DrawingFlush } from '../drawing-flush';
export default function BlackboardText(props:{sceneId:string;itemId:string;assetId:string;name:string;busy:boolean;onFlush:(flush:DrawingFlush)=>()=>void;onError:(error:string)=>void}){
  const data=createBlackboardText(readArtifact,async(id,text)=>(await saveBlackboardText(props.sceneId,props.itemId,id,text)).id,props.onError);
  createEffect(()=>{const id=props.assetId;untrack(()=>{void data.load(id).catch(error=>props.onError(error instanceof Error?error.message:String(error)));});});
  const unregister=props.onFlush(data.flush);onCleanup(unregister);
  return <textarea class="blackboard-text-editor" aria-label={props.name} spellcheck={false} value={data.text()} disabled={!data.loaded()||props.busy}
    onInput={event=>{data.edit(event.currentTarget.value,event.isComposing);if(data.text()!==event.currentTarget.value)event.currentTarget.value=data.text();}}
    onCompositionEnd={()=>data.schedule()} onBlur={()=>{if(data.loaded())void data.flush().catch(error=>props.onError(error instanceof Error?error.message:String(error)));}}
    onKeyDown={event=>event.stopPropagation()} onPointerDown={event=>event.stopPropagation()} />;
}
