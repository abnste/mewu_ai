// Run real shared-editor input handlers with synthetic DOM/frames. No native IO.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import test from 'node:test';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const transform = value => stripTypeScriptTypes(value, { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
async function pure(path) { const module = new SourceTextModule(transform(await raw(path))); await module.link(() => { throw Error('Unexpected runtime import'); }); await module.evaluate(); return module.namespace; }
const pointer = await pure('./components/drawing-pointer.ts'), geometry = await pure('./components/drawing-geometry.ts'), properties = await pure('./components/drawing-properties.ts');
const source = await raw('./components/SharedDrawingEditor.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(value => value.type === 'ExportDefaultDeclaration').declaration.body.body;
const declaration = name => { const node = body.find(value => value.type === 'FunctionDeclaration' ? value.id.name === name : value.type === 'VariableDeclaration' && value.declarations.some(declaration => declaration.id.name === name)); assert.ok(node, name); return source.slice(node.start, node.end); };
const actual = ['cacheKey', 'gestureIdentity', 'point', 'target', 'editableIds', 'commit', 'begin', 'blur', 'resize'].map(declaration).join('\n');
const module = new SourceTextModule(transform(`
import { drawingFrame,drawingPointerSamples,settleDrawingEdit,drawingMoveDelta,drawingBounds,constrainedEnd,validNextNumber,t,blackboardImage } from 'deps';
export class Element { closest() { return {getAttribute:()=>this.id}; } }
export function fixture(props,svg,window) {
 let disposed=false,serial=0,flight,cancelGesture,refreshConstraint,toolValue='pen',selectedId='',draftValue,pendingValue=false;
 let selectedMany=[];const selectedIds=()=>selectedMany,setSelectedIds=value=>selectedMany=value;
 const publications=[],tool=()=>toolValue,setTool=value=>toolValue=value,selected=()=>selectedId,setSelected=value=>selectedId=value;
 const draft=()=>draftValue,setDraft=value=>{draftValue=value;if(value)publications.push(JSON.parse(JSON.stringify(value)));},pending=()=>pendingValue,setPending=value=>pendingValue=value;
 const edit=()=>undefined,textEditor=()=>undefined,finishing=()=>false,disabled=()=>pending()||props.busy||props.inputLocked;
 const drawings=()=>props.port.drawings(),identity=()=>cacheKey(),gestureSource=()=>gestureIdentity();
 const color=()=>'#ee4848',width=()=>4,highlightWidth=()=>18,mosaicReady=()=>true,blockSize=()=>8,fontSize=()=>20;
 const report=error=>props.onError(error.message),clamp=(v,low,high)=>Math.max(low,Math.min(high,v));
 const saveText=async()=>true,openText=()=>{},ensureSelection=async()=>{},beginStored=()=>{},beginErase=()=>{};
 const nextNumber=()=>'1',numberDiameter=()=>28,numberChanges=new Map(),numberPreference=0,setNextNumber=()=>{};
 const setViewport=()=>{},innerWidth=1200,innerHeight=800;
 ${actual}
 return {begin,draft,publications,tool:setTool,selected:setSelected,cancel:()=>cancelGesture?.(),blur,resize,
   dispose:()=>{disposed=true;cancelGesture?.();},hasGesture:()=>!!cancelGesture,shift:value=>refreshConstraint?.(value)};
}`));
await module.link(() => synthetic({ ...pointer, ...geometry, ...properties, t: value => value,blackboardImage:d=>d.rich?.kind==='extracted' })); await module.evaluate();
class Events {
  listeners = new Map();
  addEventListener(type, callback) { if (!this.listeners.has(type)) this.listeners.set(type, new Set()); this.listeners.get(type).add(callback); }
  removeEventListener(type, callback) { this.listeners.get(type)?.delete(callback); }
  emit(event) { for (const callback of [...(this.listeners.get(event.type) ?? [])]) callback(event); }
  count() { return [...this.listeners.values()].reduce((sum, values) => sum + values.size, 0); }
}
function frames() {
  let next = 0; const queued = new Map(), requested = [], canceled = [];
  const before = [globalThis.requestAnimationFrame, globalThis.cancelAnimationFrame];
  globalThis.requestAnimationFrame = callback => { const id = next++; requested.push(id); queued.set(id, callback); return id; };
  globalThis.cancelAnimationFrame = id => { canceled.push(id); queued.delete(id); };
  return { queued, requested, canceled, paint() { const callbacks = [...queued.values()]; queued.clear(); callbacks.forEach(callback => callback(0)); }, restore() { [globalThis.requestAnimationFrame, globalThis.cancelAnimationFrame] = before; } };
}
const event = (type, x, y, extra = {}) => ({ type, button: 0, buttons: type === 'pointerup' ? 0 : 1, pointerId: 7, clientX: 10 + x * 2, clientY: 20 + y * .5, pressure: .5, shiftKey: false, preventDefault() {}, stopPropagation() {}, ...extra });
const tick = () => new Promise(resolve => setImmediate(resolve));
function setup(values = []) {
  const window = new Events(), svg = new Events(), commands = [], errors = [];
  let layoutReads = 0;
  svg.box = { left: 10, top: 20, width: 2000, height: 500 };
  svg.getBoundingClientRect = () => { layoutReads++; return { ...svg.box }; }; svg.focus = () => {};
  svg.setPointerCapture = id => { svg.captured = id; }; svg.hasPointerCapture = id => svg.captured === id;
  svg.releasePointerCapture = id => { svg.captured = undefined; svg.emit(event('lostpointercapture', 0, 0, { pointerId: id })); };
  const props = { box: { x: 10, y: 20, width: 2000, height: 500 }, tools: ['pen','highlighter','line','arrow','rect','ellipse','mosaic'],busy:false,inputLocked:false,revision:5,source:'source',
    port: { frame:()=>({x:0,y:0,width:1000,height:1000}),sourceSize:()=>({width:1000,height:1000}),sourceIdentity:()=>props.source,draftKey:()=>props.source,revision:()=>props.revision,drawings:()=>values,canSelect:()=>true,allows:()=>props.allowed!==false },
    onCommand:async command=>{commands.push(plain(command));},onError:error=>errors.push(error) };
  return { ...module.namespace.fixture(props,svg,window), props, svg, window, commands, errors, reads:()=>layoutReads };
}

test('real pen handler batches visual publication and layout reads while retaining ordered underlying pen samples and exact release point', async () => {
  const clock = frames(); try {
    const f = setup(); f.begin(event('pointerdown',10,10)); const points = f.draft().points;
    assert.equal(f.svg.captured,7); assert.equal(f.publications.length,1); assert.equal(f.reads(),1);
    for(let i=1;i<=80;i++) {
      const a=event('pointermove',10+i-.25,10+i*.1,{pressure:.25}),b=event('pointermove',10+i,10+i*.1,{pressure:.75});
      f.window.emit({...b,getCoalescedEvents:()=>[a,b]});
    }
    assert.equal(f.publications.length,1); assert.equal(f.reads(),1); assert.equal(clock.requested.length,1);
    assert.equal(f.draft().points,points); assert.equal(points.length,161); assert.equal(f.commands.length,0);
    clock.paint(); assert.equal(f.publications.length,2); assert.equal(f.reads(),2); assert.equal(f.draft().points,points);
    f.window.emit(event('pointermove',95,31)); f.window.emit(event('pointerup',95.125,31.25,{pressure:0})); await tick();
    assert.equal(f.commands.length,1); assert.equal(f.commands[0].type,'add_drawing');
    assert.deepEqual(f.commands[0].drawing.points.at(-1),{x:95.125,y:31.25}); assert.equal(f.commands[0].drawing.points.length,163);
    assert.equal(clock.queued.size,0); assert.equal(f.window.count(),0); assert.equal(f.svg.captured,undefined); assert.equal(f.hasGesture(),false);
    points.push({x:999,y:999}); assert.equal(f.commands[0].drawing.points.length,163,'immutable committed snapshot');
  } finally { clock.restore(); }
});

test('unpainted terminal input still commits one gesture, including a tiny endpoint that the old half-pixel filter discarded', async () => {
  const clock=frames();try{
    const f=setup();f.begin(event('pointerdown',1,2));f.window.emit(event('pointermove',3,4));f.window.emit(event('pointerup',3.125,4.125));await tick();
    assert.equal(f.commands.length,1);assert.deepEqual(f.commands[0].drawing.points,[{x:1,y:2},{x:3,y:4},{x:3.125,y:4.125}]);assert.equal(clock.queued.size,0);clock.paint();assert.equal(f.draft(),undefined);
  }finally{clock.restore();}
});

test('cancel/lost capture/buttons/source/revision/display/tool/plugin/exit/dispose reject queued visual work and never write', async () => {
  const clock=frames();try{
    for(const reason of ['cancel','pointercancel','lost','buttons','source','revision','box','surface','tool','plugin','allow','lock','blur','resize','dispose']){
      const f=setup();f.begin(event('pointerdown',10,10));f.window.emit(event('pointermove',20,20));assert.equal(clock.queued.size,1,reason);
      if(reason==='cancel')f.cancel();if(reason==='pointercancel')f.window.emit(event('pointercancel',20,20));if(reason==='lost')f.svg.emit(event('lostpointercapture',20,20));
      if(reason==='buttons')f.window.emit(event('pointermove',30,30,{buttons:0}));if(reason==='source')f.props.source='successor';if(reason==='revision')f.props.revision++;
      if(reason==='box')f.props.box.x++;if(reason==='surface')f.svg.box.left++;if(reason==='tool')f.tool('line');if(reason==='plugin')f.props.tools=[];if(reason==='allow')f.props.allowed=false;
      if(reason==='lock')f.props.inputLocked=true;if(reason==='blur')f.blur();if(reason==='resize')f.resize();if(reason==='dispose')f.dispose();
      clock.paint();f.window.emit(event('pointerup',30,30));await tick();assert.equal(f.commands.length,0,reason);assert.equal(f.draft(),undefined,reason);assert.equal(f.window.count(),0,reason);assert.equal(clock.queued.size,0,reason);assert.equal(f.hasGesture(),false,reason);
    }
  }finally{clock.restore();}
});

test('other pointers cannot contribute points or terminate capture; pressure samples are passed through without fabrication',async()=>{
  const clock=frames();try{
    const f=setup();f.begin(event('pointerdown',5,5));f.begin(event('pointerdown',90,90,{pointerId:8}));f.window.emit(event('pointermove',90,90,{pointerId:8}));f.window.emit(event('pointerup',90,90,{pointerId:8}));assert.equal(f.draft().points.length,1);assert.equal(f.hasGesture(),true);assert.equal(f.svg.captured,7);
    const a=event('pointermove',7,8,{pressure:.17}),b=event('pointermove',9,10,{pressure:.81}),combined={...b,getCoalescedEvents:()=>[a,b]};
    assert.deepEqual(pointer.drawingPointerSamples(combined),[a,b]);f.window.emit(combined);f.window.emit(event('pointerup',9,10));await tick();assert.equal(f.commands[0].drawing.points.length,3);
    const empty={...b,getCoalescedEvents:()=>[]};assert.equal(pointer.drawingPointerSamples(empty)[0],empty);assert.equal(pointer.drawingPointerSamples(b)[0],b);
  }finally{clock.restore();}
});

test('shape and long saved-stroke Move compute one preview per frame and commit the latest endpoint once',async()=>{
  const clock=frames();try{
    const shape=setup();shape.tool('rect');shape.begin(event('pointerdown',10,20));for(let i=1;i<=40;i++)shape.window.emit(event('pointermove',30+i,40+i));assert.equal(shape.publications.length,1);clock.paint();assert.equal(shape.publications.length,2);
    shape.window.emit(event('pointerup',80,90));await tick();assert.deepEqual(shape.commands[0].drawing.points,[{x:10,y:20},{x:80,y:90}]);
    const old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:Array.from({length:4096},(_,i)=>({x:10+i/100,y:20+i/100}))};
    const moved=setup([old]);moved.tool('select');const target=new module.namespace.Element();target.id='old';moved.begin(event('pointerdown',10,20,{target}));for(let i=1;i<=40;i++)moved.window.emit(event('pointermove',20+i,30+i));assert.equal(moved.publications.length,0);clock.paint();assert.equal(moved.publications.length,1);
    moved.window.emit(event('pointerup',60,70));await tick();assert.equal(moved.commands.length,1);assert.equal(moved.commands[0].type,'update_drawing');assert.deepEqual(moved.commands[0].drawing.points[0],{x:60,y:70});assert.deepEqual(old.points[0],{x:10,y:20});
  }finally{clock.restore();}
});

