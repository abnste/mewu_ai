// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const load=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const bridge=await load('./extension-surface-bridge.ts');
const drag=await load('./extension-object-drag.ts');
const pointer=await load('./extension-surface-pointer.ts');
test('two object receivers coexist; only the top hit consumes and unmounted receivers cannot consume',()=>{
  const calls=[];
  const receiver=name=>({begin:()=>({move:()=>calls.push(name+' move'),finish:()=>{calls.push(name+' finish');return true;},cancel:()=>calls.push(name+' cancel')})});
  const removeA=drag.registerObjectDrop(receiver('a')),removeB=drag.registerObjectDrop(receiver('b'));
  const one=drag.beginObjectDrag({source:{kind:'pin',id:'x'},remove(){}});
  one.move({x:1,y:1});assert.equal(one.finish({x:1,y:1}),true);
  assert.deepEqual(calls,['b move','a move','b finish','a cancel']);
  const two=drag.beginObjectDrag({source:{kind:'pin',id:'x'},remove(){}});removeB();calls.length=0;
  assert.equal(two.finish({x:1,y:1}),true);assert.deepEqual(calls,['b cancel','a finish']);removeA();
  assert.equal(drag.beginObjectDrag({source:{kind:'pin',id:'x'},remove(){}}),undefined);
});
test('surface bridge confines RPC to bound handler, validates targets and discards late replies after close',async()=>{
  const sent=[];let handler,closed=false;
  const port={set onmessage(value){handler=value;},get onmessage(){return handler;},postMessage:v=>sent.push(v),start(){},close(){closed=true;}};
  let resolve;const requests=[],targets=[];
  const scope=bridge.surfaceBridge(port,input=>{requests.push(input);return new Promise(done=>resolve=done);},v=>targets.push(v));
  await handler({data:{type:'drop-targets',targets:[{id:'one',x:0,y:0,width:100,height:50}]}});
  await handler({data:{type:'drop-targets',targets:[{id:'bad',x:0,y:0,width:Infinity,height:50}]}});
  assert.equal(targets.length,1);
  const pending=handler({data:{type:'call',id:1,input:{type:'get'},pluginId:'foreign',sceneId:'foreign',method:'shell'}});
  await handler({data:{type:'call',id:1,input:{type:'duplicate'}}});
  assert.deepEqual(requests,[{type:'get'}]);
  scope.close();resolve({saved:true});await pending;
  assert.equal(sent.length,0);assert.equal(closed,true);assert.deepEqual(targets.at(-1),[]);
});
test('surface bridge returns errors rather than treating plugin failures as success',async()=>{
  let handler;const sent=[];
  const scope=bridge.surfaceBridge({set onmessage(v){handler=v;},postMessage:v=>sent.push(v),start(){},close(){}},async()=>{throw Error('revoked');},()=>{});
  await handler({data:{type:'call',id:5,input:{type:'save'}}});
  assert.deepEqual(sent,[{type:'response',id:5,ok:false,error:'revoked'}]);scope.close();
});
test('cross-document gesture returns local coordinates and releases its shield on close or Escape',async()=>{
  const previous={document:globalThis.document,window:globalThis.window};
  class Target {
    listeners=new Map();
    addEventListener(name,fn){this.listeners.set(name,fn);}
    removeEventListener(name,fn){if(this.listeners.get(name)===fn)this.listeners.delete(name);}
    send(name,value={}){this.listeners.get(name)?.({preventDefault(){},stopPropagation(){},...value});}
  }
  const host=new Target(),children=[];
  globalThis.window=host;
  globalThis.document={body:{append(el){children.push(el);}},createElement(){
    const el=new Target();el.style={};el.setAttribute=()=>{};el.remove=()=>children.splice(children.indexOf(el),1);return el;
  }};
  const frame={getBoundingClientRect:()=>({left:40,top:80})};
  try{
    const first=pointer.surfacePointerDrag(frame);
    assert.throws(()=>pointer.surfacePointerDrag(frame),/已有拖动/);
    children[0].send('pointerup',{type:'pointerup',clientX:320,clientY:100});
    assert.deepEqual(await first.done,{cancelled:false,x:280,y:20});
    assert.equal(children.length,0);assert.equal(host.listeners.size,0);
    const relayed=pointer.surfacePointerDrag(frame);
    relayed.forward({type:'pointerup',x:450,y:-50,buttons:0});
    assert.deepEqual(await relayed.done,{cancelled:false,x:450,y:-50});
    relayed.forward({type:'pointerup',x:999,y:999,buttons:0});
    assert.equal(children.length,0);
    const closed=pointer.surfacePointerDrag(frame);closed.cancel();
    assert.equal((await closed.done).cancelled,true);
    const escaped=pointer.surfacePointerDrag(frame);host.send('keydown',{key:'Escape'});
    assert.equal((await escaped.done).cancelled,true);
    assert.equal(children.length,0);assert.equal(host.listeners.size,0);
  }finally{globalThis.document=previous.document;globalThis.window=previous.window;}
});
test('pointer relay cannot complete a foreign request or outlive its surface',async()=>{
  let handler,finish;const forwarded=[],sent=[];
  const scope=bridge.surfaceBridge({set onmessage(v){handler=v;},postMessage:v=>sent.push(v),start(){},close(){}},
    async()=>{},()=>{},()=>({done:new Promise(resolve=>finish=resolve),forward:v=>forwarded.push(v),cancel:()=>finish({cancelled:true,x:0,y:0})}));
  const pending=handler({data:{type:'pointer-drag',id:7,input:{}}});
  const event={type:'pointerup',x:100,y:20,buttons:0};
  for(const data of [
    {type:'pointer-drag-event',id:8,event},
    {type:'pointer-drag-event',id:7,event:{...event,x:Infinity}},
    {type:'pointer-drag-event',id:7,event:{...event,buttons:-1}},
  ])await handler({data});
  assert.equal(forwarded.length,0);
  await handler({data:{type:'pointer-drag-event',id:7,event}});
  assert.deepEqual(forwarded,[event]);
  scope.close();await pending;
  assert.equal(sent.length,0);
});
