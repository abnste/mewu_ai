// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
const source=await readFile(new URL('../client.js',import.meta.url),'utf8');
function client(){
  const listeners=new Map(),sent=[],parent={},port={start(){},close(){},postMessage(value){sent.push(value);}};
  class PointerEvent {constructor(trusted=true){this.isTrusted=trusted;this.type='pointerdown';this.button=0;this.buttons=1;this.pointerId=1;}preventDefault(){}}
  class Element {
    captures=new Set();
    setPointerCapture(id){this.captures.add(id);}
    hasPointerCapture(id){return this.captures.has(id);}
    releasePointerCapture(id){this.captures.delete(id);}
  }
  const context=vm.createContext({parent,PointerEvent,Element,HTMLSelectElement:class{},setTimeout,clearTimeout,
    addEventListener(name,fn){listeners.set(name,fn);},removeEventListener(name){listeners.delete(name);}});
  vm.runInContext(source,context);
  listeners.get('message')({source:parent,data:{type:'mewu-connect'},ports:[port]});
  return {api:context.mewu,listeners,sent,port,PointerEvent,Element};
}

test('pinned object Escape returns to the space without saving or closing the object; explicit close still saves',async()=>{
  const c=client();await c.api.ready;let saves=0;
  c.api.onBeforeClose(()=>{saves++;});
  c.port.onmessage({data:{type:'event',name:'window-state',value:{pinned:true}}});
  c.listeners.get('keydown')({key:'Escape',preventDefault(){},stopPropagation(){}});
  await new Promise(setImmediate);
  assert.equal(saves,0);assert.equal(c.sent[0].type,'escape-key');
  c.port.onmessage({data:{type:'event',name:'close-request'}});await new Promise(setImmediate);
  assert.equal(saves,1);assert.equal(c.sent[1].type,'escape');
  c.port.onmessage({data:{type:'event',name:'window-state',value:{pinned:false}}});
  c.listeners.get('keydown')({key:'Escape',preventDefault(){},stopPropagation(){}});await new Promise(setImmediate);
  assert.equal(saves,2);assert.equal(c.sent[2].type,'escape');c.api.dispose();
});
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