test('zero-size final shape cancels a prior valid preview; native point bound rejects overflow and screenshot bound keeps its terminal endpoint',async()=>{
  const clock=frames();try{
    const shape=setup();shape.tool('line');shape.begin(event('pointerdown',20,30));shape.window.emit(event('pointermove',70,80));clock.paint();shape.window.emit(event('pointerup',20,30));await tick();assert.equal(shape.commands.length,0);assert.equal(shape.draft(),undefined);
    for(const strict of [false,true]){
      const f=setup();f.props.port.rejectPointOverflow=strict;f.begin(event('pointerdown',0,0));
      for(let i=1;i<=4095;i++)f.window.emit(event('pointermove',i/8,i/8));
      f.window.emit(event('pointerup',600,700));await tick();assert.equal(clock.queued.size,0);
      if(strict){assert.equal(f.commands.length,0);assert.deepEqual(f.errors,['笔迹最多 4096 个点']);}
      else {assert.equal(f.commands.length,1);assert.equal(f.commands[0].drawing.points.length,4096);assert.deepEqual(f.commands[0].drawing.points.at(-1),{x:600,y:700});}
    }
  }finally{clock.restore();}
});

test('long-stroke segments keep every original edge, bound the active SVG path and preserve completed path identity',()=>{
  const segment=pointer.penSegments(8),points=[];let previous=[];
  for(let i=0;i<100;i++){
    points.push({x:i,y:i%3});const result=segment(points);
    for(let k=0;k<previous.length-1;k++)assert.equal(result[k],previous[k],'finished path reused');
    for(const part of result)assert.ok(part.path.split(' ').length<=8);
    const edges=result.flatMap(part=>part.path.split(' ').slice(1).map(command=>Number(command.slice(1).split(',')[0])));assert.deepEqual(edges,Array.from({length:i},(_,j)=>j+1));previous=result;
  }
  const next=segment([{x:20,y:30},{x:40,y:50}]);assert.deepEqual(plain(next),[{path:'M20,30 L40,50'}]);assert.throws(()=>pointer.penSegments(1));
});

