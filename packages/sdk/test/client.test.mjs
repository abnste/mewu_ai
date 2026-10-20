// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
const source=await readFile(new URL('../client.js',import.meta.url),'utf8');
function client(){
  const listeners=new Map(),sent=[],parent={},port={start(){},close(){},postMessage(value){sent.push(value);}};
  class PointerEvent {constructor(trusted=true){this.isTrusted=trusted;this.type='pointerdown';this.button=0;this.buttons=1;this.pointerId=1;}preventDefault(){}}
  const context=vm.createContext({parent,PointerEvent,Element:class{},setTimeout,clearTimeout,
    addEventListener(name,fn){listeners.set(name,fn);},removeEventListener(name){listeners.delete(name);}});
  vm.runInContext(source,context);
  listeners.get('message')({source:parent,data:{type:'mewu-connect'},ports:[port]});
  return {api:context.mewu,listeners,sent,port,PointerEvent};
}
test('client drag requires an unconsumed current trusted pointer press',async()=>{
  const c=client();await c.api.ready;
  const fake=new c.PointerEvent(false);c.listeners.get('pointerdown')(fake);
  await assert.rejects(c.api.drag(fake),/user pointer/);assert.equal(c.sent.length,0);
  const real=new c.PointerEvent();c.listeners.get('pointerdown')(real);
  const pending=c.api.drag(real);assert.equal(c.sent[0].type,'pointer-drag');
  await assert.rejects(c.api.drag(real),/user pointer/);
  c.port.onmessage({data:{type:'response',id:c.sent[0].id,ok:true,value:{cancelled:false,x:100,y:20}}});
  assert.deepEqual(await pending,{cancelled:false,x:100,y:20});
  const released=new c.PointerEvent();c.listeners.get('pointerdown')(released);c.listeners.get('pointerup')({});
  await assert.rejects(c.api.drag(released),/user pointer/);c.api.dispose();
});
test('closing a surface rejects pending calls and cannot reopen its port',async()=>{
  const c=client();await c.api.ready;
  const pending=c.api.call({type:'get'});await Promise.resolve();
  c.api.dispose();await assert.rejects(pending,/Surface closed/);
  await assert.rejects(c.api.call({type:'save'}),/unavailable/);
  assert.equal(c.sent.length,1);
});
