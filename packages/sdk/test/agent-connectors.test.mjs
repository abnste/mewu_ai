// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import activate from '../../../extensions/official/agent-connectors/index.mjs';
import {connectCodex,runCodex,connectAcp,runAcp} from '../../../extensions/official/agent-connectors/adapters.mjs';
import {openPeer} from '../../../extensions/official/agent-connectors/rpc.mjs';
import {profiles,detectProfile,validateConfig,resolveConfig} from '../../../extensions/official/agent-connectors/profiles.mjs';
import {workbuddyProcess} from '../../../extensions/official/agent-connectors/workbuddy.mjs';

function fixture(answer){
  const queue=[],writes=[],events=[],services=new Map(),data=new Map();let closed=0;
  const ctx={
    runtime:{describe:async()=>({hostApi:2,capabilities:['ui.settings','configuration.shared','process.jsonlines','process.frame-bindings']})},
    configuration:{get:async key=>data.get('config/'+key)??null,compareAndSet:async(key,old,value)=>{
      if(JSON.stringify(data.get('config/'+key)??null)!==JSON.stringify(old))return false;data.set('config/'+key,structuredClone(value));return true;
    }},
    storage:{compareAndSet:async(key,expected,value)=>{if(JSON.stringify(data.get(key)??null)!==JSON.stringify(expected))return false;data.set(key,structuredClone(value));return true;},get:async key=>data.get(key)??null,set:async(key,value)=>{data.set(key,structuredClone(value));}},
    processes:{resolve:async()=>({platform:'windows',executables:['C:/local/agent.exe']}),open:async()=>({id:'process',cwd:'C:/workspace'}),close:async()=>{closed++;},
      write:async(id,message,bindings)=>{writes.push({message:structuredClone(message),bindings:structuredClone(bindings)});const reply=answer(message,queue);if(reply!==undefined)queue.push({id:message.id,result:reply});},
      read:async()=>queue.shift()??null},
    events:{emit:async(name,value)=>{events.push({name,value});}},
    ui:{register:(descriptor,handler)=>services.set(descriptor.id,{descriptor,handler})},
    services:{register:(descriptor,handler)=>services.set(descriptor.id,{descriptor,handler})},
    agents:{register:(descriptor,handler)=>services.set(descriptor.id,{descriptor,handler})}
  };
  return {ctx,writes,events,services,data,queue,closed:()=>closed};
}
const config={executable:'C:/local/agent.exe',args:[]};
function codex(message,queue){
  switch(message.method){
    case 'initialize':return {userAgent:'codex'};
    case 'account/read':return {account:{type:'chatgpt'}};
    case 'model/list':return {data:[{model:'model',displayName:'Model',inputModalities:['image'],supportedReasoningEfforts:[]}],nextCursor:null};
    case 'config/read':return {config:{mcp_servers:{existing:{}}}};
    case 'thread/start':case 'thread/resume':return {thread:{id:'thread'}};
    case 'turn/start':
      queue.push({method:'item/agentMessage/delta',params:{threadId:'unrelated',turnId:'alien',delta:'must not leak'}});
      queue.push({method:'item/agentMessage/delta',params:{threadId:'thread',turnId:'previous-turn',delta:'old answer must not leak'}});
      queue.push({method:'item/reasoning/summaryTextDelta',params:{threadId:'thread',turnId:'previous-turn',delta:'old thought must not leak'}});
      queue.push({method:'turn/completed',params:{threadId:'thread',turn:{id:'previous-turn',status:'completed'}}});
      queue.push({method:'item/reasoning/summaryTextDelta',params:{threadId:'thread',turnId:'turn',delta:'思考'}});
      queue.push({method:'item/agentMessage/delta',params:{threadId:'thread',turnId:'turn',delta:'回答'}});
      queue.push({method:'turn/completed',params:{threadId:'thread',turn:{id:'turn',status:'completed'}}});
      return {turn:{id:'turn'}};
  }
}
function acp(message,queue){
  switch(message.method){
    case 'initialize':return {protocolVersion:1,agentCapabilities:{loadSession:true,promptCapabilities:{image:true}}};
    case 'session/new':return {sessionId:'session',models:{availableModels:[{modelId:'model',name:'Model'}]}};
    case 'session/load':return {models:{availableModels:[{modelId:'model',name:'Model'}]}};
    case 'session/set_model':return {};
    case 'session/prompt':
      queue.push({method:'session/update',params:{sessionId:'other',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'foreign'}}}});
      for(const [kind,text] of [['agent_thought_chunk','思考'],['agent_message_chunk','答'],['agent_message_chunk','案']])queue.push({method:'session/update',params:{sessionId:message.params.sessionId,update:{sessionUpdate:kind,content:{type:'text',text}}}});
      return {stopReason:'end_turn'};
  }
}
test('one independent package contributes settings and four Agent services without provider branches in host',async()=>{
  const f=fixture(()=>undefined);await activate(f.ctx);
  assert.deepEqual([...f.services.keys()],['settings','models-codex','codex','models-minimax','minimax','models-workbuddy','workbuddy','models-hermes','hermes']);
  assert.deepEqual(f.services.get('settings').descriptor.metadata,{surface:'settings'});
  const mount=await f.services.get('settings').handler({type:'mount'});
  assert.match(mount.html,/测试连接/);assert.equal(f.writes.length,0);
});
test('settings configuration survives reload, shares across roles and rejects stale provider edits',async()=>{
  const f=fixture(()=>undefined);await activate(f.ctx);const ui=f.services.get('settings').handler;
  assert.equal((await ui({type:'detect',provider:'codex'})).executable,config.executable);
  await ui({type:'save',provider:'codex',config,expected:null});
  assert.equal((await ui({type:'get'})).connections.codex.executable,config.executable);
  await assert.rejects(ui({type:'save',provider:'codex',config:{...config,model:'another'},expected:null}),/已变化/);
  const second=fixture(codex);second.data.set('config/connections',f.data.get('config/connections'));await activate(second.ctx);
  const result=await second.services.get('codex').handler({task:'继续',messages:[{role:'user',content:'继续'}]}, {context:{agentId:'other-role',sceneId:'scene'}});
  assert.equal(result.content,'回答');assert.ok(second.data.has('sessions/codex/other-role/scene'));
});
test('Codex tests actual account/model handshake and keeps reasoning separate from answer',async()=>{
  const f=fixture(codex),connected=await connectCodex(f.ctx,config);
  assert.equal(connected.models[0].id,'model');await connected.peer.close();assert.equal(f.closed(),1);
  const run=fixture(codex),result=await runCodex(run.ctx,{...config,model:'model'},{task:'看图',messages:[{role:'system',content:'灵魂'},{role:'user',content:'看图'}],images:['frame']});
  assert.equal(result.content,'回答');assert.equal(result.reasoning,'思考');assert.equal(run.closed(),1);
  assert.deepEqual(run.events.map(e=>e.name),['agent.reasoning','agent.text']);
  const start=run.writes.find(v=>v.message.method==='thread/start').message.params;
  assert.equal(start.developerInstructions,'灵魂');assert.equal(start.config['mcp_servers.existing.enabled'],false);
  const turn=run.writes.find(v=>v.message.method==='turn/start');
  assert.deepEqual(turn.bindings,[{frameId:'frame',pointer:'/params/input/1/url',encoding:'dataUrl'}]);
  assert.equal(turn.message.params.input[1].url,null);
});
test('ACP streams only the correct session and resumes without creating a redundant session',async()=>{
  const f=fixture(acp);const result=await runAcp(f.ctx,{...config,model:'model'}, {task:'本轮',messages:[{role:'user',content:'旧消息'},{role:'user',content:'本轮'}],images:['frame']}, {id:'continued'});
  assert.equal(result.content,'答案');assert.equal(result.session.id,'continued');
  assert.equal(f.writes.some(v=>v.message.method==='session/new'),false);
  const prompt=f.writes.find(v=>v.message.method==='session/prompt');
  assert.equal(prompt.message.params.prompt[0].text,'本轮');assert.equal(prompt.bindings[0].frameId,'frame');
  assert.deepEqual(f.events.map(e=>e.name),['agent.reasoning','agent.text','agent.text']);assert.equal(f.closed(),1);
});
test('missing login, invalid image capability and protocol errors never report a working connection',async()=>{
  const unauth=fixture((message,q)=>message.method==='account/read'?{account:null}:codex(message,q));
  await assert.rejects(connectCodex(unauth.ctx,config),/登录/);assert.equal(unauth.closed(),1);
  const incompatible=fixture((message,q)=>message.method==='initialize'?{protocolVersion:2}:acp(message,q));
  await assert.rejects(connectAcp(incompatible.ctx,config),/版本/);assert.equal(incompatible.closed(),1);
  const image=fixture((message,q)=>message.method==='initialize'?{protocolVersion:1,agentCapabilities:{}}:acp(message,q));
  await assert.rejects(runAcp(image.ctx,config,{task:'图',images:['frame']}),/图片/);assert.equal(image.closed(),1);
});
test('server permission requests are answered explicitly and unsupported host tools are not executed',async()=>{
  const f=fixture(()=>undefined),peer=await openPeer(f.ctx,config);
  f.queue.push({id:'permission',method:'session/request_permission',params:{options:[{optionId:'no',kind:'reject_once'},{optionId:'yes',kind:'allow_always'}]}});
  await peer.poll();assert.deepEqual(f.writes.at(-1).message.result,{outcome:{outcome:'selected',optionId:'no'}});
  f.queue.push({id:'fs',method:'fs/read_text_file',params:{path:'unrelated'}});await peer.poll();
  assert.equal(f.writes.at(-1).message.error.code,-32601);await peer.close();await peer.close();assert.equal(f.closed(),1);
});
test('resumed ACP sessions receive only this turn images, without repeatedly uploading historical screenshots',async()=>{
  const f=fixture(acp);
  await runAcp(f.ctx,config,{task:'继续',images:['historical','current'],currentImages:['current']},{id:'continued'});
  const prompt=f.writes.find(v=>v.message.method==='session/prompt');
  assert.deepEqual(prompt.bindings.map(v=>v.frameId),['current']);
});
test('remote session is checkpointed before prompt dispatch, even when that turn later fails',async()=>{
  const f=fixture((message,q)=>message.method==='session/prompt'?{stopReason:'cancelled'}:acp(message,q));
  const checkpoints=[];
  await assert.rejects(runAcp(f.ctx,config,{task:'本轮'},undefined,async session=>{
    assert.equal(f.writes.some(v=>v.message.method==='session/prompt'),false);checkpoints.push(session);
  }),/未完成/);
  assert.deepEqual(checkpoints,[{id:'session'}]);assert.equal(f.closed(),1);
});
test('ACP grouped model options are flattened and restored sessions receive current soul instructions',async()=>{
  const f=fixture((message,q)=>message.method==='session/load'?{configOptions:[{id:'model',category:'model',options:[{group:'Provider',options:[{value:'fast',name:'Fast'}]}]}]}:acp(message,q));
  await runAcp(f.ctx,config,{task:'继续',messages:[{role:'system',content:'最新灵魂'}]},{id:'continued'});
  const prompt=f.writes.find(v=>v.message.method==='session/prompt').message.params.prompt[0].text;
  assert.match(prompt,/最新灵魂/);assert.match(prompt,/继续/);
  const c=fixture((message,q)=>message.method==='session/new'?{sessionId:'one',configOptions:[{id:'model',category:'model',currentValue:'fast',options:[{group:'Provider',options:[{value:'fast',name:'Fast'}]}]}]}:acp(message,q));
  const connected=await connectAcp(c.ctx,config);assert.deepEqual(connected.models,[{id:'fast',name:'Fast',default:true}]);await connected.peer.close();
});

