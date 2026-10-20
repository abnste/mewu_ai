// SPDX-License-Identifier: MPL-2.0
/** One MessagePort belongs to one rendered generation. It cannot choose an IPC
 * method, plugin, service, scene, filesystem path or captured object source. */
export interface DropTarget {id:string;x:number;y:number;width:number;height:number}
export function surfaceBridge(port:MessagePort, call:(input:unknown)=>Promise<unknown>,
  targets:(value:DropTarget[])=>void, drag?:(input:unknown)=>Promise<unknown>, escape?:()=>void, closeError?:(message:string)=>void) {
  let closed=false, pending=0;
  const active=new Set<number>();
  port.onmessage=async ({data})=>{
    if(closed||!data||typeof data!=='object')return;
    let bytes:number;try{bytes=JSON.stringify(data).length;}catch{return;}
    if(bytes>256*1024)return;
    if(data.type==='escape'){escape?.();return;}
    if(data.type==='close-error'){closeError?.(typeof data.error==='string'?data.error.slice(0,240):'保存失败');return;}
    if(data.type==='drop-targets'){
      if(!Array.isArray(data.targets)||data.targets.length>128)return;
      const ids=new Set<string>(), accepted:DropTarget[]=[];
      for(const target of data.targets){
        if(!target||typeof target.id!=='string'||target.id.length>128||ids.has(target.id)
          ||![target.x,target.y,target.width,target.height].every(Number.isFinite)
          ||target.width<=0||target.height<=0||target.width>1600||target.height>1200)return;
        ids.add(target.id);accepted.push({id:target.id,x:target.x,y:target.y,width:target.width,height:target.height});
      }
      targets(accepted);return;
    }
    if(!['call','pointer-drag'].includes(data.type)||!Number.isSafeInteger(data.id)||data.id<1||active.has(data.id))return;
    if(pending>=16){port.postMessage({type:'response',id:data.id,ok:false,error:'Too many pending requests'});return;}
    active.add(data.id);pending++;
    try{
      if(data.type==='pointer-drag'&&!drag)throw Error('拖动不可用');
      const value=await (data.type==='pointer-drag'?drag!(data.input):call(data.input));
      if(!closed)port.postMessage({type:'response',id:data.id,ok:true,value});
    }
    catch(error){if(!closed)port.postMessage({type:'response',id:data.id,ok:false,error:error instanceof Error?error.message:'Plugin request failed'});}
    finally{active.delete(data.id);pending--;}
  };
  port.start();
  return {
    event(name:string,value:unknown){if(!closed)port.postMessage({type:'event',name,value});},
    close(){closed=true;port.onmessage=null;port.close();active.clear();targets([]);}
  };
}
