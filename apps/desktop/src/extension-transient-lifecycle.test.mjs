// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {parse} from '@babel/parser';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule,createContext} from 'node:vm';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const tick=()=>new Promise(setImmediate);

const record = (overrides={}) => ({manifest:{id:'demo',permissions:['ui.overlay','input.trigger']},revision:1,state:'enabled',services:[{kind:'command',id:'view'},{kind:'trigger',id:'hold',metadata:{trigger:{code:'Alt'}}}],...overrides});
const deferred = () => { let resolve,reject; const promise=new Promise((yes,no)=>{resolve=yes;reject=no});return {promise,resolve,reject}; };
async function fixture(disabled=false,options={}){
  const source=await readFile(new URL('./components/ExtensionTransientOverlay.tsx',import.meta.url),'utf8');
  const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
  const fn=ast.program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration;
  const ret=fn.body.body.find(n=>n.type==='ReturnStatement');
  const imports=ast.program.body.filter(n=>n.type==='ImportDeclaration').map(n=>source.slice(n.start,n.end)).join('\n');
  const code=imports+'\n'+source.slice(fn.start,ret.start)+'return {accept,destroy,mounted,loaded,setFrame(value){frame=value},attached:()=>attached};}\nexport default ExtensionTransientOverlay;';
  const win=new Map(),doc=new Map(),events=new Map(),calls=[],messages=[],raf=new Map(),ports=[],errors=[];let seq=0;
  const target=map=>({addEventListener(name,fn){const list=map.get(name)||[];list.push(fn);map.set(name,list)},removeEventListener(name,fn){map.set(name,(map.get(name)||[]).filter(x=>x!==fn))}});
  const window=target(win),document=target(doc);
  const context=createContext({console,window,document,innerWidth:1000,innerHeight:800,
    addEventListener:window.addEventListener,removeEventListener:window.removeEventListener,
    requestAnimationFrame(fn){raf.set(++seq,fn);return seq},cancelAnimationFrame(id){raf.delete(id)},
    MessageChannel:class{constructor(){this.port1={onmessage:null,postMessage:v=>messages.push(v),closed:false,close(){this.closed=true}};this.port2={};ports.push(this.port1);}}});
  const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[k,v]of Object.entries(values))this.setExport(k,v)},{context});
  const listen=async(name,fn)=>{events.set(name,fn);if(options.listen?.[name])await options.listen[name];return()=>events.delete(name)};
  const api={extensionNative:true,getExtensions:async()=>options.initial ? await options.initial : [record()],invokeExtension:async value=>{calls.push(value)},subscribeExtensions:accept=>listen('extension-state',e=>accept(e.payload)),subscribeSurfaceClosed:accept=>listen('extension-surface-closed',e=>accept(e.payload))};
  const helper=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./extension-transient-overlay.ts',import.meta.url),'utf8'),{mode:'transform'}),{context});await helper.link(()=>{});await helper.evaluate();
  const state=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./extension-state.ts',import.meta.url),'utf8'),{mode:'transform'}),{context});await state.link(()=>{});await state.evaluate();
  const module=new SourceTextModule(stripTypeScriptTypes(code,{mode:'transform'}),{context});
  await module.link(name=>name==='solid-js'?synthetic(solid):name==='solid-js/web'?synthetic({Portal(){}}):name==='@tauri-apps/api/event'?synthetic({listen}):name.endsWith('extension-bridge')?synthetic(api):name.endsWith('extension-transient-overlay')?helper:name.endsWith('extension-state')?state:synthetic({}));await module.evaluate();
  let view,dispose;const [off,setOff]=solid.createSignal(disabled);const [scene,setScene]=solid.createSignal('scene');
  solid.createRoot(cleanup=>{dispose=cleanup;view=module.namespace.default({get sceneId(){return scene()},get disabled(){return off()},onError(error){errors.push(error)}})});await tick();
  const emit=(name,payload)=>events.get(name)?.({payload});
  const show=(overrides={})=>{emit('extension-transient-surface',{surfaceId:'surface',pluginId:'demo',revision:1,serviceId:'view',sceneId:'scene',url:'http://localhost:1/surface',width:200,height:100,...overrides});view.setFrame({style:{},contentWindow:{postMessage(){}}});view.loaded()};
  return {view,dispose,setOff,setScene,show,emit,calls,messages,ports,errors,events,raf,win,doc,context,synthetic,async triggers(){
    const trigger=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./extension-triggers.ts',import.meta.url),'utf8'),{mode:'transform'}),{context});await trigger.link(()=>state);await trigger.evaluate();
    return trigger.namespace.attachExtensionTriggers({sceneId:()=> 'scene',records:()=>[record()],invoke:async value=>{calls.push(value)}});
  },pointer(flush=true){for(const fn of win.get('pointermove')||[])fn({clientX:20,clientY:30,buttons:0});if(flush){const pending=[...raf.values()];raf.clear();for(const fn of pending)fn();}},key(key,altKey=false){
    const e={key,altKey,ctrlKey:false,shiftKey:false,metaKey:false,repeat:false,isComposing:false,keyCode:0,stopped:false,immediate:false,preventDefault(){this.prevented=true},stopPropagation(){this.stopped=true},stopImmediatePropagation(){this.stopped=true;this.immediate=true}};
    for(const fn of [...win.get('keydown')||[]]){fn(e);if(e.immediate)break;}if(!e.stopped)for(const fn of [...doc.get('keydown')||[]]){fn(e);if(e.immediate)break;}return e;
  }};
}