test('live port Drawing properties stay reactive under non-keyed Solid Show without replacing the renderer on each sample',()=>{
  let dispose,renders=0,paths=[];
  solid.createRoot(cleanup=>{dispose=cleanup;const [value,set]=solid.createSignal({id:'pen',kind:'pen',points:[{x:1,y:2}],color:'#ee4848',strokeWidth:4});
    const shown=solid.Show({get when(){return value();},children:current=>{renders++;const view=pointer.liveDrawing('pen',current);solid.createComputed(()=>paths.push(geometry.penPath(view.points)));return view;}});
    assert.equal(shown().id,'pen');set({...value(),points:[{x:1,y:2},{x:3,y:4}]});set({...value(),points:[{x:1,y:2},{x:3,y:4},{x:5,y:6}]});assert.equal(renders,1);assert.deepEqual(paths,['M1,2','M1,2 L3,4','M1,2 L3,4 L5,6']);
  });dispose();
});

// Compile the actual JSX against Solid's universal renderer with an in-memory tree.
// This catches stale Show arguments and remounts which handler-only tests cannot see.
const universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'));
await universal.link(()=>synthetic(solid));await universal.evaluate();
const renderer=universal.namespace.createRenderer({
  createElement:tag=>({tag,props:{},children:[]}),createTextNode:text=>({text,children:[]}),replaceText:(node,text)=>node.text=text,isTextNode:node=>'text'in node,
  setProperty:(node,name,value)=>node.props[name]=value,
  insertNode:(parent,node,anchor)=>{if(node.parent){const i=node.parent.children.indexOf(node);if(i>=0)node.parent.children.splice(i,1);}node.parent=parent;const index=anchor?parent.children.indexOf(anchor):-1;if(index<0)parent.children.push(node);else parent.children.splice(index,0,node);},
  removeNode:(parent,node)=>{parent.children.splice(parent.children.indexOf(node),1);node.parent=undefined;},getParentNode:node=>node.parent,getFirstChild:node=>node.children[0],getNextSibling:node=>node.parent?.children[node.parent.children.indexOf(node)+1],
});
async function jsx(source,dependencies){
  const compiled=transformSync(source,{filename:'drawing-fixture.tsx',babelrc:false,configFile:false,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'universal',moduleName:'test-renderer'}]]}).code;
  const result=new SourceTextModule(transform(compiled));await result.link(name=>synthetic(name==='test-renderer'?renderer:name==='solid-js'?solid:dependencies[name]??{}));await result.evaluate();return result.namespace;
}
const shapeJsx=await jsx(await raw('./components/DrawingLayer.tsx'),{
  './drawing-geometry':geometry,'./drawing-pointer':pointer,'./RichDrawingShape':{default:()=>undefined},'../bridge':{getMosaicPreview:()=>{throw Error('No native IO');}},'../i18n':{t:value=>value},
});
const itemJsx=await jsx(`import {linkedRepairPreview} from 'selection';import {createMemo,Show} from 'solid-js';import {liveDrawing} from 'pointer';
export function fixture(props,state){props={...props,port:{...props.port,drawings:state.drawings,stored:state.stored}};const draft=()=>state.draft(),edit=()=>undefined,tool=()=>state.tool(),selected=()=>state.selected(),storedDraft=()=>state.storedDraft(),drawings=state.drawings;
${['drawingIndex','storedIndex','activeDraftId'].map(declaration).join('\n')}
const previewDrawing=()=>draft(),groupDraft=()=>[],selectedIds=()=>[selected()];${declaration('previews')}
${declaration('DrawingItem')};return <DrawingItem id={props.id}/>;}`,{pointer,selection:{linkedRepairPreview:(drawing)=>drawing}});
const descendants=(node,tag)=>[...(node.tag===tag?[node]:[]),...node.children.flatMap(child=>descendants(child,tag))];

