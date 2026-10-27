// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
const code=stripTypeScriptTypes(await readFile(new URL('./optional-capabilities.ts',import.meta.url),'utf8'),{mode:'transform'});
const {currentOptionalCapabilities,optionalCapabilityKey,optionalAgentAvailable,optionalAgentReady,settleOptionalAgentReadiness}=await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
const installed={revision:4,plugins:[{manifest:{id:'mewu.agent',contributions:[{id:'agent',kind:'agent.runtime'}]},revision:2,state:'enabled'}]};
const connections=[{id:'local',revision:3,hasKey:false},{id:'cloud',revision:9,hasKey:true}];
const receipt={pluginRevision:4,connectionRevisions:{local:3,cloud:9},agent:{pluginId:'mewu.agent',revision:2,contributionId:'agent'},blackboard:null,configuredConnectionIds:['local'],readyAgentIds:[],boundAgentIds:[]};

test('explicit send awaits delayed capability metadata once, without any model or mutation replay',async()=>{
 let ready=false,reads=0,release;const scene={id:'scene',agentId:'agent',connectionId:'local'};
 const pending=settleOptionalAgentReadiness(()=>scene,()=>ready,()=>{reads++;return new Promise(resolve=>release=()=>{ready=true;resolve();});});
 let settled=false;void pending.then(()=>settled=true);await Promise.resolve();assert.equal(settled,false);assert.equal(reads,1);
 release();assert.equal(await pending,'ready');assert.equal(reads,1);
 assert.equal(await settleOptionalAgentReadiness(()=>scene,()=>ready,async()=>{reads++;}),'ready');assert.equal(reads,1);
});
test('a send metadata refresh cannot redirect the original request to another scene, Agent, model or capture',async()=>{
 for(const patch of [{id:'other'},{agentId:'other'},{connectionId:'other'},{background:{id:'other'}}]){
  let scene={id:'scene',agentId:'agent',connectionId:'local',background:{id:'capture'}},ready=false;
  assert.equal(await settleOptionalAgentReadiness(()=>scene,()=>ready,async()=>{scene={...scene,...patch};ready=true;}),'changed');
 }
});
test('revoked or unavailable capabilities remain unavailable after one bounded read',async()=>{
 let reads=0;const scene={id:'scene',agentId:'agent',connectionId:'local'};
 assert.equal(await settleOptionalAgentReadiness(()=>scene,()=>false,async()=>{reads++;}),'unavailable');assert.equal(reads,1);
 assert.equal(await settleOptionalAgentReadiness(()=>undefined,()=>false,async()=>{reads++;}),'unavailable');assert.equal(reads,1);
 await assert.rejects(settleOptionalAgentReadiness(()=>scene,()=>false,async()=>{throw Error('metadata failed');}),/metadata failed/);
});
test('real send settles metadata before claiming a send and still rechecks all runtime guards',async()=>{
 const app=await readFile(new URL('./App.tsx',import.meta.url),'utf8');const send=app.slice(app.indexOf('  async function send('),app.indexOf('  async function cancel('));
 assert.ok(send.indexOf('await settleOptionalAgentReadiness(')<send.indexOf('setSending(true)'));
 assert.ok(send.indexOf('await settleOptionalAgentReadiness(')<send.indexOf('const current = scene()'));
 assert.match(send,/if \(!agentReady\(\) \|\| !current \|\| busy\(\)/);
});
test('host readiness accepts an explicitly configured keyless provider; UI never infers readiness from hasKey',()=>{
 assert.equal(currentOptionalCapabilities(receipt,installed,connections),true);
 assert.deepEqual(receipt.configuredConnectionIds,['local']);
 assert.equal(currentOptionalCapabilities({...receipt,configuredConnectionIds:[]},installed,connections),true);
});
test('configured metadata keeps Composer eligible without claiming a live credential unlock',()=>{
 assert.equal(optionalAgentAvailable(receipt),true);
 assert.equal(optionalAgentReady(receipt,{agentId:'local-agent',connectionId:'local'}),true);
 const legacyShape={...receipt,readyConnectionIds:['local']};delete legacyShape.configuredConnectionIds;
 assert.equal(currentOptionalCapabilities(legacyShape,installed,connections),false,'execution-ready and configuration fields cannot be silently confused');
 assert.equal(optionalAgentAvailable({...receipt,configuredConnectionIds:[]}),false);
});
test('late credential/configuration and plugin responses cannot revive revoked UI',()=>{
 assert.equal(currentOptionalCapabilities(receipt,installed,[{...connections[0],revision:4},connections[1]]),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,revision:5},connections),false);
 for(const state of ['disabled','removed']) assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],state}]},connections),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],error:'unavailable'}]},connections),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],revision:3}]},connections),false);
 assert.equal(currentOptionalCapabilities({...receipt,configuredConnectionIds:['deleted']},installed,connections),false);
});
test('grants must name the correct installed capability; missing profiles fail closed',()=>{
 for(const malformed of [null,undefined,{}, {...receipt,configuredConnectionIds:null}, {...receipt,agent:undefined}]) assert.equal(currentOptionalCapabilities(malformed,installed,connections),false);
 assert.equal(currentOptionalCapabilities({...receipt,blackboard:receipt.agent},installed,connections),false);
 assert.equal(currentOptionalCapabilities(receipt,installed,[connections[0]]),false);
 assert.equal(currentOptionalCapabilities({...receipt,agent:null,blackboard:null,configuredConnectionIds:[]},installed,connections),true);
});
test('capability refresh identity ignores drawing snapshots and profile ordering but changes with credential revisions',()=>{
 const key=optionalCapabilityKey(installed,connections);
 assert.equal(optionalCapabilityKey(installed,[...connections].reverse()),key);
 assert.notEqual(optionalCapabilityKey(installed,[{...connections[0],revision:4},connections[1]]),key);
 assert.notEqual(optionalCapabilityKey({...installed,revision:5},connections),key);
});

