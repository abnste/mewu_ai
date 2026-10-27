// SPDX-License-Identifier: MPL-2.0
import {validateConfig,resolveConfig} from './profiles.mjs';
import {connectCodex,connectAcp,selectAcpModel} from './adapters.mjs';

const selectionKey=(profile,context)=>{
  if(!context?.agentId||!context?.sceneId)throw Error('请从当前对话选择模型');
  return `model-selection/${profile.id}/${context.sceneId}`;
};
export async function configured(ctx,profile){
  const value=(await ctx.configuration.get('connections'))?.[profile.id];
  if(!value)throw Error('请先在外部 Agent 设置中检测并保存连接');
  return resolveConfig(ctx,validateConfig(value,profile),profile);
}
export async function selectedConfig(ctx,profile,context){
  const config=await configured(ctx,profile),selection=await ctx.storage.get(selectionKey(profile,context));
  return selection&&typeof selection.modelId==='string'?{...config,model:selection.modelId}:config;
}
export function registerCatalog(ctx,profile){
  const serviceId=`models-${profile.id}`;
  ctx.services.register({id:serviceId,title:`${profile.name} 模型`},async(input,call)=>{
    try{
      if(!['list','select'].includes(input?.type))throw Error('模型操作无效');
      const key=selectionKey(profile,call.context),config=await configured(ctx,profile);
      // Listing must recover when an earlier configured model has been retired.
      const connection=await (profile.protocol==='codex'?connectCodex:connectAcp)(ctx,{...config,model:''});
      let models;
      try{
        models=connection.models;
        if(input.type==='select'){
          if(typeof input.modelId!=='string'||(input.modelId&&!models.some(m=>m.id===input.modelId)))throw Error('所选模型当前不可用，请刷新模型列表');
          // Validate with the real client before persisting. This disposable
          // session never sends a task or changes an existing conversation.
          if(profile.protocol!=='codex')await selectAcpModel(connection,input.modelId);
        }
      }finally{await connection.peer.close();}
      const current=await ctx.storage.get(key);
      if(input.type==='select'){
        if(typeof input.modelId!=='string'||(input.modelId&&!models.some(m=>m.id===input.modelId)))throw Error('所选模型当前不可用，请刷新模型列表');
        if(!await ctx.storage.compareAndSet(key,input.expected??null,{modelId:input.modelId}))throw Error('模型选择已变化，请重新打开模型列表');
      }
      const selection=input.type==='select'?{modelId:input.modelId}:current;
      return {ok:true,models,selection,selectedModelId:selection?.modelId??config.model??''};
    }catch(error){return {ok:false,error:String(error?.message??'无法读取模型').slice(0,500)};}
  });
  return serviceId;
}
