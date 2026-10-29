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
const pointer = await pure('./components/drawing-pointer.ts'), historyModule = await pure('./components/drawing-history.ts'), geometry = await pure('./components/drawing-geometry.ts'), properties = await pure('./components/drawing-properties.ts'), highlightSource = await pure('./highlight-source.ts'), diagnostics = await pure('./drawing-diagnostics.ts'), mouseInput = await pure('./components/drawing-input.ts');
const selectionM=new SourceTextModule(transform(await raw('./components/drawing-selection.ts')));await selectionM.link(()=>synthetic(geometry));await selectionM.evaluate();const selection=selectionM.namespace;
const source = await raw('./components/SharedDrawingEditor.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(value => value.type === 'ExportDefaultDeclaration').declaration.body.body;
const declaration = name => { const node = body.find(value => value.type === 'FunctionDeclaration' ? value.id.name === name : value.type === 'VariableDeclaration' && value.declarations.some(declaration => declaration.id.name === name)); assert.ok(node, name); return source.slice(node.start, node.end); };
const queueSourceEffect=body.find(node=>node.type==='ExpressionStatement'&&node.expression.type==='CallExpression'&&node.expression.callee.name==='createEffect'&&node.expression.arguments[0]?.type==='CallExpression'&&node.expression.arguments[0].arguments[0]?.name==='queueSource').expression.arguments[0].arguments[1];
const gestureSourceEffect=body.find(node=>node.type==='ExpressionStatement'&&node.expression.type==='CallExpression'&&node.expression.callee.name==='createEffect'&&node.expression.arguments[0]?.type==='CallExpression'&&node.expression.arguments[0].arguments[0]?.name==='gestureSource');
assert.ok(gestureSourceEffect,'actual production gestureSource effect');
const actual = ['drawingInput', 'pointerBegin', 'mouseBegin', 'diagnoseInput', 'cacheKey', 'strokeSource', 'continuousStrokes', 'gestureIdentity', 'point', 'target', 'editableIds', 'cachedStrokes', 'restoreStrokeDraft', 'newStrokeQueue', 'strokeCacheKey', 'releaseFailedStrokes', 'storeEdit', 'cancelEdit', 'commit', 'saveText', 'loadLatest', 'confirmText', 'begin', 'beginStored', 'selectionGesture', 'beginMarquee', 'beginGroup', 'beginDuplicate', 'readHistory', 'historyAcknowledged', 'createHistoryLane', 'drainHistory', 'history', 'historyAvailable', 'choose', 'closeEditor', 'removeAction', 'remove', 'keyboard', 'blur', 'resize'].map(declaration).join('\n');
const module = new SourceTextModule(transform(`
import { DrawingMouseInput,recordDrawingInput,DrawingInputPhase,untrack,DrawingHistoryLane,projectDrawingHistory,DrawingAddQueue,drawingFrame,drawingPointerSamples,settleDrawingEdit,drawingMoveDelta,drawingBounds,constrainedEnd,validNextNumber,t,blackboardImage,propertyDraft,drawingDrafts,strokeKinds,selectionBox,marqueeDrawings,movedDrawings,selectionRemovals,batch,createSignal,createMemo,createEffect,on,nativeSelectOwnsEscape } from 'deps';
export class Element { closest() { return {getAttribute:()=>this.id}; } }
export function fixture(props,svg,window) {
 let disposed=false,serial=0,flight,cancelGesture,refreshConstraint,toolValue='pen',selectedId='',draftValue,pendingValue=false,strokeQueue,editCacheKey,editValue,finishingValue=false;
 let historyLane,historyPreparation,historyIntents=[],preparingHistory,historyView,historyBusy=false;const historyPreview=()=>historyView,setHistoryPreview=value=>{historyView=value;props.onHistoryPreview?.(value);},historyPending=()=>historyBusy,setHistoryPending=value=>historyBusy=value;
 let queued=[],recovered=[];const queuedDrawings=()=>queued,setQueuedDrawings=value=>queued=value,setRecoveredDrawings=value=>recovered=value,[strokeVersion,setStrokeVersion]=props.reactive?createSignal(0):[()=>0,()=>{}]; let eraseSource;
 const propertyMenu=()=>Boolean(props.propertyMenu),setPropertyMenu=value=>props.propertyMenu=value,setCopyModifier=()=>{}; let marqueeValue,groupDraftValue;const setMarquee=value=>marqueeValue=value,setGroupDraft=value=>groupDraftValue=value;let selectedMany=[];const selectedIds=()=>selectedMany,setSelectedIds=value=>selectedMany=value;
 const publications=[],tool=()=>toolValue,setTool=value=>toolValue=value,selected=()=>selectedId,setSelected=value=>selectedId=value;
 const draft=()=>draftValue,setDraft=value=>{draftValue=value;if(value)publications.push(JSON.parse(JSON.stringify(value)));},pending=()=>pendingValue,setPending=value=>pendingValue=value;
 const edit=()=>editValue,setEdit=value=>editValue=value,textEditor=()=>undefined,finishing=()=>finishingValue,setFinishing=value=>finishingValue=value,historyDisabled=()=>pending()||finishing()||props.busy||props.inputLocked,disabled=()=>historyDisabled()||historyPending();
 const drawings=()=>props.port.drawings(),identity=()=>cacheKey();
 const trash={begin(){},move(){},cancel(){}},trashOver=()=>props.trashOver??false;
 const color=()=>'#ee4848',width=()=>4,highlightWidth=()=>18,mosaicReady=()=>true,blockSize=()=>8,fontSize=()=>20,textStyle=()=>props.textStyleValue??{};
 const report=error=>props.onError(error.message),clamp=(v,low,high)=>Math.max(low,Math.min(high,v));
 let openedText;const openText=value=>openedText=value,ensureSelection=async()=>{},beginErase=()=>{}; let storedDraftValue;const setStoredDraft=value=>storedDraftValue=value,setStoredCopy=()=>{};
 const nextNumber=()=>'1',numberDiameter=()=>28,numberChanges=new Map(),numberPreference=0,setNextNumber=()=>{};
 const setWideBrush=()=>{},mosaicFailed=()=>false,setMosaicRetry=()=>{},highlightFailed=()=>false,setHighlightRetry=()=>{};
 const setViewport=()=>{},innerWidth=1200,innerHeight=800;
 ${actual}
 historyLane=props.port.history&&!props.port.stored?createHistoryLane():undefined;
 strokeQueue=continuousStrokes()&&!historyLane?newStrokeQueue():undefined;
 const gestureSource=props.reactive?createMemo(gestureIdentity):()=>gestureIdentity();
 if(props.reactive){ ${source.slice(gestureSourceEffect.start,gestureSourceEffect.end)} }
 return {begin,pointerBegin,mouseBegin,drawingInput,draft,selection:()=>selectedMany,marquee:()=>marqueeValue,groupDraft:()=>groupDraftValue,storedDraft:()=>storedDraftValue,publications,tool:setTool,selected:setSelected,cancel:()=>cancelGesture?.(),blur,resize,choose,closeEditor,removeAction,remove,keyboard,finishing,confirmText,loadLatest,saveText,history,historyPreview,drainHistory,historyPending,sourceChanged:${source.slice(queueSourceEffect.start,queueSourceEffect.end)},
   dispose:()=>{disposed=true;strokeQueue?.stop();historyLane?.stop();cancelGesture?.();drawingInput.reset();},hasGesture:()=>!!cancelGesture,shift:value=>refreshConstraint?.(value),queued:()=>queued,edit,queue:()=>strokeQueue,lane:()=>historyLane,opened:()=>openedText};
}`));
await module.link(() => synthetic({ ...mouseInput,...diagnostics,untrack:solid.untrack,...pointer,...historyModule, ...geometry, ...properties,...selection,batch:solid.batch,createSignal:solid.createSignal,createMemo:solid.createMemo,createEffect:solid.createEffect,on:solid.on,nativeSelectOwnsEscape:()=>false, t: value => value,blackboardImage:d=>d.rich?.kind==='extracted' })); await module.evaluate();
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
let fixtureSerial=0;
function setup(values = [], history, reactive = false, draftIdentity) {
  const window = new Events(), svg = new Events(), commands = [], errors = [];
  let layoutReads = 0;
  svg.box = { left: 10, top: 20, width: 2000, height: 500 };
  svg.getBoundingClientRect = () => { layoutReads++; return { ...svg.box }; }; svg.focus = () => {};
  svg.setPointerCapture = id => { svg.captured = id; }; svg.hasPointerCapture = id => svg.captured === id;
  svg.releasePointerCapture = id => { svg.captured = undefined; svg.emit(event('lostpointercapture', 0, 0, { pointerId: id })); };
  const props = { reactive, box: { x: 10, y: 20, width: 2000, height: 500 }, tools: ['pen','highlighter','mark','line','arrow','rect','ellipse','mosaic'],busy:false,inputLocked:false,revision:5,source:'source',
    port: { frame:()=>({x:0,y:0,width:1000,height:1000}),sourceSize:()=>({width:1000,height:1000}),sourceIdentity:()=>props.source,draftKey:()=>JSON.stringify(['pointer-scene-'+fixtureSerial,'region',props.source]),revision:()=>props.revision,drawings:()=>values,canSelect:()=>true,allows:()=>props.allowed!==false,propertyCommand:draft=>({type:draft.base?'update_drawing':'add_drawing',drawing:plain(draft.drawing),expectedRevision:draft.expectedRevision}) },
    onCommand:async command=>{commands.push(plain(command));if(command.type==='add_drawing'){const drawing=plain(command.drawing);if(history){history.undo.push({index:values.length,before:null,after:drawing});history.redo=[];}values.push(drawing);props.revision++;}else if(history&&['undo_drawing','redo_drawing'].includes(command.type)){const next=historyModule.projectDrawingHistory({source:props.source,revision:props.revision,drawings:values,...history},command.type==='redo_drawing');values.splice(0,values.length,...plain(next.drawings));history.undo=plain(next.undo);history.redo=plain(next.redo);props.revision++;}},onClose:()=>{},onError:error=>errors.push(error) };
  if(history){props.port.history=()=>history;props.port.historyAvailable=redo=>(redo?history.redo:history.undo).length>0;props.port.historyStep=redo=>(redo?history.redo:history.undo).at(-1);}
  const fixtureId=++fixtureSerial;props.port.draftKey=()=>draftIdentity??JSON.stringify(['pointer-scene-'+fixtureId,'region',props.source]);
  if(!reactive) return { ...module.namespace.fixture(props,svg,window), props, svg, window, commands, errors, reads:()=>layoutReads };
  let result;solid.createRoot(dispose=>{const [revision,setRevision]=solid.createSignal(props.revision);Object.defineProperty(props,'revision',{get:revision,set:setRevision});result={...module.namespace.fixture(props,svg,window),disposeReactive:dispose,props,svg,window,commands,errors,reads:()=>layoutReads};});return result;
}

// Capture-phase document observation and surface handlers follow the real DOM order.
function mouseSurface(f) {
  const document=new Events(),detach=f.drawingInput.attach(document);
  const pointer=(type,x,y,extra={})=>event(type,x,y,{pointerType:'mouse',pointerId:41,...extra});
  const mouse=(type,x,y,extra={})=>event(type,x,y,{pointerType:undefined,...extra});
  const hover=()=>f.drawingInput.observe(pointer('pointermove',1,2,{buttons:0}));
  return {document,detach,pointer,mouse,hover,
    down(x,y,extra={}) { const e=pointer('pointerdown',x,y,extra);document.emit(e);f.pointerBegin(e); },
    missing(x,y,extra={}) { f.mouseBegin(mouse('mousedown',x,y,extra)); },
    up(x,y,extra={}) { const e=mouse('mouseup',x,y,extra);document.emit(e);f.window.emit(e); },
  };
}

test('independent ports accept opaque draft identities through draw, Undo and immediate drawing',async()=>{
 const clock=frames(),values=[],f=setup(values,{undo:[],redo:[]},true,`opaque-draft-${++fixtureSerial}:not-json`),input=mouseSurface(f);
 try{
  input.down(10,20);f.window.emit(input.pointer('pointerup',30,40));await f.drainHistory();
  assert.equal(values.length,1);void f.history(false);
  input.down(40,50);f.window.emit(input.pointer('pointerup',60,70));await f.drainHistory();
  assert.deepEqual(f.commands.map(value=>value.type),['add_drawing','undo_drawing','add_drawing']);
  assert.deepEqual(values.map(value=>value.points),[[{x:40,y:50},{x:60,y:70}]]);
  assert.deepEqual(f.errors,[]);
 }finally{input.detach();f.dispose();f.disposeReactive();clock.restore();}
});

test('physical toolbar mouseup with stale buttons permits missing PointerDown immediately after real Solid Undo',async()=>{
 const clock=frames(),values=[],f=setup(values,{undo:[],redo:[]},true),input=mouseSurface(f);
 try{
  input.down(10,20);f.window.emit(input.pointer('pointerup',30,40));input.document.emit(input.pointer('pointerup',30,40));await f.drainHistory();
  assert.equal(values.length,1);const toolbar=input.pointer('pointerdown',0,0);input.document.emit(toolbar);
  input.document.emit(input.mouse('mouseup',0,0,{button:0,buttons:1}));
  void f.history(false);input.missing(40,50);assert.equal(f.hasGesture(),true,'new mouse event enters the unchanged drawing admission before Undo finishes');
  f.window.emit(input.mouse('mousemove',60,70));input.up(80,90,{buttons:1});await f.drainHistory();
  assert.deepEqual(f.commands.map(value=>value.type),['add_drawing','undo_drawing','add_drawing']);
  assert.deepEqual(values.map(value=>value.points),[[{x:40,y:50},{x:60,y:70},{x:80,y:90}]]);assert.deepEqual(f.errors,[]);
  assert.equal(f.svg.captured,undefined,'fallback does not acquire a nonexistent native pointer capture');assert.equal(f.window.count(),0);
 }finally{input.detach();f.dispose();f.disposeReactive();clock.restore();}
});

test('normal pointer admission and rejection both consume the matching compatibility mousedown without retry',async()=>{
 const clock=frames();try{
  for(const rejected of [false,true]){
   const f=setup(),input=mouseSurface(f);try{
    f.props.busy=rejected;input.down(10,20);f.props.busy=false;input.missing(80,90);
    f.window.emit(input.mouse('mousemove',500,600));f.window.emit(input.pointer('pointermove',30,40));
    input.document.emit(input.pointer('pointerup',50,60));f.window.emit(input.pointer('pointerup',50,60));input.up(500,600);await tick();
    assert.equal(f.commands.length,rejected?0:1);assert.equal(f.window.count(),0);
    if(!rejected)assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);
    input.hover();input.missing(70,80);input.up(90,100);await tick();assert.equal(f.commands.length,rejected?1:2,'next physical press remains usable');
   }finally{input.detach();f.dispose();}
  }
 }finally{clock.restore();}
});

test('missing PointerDown owns only its mouse stream and never duplicates pointer samples capture or completion',async()=>{
 const clock=frames(),f=setup(),input=mouseSurface(f);let captures=0,releases=0;
 f.svg.setPointerCapture=()=>{captures++;};f.svg.releasePointerCapture=()=>{releases++;};
 try{
  input.hover();input.missing(10,20);f.window.emit(input.pointer('pointermove',700,800));
  f.window.emit(input.mouse('mousemove',30,40));input.down(800,900);assert.equal(f.hasGesture(),true,'late pointerdown cannot start a second gesture');
  f.window.emit(input.pointer('pointerup',800,900));assert.equal(f.commands.length,0,'pointerup cannot finish the mouse-owned stream');
  input.up(50,60,{buttons:1});f.window.emit(input.pointer('pointerup',50,60));input.up(50,60);await tick();
  assert.equal(f.commands.length,1);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);
  assert.equal(captures,0);assert.equal(releases,0);assert.equal(f.window.count(),0);assert.equal(f.svg.count(),0);
 }finally{input.detach();f.dispose();clock.restore();}
});

test('missing PointerDown retains real source blur resize permission and disposal cancellation boundaries',async()=>{
 const clock=frames();try{
  for(const reason of ['source','revision','blur','resize','surface','lock','permission','lost','pointercancel','dispose','foreign-touch']){
   const f=setup([],undefined,true),input=mouseSurface(f);try{
    input.hover();input.missing(10,20);f.window.emit(input.mouse('mousemove',30,40));assert.equal(f.hasGesture(),true,reason);
    if(reason==='source'){f.props.source='successor';f.props.revision++;}
    if(reason==='revision')f.props.revision++;
    if(reason==='blur')f.blur();if(reason==='resize')f.resize();if(reason==='surface')f.svg.box.left++;
    if(reason==='lock')f.props.inputLocked=true;if(reason==='permission')f.props.allowed=false;
    if(reason==='lost')f.svg.emit(input.pointer('lostpointercapture',30,40));
    if(reason==='pointercancel')f.window.emit(input.pointer('pointercancel',30,40));if(reason==='dispose')f.dispose();
    if(reason==='foreign-touch')input.document.emit(input.pointer('pointerdown',30,40,{pointerType:'touch',pointerId:82}));
    clock.paint();input.up(50,60);await tick();assert.equal(f.commands.length,0,reason);assert.equal(f.hasGesture(),false,reason);assert.equal(f.window.count(),0,reason);
   }finally{input.detach();f.dispose();f.disposeReactive();}
  }
 }finally{clock.restore();}
});

