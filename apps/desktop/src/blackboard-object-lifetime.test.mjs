// SPDX-License-Identifier: MPL-2.0
// Real Solid owners and production ArtifactCard/MoveGrip handlers. The universal
// renderer records DOM identity/source writes; persistence and native IO are synthetic.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {createContext,SourceTextModule,SyntheticModule} from 'node:vm';
import {parse} from '@babel/parser';
import {transformSync} from '@babel/core';
import solidPreset from 'babel-preset-solid';

const solidPackage=import.meta.resolve('solid-js/package.json');
const solid=await import(new URL('./dist/solid.js',solidPackage));
const universal=await readFile(new URL('./universal/dist/universal.js',solidPackage),'utf8');
const source=await readFile(new URL('./components/SpaceCanvas.tsx',import.meta.url),'utf8');
const editor=await readFile(process.env.MEWU_BOARD_EDITOR_SOURCE??new URL('./components/SharedDrawingEditor.tsx',import.meta.url),'utf8');
const card=await readFile(new URL('./components/ArtifactCard.tsx',import.meta.url),'utf8');
const adapter=await readFile(new URL('./components/DrawingEditor.tsx',import.meta.url),'utf8');
const bridge=await readFile(new URL('./bridge.ts',import.meta.url),'utf8');
function nodesFor(text){
 const result=[];function visit(node){if(!node||typeof node!=='object')return;if(node.type)result.push(node);for(const [key,value]of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(visit);else if(value&&typeof value==='object')visit(value);}}
 visit(parse(text,{sourceType:'module',plugins:['typescript','jsx']}));return result;
}
const nodes=nodesFor(source),editorNodes=nodesFor(editor),adapterNodes=nodesFor(adapter),bridgeNodes=nodesFor(bridge);
const declaration=(text,nodes,name,optional=false)=>{const node=nodes.find(node=>node.type==='VariableDeclarator'&&node.id.name===name);if(optional&&!node)return '';assert.ok(node,name);return `const ${text.slice(node.start,node.end)};`;};
const editorElement=nodes.find(node=>node.type==='JSXOpeningElement'&&node.name.name==='DrawingEditor');
const objectProvider=editorElement.attributes.find(node=>node.name?.name==='objects').value.expression;
const sharedAdapter=adapterNodes.find(node=>node.type==='JSXOpeningElement'&&node.name.name==='SharedDrawingEditor');
const forwardedObjects=sharedAdapter.attributes.find(node=>node.name?.name==='objects').value.expression;
const stableSlot=editorNodes.find(node=>node.type==='JSXElement'&&node.openingElement.name.name==='Show'&&node.openingElement.attributes.some(node=>node.name?.name==='when'&&node.value.expression?.callee?.name==='objectLayerIdentity'));
const originalSlot=editorNodes.find(node=>node.type==='JSXExpressionContainer'&&node.expression?.type==='OptionalCallExpression'&&node.expression.callee.object?.name==='props'&&node.expression.callee.property?.name==='objects');
const objectSlot=stableSlot??originalSlot;assert.ok(objectSlot,'use actual production object insertion');
const pure=text=>stripTypeScriptTypes(text,{mode:'transform'});
const compile=(text,filename)=>pure(transformSync(text,{filename,babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code);
const harness=`import {For,Show,createMemo,createSignal,untrack} from 'solid-js';
import ArtifactCard from './components/ArtifactCard';
export default function Board(props){
 const geometry=()=>({x:0,y:0,width:1200,height:800}),viewport=()=>({width:1200,height:800});
 const projectBackgroundItem=item=>item;
 ${declaration(source,nodes,'backgroundAnchored')}
 ${declaration(source,nodes,'displayedItem')}
 const activeItemId=()=>'',hoveredObject=()=>'',objectTrash=()=>props.trash,drawingSession=()=>({kind:'document'}),imageDrawing=()=>undefined;
 const objectWheels=props.wheels,objectFlushes=props.flushes,videoDrawingGrant=()=>undefined,videoGrant=()=>undefined;
 const isBlackboardObject=asset=>['image','video','text','html','svg'].includes(asset.kind),updateItem=item=>props.onUpdateItem(item);
 ${declaration(source,nodes,'ArtifactItems')}
 function Shared(props){
  const [tool,setTool]=createSignal('select');props.control.tool=setTool;
  const identity=()=>props.port.draftKey();
  ${declaration(editor,editorNodes,'objectLayerIdentity',true)}
  ${declaration(editor,editorNodes,'ObjectLayer',true)}
  return <main>${editor.slice(objectSlot.start,objectSlot.end)}</main>;
 }
 function Adapter(props){return <Shared objects={${adapter.slice(forwardedObjects.start,forwardedObjects.end)}} port={props.port} control={props.control}/>;}
 return <Adapter objects={${source.slice(objectProvider.start,objectProvider.end)}} port={props.port} control={props.control}/>;
}`;

async function fixture({defer=false,fail=false}={}){
 const timers=new Map(),writes=[],removed=[],created=[],errors=[],updates=[],flushes=new Map(),wheels=new Map(),control={},trashCalls=[];
 let timerId=0,resolveWrite;
 class Element extends EventTarget {
  constructor(type){super();this.type=type;this.children=[];this.props={};this.parent=null;this.captures=new Set();created.push(this);}
  closest(selector){let node=this;while(node){if(selector.split(',').some(part=>part.trim()===node.type||part.trim()==='[role="toolbar"]'&&node.props.role==='toolbar'))return node;node=node.parent;}return null;}
  contains(node){while(node){if(node===this)return true;node=node.parent;}return false;}
  setPointerCapture(id){this.captures.add(id);}
  hasPointerCapture(id){return this.captures.has(id);}
  releasePointerCapture(id){this.captures.delete(id);}
 }
 const document=Object.assign(new EventTarget(),{activeElement:null,hidden:false}),window=Object.assign(new EventTarget(),{innerWidth:1200,innerHeight:800});
 const context=createContext({console,queueMicrotask,Error,Event,EventTarget,Element,HTMLIFrameElement:class extends Element{},document,window,innerWidth:1200,innerHeight:800,
  setTimeout:callback=>{timers.set(++timerId,callback);return timerId;},clearTimeout:id=>timers.delete(id)});
 const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const [key,value]of Object.entries(values))this.setExport(key,value);},{context});
 const core=synthetic(solid),rendererModule=new SourceTextModule(universal,{context});await rendererModule.link(()=>core);await rendererModule.evaluate();
 const renderer=rendererModule.namespace.createRenderer({
  createElement:type=>new Element(type),createTextNode:text=>Object.assign(new Element('#text'),{text}),replaceText:(node,text)=>{node.text=text;},
  setProperty:(node,key,value)=>{if(key==='src')writes.push({node,value});if(/^on[A-Z]/.test(key)){const type=key.slice(2).toLowerCase();if(node.props[key])node.removeEventListener(type,node.props[key]);if(value)node.addEventListener(type,value);}node.props[key]=value;},
  insertNode:(parent,node,anchor)=>{if(node.parent)node.parent.children.splice(node.parent.children.indexOf(node),1);const at=anchor?parent.children.indexOf(anchor):-1;parent.children.splice(at<0?parent.children.length:at,0,node);node.parent=parent;},
  removeNode:(parent,node)=>{removed.push(node);parent.children.splice(parent.children.indexOf(node),1);node.parent=null;},isTextNode:node=>node.type==='#text',
  getParentNode:node=>node.parent,getFirstChild:node=>node.children[0],getNextSibling:node=>node.parent?.children[node.parent.children.indexOf(node)+1],
 });
 const renderModule=synthetic(renderer),moduleCache=new Map();
 async function load(name){
  if(moduleCache.has(name))return moduleCache.get(name);
  if(name==='solid-js')return core;if(name==='test-renderer')return renderModule;
  if(name.endsWith('.css'))return synthetic({});
  if(name.endsWith('/i18n'))return synthetic({t:text=>text});
  if(name==='lucide-solid')return synthetic(Object.fromEntries(['Code2','Copy','Download','File','Image','Link2','Maximize2','Pencil','Pin','RotateCcw','Video','X'].map(name=>[name,()=>null])));
  if(name.endsWith('/bridge')){
   const functions=['assetUrl','documentUrl'].map(name=>{const node=bridgeNodes.find(node=>node.type==='FunctionDeclaration'&&node.id.name===name);return bridge.slice(node.start,node.end);}).join('\n');
   const module=new SourceTextModule(pure(`const native=true,runtime={contentOrigin:'http://synthetic.invalid/content'};${functions}\nconst readArtifact=async()=>'',isolatedDocument=text=>text;export {native,assetUrl,documentUrl,readArtifact,isolatedDocument};`),{context});moduleCache.set(name,module);await module.link(load);await module.evaluate();return module;
  }
  const allowed=['ArtifactCard','ObjectMoveGrip'];
  if(name.startsWith('./')&&['BlackboardText','TextArtifact','VideoArtifact','DocumentArtifact','DrawingLayer','ImageExportButton'].some(value=>name.endsWith('/'+value)||name==='./'+value))return synthetic({default:()=>null});
  const file=name.startsWith('../')?name.slice(3):name.startsWith('./components/')?name.slice(2):name.startsWith('./')?`components/${name.slice(2)}`:name;
  const isComponent=allowed.some(value=>file.endsWith(value));
  const text=await readFile(new URL('./'+file+(isComponent?'.tsx':'.ts'),import.meta.url),'utf8');
  const module=new SourceTextModule(isComponent?compile(text,file+'.tsx'):pure(text),{context});moduleCache.set(name,module);await module.link(load);await module.evaluate();return module;
 }
 const module=new SourceTextModule(compile(harness,'board-object-owner.tsx'),{context});await module.link(load);await module.evaluate();
 const initial={id:'board',kind:'blackboard',background:{id:'board-background',name:'board.board.png',path:'board.png'},regions:[],items:[{id:'picture',asset:{id:'image',name:'capture.png',kind:'image',path:'capture.png',width:600,height:300},x:.2,y:.2,width:.5,height:.375}]};
 const [scene,setScene]=solid.createSignal(initial);
 const trash={begin:()=>trashCalls.push('begin'),move:()=>trashCalls.push('move'),finish:()=>{trashCalls.push('finish');return control.remove??false;},cancel:()=>trashCalls.push('cancel')};
 const props={get scene(){return scene();},get blackboard(){return scene().kind==='blackboard';},blackboardEnabled:true,refs:[],flushes,wheels,control,trash,
  port:{draftKey:()=>JSON.stringify([scene().id,scene().background.id]),sourceIdentity:()=>scene().background.id},
  onError:error=>errors.push(error),onReference(){},onRemoveItem:id=>{setScene(previous=>({...previous,items:previous.items.filter(item=>item.id!==id)}));},
  onUpdateItem:async item=>{updates.push(item);if(defer)await new Promise(resolve=>{resolveWrite=resolve;});if(fail)throw Error('synthetic persistence failed');setScene(previous=>({...previous,items:previous.items.map(saved=>saved.id===item.id?structuredClone(item):saved)}));},
 };
 const root=renderer.createElement('root'),dispose=renderer.render(()=>module.namespace.default(props),root);await Promise.resolve();
 const all=()=>{const result=[];const visit=node=>{result.push(node);node.children.forEach(visit);};visit(root);return result;};
 const image=()=>all().find(node=>node.type==='img'),shell=()=>all().find(node=>node.props.class==='artifact-card'),grip=()=>all().find(node=>node.props.class==='object-move-grip');
 function pointer(target,type,x,y){const event=new Event(type,{cancelable:true});Object.assign(event,{button:0,pointerId:1,clientX:x,clientY:y});target.dispatchEvent(event);return event;}
 function wheel(){const event={target:image(),deltaY:-80,deltaMode:0,clientX:400,clientY:250,preventDefault(){this.defaultPrevented=true;},stopPropagation(){}};wheels.get('picture')(event);assert.equal(event.defaultPrevented,true);}
 async function flush(){for(const handler of [...flushes.values()])await handler(()=>true);}
 return{scene,setScene,root,dispose,image,shell,grip,wheel,pointer,flush,control,created,removed,writes,errors,updates,flushes,wheels,trashCalls,resolve:()=>resolveWrite?.(),setFailure:value=>{fail=value;}};
}

