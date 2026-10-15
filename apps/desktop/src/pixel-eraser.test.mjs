// SPDX-License-Identifier: MPL-2.0
// Actual geometry + actual editor pointer handlers; synthetic DOM/IPC only.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import vm from 'node:vm';
import {parse} from '@babel/parser';
const source=await readFile(new URL('./components/SharedDrawingEditor.tsx',import.meta.url),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
const body=ast.program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration.body.body;
const code=name=>{const n=body.find(n=>n.type==='FunctionDeclaration'&&n.id.name===name);return source.slice(n.start,n.end);};
const pure=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const {eraseStroke,PixelEraser,clipEraserSweep}=await pure('./components/drawing-eraser.ts');
const {drawingPointerSamples}=await pure('./components/drawing-pointer.ts');
const plain=v=>JSON.parse(JSON.stringify(v));
const rounded=v=>JSON.parse(JSON.stringify(v,(_key,value)=>typeof value==='number'?Math.round(value*1e9)/1e9:value));
const stroke=(id='ink',kind='pen',points=[{x:0,y:50},{x:100,y:50}])=>({id,kind,color:'#123456',strokeWidth:4,points});
test('pixel sweep preserves both sides and deletes the local middle, rather than the whole object',()=>{
  const original=stroke(),e=new PixelEraser([original],()=> 'fragment');
  assert.equal(e.sweep({x:50,y:20},{x:50,y:80},10),true);
  const cut=rounded(e.erasures()[0]);
  assert.deepEqual(cut,{drawingId:'ink',fragments:[{id:'ink',points:[{x:0,y:50},{x:43,y:50}]},{id:'fragment',points:[{x:57,y:50},{x:100,y:50}]}]});
  assert.deepEqual(original.points,[{x:0,y:50},{x:100,y:50}]);
});
test('dot is erased on the initial press; zero movement, fast sweeps and zero-length segments are defined',()=>{
  assert.deepEqual(plain(eraseStroke([{x:50,y:50}],{x:50,y:50},{x:50,y:50},4,4)),[]);
  assert.deepEqual(plain(eraseStroke([{x:50,y:50},{x:50,y:50}],{x:50,y:50},{x:50,y:50},4,4)),[]);
  const e=new PixelEraser([stroke('dot','highlighter',[{x:50,y:50}])],()=> 'new');
  e.sweep({x:0,y:50},{x:100,y:50},4);assert.deepEqual(plain(e.erasures()),[{drawingId:'dot',fragments:[]}]);
});
test('repeated cuts preserve highlighter/line style, survive a cut of the first piece and keep IDs unique',()=>{
  let sequence=0;const original=stroke('ink','highlighter'),e=new PixelEraser([original],()=>`fragment-${++sequence}`);
  e.sweep({x:50,y:40},{x:50,y:60},10);e.sweep({x:80,y:40},{x:80,y:60},4);e.sweep({x:0,y:50},{x:44,y:50},2);
  const values=e.preview().get('ink');assert.equal(values.length,2);assert.equal(values[0].id,'ink');assert.equal(new Set(values.map(d=>d.id)).size,2);
  assert.ok(values.every(d=>d.color===original.color&&d.kind==='highlighter'&&d.strokeWidth===4));
  assert.ok(Math.abs(values[0].points[0].x-57)<1e-9);assert.ok(Math.abs(values[0].points.at(-1).x-76)<1e-9);
  const line=new PixelEraser([stroke('line','line')],()=> 'line-fragment');line.sweep({x:50,y:50},{x:50,y:50},4);assert.ok(line.erasures()[0].fragments.every(f=>f.points.length===2));
});
test('continuous polyline corners retain connectivity; distant and tangent sweeps do not manufacture edits',()=>{
  const points=[{x:0,y:0},{x:100,y:0},{x:100,y:100}];
  assert.deepEqual(plain(eraseStroke(points,{x:50,y:50},{x:60,y:50},4,4)),[points]);
  assert.deepEqual(plain(eraseStroke([{x:0,y:0},{x:100,y:0}],{x:50,y:4},{x:50,y:4},4,4)),[[{x:0,y:0},{x:100,y:0}]]);
  const parts=eraseStroke(points,{x:50,y:-10},{x:50,y:10},4,4);assert.equal(parts.length,2);assert.deepEqual(plain(parts[1]),[{x:54,y:0},{x:100,y:0},{x:100,y:100}]);
});
test('pixel mode excludes shapes/text/rich media; object mode remains the explicit way to erase a shape',()=>{
  const inputs=['rect','ellipse','arrow','text','mosaic','rich'].map(kind=>stroke(kind,kind));const e=new PixelEraser(inputs,()=> 'never');
  assert.equal(e.sweep({x:0,y:50},{x:100,y:50},64),false);assert.deepEqual(plain(e.erasures()),[]);
  assert.ok(source.includes("title={t('局部擦除画笔、直线和荧光笔')}"));assert.ok(source.includes("title={t('整项擦除绘制对象')}"));
});
test('fragment and document budgets fail visibly without a partially persisted gesture',()=>{
  const e=new PixelEraser(Array.from({length:256},(_,i)=>stroke(`ink-${i}`)),()=> 'extra');
  assert.throws(()=>e.sweep({x:50,y:40},{x:50,y:60},4),/片段过多/);
  assert.deepEqual(plain(e.erasures()),[],'a rejected sweep leaves no partial draft');
  assert.throws(()=>eraseStroke(stroke().points,{x:NaN,y:0},{x:0,y:0},4,4),/几何无效/);
});

function fixture(){
  const listeners=new Map(),frames=new Map(),commands=[],errors=[];let frameId=0,preview,erasing=false,captured=false,revision=7,tool='pen',mode='pixel',sourceId='source',allowHit=true,disposed=false;
  const drawings=[stroke(),stroke('dot','pen',[{x:25,y:50}]),{...stroke('media','rich'),rich:{kind:'extracted'}}];
  const props={documentOnly:false,busy:false,inputLocked:false,box:{x:0,y:0,width:100,height:100},port:{pixelEraser:true,revision:()=>revision,frame:()=>({x:0,y:0,width:100,height:100}),sourceIdentity:()=>sourceId,sourceSize:()=>({width:100,height:100})}};
  const svg={getBoundingClientRect:()=>({left:0,top:0,width:100,height:100}),contains:()=>false,addEventListener:(n,f)=>listeners.set(n,f),removeEventListener:n=>listeners.delete(n),setPointerCapture:()=>{captured=true;},hasPointerCapture:()=>captured,releasePointerCapture:()=>{captured=false;}};
  let resolveCommit;
  const context=vm.createContext({props,svg,PixelEraser,clipEraserSweep,drawingPointerSamples,crypto:{randomUUID:()=>`fragment-${++frameId}`},document:{elementFromPoint:()=>allowHit?svg:{}},drawingFrame:publish=>{let id;return {request(){if(id!==undefined)return;id=++frameId;frames.set(id,()=>{id=undefined;publish();});},cancel(){frames.delete(id);id=undefined;}};},drawings:()=>drawings,identity:()=>sourceId,tool:()=>tool,eraserMode:()=>mode,eraseWidth:()=>4,finishing:()=>false,get disposed(){return disposed;},setErasing:v=>erasing=v,setPixelDraft:v=>preview=v,report:e=>errors.push(String(e)),target:expectedRevision=>({expectedRevision}),eraseSource:undefined,cancelGesture:undefined,eraseFlight:undefined,commit:command=>{commands.push(plain(command));return new Promise(resolve=>resolveCommit=resolve);}});
  vm.runInContext(stripTypeScriptTypes(code('beginPixelErase'),{mode:'transform'}),context);
  const event=(type,x,y=50,buttons=2)=>({type,clientX:x,clientY:y,pointerId:1,button:2,buttons,preventDefault(){}});
  return {props,commands,errors,context,drawings,event,begin:(x=50)=>context.beginPixelErase(event('pointerdown',x),true),fire:(type,x,y=50,buttons=2)=>listeners.get(type)?.(event(type,x,y,buttons)),frames:()=>{for(const callback of frames.values())callback();frames.clear();},preview:()=>preview,erasing:()=>erasing,resolve:success=>resolveCommit(success),source:v=>sourceId=v,revision:()=>revision++,covered:()=>allowHit=false,disposed:()=>disposed=true,listeners};
}
test('actual pointer handler publishes local fragments at RAF cadence and sends one atomic revision on release',async()=>{
  const f=fixture();f.begin();f.fire('pointermove',60);f.fire('pointermove',70);assert.equal(f.commands.length,0);assert.equal(f.preview(),undefined);f.frames();assert.equal(f.preview().get('ink').length,2);
  f.fire('pointerup',70,50,0);assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'erase_drawing_strokes');assert.equal(f.commands[0].expectedRevision,7);assert.equal(f.listeners.size,0);assert.equal(f.erasing(),false);
  f.props.inputLocked=true;f.context.cancelGesture();let settled=false;const pending=f.context.eraseFlight.then(()=>settled=true);await Promise.resolve();assert.equal(settled,false);f.resolve(true);await pending;assert.equal(f.preview(),undefined);
});
test('actual pointer handler initial right press erases dot, protects media and keeps the selected pen',async()=>{
  const f=fixture();f.begin(25);f.fire('pointerup',25,50,0);assert.equal(f.commands.length,1);const erasures=f.commands[0].erasures;assert.ok(erasures.some(e=>e.drawingId==='dot'&&e.fragments.length===0));assert.ok(!erasures.some(e=>e.drawingId==='media'));f.resolve(true);await f.context.eraseFlight;
});
test('source/revision/lock/lost-capture invalidation rolls back the whole pending pixel preview',()=>{
  for(const invalidate of [f=>f.source('next'),f=>f.revision(),f=>f.props.inputLocked=true,f=>f.covered()]){
    const f=fixture();f.begin();invalidate(f);f.fire('pointermove',60);f.fire('pointercancel',60);assert.equal(f.commands.length,0);assert.equal(f.preview(),undefined);assert.equal(f.listeners.size,0);
  }
  const f=fixture();f.begin();f.frames();f.fire('lostpointercapture',50);assert.equal(f.commands.length,0);assert.equal(f.preview(),undefined);
});
test('actual release persistence failure is retained for the exit flush instead of reporting success',async()=>{
  const f=fixture();f.begin();f.fire('pointerup',50,50,0);const pending=f.context.eraseFlight;f.resolve(false);await assert.rejects(pending,/尚未保存/);assert.equal(f.preview(),undefined);
});
