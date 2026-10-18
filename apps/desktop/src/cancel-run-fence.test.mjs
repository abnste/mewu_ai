// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
import {createContext,runInContext} from 'node:vm';
const bridgeSource=await readFile(new URL('./bridge.ts',import.meta.url),'utf8');
const appSource=await readFile(new URL('./App.tsx',import.meta.url),'utf8');
function functionSource(source,name){
 const nodes=[];
 function walk(n){if(!n||typeof n!=='object')return;if(n.type==='FunctionDeclaration'&&n.id?.name===name)nodes.push(n);for(const[k,v]of Object.entries(n)){if(k==='loc')continue;if(Array.isArray(v))v.forEach(walk);else if(v&&typeof v==='object')walk(v);}}
 walk(parse(source,{sourceType:'module',plugins:['typescript','jsx']}));assert.equal(nodes.length,1,name);
 return stripTypeScriptTypes(source.slice(nodes[0].start,nodes[0].end),{mode:'transform'});
}
const bridgeCancel=functionSource(bridgeSource,'cancelRun'),appCancel=functionSource(appSource,'cancel');
function bind(source,name,globals){const context=createContext(globals);runInContext(source,context);return context[name];}
test('native cancellation sends the captured expected run ID without changing the command name',async()=>{
 const calls=[];const cancel=bind(bridgeCancel,'cancelRun',{native:true,invokeSnapshot:async(...args)=>{calls.push(args);return 'saved';}});
 assert.equal(await cancel('scene','old-run'),'saved');assert.deepEqual(JSON.parse(JSON.stringify(calls)),[['cancel_run',{sceneId:'scene',expectedRunId:'old-run'}]]);
});
test('browser preview refuses a stale old run and preserves the newer run and publication count',async()=>{
 const scene={id:'scene',run:{id:'new-run',status:'running'}};let publications=0;
 const cancel=bind(bridgeCancel,'cancelRun',{native:false,findScene:()=>scene,publish:()=>{publications++;return scene;}});
 await assert.rejects(cancel('scene','old-run'),/运行已取消、结束或被替换，拒绝迟到结果/);
 assert.equal(scene.run.status,'running');assert.equal(scene.run.id,'new-run');assert.equal(publications,0);
 await cancel('scene','new-run');assert.equal(scene.run.status,'canceled');assert.equal(publications,1);
 await assert.rejects(cancel('scene','new-run'),/运行已取消、结束或被替换，拒绝迟到结果/);assert.equal(publications,1);
});
test('the real App cancel handler captures its current scene and run before the asynchronous reply',async()=>{
 let current={id:'scene',run:{id:'old-run',status:'running'}},release;const calls=[],accepted=[],errors=[];
 const cancel=bind(appCancel,'cancel',{scene:()=>current,bridge:{cancelRun:(...args)=>{calls.push(args);return new Promise(r=>{release=r;});}},accept:value=>accepted.push(value),showError:error=>errors.push(error)});
 const pending=cancel();current={id:'scene',run:{id:'new-run',status:'running'}};release('authoritative-snapshot');await pending;
 assert.deepEqual(JSON.parse(JSON.stringify(calls)),[['scene','old-run']]);assert.deepEqual(accepted,['authoritative-snapshot']);assert.equal(errors.length,0);
});
test('the real App handler dispatches no cancellation without a currently running identity',async()=>{
 for(const current of [undefined,{id:'scene'}, {id:'scene',run:{id:'run',status:'completed'}},{id:'scene',run:{id:'run',status:'canceled'}}]){
  const cancel=bind(appCancel,'cancel',{scene:()=>current,bridge:{cancelRun:()=>{throw Error('unexpected call');}},accept:()=>{throw Error('unexpected accept');},showError:()=>{throw Error('unexpected error');}});await cancel();
 }
});
