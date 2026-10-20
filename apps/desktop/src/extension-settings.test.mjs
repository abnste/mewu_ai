// SPDX-License-Identifier: MPL-2.0
// Production settings surface, real Solid ownership; native IPC and browser
// ports are replaced so delayed replies and unmount order are deterministic.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {createContext,SourceTextModule,SyntheticModule} from 'node:vm';
import {transformSync} from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const tick=()=>new Promise(resolve=>setImmediate(resolve));
async function fixture(){
  const calls=[],bridges=[];
  const context=createContext({console,MessageChannel:class {port1={};port2={};},getComputedStyle:()=>({colorScheme:'light',getPropertyValue:()=>''})});
  const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const [name,value] of Object.entries(values))this.setExport(name,value);},{context});
  const core=synthetic(solid);
  const universal=new SourceTextModule(await readFile(new URL('../../../node_modules/solid-js/universal/dist/universal.js',import.meta.url),'utf8'),{context});
  await universal.link(()=>core);await universal.evaluate();
  const renderer=universal.namespace.createRenderer({
    createElement:type=>({type,props:{},children:[],parent:null,contentWindow:{postMessage(){}},parentElement:{}}),
    createTextNode:text=>({text,children:[],parent:null}),replaceText:(node,text)=>{node.text=text;},
    setProperty:(node,key,value)=>{node.props[key]=value;},
    insertNode:(parent,node,anchor)=>{if(node.parent)node.parent.children.splice(node.parent.children.indexOf(node),1);const at=anchor?parent.children.indexOf(anchor):-1;parent.children.splice(at<0?parent.children.length:at,0,node);node.parent=parent;},
    removeNode:(parent,node)=>{parent.children.splice(parent.children.indexOf(node),1);node.parent=null;},
    isTextNode:node=>'text'in node,getParentNode:node=>node.parent,getFirstChild:node=>node.children[0],getNextSibling:node=>node.parent?.children[node.parent.children.indexOf(node)+1],
  });
  const native=synthetic({invokeExtension:input=>new Promise((resolve,reject)=>calls.push({input,resolve,reject}))});
  const bridge=synthetic({surfaceBridge:()=>{const value={closed:false,close(){value.closed=true;},event(){}};bridges.push(value);return value;}});
  const source=await readFile(new URL('./components/ExtensionSettings.tsx',import.meta.url),'utf8');
  const compiled=stripTypeScriptTypes(transformSync(source,{filename:'ExtensionSettings.tsx',babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code,{mode:'transform'});
  const module=new SourceTextModule(compiled,{context});
  await module.link(name=>name==='solid-js'?core:name==='test-renderer'?synthetic(renderer):name==='../extension-bridge'?native:bridge);await module.evaluate();
  const choice=id=>({record:{manifest:{id},revision:1},service:{id:'settings',title:id}});
  const [selected,setSelected]=solid.createSignal(choice('first'));
  const root=renderer.createElement('root');
  const dispose=renderer.render(()=>module.namespace.default({get choice(){return selected();},onEscape(){}}),root);
  const iframe=()=>{const walk=node=>node.type==='iframe'?node:node.children.map(walk).find(Boolean);return walk(root);};
  await tick();
  return {calls,bridges,iframe,dispose,switchTo:id=>setSelected(choice(id))};
}
test('late settings mount closes only its own native surface, preserving the current bridge',async()=>{
  const f=await fixture();try{
    const old=f.calls[0];f.switchTo('second');await tick();
    const current=f.calls.find(call=>call.input.pluginId==='second');
    current.resolve({surfaceId:'surface-second',url:'mewu://second'});await tick();f.iframe().props.onLoad();
    const active=f.bridges[0];assert.equal(active.closed,false);
    old.resolve({surfaceId:'surface-first',url:'mewu://first'});await tick();
    assert.equal(active.closed,false,'stale reply cannot revoke a different settings document');
    assert.ok(f.calls.some(call=>call.input.surface.action==='close'&&call.input.surface.id==='surface-first'));
    assert.equal(f.iframe().props.sandbox,'allow-scripts');
  }finally{f.dispose();}
  assert.equal(f.bridges.at(-1).closed,true);
  assert.ok(f.calls.some(call=>call.input.surface.action==='close'&&call.input.surface.id==='surface-second'));
});
