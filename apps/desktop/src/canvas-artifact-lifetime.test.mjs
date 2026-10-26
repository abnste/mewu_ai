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
const desktopSource=new URL('./src/',import.meta.resolve('@mewu/desktop/package.json'));
const solidPackage=import.meta.resolve('solid-js/package.json');
const solid=await import(new URL('./dist/solid.js',solidPackage));
const modules=new Map();
async function load(name){
 if(modules.has(name))return modules.get(name);
 let code=stripTypeScriptTypes(await readFile(new URL(`./${name}.ts`,desktopSource),'utf8'),{mode:'transform'});
 for(const dependency of [...code.matchAll(/from ['"]\.\/([^'"]+)['"]/g)].map(value=>value[1]))code=code.replaceAll(`'./${dependency}'`,JSON.stringify(await load(dependency))).replaceAll(`"./${dependency}"`,JSON.stringify(await load(dependency)));
 const url=`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;modules.set(name,url);return url;
}
const {VideoPlayback}=await import(await load('video-media'));
const {VideoFlushRegistry,VideoPauseRegistry}=await import(await load('video-requests'));
const {checkedRange}=await import(await load('video-trim'));
const {videoAnnotationPreviewTicks}=await import(await load('video-annotations'));
const videoSource=await readFile(new URL('./components/VideoArtifact.tsx',desktopSource),'utf8');
const videoNodes=[];const videoWalk=node=>{if(!node||typeof node!=='object')return;if(node.type)videoNodes.push(node);for(const value of Object.values(node))if(Array.isArray(value))value.forEach(videoWalk);else if(value&&typeof value==='object')videoWalk(value);};videoWalk(parse(videoSource,{sourceType:'module',plugins:['typescript','jsx']}));
const videoDefinition=name=>{const node=videoNodes.find(value=>value.type==='VariableDeclarator'&&value.id.name===name);assert.ok(node,name);return `const ${videoSource.slice(node.start,node.end)};`;};
const playerNode=videoNodes.find(node=>node.type==='NewExpression'&&node.callee.name==='VideoPlayback');
const observationNode=playerNode.arguments[1].properties.find(node=>node.key.name==='observation').value;
function actualObservation(props,video){
 const context={disposed:false,props,video,checkedRange,videoAnnotationPreviewTicks,metadata:()=>({durationTicks:10000000}),videoNative:true,mediaDuration:()=>10000000,setAnnotationPosition(){},annotationReader:{at(){}}};vm.createContext(context);
 return vm.runInContext(stripTypeScriptTypes(`${videoDefinition('duration')}\n${videoDefinition('range')}\n({range,observation:${videoSource.slice(observationNode.start,observationNode.end)}})`,{mode:'transform'}),context);
}
const universalSource=await readFile(new URL('./universal/dist/universal.js',solidPackage),'utf8');

const source=await readFile(new URL('./components/SpaceCanvas.tsx',desktopSource),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
const nodes=[];
const walk=node=>{if(!node||typeof node!=='object')return;if(node.type)nodes.push(node);for(const [key,value]of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}};
walk(ast);
const definition=nodes.find(node=>node.type==='VariableDeclarator'&&['renderItems','ArtifactItems'].includes(node.id.name));
const layer=nodes.find(node=>node.type==='JSXElement'&&node.openingElement.name.name==='div'&&node.openingElement.attributes.some(attribute=>attribute.name?.name==='class'&&attribute.value?.value==='artifact-layer'));
assert.ok(definition&&layer,'exercise the actual production list definition and call site');
const actual=name=>{const node=nodes.find(value=>value.type==='VariableDeclarator'&&value.id.name===name);assert.ok(node,name);return `const ${source.slice(node.start,node.end)};`;};
test('object flush does not call a removed or replaced card captured before a prior await', async () => {
 const objectFlushes=new Map();let release,stale=0,replacement=0;
 objectFlushes.set('first',()=>new Promise(resolve=>{release=resolve;}));
 objectFlushes.set('removed',async()=>{stale++;throw Error('disposed');});
 objectFlushes.set('replaced',async()=>{stale++;throw Error('disposed');});
 const flush=vm.runInNewContext(stripTypeScriptTypes(`${actual('flushObjects')}\nflushObjects`,{mode:'transform'}),{objectFlushes});
 const pending=flush(()=>true);await new Promise(resolve=>setImmediate(resolve));
 objectFlushes.delete('removed');objectFlushes.set('replaced',async()=>{replacement++;});release();
 await pending;assert.equal(stale,0);assert.equal(replacement,0);
 objectFlushes.delete('first');await flush(()=>true);assert.equal(replacement,1);
});
const appSource=await readFile(process.env.MEWU_SCENE_OWNER_SOURCE??new URL('./App.tsx',desktopSource),'utf8');
const appAst=parse(appSource,{sourceType:'module',plugins:['typescript','jsx']});
const appNodes=[];const appWalk=node=>{if(!node||typeof node!=='object')return;if(node.type)appNodes.push(node);for(const value of Object.values(node))if(Array.isArray(value))value.forEach(appWalk);else if(value&&typeof value==='object')appWalk(value);};appWalk(appAst);
const appDefinition=name=>{const node=appNodes.find(value=>value.type==='VariableDeclarator'&&value.id.name===name);assert.ok(node,name);return `const ${appSource.slice(node.start,node.end)};`;};
const canvas=appNodes.find(node=>node.type==='JSXOpeningElement'&&node.name.name==='SpaceCanvas');
const sceneExpression=canvas.attributes.find(value=>value.name?.name==='scene').value.expression;
const callback=appNodes.find(node=>node.type==='ArrowFunctionExpression'&&node.params[0]?.name==='currentId');
const ownedDeclarations=callback.body.type==='BlockStatement'?callback.body.body.filter(node=>node.type==='VariableDeclaration').map(node=>appSource.slice(node.start,node.end)).join('\n'):'';
const harness=`import {For,Show,createMemo,untrack,onCleanup} from 'solid-js';
export function Harness(props){
 const ArtifactCard=props.Card;
 const geometry=()=>({x:0,y:0,width:100,height:100});
 const viewport=()=>({width:100,height:100});
 const projectBackgroundItem=(item)=>item;
 ${actual('backgroundAnchored')}
 ${actual('displayedItem')}
 const activeItemId=()=>props.activeId;
 const hoveredObject=()=>'';
 const objectTrash=()=>undefined;
 const drawingSession=()=>undefined;
 const videoDrawingGrant=()=>undefined,videoGrant=()=>undefined;
 const objectWheels=new Map(),objectFlushes=new Map();
 const updateItem=()=>{};
 const isBlackboardObject=asset=>['image','video','text','html','svg'].includes(asset.kind);
 const ${source.slice(definition.start,definition.end)};
 return <section data-background={props.scene.background?.id}>${source.slice(layer.start,layer.end)}</section>;
}
export function SceneOwner(props){
 const snapshot=()=>props.snapshot;
 const pinObjects=()=>[];
 const referenceDrafts=()=>({}),drafts=()=>({});
 ${appDefinition('scene')}
 ${appDefinition('canvasScene')}
 const isBlackboard=()=>false;
 return <Show when={scene()?.id} keyed>{currentId=>{
  ${ownedDeclarations}
  props.onOwner?.(currentId,ownedScene);
  onCleanup(()=>props.onOwnerCleanup?.(currentId,ownedScene()));
  return <Harness {...props} scene={${appSource.slice(sceneExpression.start,sceneExpression.end)}}/>;
 }}</Show>;
}
// Held synthetic parent evaluates the actual production fallback branch.
export function HeldSceneOwner(props){
 const snapshot=()=>props.snapshot,scene=()=>props.initialScene;
 const pinObjects=()=>[],referenceDrafts=()=>({}),drafts=()=>({});
 ${appDefinition('canvasScene')}
 const isBlackboard=()=>false,currentId=props.initialScene.id;
 ${ownedDeclarations}
 props.onOwner?.(currentId,ownedScene);
 onCleanup(()=>props.onOwnerCleanup?.(currentId,ownedScene()));
 return <Harness {...props} scene={${appSource.slice(sceneExpression.start,sceneExpression.end)}}/>;
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
async function mount(owner=false,held=false){
 const renderer=await fixture();
 const core=new vm.SyntheticModule(Object.keys(solid),function(){for(const [key,value]of Object.entries(solid))this.setExport(key,value);});
 const dom=new vm.SyntheticModule(Object.keys(renderer),function(){for(const [key,value]of Object.entries(renderer))this.setExport(key,value);});
 const module=new vm.SourceTextModule(compiled);
 await module.link(name=>name==='solid-js'?core:name==='test-renderer'?dom:Promise.reject(Error(name)));
 await module.evaluate();
 const initial={id:'scene',kind:'capture',background:{id:'old-background'},items:[{id:'video',asset:{id:'source',kind:'video',name:'recording.mp4'},x:.1,y:.2,width:.5,height:.4}]};
 const [snapshot,setSnapshot]=solid.createSignal({activeSceneId:initial.id,scenes:[initial]});
 const scene=()=>snapshot().scenes.find(value=>value.id===snapshot().activeSceneId);
 const setScene=value=>setSnapshot(previous=>({...previous,scenes:previous.scenes.map(old=>old.id===value.id?value:old)}));
 const [active,setActive]=solid.createSignal('video');
 const mounted=[],disposed=[],observed=[],players=[],cleanupOwners=[],flushedOwners=[],ownerCleanup=[];const owners=new Map();
 const flushes=new VideoFlushRegistry(),pauses=new VideoPauseRegistry();let flushed=0;
 function Card(props){
  const instance={id:props.item.id,open:true};mounted.push(instance);
  const media=new EventTarget();const listeners=new Map();
  const add=media.addEventListener.bind(media),remove=media.removeEventListener.bind(media);
  media.addEventListener=(type,fn)=>{(listeners.get(type)??(listeners.set(type,new Set()),listeners.get(type))).add(fn);add(type,fn);};
  media.removeEventListener=(type,fn)=>{listeners.get(type)?.delete(fn);remove(type,fn);};Object.assign(media,{paused:true,readyState:2,seeking:false,currentTime:0,pause(){this.paused=true;this.dispatchEvent(new Event('pause'));},removeAttribute(){},load(){}});
  const playback=new VideoPlayback(media,{identity:()=>props.item.asset.id,...actualObservation(props,media),position(){},playing(){},error:()=>{}});
  const stopFlush=flushes.register(async()=>{flushed++;flushedOwners.push({sceneId:props.sceneId,source:props.item.asset.id,revision:props.item.videoAnnotations?.revision??0});await playback.seeks.flush();});
  const stopPause=pauses.register(()=>playback.pause());
  players.push({media,listeners,playback});
  solid.onCleanup(()=>{cleanupOwners.push({sceneId:props.sceneId,source:props.item.asset.id,revision:props.item.videoAnnotations?.revision??0});stopFlush();playback.dispose();stopPause();disposed.push(instance);});
  const source=solid.createMemo(()=>props.item.asset.id);
  const geometry=solid.createMemo(()=>`${props.item.x}:${props.item.y}:${source()}`);
  solid.createEffect(()=>observed.push({instance,revision:props.item.videoAnnotations?.revision??0,active:props.active,geometry:geometry()}));
  return renderer.createElement('card');
 }
 const props={get snapshot(){return snapshot();},get scene(){return scene();},get blackboard(){return scene()?.kind==='blackboard';},get activeId(){return active();},Card,initialScene:initial,refs:[],busy:false,onReference(){},onRemoveItem(){},onOwner:(id,read)=>owners.set(id,read),onOwnerCleanup:(id,value)=>ownerCleanup.push({id,value})};
 const root=renderer.createElement('root');
 const stop=renderer.render(()=>(owner?(held?module.namespace.HeldSceneOwner:module.namespace.SceneOwner):module.namespace.Harness)(props),root);
 await Promise.resolve();
 return{mounted,disposed,observed,players,cleanupOwners,flushedOwners,ownerCleanup,owners,flushes,pauses,get flushed(){return flushed;},stop,setScene,setActive,setSnapshot,scene,root};
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
  assert.deepEqual(f.disposed,[first]);assert.equal(f.root.children[0].children[0].children.length,0);
 }finally{f.stop();}
});


