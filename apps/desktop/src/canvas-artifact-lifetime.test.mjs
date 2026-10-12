// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
import {transformSync} from '@babel/core';
import solidPreset from 'babel-preset-solid';
// Use Solid's real reactive runtime even when node's default export condition
// selects the SSR implementation. No global --conditions change for other tests.
const solidPackage=import.meta.resolve('solid-js/package.json');
const solid=await import(new URL('./dist/solid.js',solidPackage));
const universalSource=await readFile(new URL('./universal/dist/universal.js',solidPackage),'utf8');

const source=await readFile(new URL('./components/SpaceCanvas.tsx',import.meta.url),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
const nodes=[];
const walk=node=>{if(!node||typeof node!=='object')return;if(node.type)nodes.push(node);for(const [key,value]of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}};
walk(ast);
const definition=nodes.find(node=>node.type==='VariableDeclarator'&&['renderItems','ArtifactItems'].includes(node.id.name));
const layer=nodes.find(node=>node.type==='JSXElement'&&node.openingElement.name.name==='div'&&node.openingElement.attributes.some(attribute=>attribute.name?.name==='class'&&attribute.value?.value==='artifact-layer'));
assert.ok(definition&&layer,'exercise the actual production list definition and call site');
const harness=`import {For} from 'solid-js';
export function Harness(props){
 const ArtifactCard=props.Card;
 const displayedItem=id=>props.scene.items.find(item=>item.id===id);
 const activeItemId=()=>props.activeId;
 const hoveredObject=()=>'';
 const objectTrash=()=>undefined;
 const drawingSession=()=>undefined;
 const videoDrawingGrant=()=>undefined,videoGrant=()=>undefined;
 const objectWheels=new Map(),objectFlushes=new Map();
 const updateItem=()=>{};
 const isBlackboardObject=asset=>['image','video','text','html','svg'].includes(asset.kind);
 const ${source.slice(definition.start,definition.end)};
 return ${source.slice(layer.start,layer.end)};
}`;
const compiled=stripTypeScriptTypes(transformSync(harness,{filename:'artifact-lifetime.tsx',babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code,{mode:'transform'});

async function fixture(){
 const core=new vm.SyntheticModule(Object.keys(solid),function(){for(const [key,value]of Object.entries(solid))this.setExport(key,value);});
 const universal=new vm.SourceTextModule(universalSource);
 await universal.link(name=>name==='solid-js'?core:Promise.reject(Error(name)));
 await universal.evaluate();
 const renderer=universal.namespace.createRenderer({
  createElement:type=>({type,children:[],props:{},parent:null}),
  createTextNode:text=>({text,children:[],parent:null}),
  replaceText:(node,text)=>{node.text=text;},
  setProperty:(node,name,value)=>{node.props[name]=value;},
  insertNode:(parent,node,anchor)=>{if(node.parent){const list=node.parent.children;list.splice(list.indexOf(node),1);}const at=anchor?parent.children.indexOf(anchor):-1;parent.children.splice(at<0?parent.children.length:at,0,node);node.parent=parent;},
  removeNode:(parent,node)=>{parent.children.splice(parent.children.indexOf(node),1);node.parent=null;},
  isTextNode:node=>'text'in node,
  getParentNode:node=>node.parent,
  getFirstChild:node=>node.children[0],
  getNextSibling:node=>node.parent?.children[node.parent.children.indexOf(node)+1],
 });
 return renderer;
}
async function mount(){
 const renderer=await fixture();
 const core=new vm.SyntheticModule(Object.keys(solid),function(){for(const [key,value]of Object.entries(solid))this.setExport(key,value);});
 const dom=new vm.SyntheticModule(Object.keys(renderer),function(){for(const [key,value]of Object.entries(renderer))this.setExport(key,value);});
 const module=new vm.SourceTextModule(compiled);
 await module.link(name=>name==='solid-js'?core:name==='test-renderer'?dom:Promise.reject(Error(name)));
 await module.evaluate();
 const initial={id:'scene',kind:'capture',items:[{id:'video',asset:{id:'source',kind:'video',name:'recording.mp4'},x:.1,y:.2,width:.5,height:.4}]};
 const [scene,setScene]=solid.createSignal(initial),[active,setActive]=solid.createSignal('video');
 const mounted=[],disposed=[],observed=[];
 function Card(props){
  const instance={id:props.item.id,open:true};mounted.push(instance);
  solid.onCleanup(()=>disposed.push(instance));
  solid.createEffect(()=>observed.push({instance,revision:props.item.videoAnnotations?.revision??0,active:props.active}));
  return renderer.createElement('card');
 }
 const props={get scene(){return scene();},get blackboard(){return scene().kind==='blackboard';},get activeId(){return active();},Card,refs:[],busy:false,onReference(){},onRemoveItem(){}};
 const root=renderer.createElement('root');
 const stop=renderer.render(()=>module.namespace.Harness(props),root);
 await Promise.resolve();
 return{mounted,disposed,observed,stop,setScene,setActive,scene,root};
}

test('committed annotation snapshots keep the same active video component and drawing session',async()=>{
 const f=await mount();
 try{
  const first=f.mounted[0];assert.equal(f.mounted.length,1);
  for(const revision of [1,2,3]){
   f.setScene({...f.scene(),items:f.scene().items.map(item=>({...item,asset:{...item.asset},videoAnnotations:{revision,objects:[{id:`stroke-${revision}`}]}}))});
   await Promise.resolve();
   assert.equal(f.mounted.length,1,'a drawing receipt must not recreate the artifact component');
   assert.equal(f.disposed.length,0);assert.equal(first.open,true);
   assert.equal(f.observed.at(-1).revision,revision);assert.equal(f.observed.at(-1).instance,first);
  }
 }finally{f.stop();}
});

test('moving/selecting retained objects does not recreate cards; removing the object still disposes it',async()=>{
 const f=await mount();
 try{
  const first=f.mounted[0];
  f.setScene({...f.scene(),items:f.scene().items.map(item=>({...item,x:.3,y:.4}))});
  f.setActive('');await Promise.resolve();
  assert.equal(f.mounted.length,1);assert.equal(f.disposed.length,0);assert.equal(f.observed.at(-1).active,false);
  f.setScene({...f.scene(),items:[]});await Promise.resolve();
  assert.deepEqual(f.disposed,[first]);assert.equal(f.root.children[0].children.length,0);
 }finally{f.stop();}
});