test('actual DrawingItem/Shape JSX updates live pen pixels and reuses completed SVG nodes; removal disposes the active tail',()=>{
  const [draft,setDraft]=solid.createSignal(),[drawings]=solid.createSignal([]),[stored]=solid.createSignal([]),[tool,setTool]=solid.createSignal('pen'),[selected,setSelected]=solid.createSignal('');
  const root=renderer.createElement('root'),points=[{x:0,y:0}],base={id:'active',kind:'pen',color:'#ee4848',strokeWidth:4,points};let renders=0;
  setDraft(base);
  const dispose=renderer.render(()=>itemJsx.fixture({id:'active',port:{canSelect:()=>true,render:drawing=>{renders++;return renderer.createComponent(shapeJsx.DrawingShape,{drawing});}}},{draft,drawings,stored,tool,selected,storedDraft:()=>undefined}),root);
  assert.equal(descendants(root,'circle').length,1);assert.equal(renders,1);
  for(let i=1;i<128;i++){points.push({x:i,y:i%2});}setDraft({...base});
  assert.equal(descendants(root,'path').length,1);assert.equal(descendants(root,'circle').length,0);const first=descendants(root,'path')[0],path=first.props.d;
  points.push({x:128,y:0},{x:129,y:1});setDraft({...base});assert.equal(descendants(root,'path').length,2);assert.equal(descendants(root,'path')[0],first);assert.equal(first.props.d,path);assert.ok(descendants(root,'path')[1].props.d.endsWith('L129,1'));assert.equal(renders,1);
  setSelected('active');setTool('select');assert.equal(descendants(root,'path').length,2);setDraft(undefined);assert.equal(descendants(root,'path').length,0);dispose();
});