test('pen and touch release without compatibility mouseup cannot poison the next real mouse press',async()=>{
 const clock=frames();try{
  for(const pointerType of ['touch','pen']){
   const f=setup(),input=mouseSurface(f);try{
    input.down(10,20,{pointerType,pointerId:82});const up=input.pointer('pointerup',30,40,{pointerType,pointerId:82});input.document.emit(up);f.window.emit(up);await tick();
    assert.equal(f.commands.length,1);input.missing(50,60);assert.equal(f.hasGesture(),false,'touch/pen compatibility mouse event stays excluded');
    input.hover();input.missing(50,60);input.up(70,80);await tick();assert.equal(f.commands.length,2);assert.deepEqual(f.errors,[]);
   }finally{input.detach();f.dispose();}
  }
 }finally{clock.restore();}
});

test('missing PointerDown rejects touch compatibility right chord resize handle and non-vector tools',async()=>{
 const clock=frames();try{
  for(const reason of ['touch','right','chord','secondary-up','resize-handle','select','eraser','mosaic','text','number']){
   const f=setup(),input=mouseSurface(f);try{
    input.hover();const extra={};
    if(reason==='touch')extra.sourceCapabilities={firesTouchEvents:true};if(reason==='right'){extra.button=2;extra.buttons=2;}if(reason==='chord')extra.buttons=3;
    if(reason==='secondary-up'){f.props.busy=true;input.down(10,20);f.props.busy=false;input.document.emit(input.mouse('mouseup',0,0,{button:2,buttons:1}));}
    if(reason==='resize-handle'){const target=new module.namespace.Element();target.closest=selector=>selector==='.drawing-resize-handle'?{}:null;extra.target=target;}
    if(['select','eraser','mosaic','text','number'].includes(reason))f.tool(reason);
    input.missing(10,20,extra);input.up(30,40);await tick();assert.equal(f.commands.length,0,reason);assert.equal(f.hasGesture(),false,reason);
   }finally{input.detach();f.dispose();}
  }
 }finally{clock.restore();}
});

test('missing PointerDown uses the actual vector constraint and blackboard drawing paths without changing tools',async()=>{
 const clock=frames();try{
  for(const blackboard of [false,true])for(const kind of ['pen','line','highlighter','arrow','rect','ellipse','mark']){
   const f=setup(),input=mouseSurface(f);try{
    f.props.blackboard=blackboard;f.tool(kind);input.hover();input.missing(10,20,{shiftKey:kind==='highlighter'||kind==='line'});
    f.window.emit(input.mouse('mousemove',80,35,{shiftKey:kind==='highlighter'||kind==='line'}));input.up(100,40,{shiftKey:kind==='highlighter'||kind==='line'});await tick();
    assert.equal(f.commands.length,1,`${blackboard}:${kind}`);assert.equal(f.commands[0].drawing.kind,kind);
    if(kind==='line'||kind==='highlighter')assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:100,y:20}]);
    assert.deepEqual(f.errors,[]);assert.equal(f.window.count(),0);
   }finally{input.detach();f.dispose();}
  }
 }finally{clock.restore();}
});

test('new inline text copies the selected text effects without adding styles to a plain legacy text',()=>{
  const styled=setup();styled.tool('text');styled.props.textStyleValue={bold:true,italic:true,underline:true};
  styled.begin(event('pointerdown',10,20));assert.deepEqual(plain(styled.opened().textStyle),{bold:true,italic:true,underline:true});
  styled.props.textStyleValue.bold=false;assert.equal(styled.opened().textStyle.bold,true);
  const legacy=setup();legacy.tool('text');legacy.begin(event('pointerdown',10,20));assert.equal(legacy.opened().textStyle,undefined);
});

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

test('real highlighter Shift gesture snaps to horizontal, vertical and all diagonal directions without changing its ink kind or width',async()=>{
  const clock=frames();try{
    for(const [dx,dy,x,y] of [[120,20,620,500],[-120,20,380,500],[20,120,500,620],[20,-120,500,380],[90,70,580,580],[-90,70,420,580],[90,-70,580,420],[-90,-70,420,420]]){
      const f=setup();f.tool('highlighter');f.begin(event('pointerdown',500,500,{shiftKey:true}));
      f.window.emit(event('pointermove',500+dx,500+dy,{shiftKey:true}));clock.paint();
      assert.deepEqual(f.draft().points,[{x:500,y:500},{x,y}]);
      f.window.emit(event('pointerup',500+dx,500+dy,{shiftKey:true}));await tick();
      const drawing=f.commands[0].drawing;assert.equal(drawing.kind,'highlighter');assert.equal(drawing.strokeWidth,18);assert.deepEqual(drawing.points,[{x:500,y:500},{x,y}]);
      assert.equal(f.window.count(),0);assert.equal(clock.queued.size,0);
    }
    const f=setup();f.tool('highlighter');f.begin(event('pointerdown',950,960,{shiftKey:true}));
    f.window.emit(event('pointerup',1000,1000,{shiftKey:true}));await tick();assert.deepEqual(f.commands[0].drawing.points,[{x:950,y:960},{x:990,y:1000}]);
  }finally{clock.restore();}
});

test('actual Mark gesture constrains and releases square shape, keeps one fixed border and rejects zero-area/source changes',async()=>{
  const clock=frames();try{
    const f=setup();f.tool('mark');f.begin(event('pointerdown',10,20));
    f.window.emit(event('pointermove',100,50));clock.paint();assert.deepEqual(plain(f.draft().points),[{x:10,y:20},{x:100,y:50}]);
    assert.equal(f.draft().strokeWidth,1);f.shift(true);clock.paint();assert.deepEqual(plain(f.draft().points),[{x:10,y:20},{x:100,y:110}]);
    f.shift(false);clock.paint();f.window.emit(event('pointerup',105,55));await tick();assert.equal(f.commands.length,1);
    assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:105,y:55}]);assert.equal(f.commands[0].drawing.kind,'mark');
    for(const reason of ['zero-width','zero-height','source','revision']){
      const g=setup();g.tool('mark');g.begin(event('pointerdown',20,20));
      if(reason==='source')g.props.source='successor';if(reason==='revision')g.props.revision++;
      g.window.emit(event('pointerup',reason==='zero-width'?20:80,reason==='zero-height'?20:70));await tick();assert.equal(g.commands.length,0,reason);
    }
  }finally{clock.restore();}
});

test('highlighter modifier changes repaint while stationary and restore all sampled freehand ink without duplicating points',async()=>{
  const clock=frames();try{
    const f=setup();f.tool('highlighter');f.begin(event('pointerdown',10,20));const sampled=f.draft().points;
    for(let i=1;i<=160;i++)f.window.emit(event('pointermove',10+i,20+i%4));clock.paint();const freehand=plain(sampled);
    f.shift(true);assert.equal(clock.queued.size,1);clock.paint();assert.deepEqual(f.draft().points,[{x:10,y:20},{x:170,y:20}]);
    assert.notEqual(f.draft().points,sampled);assert.deepEqual(sampled,freehand);
    f.shift(false);clock.paint();assert.equal(f.draft().points,sampled);assert.deepEqual(f.draft().points,freehand);
    f.shift(true);f.window.emit(event('pointermove',180,42,{shiftKey:true}));clock.paint();
    assert.deepEqual(f.draft().points,[{x:10,y:20},{x:180,y:20}],'queued key constraint must not overwrite newer pointer endpoint');
    f.shift(false);f.window.emit(event('pointerup',181,43));await tick();
    assert.deepEqual(f.commands[0].drawing.points,[...freehand,{x:180,y:42},{x:181,y:43}]);
    sampled.push({x:999,y:999});assert.equal(f.commands[0].drawing.points.length,163);
  }finally{clock.restore();}
});

test('highlighter uses dispatched pointer modifiers for coalesced input and the exact unpainted release endpoint',async()=>{
  const clock=frames();try{
    const f=setup();f.tool('highlighter');f.begin(event('pointerdown',20,30));
    const a=event('pointermove',40,44),b=event('pointermove',80,66);
    f.window.emit({...b,shiftKey:true,getCoalescedEvents:()=>[a,b]});clock.paint();
    assert.deepEqual(f.draft().points,[{x:20,y:30},{x:68,y:78}]);
    f.shift(false);f.window.emit(event('pointerup',100,77,{shiftKey:true}));await tick();
    assert.deepEqual(f.commands[0].drawing.points,[{x:20,y:30},{x:83.5,y:93.5}]);assert.equal(clock.queued.size,0);
    const free=setup();free.tool('highlighter');free.begin(event('pointerdown',20,30,{shiftKey:true}));
    free.window.emit(event('pointermove',40,44,{shiftKey:true}));free.window.emit(event('pointerup',60,50));await tick();
    assert.deepEqual(free.commands[0].drawing.points,[{x:20,y:30},{x:40,y:44},{x:60,y:50}]);
  }finally{clock.restore();}
});

test('constrained highlighter cancels stale or lost gestures and retains the existing strict point bound',async()=>{
  const clock=frames();try{
    for(const reason of ['source','revision','lost','cancel','surface','lock']){
      const f=setup();f.tool('highlighter');f.begin(event('pointerdown',10,20));f.window.emit(event('pointermove',100,70));f.shift(true);
      if(reason==='source')f.props.source='successor';if(reason==='revision')f.props.revision++;
      if(reason==='lost')f.svg.emit(event('lostpointercapture',100,70));if(reason==='cancel')f.cancel();
      if(reason==='surface')f.svg.box.left++;if(reason==='lock')f.props.inputLocked=true;
      clock.paint();f.window.emit(event('pointerup',100,70,{shiftKey:true}));await tick();assert.equal(f.commands.length,0,reason);assert.equal(f.draft(),undefined,reason);assert.equal(f.window.count(),0,reason);
    }
    const f=setup();f.tool('highlighter');f.props.port.rejectPointOverflow=true;f.begin(event('pointerdown',0,0,{shiftKey:true}));
    for(let i=1;i<=4096;i++)f.window.emit(event('pointermove',i/8,i/8,{shiftKey:true}));
    clock.paint();f.window.emit(event('pointerup',600,700,{shiftKey:true}));await tick();assert.equal(f.commands.length,0);assert.deepEqual(f.errors,['笔迹最多 4096 个点']);
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

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
test('real pen accepts the next strokes before slow persistence, preserves pending previews and rebases only exact own acknowledgements',async()=>{
  const clock=frames();try{
    const f=setup(),baseSave=f.props.onCommand,gates=[deferred(),deferred(),deferred()],sent=[];
    f.props.onCommand=async command=>{const index=sent.length;sent.push(plain(command));await gates[index].promise;await baseSave(command);};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    assert.equal(sent.length,1);assert.equal(f.queued().length,1);assert.equal(f.draft(),undefined);
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));
    assert.equal(f.queued().length,2);assert.equal(sent.length,1,'one native persistence flight');
    f.begin(event('pointerdown',70,80));f.window.emit(event('pointermove',90,100));
    gates[0].resolve();await tick();
    assert.equal(f.hasGesture(),true,'first own ack does not cancel third live gesture');
    clock.paint();assert.ok(f.draft());f.window.emit(event('pointerup',110,120));
    assert.equal(f.queued().length,2);assert.equal(sent.length,2);assert.equal(sent[1].expectedRevision,6);
    let drained=false;const drain=f.queue().drain().then(()=>{drained=true;});await tick();assert.equal(drained,false);
    gates[1].resolve();await tick();assert.equal(sent.length,3);assert.equal(sent[2].expectedRevision,7);assert.equal(f.queued().length,1);
    gates[2].resolve();await drain;assert.equal(drained,true);assert.equal(f.queued().length,0);assert.equal(f.props.revision,8);
    assert.deepEqual(f.props.port.drawings().map(value=>value.id),sent.map(value=>value.drawing.id));assert.deepEqual(f.errors,[]);
  }finally{clock.restore();}
});

test('a publish before its delayed reply permits the next live stroke but still drains the actual IPC and serializes later writes',async()=>{
  const clock=frames();try{
    const f=setup(),baseSave=f.props.onCommand,reply=deferred(),sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await baseSave(command);if(sent.length===1)await reply.promise;};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));assert.equal(f.queued().length,2);assert.equal(sent.length,1);
    let drained=false;const drain=f.queue().drain().then(()=>{drained=true;});await tick();assert.equal(drained,false);
    reply.resolve();await drain;assert.equal(sent.length,2);assert.deepEqual(sent.map(value=>value.expectedRevision),[5,6]);assert.equal(f.queued().length,0);
  }finally{clock.restore();}
});

test('an external revision or changed saved drawing stops the queue and retains every unacknowledged geometry draft',async()=>{
  const clock=frames();try{
    for(const reason of ['revision','content','source','permission']){
      const f=setup(),first=deferred(),baseSave=f.props.onCommand,sent=[];
      f.props.onCommand=async command=>{sent.push(plain(command));if(sent.length===1){await first.promise;await baseSave(command);if(reason==='revision')f.props.revision++;if(reason==='content')f.props.port.drawings()[0].color='#123456';}else await baseSave(command);};
      f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
      f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));
      if(reason==='source')f.props.source='successor';if(reason==='permission')f.props.allowed=false;
      first.resolve();await assert.rejects(()=>f.queue().drain());assert.equal(sent.length,1,reason);
      if(reason!=='source')assert.ok(f.edit(),reason);
      const drafts=properties.drawingDrafts.list().filter(entry=>JSON.parse(entry.key)[2]===f.props.port.draftKey());
      if(reason!=='source')assert.ok(drafts.length>=1,reason);
      for(const entry of drafts)assert.equal(properties.draftChanged(entry.draft),true,'new nontext geometry remains a saveable draft');
    }
  }finally{clock.restore();}
});

test('first persistence failure retains separate immutable strokes, stops the lane and prevents a false successful handoff',async()=>{
  const clock=frames();try{
    const f=setup(),first=deferred(),sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await first.promise;throw Error('disk unavailable');};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));
    first.resolve();await assert.rejects(()=>f.queue().drain(),/disk unavailable/);
    assert.equal(sent.length,1);assert.equal(f.queued().length,2);assert.ok(f.edit()?.error);
    const drafts=properties.drawingDrafts.list().filter(entry=>JSON.parse(entry.key)[2]===f.props.port.draftKey());
    assert.equal(drafts.length,2);assert.notEqual(drafts[0].key,drafts[1].key);assert.notEqual(drafts[0].draft.drawing.id,drafts[1].draft.drawing.id);
    for(const entry of drafts)assert.equal(properties.drawingPropertyCommand(entry.draft,{sceneId:'scene',regionId:'region',backgroundId:'source'}).type,'add_drawing');
    assert.equal(f.queue().capacity(),false);
  }finally{clock.restore();}
});

test('stopping an old source drains its actual first flight and never dispatches the following stroke into another editor',async()=>{
  const clock=frames();try{
    const f=setup(),first=deferred(),baseSave=f.props.onCommand,sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await first.promise;await baseSave(command);};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));f.dispose();
    let finished=false;const drain=f.queue().drain().finally(()=>{finished=true;});await tick();assert.equal(finished,false);
    first.resolve();await assert.rejects(()=>drain);assert.equal(sent.length,1);
  }finally{clock.restore();}
});

test('pending new ink never admits a right-button eraser as another queueable pen gesture',async()=>{
  const clock=frames();try{
    const f=setup(),first=deferred(),baseSave=f.props.onCommand;
    f.props.onCommand=async command=>{await first.promise;await baseSave(command);};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    f.begin(event('pointerdown',40,50,{button:2,buttons:2}));
    assert.equal(f.hasGesture(),false);assert.equal(f.queued().length,1);assert.equal(f.draft(),undefined);
    first.resolve();await f.queue().drain();assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'add_drawing');
  }finally{clock.restore();}
});

test('close and tool handoffs lock new gestures only while draining, and a failed save restores usable controls without closing',async()=>{
  const clock=frames();try{
    for(const action of ['close','choose','failed-close']){
      const f=setup(),first=deferred(),baseSave=f.props.onCommand;let closes=0;
      f.props.onClose=()=>{closes++;};f.props.onCommand=async command=>{await first.promise;if(action==='failed-close')throw Error('disk unavailable');await baseSave(command);};
      f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
      const handoff=action==='choose'?f.choose('line'):f.closeEditor();assert.equal(f.finishing(),true);
      f.begin(event('pointerdown',40,50));assert.equal(f.hasGesture(),false);assert.equal(f.draft(),undefined);
      first.resolve();await handoff;assert.equal(f.finishing(),false);assert.equal(closes,action==='close'?1:0);
      if(action==='choose')assert.equal(f.errors.length,0);
      if(action==='failed-close'){assert.ok(f.edit()?.error);assert.ok(f.queued().length);}
    }
  }finally{clock.restore();}
});

test('failed multi-stroke drafts recover individually through real retry controls and require explicit latest consent after a revision change',async()=>{
  const clock=frames();try{
    const f=setup(),first=deferred(),baseSave=f.props.onCommand;
    f.props.onCommand=async()=>{await first.promise;throw Error('disk unavailable');};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));first.resolve();
    await assert.rejects(()=>f.queue().drain());const ids=f.queued().map(value=>value.id);
    f.props.onCommand=async command=>{if(command.expectedRevision!==f.props.revision)throw Error('revision conflict');await baseSave(command);};
    f.confirmText();await tick();assert.equal(f.props.revision,6);assert.equal(f.edit()?.drawing.id,ids[1]);
    f.confirmText();await tick();assert.equal(f.props.revision,6);assert.ok(f.edit()?.error,'stale second retry remains unsaved');
    f.loadLatest();assert.equal(f.edit()?.expectedRevision,6);f.confirmText();await tick();
    assert.equal(f.props.revision,7);assert.equal(f.edit(),undefined);assert.deepEqual(f.props.port.drawings().map(value=>value.id),ids);
    assert.equal(properties.drawingDrafts.list().filter(entry=>JSON.parse(entry.key)[2]===f.props.port.draftKey()).length,0);
  }finally{clock.restore();}
});