test('board wheel persistence preserves actual image/card/grip owners and writes no repeated media source',async()=>{
 const f=await fixture();try{
  const image=f.image(),shell=f.shell(),grip=f.grip(),src=image.props.src;assert.ok(image&&shell&&grip);const writes=f.writes.length;
  for(let i=0;i<4;i++){f.wheel();const preview={...shell.props.style};await f.flush();assert.equal(f.image(),image);assert.equal(f.shell(),shell);assert.equal(f.grip(),grip);assert.deepEqual({...shell.props.style},preview);}
  assert.equal(f.updates.length,4);assert.equal(f.writes.length,writes);assert.equal(image.props.src,src);assert.equal(f.created.filter(node=>node.type==='img').length,1);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();}
});

test('board pointer release keeps the same image through delayed commit and the actual trash path still removes it',async()=>{
 const f=await fixture({defer:true});try{
  const image=f.image(),shell=f.shell(),grip=f.grip();f.pointer(grip,'pointerdown',350,180);f.pointer(grip,'pointermove',430,240);const preview={...shell.props.style};f.pointer(grip,'pointerup',430,240);
  await Promise.resolve();assert.equal(f.updates.length,1);assert.equal(f.image(),image);assert.deepEqual({...shell.props.style},preview);f.resolve();await new Promise(resolve=>setImmediate(resolve));
  assert.equal(f.image(),image);assert.deepEqual({...shell.props.style},preview);assert.equal(f.created.filter(node=>node.type==='img').length,1);assert.ok(f.trashCalls.includes('finish'));
  f.control.remove=true;f.pointer(grip,'pointerdown',430,240);f.pointer(grip,'pointermove',500,300);f.pointer(grip,'pointerup',500,300);await new Promise(resolve=>setImmediate(resolve));
  assert.equal(f.image(),undefined);assert.equal(f.flushes.size,0);assert.equal(f.wheels.size,0);assert.equal(f.scene().items.length,0);
 }finally{f.dispose();}
});

