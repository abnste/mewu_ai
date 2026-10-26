// SPDX-License-Identifier: MPL-2.0
// Production controller + real Solid effects; synthetic native IPC only.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
import {parse} from '@babel/parser';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const source=await readFile(new URL('./components/HookShortcutSetting.tsx',import.meta.url),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
const body=ast.program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration.body;
const returned=body.body.find(n=>n.type==='ReturnStatement');
const code=stripTypeScriptTypes(source.slice(0,returned.start)+'return {view,draft,error,ids,arm,disarm,record,recording,save,cancel,reload,keyboard};}',{mode:'transform'});
const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const [k,v]of Object.entries(values))this.setExport(k,v);});
const tick=()=>new Promise(r=>setImmediate(r));
const copy=value=>JSON.parse(JSON.stringify(value));
async function fixture({empty=false,begin}={}){
  const id='92d23e06-8ccd-4e07-9fbc-092f29679887';
  let state={revision:4,enabled:true,platform:'windows',shortcuts:{},rules:empty?[]:[{id,revision:2,name:'我的截图',enabled:true,trigger:{kind:'shortcut',accelerator:'shift+alt+KeyS'},condition:'return true;',actions:[{kind:'app',method:'capture.open',params:{}},{kind:'delay',milliseconds:100}],cooldownMs:950,timeoutMs:2000,stopOnError:false}]};
  const writes=[],leases=[],ends=[],activations=[];let recorded,notify,dispose,active,enabled,api;
  const bridge={getHooks:async()=>copy(state),subscribeHooks:async(cb,rec)=>{notify=cb;recorded=rec;return()=>{};},editHookShortcut:async(id,on)=>activations.push([id,on]),
    saveHook:async(rev,rule)=>{assert.equal(rev,state.revision,'CAS conflict');writes.push(copy(rule));state={...state,revision:rev+1,rules:[{...copy(rule),revision:rule.revision+1}]};notify();return copy(state);},
    removeHook:async(rev,key,ruleRevision)=>{assert.equal(rev,state.revision);assert.equal(ruleRevision,state.rules.find(r=>r.id===key).revision);writes.push({removed:key});state={...state,revision:rev+1,rules:[]};notify();return copy(state);}};
  const shortcut={beginCaptureShortcutEdit:async id=>{leases.push(id);if(begin)await begin();},endCaptureShortcutEdit:async id=>ends.push(id),subscribeCaptureShortcut:async()=>()=>{}};
  async function load(file){const m=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(file,import.meta.url),'utf8'),{mode:'transform'}));await m.link(()=>{throw Error('unexpected dependency');});await m.evaluate();return m;}
  const contracts=await load('./hooks-contracts.ts'),editor=await load('./capture-shortcut-editor.ts');
  const m=new SourceTextModule(code);await m.link(name=>{
    if(name==='solid-js')return synthetic(solid);
    if(name==='lucide-solid')return synthetic({Check(){},LoaderCircle(){},X(){}});
    if(name.endsWith('/hooks-bridge'))return synthetic(bridge);
    if(name.endsWith('/shortcut-bridge'))return synthetic(shortcut);
    if(name.endsWith('/hooks-contracts'))return contracts;
    if(name.endsWith('/capture-shortcut-editor'))return editor;
    if(name.endsWith('/i18n'))return synthetic({t:s=>s});
    if(name.endsWith('.css'))return synthetic({});
    throw Error(name);
  });await m.evaluate();
  const oldWindow=globalThis.window,oldDocument=globalThis.document;
  globalThis.window={addEventListener(){},removeEventListener(){}};
  globalThis.document={activeElement:{matches:s=>s==='[data-operation-shortcut]'}};
  solid.createRoot(d=>{dispose=d;const [a,sa]=solid.createSignal(true),[e,se]=solid.createSignal(true);active=sa;enabled=se;api=m.namespace.default({get active(){return a();},get enabled(){return e();},method:'capture.open',label:'截图快捷键'});});
  await tick();await tick();
  return{api,id,writes,leases,ends,activations,active,enabled,record:(lease,key)=>recorded(lease,key),external:patch=>{state={...state,...patch};notify();},state:()=>copy(state),async stop(){dispose();await tick();globalThis.window=oldWindow;globalThis.document=oldDocument;}};
}
test('inline save updates the same ordinary rule and preserves actions, metadata and identity',async()=>{
  const f=await fixture();try{const old=f.state().rules[0];await f.api.arm(f.id);f.api.record('Control+KeyP');await f.api.save();assert.equal(f.api.error(),'');assert.equal(f.writes.length,1);assert.deepEqual(f.writes[0],{...old,trigger:{kind:'shortcut',accelerator:'Control+KeyP'}});assert.equal(f.api.draft(),undefined);assert.ok(f.ends.includes(f.leases[0]));}finally{await f.stop();}
});
test('Delete then save removes the exact rule; empty view does not resurrect defaults',async()=>{
  const f=await fixture();try{await f.api.arm(f.id);f.api.keyboard({key:'Delete',preventDefault(){},stopPropagation(){}});assert.equal(f.api.draft().value,'');await f.api.save();assert.deepEqual(f.writes,[{removed:f.id}]);assert.deepEqual(copy(f.api.ids()),['']);await f.api.reload();assert.equal(f.writes.length,1);}finally{await f.stop();}
});
test('an empty shortcut only creates a rule after explicit recording and save',async()=>{
  const f=await fixture({empty:true});try{assert.equal(f.writes.length,0);await f.api.arm('');f.api.record('F9');await f.api.save();assert.equal(f.writes.length,1);assert.deepEqual(f.writes[0].actions,[{kind:'app',method:'capture.open',params:{}}]);assert.equal(f.writes[0].trigger.accelerator,'F9');}finally{await f.stop();}
});
test('stale revisions cannot overwrite a rule edited in Hooks',async()=>{
  const f=await fixture();try{await f.api.arm(f.id);f.api.record('F9');f.external({revision:9});await tick();await f.api.save();assert.match(f.api.error(),/CAS conflict/);assert.equal(f.writes.length,0);assert.equal(f.api.draft().value,'F9');f.api.cancel();await f.api.arm(f.id);f.api.record('F10');await f.api.save();assert.equal(f.writes.length,1);}finally{await f.stop();}
});
test('hidden, disabled and expired recording leases cannot change drafts or register late',async()=>{
  let release;const pending=new Promise(r=>release=r);const f=await fixture({begin:()=>pending});
  try{const flight=f.api.arm(f.id);f.active(false);release();await flight;assert.equal(f.api.recording(),'');assert.equal(f.api.draft(),undefined);assert.equal(f.activations.filter(([,on])=>on).length,0);f.record(f.leases[0],'F11');assert.equal(f.api.draft(),undefined);assert.equal(f.writes.length,0);}finally{await f.stop();}
});
test('cancel and blur reject stale native keys without changing persisted configuration',async()=>{
  const f=await fixture();try{await f.api.arm(f.id);const old=f.leases[0];f.api.record('F9');f.api.cancel();await f.api.arm(f.id);f.record(old,'F10');assert.equal(f.api.draft().value,'shift+alt+KeyS');f.enabled(false);await tick();assert.equal(f.api.draft(),undefined);assert.equal(f.writes.length,0);}finally{await f.stop();}
});
