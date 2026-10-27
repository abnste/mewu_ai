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
 const context=createContext({console,HTMLElement:Element});
 const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[k,v]of Object.entries(values))this.setExport(k,v);},{context});
 const core=synthetic(solid);
 const universal=new SourceTextModule(await readFile(new URL('../../../node_modules/solid-js/universal/dist/universal.js',import.meta.url),'utf8'),{context});await universal.link(()=>core);await universal.evaluate();
 const renderer=universal.namespace.createRenderer({createElement:()=>new Element(),createTextNode:text=>({text,children:[],parent:null}),replaceText:(n,text)=>n.text=text,setProperty:(n,k,v)=>n[k]=v,
 insertNode:(p,n,a)=>{if(n===a)return;if(n.parent)n.parent.children.splice(n.parent.children.indexOf(n),1);const i=a?p.children.indexOf(a):-1;p.children.splice(i<0?p.children.length:i,0,n);n.parent=p;},removeNode:(p,n)=>{p.children.splice(p.children.indexOf(n),1);n.parent=null;},isTextNode:n=>'text'in n,getParentNode:n=>n.parent,getFirstChild:n=>n.children[0],getNextSibling:n=>n.parent?.children[n.parent.children.indexOf(n)+1]});
 const load=async path=>new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'}),{context});
 const state=await load('./extension-state.ts');await state.link(()=>{throw Error('unexpected')});await state.evaluate();
 const layout=await load('./toolbar-layout.ts');await layout.link(()=>state);await layout.evaluate();
 const api=synthetic({extensionNative:true,subscribeExtensions:async f=>{receive=f;return()=>stops++;},getExtensions:async()=>[record],invokeExtension:input=>new Promise(resolve=>calls.push({input,resolve}))});
 const source=await readFile(new URL('./components/ToolbarLayout.tsx',import.meta.url),'utf8');
 const compiled=stripTypeScriptTypes(transformSync(source,{filename:'ToolbarLayout.tsx',babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code,{mode:'transform'});
 const component=new SourceTextModule(compiled,{context});await component.link(n=>n==='solid-js'?core:n==='test-renderer'?synthetic(renderer):n.endsWith('/event')?synthetic({listen:async(_,f)=>{config=f;return()=>stops++;}}):n.endsWith('/extension-bridge')?api:n.endsWith('/extension-state')?state:layout);await component.evaluate();
 const initial=[new Element('draw'),new Element('copy'),new Element('export')];const [nodes,setNodes]=solid.createSignal(initial);const root=new Element();
 const dispose=renderer.render(()=>component.namespace.default({surface:'capture.selection',get children(){return nodes();}}),root);await tick();
 return{initial,root,calls,dispose,setNodes,receive:v=>receive(v),config:()=>config(),stops:()=>stops,element:id=>new Element(id)};
}
test('layout moves original nodes and handler owners, refresh does not flash default order, disable rejects late replies',async()=>{
 const f=await fixture();try{
  assert.equal(f.calls.length,1);f.calls[0].resolve({order:['copy','export','draw']});await tick();
  assert.deepEqual(f.root.children,[f.initial[1],f.initial[2],f.initial[0]]);
  f.config();assert.deepEqual(f.root.children,[f.initial[1],f.initial[2],f.initial[0]]);
  assert.equal(f.calls.length,2);f.receive([{...record,revision:2,state:'disabled'}]);
  assert.deepEqual(f.root.children,f.initial);f.calls[1].resolve({order:['export','copy']});await tick();assert.deepEqual(f.root.children,f.initial);
 }finally{f.dispose();}assert.equal(f.stops(),2);
});
test('popover appearance cannot restart a layout request or replace the existing control nodes',async()=>{
 const f=await fixture();try{
  f.calls[0].resolve({order:['copy']});await tick();const popup=f.element(undefined);f.setNodes([...f.initial,popup]);await tick();
  assert.equal(f.calls.length,1);assert.deepEqual(f.root.children,[f.initial[1],f.initial[0],f.initial[2],popup]);
  f.dispose();assert.equal(f.stops(),2);
 }finally{f.dispose();}
});