test('normal mount receives pointer samples and pointerup releases the surface',async()=>{const f=await fixture();try{f.show();f.pointer();assert.equal(f.messages.length,1);for(const fn of f.win.get('pointerup')||[])fn();assert.equal(f.view.mounted(),undefined);assert.equal(f.calls.at(-1).surface.action,'close');}finally{f.dispose()}});
test('disabling the owning plugin must remove its live frame and pointer subscription',async()=>{const f=await fixture();try{f.show();f.emit('extension-state',[{manifest:{id:'demo'},revision:2,state:'disabled',services:[]}]);f.pointer();assert.deepEqual({mounted:Boolean(f.view.mounted()),pointerMessages:f.messages.length},{mounted:false,pointerMessages:0},'revoked overlay remains mounted and still receives pointer samples');}finally{f.dispose()}});
test('one Escape must destroy an overlay opened while a modifier trigger is held',async()=>{const f=await fixture();let stop;try{stop=await f.triggers();f.key('Alt',true);f.show();f.key('Escape',true);assert.equal(f.view.mounted(),undefined,'window trigger consumes Escape before document overlay');assert.equal(f.calls.filter(c=>c.input?.phase==='cancel').length,1);}finally{stop?.();f.dispose()}});
test('a late overlay while disabled must not leave a hidden Escape consumer',async()=>{const f=await fixture(true);try{f.show();assert.equal(f.view.mounted(),undefined);assert.equal(f.view.attached(),false,'accept reattaches after disabled effect destroyed the frame');}finally{f.dispose()}});

for (const change of [record({revision:2}),record({state:'removed',revision:2}),record({platformError:'unsupported'}),record({unsupportedServices:{view:'unsupported'}}),record({services:[]})]) {
  test(`revocation closes the port and queued frame: ${JSON.stringify(change)}`,async()=>{
    const f=await fixture();try {
      f.show();f.pointer(false);assert.equal(f.raf.size,1);
      f.emit('extension-state',[change]);f.pointer();
      assert.equal(f.view.mounted(),undefined);assert.equal(f.view.attached(),false);
      assert.equal(f.ports[0].closed,true);assert.equal(f.ports[0].onmessage,null);
      assert.equal(f.raf.size,0);assert.equal(f.messages.length,0);
      assert.equal(f.calls.filter(c=>c.surface?.action==='close').length,1);
      f.show();assert.equal(f.view.mounted(),undefined);
    } finally { f.dispose(); }
  });
}