test('a full global draft cache rejects new ink before its first sample and canceled gestures release their reserved slot and bytes',async()=>{
  const clock=frames(),inserted=[];try{
    const cache=properties.drawingDrafts,drawing={id:'quota',kind:'pen',color:'#ee4848',strokeWidth:4,points:[{x:10,y:20}]};
    while(cache.list().length<32){const key=JSON.stringify(['quota-scene',inserted.length]);cache.set(key,properties.propertyDraft(drawing,0,'style',false));inserted.push(key);}
    const full=setup();full.begin(event('pointerdown',10,20));assert.equal(full.draft(),undefined);assert.equal(full.hasGesture(),false);assert.equal(full.queued().length,0);assert.ok(full.errors.length);
    const historyFull=setup([],addedHistory([]));historyFull.begin(event('pointerdown',10,20));assert.equal(historyFull.draft(),undefined);assert.equal(historyFull.hasGesture(),false);assert.ok(historyFull.errors.length);
    for(const key of inserted)cache.delete(key);inserted.length=0;
    const canceled=setup();for(let i=0;i<16;i++){canceled.begin(event('pointerdown',10,20));assert.equal(canceled.hasGesture(),true);canceled.cancel();assert.equal(canceled.hasGesture(),false);}
    canceled.begin(event('pointerdown',10,20));canceled.window.emit(event('pointerup',30,40));await canceled.queue().drain();assert.equal(canceled.commands.length,1);
    const bounded=new properties.DrawingDraftCache(3,1500),release=bounded.reserve('reserved',1200);
    assert.throws(()=>bounded.reserve('second',500),/过多/);assert.throws(()=>bounded.set('large',properties.propertyDraft({...drawing,points:Array.from({length:100},(_,i)=>({x:i,y:i}))},0,'style',false)),/过多/);
    bounded.set('reserved',properties.propertyDraft(drawing,0,'style',false));release();
    const second=bounded.reserve('second',500);second();assert.equal(bounded.list().length,1);
  }finally{for(const key of inserted)properties.drawingDrafts.delete(key);clock.restore();}
});

const historyDrawing=id=>({id,kind:'pen',color:'#ee4848',strokeWidth:4,points:[{x:10,y:20},{x:30,y:40}]});
const addedHistory=values=>({undo:values.map((after,index)=>({index,before:null,after:plain(after)})),redo:[]});
test('actual Undo and Redo immediately project consecutive visible states while real CAS writes remain strictly serial',async()=>{
  const values=['first','second','third'].map(historyDrawing),f=setup(values,addedHistory(values)),baseSave=f.props.onCommand,gates=[deferred(),deferred(),deferred()],sent=[];
  f.props.onCommand=async command=>{sent.push(plain(command));await gates[sent.length-1].promise;assert.equal(command.expectedRevision,f.props.revision);await baseSave(command);};
  void f.history(false);assert.deepEqual(f.historyPreview().map(value=>value.id),['first','second']);
  void f.history(false);assert.deepEqual(f.historyPreview().map(value=>value.id),['first']);
  void f.history(true);assert.deepEqual(f.historyPreview().map(value=>value.id),['first','second']);
  await tick();assert.equal(sent.length,1);assert.equal(values.length,3);
  let drained=false;const drain=f.drainHistory().then(()=>{drained=true;});await tick();assert.equal(drained,false);
  gates[0].resolve();await tick();assert.equal(sent.length,2);assert.deepEqual(f.historyPreview().map(value=>value.id),['first','second']);
  gates[1].resolve();await tick();assert.equal(sent.length,3);gates[2].resolve();await drain;
  assert.deepEqual(values.map(value=>value.id),['first','second']);assert.deepEqual(sent.map(value=>value.expectedRevision),[5,6,7]);assert.equal(f.historyPreview(),undefined);assert.equal(f.historyPending(),false);
});

test('document-only rich image history projects immediately without enabling a continuous add queue',async()=>{
  const values=[{...historyDrawing('rich'),kind:'rich',rich:{layoutId:'saved-layout',kind:'translation',width:20,height:20}}],f=setup(values,addedHistory(values)),saved=deferred(),baseSave=f.props.onCommand;
  f.props.documentOnly=true;f.sourceChanged();assert.equal(f.queue(),undefined);
  f.props.onCommand=async command=>{await saved.promise;await baseSave(command);};
  void f.history(false);assert.deepEqual(plain(f.historyPreview()),[]);assert.equal(values.length,1);
  saved.resolve();await f.drainHistory();assert.equal(values.length,0);assert.deepEqual(f.errors,[]);
});

test('Undo immediately hides the final not-yet-saved stroke, keeps its real save alive, then uses the acknowledged native history',async()=>{
  const clock=frames();try{
    const values=[],f=setup(values,addedHistory(values)),first=deferred(),undo=deferred(),baseSave=f.props.onCommand,sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await(command.type==='add_drawing'?first.promise:undo.promise);assert.equal(command.expectedRevision,f.props.revision);await baseSave(command);};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();
    assert.equal(f.queued().length,1);void f.history(false);assert.deepEqual(plain(f.historyPreview()),[]);assert.equal(sent.length,1);
    first.resolve();await tick();assert.equal(sent.length,2);assert.equal(sent[1].type,'undo_drawing');assert.equal(sent[1].expectedRevision,6);assert.equal(values.length,1);assert.deepEqual(plain(f.historyPreview()),[]);
    undo.resolve();await f.drainHistory();assert.equal(values.length,0);assert.equal(f.queued().length,0);assert.equal(f.historyPreview(),undefined);assert.deepEqual(f.errors,[]);
  }finally{clock.restore();}
});

test('Undo immediately accepts new pen gestures and serializes add, undo, redo and another add in their original order',async()=>{
  const clock=frames();try{
    const values=['first','second'].map(historyDrawing),f=setup(values,addedHistory(values)),baseSave=f.props.onCommand,gates=Array.from({length:5},deferred),sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await gates[sent.length-1].promise;assert.equal(command.expectedRevision,f.props.revision);await baseSave(command);};
    void f.history(false);await tick();assert.equal(sent.length,1);assert.deepEqual(f.historyPreview().map(value=>value.id),['first']);
    f.begin(event('pointerdown',10,20));assert.equal(f.hasGesture(),true);f.window.emit(event('pointerup',30,40));const third=f.queued()[0].id;
    assert.deepEqual(f.historyPreview().map(value=>value.id),['first',third]);assert.equal(values.length,2);assert.equal(sent.length,1);
    void f.history(false);assert.deepEqual(f.historyPreview().map(value=>value.id),['first']);
    void f.history(true);assert.deepEqual(f.historyPreview().map(value=>value.id),['first',third]);
    f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));const fourth=f.queued()[1].id;
    assert.deepEqual(f.historyPreview().map(value=>value.id),['first',third,fourth]);
    let closed=false;f.props.onClose=()=>closed=true;const close=f.closeEditor();assert.equal(f.finishing(),true);assert.equal(closed,false);
    for(let index=0;index<gates.length;index++){assert.equal(sent.length,index+1);gates[index].resolve();await tick();}
    await close;assert.equal(closed,true);assert.equal(f.historyPreview(),undefined);assert.deepEqual(f.errors,[]);
    assert.deepEqual(sent.map(value=>value.type),['undo_drawing','add_drawing','undo_drawing','redo_drawing','add_drawing']);assert.deepEqual(sent.map(value=>value.expectedRevision),[5,6,7,8,9]);
    assert.deepEqual(values.map(value=>value.id),['first',third,fourth]);assert.equal(properties.drawingDrafts.list().some(entry=>[third,fourth].includes(entry.draft.drawing.id)),false);
  }finally{clock.restore();}
});

test('a pending own Undo acknowledgement never cancels the next live stroke, including a publish before its IPC reply',async()=>{
  const clock=frames();try{
    const values=['first'].map(historyDrawing),f=setup(values,addedHistory(values)),baseSave=f.props.onCommand,reply=deferred(),sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await baseSave(command);if(sent.length===1)await reply.promise;};
    void f.history(false);f.begin(event('pointerdown',10,20));assert.equal(f.hasGesture(),true);await tick();assert.equal(f.props.revision,6);
    f.window.emit(event('pointermove',20,30));clock.paint();assert.equal(f.hasGesture(),true);f.window.emit(event('pointerup',30,40));
    const id=f.queued()[0].id;assert.equal(sent.length,1);assert.deepEqual(f.historyPreview().map(value=>value.id),[id]);
    reply.resolve();await f.drainHistory();assert.deepEqual(sent.map(value=>value.expectedRevision),[5,6]);assert.deepEqual(values.map(value=>value.id),[id]);assert.deepEqual(f.errors,[]);
  }finally{clock.restore();}
});

test('failed or invalidated Undo preserves each newly accepted stroke and never sends a late add',async()=>{
  const clock=frames();try{
    for(const reason of ['failure','source','revision','permission']){
      const values=['original'].map(historyDrawing),f=setup(values,addedHistory(values)),baseSave=f.props.onCommand,first=deferred(),sent=[];
      f.props.onCommand=async command=>{sent.push(plain(command));await first.promise;if(reason==='failure')throw Error('undo failed');await baseSave(command);if(reason==='revision')f.props.revision++;};
      void f.history(false);await tick();
      for(let index=0;index<2;index++){f.begin(event('pointerdown',10+index*30,20));f.window.emit(event('pointerup',30+index*30,40));}
      const ids=f.queued().map(value=>value.id);assert.equal(ids.length,2);const drain=f.drainHistory();
      if(reason==='source'){f.props.source='new-source';f.sourceChanged();}if(reason==='permission')f.props.allowed=false;
      first.resolve();await assert.rejects(drain);assert.equal(sent.length,1,reason);
      const retained=properties.drawingDrafts.list().filter(entry=>ids.includes(entry.draft.drawing.id));assert.equal(retained.length,2,reason);
      for(const entry of retained)properties.drawingDrafts.delete(entry.key,entry.draft);
    }
  }finally{clock.restore();}
});

test('new-add acknowledgements allow only a retained undo suffix and replay later intents from real bounded host history',async()=>{
  const clock=frames();try{
    for(const limit of [128,1]){
      const values=Array.from({length:limit===128?128:3},(_,index)=>historyDrawing('old-'+index)),history=addedHistory(values),f=setup(values,history),baseSave=f.props.onCommand;
      f.props.onCommand=async command=>{assert.equal(command.expectedRevision,f.props.revision);await baseSave(command);if(command.type==='add_drawing')history.undo=history.undo.slice(-limit);};
      f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));const first=f.queued()[0].id;void f.history(false);
      f.begin(event('pointerdown',40,50));f.window.emit(event('pointerup',60,70));const second=f.queued()[1].id;
      await f.drainHistory();assert.deepEqual(f.errors,[]);assert.equal(history.redo.length,0);assert.equal(history.undo.length,limit);
      assert.equal(values.some(value=>value.id===first),false);assert.equal(values.at(-1).id,second);
      assert.deepEqual(f.commands.map(value=>value.type),['add_drawing','undo_drawing','add_drawing']);assert.deepEqual(f.commands.map(value=>value.expectedRevision),[5,6,7]);
    }
  }finally{clock.restore();}
});

test('a failed unified lane retains its explicit recovery state after drains even when diagnostic text cannot fit the draft budget',async()=>{
  const clock=frames();try{
    const values=['original'].map(historyDrawing),f=setup(values,addedHistory(values)),baseSave=f.props.onCommand,first=deferred();
    f.props.onCommand=async()=>{await first.promise;throw Error('x'.repeat(4*1024*1024));};
    void f.history(false);f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));const id=f.queued()[0].id;
    first.resolve();await assert.rejects(f.drainHistory());assert.equal(f.edit().drawing.id,id);assert.equal(f.edit().error,undefined);assert.equal(f.lane().failed(),true);
    f.props.onCommand=baseSave;f.confirmText();await tick();assert.equal(f.lane().failed(),false);assert.equal(f.edit(),undefined);assert.equal(values.at(-1).id,id);
  }finally{clock.restore();}
});

test('history failures restore host pixels, drop later undo intents and keep unsaved add drafts rather than faking persistence',async()=>{
  for(const reason of ['failure','source','revision','permission']){
    const values=['first','second'].map(historyDrawing),f=setup(values,addedHistory(values)),first=deferred(),baseSave=f.props.onCommand,sent=[];
    f.props.onCommand=async command=>{sent.push(plain(command));await first.promise;if(reason==='failure')throw Error('disk unavailable');await baseSave(command);if(reason==='revision')f.props.revision++;};
    void f.history(false);void f.history(false);await tick();assert.deepEqual(plain(f.historyPreview()),[]);
    const drain=f.drainHistory();
    if(reason==='source'){f.props.source='successor';f.sourceChanged();}if(reason==='permission')f.props.allowed=false;
    first.resolve();await assert.rejects(()=>drain);assert.equal(sent.length,1,reason);assert.equal(f.historyPreview(),undefined,reason);assert.equal(f.historyPending(),false,reason);
    if(reason==='failure')assert.equal(values.length,2);
  }
  const clock=frames();try{
    const values=[],f=setup(values,addedHistory(values)),first=deferred();f.props.onCommand=async()=>{await first.promise;throw Error('add failed');};
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));await tick();void f.history(false);assert.deepEqual(plain(f.historyPreview()),[]);
    const unsaved=f.queued()[0].id;first.resolve();await assert.rejects(()=>f.drainHistory());assert.equal(f.historyPreview(),undefined);assert.equal(f.queued().length,0);assert.ok(properties.drawingDrafts.list().some(entry=>entry.draft.drawing.id===unsaved));assert.equal(f.edit().drawing.id,unsaved);
  }finally{clock.restore();}
});

test('history batch replay is reversible without changing IDs, provenance, text style or unrelated paths',()=>{
  const a=historyDrawing('a'),before={...historyDrawing('b'),kind:'text',text:'文字',fontSize:20,textStyle:{bold:true},points:[{x:10,y:20}]},after={...before,textStyle:{bold:true,underline:true}},step={batch:[{index:1,before,after}]};
  const state={source:'source',revision:5,drawings:[a,after],undo:[step],redo:[]},undone=historyModule.projectDrawingHistory(state,false),redone=historyModule.projectDrawingHistory(undone,true);
  assert.equal(undone.drawings[0],a);assert.deepEqual(plain(undone.drawings[1]),before);assert.deepEqual(plain(redone.drawings),state.drawings);assert.equal(state.drawings[1],after);
});

test('every concurrent history drain rejects the same failed flight and a later explicit undo can retry',async()=>{
  const values=['first','second'].map(historyDrawing),f=setup(values,addedHistory(values)),first=deferred(),baseSave=f.props.onCommand,failure=Error('history disk failure');let attempts=0;
  f.props.onCommand=async command=>{attempts++;if(attempts===1){await first.promise;throw failure;}await baseSave(command);};
  void f.history(false);void f.history(false);await tick();
  const waits=Promise.allSettled([f.drainHistory(),f.drainHistory()]);first.resolve();
  const results=await waits;assert.equal(results.length,2);
  for(const result of results){assert.equal(result.status,'rejected');assert.equal(result.reason,failure);}
  assert.equal(attempts,1);assert.deepEqual(values.map(value=>value.id),['first','second']);assert.equal(f.historyPreview(),undefined);
  void f.history(false);await f.drainHistory();assert.equal(attempts,2);assert.deepEqual(values.map(value=>value.id),['first']);
});

