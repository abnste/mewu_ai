// SPDX-License-Identifier: MPL-2.0
import { invoke,isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { OcrDocument } from './contracts';
import type { SnapBounds } from './window-snap';
export interface QuickInteraction { generation:number;active:boolean;held:boolean;interacted:boolean;sceneId?:string|null }
export async function subscribeQuickInteraction(accept:(view:QuickInteraction)=>void){
  if(!isTauri())return()=>{};
  let observed=false;
  const stop=await listen<QuickInteraction>('quick-interaction',event=>{observed=true;accept(event.payload);});
  try{const view=await invoke<QuickInteraction>('quick_interaction_state');if(!observed)accept(view);return stop;}catch(error){stop();throw error;}
}
export function touchQuickInteraction(generation:number){return isTauri()?invoke<void>('quick_interaction_touch',{generation}):Promise.resolve();}
export function openQuickInteraction(proof?:{id:string;nonce:string}){if(!isTauri())return Promise.reject(Error('请在桌面版使用快速交互'));return invoke<QuickInteraction>('quick_interaction_open',{operationId:proof?.id,operationNonce:proof?.nonce});}
export async function recognizeQuickInteraction(generation:number,current:()=>boolean){
  try{return await invoke<OcrDocument>('quick_interaction_recognize',{generation});}
  catch(error){
    // Native OCR only reads owned pixels; the first worker has actually ended.
    // Retry once while the same scene/provider still owns the request. This
    // policy must never be used for commands, input or clipboard publication.
    if(!current())throw error;
    return invoke<OcrDocument>('quick_interaction_recognize',{generation});
  }
}
export function quickImageRegions(generation:number){return invoke<SnapBounds[]>('quick_interaction_images',{generation});}
