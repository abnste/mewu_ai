// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
const load=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const actions=await load('./extension-actions.ts');
test('requests must carry a known action and a usable region identifier',()=>{
  assert.deepEqual(actions.parseExtensionActionRequest({sceneId:'scene-A',id:'r1',action:'copy',regionId:'region-9'}),{sceneId:'scene-A',id:'r1',action:'copy',regionId:'region-9'});
  assert.deepEqual(actions.parseExtensionActionRequest({sceneId:'scene-A',id:'r1',action:'save',regionId:'region-9',variant:'original'}),{sceneId:'scene-A',id:'r1',action:'save',regionId:'region-9',variant:'original'});
  // Native emits a null variant when the plugin omitted one.
  assert.deepEqual(actions.parseExtensionActionRequest({sceneId:'scene-A',id:'r1',action:'reference',regionId:'region-9',variant:null}),{sceneId:'scene-A',id:'r1',action:'reference',regionId:'region-9'});
  for(const payload of [
    null, 'copy', [],
    {sceneId:'scene-A',id:'r1',action:'copy'},
    {sceneId:'scene-A',id:'r1',regionId:'region-9'},
    {sceneId:'scene-A',id:'r1',action:'copy',regionId:'region-9',extra:1},
    {sceneId:'scene-A',id:'',action:'copy',regionId:'region-9'},
    {sceneId:'scene-A',id:'r1',action:'evil',regionId:'region-9'},
    {sceneId:'scene-A',id:'r1',action:'copy',regionId:'region-9',variant:'retouched'},
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
  handler({payload:{sceneId:'scene-A',id:'a1',action:'copy',regionId:'r'}});
  handler({payload:{sceneId:'scene-A',id:'a2',action:'pin',regionId:'r'}});
  handler({payload:{sceneId:'scene-A',id:'a3',action:'copy'}});
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
  handler({payload:{sceneId:'scene-A',id:'b1',action:'save',regionId:'r',variant:'annotated'}});
  await settle();
  assert.deepEqual(errors,[['b1',false,'已取消']]);
  stopThrowing();
});

test('duplicate in-flight events and events after detach never start another effect', async () => {
  let handler, release;
  const pending = new Promise(done => { release = done; });
  let executions = 0;
  const stop = await actions.attachExtensionActions({
    execute: async () => { executions++; await pending; return {ok:true}; }, resolve: async () => {},
  }, (_, listener) => { handler = listener; return Promise.resolve(() => {}); });
  const event = {payload:{id:'once',sceneId:'a',regionId:'r',action:'save'}};
  handler(event); handler(event); handler(event);
  assert.equal(executions, 1);
  stop(); handler({payload:{...event.payload,id:'later'}});
  assert.equal(executions, 1);
  release(); await new Promise(done => setImmediate(done));
});

async function executor(context) {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const ast = parse(source, {sourceType:'module',plugins:['typescript','jsx']});
  let node;
  function visit(value) {
    if (!value || typeof value !== 'object') return;
    if (value.type === 'FunctionDeclaration' && value.id?.name === 'executeHostAction') node = value;
    else for (const child of Object.values(value)) Array.isArray(child) ? child.forEach(visit) : visit(child);
  }
  visit(ast); assert.ok(node);
  const code = stripTypeScriptTypes(source.slice(node.start,node.end),{mode:'transform'});
  return new Function(...Object.keys(context), `${code};return executeHostAction`)(...Object.values(context));
}
function scene(id='a') { return {id,regions:[{id:'r'}],background:{id:'bg'},closed:false,frozen:false}; }
async function appFixture() {
  const state = {scene:scene(),calls:[],references:[],flush:async()=>{},native:async()=>true};
  const execute = await executor({
    scene:()=>state.scene, busy:()=>false, recording:()=>false, scroll:()=>false, exitPreparing:()=>false,
    disposed:false, spaceOperations:{begin:()=>()=>state.calls.push('finish')},setBusy:()=>{},flush:()=>state.flush(),
    executeExtensionAction:async id=>{state.calls.push(id);return state.native();},
    refs:()=>state.references,toggleReference:value=>state.references.push(value),extensionActionReason:actions.extensionActionReason,
  });
  return {...state, state, execute};
}
test('actual App action refuses another scene before or after asynchronous drawing flush', async () => {
  for (const afterFlush of [false,true]) {
    const {state,execute} = await appFixture();
    if (afterFlush) state.flush=async()=>{state.scene=scene('b');}; else state.scene=scene('b');
    assert.equal((await execute({id:'ticket',sceneId:'a',regionId:'r',action:'save'})).ok,false);
    assert.equal(state.calls.includes('ticket'),false);
  }
});
test('native denial after flush never inserts references; successful reference is idempotent', async () => {
  const {state,execute} = await appFixture();
  const request={id:'ticket',sceneId:'a',regionId:'r',action:'reference'};
  state.native=async()=>{throw new Error('已取消');};
  assert.equal((await execute(request)).ok,false); assert.deepEqual(state.references,[]);
  state.native=async()=>true;
  assert.equal((await execute(request)).ok,true);
  assert.equal((await execute({...request,id:'next'})).ok,true);
  assert.deepEqual(state.references,[{kind:'region',id:'r'}]);
});
