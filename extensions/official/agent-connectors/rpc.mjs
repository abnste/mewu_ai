// SPDX-License-Identifier: MPL-2.0
import {workbuddyProcess} from './workbuddy.mjs';
/** Protocol implementation belongs to this independently packaged module. */
export async function openPeer(ctx, config, jsonrpc = true){
  config=workbuddyProcess(config);
  let process;
  try{process=await ctx.processes.open({executable:config.executable,args:config.args,...(config.cwd?{cwd:config.cwd}:{}),...(config.env?{env:config.env}:{})});}
  catch{throw Error('无法启动外部程序，请检查运行程序和启动参数');}
  let sequence=0,closed=false;
  const envelope=value=>jsonrpc?{jsonrpc:'2.0',...value}:value;
  const peer={
    cwd:process.cwd,
    notification:async()=>{},
    async send(message,bindings){if(closed)throw Error('连接已关闭');await ctx.processes.write(process.id,envelope(message),bindings?.length?bindings:undefined);},
    async poll(){
      if(closed)throw Error('连接已关闭');
      let message;
      try{message=await ctx.processes.read(process.id,250);}
      catch{throw Error('外部程序在返回有效协议响应前退出，请检查 CLI 版本和启动参数');}
      if(!message)return null;
      if(typeof message.method==='string'){
        if(message.id!==undefined){
          if(message.method==='session/request_permission'){
            const option=message.params?.options?.find(item=>item.kind==='reject_once');
            await peer.send({id:message.id,result:{outcome:option?{outcome:'selected',optionId:option.optionId}:{outcome:'cancelled'}}});
          }else if(message.method.endsWith('/requestApproval')){
            await peer.send({id:message.id,result:{decision:'decline'}});
          }else await peer.send({id:message.id,error:{code:-32601,message:'Client capability not available'}});
        }else await peer.notification(message.method,message.params??{});
        return null;
      }
      return message;
    },
    async request(method,params,bindings,timeout=25000){
      const id=++sequence;await peer.send({id,method,params},bindings);
      const until=Date.now()+timeout;
      while(Date.now()<until){
        const message=await peer.poll();
        if(!message||message.id!==id)continue;
        if(message.error)throw Error(`${method}: ${String(message.error.message??'请求失败').slice(0,400)}`);
        if(!Object.hasOwn(message,'result'))throw Error(`${method}: 响应不完整`);
        return message.result;
      }
      throw Error(`${method}: 连接超时`);
    },
    async wait(done,timeout=250000){
      const until=Date.now()+timeout;
      while(!done()){if(Date.now()>=until)throw Error('Agent 本轮响应超时');await peer.poll();}
    },
    async close(){if(closed)return;closed=true;await ctx.processes.close(process.id);}
  };
  return peer;
}
