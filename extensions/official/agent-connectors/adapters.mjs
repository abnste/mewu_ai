// SPDX-License-Identifier: MPL-2.0
import {openPeer} from './rpc.mjs';

const textInput=(input,resumed)=>{
  const messages=Array.isArray(input.messages)?input.messages:[];
  if(resumed)return String(input.task??messages.filter(m=>m.role==='user').at(-1)?.content??'');
  return messages.map(m=>`[${m.role}]\n${m.content}`).join('\n\n')||String(input.task??'');
};
const soul=input=>(input.messages??[]).filter(m=>m.role==='system'||m.role==='developer').map(m=>m.content).join('\n\n');
const imagesForTurn=(input,resumed)=>resumed&&Array.isArray(input.currentImages)?input.currentImages:input.images??[];
const stream=(ctx,kind,text)=>text?ctx.events.emit(kind,text):Promise.resolve();
export async function connectCodex(ctx,config){
  const peer=await openPeer(ctx,config,false);
  try{
    await peer.request('initialize',{clientInfo:{name:'mewu_agent_connectors',title:'Mewu',version:'0.1.0'}});
    await peer.send({method:'initialized',params:{}});
    const account=await peer.request('account/read',{refreshToken:false});
    if(!account.account&&account.requiresOpenaiAuth!==false)throw Error('请先在 Codex 官方客户端登录');
    const models=[];let cursor=null;
    for(let page=0;page<8;page++){
      const result=await peer.request('model/list',{limit:100,includeHidden:false,cursor});
      for(const model of result.data??[])models.push({id:model.model,name:model.displayName??model.model,default:model.isDefault===true,image:(model.inputModalities??[]).includes('image'),efforts:(model.supportedReasoningEfforts??[]).map(v=>v.reasoningEffort)});
      cursor=result.nextCursor;if(!cursor)break;if(page===7)throw Error('模型目录分页过多');
    }
    if(config.model&&!models.some(m=>m.id===config.model))throw Error('所选 Codex 模型不可用');
    return {peer,models};
  }catch(error){await peer.close();throw error;}
}
export async function runCodex(ctx,config,input,session,checkpoint=async()=>{}){
  const {peer,models}=await connectCodex(ctx,config);
  try{
    const configOverrides={};
    const configured=await peer.request('config/read',{includeLayers:false});
    for(const name of Object.keys(configured.config?.mcp_servers??{})){
      if(!/^[a-zA-Z0-9_-]+$/.test(name))throw Error('Codex MCP 名称不支持隔离');
      configOverrides[`mcp_servers.${name}.enabled`]=false;
    }
    const model=config.model||models.find(value=>value.default)?.id;
    const options={cwd:peer.cwd,...(model?{model}:{}),approvalPolicy:'on-request',sandbox:'read-only',config:configOverrides,developerInstructions:soul(input)};
    const resumed=Boolean(session?.id);
    const thread=await peer.request(resumed?'thread/resume':'thread/start',resumed?{...options,threadId:session.id}:{...options,ephemeral:false});
    const threadId=thread.thread?.id;if(typeof threadId!=='string'||!threadId)throw Error('Codex 未返回会话标识');
    await checkpoint({id:threadId});
    const content=[{type:'text',text:textInput(input,resumed)}],bindings=[];
    const images=imagesForTurn(input,resumed);
    if(images.length&&model&&!models.find(m=>m.id===model)?.image)throw Error('所选 Codex 模型不支持图片');
    for(const frameId of images){const index=content.length;content.push({type:'image',url:null});bindings.push({frameId,pointer:`/params/input/${index}/url`,encoding:'dataUrl'});}
    let answer='',reasoning='',completed,turnId;
    const early=[];
    const consume=async(method,params)=>{
      if(params.threadId!==threadId||(params.turnId??params.turn?.id)!==turnId)return;
      if(method==='item/agentMessage/delta'){answer+=params.delta??'';await stream(ctx,'agent.text',params.delta);}
      if(method==='item/reasoning/summaryTextDelta'){reasoning+=params.delta??'';await stream(ctx,'agent.reasoning',params.delta);}
      if(method==='turn/completed'){
        completed=params.turn;
      }
    };
    peer.notification=async(method,params)=>{
      if(params.threadId!==threadId||!['item/agentMessage/delta','item/reasoning/summaryTextDelta','turn/completed'].includes(method))return;
      if(!turnId){if(early.length>=1024)throw Error('Codex 提前事件过多');early.push([method,params]);return;}
      await consume(method,params);
    };
    const start=await peer.request('turn/start',{threadId,input:content,...(config.effort?{effort:config.effort}:{})},bindings);
    turnId=start.turn?.id;if(typeof turnId!=='string'||!turnId)throw Error('Codex 未返回当前轮次');
    for(const [method,params] of early)await consume(method,params);
    await peer.wait(()=>Boolean(completed));
    if(completed.status!=='completed')throw Error(completed.error?.message??'Codex 未完成本轮回答');
    if(!answer.trim())throw Error('Codex 未返回回答');
    return {content:answer,session:{id:threadId},reasoning};
  }finally{await peer.close();}
}
export async function connectAcp(ctx,config,previous){
  const peer=await openPeer(ctx,config);
  try{
    const initialized=await peer.request('initialize',{protocolVersion:1,clientCapabilities:{},clientInfo:{name:'mewu',title:'Mewu',version:'0.1.0'}});
    if(initialized.protocolVersion!==1)throw Error('ACP 协议版本不匹配');
    const resumed=Boolean(previous?.id&&initialized.agentCapabilities?.loadSession===true);
    const session=await peer.request(resumed?'session/load':'session/new',{...(resumed?{sessionId:previous.id}:{}),cwd:peer.cwd,mcpServers:[]});
    if(resumed)session.sessionId=previous.id;
    if(typeof session.sessionId!=='string'||!session.sessionId||session.sessionId.length>1024)throw Error('Agent 未返回 ACP 会话');
    const modelOption=session.configOptions?.find(option=>option.category==='model');
    const options=modelOption?.options??[];
    const flattened=options.flatMap(option=>Array.isArray(option.options)?option.options:option);
    const models=(session.models?.availableModels??flattened).map(model=>({id:model.modelId??model.value,name:model.name??model.modelId??model.value,default:(model.modelId??model.value)===(session.models?.currentModelId??modelOption?.currentValue)}))
      .filter(model=>typeof model.id==='string'&&model.id.length>0&&model.id.length<=300);
    return {peer,initialized,session,models,resumed};
  }catch(error){await peer.close();throw error;}
}
export async function selectAcpModel(connection,model){
  if(!model)return;
  const {peer,session,models}=connection;
  if(!models.some(value=>value.id===model))throw Error('该 Agent 未提供所选模型');
  const option=session.configOptions?.find(value=>value.category==='model');
  // Some clients reject even a redundant model change. The advertised current
  // selection already satisfies the request; do not mutate it unnecessarily.
  if((option?.currentValue??session.models?.currentModelId)===model)return;
  try{
    const result=option
      ?await peer.request('session/set_config_option',{sessionId:session.sessionId,configId:option.id,value:model})
      :await peer.request('session/set_model',{sessionId:session.sessionId,modelId:model});
    const current=result.configOptions?.find(value=>value.category==='model')?.currentValue;
    if(current!==undefined&&current!==model)throw Error('selection not applied');
  }catch{
    // Never leak an upstream error body (which can contain credentials/paths),
    // silently switch models or retry a prompt after an uncertain failure.
    throw Error('该 Agent 无法切换到所选模型，请选择默认模型或更新官方客户端');
  }
}
export async function runAcp(ctx,config,input,previous,checkpoint=async()=>{}){
  const {peer,initialized,session,models,resumed}=await connectAcp(ctx,config,previous);
  try{
    const sessionId=session.sessionId;
    await checkpoint({id:sessionId});
    await selectAcpModel({peer,session,models},config.model);
    const images=imagesForTurn(input,resumed);
    if(images.length&&initialized.agentCapabilities?.promptCapabilities?.image!==true)throw Error('该 Agent 未声明图片支持');
    const context=resumed&&soul(input)?`[Mewu 角色设定]\n${soul(input)}\n\n`:'';
    const prompt=[{type:'text',text:context+textInput(input,resumed)}],bindings=[];
    for(const frameId of images){const index=prompt.length;prompt.push({type:'image',mimeType:'image/png',data:null});bindings.push({frameId,pointer:`/params/prompt/${index}/data`,encoding:'base64'});}
    let content='';
    peer.notification=async(method,params)=>{
      if(method!=='session/update'||params.sessionId!==sessionId)return;
      const update=params.update;
      if(update?.content?.type!=='text')return;
      if(update.sessionUpdate==='agent_message_chunk'){content+=update.content.text;await stream(ctx,'agent.text',update.content.text);}
      if(update.sessionUpdate==='agent_thought_chunk')await stream(ctx,'agent.reasoning',update.content.text);
    };
    const result=await peer.request('session/prompt',{sessionId,prompt},bindings,250000);
    if(result.stopReason!=='end_turn'){
      let detail=result._meta?.['codebuddy.ai/errorMessage'];
      if(typeof detail==='string'){try{detail=JSON.parse(detail);}catch{detail=null;}}
      if(detail?.data?.category==='auth')throw Error('请先在该 Agent 的官方客户端或 CLI 中登录');
      throw Error(`Agent 未完成回答：${result.stopReason??'未知状态'}`);
    }
    if(!content.trim())throw Error('Agent 未返回回答');
    return {content,session:{id:sessionId}};
  }finally{await peer.close();}
}
