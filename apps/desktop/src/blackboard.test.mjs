// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import vm from 'node:vm';
import {parse} from '@babel/parser';
const source=await readFile(new URL('./components/SharedDrawingEditor.tsx',import.meta.url),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
const body=ast.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
const slice=name=>{const node=body.find(node=>node.type==='FunctionDeclaration'&&node.id.name===name);return source.slice(node.start,node.end);};
const pure=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const {ObjectEraser,eraserStrokeHits,clipEraserSweep}=await pure('./components/drawing-eraser.ts');
const {drawingPointerSamples}=await pure('./components/drawing-pointer.ts');
const {isBlackboard}=await pure('./blackboard.ts');
const {blackboardImage}=await pure('./blackboard-image.ts');
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(blackboard=true){
  let captured=false,revision=0,mode='pen',erasing=false;
  const listeners=new Map(),removed=[],objects=new Set(['first','second']);
  const svg={getBoundingClientRect:()=>({left:0,top:0,width:100,height:100}),addEventListener:(name,handler)=>listeners.set(name,handler),removeEventListener:name=>listeners.delete(name),setPointerCapture(){captured=true;},hasPointerCapture:()=>captured,releasePointerCapture(){captured=false;},focus(){}};
  const props={blackboard,busy:false,inputLocked:false,box:{x:0,y:0,width:100,height:100},port:{canSelect:()=>true,sourceIdentity:()=>props.source,sourceSize:()=>({width:100,height:100}),frame:()=>({x:0,y:0,width:100,height:100})},source:'original'};
  const context=vm.createContext({props,svg,ObjectEraser,eraserStrokeHits,clipEraserSweep,drawingPointerSamples,blackboardImage,eraseWidth:()=>4,document:{elementFromPoint:()=>svg},drawings:()=>[...objects].map(id=>({id,kind:id==='picture'?'rich':'pen',points:[{x:id==='first'?10:id==='second'?40:80,y:10}],strokeWidth:2,rich:id==='picture'?{kind:'extracted'}:undefined})), identity:()=>props.source,tool:()=>mode,disabled:()=>props.busy||props.inputLocked,textEditor:()=>false,edit:()=>false,finishing:()=>false,disposed:false,cancelGesture:undefined,eraseSource:undefined,eraseFlight:undefined,setErasing:value=>{erasing=value;},editableIds:()=>objects,eraserHit:(_svg,point,allowed)=>{const id=point.x<30?'first':point.x<60?'second':'picture';return allowed.has(id)?id:undefined;},target:()=>({expectedRevision:revision}),commit:async command=>{removed.push(command);objects.delete(command.drawingId);revision++;return true;}});
  vm.runInContext(stripTypeScriptTypes(slice('beginErase')+'\n'+slice('begin'),{mode:'transform'}),context);
  const event=(type,x,buttons=2)=>({type,button:2,buttons,pointerId:1,clientX:x,clientY:10,preventDefault(){},stopPropagation(){}});
  return {context,props,removed,objects,event,mode:value=>{mode=value;},erasing:()=>erasing,fire:(type,x,buttons)=>listeners.get(type)?.(event(type,x,buttons)),begin:()=>context.begin(event('pointerdown',10)),listenerCount:()=>listeners.size};
}
test('blackboard secondary button removes objects while preserving the active pen tool and releases cleanly',async()=>{
  const f=fixture();f.begin();await tick();assert.equal(f.removed.length,1);assert.equal(f.erasing(),true);
  f.fire('pointermove',40,2);await tick();assert.equal(f.removed.length,2);assert.equal(f.removed[1].expectedRevision,1);
  f.fire('pointerup',40,0);assert.equal(f.erasing(),false);assert.equal(f.listenerCount(),0);assert.equal(f.context.tool(),'pen');
});
test('secondary eraser requires the right-button bit, and left-only movement cannot continue deleting',async()=>{
  const f=fixture();f.begin();await tick();f.fire('pointermove',40,1);await tick();assert.equal(f.removed.length,1);assert.equal(f.erasing(),false);
});
test('secondary eraser stops on lost capture, input lock or source replacement without touching successor objects',async()=>{
  for(const change of [f=>f.fire('lostpointercapture',40,2),f=>{f.props.inputLocked=true;f.fire('pointermove',40,2);},f=>{f.props.source='replacement';f.fire('pointermove',40,2);}]){
    const f=fixture();f.begin();await tick();change(f);await tick();assert.equal(f.removed.length,1);assert.equal(f.erasing(),false);
  }
});
test('screenshot drawing uses the same temporary right-button eraser without changing its selected pen',async()=>{
  const f=fixture(false);f.begin();await tick();assert.equal(f.removed.length,1);assert.equal(f.context.tool(),'pen');f.fire('pointerup',10,0);assert.equal(f.listenerCount(),0);
});

test('released eraser detaches capture immediately but exit flush waits for all already accepted hits',async()=>{
  const f=fixture();let resume;const gate=new Promise(resolve=>resume=resolve),original=f.context.commit;let requests=0;
  f.context.commit=async(...args)=>{if(++requests===1)await gate;return original(...args);};
  f.begin();f.fire('pointermove',40,2);f.fire('pointerup',40,0);
  assert.equal(f.erasing(),false);assert.equal(f.listenerCount(),0);assert.equal(requests,1);
  f.props.inputLocked=true;f.context.cancelGesture();f.fire('lostpointercapture',40,0);
  let finished=false;const flushing=f.context.eraseFlight.then(()=>finished=true);await tick();assert.equal(finished,false);
  resume();await flushing;assert.deepEqual(f.removed.map(v=>v.drawingId),['first','second']);assert.deepEqual(f.removed.map(v=>v.expectedRevision),[0,1]);
});
test('real JSX context-menu handler suppresses browser menu only for a blackboard',()=>{
  const nodes=[];const walk=node=>{if(!node||typeof node!=='object')return;if(node.type)nodes.push(node);for(const value of Object.values(node)){if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}};walk(ast);
  const attribute=nodes.find(node=>node.type==='JSXAttribute'&&node.name.name==='onContextMenu');
  const callback=stripTypeScriptTypes('const callback='+source.slice(attribute.value.expression.start,attribute.value.expression.end),{mode:'transform'});
  for(const enabled of [false,true]){let prevented=0;const run=vm.runInNewContext(callback+'\ncallback',{props:{blackboard:enabled}});run({preventDefault(){prevented++;},stopPropagation(){prevented++;}});assert.equal(prevented,enabled?2:0);}
});
test('blackboard presentation recognizes only generated PNG identities, not ordinary imported images',()=>{
  const make=path=>({background:{id:'asset',kind:'image',name:'黑板.png',path}});
  assert.equal(isBlackboard(make('D:\\assets\\asset.board.png')),true);
  assert.equal(isBlackboard(make('data:image/png;base64,generated')),true);
  assert.equal(isBlackboard(make('D:/imports/黑板.png')),false);
  assert.equal(isBlackboard(make('D:/assets/other.board.png')),false);
});

const appSource=await readFile(new URL('./App.tsx',import.meta.url),'utf8');
const appBody=parse(appSource,{sourceType:'module',plugins:['typescript','jsx']}).program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
const opening=appBody.find(node=>node.type==='FunctionDeclaration'&&node.id.name==='openBlackboard');
function closeFixture(flushDrawing,finishBlackboard=async()=>({activeSceneId:'parent'})){
  let editing='board',busy=false,operations=0;const errors=[];
  const context=vm.createContext({scene:()=>({id:'board'}),busy:()=>busy,sending:()=>false,recording:()=>false,scroll:()=>false,exitPreparing:()=>false,disposed:false,isBlackboard:()=>true,blackboardEditing:()=>editing,setBlackboardEditing:value=>{editing=value;},spaceOperations:{begin:()=>{operations++;return()=>{operations--;};}},setBusy:value=>{busy=value;},setReferenceDrafts:()=>{},withoutScene:()=>{},flushDrawing,bridge:{finishBlackboard},accept:()=>{},showError:error=>errors.push(error.message)});
  vm.runInContext(stripTypeScriptTypes(appSource.slice(opening.start,opening.end),{mode:'transform'}),context);
  return {close:()=>context.openBlackboard(),editing:()=>editing,busy:()=>busy,operations:()=>operations,errors};
}
test('closing blackboard authoring waits for its real draft flush before removing the editor',async()=>{
  let settle;const gate=new Promise(resolve=>{settle=resolve;}),f=closeFixture(()=>gate),closing=f.close();
  assert.equal(f.editing(),'board');assert.equal(f.busy(),true);assert.equal(f.operations(),1);
  settle();await closing;assert.equal(f.editing(),undefined);assert.equal(f.busy(),false);assert.equal(f.operations(),0);
});
test('failed blackboard draft flush keeps the editor and reports the error',async()=>{
  const f=closeFixture(async()=>{throw Error('保存失败');});await f.close();
  assert.equal(f.editing(),'board');assert.deepEqual(f.errors,['保存失败']);assert.equal(f.busy(),false);assert.equal(f.operations(),0);
});
test('preview save must finish before exiting and failed materialization keeps authoring mounted',async()=>{
  let settle;const gate=new Promise(resolve=>{settle=resolve;}),f=closeFixture(async()=>{},()=>gate),closing=f.close();
  await tick();assert.equal(f.editing(),'board');assert.equal(f.busy(),true);settle({activeSceneId:'parent'});await closing;assert.equal(f.editing(),undefined);
  const failure=closeFixture(async()=>{},async()=>{throw Error('图片保存失败');});await failure.close();assert.equal(failure.editing(),'board');assert.deepEqual(failure.errors,['图片保存失败']);assert.equal(failure.operations(),0);
});
test('successful board finish removes only the parent reference draft so automatic image references remain visible',async()=>{
  const node=appBody.find(node=>node.type==='FunctionDeclaration'&&node.id.name==='finishBlackboard');
  let references={parent:[],other:[{kind:'item',id:'keep'}]},current={id:'board'},busy=false;
  const context=vm.createContext({scene:()=>current,isBlackboard:()=>true,busy:()=>busy,sending:()=>false,exitPreparing:()=>false,disposed:false,
    spaceOperations:{begin:()=>()=>{}},setBusy:value=>{busy=value;},flushDrawing:async()=>{},
    bridge:{finishBlackboard:async()=>({activeSceneId:'parent',scenes:[{id:'parent',refs:[{kind:'item',id:'board-image'}]}]})},
    setReferenceDrafts:update=>{references=update(references);},withoutScene:(old,id)=>{const next={...old};delete next[id];return next;},
    accept:next=>{current=next.scenes[0];},setBlackboardEditing:()=>{},setFocusRequest:()=>{},showError:error=>{throw error;}});
  vm.runInContext(stripTypeScriptTypes(appSource.slice(node.start,node.end),{mode:'transform'}),context);
  assert.equal(await context.finishBlackboard(),true);assert.equal(references.parent,undefined);assert.deepEqual(references.other,[{kind:'item',id:'keep'}]);assert.deepEqual(current.refs,[{kind:'item',id:'board-image'}]);assert.equal(busy,false);
});

test('secondary eraser leaves board images intact and removes all crossed ink',async()=>{const f=fixture();f.objects.add('picture');f.begin();await tick();f.fire('pointermove',80,2);await tick();assert.equal(f.removed.length,2);assert.ok(f.objects.has('picture'));f.fire('pointerup',80,0);});