test('history comparisons retain shared-node fast paths while checking separate host history replies exactly',async()=>{
  const protectedDrawing=new Proxy(historyDrawing('shared'),{ownKeys(){throw Error('Shared drawing must not be traversed');},get(){throw Error('Shared drawing must not be read');}});
  let shared={source:'source',revision:5,drawings:[protectedDrawing],undo:[{index:0,before:null,after:protectedDrawing}],redo:[]};
  const lane=new historyModule.DrawingHistoryLane({read:()=>shared,active:()=>true,save:async(redo,revision)=>{assert.equal(revision,shared.revision);shared=historyModule.projectDrawingHistory(shared,redo);},changed:()=>{},completed:()=>{},failed:error=>{throw error;}});
  assert.equal(lane.enqueue(false),true);assert.equal(lane.enqueue(true),true);await lane.drain();assert.equal(shared.drawings[0],protectedDrawing);
  for(const corrupt of [false,true]){
    const values=['first','second'].map(historyDrawing);let state={source:'source',revision:5,drawings:values,...addedHistory(values)},sends=0,failures=0;
    const copyLane=new historyModule.DrawingHistoryLane({read:()=>state,active:()=>true,save:async(redo,revision)=>{
      assert.equal(revision,state.revision);sends++;
      state=structuredClone(historyModule.projectDrawingHistory(state,redo));
      state.drawings=state.drawings.map(value=>({points:value.points,...value,color:value.color}));
      if(corrupt)state.redo[0].after.points[0].x++;
    },changed:()=>{},completed:()=>{},failed:()=>failures++});
    assert.equal(copyLane.enqueue(false),true);assert.equal(copyLane.enqueue(false),true);
    if(corrupt)await assert.rejects(copyLane.drain(),/回执不一致/);else await copyLane.drain();
    assert.equal(sends,corrupt?1:2);assert.equal(failures,corrupt?1:0);
  }
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
const colors=await pure('./drawing-colors.ts');
const colorPickerJsx=await jsx(await raw('./components/DrawingColorPicker.tsx'),{
  '../drawing-colors':colors,'../i18n':{t:value=>value},'solid-js/web':{Portal:props=>props.children},
  'lucide-solid':{Check:()=>renderer.createElement('svg'),Palette:()=>renderer.createElement('svg'),X:()=>renderer.createElement('svg')},
});
let highlightLoader=()=>{throw Error('No native IO');};
const shapeJsx=await jsx(await raw('./components/DrawingLayer.tsx'),{
  '../drawing-colors':colors,
  './drawing-geometry':geometry,'./drawing-pointer':pointer,'./RichDrawingShape':{default:()=>undefined},'../highlight-source':highlightSource,'../bridge':{getMosaicPreview:()=>{throw Error('No native IO');},getHighlightPreview:(...args)=>highlightLoader(...args)},'../i18n':{t:value=>value},
});
const itemJsx=await jsx(`import {linkedRepairPreview} from 'selection';import {createMemo,Show} from 'solid-js';import {liveDrawing} from 'pointer';
export function fixture(props,state){props={...props,port:{...props.port,drawings:state.drawings,stored:state.stored}};const draft=()=>state.draft(),edit=()=>undefined,tool=()=>state.tool(),selected=()=>state.selected(),storedDraft=()=>state.storedDraft(),drawings=state.drawings;
const visibleDrawings=()=>state.historyPreview?.()??drawings();${['drawingIndex','storedIndex','activeDraftId'].map(declaration).join('\n')}
const previewDrawing=()=>draft(),groupDraft=()=>[],selectedIds=()=>[selected()],queuedDrawings=()=>state.queued?.()??[],recoveredDrawings=()=>state.recovered?.()??[];${declaration('previews')}
const copyModifier=()=>false;const pixelIndex=()=>state.pixelIndex?.()??new Map();
${declaration('DrawingItem')};return <DrawingItem id={props.id}/>;}`,{pointer,selection:{linkedRepairPreview:(drawing)=>drawing}});
const descendants=(node,tag)=>[...(node.tag===tag?[node]:[]),...node.children.flatMap(child=>descendants(child,tag))];

test('real DrawingItem SVG removes optimistic undo paths synchronously, preserves unrelated DOM and rolls back on a rejected native write',async()=>{
  const values=['svg-first','svg-second','svg-third'].map(historyDrawing),f=setup(values,addedHistory(values)),first=deferred();
  const [visible,setVisible]=solid.createSignal(values);f.props.onHistoryPreview=value=>setVisible(value??[...values]);
  f.props.onCommand=async()=>{await first.promise;throw Error('disk unavailable');};
  const root=renderer.createElement('root'),state={drawings:visible,stored:()=>[],draft:()=>undefined,tool:()=> 'pen',selected:()=>'',storedDraft:()=>undefined};
  const port={canSelect:()=>true,render:(drawing,interactive,selected)=>renderer.createComponent(shapeJsx.DrawingShape,{drawing,interactive,selected})};
  const dispose=renderer.render(()=>['svg-first','svg-second','svg-third'].map(id=>itemJsx.fixture({id,port},state)),root);
  try{
    const original=descendants(root,'path');assert.equal(original.length,3);
    void f.history(false);assert.equal(descendants(root,'path').length,2);assert.equal(descendants(root,'path')[0],original[0]);
    void f.history(false);assert.equal(descendants(root,'path').length,1);assert.equal(descendants(root,'path')[0],original[0]);
    await tick();first.resolve();await assert.rejects(()=>f.drainHistory());
    assert.equal(descendants(root,'path').length,3);assert.equal(descendants(root,'path')[0],original[0]);assert.equal(values.length,3);
  }finally{dispose();}
});

test('real DrawingItem SVG renders a new stroke before the preceding Undo is acknowledged and keeps unrelated nodes stable',async()=>{
  const clock=frames(),values=['svg-keep','svg-undo'].map(historyDrawing),f=setup(values,addedHistory(values)),first=deferred(),baseSave=f.props.onCommand;
  const [visible,setVisible]=solid.createSignal(values);f.props.onHistoryPreview=value=>setVisible(value??[...values]);
  f.props.onCommand=async command=>{if(command.type==='undo_drawing')await first.promise;await baseSave(command);};
  const root=renderer.createElement('root'),state={drawings:visible,stored:()=>[],draft:()=>undefined,tool:()=> 'pen',selected:()=>'',storedDraft:()=>undefined};
  const port={canSelect:()=>true,render:(drawing,interactive,selected)=>renderer.createComponent(shapeJsx.DrawingShape,{drawing,interactive,selected})};
  const dispose=renderer.render(()=>renderer.createComponent(solid.For,{get each(){return visible().map(value=>value.id);},children:id=>itemJsx.fixture({id,port},state)}),root);
  try{
    const keep=descendants(root,'path')[0],keepObject=descendants(root,'g').find(node=>node.props['data-drawing-id']==='svg-keep');void f.history(false);assert.equal(descendants(root,'path').length,1);assert.equal(descendants(root,'path')[0],keep);
    f.begin(event('pointerdown',10,20));f.window.emit(event('pointerup',30,40));assert.equal(descendants(root,'path').length,2);assert.equal(descendants(root,'path')[0],keep);assert.equal(values.length,2);
    assert.equal(descendants(root,'path')[1].props.d,'M10,20 L30,40');await tick();assert.equal(f.commands.length,0,'native Undo still awaits its real save');
    first.resolve();await f.drainHistory();assert.equal(descendants(root,'path').length,2);assert.equal(descendants(root,'g').find(node=>node.props['data-drawing-id']==='svg-keep'),keepObject);assert.deepEqual(f.errors,[]);
  }finally{dispose();clock.restore();}
});

test('real text effect controls combine independent styles, preserve typing focus and ignore disabled input',async()=>{
  const Icon=()=>renderer.createElement('svg');
  const controls=await jsx(await raw('./components/DrawingTextStylePicker.tsx'),{'../i18n':{t:value=>value},'lucide-solid':{Bold:Icon,Italic:Icon,Underline:Icon,Strikethrough:Icon}});
  const [value,setValue]=solid.createSignal({}),[disabled,setDisabled]=solid.createSignal(false),root=renderer.createElement('root'),changes=[];
  const dispose=renderer.render(()=>renderer.createComponent(controls.default,{get value(){return value();},get disabled(){return disabled();},preserveFocus:true,onChange:next=>{changes.push(next);setValue(next);}}),root);
  try{
    const buttons=descendants(root,'button');assert.deepEqual(buttons.map(button=>button.props['aria-label']),['加粗','斜体','下划线','删除线']);
    let prevented=false;buttons[0].props.onPointerDown({button:0,preventDefault(){prevented=true;}});assert.equal(prevented,true);
    for(const button of buttons)button.props.onClick();
    assert.deepEqual(plain(value()),{bold:true,italic:true,underline:true,strikethrough:true});
    assert.ok(buttons.every(button=>button.props['aria-pressed']===true));
    buttons[1].props.onClick();assert.deepEqual(plain(value()),{bold:true,underline:true,strikethrough:true});
    assert.deepEqual(plain(changes[0]),{bold:true},'later toggles never mutate an earlier property command');
    setDisabled(true);buttons[0].props.onClick();assert.equal(changes.length,5);assert.ok(buttons.every(button=>button.props.disabled));
    assert.deepEqual(descendants(root,'button'),buttons);
  }finally{dispose();}
});

test('real color picker switches fixed slots first, opens on the second click and persists only the edited slot in order',async()=>{
  const names=['document','window','localStorage','innerWidth','innerHeight'];const before=names.map(name=>Object.getOwnPropertyDescriptor(globalThis,name));
  const document=new Events(),window=new Events();window.dispatchEvent=event=>{window.emit(event);return true;};
  const stored=new Map([[colors.colorHistoryKey,JSON.stringify(['#123456','#ABCDEF','#789ABC'])]]),writes=[],changes=[];
  const storage={getItem:key=>stored.get(key),setItem:(key,value)=>{writes.push({key,value});stored.set(key,value);}};
  for(const [name,value]of Object.entries({document,window,localStorage:storage,innerWidth:1200,innerHeight:800}))Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});
  const [value,setValue]=solid.createSignal('#123456'),[disabled,setDisabled]=solid.createSignal(false),root=renderer.createElement('root');
  let dispose;
  try{
    dispose=renderer.render(()=>renderer.createComponent(colorPickerJsx.default,{get value(){return value();},get disabled(){return disabled();},onChange:(color,submit)=>{changes.push({color,submit});setValue(color);}}),root);
    const buttons=descendants(root,'button'),swatches=()=>buttons.map(button=>button.children[0].props.style.background);
    assert.equal(buttons.length,3);
    assert.deepEqual(swatches(),['#123456','#ABCDEF','#789ABC']);
    buttons[1].getBoundingClientRect=()=>({left:100,top:400,bottom:448});buttons[1].focus=()=>{};
    buttons[1].props.onClick();assert.deepEqual(changes,[{color:'#ABCDEF',submit:true}]);
    assert.equal(descendants(root,'div').find(node=>node.props.class==='drawing-color-panel'),undefined);
    assert.deepEqual(swatches(),['#123456','#ABCDEF','#789ABC']);
    assert.deepEqual(descendants(root,'button'),buttons,'slot nodes do not move with MRU values');
    assert.equal(writes.some(write=>write.key===colors.colorPaletteKey),false);
    buttons[1].props.onClick();const panel=descendants(root,'div').find(node=>node.props.class==='drawing-color-panel');assert.ok(panel);panel.querySelector=()=>({focus(){}});await tick();
    const hex=descendants(root,'input').find(node=>node.props['aria-label']==='十六进制颜色');hex.props.onInput({currentTarget:{value:'#654321'}});hex.props.onBlur();
    assert.deepEqual(swatches(),['#123456','#654321','#789ABC']);
    assert.deepEqual(JSON.parse(stored.get(colors.colorPaletteKey)),['#123456','#654321','#789ABC']);
    const rgb=channel=>descendants(root,'input').find(node=>node.props['aria-label']===`RGB ${channel}`);
    for(const [channel,input]of [['R','18'],['G','52'],['B','86']])rgb(channel).props.onInput({currentTarget:{value:input}});
    assert.equal(value(),'#123456');assert.equal(buttons[1].props['aria-pressed'],true,'preview keeps the original editing-slot identity even at another slot color');
    rgb('R').props.onInput({currentTarget:{value:'19'}});rgb('R').props.onBlur({currentTarget:{value:'19'}});
    assert.deepEqual(swatches(),['#123456','#133456','#789ABC']);
    hex.props.onInput({currentTarget:{value:'#123456'}});hex.props.onBlur();
    assert.deepEqual(swatches(),['#123456','#133456','#789ABC'],'a confirmed duplicate selects the existing slot without replacing or swapping either slot');
    assert.equal(buttons[0].props['aria-pressed'],true);
    assert.equal(descendants(root,'div').find(node=>node.props.class==='drawing-color-panel'),undefined);
    assert.deepEqual(descendants(root,'button'),buttons);assert.deepEqual(buttons.map(button=>button.props['data-caption']),['颜色','颜色','颜色']);
    assert.equal(buttons[1].props['aria-label'],'颜色 2：#133456');
    buttons[1].props.onClick();buttons[1].props.onClick();
    const canceledPanel=descendants(root,'div').find(node=>node.props.class==='drawing-color-panel');assert.ok(canceledPanel);
    canceledPanel.querySelector=()=>({focus(){}});await tick();
    for(const [channel,input]of [['R','18'],['G','52'],['B','86']])rgb(channel).props.onInput({currentTarget:{value:input}});
    const plane=descendants(root,'div').find(node=>node.props.class==='drawing-color-plane');plane.props.onPointerCancel();
    document.emit({type:'keydown',key:'Escape',preventDefault(){},stopImmediatePropagation(){}});
    assert.deepEqual(swatches(),['#123456','#133456','#789ABC'],'canceling a duplicate preview must retain the last confirmed edited-slot color');
    assert.equal(buttons[0].props['aria-pressed'],true);
    assert.deepEqual(JSON.parse(stored.get(colors.colorPaletteKey)),['#123456','#133456','#789ABC']);
    const savedHistory=stored.get(colors.colorHistoryKey),savedPalette=stored.get(colors.colorPaletteKey);
    setValue('#E8ECF4');assert.deepEqual(swatches(),['#E8ECF4','#133456','#789ABC']);
    assert.equal(stored.get(colors.colorHistoryKey),savedHistory);assert.equal(stored.get(colors.colorPaletteKey),savedPalette);
    const count=changes.length;setDisabled(true);assert.ok(buttons.every(button=>button.props.disabled));buttons[2].props.onClick();assert.equal(changes.length,count);
    dispose();dispose=undefined;assert.equal(document.count(),0);assert.equal(window.count(),0);
    assert.deepEqual([...colors.initialColorPalette('#123456',storage)],['#123456','#133456','#789ABC'],'reopening keeps the confirmed ordered palette, independent of MRU');
  }finally{dispose?.();for(let i=0;i<names.length;i++){if(before[i])Object.defineProperty(globalThis,names[i],before[i]);else delete globalThis[names[i]];}}
});

test('real color picker keeps fallback out of the panel, saves only final custom color and preserves its pre-preview current color',async()=>{
  const names=['document','window','localStorage','innerWidth','innerHeight'];const before=names.map(name=>Object.getOwnPropertyDescriptor(globalThis,name));
  const document=new Events(),window=new Events();window.dispatchEvent=event=>{window.emit(event);return true;};
  const stored=new Map(),writes=[];
  const storage={getItem:key=>stored.get(key),setItem:(key,value)=>{writes.push({key,value});stored.set(key,value);}};
  for(const [name,value]of Object.entries({document,window,localStorage:storage,innerWidth:1200,innerHeight:800}))Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});
  const [value,setValue]=solid.createSignal('#E8ECF4'),changes=[],root=renderer.createElement('root');let dispose;
  try{
    dispose=renderer.render(()=>renderer.createComponent(colorPickerJsx.default,{get value(){return value();},onChange:(color,submit)=>{changes.push({color,submit});setValue(color);}}),root);
    const buttons=descendants(root,'button'),swatches=()=>buttons.map(button=>button.children[0].props.style.background);
    buttons[0].getBoundingClientRect=()=>({left:100,top:400,bottom:448});let focused=false;buttons[0].focus=()=>{focused=true;};
    buttons[0].props.onClick();
    const panel=descendants(root,'div').find(node=>node.props.class==='drawing-color-panel');assert.ok(panel);
    panel.querySelector=()=>({focus(){}});await tick();
    const panelRecent=()=>descendants(root,'div').find(node=>node.props.class==='drawing-color-recent');
    assert.equal(panelRecent(),undefined);
    stored.set(colors.colorHistoryKey,JSON.stringify(['#123456']));
    window.emit({type:colors.colorHistoryChanged,detail:['#123456','bad',42,'#123456']});
    assert.deepEqual(swatches(),['#E8ECF4','#EE4848','#F4C94D']);assert.equal(writes.length,0);
    assert.equal(descendants(panelRecent(),'button').length,1,'fallback swatches never become used colors');
    const hue=descendants(root,'input').find(node=>node.props.class==='drawing-color-hue');
    hue.props.onInput({currentTarget:{value:'120'}});hue.props.onInput({currentTarget:{value:'240'}});
    assert.equal(writes.length,0);assert.ok(changes.every(change=>change.submit===false));
    const final=value();hue.props.onChange();
    assert.deepEqual(JSON.parse(stored.get(colors.colorHistoryKey)),[final,'#E8ECF4','#123456']);
    assert.equal(writes.filter(write=>write.key===colors.colorHistoryKey).length,1);assert.equal(changes.at(-1).submit,true);
    assert.deepEqual(swatches(),[final,'#EE4848','#F4C94D']);
    stored.set(colors.colorHistoryKey,JSON.stringify(['#ABCDEF']));window.emit({type:'storage',key:colors.colorHistoryKey});
    assert.deepEqual(swatches(),[final,'#EE4848','#F4C94D']);
    stored.delete(colors.colorHistoryKey);window.emit({type:'storage',key:null});assert.deepEqual(swatches(),[final,'#EE4848','#F4C94D']);
    assert.equal(panelRecent(),undefined);
    hue.props.onInput({currentTarget:{value:'90'}});assert.equal(writes.filter(write=>write.key===colors.colorHistoryKey).length,1);
    let prevented=false,stopped=false;document.emit({type:'keydown',key:'Escape',preventDefault(){prevented=true;},stopImmediatePropagation(){stopped=true;}});
    assert.equal(prevented,true);assert.equal(stopped,true);assert.equal(focused,true);
    assert.equal(descendants(root,'div').find(node=>node.props.class==='drawing-color-panel'),undefined);
    setValue('#765432');buttons[1].props.onClick();
    assert.deepEqual(JSON.parse(stored.get(colors.colorHistoryKey)),['#EE4848','#765432'],'closing a preview clears its old base before another object color is selected');
    dispose();dispose=undefined;
  }finally{dispose?.();for(let i=0;i<names.length;i++){if(before[i])Object.defineProperty(globalThis,names[i],before[i]);else delete globalThis[names[i]];}}
});

// Exercise the actual screenshot callsite and actual DrawingLayer together.
// Native/WebKit paint acceptance is separate; this gate proves node lifetime.
const canvasCode=await raw('./components/SpaceCanvas.tsx');
const canvasAst=parse(canvasCode,{sourceType:'module',plugins:['typescript','jsx']});
const layerSites=[];
function findLayerSite(node,ancestors=[]){
  if(!node||typeof node!=='object')return;
  if(node.type==='JSXElement'&&node.openingElement.name.name==='DrawingLayer'){
    const parent=ancestors.findLast(value=>value.type==='JSXElement');
    layerSites.push(parent?.openingElement.name.name==='Show'?parent:node);
  }
  for(const [key,value] of Object.entries(node)){
    if(['loc','comments','tokens'].includes(key))continue;
    if(Array.isArray(value))value.forEach(child=>findLayerSite(child,[...ancestors,node]));
    else if(value&&typeof value==='object')findLayerSite(value,[...ancestors,node]);
  }
}
findLayerSite(canvasAst);assert.equal(layerSites.length,1);
const layerSite=layerSites[0];
const captureLayerJsx=await jsx(`import DrawingLayer from 'layer';import {Show} from 'solid-js';
export function fixture(props,id,image,drawingSession){return ${canvasCode.slice(layerSite.start,layerSite.end)};}`,{layer:{default:shapeJsx.default}});