test('WorkBuddy desktop discovery selects the plugin HTTP bridge rather than its broken stdio entry',async()=>{
  const f=fixture(acp);let calls=0;
  f.ctx.processes.resolve=async()=>++calls===1?{platform:'windows',executables:[]}:{platform:'windows',executables:['C:/Program Files/WorkBuddy/WorkBuddy.exe'],files:['C:/Program Files/WorkBuddy/resources/app.asar.unpacked/cli/bin/codebuddy']};
  const profile=profiles.find(p=>p.id==='workbuddy'),found=await detectProfile(f.ctx,profile);
  assert.equal(found.transport,'workbuddy-http');assert.equal(found.args.length,1);
  assert.equal(validateConfig(found,profile).transport,'workbuddy-http');
  assert.throws(()=>validateConfig(found,profiles[0]),/方式/);
});

test('WorkBuddy discovery sends only absolute candidates for the actual host platform',async()=>{
  for(const platform of ['windows','macos','linux']){
    const f=fixture(acp);let calls=0;
    f.ctx.processes.resolve=async(candidates,files=[])=>{
      calls++;
      if(calls===1)return {platform,executables:[]};
      const expand=value=>value.replace('${programFiles}','C:/Program Files').replace('${localAppData}','C:/Users/Test/AppData/Local');
      const isAbsolute=platform==='windows'?path.win32.isAbsolute:path.posix.isAbsolute;
      assert.ok(candidates.every(value=>isAbsolute(expand(value))));
      assert.ok(files.every(value=>isAbsolute(expand(value))));
      assert.equal(candidates.some(value=>value.startsWith('/Applications')),platform==='macos');
      return {platform,executables:[],files:[]};
    };
    assert.equal((await detectProfile(f.ctx,profiles.find(p=>p.id==='workbuddy'))).executable,'');
    assert.equal(calls,platform==='linux'?1:2);
  }
});