test('actual App keyed scene owner switches media scene to empty new background and unregisters every media listener',async()=>{
 const f=await mount(true);
 try{
  const previous=f.scene(),first=f.mounted[0],player=f.players[0];
  f.setSnapshot({activeSceneId:'new-scene',scenes:[previous,{id:'new-scene',kind:'capture',background:{id:'new-background'},items:[]}]});
  await Promise.resolve();
  assert.deepEqual(f.disposed,[first]);assert.equal(f.mounted.length,1);
  assert.equal(f.root.children[0].props['data-background'],'new-background');
  assert.equal(f.root.children[0].children[0].children.length,0);
  assert.equal([...player.listeners.values()].reduce((sum,value)=>sum+value.size,0),0);
  assert.equal(player.playback.seeks.request({identity:'source',ticks:0}),false);
  await f.flushes.flush(()=>true);f.pauses.pauseAll();assert.equal(f.flushed,0,'no removed player remains registered for exit flushing');
 }finally{f.stop();}
});

test('same-ID replacement after removal gets a new owner while the old media remains fully disposed',async()=>{
 const f=await mount();
 try{
  const first=f.mounted[0],player=f.players[0];f.setScene({...f.scene(),items:[]});
  f.setScene({...f.scene(),items:[{id:'video',asset:{id:'new-source',kind:'video'},x:.2,y:.2,width:.4,height:.4}]});
  assert.equal(f.mounted.length,2);assert.deepEqual(f.disposed,[first]);
  assert.equal([...player.listeners.values()].reduce((sum,value)=>sum+value.size,0),0);
  await f.flushes.flush(()=>true);assert.equal(f.flushed,1,'only the new mounted owner participates');
 }finally{f.stop();}
});