test('actual stored JSX remains on the native raster port while selection, interactivity and exact preview bounds update',()=>{
  const [draft]=solid.createSignal(),[drawings]=solid.createSignal([]),[stored]=solid.createSignal([{id:'stored'}]),[tool,setTool]=solid.createSignal('pen'),[selected,setSelected]=solid.createSignal(''),[storedDraft,setStoredDraft]=solid.createSignal();
  const root=renderer.createElement('root'),renders=[];
  const dispose=renderer.render(()=>itemJsx.fixture({id:'stored',port:{canSelect:()=>true,render:()=>{throw Error('Stored preview must use native raster');},renderStored:(id,interactive,selected,preview)=>{renders.push({id,interactive,selected,preview});return renderer.createElement('native-raster');}}},{draft,drawings,stored,tool,selected,storedDraft}),root);
  assert.equal(descendants(root,'native-raster').length,1);setSelected('stored');setTool('select');const bounds={x:3,y:4,width:50,height:60};setStoredDraft({id:'stored',bounds});assert.deepEqual(renders.at(-1),{id:'stored',interactive:true,selected:true,preview:bounds});assert.equal(descendants(root,'native-raster').length,1);dispose();
});

test('moving one of many stored rasters never rerenders unrelated annotations',()=>{
  const values=Array.from({length:128},(_,i)=>({id:`stored-${i}`})),[stored]=solid.createSignal(values),[storedDraft,setStoredDraft]=solid.createSignal();
  const state={draft:()=>undefined,drawings:()=>[],stored,tool:()=> 'select',selected:()=> 'stored-0',storedDraft},counts=new Map(),root=renderer.createElement('root');
  const port={canSelect:()=>true,render:()=>{throw Error('Stored objects use native raster');},renderStored:id=>{counts.set(id,(counts.get(id)??0)+1);return renderer.createElement('native-raster');}};
  const dispose=renderer.render(()=>values.map(value=>itemJsx.fixture({id:value.id,port},state)),root);assert.equal(descendants(root,'native-raster').length,128);
  for(let i=0;i<20;i++)setStoredDraft({id:'stored-0',bounds:{x:i,y:i,width:20,height:20}});
  assert.equal(counts.get('stored-0'),21);for(const value of values.slice(1))assert.equal(counts.get(value.id),1);assert.equal(descendants(root,'native-raster').length,128);dispose();
});