test('WorkBuddy bridge accepts the native resolver Windows namespace script path',()=>{
  const executable=String.raw`\\?\C:\Program Files\WorkBuddy\WorkBuddy.exe`;
  const cli=String.raw`\\?\C:\Program Files\WorkBuddy\resources\app.asar.unpacked\cli\bin\codebuddy`;
  const command=workbuddyProcess({executable,args:[cli],transport:'workbuddy-http'});
  assert.equal(command.executable,executable);
  assert.equal(JSON.parse(command.args[2]).cli,cli.slice(4));
  const unix='/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/cli/bin/codebuddy';
  assert.equal(JSON.parse(workbuddyProcess({args:[unix],transport:'workbuddy-http'}).args[2]).cli,unix);
});

test('WorkBuddy upstream authentication failures are reported as login requirements rather than opaque refusals',async()=>{
  const f=fixture((message,q)=>message.method==='session/prompt'?{stopReason:'refusal',_meta:{'codebuddy.ai/errorMessage':JSON.stringify({code:-32000,message:'Authentication required',data:{category:'auth'}})}}:acp(message,q));
  await assert.rejects(runAcp(f.ctx,config,{task:'hello'}),/官方客户端或 CLI 中登录/);assert.equal(f.closed(),1);
});

test('independent Agent emits only an intentional public login diagnostic before failing its run',async()=>{
  const f=fixture((message,q)=>message.method==='session/prompt'?{stopReason:'refusal',_meta:{'codebuddy.ai/errorMessage':JSON.stringify({data:{category:'auth'}})}}:acp(message,q));
  f.data.set('config/connections',{workbuddy:config});await activate(f.ctx);
  await assert.rejects(f.services.get('workbuddy').handler({task:'hello'},{context:{agentId:'a',sceneId:'s'}}),/登录/);
  assert.deepEqual(f.events,[{name:'agent.text',value:'请先在该 Agent 的官方客户端或 CLI 中登录'}]);
});