test('actual App keyed owner cleanup keeps its latest same-ID scene when old history disappears',async()=>{
 const f=await mount(true);
 try{
  const initial=f.scene(),first=f.mounted[0];let latest;
  for(const revision of [1,2,3]){
   latest={...f.scene(),draft:'synthetic latest '+revision,items:f.scene().items.map(item=>({...item,asset:{...item.asset},videoAnnotations:{revision,objects:[{id:'synthetic-stroke-'+revision}]}}))};
   f.setScene(latest);await Promise.resolve();assert.equal(f.owners.get(initial.id)(),latest);assert.equal(f.mounted.length,1);assert.equal(f.disposed.length,0);
  }
  f.setSnapshot({activeSceneId:'new-scene',scenes:[{id:'new-scene',kind:'capture',background:{id:'new-background'},items:[]}]});
  await Promise.resolve();
  const old=f.ownerCleanup.find(value=>value.id===initial.id);assert.ok(old);assert.equal(old.value,latest);assert.notEqual(old.value,initial);
  assert.deepEqual(f.disposed,[first]);assert.deepEqual(f.cleanupOwners,[{sceneId:'scene',source:'source',revision:3}]);
 }finally{f.stop();}
});

test('actual App owner cleanup stays on the latest old scene when a different media scene becomes active',async()=>{
 const f=await mount(true);
 try{
  const latest={...f.scene(),items:f.scene().items.map(item=>({...item,videoAnnotations:{revision:7,objects:[]}}))};f.setScene(latest);await Promise.resolve();
  const next={id:'next-scene',kind:'capture',background:{id:'next-background'},items:[{id:'next-video',asset:{id:'next-source',kind:'video'},x:.1,y:.1,width:.4,height:.4}]};
  f.setSnapshot({activeSceneId:next.id,scenes:[latest,next]});await Promise.resolve();
  assert.equal(f.ownerCleanup[0].id,'scene');assert.equal(f.ownerCleanup[0].value,latest);assert.deepEqual(f.cleanupOwners,[{sceneId:'scene',source:'source',revision:7}]);
  assert.equal(f.mounted.length,2);assert.equal(f.disposed.length,1);assert.equal(f.owners.get(next.id)(),next);
  await f.flushes.flush(()=>true);assert.deepEqual(f.flushedOwners,[{sceneId:'next-scene',source:'next-source',revision:0}]);
 }finally{f.stop();}
});