test('fast native WebKit primary presses with buttons=0 remain single-use and release-bound',async()=>{
  const c=client();await c.api.ready;
  const press=new c.PointerEvent();press.buttons=0;press.target=new c.Element();
  c.listeners.get('pointerdown')(press);
  const pending=c.api.drag(press),id=c.sent[0].id;
  assert.equal(press.target.hasPointerCapture(1),true);
  await assert.rejects(c.api.drag(press),/user pointer/);
  c.listeners.get('pointerup')({isTrusted:true,pointerId:1,type:'pointerup',buttons:0,clientX:12,clientY:18});
  assert.equal(c.sent[1].event.type,'pointerup');
  c.port.onmessage({data:{type:'response',id,ok:true,value:{cancelled:false,x:12,y:18}}});
  assert.deepEqual(await pending,{cancelled:false,x:12,y:18});
  const released=new c.PointerEvent();released.buttons=0;
  c.listeners.get('pointerdown')(released);c.listeners.get('pointerup')({});
  await assert.rejects(c.api.drag(released),/user pointer/);
  const synthetic=new c.PointerEvent(false);synthetic.buttons=0;c.listeners.get('pointerdown')(synthetic);
  await assert.rejects(c.api.drag(synthetic),/user pointer/);
  const wrongButton=new c.PointerEvent();wrongButton.button=2;wrongButton.buttons=0;c.listeners.get('pointerdown')(wrongButton);
  await assert.rejects(c.api.drag(wrongButton),/user pointer/);
  c.api.dispose();
});
test('Escape waits for saved edits; failed persistence keeps the surface open and allows retry',async()=>{
  const c=client();await c.api.ready;
  let resolve,attempt=0;
  c.api.onBeforeClose(()=>{attempt++;return new Promise(done=>resolve=done);});
  const event={key:'Escape',preventDefault(){},stopPropagation(){}};
  c.listeners.get('keydown')(event);c.listeners.get('keydown')(event);
  assert.equal(attempt,1);assert.equal(c.sent.length,0);
  resolve();await new Promise(setImmediate);
  assert.equal(c.sent[0].type,'escape');c.api.dispose();
  const failed=client();await failed.api.ready;
  const stop=failed.api.onBeforeClose(()=>{throw Error('disk unavailable');});
  failed.port.onmessage({data:{type:'event',name:'close-request'}});
  await new Promise(setImmediate);
  assert.equal(failed.sent[0].type,'close-error');assert.equal(failed.sent[0].error,'disk unavailable');
  stop();failed.port.onmessage({data:{type:'event',name:'close-request'}});
  await new Promise(setImmediate);assert.equal(failed.sent[1].type,'escape');failed.api.dispose();
});
test('composition and repeat Escape never close the surface',async()=>{
  const c=client();await c.api.ready;
  for(const extra of [{isComposing:true},{keyCode:229},{repeat:true}]){
    c.listeners.get('keydown')({key:'Escape',...extra,preventDefault(){throw Error('consumed');}});
  }
  await new Promise(setImmediate);assert.equal(c.sent.length,0);c.api.dispose();
});
test('closing a surface rejects pending calls and cannot reopen its port',async()=>{
  const c=client();await c.api.ready;
  const pending=c.api.call({type:'get'});await Promise.resolve();
  c.api.dispose();await assert.rejects(pending,/Surface closed/);
  await assert.rejects(c.api.call({type:'save'}),/unavailable/);
  assert.equal(c.sent.length,1);
});
test('WebKit pointer events relay only the live trusted pointer and stop after completion',async()=>{
  const c=client();await c.api.ready;
  const press=new c.PointerEvent();c.listeners.get('pointerdown')(press);
  const pending=c.api.drag(press);
  const id=c.sent[0].id;
  const sample={isTrusted:true,pointerId:1,type:'pointermove',clientX:120,clientY:-30,buttons:1};
  c.listeners.get('pointermove')({...sample,isTrusted:false});
  c.listeners.get('pointermove')({...sample,pointerId:9});
  assert.equal(c.sent.length,1);
  c.listeners.get('pointermove')(sample);
  assert.equal(c.sent[1].type,'pointer-drag-event');
  assert.equal(c.sent[1].id,id);
  assert.equal(c.sent[1].event.y,-30);
  c.listeners.get('pointerup')({...sample,type:'pointerup',buttons:0});
  assert.equal(c.sent[2].event.type,'pointerup');
  c.listeners.get('pointermove')(sample);
  assert.equal(c.sent.length,3);
  c.port.onmessage({data:{type:'response',id,ok:true,value:{cancelled:false,x:120,y:-30}}});
  assert.deepEqual(await pending,{cancelled:false,x:120,y:-30});
  c.api.dispose();
});
test('originating pointer capture survives iframe crossings and is released on reply, release, or disposal',async()=>{
  for(const end of ['reply','release','dispose']){
    const c=client();await c.api.ready;
    const press=new c.PointerEvent();press.target=new c.Element();
    c.listeners.get('pointerdown')(press);
    const pending=c.api.drag(press),id=c.sent[0].id;
    assert.equal(press.target.hasPointerCapture(1),true);
    const next=new c.PointerEvent();c.listeners.get('pointerdown')(next);
    await assert.rejects(c.api.drag(next),/already active/);
    assert.equal(press.target.hasPointerCapture(1),true);
    if(end==='dispose'){c.api.dispose();await assert.rejects(pending,/Surface closed/);}
    else{
      if(end==='release'){
        c.listeners.get('pointerup')({type:'pointerup',isTrusted:true,pointerId:1,buttons:0,clientX:-30,clientY:900});
        assert.equal(press.target.hasPointerCapture(1),false);
      }
      c.port.onmessage({data:{type:'response',id,ok:true,value:{cancelled:false,x:-30,y:900}}});
      await pending;c.api.dispose();
    }
    assert.equal(press.target.hasPointerCapture(1),false);
  }
});