function captureLayerFixture(initial=[]){
  const pen={id:'pen',kind:'pen',color:'#ee4848',strokeWidth:4,points:[{x:20,y:30},{x:90,y:75}]};
  const [drawings,setDrawings]=solid.createSignal(initial),[session,setSession]=solid.createSignal();
  const region=()=>({id:'selection',x:10,y:15,width:180,height:120,drawings:drawings()});
  const props={get scene(){return{id:'scene',background:undefined,regions:[region()]};}};
  const root=renderer.createElement('root');
  const dispose=renderer.render(()=>captureLayerJsx.fixture(props,'selection',()=>({region:region()}),session),root);
  return{pen,root,setDrawings,setSession,dispose};
}
test('actual screenshot layer keeps its SVG and stored strokes when entering and leaving drawing',()=>{
  const text={id:'text',kind:'text',color:'#ee4848',strokeWidth:4,points:[{x:25,y:40}],text:'note',fontSize:20};
  const view=captureLayerFixture([{id:'pen',kind:'pen',color:'#ee4848',strokeWidth:4,points:[{x:20,y:30},{x:90,y:75}]},text]);
  try{
    const svg=descendants(view.root,'svg')[0],path=descendants(view.root,'path')[0];
    assert.equal(svg.props.style?.visibility??'visible','visible');
    view.setSession({regionId:'selection'});
    assert.equal(descendants(view.root,'svg')[0],svg);
    assert.equal(svg.props.style.visibility,'hidden');
    assert.equal(descendants(view.root,'path')[0],path);
    view.setDrawings([{...view.pen,points:[{x:20,y:30},{x:100,y:85}]},text]);
    const committed=descendants(view.root,'path')[0];
    assert.equal(committed.props.d,'M20,30 L100,85');
    view.setSession(undefined);
    assert.equal(descendants(view.root,'svg')[0],svg);
    assert.equal(descendants(view.root,'path')[0],committed);
    assert.equal(svg.props.style.visibility,'visible');
    assert.equal(descendants(view.root,'svg').length,1);
    assert.equal(descendants(view.root,'text').length,1);
    view.setSession({regionId:'other-selection'});
    assert.equal(svg.props.style.visibility,'visible');
  }finally{view.dispose();}
});
test('first screenshot stroke is retained hidden during editing and deleted drawings are not resurrected',()=>{
  const view=captureLayerFixture();
  try{
    view.setSession({regionId:'selection'});
    assert.equal(descendants(view.root,'svg').length,0);
    view.setDrawings([view.pen]);
    const svg=descendants(view.root,'svg')[0],path=descendants(view.root,'path')[0];
    assert.ok(svg,'committed strokes must keep the ordinary layer mounted while editing');
    assert.equal(svg.props.style.visibility,'hidden');
    view.setSession(undefined);
    assert.equal(descendants(view.root,'svg')[0],svg);
    assert.equal(descendants(view.root,'path')[0],path);
    assert.equal(svg.props.style.visibility,'visible');
    view.setSession({regionId:'selection'});view.setDrawings([]);
    assert.equal(descendants(view.root,'svg').length,0);
    view.setDrawings([{...view.pen,id:'replacement'}]);
    const replacement=descendants(view.root,'svg')[0];
    assert.notEqual(replacement,svg);
    assert.equal(replacement.props.style.visibility,'hidden');
    view.setSession(undefined);
    assert.equal(descendants(view.root,'svg')[0],replacement);
    assert.equal(replacement.props.style.visibility,'visible');
    assert.equal(descendants(view.root,'svg').length,1);
  }finally{view.dispose();}
});

const videoCode=await raw('./components/VideoDrawingEditor.tsx');
const videoBody=parse(videoCode,{sourceType:'module',plugins:['typescript','jsx']}).program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration.body.body;
const videoRender=videoBody.find(n=>n.type==='VariableDeclaration'&&n.declarations[0].id.name==='port').declarations[0].init.properties.find(n=>n.key.name==='render').value;
const videoPreviewJsx=await jsx(`import {DrawingShape} from 'shapes';export function fixture(props,object){return ${videoCode.slice(videoRender.start,videoRender.end)};}`,{shapes:shapeJsx});
test('editing a stored video text paints the live draft rather than its old native raster',()=>{
  const [text,setText]=solid.createSignal('new text');let rasterCalls=0;
  const root=renderer.createElement('root'),draw={id:'stored',kind:'text',color:'#242832',fontSize:28,strokeWidth:1,points:[{x:10,y:20}],get text(){return text();}};
  const render=videoPreviewJsx.fixture({renderStored:()=>{rasterCalls++;return renderer.createElement('image');}},()=>({primitive:{kind:'text'}}));
  const dispose=renderer.render(()=>render(draw,true,false,true),root);
  assert.equal(rasterCalls,0);assert.equal(descendants(root,'image').length,0);assert.equal(descendants(root,'text').length,1);
  const textContent=node=>node.text??node.children.map(textContent).join('');
  setText('实时文字');assert.equal(textContent(descendants(root,'text')[0]),'实时文字');
  dispose();render(draw,true,false,false);assert.equal(rasterCalls,1);
});

test('actual number SVG switches its glyph contrast live when the circle fill changes',()=>{
  const [color,setColor]=solid.createSignal('#FFFFFF');
  const root=renderer.createElement('root');
  const dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return{id:'number',kind:'number',color:color(),strokeWidth:1,points:[{x:40,y:50}],text:'1',fontSize:36};}}),root);
  const label=descendants(root,'text')[0];
  for(const [fill,foreground] of [['#FFFFFF','#000000'],['#242832','#FFFFFF'],['#F4C94D','#000000'],['#0000FF','#FFFFFF']]){
    setColor(fill);assert.equal(descendants(root,'text')[0],label);assert.equal(label.props.fill,foreground);
    assert.equal(descendants(root,'circle')[0].props.fill,fill);
  }
  dispose();
});

test('actual arrow SVG uses open adaptive wings and reuses nodes while its endpoint and line width change',()=>{
  const [length,setLength]=solid.createSignal(32),[width,setWidth]=solid.createSignal(2);
  const root=renderer.createElement('root');
  const dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return{id:'arrow',kind:'arrow',color:'#ff0000',strokeWidth:width(),points:[{x:0,y:0},{x:length(),y:0}]};}}),root);
  const head=descendants(root,'polyline')[0],shaft=descendants(root,'line')[0],points=head.props.points;
  assert.equal(head.props.fill,'none');assert.equal(descendants(root,'polygon').length,0);
  setWidth(12);assert.equal(head.props.points,points);assert.equal(descendants(root,'polyline')[0],head);
  assert.equal(descendants(root,'g')[0].props['stroke-width'],12);
  for(const [size,wing] of [[8,2],[80,20],[200,24]]){
    setLength(size);const [left,tip,right]=head.props.points.split(' ').map(value=>value.split(',').map(Number));
    assert.equal(descendants(root,'polyline')[0],head);assert.equal(descendants(root,'line')[0],shaft);
    assert.deepEqual(tip,[size,0]);assert.ok(Math.abs(Math.hypot(left[0]-size,left[1])-wing)<1e-9);
    assert.equal(left[1],-right[1]);assert.equal(shaft.props.x2,size);
  }
  dispose();
});

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

test('actual highlighter SVG switches between segmented ink and a straight preview without stale paths or changing translucency',()=>{
  const points=Array.from({length:260},(_,i)=>({x:10+i,y:20+i%4})),[draft,setDraft]=solid.createSignal({id:'highlight',kind:'highlighter',color:'#fff200',strokeWidth:18,points});
  const root=renderer.createElement('root'),state={draft,drawings:()=>[],stored:()=>[],tool:()=> 'highlighter',selected:()=> '',storedDraft:()=>undefined};let renders=0;
  const dispose=renderer.render(()=>itemJsx.fixture({id:'highlight',port:{canSelect:()=>true,render:drawing=>{renders++;return renderer.createComponent(shapeJsx.DrawingShape,{drawing});}}},state),root);
  assert.equal(descendants(root,'path').length,3);const ink=descendants(root,'g').find(node=>node.props.opacity===.35);assert.ok(ink);assert.equal(descendants(root,'g').find(node=>node.props['data-drawing-id']==='highlight').props['stroke-width'],18);
  setDraft({...draft(),points:[points[0],{x:269,y:20}]});assert.equal(descendants(root,'path').length,1);assert.equal(descendants(root,'path')[0].props.d,'M10,20 L269,20');
  setDraft({...draft(),points});assert.equal(descendants(root,'path').length,3);assert.ok(descendants(root,'path').at(-1).props.d.endsWith('L269,23'));
  points.push({x:270,y:24});setDraft({...draft()});assert.ok(descendants(root,'path').at(-1).props.d.endsWith('L270,24'));assert.equal(renders,1);assert.equal(descendants(root,'g').find(node=>node.props.opacity===.35),ink);
  dispose();
});

test('actual rectangular highlight renders distinct fill/border masks and limits its hit shape to the rounded rectangle',async()=>{
  const tick=()=>new Promise(resolve=>setImmediate(resolve));
  const calls=[];highlightLoader=async(...args)=>{calls.push(args);return {backgroundId:'a',width:120,height:80,dataUrl:'data:image/png;base64,a'};};
  const [drawing,setDrawing]=solid.createSignal({id:'mark',kind:'mark',color:'#fff200',strokeWidth:1,points:[{x:20,y:15},{x:90,y:55}]});
  const [interactive,setInteractive]=solid.createSignal(true);
  const root=renderer.createElement('root'),dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return drawing();},get interactive(){return interactive();},mosaicContext:{sceneId:'s',background:{id:'a',width:120,height:80}}}),root);
  await tick();const images=descendants(root,'image');assert.equal(images.length,2);assert.equal(calls.length,1);
  const paths=()=>descendants(root,'path');
  assert.ok(paths().some(node=>node.props['fill-opacity']===84/255));assert.ok(paths().some(node=>node.props['fill-opacity']===190/255&&node.props['fill-rule']==='evenodd'));
  const hit=paths().find(node=>node.props['pointer-events']==='fill');assert.ok(hit);assert.equal(hit.props.d,geometry.markPaths(drawing()).outer);
  const before=hit.props.d;setDrawing({...drawing(),color:'#347bf1',points:[{x:25,y:20},{x:100,y:60}]});
  assert.notEqual(paths().find(node=>node.props['pointer-events']==='fill').props.d,before);assert.deepEqual(descendants(root,'image'),images);assert.equal(calls.length,1);
  const mask=descendants(root,'mask').find(node=>String(node.props.id).endsWith('-ink'));assert.deepEqual(['x','y','width','height'].map(key=>mask.props[key]),[25,20,75,40]);
  setInteractive(false);assert.equal(paths().filter(node=>node.props['pointer-events']==='fill').length,0);dispose();
});

test('actual protected marker fences stale source responses and retains its loaded source while stroke bounds and color change',async()=>{
  const deferred=()=>{let resolve;const promise=new Promise(yes=>{resolve=yes;});return {resolve,promise};};
  const tick=()=>new Promise(resolve=>setImmediate(resolve));
  const waits=[],calls=[],errors=[];highlightLoader=(...args)=>{calls.push(args);const wait=deferred();waits.push(wait);return wait.promise;};
  const [drawing,setDrawing]=solid.createSignal({id:'protected',kind:'highlighter',color:'#fff200',strokeWidth:18,points:[{x:20,y:20},{x:60,y:20}]});
  const [context,setContext]=solid.createSignal({sceneId:'s',background:{id:'a',width:80,height:40}});
  const root=renderer.createElement('root');const dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return drawing();},get mosaicContext(){return context();},onError:error=>errors.push(error)}),root);
  const receipt=id=>({backgroundId:id,width:80,height:40,dataUrl:`data:image/png;base64,${id}`});
  assert.equal(calls.length,1);setContext({sceneId:'s',background:{id:'b',width:80,height:40}});assert.equal(calls.length,2);
  waits[0].resolve(receipt('a'));await tick();assert.equal(descendants(root,'image').length,0);
  waits[1].resolve(receipt('b'));await tick();assert.equal(descendants(root,'image').length,2);
  const images=descendants(root,'image'),filter=descendants(root,'filter')[0];
  setDrawing({...drawing(),color:'#347bf1',points:[{x:25,y:20},{x:70,y:20}]});
  assert.equal(descendants(root,'image')[0],images[0]);assert.equal(descendants(root,'image')[1],images[1]);assert.equal(descendants(root,'path')[0].props.d,'M25,20 L70,20');assert.equal(calls.length,2);
  assert.equal(filter.props.x,16);assert.equal(filter.props.width,63);assert.equal(descendants(root,'rect')[0].props.fill,'#347bf1');
  setContext({sceneId:'s',background:{id:'c',width:80,height:40}});assert.equal(descendants(root,'image').length,0);dispose();waits[2].resolve(receipt('c'));await tick();assert.deepEqual(errors,[]);
  highlightLoader=()=>{throw Error('No native IO');};
});

test('protected preview keeps crop mapping and pixels across ink-only commits but fences changed clean raster content',async()=>{
  const tick=()=>new Promise(resolve=>setImmediate(resolve)),waits=[];highlightLoader=()=>new Promise(resolve=>waits.push(resolve));
  const raster={id:'mosaic',kind:'mosaic',color:'#ff0000',strokeWidth:6,points:[{x:25,y:20},{x:40,y:35}]};
  const base={id:'r',x:20,y:10,width:100,height:60,drawings:[raster],drawingRevision:1};
  const [region,setRegion]=solid.createSignal(base),root=renderer.createElement('root');
  const dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{drawing:{id:'h',kind:'highlighter',color:'#ffff00',strokeWidth:18,points:[{x:30,y:30},{x:100,y:30}]},get mosaicContext(){return {sceneId:'s',background:{id:'a',width:180,height:100},region:region()};}}),root);
  const receipt={backgroundId:'a',x:20,y:10,width:100,height:60,dataUrl:'data:image/png;base64,a'};
  waits[0](receipt);await tick();const images=descendants(root,'image');assert.equal(images.length,2);assert.ok(images.every(node=>node.props.x===20&&node.props.y===10));
  setRegion({...base,drawingRevision:2,drawings:[raster,{id:'pen',kind:'pen',color:'#ff0000',strokeWidth:3,points:[{x:60,y:40}]}]});
  assert.equal(waits.length,1);assert.deepEqual(descendants(root,'image'),images);
  setRegion({...base,drawingRevision:3,drawings:[]});assert.equal(waits.length,2);assert.equal(descendants(root,'image').length,0);
  setRegion({...base,x:21});assert.equal(waits.length,3);waits[1](receipt);await tick();assert.equal(descendants(root,'image').length,0);
  waits[2]({...receipt,x:21});await tick();assert.equal(descendants(root,'image')[0].props.x,21);dispose();
  highlightLoader=()=>{throw Error('No native IO');};
});

test('protected foreground images never intercept clicks; only ink geometry participates in marker selection',async()=>{
  const tick=()=>new Promise(resolve=>setImmediate(resolve));
  highlightLoader=async()=>({backgroundId:'a',width:80,height:40,dataUrl:'data:image/png;base64,a'});
  const [drawing,setDrawing]=solid.createSignal({id:'hit',kind:'highlighter',color:'#fff200',strokeWidth:18,points:[{x:20,y:20},{x:60,y:20}]});
  const [interactive,setInteractive]=solid.createSignal(true),root=renderer.createElement('root');
  const dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return drawing();},get interactive(){return interactive();},mosaicContext:{sceneId:'s',background:{id:'a',width:80,height:40}}}),root);
  await tick();
  const paint=descendants(root,'g').find(node=>node.props.opacity===.35),hit=descendants(root,'g').find(node=>node.props.opacity===0);
  assert.equal(paint.props['pointer-events'],'none');assert.equal(hit.props['pointer-events'],'stroke');assert.equal(descendants(hit,'path').length,1);assert.equal(descendants(hit,'image').length,0);assert.equal(descendants(hit,'rect').length,0);
  setDrawing({...drawing(),points:[{x:20,y:20}]});assert.equal(descendants(hit,'path').length,0);assert.equal(descendants(hit,'circle')[0].props['pointer-events'],'fill');
  setInteractive(false);assert.equal(descendants(root,'g').some(node=>node.props.opacity===0),false);
  dispose();highlightLoader=()=>{throw Error('No native IO');};
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
  const props={drawing,get context(){return{sceneId:'scene',background:asset,region:{id:'region',x:0,y:0,width:500,height:400,drawingRevision:revision(),drawings:[drawing]}};}};
  const dispose=renderer.render(()=>renderer.createComponent(rich.default,props),root);
  await tick();jobs[0].resolve({...jobs[0].target,dataUrl:'synthetic-pixels'});await tick();const node=descendants(root,'image')[0];assert.ok(node);
  for(let i=1;i<=8;i++){setRevision(i);await tick();assert.equal(descendants(root,'image')[0],node);jobs[i].resolve({...jobs[i].target,dataUrl:'synthetic-pixels'});await tick();assert.equal(descendants(root,'image')[0],node);}
  setRevision(9);await tick();jobs[9].reject(Error('native rejected current object'));await tick();assert.equal(descendants(root,'image').length,0);dispose();
});

