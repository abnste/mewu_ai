// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
import {profiles,validateConfig,detectProfile,resolveConfig} from './profiles.mjs';
import {registerCatalog,selectedConfig} from './catalog.mjs';
import {connectCodex,connectAcp,runCodex,runAcp,codexEffort} from './adapters.mjs';
import {html} from './view.mjs';

export default definePlugin(async ctx=>{
  const runtime=await ctx.runtime.describe();
  if(!['ui.settings','configuration.shared','process.jsonlines','process.frame-bindings'].every(feature=>runtime.capabilities.includes(feature)))throw Error('请更新 Mewu 后使用外部 Agent');
  ctx.ui.register({id:'settings',title:'外部 Agent',metadata:{surface:'settings'}},async input=>{
    if(input?.type==='mount')return {html,width:800,height:650};
    const stored=await ctx.configuration.get('connections');
    const current=stored??{};
    if(input?.type==='get')return {profiles:profiles.map(({id,name,args})=>({id,name,args})),connections:current};
    const profile=profiles.find(v=>v.id===input?.provider);if(!profile)throw Error('Agent 不存在');
    if(input.type==='detect'){
      return detectProfile(ctx,profile);
    }
    const config=validateConfig(input.config,profile);
    if(input.type==='save'){
      if(JSON.stringify(current[profile.id]??null)!==JSON.stringify(input.expected??null))throw Error('此连接已变化，请重新载入');
      const next={...current,[profile.id]:config};
      if(!await ctx.configuration.compareAndSet('connections',stored,next))throw Error('连接已变化，请重新载入');
      return {saved:true,config};
    }
    if(input.type==='test'){
      try{
        const connection=await (profile.protocol==='codex'?connectCodex:connectAcp)(ctx,{...await resolveConfig(ctx,config,profile),model:''});
        try{if(profile.protocol==='codex')codexEffort(connection.models,config);return {ok:true,models:connection.models,image:profile.protocol==='codex'?connection.models.some(m=>m.image):connection.initialized.agentCapabilities?.promptCapabilities?.image===true};}
        finally{await connection.peer.close();}
      }catch(error){return {ok:false,error:String(error?.message??'连接失败').slice(0,500)};}
    }
    throw Error('不支持的设置操作');
  });
  for(const profile of profiles){
  const modelsService=registerCatalog(ctx,profile);
  ctx.agents.register({id:profile.id,title:profile.name,description:'使用本机官方客户端的登录与模型',metadata:{modelsService}},async(input,call)=>{
    try{
    const config=await selectedConfig(ctx,profile,call.context);
    const key=`sessions/${profile.id}/${call.context.agentId}/${call.context.sceneId}`;
    const saved=await ctx.storage.get(key);
    const fingerprint=JSON.stringify(config);
    const result=await (profile.protocol==='codex'?runCodex:runAcp)(ctx,config,input,saved?.config===fingerprint?saved.session:undefined,
      session=>ctx.storage.set(key,{config:fingerprint,session}));
    return {content:result.content};
    }catch(error){
      // Explicit public diagnostics, never arbitrary exceptions, paths or stacks.
      const publicErrors=['请先在该 Agent 的官方客户端或 CLI 中登录','请先在 Codex 官方客户端登录','请选择已安装的运行程序','所选 Codex 模型不可用','该 Agent 未声明图片支持','请先在外部 Agent 设置中检测并保存连接','未找到已安装的官方客户端，请在外部 Agent 设置中重新检测','无法启动外部程序，请检查程序路径、工作目录和启动参数'];
      const message=publicErrors.includes(error?.message)||['该 Agent 无法切换到所选模型，请选择默认模型或更新官方客户端','该 Agent 未提供所选模型'].includes(error?.message)
        ?error.message:error?.publicMessage??'外部 Agent 未能完成请求，请检查官方客户端的连接和模型设置';
      await ctx.events.emit('agent.text',message);
      throw error;
    }
  });}
});
