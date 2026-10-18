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
const pointer = await pure('./components/drawing-pointer.ts'), geometry = await pure('./components/drawing-geometry.ts'), properties = await pure('./components/drawing-properties.ts'), highlightSource = await pure('./highlight-source.ts');
const source = await raw('./components/SharedDrawingEditor.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(value => value.type === 'ExportDefaultDeclaration').declaration.body.body;
const declaration = name => { const node = body.find(value => value.type === 'FunctionDeclaration' ? value.id.name === name : value.type === 'VariableDeclaration' && value.declarations.some(declaration => declaration.id.name === name)); assert.ok(node, name); return source.slice(node.start, node.end); };
const actual = ['cacheKey', 'strokeSource', 'continuousStrokes', 'gestureIdentity', 'point', 'target', 'editableIds', 'cachedStrokes', 'restoreStrokeDraft', 'newStrokeQueue', 'strokeCacheKey', 'releaseFailedStrokes', 'storeEdit', 'cancelEdit', 'commit', 'saveText', 'loadLatest', 'confirmText', 'begin', 'choose', 'closeEditor', 'blur', 'resize'].map(declaration).join('\n');
const module = new SourceTextModule(transform(`
import { DrawingAddQueue,drawingFrame,drawingPointerSamples,settleDrawingEdit,drawingMoveDelta,drawingBounds,constrainedEnd,validNextNumber,t,blackboardImage,propertyDraft,drawingDrafts,strokeKinds,batch } from 'deps';
export class Element { closest() { return {getAttribute:()=>this.id}; } }
export function fixture(props,svg,window) {
 let disposed=false,serial=0,flight,cancelGesture,refreshConstraint,toolValue='pen',selectedId='',draftValue,pendingValue=false,strokeQueue,editCacheKey,editValue,finishingValue=false;
 let queued=[],recovered=[];const queuedDrawings=()=>queued,setQueuedDrawings=value=>queued=value,setRecoveredDrawings=value=>recovered=value,strokeVersion=()=>0,setStrokeVersion=()=>{};
 let selectedMany=[];const selectedIds=()=>selectedMany,setSelectedIds=value=>selectedMany=value;
 const publications=[],tool=()=>toolValue,setTool=value=>toolValue=value,selected=()=>selectedId,setSelected=value=>selectedId=value;
 const draft=()=>draftValue,setDraft=value=>{draftValue=value;if(value)publications.push(JSON.parse(JSON.stringify(value)));},pending=()=>pendingValue,setPending=value=>pendingValue=value;
 const edit=()=>editValue,setEdit=value=>editValue=value,textEditor=()=>undefined,finishing=()=>finishingValue,setFinishing=value=>finishingValue=value,disabled=()=>pending()||finishing()||props.busy||props.inputLocked;
 const drawings=()=>props.port.drawings(),identity=()=>cacheKey(),gestureSource=()=>gestureIdentity();
 const color=()=>'#ee4848',width=()=>4,highlightWidth=()=>18,mosaicReady=()=>true,blockSize=()=>8,fontSize=()=>20,textStyle=()=>props.textStyleValue??{};
 const report=error=>props.onError(error.message),clamp=(v,low,high)=>Math.max(low,Math.min(high,v));
 let openedText;const openText=value=>openedText=value,ensureSelection=async()=>{},beginStored=()=>{},beginErase=()=>{};
 const nextNumber=()=>'1',numberDiameter=()=>28,numberChanges=new Map(),numberPreference=0,setNextNumber=()=>{};
 const setWideBrush=()=>{},mosaicFailed=()=>false,setMosaicRetry=()=>{},highlightFailed=()=>false,setHighlightRetry=()=>{};
 const setViewport=()=>{},innerWidth=1200,innerHeight=800;
 ${actual}
 strokeQueue=continuousStrokes()?newStrokeQueue():undefined;
 return {begin,draft,publications,tool:setTool,selected:setSelected,cancel:()=>cancelGesture?.(),blur,resize,choose,closeEditor,finishing,confirmText,loadLatest,saveText,
   dispose:()=>{disposed=true;strokeQueue?.stop();cancelGesture?.();},hasGesture:()=>!!cancelGesture,shift:value=>refreshConstraint?.(value),queued:()=>queued,edit,queue:()=>strokeQueue,opened:()=>openedText};
}`));
await module.link(() => synthetic({ ...pointer, ...geometry, ...properties,batch:solid.batch, t: value => value,blackboardImage:d=>d.rich?.kind==='extracted' })); await module.evaluate();
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
function setup(values = []) {
  const window = new Events(), svg = new Events(), commands = [], errors = [];
  let layoutReads = 0;
  svg.box = { left: 10, top: 20, width: 2000, height: 500 };
  svg.getBoundingClientRect = () => { layoutReads++; return { ...svg.box }; }; svg.focus = () => {};
  svg.setPointerCapture = id => { svg.captured = id; }; svg.hasPointerCapture = id => svg.captured === id;
  svg.releasePointerCapture = id => { svg.captured = undefined; svg.emit(event('lostpointercapture', 0, 0, { pointerId: id })); };
  const props = { box: { x: 10, y: 20, width: 2000, height: 500 }, tools: ['pen','highlighter','mark','line','arrow','rect','ellipse','mosaic'],busy:false,inputLocked:false,revision:5,source:'source',
    port: { frame:()=>({x:0,y:0,width:1000,height:1000}),sourceSize:()=>({width:1000,height:1000}),sourceIdentity:()=>props.source,draftKey:()=>JSON.stringify(['pointer-scene-'+fixtureSerial,'region',props.source]),revision:()=>props.revision,drawings:()=>values,canSelect:()=>true,allows:()=>props.allowed!==false,propertyCommand:draft=>({type:draft.base?'update_drawing':'add_drawing',drawing:plain(draft.drawing),expectedRevision:draft.expectedRevision}) },
    onCommand:async command=>{commands.push(plain(command));if(command.type==='add_drawing'){values.push(plain(command.drawing));props.revision++;}},onClose:()=>{},onError:error=>errors.push(error) };
  const fixtureId=++fixtureSerial;props.port.draftKey=()=>JSON.stringify(['pointer-scene-'+fixtureId,'region',props.source]);
  return { ...module.namespace.fixture(props,svg,window), props, svg, window, commands, errors, reads:()=>layoutReads };
}

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
    for(const key of inserted)cache.delete(key);inserted.length=0;
    const canceled=setup();for(let i=0;i<16;i++){canceled.begin(event('pointerdown',10,20));assert.equal(canceled.hasGesture(),true);canceled.cancel();assert.equal(canceled.hasGesture(),false);}
    canceled.begin(event('pointerdown',10,20));canceled.window.emit(event('pointerup',30,40));await canceled.queue().drain();assert.equal(canceled.commands.length,1);
    const bounded=new properties.DrawingDraftCache(3,1500),release=bounded.reserve('reserved',1200);
    assert.throws(()=>bounded.reserve('second',500),/过多/);assert.throws(()=>bounded.set('large',properties.propertyDraft({...drawing,points:Array.from({length:100},(_,i)=>({x:i,y:i}))},0,'style',false)),/过多/);
    bounded.set('reserved',properties.propertyDraft(drawing,0,'style',false));release();
    const second=bounded.reserve('second',500);second();assert.equal(bounded.list().length,1);
  }finally{for(const key of inserted)properties.drawingDrafts.delete(key);clock.restore();}
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
${['drawingIndex','storedIndex','activeDraftId'].map(declaration).join('\n')}
const previewDrawing=()=>draft(),groupDraft=()=>[],selectedIds=()=>[selected()],queuedDrawings=()=>state.queued?.()??[],recoveredDrawings=()=>state.recovered?.()??[];${declaration('previews')}
const pixelIndex=()=>state.pixelIndex?.()??new Map();
${declaration('DrawingItem')};return <DrawingItem id={props.id}/>;}`,{pointer,selection:{linkedRepairPreview:(drawing)=>drawing}});
const descendants=(node,tag)=>[...(node.tag===tag?[node]:[]),...node.children.flatMap(child=>descendants(child,tag))];

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
    assert.deepEqual(swatches(),['#E8ECF4','#EE4848','#348BF1']);assert.equal(writes.length,0);
    assert.equal(descendants(panelRecent(),'button').length,1,'fallback swatches never become used colors');
    const hue=descendants(root,'input').find(node=>node.props.class==='drawing-color-hue');
    hue.props.onInput({currentTarget:{value:'120'}});hue.props.onInput({currentTarget:{value:'240'}});
    assert.equal(writes.length,0);assert.ok(changes.every(change=>change.submit===false));
    const final=value();hue.props.onChange();
    assert.deepEqual(JSON.parse(stored.get(colors.colorHistoryKey)),[final,'#E8ECF4','#123456']);
    assert.equal(writes.filter(write=>write.key===colors.colorHistoryKey).length,1);assert.equal(changes.at(-1).submit,true);
    assert.deepEqual(swatches(),[final,'#EE4848','#348BF1']);
    stored.set(colors.colorHistoryKey,JSON.stringify(['#ABCDEF']));window.emit({type:'storage',key:colors.colorHistoryKey});
    assert.deepEqual(swatches(),[final,'#EE4848','#348BF1']);
    stored.delete(colors.colorHistoryKey);window.emit({type:'storage',key:null});assert.deepEqual(swatches(),[final,'#EE4848','#348BF1']);
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
  const props={drawing,get context(){return{sceneId:'scene',background:asset,region:{id:'region',x:0,y:0,width:500,height:400,drawingRevision:revision()}};}};
  const dispose=renderer.render(()=>renderer.createComponent(rich.default,props),root);
  await tick();jobs[0].resolve({...jobs[0].target,dataUrl:'synthetic-pixels'});await tick();const node=descendants(root,'image')[0];assert.ok(node);
  for(let i=1;i<=8;i++){setRevision(i);await tick();assert.equal(descendants(root,'image')[0],node);jobs[i].resolve({...jobs[i].target,dataUrl:'synthetic-pixels'});await tick();assert.equal(descendants(root,'image')[0],node);}
  setRevision(9);await tick();jobs[9].reject(Error('native rejected current object'));await tick();assert.equal(descendants(root,'image').length,0);dispose();
});