test('actual rich JSX never reads an optimistic redo until native snapshot acknowledges it',async()=>{
  const layout=await pure('./drawing-layout-preview.ts'),jobs=[],errors=[];
  const rich=await jsx(await raw('./components/RichDrawingShape.tsx'),{'../drawing-layout-preview':layout,'../drawing-layout-bridge':{getDrawingLayoutPreview:target=>new Promise((resolve,reject)=>jobs.push({target,resolve,reject}))}});
  const drawing={id:'formula',kind:'rich',points:[{x:10,y:10},{x:100,y:60}],rich:{layoutId:'immutable',kind:'formula',width:90,height:50}},asset={id:'source',kind:'image',width:500,height:400};
  const [document,setDocument]=solid.createSignal({id:'region',x:0,y:0,width:500,height:400,drawingRevision:2,drawings:[]}),root=renderer.createElement('root');
  const dispose=renderer.render(()=>renderer.createComponent(rich.default,{drawing,get context(){return{sceneId:'scene',background:asset,region:document()};},onError:value=>errors.push(value)}),root);
  await tick();assert.equal(jobs.length,0);assert.equal(errors.length,0);
  setDocument({...document(),drawingRevision:3,drawings:[drawing]});await tick();assert.equal(jobs.length,1);assert.equal(jobs[0].target.expectedRevision,3);
  jobs[0].resolve({...jobs[0].target,dataUrl:'synthetic-pixels'});await tick();assert.equal(descendants(root,'image').length,1);assert.equal(errors.length,0);
  setDocument({...document(),drawingRevision:4,drawings:[]});await tick();assert.equal(jobs.length,1);assert.equal(descendants(root,'image').length,0);dispose();
});


test('native-cloned pen points retain exactly equal segment objects including the tail',()=>{
  const segment=pointer.penSegments(8),points=Array.from({length:33},(_,i)=>({x:i,y:i%5}));
  const original=segment(points),copy=segment(structuredClone(points));
  assert.equal(original.length,5);assert.equal(copy.length,original.length);
  for(let i=0;i<copy.length;i++)assert.equal(copy[i],original[i]);
  assert.deepEqual(plain(copy),plain(original));
});

test('native-cloned pen edits regenerate real paths and invalidate only the changed segments',()=>{
  const segment=pointer.penSegments(8),points=Array.from({length:33},(_,i)=>({x:i,y:i%5}));
  const original=segment(points),copy=structuredClone(points);copy[10].y=100;
  const changed=segment(copy);
  for(let i=0;i<changed.length;i++)assert.equal(changed[i]===original[i],i!==1,`segment ${i}`);
  assert.ok(changed[1].path.includes('L10,100'));
  const reordered=structuredClone(copy);[reordered[10],reordered[17]]=[reordered[17],reordered[10]];
  const next=segment(reordered);assert.notEqual(next[1],changed[1]);assert.notEqual(next[2],changed[2]);
  assert.equal(next[0],changed[0]);assert.equal(next.at(-1),changed.at(-1));
});

test('native-cloned pen shortening and growth preserve only the unchanged prefix geometry',()=>{
  const segment=pointer.penSegments(8),points=Array.from({length:33},(_,i)=>({x:i,y:i%5}));
  const original=segment(points),short=segment(structuredClone(points.slice(0,20)));
  assert.equal(short.length,3);assert.equal(short[0],original[0]);assert.equal(short[1],original[1]);
  assert.notEqual(short[2],original[2]);assert.ok(short[2].path.endsWith('L19,4'));
  const grown=segment(structuredClone(points.slice(0,25)));
  assert.equal(grown.length,4);assert.equal(grown[0],short[0]);assert.equal(grown[1],short[1]);
  assert.notEqual(grown[2],short[2]);assert.ok(grown.at(-1).path.endsWith('L24,4'));
  assert.deepEqual(plain(segment([])),[]);assert.deepEqual(plain(segment([{x:1,y:2}])),[]);
});

test('mutating the old pen array before a native clone cannot reuse a stale rendered path',()=>{
  const segment=pointer.penSegments(8),points=Array.from({length:33},(_,i)=>({x:i,y:i%5}));
  const original=segment(points),oldPath=original[1].path;
  points[10].y=777;
  const result=segment(structuredClone(points));
  assert.notEqual(result[1],original[1]);assert.equal(original[1].path,oldPath);
  assert.ok(result[1].path.includes('L10,777'));
  for(const index of [0,2,3,4])assert.equal(result[index],original[index]);
});

test('same-array append after native adoption reads only the bounded tail coordinates',()=>{
  let protectPrefix=false,reads=0;
  const point=i=>({get x(){if(protectPrefix&&i<28)throw Error('completed prefix read');reads++;return i;},get y(){if(protectPrefix&&i<28)throw Error('completed prefix read');reads++;return i%5;}});
  const segment=pointer.penSegments(8),points=Array.from({length:33},(_,i)=>point(i));
  const original=segment(points);protectPrefix=true;reads=0;
  points.push(point(33));const appended=segment(points);
  for(let i=0;i<original.length-1;i++)assert.equal(appended[i],original[i]);
  assert.notEqual(appended.at(-1),original.at(-1));assert.ok(appended.at(-1).path.endsWith('L33,3'));
  assert.ok(reads<=16,`tail coordinate reads ${reads}`);
});

test('actual DrawingShape JSX preserves all native-cloned SVG path nodes and replaces changed geometry precisely',()=>{
  const points=Array.from({length:401},(_,i)=>({x:i,y:i%5}));
  const [drawing,setDrawing]=solid.createSignal({id:'native-pen',kind:'pen',color:'#ee4848',strokeWidth:4,points});
  const root=renderer.createElement('root'),dispose=renderer.render(()=>renderer.createComponent(shapeJsx.DrawingShape,{get drawing(){return drawing();}}),root);
  try{
    const original=descendants(root,'path');assert.equal(original.length,4);
    setDrawing(structuredClone(drawing()));const copy=descendants(root,'path');
    for(let i=0;i<original.length;i++)assert.equal(copy[i],original[i]);
    const changed=structuredClone(drawing());changed.points[150].y=777;setDrawing(changed);
    const edited=descendants(root,'path');
    for(let i=0;i<original.length;i++)assert.equal(edited[i]===original[i],i!==1,`rendered segment ${i}`);
    assert.ok(edited[1].props.d.includes('L150,777'));
    const short=structuredClone(drawing());short.points=short.points.slice(0,300);setDrawing(short);
    const shortened=descendants(root,'path');assert.equal(shortened.length,3);
    assert.equal(shortened[0],edited[0]);assert.equal(shortened[1],edited[1]);assert.notEqual(shortened[2],edited[2]);
    assert.ok(shortened[2].props.d.endsWith('L299,4'));
    setDrawing({...drawing(),color:'#0000ff'});assert.equal(descendants(root,'path')[0],shortened[0]);
    assert.equal(descendants(root,'g').find(node=>node.props['data-drawing-id']==='native-pen').props.stroke,'#0000ff');
  }finally{dispose();}
});

const mouse=(type,x,y,extra={})=>event(type,x,y,{pointerType:'mouse',...extra});

test('release fallback preserves only pressed pen samples when zero-button move precedes lost capture and up',async()=>{
 const clock=frames();try{
  const f=setup();f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',50,60));
  f.window.emit(mouse('pointermove',900,950,{buttons:0,getCoalescedEvents(){throw Error('hover coordinates must not be sampled');}}));await tick();
  assert.equal(f.commands.length,1);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);
  assert.equal(f.hasGesture(),false);assert.equal(clock.queued.size,0);assert.equal(f.window.count(),0);
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',950,999));clock.paint();await tick();
  assert.equal(f.commands.length,1,'late lost/up must not double-save');assert.deepEqual(f.errors,[]);
 }finally{clock.restore();}
});

test('release fallback flushes the latest pressed vector preview without appending the hover endpoint',async()=>{
 const clock=frames();try{
  for(const kind of ['highlighter','line','arrow','rect','ellipse']){
   const f=setup();f.tool(kind);f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',50,80));
   assert.equal(clock.queued.size,1,kind);f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();
   assert.equal(f.commands.length,1,kind);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:50,y:80}],kind);assert.equal(clock.queued.size,0,kind);
   f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',900,950));await tick();assert.equal(f.commands.length,1,kind);
  }
 }finally{clock.restore();}
});

test('release fallback permits an immediate next stroke while its previous save is pending and Undo immediately followed by drawing',async()=>{
 const clock=frames(),values=[],f=setup(values,{undo:[],redo:[]});try{
  const save=f.props.onCommand,gate=deferred();let first=true;
  f.props.onCommand=async command=>{if(first){first=false;await gate.promise;}await save(command);};
  f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();
  f.begin(mouse('pointerdown',40,50));assert.equal(f.hasGesture(),true);f.window.emit(mouse('pointermove',60,70));f.window.emit(mouse('pointermove',900,950,{buttons:0}));
  gate.resolve();await f.drainHistory();assert.equal(values.length,2);assert.equal(f.commands.length,2);
  void f.history(false);f.begin(mouse('pointerdown',70,80));assert.equal(f.hasGesture(),true,'new pen admitted immediately after toolbar history entry');
  f.window.emit(mouse('pointermove',90,100));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await f.drainHistory();
  assert.deepEqual(f.commands.map(v=>v.type),['add_drawing','add_drawing','undo_drawing','add_drawing']);assert.equal(values.length,2);assert.deepEqual(values[1].points,[{x:70,y:80},{x:90,y:100}]);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();clock.restore();}
});

test('release fallback still cancels invalid source geometry tool permission blur cancel and explicit lost capture',async()=>{
 const clock=frames();try{
  for(const reason of ['source','revision','box','surface','tool','plugin','allow','lock','blur','resize','pointercancel','lost','dispose']){
   const f=setup();f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));
   if(reason==='source')f.props.source='successor';if(reason==='revision')f.props.revision++;
   if(reason==='box')f.props.box.x++;if(reason==='surface')f.svg.box.left++;if(reason==='tool')f.tool('line');if(reason==='plugin')f.props.tools=[];if(reason==='allow')f.props.allowed=false;
   if(reason==='lock')f.props.inputLocked=true;if(reason==='blur')f.blur();if(reason==='resize')f.resize();if(reason==='pointercancel')f.window.emit(mouse('pointercancel',30,40));if(reason==='lost')f.svg.emit(mouse('lostpointercapture',30,40));if(reason==='dispose')f.dispose();
   f.window.emit(mouse('pointermove',900,950,{buttons:0}));clock.paint();f.window.emit(mouse('pointerup',900,950));await tick();
   assert.equal(f.commands.length,0,reason);assert.equal(f.draft(),undefined,reason);assert.equal(f.hasGesture(),false,reason);assert.equal(clock.queued.size,0,reason);
  }
 }finally{clock.restore();}
});

test('release fallback excludes pen touch unknown devices mismatched types other buttons and chorded initial down',async()=>{
 const clock=frames();try{
  for(const [downType,moveType,downButtons,moveButtons]of [['pen','pen',1,0],['touch','touch',1,0],[undefined,undefined,1,0],['mouse','pen',1,0],['pen','mouse',1,0],['mouse','mouse',1,2],['mouse','mouse',1,4],['mouse','mouse',3,0]]){
   const f=setup();f.begin(event('pointerdown',10,20,{pointerType:downType,buttons:downButtons}));f.window.emit(event('pointermove',30,40,{pointerType:moveType}));
   f.window.emit(event('pointermove',900,950,{pointerType:moveType,buttons:moveButtons}));clock.paint();await tick();assert.equal(f.commands.length,0,JSON.stringify([downType,moveType,downButtons,moveButtons]));assert.equal(f.hasGesture(),false);
  }
 }finally{clock.restore();}
});

test('release fallback saves ordinary mouse selection movement and ignores another pointer zero-button move',async()=>{
 const clock=frames();try{
  const old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],f=setup(values);f.tool('select');const target=new module.namespace.Element();target.id='old';
  f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'update_drawing');assert.equal(f.commands[0].expectedRevision,5);assert.deepEqual(f.commands[0].drawing.points,[{x:30,y:40},{x:70,y:80}]);
  const g=setup();g.begin(mouse('pointerdown',10,20));g.window.emit(mouse('pointermove',30,40));g.window.emit(mouse('pointermove',900,950,{pointerId:8,buttons:0}));assert.equal(g.hasGesture(),true);assert.equal(g.commands.length,0);
  g.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(g.commands.length,1);assert.deepEqual(g.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40}]);
 }finally{clock.restore();}
});

const publication=await pure('./published-revision.ts');
test('release fallback reactive real gestureSource survives own publication during the next pen and immediate Undo then pen',async()=>{
 const clock=frames(),values=[],f=setup(values,{undo:[],redo:[]},true);
 const baseSave=f.props.onCommand,reply=deferred(),publish=deferred();let calls=0;
 const witness=new publication.PublishedRevision(()=>f.props.revision,async()=>{throw Error('unexpected refresh');},1000);
 const mouse=(type,x,y,extra={})=>event(type,x,y,{pointerType:'mouse',...extra});
 try{
  f.props.onCommand=async command=>{calls++;const first=calls===1;if(first)await publish.promise;await baseSave(command);witness.changed();const receipt=f.props.revision;if(first)await reply.promise;await witness.wait(receipt);};
  f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();
  f.begin(mouse('pointerdown',40,50));f.window.emit(mouse('pointermove',60,70));assert.equal(f.hasGesture(),true);
  publish.resolve();await tick();assert.equal(f.hasGesture(),true,'own publication preserves live next pen');reply.resolve();await tick();assert.equal(f.hasGesture(),true,'numeric receipt preserves live next pen');
  f.window.emit(mouse('pointermove',900,950,{buttons:0}));f.begin(mouse('pointerdown',70,80));f.window.emit(mouse('pointermove',90,100));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await f.drainHistory();
  assert.equal(values.length,3);assert.equal(f.commands.length,3);
  void f.history(false);f.begin(mouse('pointerdown',100,110));assert.equal(f.hasGesture(),true);f.window.emit(mouse('pointermove',120,130));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await f.drainHistory();
  assert.equal(values.length,3);assert.deepEqual(f.commands.map(v=>v.type),['add_drawing','add_drawing','add_drawing','undo_drawing','add_drawing']);assert.deepEqual(values.at(-1).points,[{x:100,y:110},{x:120,y:130}]);assert.deepEqual(f.errors,[]);
 }finally{witness.dispose();f.dispose();f.disposeReactive();clock.restore();}
});

const videoAllowsNode=videoBody.find(n=>n.type==='VariableDeclaration'&&n.declarations[0].id.name==='port').declarations[0].init.properties.find(n=>n.key.name==='allows').value;
const videoCurrentNode=videoBody.find(n=>n.type==='VariableDeclaration'&&n.declarations[0].id.name==='current').declarations[0].init;
const videoStoredMoveNode=videoBody.find(n=>n.type==='VariableDeclaration'&&n.declarations[0].id.name==='port').declarations[0].init.properties.find(n=>n.key.name==='storedMove').value;
const videoGateM=new SourceTextModule(transform(`export function gate(props,object){let disposed=false;const current=${videoCode.slice(videoCurrentNode.start,videoCurrentNode.end)};return{allows:${videoCode.slice(videoAllowsNode.start,videoAllowsNode.end)},storedMove:${videoCode.slice(videoStoredMoveNode.start,videoStoredMoveNode.end)},dispose(){disposed=true;}};}`));
await videoGateM.link(()=>{throw Error('No native/model IO in video release fixture');});await videoGateM.evaluate();
function storedVideoReleaseSetup(values=[]){
 const f=setup(values);f.props.active=true;f.props.item={asset:{id:'synthetic-video-source',kind:'video'}};f.props.grant={pluginId:'mewu.core.drawing',revision:1,contributionId:'video-drawing-tools',tools:['pen','line','arrow','rect','ellipse','text','number']};f.props.info={width:1000,height:1000};
 const gate=videoGateM.namespace.gate(f.props,id=>f.props.videoObject?.id===id?f.props.videoObject:undefined);f.props.tools=[...f.props.grant.tools];
 f.props.port.stored=()=>f.props.storedEntries??[];f.props.port.storedMove=gate.storedMove;f.props.port.frame=()=>({x:0,y:0,width:f.props.info.width,height:f.props.info.height});f.props.port.sourceSize=()=>({...f.props.info});
 f.props.port.sourceIdentity=()=>JSON.stringify([f.props.source,f.props.item.asset,f.props.info,f.props.grant]);f.props.port.allows=gate.allows;f.sourceChanged();
 assert.equal(f.queue(),undefined,'stored video never enters the screenshot add queue');assert.equal(f.lane(),undefined,'stored video retains the single-CAS path');
 const save=f.props.onCommand;let admissions=0;f.props.onCommand=async action=>{assert.equal(action.expectedRevision,f.props.revision,'actual Shared commit preserves one current CAS');assert.equal(gate.allows(action),true,'actual video port authorizes before dispatch');admissions++;await f.props.nativeHold?.promise;if(action.type==='move_stored'){f.commands.push(plain(action));f.props.revision++;return;}return save(action);};
 return{...f,gate,admissions:()=>admissions};
}

