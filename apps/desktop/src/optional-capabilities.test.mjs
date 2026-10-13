// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import test from 'node:test';
const code=stripTypeScriptTypes(await readFile(new URL('./optional-capabilities.ts',import.meta.url),'utf8'),{mode:'transform'});
const {currentOptionalCapabilities,optionalCapabilityKey}=await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
const installed={revision:4,plugins:[{manifest:{id:'mewu.agent',contributions:[{id:'agent',kind:'agent.runtime'}]},revision:2,state:'enabled'}]};
const connections=[{id:'local',revision:3,hasKey:false},{id:'cloud',revision:9,hasKey:true}];
const receipt={pluginRevision:4,connectionRevisions:{local:3,cloud:9},agent:{pluginId:'mewu.agent',revision:2,contributionId:'agent'},blackboard:null,readyConnectionIds:['local']};
test('host readiness accepts an explicitly configured keyless provider; UI never infers readiness from hasKey',()=>{
 assert.equal(currentOptionalCapabilities(receipt,installed,connections),true);
 assert.deepEqual(receipt.readyConnectionIds,['local']);
 assert.equal(currentOptionalCapabilities({...receipt,readyConnectionIds:[]},installed,connections),true);
});
test('late credential/configuration and plugin responses cannot revive revoked UI',()=>{
 assert.equal(currentOptionalCapabilities(receipt,installed,[{...connections[0],revision:4},connections[1]]),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,revision:5},connections),false);
 for(const state of ['disabled','removed']) assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],state}]},connections),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],error:'unavailable'}]},connections),false);
 assert.equal(currentOptionalCapabilities(receipt,{...installed,plugins:[{...installed.plugins[0],revision:3}]},connections),false);
 assert.equal(currentOptionalCapabilities({...receipt,readyConnectionIds:['deleted']},installed,connections),false);
});
test('grants must name the correct installed capability; missing profiles fail closed',()=>{
 for(const malformed of [null,undefined,{}, {...receipt,readyConnectionIds:null}, {...receipt,agent:undefined}]) assert.equal(currentOptionalCapabilities(malformed,installed,connections),false);
 assert.equal(currentOptionalCapabilities({...receipt,blackboard:receipt.agent},installed,connections),false);
 assert.equal(currentOptionalCapabilities(receipt,installed,[connections[0]]),false);
 assert.equal(currentOptionalCapabilities({...receipt,agent:null,blackboard:null,readyConnectionIds:[]},installed,connections),true);
});
test('capability refresh identity ignores drawing snapshots and profile ordering but changes with credential revisions',()=>{
 const key=optionalCapabilityKey(installed,connections);
 assert.equal(optionalCapabilityKey(installed,[...connections].reverse()),key);
 assert.notEqual(optionalCapabilityKey(installed,[{...connections[0],revision:4},connections[1]]),key);
 assert.notEqual(optionalCapabilityKey({...installed,revision:5},connections),key);
});