test('same-ID production owner updates keep the actual VideoPlayback alive and flush the latest media once',async()=>{
 const f=await mount(true);
 try{
  const first=f.mounted[0],player=f.players[0];player.media.paused=false;player.media.dispatchEvent(new Event('play'));
  for(const revision of [1,2,3]){f.setScene({...f.scene(),items:f.scene().items.map(item=>({...item,videoAnnotations:{revision,objects:[]}}))});await Promise.resolve();}
  assert.equal(f.mounted.length,1);assert.equal(f.disposed.length,0);assert.equal(f.mounted[0],first);assert.equal(player.media.paused,false);
  await f.flushes.flush(()=>true);assert.deepEqual(f.flushedOwners,[{sceneId:'scene',source:'source',revision:3}]);assert.equal(f.flushed,1);
  f.stop();assert.equal(player.media.paused,true);assert.equal([...player.listeners.values()].reduce((sum,value)=>sum+value.size,0),0);
 }finally{if(!f.disposed.length)f.stop();}
});


test('production owner fallback keeps the latest same-ID media and flush when a held parent temporarily omits its scene',async()=>{
 const f=await mount(true,true);
 try{
  const initial=f.scene(),latest={...initial,items:initial.items.map(item=>({...item,videoAnnotations:{revision:9,objects:[]}}))};
  f.setScene(latest);await Promise.resolve();assert.equal(f.owners.get(initial.id)(),latest);
  f.setSnapshot({activeSceneId:'other',scenes:[{id:'other',kind:'capture',background:{id:'other-background'},items:[]}]});
  await Promise.resolve();assert.equal(f.owners.get(initial.id)(),latest);assert.equal(f.mounted.length,1);assert.equal(f.disposed.length,0);
  assert.equal(f.observed.at(-1).revision,9);await f.flushes.flush(()=>true);
  assert.deepEqual(f.flushedOwners,[{sceneId:'scene',source:'source',revision:9}]);
  f.stop();assert.equal(f.ownerCleanup[0].value,latest);assert.deepEqual(f.cleanupOwners,[{sceneId:'scene',source:'source',revision:9}]);
 }finally{if(!f.disposed.length)f.stop();}
});