test('missing PointerDown preserves production video grant and image edit adapter admission and source mapping',async()=>{
 const clock=frames();try{
  const video=storedVideoReleaseSetup(),input=mouseSurface(video);try{
   input.hover();input.missing(10,20);video.window.emit(input.mouse('mousemove',30,40));input.up(50,60);await tick();
   assert.equal(video.admissions(),1);assert.equal(video.commands[0].expectedRevision,5);assert.deepEqual(video.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);
   video.props.grant={...video.props.grant,tools:['line']};input.hover();input.missing(70,80);input.up(90,100);await tick();assert.equal(video.admissions(),1,'fallback does not expand the current video grant');
  }finally{input.detach();video.dispose();video.gate.dispose();}
  const code=await raw('./components/ImageDrawingEditor.tsx'),tree=parse(code,{sourceType:'module',plugins:['typescript','jsx']});
  const statements=tree.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
  const port=statements.find(node=>node.type==='VariableDeclaration'&&node.declarations[0].id.name==='port').declarations[0].init;
  const allows=port.properties.find(node=>node.key.name==='allows').value;
  const command=statements.find(node=>node.type==='VariableDeclaration'&&node.declarations[0].id.name==='command').declarations[0].init;
  const adapter=new SourceTextModule(transform(`export function bind(props){return{allows:${code.slice(allows.start,allows.end)},command:${code.slice(command.start,command.end)}};}`));
  await adapter.link(()=>{throw Error('Image adapter has no native side channel');});await adapter.evaluate();
  const image=setup(),mouse=mouseSurface(image),saved=image.props.onCommand,outer=[];
  const binding=adapter.namespace.bind({sceneId:'image-scene',item:{id:'image-object',asset:{id:'image-asset'}},onCommand:async value=>{outer.push(plain(value));await saved({...value.action,expectedRevision:value.expectedRevision});}});
  image.props.port.allows=binding.allows;image.props.onCommand=binding.command;
  try{
   mouse.hover();mouse.missing(10,20);image.window.emit(mouse.mouse('mousemove',30,40));mouse.up(50,60);await tick();
   assert.equal(outer.length,1);assert.equal(outer[0].type,'edit_image_annotations');assert.equal(outer[0].sceneId,'image-scene');assert.equal(outer[0].itemId,'image-object');assert.equal(outer[0].sourceAssetId,'image-asset');assert.equal(outer[0].expectedRevision,5);
   assert.equal(outer[0].action.type,'add_drawing');assert.deepEqual(outer[0].action.drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);assert.deepEqual(image.errors,[]);
  }finally{mouse.detach();image.dispose();}
 }finally{clock.restore();}
});

test('stored video release fallback saves a new pen once through the production commit and actual video grant gate',async()=>{
 const clock=frames(),f=storedVideoReleaseSetup();try{
  f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',50,60));f.window.emit(mouse('pointermove',900,950,{buttons:0,getCoalescedEvents(){throw Error('Hover must not become video ink');}}));await tick();
  assert.equal(f.admissions(),1);assert.equal(f.commands.length,1);assert.equal(f.commands[0].expectedRevision,5);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:30,y:40},{x:50,y:60}]);
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',900,950));clock.paint();await tick();assert.equal(f.admissions(),1);assert.equal(f.props.revision,6);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();f.gate.dispose();clock.restore();}
});

test('stored video release fallback flushes every authorized vector preview but still rejects ungranted highlighter',async()=>{
 const clock=frames();try{
  for(const kind of ['line','arrow','rect','ellipse']){
   const f=storedVideoReleaseSetup();try{f.tool(kind);f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',50,80));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.admissions(),1,kind);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:20},{x:50,y:80}],kind);assert.equal(f.commands[0].drawing.kind,kind);}finally{f.dispose();f.gate.dispose();}
  }
  const denied=storedVideoReleaseSetup();try{denied.props.tools.push('highlighter');denied.tool('highlighter');denied.begin(mouse('pointerdown',10,20));denied.window.emit(mouse('pointermove',30,40));denied.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(denied.admissions(),0);assert.equal(denied.commands.length,0);}finally{denied.dispose();denied.gate.dispose();}
 }finally{clock.restore();}
});

test('stored video release fallback retains pending single-CAS and Busy inputLocked admission barriers',async()=>{
 const clock=frames(),f=storedVideoReleaseSetup();try{
  f.props.nativeHold=deferred();f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.admissions(),1);
  f.begin(mouse('pointerdown',40,50));assert.equal(f.hasGesture(),false,'pending single-CAS still blocks a new video gesture');f.window.emit(mouse('pointermove',60,70));f.window.emit(mouse('pointerup',60,70));assert.equal(f.admissions(),1);
  f.props.nativeHold.resolve();await tick();await tick();assert.equal(f.props.revision,6);
  for(const gate of ['busy','inputLocked']){f.props[gate]=true;f.begin(mouse('pointerdown',40,50));assert.equal(f.hasGesture(),false,gate);f.props[gate]=false;}
  f.props.nativeHold=undefined;f.begin(mouse('pointerdown',40,50));f.window.emit(mouse('pointermove',60,70));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.admissions(),2);assert.equal(f.commands[1].expectedRevision,6);
 }finally{f.dispose();f.gate.dispose();clock.restore();}
});

test('stored video release fallback cancels changed source revision frame display active grant and explicit cancellation',async()=>{
 const clock=frames();try{
  for(const reason of ['source','asset','revision','frame','box','surface','tool','grant','inactive','busy','inputLocked','blur','pointercancel','lost']){
   const f=storedVideoReleaseSetup();try{
    f.begin(mouse('pointerdown',10,20));f.window.emit(mouse('pointermove',30,40));
    if(reason==='source')f.props.source='successor';if(reason==='asset')f.props.item.asset.id='successor';if(reason==='revision')f.props.revision++;if(reason==='frame')f.props.info.width++;
    if(reason==='box')f.props.box.x++;if(reason==='surface')f.svg.box.left++;if(reason==='tool')f.tool('line');if(reason==='grant')f.props.grant.tools=[];if(reason==='inactive')f.props.active=false;if(reason==='busy')f.props.busy=true;if(reason==='inputLocked')f.props.inputLocked=true;
    if(reason==='blur')f.blur();if(reason==='pointercancel')f.window.emit(mouse('pointercancel',30,40));if(reason==='lost')f.svg.emit(mouse('lostpointercapture',30,40));
    f.window.emit(mouse('pointermove',900,950,{buttons:0}));clock.paint();f.window.emit(mouse('pointerup',900,950));await tick();assert.equal(f.admissions(),0,reason);assert.equal(f.commands.length,0,reason);assert.equal(f.draft(),undefined,reason);
   }finally{f.dispose();f.gate.dispose();}
  }
 }finally{clock.restore();}
});

test('stored video release fallback preserves pen and touch cancellation',async()=>{
 const clock=frames();try{
  for(const pointerType of ['pen','touch']){const g=storedVideoReleaseSetup();try{g.begin(event('pointerdown',10,20,{pointerType}));g.window.emit(event('pointermove',30,40,{pointerType}));g.window.emit(event('pointermove',900,950,{pointerType,buttons:0}));await tick();assert.equal(g.admissions(),0,pointerType);}finally{g.dispose();g.gate.dispose();}}
 }finally{clock.restore();}
});

function prepareStoredVideoMove(f){
 const bounds={x:10,y:20,width:40,height:30};f.props.storedEntries=[{id:'stored-vector',kind:'vector',bounds}];f.props.videoObject={id:'stored-vector',primitive:{kind:'vector',topLeft:{x:10,y:20},layout:{width:40,height:30,geometryBounds:{x:0,y:0,width:40,height:30}}}};
 f.tool('select');const target=new module.namespace.Element();target.id='stored-vector';return target;
}
test('stored video Move release fallback commits only the last pressed actual storedMove once and never removes it',async()=>{
 const clock=frames(),f=storedVideoReleaseSetup();try{
  const target=prepareStoredVideoMove(f);f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));assert.deepEqual(plain(f.storedDraft().bounds),{x:30,y:40,width:40,height:30});
  f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.admissions(),1);assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'move_stored');assert.equal(f.commands[0].expectedRevision,5);assert.deepEqual(f.commands[0].from,{x:10,y:20,width:40,height:30});assert.deepEqual(f.commands[0].to,{x:30,y:40,width:40,height:30});
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',900,950));await tick();assert.equal(f.admissions(),1);assert.equal(f.props.revision,6);assert.equal(f.storedDraft(),undefined);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();f.gate.dispose();clock.restore();}
});
test('stored video Move release fallback retains invalid-source geometry tool Busy and explicit-cancel gates',async()=>{
 const clock=frames();try{
  for(const reason of ['source','asset','revision','frame','box','surface','tool','inactive','busy','inputLocked','blur','pointercancel','lost','pen','touch','otherbuttons']){
   const f=storedVideoReleaseSetup();try{
    const target=prepareStoredVideoMove(f),pointerType=reason==='pen'?'pen':reason==='touch'?'touch':'mouse';f.begin(event('pointerdown',10,20,{target,pointerType}));f.window.emit(event('pointermove',30,40,{pointerType}));
    if(reason==='source')f.props.source='successor';if(reason==='asset')f.props.item.asset.id='successor';if(reason==='revision')f.props.revision++;if(reason==='frame')f.props.info.width++;if(reason==='box')f.props.box.x++;if(reason==='surface')f.svg.box.left++;if(reason==='tool')f.tool('pen');if(reason==='inactive')f.props.active=false;if(reason==='busy')f.props.busy=true;if(reason==='inputLocked')f.props.inputLocked=true;
    if(reason==='blur')f.blur();if(reason==='pointercancel')f.window.emit(event('pointercancel',30,40,{pointerType}));if(reason==='lost')f.svg.emit(event('lostpointercapture',30,40,{pointerType}));
    f.window.emit(event('pointermove',900,950,{pointerType,buttons:reason==='otherbuttons'?2:0}));f.window.emit(event('pointerup',900,950,{pointerType}));await tick();assert.equal(f.admissions(),0,reason);assert.equal(f.commands.length,0,reason);assert.equal(f.storedDraft(),undefined,reason);
   }finally{f.dispose();f.gate.dispose();}
  }
 }finally{clock.restore();}
});

function ordinaryMoveFixture(values,history){
 const f=setup(values,history),save=f.props.onCommand;
 f.props.onCommand=async action=>{
  if(action.type!=='update_drawing'&&action.type!=='update_drawings')return save(action);
  assert.equal(action.expectedRevision,f.props.revision,'ordinary move retains exact current CAS');
  const changes=[];for(const drawing of action.type==='update_drawings'?action.drawings:[action.drawing]){const index=values.findIndex(value=>value.id===drawing.id);assert.notEqual(index,-1);const before=plain(values[index]),after=plain(drawing);changes.push({index,before,after});values[index]=after;}
  f.commands.push(plain(action));if(history){history.undo.push(changes.length===1?changes[0]:{batch:changes});history.redo=[];}f.props.revision++;
 };
 f.tool('select');return f;
}
test('ordinary vector Move mouse release fallback publishes the prior pressed preview once and permits another move after Undo is acknowledged',async()=>{
 const clock=frames(),old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],history={undo:[],redo:[]},f=ordinaryMoveFixture(values,history),target=new module.namespace.Element();target.id='old';
 try{
  f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));assert.equal(clock.queued.size,1);
  f.window.emit(mouse('pointermove',900,950,{buttons:0,getCoalescedEvents(){throw Error('Released hover must never be sampled');}}));await tick();
  assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'update_drawing');assert.equal(f.commands[0].expectedRevision,5);assert.deepEqual(values[0].points,[{x:30,y:40},{x:70,y:80}]);
  assert.equal(f.hasGesture(),false);assert.equal(clock.queued.size,0);assert.equal(f.props.revision,6);assert.equal(history.undo.length,1);
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',990,999));clock.paint();await tick();assert.equal(f.commands.length,1);
  await f.history(false);await f.drainHistory();assert.deepEqual(values,[old]);assert.equal(f.props.revision,7);
  f.begin(mouse('pointerdown',10,20,{target}));assert.equal(f.hasGesture(),true);f.window.emit(mouse('pointermove',50,60));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.commands.at(-1).expectedRevision,7);assert.deepEqual(values[0].points,[{x:50,y:60},{x:90,y:100}]);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();clock.restore();}
});
test('ordinary vector Move mouse release fallback preserves exact non-pen geometry and leaves an unmoved click unchanged',async()=>{
 const clock=frames();try{
  for(const kind of ['line','arrow','rect','ellipse','highlighter']){
   const old={id:'old',kind,color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],f=ordinaryMoveFixture(values),target=new module.namespace.Element();target.id='old';
   try{f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));clock.paint();f.window.emit(mouse('pointermove',50,60));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.commands.length,1,kind);assert.equal(values[0].kind,kind);assert.deepEqual(values[0].points,[{x:50,y:60},{x:90,y:100}],kind);}finally{f.dispose();}
  }
  const old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},f=ordinaryMoveFixture([old]),target=new module.namespace.Element();target.id='old';
  try{f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.commands.length,0);}finally{f.dispose();}
 }finally{clock.restore();}
});
test('ordinary vector Move mouse release fallback retains source geometry selected tool permission Busy and explicit-cancel gates',async()=>{
 const clock=frames();try{
  for(const reason of ['source','revision','size','box','surface','selected','tool','permission','busy','locked','blur','pointercancel','lost']){
   const old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],f=ordinaryMoveFixture(values),target=new module.namespace.Element();target.id='old';
   try{
    f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));
    if(reason==='source')f.props.source='new';if(reason==='revision')f.props.revision++;if(reason==='size')f.props.port.sourceSize=()=>({width:999,height:1000});if(reason==='box')f.props.box.x++;
    if(reason==='surface')f.svg.box.top++;if(reason==='selected')f.selected('new');if(reason==='tool')f.tool('line');if(reason==='permission')f.props.allowed=false;if(reason==='busy')f.props.busy=true;if(reason==='locked')f.props.inputLocked=true;
    if(reason==='blur')f.blur();if(reason==='pointercancel')f.window.emit(mouse('pointercancel',30,40));if(reason==='lost')f.svg.emit(mouse('lostpointercapture',30,40));
    f.window.emit(mouse('pointermove',900,950,{buttons:0}));clock.paint();f.window.emit(mouse('pointerup',900,950));await tick();assert.equal(f.commands.length,0,reason);assert.deepEqual(values,[old],reason);assert.equal(f.draft(),undefined,reason);
   }finally{f.dispose();}
  }
 }finally{clock.restore();}
});
test('ordinary vector Move mouse release fallback excludes pen touch mixed buttons and blackboard trash-object movement',async()=>{
 const clock=frames();try{
  for(const [downType,moveType,downButtons,moveButtons]of [['pen','pen',1,0],['touch','touch',1,0],['mouse','pen',1,0],['mouse','mouse',3,0],['mouse','mouse',1,2]]){
   const old={id:'old',kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],f=ordinaryMoveFixture(values),target=new module.namespace.Element();target.id='old';
   try{f.begin(event('pointerdown',10,20,{target,pointerType:downType,buttons:downButtons}));f.window.emit(event('pointermove',30,40,{pointerType:downType,buttons:downButtons}));f.window.emit(event('pointermove',900,950,{pointerType:moveType,buttons:moveButtons}));await tick();assert.equal(f.commands.length,0,downType+'/'+moveButtons);assert.deepEqual(values,[old]);}finally{f.dispose();}
  }
  const old={id:'old',kind:'rich',rich:{kind:'extracted'},color:'#123456',strokeWidth:4,points:[{x:10,y:20},{x:50,y:60}]},values=[structuredClone(old)],f=ordinaryMoveFixture(values),target=new module.namespace.Element();target.id='old';f.props.blackboard=true;f.props.trashOver=true;
  try{f.begin(mouse('pointerdown',10,20,{target}));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();assert.equal(f.commands.length,0,'MOVE0 must neither move nor delete blackboard objects');assert.deepEqual(values,[old]);}finally{f.dispose();}
 }finally{clock.restore();}
});