test('Codex custom-provider auth does not require an unrelated OpenAI login',async()=>{
 const f=fixture((m,q)=>m.method==='account/read'?{account:null,requiresOpenaiAuth:false}:codex(m,q));
 const connected=await connectCodex(f.ctx,config);assert.equal(connected.models.length,1);await connected.peer.close();
});
test('official Node CLI layouts resolve on Windows, macOS and Linux without shell shims or user-specific directories',async()=>{
 for(const platform of ['windows','macos','linux'])for(const provider of ['codex','minimax']){
  const f=fixture(()=>{}),profile=profiles.find(p=>p.id===provider);
  const node=platform==='windows'?String.raw`\\?\C:\Custom Node\node.exe`:'/opt/custom/bin/node';
  const script=platform==='windows'?String.raw`\\?\C:\Users\Other User\CLI\entry.js`:'/home/other/CLI/entry.js';
  f.ctx.processes.resolve=async(candidates,files=[])=>{
   if(platform==='windows')assert.ok(candidates.every(p=>!p.startsWith('/')));
   if(candidates.includes('node'))return {platform,executables:[node]};
   if(files.length)return {platform,executables:[],files:[script]};
   return {platform,executables:[],files:[]};
  };
  const found=await detectProfile(f.ctx,profile);
  assert.equal(found.executable,node);assert.equal(found.launchMode,'auto');
  assert.deepEqual(found.args,[platform==='windows'?script.slice(4):script,...profile.args]);
 }
});
test('automatic detection follows upgrades; manual executable, args and environment are preserved',async()=>{
 const f=fixture(()=>{}),profile=profiles[0];
 const manual={...config,args:['custom'],cwd:'C:/workspace',env:{CUSTOM:'kept'}};
 assert.deepEqual(await resolveConfig(f.ctx,manual,profile),manual);
 f.ctx.processes.resolve=async()=>({platform:'windows',executables:['C:/new version/codex.exe']});
 const next=await resolveConfig(f.ctx,{...manual,launchMode:'auto'},profile);
 assert.equal(next.executable,'C:/new version/codex.exe');assert.deepEqual(next.args,profile.args);assert.deepEqual(next.env,manual.env);assert.equal(next.cwd,manual.cwd);
});
test('desktop version directories are requested only through the advertised generic host capability',async()=>{
 const f=fixture(()=>{});f.ctx.runtime.describe=async()=>({capabilities:['process.installation-search']});
 f.ctx.processes.resolve=async(candidates,files,searches)=>{
  if(searches){assert.equal(searches.length,2);assert.equal(searches[0].root,'${localAppData}/OpenAI/Codex/bin');return {platform:'windows',executables:['C:/desktop/new/codex.exe']};}
  return {platform:'windows',executables:[]};
 };
 assert.equal((await detectProfile(f.ctx,profiles[0])).executable,'C:/desktop/new/codex.exe');
});
test('model catalogs are live, selection is CAS-protected and scoped to the conversation',async()=>{
 const f=fixture(codex);f.data.set('config/connections',{codex:{...config,model:'retired'}});await activate(f.ctx);
 const service=f.services.get('models-codex').handler,context={context:{agentId:'a',sceneId:'one'}};
 assert.equal(f.services.get('codex').descriptor.metadata.modelsService,'models-codex');
 const listed=await service({type:'list'},context);assert.equal(listed.ok,true);assert.equal(listed.selectedModelId,'retired');assert.equal(f.closed(),1);
 const chosen=await service({type:'select',modelId:'model',expected:null},context);assert.equal(chosen.ok,true);
 assert.equal((await service({type:'select',modelId:'',expected:null},context)).ok,false);
 assert.equal((await service({type:'select',modelId:'invalid',expected:chosen.selection},context)).ok,false);
 assert.equal((await service({type:'list'},{context:{agentId:'a',sceneId:'two'}})).selectedModelId,'retired');
 await f.services.get('codex').handler({task:'hello'},context);
 assert.equal(f.writes.find(w=>w.message.method==='thread/start').message.params.model,'model');
 assert.equal((await service({type:'list'},{context:{}})).ok,false);
});