test('actual rich-image JSX retains one loaded SVG image across drawing revisions, but clears rejected previews',async()=>{
  const layout=await pure('./drawing-layout-preview.ts'),jobs=[];
  const rich=await jsx(await raw('./components/RichDrawingShape.tsx'),{'../drawing-layout-preview':layout,'../drawing-layout-bridge':{getDrawingLayoutPreview:target=>new Promise((resolve,reject)=>jobs.push({target,resolve,reject}))}});
  const [revision,setRevision]=solid.createSignal(0),root=renderer.createElement('root');
  const drawing={id:'picture',kind:'rich',points:[{x:10,y:10},{x:100,y:60}],rich:{layoutId:'immutable',kind:'extracted',width:90,height:50}},asset={id:'source',kind:'image',width:500,height:400};
  const props={drawing,get context(){return{sceneId:'scene',background:asset,region:{id:'region',x:0,y:0,width:500,height:400,drawingRevision:revision()}};}};
  const dispose=renderer.render(()=>renderer.createComponent(rich.default,props),root);
  await tick();jobs[0].resolve({...jobs[0].target,dataUrl:'synthetic-pixels'});await tick();const node=descendants(root,'image')[0];assert.ok(node);
  for(let i=1;i<=8;i++){setRevision(i);await tick();assert.equal(descendants(root,'image')[0],node);jobs[i].resolve({...jobs[i].target,dataUrl:'synthetic-pixels'});await tick();assert.equal(descendants(root,'image')[0],node);}
  setRevision(9);await tick();jobs[9].reject(Error('native rejected current object'));await tick();assert.equal(descendants(root,'image').length,0);dispose();
});