function selectThreeFixture(history){
 const values=[10,40,70].map((y,i)=>({id:'s'+i,kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y},{x:50,y:y+4}]})),f=ordinaryMoveFixture(values,history);return{...f,values};
}
function selectThree(f){f.begin(mouse('pointerdown',0,0));f.window.emit(mouse('pointermove',60,90));f.window.emit(mouse('pointermove',900,950,{buttons:0}));}
test('Ctrl copy works while pen remains selected and does not enqueue an accidental new stroke',async()=>{
 const clock=frames(),f=selectThreeFixture(),old=plain(f.values);try{
  f.tool('pen');const target=new module.namespace.Element();target.id='s0';
  f.begin(mouse('pointerdown',10,10,{target,ctrlKey:true}));f.window.emit(mouse('pointermove',30,40));f.window.emit(mouse('pointerup',30,40));await tick();
  assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'duplicate_drawings');assert.deepEqual(f.commands[0].drawingIds,['s0']);assert.deepEqual(f.values,old);
 }finally{f.dispose();clock.restore();}
});
test('Escape closes a blackboard property menu before finishing the board',()=>{
 const f=selectThreeFixture(),prior=globalThis.document;f.props.blackboard=true;f.props.propertyMenu=true;globalThis.document={activeElement:null,querySelector:()=>null};
 try{f.keyboard({type:'keydown',key:'Escape',target:null,preventDefault(){},stopImmediatePropagation(){}});assert.equal(f.props.propertyMenu,false);assert.equal(f.commands.length,0);}finally{globalThis.document=prior;f.dispose();}
});
test('Ctrl drag duplicates the exact selected group once without moving originals; cancellation writes nothing',async()=>{
 const clock=frames();try{
  for(const cancel of [false,true]){
   const f=selectThreeFixture(),old=plain(f.values);selectThree(f);const target=new module.namespace.Element();target.id='s0';
   f.begin(mouse('pointerdown',10,10,{target,ctrlKey:true}));f.window.emit(mouse('pointermove',30,40));
   assert.equal(f.groupDraft().length,3);assert.ok(f.groupDraft().every(d=>!old.some(o=>o.id===d.id)));
   assert.deepEqual(f.values,old,'copies never mutate their sources');
   if(cancel)f.cancel();else f.window.emit(mouse('pointerup',30,40));await tick();
   assert.equal(f.commands.length,cancel?0:1);
   if(!cancel)assert.deepEqual(f.commands[0],{expectedRevision:5,type:'duplicate_drawings',drawingIds:['s0','s1','s2'],offset:{x:20,y:30}});
   assert.equal(f.groupDraft().length,0);assert.equal(f.window.count(),0);f.dispose();
  }
 }finally{clock.restore();}
});
function selectionDeleteFixture(history = {undo:[],redo:[]}) {
 const f=selectThreeFixture(history),previous=f.props.onCommand;
 f.props.onCommand=async action=>{
  if(action.type!=='remove_drawings'&&action.type!=='remove_drawing')return previous(action);
  f.commands.push(plain(action));if(f.deleteWait)await f.deleteWait.promise;if(f.deleteError)throw Error(f.deleteError);
  assert.equal(action.expectedRevision,f.props.revision);
  const ids=new Set(action.type==='remove_drawings'?action.drawingIds:[action.drawingId]),batch=f.values.flatMap((value,index)=>ids.has(value.id)?[{index,before:plain(value),after:null}]:[]).reverse();
  for(const edit of batch)f.values.splice(edit.index,1);history.undo.push({batch});history.redo=[];f.props.revision++;
 };
 return Object.assign(f,{savedHistory:history});
}
function deleteKey(f,extra={}){
 const old=globalThis.document;globalThis.document={activeElement:null,querySelector:()=>null};
 const e={type:'keydown',key:'Delete',target:null,isComposing:false,keyCode:46,repeat:false,preventDefault(){this.prevented=true;},stopImmediatePropagation(){this.stopped=true;},...extra};
 try{f.keyboard(e);return e;}finally{globalThis.document=old;}
}
test('actual marquee followed by Delete sends one exact batch; one Undo restores every object in original order',async()=>{
 const clock=frames(),f=selectionDeleteFixture();try{
  const original=plain(f.values);selectThree(f);const e=deleteKey(f);await tick();
  assert.equal(e.prevented,true);assert.equal(e.stopped,true);assert.deepEqual(f.commands,[{expectedRevision:5,type:'remove_drawings',drawingIds:['s0','s1','s2']}]);
  assert.equal(f.values.length,0);assert.equal(f.savedHistory.undo.length,1);assert.deepEqual(plain(f.selection()),[]);
  void f.history(false);await f.drainHistory();assert.deepEqual(f.values,original);assert.equal(f.commands.at(-1).type,'undo_drawing');
  void f.history(true);await f.drainHistory();assert.equal(f.values.length,0);assert.equal(f.commands.at(-1).type,'redo_drawing');assert.deepEqual(f.errors,[]);
 }finally{f.dispose();clock.restore();}
});
test('batch Delete waits its real authority and preserves all selection and objects after a rejected save',async()=>{
 const clock=frames(),f=selectionDeleteFixture();try{
  f.deleteWait=deferred();f.deleteError='CAS changed';selectThree(f);const before=plain(f.values);f.remove();await tick();
  assert.deepEqual(plain(f.selection()),['s0','s1','s2']);assert.deepEqual(f.values,before);assert.equal(f.commands.length,1);
  f.deleteWait.resolve();await tick();assert.deepEqual(f.values,before);assert.deepEqual(plain(f.selection()),['s0','s1','s2']);assert.deepEqual(f.errors,['CAS changed']);
 }finally{f.dispose();clock.restore();}
});
test('unsupported stored multi-delete has no action and never falls back to deleting the first item',async()=>{
 const clock=frames(),f=selectionDeleteFixture();try{
  selectThree(f);f.props.port.stored=()=>f.values.map(value=>({id:value.id,kind:'vector',bounds:{x:0,y:0,width:1,height:1}}));
  f.props.port.allows=action=>action.type!=='remove_drawings';assert.equal(f.removeAction(),undefined);
  deleteKey(f);await tick();assert.equal(f.commands.length,0);assert.equal(f.values.length,3);assert.deepEqual(plain(f.selection()),['s0','s1','s2']);
 }finally{f.dispose();clock.restore();}
});
test('a stale member revokes the entire multi-delete instead of partially deleting surviving objects',async()=>{
 const clock=frames(),f=selectionDeleteFixture();try{
  selectThree(f);f.values.pop();const before=plain(f.values);assert.equal(f.removeAction(),undefined);
  deleteKey(f);await tick();assert.equal(f.commands.length,0);assert.deepEqual(f.values,before);
 }finally{f.dispose();clock.restore();}
});
test('blackboard multi-delete removes only selected ink and leaves extracted media selected for drag-to-trash',async()=>{
 const clock=frames(),f=selectionDeleteFixture();try{
  f.props.blackboard=true;const picture={id:'picture',kind:'rich',points:[{x:5,y:5},{x:55,y:85}],rich:{kind:'extracted'}};f.values.unshift(picture);selectThree(f);
  assert.deepEqual(plain(f.selection()),['picture','s0','s1','s2']);deleteKey(f);await tick();
  assert.deepEqual(f.commands,[{expectedRevision:5,type:'remove_drawings',drawingIds:['s0','s1','s2']}]);assert.deepEqual(f.values,[picture]);assert.deepEqual(plain(f.selection()),['picture']);
  assert.equal(f.removeAction(),undefined);deleteKey(f);await tick();assert.equal(f.commands.length,1);
 }finally{f.dispose();clock.restore();}
});
test('Delete in a text input, composition, locked input or active gesture cannot remove the marquee',async()=>{
 const clock=frames();try{
  for(const mode of ['input','composition','locked','gesture']){
   const f=selectionDeleteFixture();selectThree(f);let extra={};
   if(mode==='input'){const input=new module.namespace.Element();input.closest=selector=>selector.includes('textarea')?input:null;extra.target=input;}
   if(mode==='composition')extra.isComposing=true;if(mode==='locked')f.props.inputLocked=true;
   if(mode==='gesture')f.begin(mouse('pointerdown',0,0));
   deleteKey(f,extra);await tick();assert.equal(f.commands.length,0,mode);assert.equal(f.values.length,3,mode);f.dispose();
  }
 }finally{clock.restore();}
});
test('selectionGesture mouse release fallback retains a pressed marquee without including the zero-button hover and ignores late lost/up',async()=>{
 const clock=frames(),f=selectThreeFixture();try{
  selectThree(f);assert.deepEqual(plain(f.selection()),['s0','s1','s2']);assert.equal(f.marquee(),undefined);assert.equal(f.commands.length,0);assert.equal(f.hasGesture(),false);
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',900,950));assert.deepEqual(plain(f.selection()),['s0','s1','s2']);assert.deepEqual(f.errors,[]);
  f.begin(mouse('pointerdown',0,0));f.window.emit(mouse('pointermove',60,20));f.window.emit(mouse('pointermove',900,950,{buttons:0}));assert.deepEqual(plain(f.selection()),['s0'],'hover must not extend the saved rectangle to all three');
 }finally{f.dispose();clock.restore();}
});
test('selectionGesture mouse release fallback saves the last pressed group movement through one batch CAS and preserves Undo',async()=>{
 const clock=frames(),history={undo:[],redo:[]},f=selectThreeFixture(history);try{
  const old=plain(f.values);
  selectThree(f);const target=new module.namespace.Element();target.id='s0';f.begin(mouse('pointerdown',10,10,{target}));f.window.emit(mouse('pointermove',30,40));assert.equal(f.groupDraft().length,3);f.window.emit(mouse('pointermove',900,950,{buttons:0}));await tick();
  assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'update_drawings');assert.equal(f.commands[0].expectedRevision,5);assert.deepEqual(f.values.map(v=>v.points),old.map(v=>v.points.map(p=>({x:p.x+20,y:p.y+30}))));assert.equal(f.groupDraft().length,0);
  f.svg.emit(mouse('lostpointercapture',900,950,{buttons:0}));f.window.emit(mouse('pointerup',900,950));await tick();assert.equal(f.commands.length,1);
  await f.history(false);await f.drainHistory();assert.deepEqual(f.values,old);assert.equal(f.props.revision,7);assert.deepEqual(f.errors,[]);
 }finally{f.dispose();clock.restore();}
});
test('selectionGesture mouse release fallback keeps valid-source permission Busy tool and explicit cancel barriers for marquee and group',async()=>{
 const clock=frames();try{
  for(const mode of ['marquee','group'])for(const reason of ['source','revision','size','box','tool','busy','locked','blur','pointercancel','lost','pen','touch','chord','other-pointer']){
   const f=selectThreeFixture();try{
    if(mode==='group')selectThree(f);const initial=plain(f.selection()),old=plain(f.values),target=mode==='group'?Object.assign(new module.namespace.Element(),{id:'s0'}):undefined,pointerType=['pen','touch'].includes(reason)?reason:'mouse',buttons=reason==='chord'?3:1;
    f.begin(event('pointerdown',0,0,{target,pointerType,buttons}));f.window.emit(event('pointermove',30,40,{pointerType,buttons}));
    if(reason==='source')f.props.source='new';if(reason==='revision')f.props.revision++;if(reason==='size')f.props.port.sourceSize=()=>({width:999,height:1000});if(reason==='box')f.props.box.x++;if(reason==='tool')f.tool('line');if(reason==='busy')f.props.busy=true;if(reason==='locked')f.props.inputLocked=true;
    if(reason==='blur')f.blur();if(reason==='pointercancel')f.window.emit(event('pointercancel',30,40,{pointerType}));if(reason==='lost')f.svg.emit(event('lostpointercapture',30,40,{pointerType}));
    f.window.emit(event('pointermove',900,950,{pointerType,buttons:0,...(reason==='other-pointer'?{pointerId:8}:{})}));await tick();assert.equal(f.commands.length,0,mode+'/'+reason);assert.deepEqual(f.values,old,mode+'/'+reason);assert.deepEqual(plain(f.selection()),initial,mode+'/'+reason);
   }finally{f.dispose();}
  }
 }finally{clock.restore();}
});

function press(input,f,x,y,time,extra={}){const e=input.pointer('pointerdown',x,y,{isPrimary:true,timeStamp:time,target:f.svg,...extra});input.document.emit(e);f.pointerBegin(e);return e;}
function drag(input,f,x,y,time,extra={}){f.window.emit(input.pointer('pointermove',x,y,{isPrimary:true,timeStamp:time,...extra}));}
function release(input,f,x,y,time,extra={}){const e=input.pointer('pointerup',x,y,{isPrimary:true,timeStamp:time,...extra});input.document.emit(e);f.window.emit(e);return e;}
function recoveryScope(){const clock=frames(),f=setup([],undefined,true),input=mouseSurface(f);return{f,input,clock,close(){input.detach();f.dispose();f.disposeReactive();clock.restore();}};}

test('a complete ordinary mouse stroke and a repeated identical down never create an extra drawing',async()=>{
 const s=recoveryScope(),{f,input}=s;try{
  const old=press(input,f,10,10,10);drag(input,f,20,20,11);release(input,f,30,30,12);await tick();
  const next=press(input,f,70,70,20);f.pointerBegin(next);f.pointerBegin(old);drag(input,f,80,80,21);release(input,f,90,90,22);await tick();
  assert.deepEqual(f.commands.map(c=>c.drawing.points),[[{x:10,y:10},{x:20,y:20},{x:30,y:30}],[{x:70,y:70},{x:80,y:80},{x:90,y:90}]]);
  assert.equal(f.window.count(),0);assert.deepEqual(f.errors,[]);
 }finally{s.close();}
});

test('a newer primary mouse press separates missing releases without using its coordinates in the old drawing',async()=>{
 for(const compatibility of ['none','zero','stale']){
  const s=recoveryScope(),{f,input}=s;try{
   press(input,f,10,10,10);drag(input,f,20,20,11);
   if(compatibility!=='none')input.up(20,20,{buttons:compatibility==='stale'?1:0,timeStamp:12});
   press(input,f,70,70,20);drag(input,f,80,80,21);release(input,f,90,90,22);await tick();
   assert.deepEqual(f.commands.map(c=>c.drawing.points),[[{x:10,y:10},{x:20,y:20}],[{x:70,y:70},{x:80,y:80},{x:90,y:90}]],compatibility);
   assert.equal(f.window.count(),0);assert.deepEqual(f.errors,[]);
  }finally{s.close();}
 }
});

test('ordinary vector recovery publishes each prior unpainted preview and supports the blackboard through the original command path',async()=>{
 for(const tool of ['pen','line','arrow','rect','ellipse','highlighter'])for(const blackboard of [false,true]){
  const s=recoveryScope(),{f,input}=s;try{
   f.props.blackboard=blackboard;f.tool(tool);press(input,f,10,10,10);drag(input,f,20,20,11);press(input,f,70,70,20);drag(input,f,80,80,21);release(input,f,90,90,22);await tick();
   assert.equal(f.commands.length,2,tool+'/'+blackboard);assert.equal(f.commands[0].drawing.kind,tool);assert.deepEqual(f.commands[0].drawing.points,[{x:10,y:10},{x:20,y:20}]);
   assert.deepEqual(f.errors,[]);assert.equal(f.window.count(),0);
  }finally{s.close();}
 }
});

test('old-timestamp pointer events and detached old callbacks cannot finish or append to the successor',async()=>{
 const s=recoveryScope(),{f,input}=s;try{
  press(input,f,10,10,10);drag(input,f,20,20,11);
  const oldMove=[...f.window.listeners.get('pointermove')],oldUp=[...f.window.listeners.get('pointerup')],oldCancel=[...f.window.listeners.get('pointercancel')],oldLost=[...f.svg.listeners.get('lostpointercapture')];
  press(input,f,70,70,20);drag(input,f,80,80,21);
  for(const type of ['pointermove','pointerup','pointercancel','lostpointercapture']){
   const e=input.pointer(type,35,35,{isPrimary:true,timeStamp:12});if(type==='lostpointercapture')f.svg.emit(e);else{input.document.emit(e);f.window.emit(e);}
   assert.equal(f.hasGesture(),true,type+' has an old timestamp');
  }
  for(const [callbacks,type] of [[oldMove,'pointermove'],[oldUp,'pointerup'],[oldCancel,'pointercancel'],[oldLost,'lostpointercapture']])for(const callback of callbacks)callback(input.pointer(type,44,44,{isPrimary:true,timeStamp:25}));
  assert.equal(f.hasGesture(),true,'detached downstream callbacks cannot act on the successor');
  release(input,f,90,90,26);await tick();
  assert.deepEqual(f.commands.map(c=>c.drawing.points),[[{x:10,y:10},{x:20,y:20}],[{x:70,y:70},{x:80,y:80},{x:90,y:90}]]);assert.equal(f.window.count(),0);
 }finally{s.close();}
});

test('different pointer pen touch secondary chord and non-new timestamps cannot recover an ordinary mouse vector',async()=>{
 const alternatives=[{pointerId:82},{pointerType:'pen'},{pointerType:'touch'},{isPrimary:false},{button:2,buttons:2},{buttons:3},{timeStamp:9},{timeStamp:10}];
 for(const extra of alternatives){const s=recoveryScope(),{f,input}=s;try{
  press(input,f,10,10,10);drag(input,f,20,20,11);press(input,f,70,70,20,extra);await tick();assert.equal(f.commands.length,0,JSON.stringify(extra));f.cancel();assert.equal(f.window.count(),0);assert.deepEqual(f.errors,[]);
 }finally{s.close();}}
});

test('recovery preserves the actual source geometry permission busy blur cancel and disposal boundaries',async()=>{
 for(const reason of ['source','revision','geometry','permission','busy','inputLocked','tool','blur','resize','cancel','dispose']){
  const s=recoveryScope(),{f,input}=s;try{
   press(input,f,10,10,10);drag(input,f,20,20,11);
   if(reason==='source'){f.props.source='new-source';f.props.revision++;f.sourceChanged();}if(reason==='revision')f.props.revision++;
   if(reason==='geometry')f.svg.box.width++;if(reason==='permission')f.props.allowed=false;
   if(reason==='busy')f.props.busy=true;if(reason==='inputLocked')f.props.inputLocked=true;if(reason==='tool')f.tool('line');
   if(reason==='blur')f.blur();if(reason==='resize')f.resize();if(reason==='cancel')f.cancel();if(reason==='dispose')f.dispose();
   press(input,f,70,70,20);await tick();assert.equal(f.commands.length,0,reason+' cannot save the old vector');
   f.cancel();assert.equal(f.window.count(),0,reason);assert.deepEqual(f.errors,reason==='dispose'?['请先保存未完成的笔迹']:[],reason+' preserves the existing queue error');
  }finally{s.close();}
 }
});

test('mark and existing-object movement retain their active-gesture behavior without automatic recovery',async()=>{
 for(const kind of ['mark','existing']){
  const clock=frames(),values=kind==='existing'?[{id:'existing',kind:'line',color:'#eee',strokeWidth:4,points:[{x:10,y:10},{x:40,y:40}]}]:[],f=setup(values,undefined,true),input=mouseSurface(f);
  try{
   if(kind==='mark')f.tool('mark');else f.tool('select');const target=kind==='existing'?new module.namespace.Element():f.svg;if(kind==='existing')target.id='existing';
   press(input,f,10,10,10,{target});drag(input,f,20,20,11);press(input,f,70,70,20,{target});await tick();
   assert.equal(f.commands.length,0,kind);f.cancel();assert.equal(f.window.count(),0);assert.deepEqual(f.errors,[]);
  }finally{input.detach();f.dispose();f.disposeReactive();clock.restore();}
 }
});

test('delayed compatibility mouse events from a rejected old press cannot terminate a valid normal-pointer successor',async()=>{
 const s=recoveryScope(),{f,input}=s;try{
  f.props.busy=true;press(input,f,10,10,10);assert.equal(f.hasGesture(),false);release(input,f,30,30,12);f.props.busy=false;
  press(input,f,70,70,20);drag(input,f,80,80,21);input.missing(10,10,{timeStamp:11});input.up(30,30,{buttons:1,timeStamp:13});
  assert.equal(f.hasGesture(),true,'mouse compatibility release is not a pointer terminal');drag(input,f,90,90,23);release(input,f,100,100,24);await tick();
  assert.equal(f.commands.length,1);assert.deepEqual(f.commands[0].drawing.points,[{x:70,y:70},{x:80,y:80},{x:90,y:90},{x:100,y:100}]);assert.equal(f.window.count(),0);assert.deepEqual(f.errors,[]);
 }finally{s.close();}
});