test('board drawing receipts and select-tool changes retain objects while a real asset replacement updates src',async()=>{
 const f=await fixture();try{
  const image=f.image(),shell=f.shell(),grip=f.grip(),writes=f.writes.length;
  for(let revision=1;revision<=3;revision++){f.setScene({...structuredClone(f.scene()),regions:[{id:'drawing',drawingRevision:revision,drawings:[{id:'stroke-'+revision}]}]});f.control.tool('pen');assert.equal(f.image(),image);assert.equal(f.shell(),shell);f.control.tool('select');assert.equal(f.grip(),grip);}
  assert.equal(f.writes.length,writes);
  f.setScene({...f.scene(),items:f.scene().items.map(item=>({...item,asset:{...item.asset,id:'new-image',name:'new-capture.png'}}))});
  assert.equal(f.image(),image);assert.equal(image.props.src,'http://synthetic.invalid/content/new-image');assert.equal(f.writes.length,writes+1);
 }finally{f.dispose();}
});

test('true board source change and layer withdrawal clean old owners and registered flushes',async()=>{
 const f=await fixture();try{
  const first=f.image();f.wheel();f.setScene({...f.scene(),background:{...f.scene().background,id:'replacement-background'}});
  assert.notEqual(f.image(),first);assert.equal(f.created.filter(node=>node.type==='img').length,2);assert.equal(f.flushes.size,1);assert.equal(f.wheels.size,1);await f.flush();assert.equal(f.updates.length,0,'removed source cannot commit its wheel draft');
  f.setScene({...f.scene(),kind:'capture'});assert.equal(f.image(),undefined);assert.equal(f.flushes.size,0);assert.equal(f.wheels.size,0);
 }finally{f.dispose();}
});

test('failed wheel writes retain the visible edit and owner for explicit retry',async()=>{
 const f=await fixture({fail:true});try{
  const image=f.image(),shell=f.shell();f.wheel();const preview={...shell.props.style};await assert.rejects(f.flush(),/synthetic persistence failed/);assert.equal(f.image(),image);assert.deepEqual({...shell.props.style},preview);assert.equal(f.updates.length,1);
  f.setFailure(false);await f.flush();assert.equal(f.updates.length,2);assert.equal(f.image(),image);assert.deepEqual({...shell.props.style},preview);
 }finally{f.dispose();}
});