test('Codex default uses the live catalog instead of a stale CLI preference',async()=>{
 const f=fixture((m,q)=>m.method==='model/list'?{data:[{model:'current',displayName:'Current',isDefault:true,inputModalities:['image']}]}:codex(m,q));
 await runCodex(f.ctx,config,{task:'hello'});
 assert.equal(f.writes.find(w=>w.message.method==='thread/start').message.params.model,'current');
});

function configurableAcp(message,queue){
 if(message.method==='session/new')return {sessionId:'configured',configOptions:[{id:'models',category:'model',currentValue:'working',options:[{value:'working',name:'Working'},{value:'broken',name:'Broken'}]}]};
 if(message.method==='session/set_config_option'){
  queue.push({id:message.id,error:{code:-32603,message:'Internal error',data:{details:'private provider body'}}});return;
 }
 return acp(message,queue);
}
test('ACP keeps an already selected model without a redundant unsupported mutation',async()=>{
 const f=fixture(configurableAcp);await runAcp(f.ctx,{...config,model:'working'},{task:'hello'});
 assert.equal(f.writes.some(v=>v.message.method==='session/set_config_option'),false);
 assert.equal(f.writes.filter(v=>v.message.method==='session/prompt').length,1);
});
test('ACP model picker validates with the client before saving and preserves a working choice on failure',async()=>{
 const f=fixture(configurableAcp);f.data.set('config/connections',{minimax:config});await activate(f.ctx);
 const handler=f.services.get('models-minimax').handler,context={context:{agentId:'a',sceneId:'one'}};
 const chosen=await handler({type:'select',modelId:'working',expected:null},context);assert.equal(chosen.ok,true);
 const rejected=await handler({type:'select',modelId:'broken',expected:chosen.selection},context);
 assert.equal(rejected.ok,false);assert.match(rejected.error,/无法切换/);assert.doesNotMatch(rejected.error,/private/);
 assert.deepEqual(f.data.get('model-selection/minimax/one'),{modelId:'working'});
 assert.equal(f.writes.some(v=>v.message.method==='session/prompt'),false);assert.equal(f.closed(),2);
});
test('ACP failed model changes stop before the prompt and expose only a public diagnostic',async()=>{
 const f=fixture(configurableAcp);f.data.set('config/connections',{minimax:{...config,model:'broken'}});await activate(f.ctx);
 await assert.rejects(f.services.get('minimax').handler({task:'hello'},{context:{agentId:'a',sceneId:'one'}}),/无法切换/);
 assert.equal(f.writes.some(v=>v.message.method==='session/prompt'),false);assert.equal(f.closed(),1);
 assert.deepEqual(f.events,[{name:'agent.text',value:'该 Agent 无法切换到所选模型，请选择默认模型或更新官方客户端'}]);
});

test('upstream prompt failures emit safe actionable diagnostics without replaying a task',async()=>{
 for(const [code,message,expected] of [
  [-32000,'private credential content',/登录/],
  [-32603,'429 rate limit private credential content',/额度/],
  [-32603,'private credential content',/模型.*权限/]
 ]){
  const f=fixture((m,q)=>{
   if(m.method==='session/prompt'){q.push({id:m.id,error:{code,message}});return;}
   return acp(m,q);
  });
  f.data.set('config/connections',{minimax:config});await activate(f.ctx);
  await assert.rejects(f.services.get('minimax').handler({task:'hello'},{context:{agentId:'a',sceneId:'one'}}));
  assert.equal(f.writes.filter(v=>v.message.method==='session/prompt').length,1);
  assert.equal(f.closed(),1);
  assert.equal(f.events.filter(v=>v.name==='agent.text').length,1);
  assert.match(f.events[0].value,expected);assert.doesNotMatch(JSON.stringify(f.events),/private|credential/);
 }
});
