// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const load=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const actions=await load('./extension-actions.ts');
test('requests must carry a known action and a usable region identifier',()=>{
  assert.deepEqual(actions.parseExtensionActionRequest({id:'r1',action:'copy',regionId:'region-9'}),{id:'r1',action:'copy',regionId:'region-9'});
  assert.deepEqual(actions.parseExtensionActionRequest({id:'r1',action:'save',regionId:'region-9',variant:'original'}),{id:'r1',action:'save',regionId:'region-9',variant:'original'});
  // Native emits a null variant when the plugin omitted one.
  assert.deepEqual(actions.parseExtensionActionRequest({id:'r1',action:'reference',regionId:'region-9',variant:null}),{id:'r1',action:'reference',regionId:'region-9'});
  for(const payload of [
    null, 'copy', [],
    {id:'r1',action:'copy'},
    {id:'r1',regionId:'region-9'},
    {id:'r1',action:'copy',regionId:'region-9',extra:1},
    {id:'',action:'copy',regionId:'region-9'},
    {id:'r1',action:'evil',regionId:'region-9'},
    {id:'r1',action:'copy',regionId:'region-9',variant:'retouched'},
  ]) assert.throws(()=>actions.parseExtensionActionRequest(payload));
});
test('only bounded first-party text becomes an action failure reason',()=>{
  assert.equal(actions.extensionActionReason(new Error('已取消')),'已取消');
  assert.equal(actions.extensionActionReason('宿主正忙'),'宿主正忙');
  assert.equal(actions.extensionActionReason(new Error('')),'宿主动作执行失败');
  assert.equal(actions.extensionActionReason(new Error('x'.repeat(257))),'宿主动作执行失败');
  assert.equal(actions.extensionActionReason(new Error('a\nb')),'宿主动作执行失败');
});
test('facade resolves success, executor failures and malformed payloads exactly once',async()=>{
  const settle=()=>new Promise(done=>setImmediate(done));
  let handler;
  const replies=[];
  const stop=await actions.attachExtensionActions({
    execute:async request=>request.action==='pin'?{ok:false,reason:'没有可用的贴图功能'}:{ok:true},
    resolve:(id,ok,reason)=>{replies.push([id,ok,reason]);return Promise.resolve();},
  },(name,h)=>{handler=h;return Promise.resolve(()=>{});});
  handler({payload:{id:'a1',action:'copy',regionId:'r'}});
  handler({payload:{id:'a2',action:'pin',regionId:'r'}});
  handler({payload:{id:'a3',action:'copy'}});
  handler({payload:'junk'});
  await settle();await settle();
  // Reply ordering follows each request's own async settle; compare as a set.
  assert.deepEqual([...replies].sort((left,right)=>left[0]<right[0]?-1:1),[
    ['a1',true,undefined],
    ['a2',false,'没有可用的贴图功能'],
    ['a3',false,'宿主动作请求无效'],
  ]);
  stop();
  const errors=[];
  const stopThrowing=await actions.attachExtensionActions({
    execute:async()=>{throw new Error('已取消');},
    resolve:(id,ok,reason)=>{errors.push([id,ok,reason]);return Promise.resolve();},
  },(name,h)=>{handler=h;return Promise.resolve(()=>{});});
  handler({payload:{id:'b1',action:'save',regionId:'r',variant:'annotated'}});
  await settle();
  assert.deepEqual(errors,[['b1',false,'已取消']]);
  stopThrowing();
});
