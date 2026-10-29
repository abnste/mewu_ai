// SPDX-License-Identifier: MPL-2.0
/** One MessagePort belongs to one rendered generation. It cannot choose an IPC
 * method, plugin, service, scene, filesystem path or captured object source. */
import type {SurfacePointerGesture,SurfacePointerSample} from './extension-surface-pointer';
export interface DropTarget {id:string;x:number;y:number;width:number;height:number;default?:boolean}
export function surfaceBridge(port:MessagePort, call:(input:unknown)=>Promise<unknown>,
  targets:(value:DropTarget[])=>void, drag?:(input:unknown)=>SurfacePointerGesture, escape?:()=>void, closeError?:(message:string)=>void, escapeKey?:()=>void) {
  let closed=false, pending=0;
  // Presentation data only. No identity or invocation authority crosses.
  const appearance=()=>{
    if(closed||typeof document==='undefined'||!document.documentElement||typeof getComputedStyle!=='function')return;
    const style=getComputedStyle(document.documentElement),tokens:Record<string,string>={};
    for(let i=0;i<style.length&&Object.keys(tokens).length<128;i++){
      const key=style[i],value=style.getPropertyValue(key).trim();
      if(/^--[a-z][a-z-]{0,63}$/.test(key)&&/^#[0-9a-f]{3}(?:[0-9a-f]{3})?$/i.test(value))tokens[key]=value.length===4?'#'+[...value.slice(1)].map(c=>c+c).join(''):value;
    }
    port.postMessage({type:'event',name:'appearance',value:{version:1,colorScheme:style.colorScheme==='dark'?'dark':'light',tokens,colors:{text:style.getPropertyValue('--text').trim(),muted:style.getPropertyValue('--secondary').trim(),field:style.getPropertyValue('--surface-subtle').trim(),line:style.getPropertyValue('--line').trim(),accent:style.getPropertyValue('--accent').trim()}}});
  };
  const appearanceObserver=typeof MutationObserver==='function'&&typeof document!=='undefined'?new MutationObserver(appearance):undefined;
  appearanceObserver?.observe(document.documentElement,{attributes:true,attributeFilter:['style','data-mewu-theme','data-mewu-theme-base']});
  const active=new Set<number>();
  const gestures=new Map<number,SurfacePointerGesture>();
  port.onmessage=async ({data})=>{
    if(closed||!data||typeof data!=='object')return;
    let bytes:number;try{bytes=JSON.stringify(data).length;}catch{return;}
    if(bytes>256*1024)return;
    if(data.type==='pointer-drag-event'){
      const gesture=gestures.get(data.id),event=data.event;
      if(!gesture||!event||!['pointermove','pointerup','pointercancel'].includes(event.type)
        ||![event.x,event.y].every((n:unknown)=>typeof n==='number'&&Number.isFinite(n)&&Math.abs(n)<=32768)
        ||!Number.isSafeInteger(event.buttons)||event.buttons<0||event.buttons>31)return;
      gesture.forward(event as SurfacePointerSample);return;
    }
    if(data.type==='escape'){escape?.();return;}
    if(data.type==='escape-key'){escapeKey?.();return;}
    if(data.type==='close-error'){closeError?.(typeof data.error==='string'?data.error.slice(0,240):'保存失败');return;}
    if(data.type==='drop-targets'){
      if(!Array.isArray(data.targets)||data.targets.length>128)return;
      const ids=new Set<string>(), accepted:DropTarget[]=[];let defaults=0;
      for(const target of data.targets){
        if(!target||typeof target.id!=='string'||target.id.length>128||ids.has(target.id)
          ||![target.x,target.y,target.width,target.height].every(Number.isFinite)
          ||target.width<=0||target.height<=0||target.width>1600||target.height>1200
          ||(target.default!==undefined&&typeof target.default!=='boolean')||(target.default===true&&++defaults>1))return;
        ids.add(target.id);accepted.push({id:target.id,x:target.x,y:target.y,width:target.width,height:target.height,...(target.default===true?{default:true}:{})});
      }
      targets(accepted);return;
    }
    if(!['call','pointer-drag'].includes(data.type)||!Number.isSafeInteger(data.id)||data.id<1||active.has(data.id))return;
    if(pending>=16){port.postMessage({type:'response',id:data.id,ok:false,error:'Too many pending requests'});return;}
    active.add(data.id);pending++;
    try{
      if(data.type==='pointer-drag'&&!drag)throw Error('拖动不可用');
      let value:unknown;
      if(data.type==='pointer-drag'){
        const gesture=drag!(data.input);gestures.set(data.id,gesture);
        value=await gesture.done;
      }else value=await call(data.input);
      if(!closed)port.postMessage({type:'response',id:data.id,ok:true,value});
    }
    catch(error){if(!closed)port.postMessage({type:'response',id:data.id,ok:false,error:error instanceof Error?error.message:'Plugin request failed'});}
    finally{gestures.delete(data.id);active.delete(data.id);pending--;}
  };
  port.start();
  appearance();
  return {
    event(name:string,value:unknown){if(!closed)port.postMessage({type:'event',name,value});},
    close(){closed=true;appearanceObserver?.disconnect();port.onmessage=null;for(const gesture of gestures.values())gesture.cancel();gestures.clear();port.close();active.clear();targets([]);}
  };
}
