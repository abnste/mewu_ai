// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
import {api,config,bank,probe,route} from './transport.mjs';
import {html} from './view.mjs';

// Retain completion and forgetting can overlap. Never replace a newer
// tombstone with the stale receipt captured before a network request.
async function updateReceipt(storage,key,change) {
  for(let attempt=0;attempt<32;attempt++) {
    const current=await storage.get(key);
    if(!current)throw Error('记忆记录不存在');
    const next=change(current);
    if(await storage.compareAndSet(key,current,next))return next;
  }
  throw Error('记忆任务正在执行');
}

const publicError=error=>{
  const value=error?.message;
  return typeof value==='string'&&(/^(服务返回 HTTP \d{3})$/u.test(value)||[
    '请配置 Hindsight 服务','服务地址无效','记忆库前缀无效','Agent 身份无效',
    '连接中断，操作结果未知','服务拒绝认证','服务返回内容无效','服务 API 不兼容','服务尚未就绪',
    '配置已变化，请重新载入','记忆库身份不匹配','记忆内容无效','记忆记录不存在','记忆任务正在执行'
  ].includes(value))?value:'记忆服务操作失败';
};

export default definePlugin(async ctx=>{
  const runtime=await ctx.runtime.describe();
  if(!runtime.capabilities.includes('secrets.network'))throw Error('请更新 Mewu 后使用此插件');
  ctx.ui.register({id:'settings',title:'Hindsight',metadata:{surface:'settings'}},async input=>{
    if(input?.type==='mount')return {html,width:800,height:650};
    if(input?.type==='get')return {config:await ctx.configuration.get('connection'),secret:await ctx.secrets.status('api')};
    try{
      if(input?.type==='save'){
        const value=config(input.config),previous=await ctx.configuration.get('connection');
        if(JSON.stringify(previous)!==JSON.stringify(input.expected))throw Error('配置已变化，请重新载入');
        // Secret origin is bound before publishing the new endpoint. A failed
        // CAS leaves the prior endpoint unable to consume the new credential.
        if(typeof input.key==='string')await ctx.secrets.set('api',input.key,value.endpoint,input.secretRevision);
        if(!await ctx.configuration.compareAndSet('connection',previous,value))throw Error('配置已变化，请重新载入');
        return {ok:true,config:value,secret:await ctx.secrets.status('api')};
      }
      if(input?.type==='test')return await probe(ctx,await ctx.configuration.get('connection'));
      return {ok:false,error:'不支持的设置操作'};
    }catch(error){return {ok:false,error:publicError(error)};}
  });
  ctx.memory.register({id:'memory',title:'Hindsight',metadata:{operations:['retain','recall','forget']},
    inputSchema:{type:'object',properties:{operation:{enum:['retain','recall','forget']},text:{type:'string',maxLength:65536},query:{type:'string',maxLength:4000},id:{type:'string'},limit:{type:'integer',minimum:1,maximum:100}},required:['operation'],additionalProperties:false}
  },async(input,{context})=>{
    const settings=config(await ctx.configuration.get('connection'));
    const id=bank(settings,context.agentId),request=api(ctx,settings);
    const namespace=`bank.${encodeURIComponent(settings.endpoint)}.${id}.`;
    if(input.operation==='recall'){
      const response=await request('POST',route(id,'memories','recall'),{query:String(input.query??''),budget:'low',max_tokens:1500,types:['world','experience'],prefer_observations:false,trace:false,include:{entities:null,chunks:null,source_facts:null}},{missing:true});
      if(response===null)return {memories:[]};
      if(!Array.isArray(response.results))throw Error('服务返回内容无效');
      const memories=[];
      for(const item of response.results){
        if(!item||typeof item.text!=='string'||typeof item.document_id!=='string')continue;
        const own=await ctx.storage.get(namespace+'document.'+item.document_id);
        // Only plugin-owned documents in this agent's bank, never another
        // installation's remote content; tombstones suppress late writes too.
        if(!own||own.forgotten||own.state!=='committed')continue;
        memories.push({id:item.document_id,text:item.text.slice(0,8192)});
        if(memories.length>=Math.min(100,input.limit??8))break;
      }
      return {memories};
    }
    if(input.operation==='forget'){
      if(typeof input.id!=='string'||!/^[A-Za-z0-9_-]{1,200}$/u.test(input.id))throw Error('记忆记录不存在');
      const key=namespace+'document.'+input.id,stored=await ctx.storage.get(key);
      if(!stored)throw Error('记忆记录不存在');
      // Suppress immediately and durably before a remote deletion. A network
      // failure is not a license to resurrect or blindly replay the mutation.
      await updateReceipt(ctx.storage,key,current=>({...current,forgotten:true}));
      const response=await request('DELETE',route(id,'documents',input.id),undefined,{missing:true});
      if(response!==null&&(response.document_id!==input.id||response.success!==true))throw Error('服务返回内容无效');
      return {forgotten:true,id:input.id};
    }
    const text=typeof input.text==='string'?input.text.trim():'';
    if(input.operation!=='retain'||!text||text.length>65536)throw Error('记忆内容无效');
    const sequenceKey=namespace+'sequence';let sequence;
    for(let attempt=0;attempt<32;attempt++){
      const previous=await ctx.storage.get(sequenceKey);sequence=(previous??0)+1;
      if(!Number.isSafeInteger(sequence)||sequence<1)throw Error('记忆任务正在执行');
      if(await ctx.storage.compareAndSet(sequenceKey,previous,sequence))break;
      if(attempt===31)throw Error('记忆任务正在执行');
    }
    const document=`mewu-${context.agentId}-${sequence}`,key=namespace+'document.'+document;
    const receipt={id:document,state:'pending',forgotten:false,text,createdAt:new Date().toISOString()};
    await ctx.storage.set(key,receipt);
    try{
      // One synchronous retain; a timeout is retained as Unknown, never retried.
      const result=await request('POST',route(id,'memories'),{async:false,items:[{content:text,document_id:document,update_mode:'replace'}]});
      if(result.success!==true||result.bank_id!==id)throw Error('记忆库身份不匹配');
      const current=await updateReceipt(ctx.storage,key,current=>({...current,state:'committed'}));
      return {id:document,retained:!current.forgotten,forgotten:current.forgotten};
    }catch(error){
      await updateReceipt(ctx.storage,key,current=>({...current,state:'unknown'}));
      return {isError:true,id:document,state:'unknown',error:publicError(error),retryable:false};
    }
  });
});