test('independent Agent can be ready without host model or old Agent plugin, but invalid bindings never fall back',()=>{
 const configured={activeSceneId:'scene',activeScene:{id:'scene',agentId:'external',connectionId:null},agents:[{id:'external',defaultConnectionId:null}],defaultConnectionId:null};
 const external={...receipt,agent:null,configuredConnectionIds:[],boundAgentIds:['external'],readyAgentIds:['external']};
 assert.equal(currentOptionalCapabilities(external,installed,connections,configured),true);
 assert.equal(optionalAgentAvailable(external),true);assert.equal(optionalAgentReady(external,configured.activeScene),true);
 const invalid={...receipt,boundAgentIds:['external'],readyAgentIds:[]};
 assert.equal(optionalAgentReady(invalid,{agentId:'external',connectionId:'local'}),false,'disabled binding must not use ready default connection');
 assert.equal(optionalAgentReady(invalid,{agentId:'legacy',connectionId:'local'}),true);
 assert.equal(currentOptionalCapabilities({...external,readyAgentIds:['unknown']},installed,connections,configured),false);
 assert.equal(currentOptionalCapabilities({...external,boundAgentIds:['unknown'],readyAgentIds:[]},installed,connections,configured),false);
 for(const patch of [{readyAgentIds:null},{boundAgentIds:null},{boundAgentIds:['external','external']},{readyAgentIds:['external',null]}])assert.equal(currentOptionalCapabilities({...external,...patch},installed,connections,configured),false);
});

test('Agent configuration key reacts only to active selection and provider defaults, not drawings or instructions',()=>{
 const configured={activeSceneId:'scene',activeScene:{id:'scene',agentId:'external',connectionId:'local'},agents:[{id:'external',defaultConnectionId:null}],defaultConnectionId:'local'};
 const key=optionalCapabilityKey(installed,connections,configured);
 assert.equal(optionalCapabilityKey(installed,connections,{...configured,activeScene:{...configured.activeScene,drawings:['changed']},agents:[{...configured.agents[0],instructions:'changed soul'}]}),key);
 for(const patch of [{activeSceneId:'other'},{activeScene:{...configured.activeScene,connectionId:null}},{activeScene:{...configured.activeScene,agentId:'legacy'}},{defaultConnectionId:'cloud'},{agents:[{...configured.agents[0],defaultConnectionId:'cloud'}]}])assert.notEqual(optionalCapabilityKey(installed,connections,{...configured,...patch}),key);
});
