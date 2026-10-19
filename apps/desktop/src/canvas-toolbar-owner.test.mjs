// SPDX-License-Identifier: MPL-2.0
// Execute production selection handlers, ArtifactItems JSX, toolbar admission,
// and ArtifactCard's real pointer/focus activation with Solid's reactive runtime.
// DOM/media rendering and native persistence are synthetic; no user data or GUI.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';

const desktop = import.meta.resolve('@mewu/desktop/package.json');
const solidPackage = import.meta.resolve('solid-js/package.json');
const solid = await import(new URL('./dist/solid.js', solidPackage));
const source = await readFile(process.env.MEWU_CANVAS_OWNER_SOURCE ?? new URL('./src/components/SpaceCanvas.tsx', desktop), 'utf8');
const cardSource = await readFile(new URL('./src/components/ArtifactCard.tsx', desktop), 'utf8');
const videoSource = await readFile(new URL('./src/components/VideoArtifact.tsx', desktop), 'utf8');
const universalSource = await readFile(new URL('./universal/dist/universal.js', solidPackage), 'utf8');
const nodesFor = text => {
  const nodes = [];
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type) nodes.push(node);
    for (const [key, value] of Object.entries(node)) {
      if (key === 'loc') continue;
      if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
    }
  }
  visit(parse(text, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
  return nodes;
};
const nodes = nodesFor(source), cardNodes = nodesFor(cardSource), videoNodes = nodesFor(videoSource);
const videoSlice = node => videoSource.slice(node.start, node.end);
const videoScopeNode = videoNodes.find(node => node.type === 'VariableDeclarator' && node.id.name === 'playbackScope');
const videoScopeEffect = videoNodes.find(node => node.type === 'CallExpression' && node.callee.name === 'createEffect' && node.arguments[0]?.callee?.name === 'on' && node.arguments[0].arguments[0]?.name === 'playbackScope');
const drawingEditor = videoNodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'VideoDrawingEditor');
const drawingOwner = videoNodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < drawingEditor.start && node.end > drawingEditor.end);
const slice = node => source.slice(node.start, node.end);
const variable = name => {
  const node = nodes.find(node => node.type === 'VariableDeclarator' && (node.id.name === name
    || node.id.type === 'ArrayPattern' && node.id.elements.some(value => value?.name === name)));
  assert.ok(node, name); return `const ${slice(node)};`;
};
const handler = (name, optional = false) => {
  const node = nodes.find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
  if (optional && !node) return '';
  assert.ok(node, name); return slice(node);
};
const toolbar = nodes.find(node => node.type === 'JSXElement' && node.openingElement.attributes.some(attr => attr.name?.name === 'class' && attr.value?.value === 'capture-toolbar'));
const toolbarShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < toolbar.start && node.end > toolbar.end);
const toolbarWhen = toolbarShow.openingElement.attributes.find(attr => attr.name?.name === 'when').value.expression;
const activate = cardNodes.find(node => node.type === 'VariableDeclarator' && node.id.name === 'activate');
const shell = cardNodes.find(node => node.type === 'JSXOpeningElement' && node.name.name === 'section' && node.attributes.some(attr => attr.name?.name === 'class' && attr.value?.value === 'artifact-card'));
const shellRef = shell.attributes.find(attr => attr.name?.name === 'ref').value.expression;
const harness = `import { batch, createSignal, createMemo, createEffect, For, Show, untrack, on, onCleanup } from 'solid-js';
import {VideoPlayback} from 'test-video-playback';
import {DrawingInputPhase,drawingDiagnosticsEnabled,recordDrawingInput} from './drawing-diagnostics';
export default function Owner(props) {
  let disposed = false, interactionVersion = 0, pendingSelectionEpoch = 0, flashTimer;
  const imageDrawingItem = () => props.scene.items.find(item => item.id === imageDrawing()?.itemId);
  ${['active', 'activeItemId', 'toolbarVisible', 'pendingSelection', 'pendingSelectionOutlines', 'activity', 'forceNew', 'pluginMenu', 'imageExportOpen', 'imageDrawing', 'flashedRegion', 'backgroundAnchored', 'displayedItem'].map(variable).join('\n')}
  const drawingSession = () => undefined, exporting = () => false, settling = () => false, extensionRunning = () => false, extensionInteracting = () => false;
  const viewport = () => ({ width: 1200, height: 800 }), geometry = () => ({x:0,y:0,width:1200,height:800,scale:1});
  const projectBackgroundItem = item => item, regionById = id => props.scene.regions.find(region => region.id === id);
  ${variable('activeRegion')}
  const blocked = () => false, videoGrant = () => undefined, videoDrawingGrant = () => undefined;
  const hoveredObject = () => '', objectTrash = () => undefined, objectWheels = new Map(), objectFlushes = new Map();
  const isBlackboardObject = asset => ['image','video','text','html','svg'].includes(asset.kind);
  const updateItem = item => props.onUpdateItem(item), exportImage = () => {};
  const clearHideTimer = () => {}, placeToolbar = region => props.placed.push(region.id), endNudge = () => {}, scheduleHide = () => {};
  const clearSnapPreview = () => {}, updateSnapPreview = () => {}, composerBox = () => undefined, toolbarBox = () => ({x:0,y:0,width:0,height:0}), TOLERANCE=24;
  const regions=()=>props.scene.regions.map(region=>({region,box:region})), contains=(box,x,y,pad=0)=>x>=box.x-pad&&y>=box.y-pad&&x<=box.x+box.width+pad&&y<=box.y+box.height+pad;
  const snapshotSelections = {bind(){}}, setSnapshotVersion = () => {}, sameGeometry = (a,b) => ['x','y','width','height'].every(k => a[k] === b[k]);
  const canvas = { focus(){} }, deferGeometryPointer = () => false, begin = () => {}, candidateAt = () => undefined, snapResize = value => value;
  const clamp = (value,low,high) => Math.max(low,Math.min(Math.max(low,high),value));
  const geometryTarget = (scene,id) => {const r = scene.regions.find(r=>r.id===id);return r&&{geometry:r,width:1200,height:800};};
  const trackPointer = (event, move, finish) => finish(true);
  ${['clearPendingSelection', 'retirePendingSelection', 'removePendingOutline', 'currentPendingSource', 'currentPendingSelection', 'reconcilePendingSelection', 'hideToolbar', 'revealToolbar', 'setSelectionActivity', 'editRegion', 'hover'].map(name => handler(name)).join('\n')}
  ${handler('activateItem', true)}
  ${variable('beginImageDrawing')}
  ${variable('focusReference')}
  function VideoScope(props) {
    const video=new EventTarget();Object.assign(video,{paused:true,readyState:2,seeking:false,currentTime:.1,ended:false,
      play(){this.paused=false;this.dispatchEvent(new Event('play'));return Promise.resolve();},
      pause(){if(!this.paused){this.paused=true;this.dispatchEvent(new Event('pause'));}},removeAttribute(){},load(){}});
    const annotationKey=()=>props.item.asset.id,range=()=>({startTicks:0,endTicks:10000000}),position=()=>1000000,clampPosition=(value)=>value;
    const playback=new VideoPlayback(video,{identity:annotationKey,range,position(){},playing(){},error:error=>{throw error;}});
    const pausePlayback=()=>{props.probe.pauseCalls++;playback.pause();};
    const ${videoSlice(videoScopeNode)};
    ${videoSlice(videoScopeEffect)};
    const [drawingOpen,setDrawingOpen]=createSignal(false),metadata=()=>({width:300,height:200}),drawingBox=()=>({x:100,y:100,width:300,height:200});
    const renderStoredDrawing=()=>null,visibleAnnotations=()=>[],Portal=props=>props.children;
    function VideoDrawingEditor(){const instance={id:props.item.id};props.probe.editorsMounted.push(instance);onCleanup(()=>props.probe.editorsDisposed.push(instance));return <section data-video-drawing={props.item.id}/>;}
    const owner={video,play:()=>playback.resumeCurrent(),openDrawing:()=>setDrawingOpen(true)};props.probe.videoOwners.set(props.item.id,owner);
    onCleanup(()=>{playback.dispose();if(props.probe.videoOwners.get(props.item.id)===owner)props.probe.videoOwners.delete(props.item.id);});
    return ${videoSlice(drawingOwner)};
  }
  function ArtifactCard(props) {
    const instance = {id:props.item.id}; props.probe.mounted.push(instance); let shell;
    const ${cardSource.slice(activate.start, activate.end)};
    onCleanup(() => { props.probe.disposed.push(instance); shell?.removeEventListener('pointerdown', activate, true); shell?.removeEventListener('focusin', activate); });
    createEffect(() => props.probe.observed.push({instance, active:props.active, asset:props.item.asset.id}));
    return <article ref={${cardSource.slice(shellRef.start, shellRef.end)}} data-item={props.item.id}>
      <Show when={props.active}><aside data-object-toolbar={props.item.id} /></Show>
      <Show when={props.item.asset.kind==='video'}><VideoScope {...props.video} item={props.item} active={props.active} probe={props.probe}/></Show>
    </article>;
  }
  const Card = ArtifactCard;
  function ProbedCard(cardProps) { return <Card {...cardProps} probe={props.probe} />; }
  ${variable('ArtifactItems').replace('ArtifactCard', 'ProbedCard')}
  createEffect(reconcilePendingSelection);
  createEffect(() => props.frames.push({item:activeItemId(),region:active(),toolbar:Boolean(${slice(toolbarWhen)})}));
  onCleanup(() => { disposed = true; clearTimeout(flashTimer); });
  Object.assign(props.control, {
    revealToolbar, editRegion, beginImageDrawing, focusReference, hover, exportHold:setImageExportOpen,
    pending: value => setPendingSelection(value),
    state: () => ({item:activeItemId(),region:active(),pending:pendingSelection(),imageDrawing:imageDrawing()})
  });
  return <main><ArtifactItems board={Boolean(props.blackboard)}/><Show when={${slice(toolbarWhen)}}>{region => <nav class="capture-toolbar" data-region={region().id}/>}</Show></main>;
}`;
const plain = text => stripTypeScriptTypes(text, { mode: 'transform' });
const compiled = plain(transformSync(harness, { filename: 'canvas-toolbar-owner.tsx', babelrc: false, configFile: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code);
const trimUrl = `data:text/javascript;base64,${Buffer.from(plain(await readFile(new URL('./src/video-trim.ts', desktop),'utf8'))).toString('base64')}`;
const mediaCode = plain(await readFile(new URL('./src/video-media.ts', desktop),'utf8')).replace("'./video-trim'",JSON.stringify(trimUrl));
const {VideoPlayback} = await import(`data:text/javascript;base64,${Buffer.from(mediaCode).toString('base64')}`);

async function fixture({ kind = 'video', blackboard = false } = {}) {
  class Element { closest(selector){return selector==='.capture-surface'?this:null;} }
  const context = createContext({ console, queueMicrotask, setTimeout, clearTimeout, Event, EventTarget, Element, document: { hidden:false } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function() { for (const [key,value] of Object.entries(values)) this.setExport(key,value); }, { context });
  const core = synthetic(solid), universal = new SourceTextModule(universalSource, { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({type,children:[],props:{},parent:null,listeners:new Map(),addEventListener(name,fn){(this.listeners.get(name)??(this.listeners.set(name,new Set()),this.listeners.get(name))).add(fn);},removeEventListener(name,fn){this.listeners.get(name)?.delete(fn);}}),
    createTextNode:text=>({text,children:[],parent:null}), replaceText:(node,text)=>{node.text=text;},setProperty:(node,key,value)=>{node.props[key]=value;},
    insertNode:(parent,node,anchor)=>{if(node.parent)node.parent.children.splice(node.parent.children.indexOf(node),1);const index=anchor?parent.children.indexOf(anchor):-1;parent.children.splice(index<0?parent.children.length:index,0,node);node.parent=parent;},
    removeNode:(parent,node)=>{parent.children.splice(parent.children.indexOf(node),1);node.parent=null;},isTextNode:node=>'text'in node,
    getParentNode:node=>node.parent,getFirstChild:node=>node.children[0],getNextSibling:node=>node.parent?.children[node.parent.children.indexOf(node)+1],
  });
  const diagnosticModule = new SourceTextModule(plain(await readFile(new URL('./src/drawing-diagnostics.ts', desktop), 'utf8')), {context});
  await diagnosticModule.link(() => { throw Error('unexpected diagnostics import'); }); await diagnosticModule.evaluate();
  const module = new SourceTextModule(compiled, {context}); await module.link(name=>name==='solid-js'?core:name==='test-video-playback'?synthetic({VideoPlayback}):name==='./drawing-diagnostics'?diagnosticModule:synthetic(renderer));await module.evaluate();
  const [scene,setScene] = solid.createSignal({id:'scene',background:{id:'background',width:1200,height:800},regions:[{id:'region',x:100,y:100,width:300,height:200}],items:[{id:'object',asset:{id:'asset',kind},x:.1,y:.2,width:.3,height:.4}]});
  const control={},probe={mounted:[],disposed:[],observed:[],videoOwners:new Map(),pauseCalls:0,editorsMounted:[],editorsDisposed:[]},frames=[],placed=[],completions=[],updates=[];
  const props={get scene(){return scene();},blackboard,blackboardEnabled:true,busy:false,inputLocked:false,aiAvailable:true,refs:[{kind:'region',id:'region'},{kind:'item',id:'object'}],control,probe,frames,placed,
    onImageDrawingCommand:async()=>{},onUpdateItem:item=>updates.push(item),onError:error=>{throw Error(error);},onReference(){},onRemoveItem(){},
    video:{drawing:{}},geometryView:{},onSelectionComplete:region=>completions.push(region),onSelectionActivity(){},geometryController:{beginPointer:()=>true,endPointer:async()=>{}}};
  const root=renderer.createElement('root'),dispose=renderer.render(()=>module.namespace.default(props),root);
  const all=()=>{const nodes=[];const visit=node=>{nodes.push(node);node.children.forEach(visit);};visit(root);return nodes;};
  const object=()=>all().find(node=>node.props?.['data-item']==='object');
  const activateObject=(event='pointerdown')=>{const listeners=object().listeners.get(event);assert.ok(listeners?.size);for(const listener of listeners)listener({});};
  return{control,probe,frames,placed,completions,updates,scene,setScene,props,dispose,object,activateObject,
    hoverRegion:()=>control.hover({target:new Element(),buttons:0,clientX:150,clientY:150,shiftKey:false}),
    regionToolbar:()=>all().find(node=>node.props?.class==='capture-toolbar'),objectToolbar:()=>all().find(node=>node.props?.['data-object-toolbar']==='object')};
}

for (const [name,options] of [['video',{kind:'video'}],['image',{kind:'image'}],['board object',{kind:'image',blackboard:true}]]) {
  test(`selecting ${name} atomically retires the existing screenshot region toolbar`,async()=>{
    const f=await fixture(options);
    try{
      f.control.revealToolbar(f.scene().regions[0]);assert.ok(f.regionToolbar());const card=f.object();
      f.activateObject();assert.equal(f.regionToolbar(),undefined);assert.ok(f.objectToolbar());assert.equal(f.control.state().region,'');
      assert.equal(f.object(),card);assert.equal(f.probe.mounted.length,1);assert.equal(f.probe.disposed.length,0);
      assert.ok(f.frames.every(frame=>!frame.toolbar||!frame.item),'no reactive observer sees both toolbar owners');assert.equal(f.props.refs.length,2);assert.deepEqual(f.updates,[]);
    }finally{f.dispose();}
  });
}

test('clicking a region collapses video controls without replacing the media card',async()=>{
  const f=await fixture();
  try{
    f.activateObject();const card=f.object();assert.ok(f.objectToolbar());
    f.control.editRegion({button:0,shiftKey:false,clientX:150,clientY:150},f.scene().regions[0]);await Promise.resolve();
    assert.equal(f.objectToolbar(),undefined);assert.ok(f.regionToolbar());assert.equal(f.control.state().item,'');assert.equal(f.object(),card);assert.equal(f.probe.mounted.length,1);
  }finally{f.dispose();}
});

test('hovering another region keeps an actually playing video and its toolbar active',async()=>{
  const f=await fixture();
  try{f.activateObject();const owner=f.probe.videoOwners.get('object');await owner.play();assert.equal(owner.video.paused,false);const pauses=f.probe.pauseCalls;
    f.hoverRegion();assert.ok(f.objectToolbar());assert.equal(f.regionToolbar(),undefined);assert.equal(owner.video.paused,false);assert.equal(f.probe.pauseCalls,pauses);}
  finally{f.dispose();}
});

test('hover leaves the production video drawing Portal owner mounted; an explicit region click still exits it',async()=>{
  const f=await fixture();
  try{f.activateObject();f.probe.videoOwners.get('object').openDrawing();assert.equal(f.probe.editorsMounted.length,1);assert.equal(f.probe.editorsDisposed.length,0);
    f.hoverRegion();assert.equal(f.probe.editorsDisposed.length,0);assert.equal(f.probe.editorsMounted.length,1);assert.ok(f.objectToolbar());
    f.control.editRegion({button:0,shiftKey:false,clientX:150,clientY:150},f.scene().regions[0]);await Promise.resolve();
    assert.equal(f.probe.editorsDisposed.length,1);assert.equal(f.objectToolbar(),undefined);assert.ok(f.regionToolbar());}
  finally{f.dispose();}
});

test('without an active object the original region-hover toolbar still appears',async()=>{
  const f=await fixture();try{f.hoverRegion();assert.ok(f.regionToolbar());assert.equal(f.objectToolbar(),undefined);}finally{f.dispose();}
});

test('an open export-choice hold cannot keep the old region toolbar over a newly activated object',async()=>{
  const f=await fixture();try{f.control.revealToolbar(f.scene().regions[0]);f.control.exportHold(true);f.activateObject();assert.equal(f.regionToolbar(),undefined);assert.ok(f.objectToolbar());assert.equal(f.control.state().region,'');assert.equal(f.scene().regions.length,1);assert.deepEqual(f.updates,[]);}finally{f.dispose();}
});

test('@ item and region references and iframe focus use the same mutually exclusive owners',async()=>{
  const f=await fixture({kind:'html'});
  try{
    f.control.revealToolbar(f.scene().regions[0]);f.activateObject('focusin');assert.equal(f.regionToolbar(),undefined);assert.ok(f.objectToolbar());
    f.control.focusReference({detail:{kind:'region',id:'region'}});assert.equal(f.objectToolbar(),undefined);assert.ok(f.regionToolbar());
    f.control.focusReference({detail:{kind:'item',id:'object'}});assert.equal(f.regionToolbar(),undefined);assert.ok(f.objectToolbar());
  }finally{f.dispose();}
});

test('activating an object revokes a pending region outline so its late real snapshot cannot steal selection',async()=>{
  const f=await fixture();
  try{
    const region={id:'late',x:500,y:100,width:200,height:100};
    f.control.pending({sceneId:'scene',backgroundId:'background',region,version:0,complete:true});
    f.activateObject();assert.equal(f.control.state().pending,undefined);
    f.setScene({...f.scene(),regions:[...f.scene().regions,region]});
    assert.ok(f.objectToolbar());assert.equal(f.regionToolbar(),undefined);assert.deepEqual(f.completions,[]);assert.equal(f.scene().regions.length,2);
  }finally{f.dispose();}
});

test('same-ID snapshots preserve the selected media owner and entering image drawing clears the old region selection',async()=>{
  const f=await fixture({kind:'image'});
  try{
    f.activateObject();const card=f.object();f.setScene(structuredClone(f.scene()));assert.equal(f.object(),card);assert.ok(f.objectToolbar());assert.equal(f.probe.mounted.length,1);
    f.control.revealToolbar(f.scene().regions[0]);f.control.beginImageDrawing('object');assert.equal(f.control.state().region,'');assert.equal(f.regionToolbar(),undefined);assert.equal(f.control.state().imageDrawing.itemId,'object');
  }finally{f.dispose();}
});
