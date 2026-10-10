// SPDX-License-Identifier: MPL-2.0
import type {RunEvent} from './contracts';
import {splitReplyReasoning} from './reply-reasoning';
export interface LivePreview {runId:string;text:string;reasoning:string}
export function livePreview(previous:LivePreview|undefined,event:RunEvent):LivePreview {
  const old=previous?.runId===event.runId?previous:{runId:event.runId,text:'',reasoning:''};
  return {runId:event.runId,text:Array.from(old.text+(event.text??'')).slice(-1600).join(''),reasoning:Array.from(old.reasoning+(event.reasoning??'')).slice(-1600).join('')};
}
// This is a transient preview cache, independent of saved conversations/memory.
export function mergeLivePreview(previous:ReadonlyMap<string,LivePreview>,event:RunEvent):ReadonlyMap<string,LivePreview>{
  const entries=Array.from(previous).filter(([id])=>id!==event.sceneId).slice(-127);
  return new Map([...entries,[event.sceneId,livePreview(previous.get(event.sceneId),event)]]);
}
export function previewSnippet(text:string,tail=false):string {
  const clean=text.replace(/\s+/g,' ').trim(),chars=Array.from(clean);
  return chars.length>90?(tail?chars.slice(-90).join(''):chars.slice(0,90).join('')+'...'):clean;
}
export function liveSnippet(value:LivePreview|undefined):string {
  if(!value)return '';
  const parts=splitReplyReasoning(value.text,value.reasoning,true);
  return previewSnippet(parts.text||parts.reasoning,true);
}
