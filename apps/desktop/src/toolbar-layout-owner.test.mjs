// SPDX-License-Identifier: MPL-2.0
// Real Solid component/effects and keyed node movement, synthetic DOM/native IPC.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {createContext,SourceTextModule,SyntheticModule} from 'node:vm';
import {transformSync} from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
const record={manifest:{id:'independent.layout',permissions:['ui.render']},revision:1,state:'enabled',services:[{kind:'service',id:'order',metadata:{surface:'toolbar-layout',targets:['capture.selection']}}]};
async function fixture(){
 class Element {constructor(id){this.dataset={layoutId:id};this.children=[];this.parent=null;}getAttribute(){return null;}}
 let receive,config,stops=0;const calls=[];
 const context=createContext({console,HTMLElement:Element,setTimeout,clearTimeout});
 const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[k,v]of Object.entries(values))this.setExport(k,v);},{context});
 const core=synthetic(solid);
 const universal=new SourceTextModule(await readFile(new URL('../../../node_modules/solid-js/universal/dist/universal.js',import.meta.url),'utf8'),{context});await universal.link(()=>core);await universal.evaluate();
 const renderer=universal.namespace.createRenderer({createElement:()=>new Element(),createTextNode:text=>({text,children:[],parent:null}),replaceText:(n,text)=>n.text=text,setProperty:(n,k,v)=>n[k]=v,
 insertNode:(p,n,a)=>{if(n===a)return;if(n.parent)n.parent.children.splice(n.parent.children.indexOf(n),1);const i=a?p.children.indexOf(a):-1;p.children.splice(i<0?p.children.length:i,0,n);n.parent=p;},removeNode:(p,n)=>{p.children.splice(p.children.indexOf(n),1);n.parent=null;},isTextNode:n=>'text'in n,getParentNode:n=>n.parent,getFirstChild:n=>n.children[0],getNextSibling:n=>n.parent?.children[n.parent.children.indexOf(n)+1]});
 const load=async path=>new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'}),{context});
 const state=await load('./extension-state.ts');await state.link(()=>{throw Error('unexpected')});await state.evaluate();
 const layout=await load('./toolbar-layout.ts');await layout.link(()=>state);await layout.evaluate();
 const published=[];
 const api=synthetic({extensionNative:true,subscribeExtensions:async f=>{receive=f;return()=>stops++;},getExtensions:async()=>[record],invokeExtension:input=>new Promise(resolve=>calls.push({input,resolve})),publishToolbarCatalog:async(surface,locale,items)=>published.push({surface,locale,items})});
 const events=synthetic({listen:async(_,f)=>{config=f;return()=>stops++;}});
 const session=await load('./toolbar-layout-session.ts');await session.link(n=>n.endsWith('/extension-bridge')?api:n.endsWith('/event')?events:n.endsWith('/extension-state')?state:layout);await session.evaluate();
 const catalog=await load('./toolbar-catalog.ts');await catalog.link(n=>n.endsWith('.json')?synthetic({default:JSON.parse(awaitedCatalog)}):synthetic({t:s=>s}));await catalog.evaluate();
 const source=await readFile(new URL('./components/ToolbarLayout.tsx',import.meta.url),'utf8');
 const compiled=stripTypeScriptTypes(transformSync(source,{filename:'ToolbarLayout.tsx',babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code,{mode:'transform'});
 const component=new SourceTextModule(compiled,{context});await component.link(n=>n==='solid-js'?core:n.endsWith('/i18n')?synthetic({resolveLanguage:()=> 'zh-CN',uiLanguage:()=> 'zh-CN'}):n==='test-renderer'?synthetic(renderer):n.endsWith('/extension-bridge')?api:n.endsWith('/toolbar-layout-session')?session:n.endsWith('/toolbar-catalog')?catalog:layout);await component.evaluate();
 const initial=[new Element('draw'),new Element('copy'),new Element('export')];const [nodes,setNodes]=solid.createSignal(initial);const root=new Element();
 const dispose=renderer.render(()=>component.namespace.default({surface:'capture.selection',get children(){return nodes();}}),root);await tick();
 return{initial,root,calls,dispose,setNodes,published,hold:()=>session.namespace.startToolbarLayouts(),remount:()=>{const root=new Element();const dispose=renderer.render(()=>component.namespace.default({surface:'capture.selection',get children(){return nodes();}}),root);return{root,dispose};},receive:v=>receive(v),config:()=>config(),stops:()=>stops,element:id=>new Element(id)};
}
const awaitedCatalog=await readFile(new URL('../ui/toolbar-catalog.json',import.meta.url),'utf8');
test('layout moves original nodes and handler owners, refresh does not flash default order, disable rejects late replies',async()=>{
 const f=await fixture();try{
  assert.deepEqual(f.root.children,[], 'cold first frame must not render the default arrangement');
  assert.equal(f.calls.length,1);f.calls[0].resolve({order:['copy','export','draw']});await tick();
  assert.deepEqual(f.root.children,[f.initial[1],f.initial[2],f.initial[0]]);
  f.config();assert.deepEqual(f.root.children,[f.initial[1],f.initial[2],f.initial[0]]);
  assert.equal(f.calls.length,2);f.receive([{...record,revision:2,state:'disabled'}]);
  await tick();assert.deepEqual(f.root.children,f.initial);f.calls[1].resolve({order:['export','copy']});await tick();assert.deepEqual(f.root.children,f.initial);
 }finally{f.dispose();}assert.equal(f.stops(),2);
});

test('remount uses the saved layout in its first frame; configuration invalidation cannot replay stale cache',async()=>{
 const f=await fixture(),release=f.hold();let next;
 try{
  f.calls[0].resolve({order:['export','copy','draw']});await tick();f.dispose();
  next=f.remount();assert.deepEqual(next.root.children,[f.initial[2],f.initial[1],f.initial[0]]);assert.equal(f.calls.length,1);
  next.dispose();f.config();next=f.remount();assert.deepEqual(next.root.children,[]);assert.equal(f.calls.length,2);
  f.calls[1].resolve({order:['copy','draw','export']});await tick();assert.deepEqual(next.root.children,[f.initial[1],f.initial[0],f.initial[2]]);
 }finally{next?.dispose();f.dispose();release();}
});

test('host catalog supplies actual captions and explicit separator kind before any screenshot history',async()=>{
 const catalog=JSON.parse(awaitedCatalog).surfaces.find(s=>s.id==='capture.selection');
 assert.deepEqual(catalog.items.filter(i=>i.kind==='separator').map(i=>i.id),['separator-edit','separator-capture','separator-export']);
 assert.equal(catalog.items.find(i=>i.id==='ocr').title,'文字');assert.equal(catalog.items.find(i=>i.id==='export').title,'保存');
 const f=await fixture();try{
  const separator=f.element('separator-edit');f.setNodes([f.initial[0],separator,...f.initial.slice(1)]);await tick();
  assert.equal(f.published.at(-1).items.find(i=>i.id==='separator-edit').kind,'separator');
  f.calls.at(-1).resolve({order:['copy','separator-edit','draw','export']});await tick();
  assert.deepEqual(f.root.children,[f.initial[1],separator,f.initial[0],f.initial[2]]);
  f.calls[0].resolve({order:['draw']});await tick();assert.deepEqual(f.root.children,[f.initial[1],separator,f.initial[0],f.initial[2]]);
 }finally{f.dispose();}
});
test('popover appearance cannot restart a layout request or replace the existing control nodes',async()=>{
 const f=await fixture();try{
  f.calls[0].resolve({order:['copy']});await tick();const popup=f.element(undefined);f.setNodes([...f.initial,popup]);await tick();
  assert.equal(f.calls.length,1);assert.deepEqual(f.root.children,[f.initial[1],f.initial[0],f.initial[2],popup]);
  f.dispose();assert.equal(f.stops(),2);
 }finally{f.dispose();}
});