test('native close destroys only the matching surface',async()=>{
  const f=await fixture();try {
    f.show();f.emit('extension-surface-closed','another');assert.ok(f.view.mounted());
    f.emit('extension-surface-closed','surface');assert.equal(f.view.mounted(),undefined);
    assert.equal(f.ports[0].closed,true);assert.equal(f.key('Escape').stopped,false);
  } finally { f.dispose(); }
});

test('Escape also cancels a hold when the overlay listener was registered first',async()=>{
  const f=await fixture();let stop;try {
    f.show();stop=await f.triggers();f.key('Alt',true);
    let screenshotEscapes=0;f.context.document.addEventListener('keydown',()=>screenshotEscapes++);
    const event=f.key('Escape',true);
    assert.equal(f.view.mounted(),undefined);assert.equal(event.prevented,true);
    assert.equal(f.calls.filter(c=>c.input?.phase==='cancel').length,1);
    assert.equal(screenshotEscapes,0);f.key('Escape');assert.equal(screenshotEscapes,1);
  } finally { stop?.();f.dispose(); }
});

for (const change of ['scene','disabled']) test(`${change} changes release the overlay and reject late events`,async()=>{
  const f=await fixture();try {
    f.show();if(change==='scene')f.setScene('other');else f.setOff(true);
    assert.equal(f.ports[0].closed,true);f.show();assert.equal(f.view.mounted(),undefined);
    assert.equal(f.view.attached(),false);assert.equal(f.key('Escape').stopped,false);
  } finally { f.dispose(); }
});

test('a delayed initial catalog cannot resurrect a revoked generation',async()=>{
  const initial=deferred();const f=await fixture(false,{initial:initial.promise});try {
    f.show();assert.equal(f.view.mounted(),undefined);
    f.emit('extension-state',[record({revision:2,state:'disabled'})]);
    initial.resolve([record()]);await tick();f.show();assert.equal(f.view.mounted(),undefined);
    f.emit('extension-state',[record({revision:3})]);f.show({revision:3});assert.ok(f.view.mounted());
  } finally { f.dispose(); }
});

test('initial catalog failure releases subscriptions and leaves normal Escape available',async()=>{
  const initial=deferred();const f=await fixture(false,{initial:initial.promise});try {
    initial.reject(new Error('catalog unavailable'));await tick();
    assert.equal(f.events.size,0);assert.equal(f.errors.length,1);
    assert.equal(f.view.attached(),false);assert.equal(f.key('Escape').stopped,false);
  } finally { f.dispose(); }
});

test('partial subscription failure also releases a late successful subscription',async()=>{
  const failure=deferred(),late=deferred();
  const f=await fixture(false,{listen:{'extension-state':failure.promise,'extension-surface-closed':late.promise}});
  try {
    failure.reject(new Error('subscription unavailable'));await tick();
    late.resolve();await tick();assert.equal(f.errors.length,1);
    assert.equal(f.events.has('extension-surface-closed'),false);
    assert.equal(f.events.has('extension-transient-surface'),false);
    assert.equal(f.view.attached(),false);
  } finally { f.dispose(); }
});

test('unmount closes ports, cancels frames and unsubscribes even with a late subscription',async()=>{
  const f=await fixture();f.show();f.pointer(false);f.dispose();
  assert.equal(f.ports[0].closed,true);assert.equal(f.raf.size,0);assert.equal(f.events.size,0);
  assert.ok([...f.win.values()].every(list=>list.length===0));
  const pending=deferred();const late=await fixture(false,{listen:{'extension-state':pending.promise}});
  late.dispose();pending.resolve();await tick();assert.equal(late.events.size,0);
});
