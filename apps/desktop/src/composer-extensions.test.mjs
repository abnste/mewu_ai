// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const synthetic = values => new SyntheticModule(Object.keys(values), function(){for(const [k,v] of Object.entries(values))this.setExport(k,v);});
const load = async name => new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(name,import.meta.url),'utf8'),{mode:'transform'}));
const tick=()=>new Promise(r=>setImmediate(r));
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
const record={manifest:{id:'community.engine'},revision:1,state:'enabled',services:[{kind:'agent',id:'engine',title:'Independent',metadata:{modelsService:'models'}},{kind:'service',id:'models'}]};
const bound={agentId:'a',kind:'agent',pluginId:'community.engine',serviceId:'engine',revision:1};
const catalog={ok:true,models:[{id:'fast',name:'Fast'}],selectedModelId:'fast',selection:{modelId:'fast'}};
async function fixture(){
 let events, stopped=false,calls=[],invoke=async()=>catalog;const api={extensionNative:true,subscribeExtensionBindings:async f=>{events=f;return()=>{stopped=true;}},getExtensionBindings:async()=>[bound],setExtensionBinding:async(...args)=>{calls.push(args);return [{...bound,revision:2,pluginId:args[3]?.pluginId??null,serviceId:args[3]?.serviceId??null}];},invokeExtension:async v=>{calls.push(v);return invoke(v);}};
 const state=await load('./extension-state.ts');await state.link(()=>{throw Error('unexpected import')});await state.evaluate();
 const module=await load('./composer-extensions.ts');await module.link(n=>n==='solid-js'?synthetic(solid):n==='./extension-bridge'?synthetic(api):state);await module.evaluate();
 let value,dispose,setScene,setRecords,setBusy;
 solid.createRoot(d=>{dispose=d;const [scene,s]=solid.createSignal('one'),[records,r]=solid.createSignal([record]),[busy,b]=solid.createSignal(false);setScene=s;setRecords=r;setBusy=b;value=module.namespace.createComposerExtensions({sceneId:scene,agentId:()=> 'a',records,busy});});
 await tick();return {value,dispose,setScene,setRecords,setBusy,calls,api,events: v=>events(v),module,setInvoke:v=>{invoke=v;},stopped:()=>stopped};
}
test('independent engines and models use generic registrations and committed CAS selections',async()=>{
 const f=await fixture();try{
  assert.equal(f.value.engine().service.title,'Independent');assert.equal(await f.value.refreshModels(),true);
  assert.equal(f.calls[0].serviceId,'models');assert.equal(f.calls[0].sceneId,'one');
  await f.value.selectModel('fast');assert.deepEqual(JSON.parse(JSON.stringify(f.calls[1].input)),{type:'select',modelId:'fast',expected:{modelId:'fast'}});
  assert.equal(await f.value.selectEngine(''),true);assert.equal(f.value.selected(),'');assert.equal(f.value.catalog(),undefined);
 }finally{f.dispose();}assert.equal(f.stopped(),true);
});
test('late catalog replies cannot cross conversation, disable, upgrade or disposal boundaries',async()=>{
 for(const change of ['scene','disable','upgrade','dispose']){
  const f=await fixture(),wait=deferred();f.setInvoke(()=>wait.promise);
  const request=f.value.refreshModels();assert.equal(f.value.pending(),true);
  if(change==='scene')f.setScene('two');
  if(change==='disable')f.setRecords([{...record,revision:2,state:'disabled'}]);
  if(change==='upgrade')f.setRecords([{...record,revision:2}]);
  if(change==='dispose')f.dispose();
  wait.resolve(catalog);assert.equal(await request,false);assert.equal(f.value.catalog(),undefined);
  f.dispose();
 }
});
test('disabled providers revoke model entry and stale binding events cannot restore an older engine',async()=>{
 const f=await fixture();try{
  f.events([{...bound,revision:3,pluginId:null,serviceId:null}]);f.events([bound]);assert.equal(f.value.selected(),'');
  f.events([{...bound,revision:4}]);f.setRecords([{...record,revision:2,state:'disabled'}]);assert.equal(f.value.engine(),undefined);assert.equal(f.value.modelsService(),undefined);assert.equal(await f.value.refreshModels(),false);
 }finally{f.dispose();}
});
test('busy UI cannot mutate engine or model; malformed and failed catalogs are rejected',async()=>{
 const f=await fixture();try{
  f.setBusy(true);assert.equal(await f.value.selectEngine(''),false);assert.equal(await f.value.selectModel('fast'),false);assert.equal(f.calls.length,0);
  for(const value of [null,{}, {ok:false,error:'需要登录'}, {...catalog,models:[{id:'x',name:'X'},{id:'x',name:'Y'}]}])assert.throws(()=>f.module.namespace.parseAgentModelCatalog(value));
 }finally{f.dispose();}
});
