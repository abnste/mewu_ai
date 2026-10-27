// SPDX-License-Identifier: MPL-2.0
import * as api from './extension-bridge';
import { listen } from '@tauri-apps/api/event';
import { mergeExtensionRecords } from './extension-state';
import type { ExtensionRecord } from './extension-contracts';
import type { ToolbarItem } from './toolbar-catalog';
import { reorderToolbar, toolbarLayoutServices } from './toolbar-layout';

const bounded = <T>(operation: Promise<T>,milliseconds=1500): Promise<T> => new Promise((resolve,reject) => {
  const timer=setTimeout(()=>reject(Error('Toolbar layout timed out')),milliseconds);
  operation.then(value=>{clearTimeout(timer);resolve(value);},error=>{clearTimeout(timer);reject(error);});
});
// Window-lifetime metadata and bounded result cache. Never caches DOM nodes,
// handler owners or authority. Revocation/configuration clears all results.
let records:ExtensionRecord[]=[], initialized=!api.extensionNative, epoch=0, users=0, session=0, events=0;
let starting:Promise<void>|undefined;
let stops:Array<()=>void>=[];
const listeners=new Set<()=>void>();
const cache=new Map<string,{epoch:number;promise:Promise<unknown[]>;value?:unknown[]}>();
const notify=()=>{for(const accept of listeners)accept();};
function invalidate(){epoch++;cache.clear();notify();}
function identity(values:ExtensionRecord[]){return JSON.stringify(values.map(r=>[r.manifest.id,r.revision,r.state,r.services,r.platformError,r.unsupportedServices]));}
export function startToolbarLayouts():()=>void {
  users++;
  if(!starting&&api.extensionNative){
    const current=++session;
    starting=(async()=>{
      try {
        const stop=await api.subscribeExtensions(values=>{
          if(current!==session)return;
          events++;const next=mergeExtensionRecords(records,values),changed=identity(next)!==identity(records);
          records=next; initialized=true;if(changed)invalidate();else notify();
        });
        if(current!==session){stop();return;}stops.push(stop);
        const unlisten=await listen('extension-configuration-changed',()=>{if(current===session)invalidate();});
        if(current!==session){unlisten();return;}stops.push(unlisten);
        const before=events,values=await bounded(api.getExtensions());
        if(current!==session)return;
        if(before===events){records=values;invalidate();}
      } catch { /* Native controls still become usable on metadata failure. */ }
      finally {if(current===session){initialized=true;notify();}}
    })();
  }
  let active=true;
  return()=>{if(!active)return;active=false;if(--users===0){session++;stops.forEach(stop=>stop());stops=[];starting=undefined;records=[];initialized=!api.extensionNative;invalidate();}};
}
export function subscribeToolbarLayouts(accept:()=>void):()=>void{listeners.add(accept);return()=>{listeners.delete(accept);};}
export function toolbarLayoutsReady(){return initialized;}
export function toolbarLayoutOwner(surface:string){return JSON.stringify(toolbarLayoutServices(records,surface).map(c=>[c.record.manifest.id,c.record.revision,c.service.id]));}
const keyFor=(surface:string,items:ToolbarItem[])=>JSON.stringify([surface,items]);
export function cachedToolbarLayout(surface:string,items:ToolbarItem[]):unknown[]|undefined{return cache.get(keyFor(surface,items))?.value;}
export function resolveToolbarLayout(surface:string,items:ToolbarItem[]):Promise<unknown[]> {
  const key=keyFor(surface,items),found=cache.get(key);if(found)return found.promise;
  const generation=epoch,choices=toolbarLayoutServices(records,surface);
  const row:{epoch:number;promise:Promise<unknown[]>;value?:unknown[]}={epoch:generation,promise:Promise.resolve([])};
  row.promise=(async()=>{
    let value=items;const plans:unknown[]=[];const deadline=Date.now()+1500;
    try {
      for(const choice of choices){
        const remaining=deadline-Date.now();if(remaining<=0)throw Error('Toolbar layout timed out');
        const result=await bounded(api.invokeExtension({pluginId:choice.record.manifest.id,revision:choice.record.revision,serviceId:choice.service.id,input:{type:'layout',surface,items:value}}),remaining);
        if(generation!==epoch)throw Error('Toolbar layout invalidated');
        value=reorderToolbar(value,item=>item.id,result);plans.push(result);
      }
    } catch {plans.length=0;}
    if(generation!==epoch)throw Error('Toolbar layout invalidated');
    row.value=plans;return plans;
  })();
  if(cache.size>=32)cache.delete(cache.keys().next().value!);
  cache.set(key,row);return row.promise;
}
