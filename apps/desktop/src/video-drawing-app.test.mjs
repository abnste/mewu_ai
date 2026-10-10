// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
import vm from 'node:vm';
import test from 'node:test';
const source=await readFile(new URL('./App.tsx',import.meta.url),'utf8');
const core=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./core-drawing.ts',import.meta.url),'utf8'),{mode:'transform'})).toString('base64')}`);
const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
function productFunction(name){let found;const walk=node=>{if(!node||typeof node!=='object')return;if(node.type==='FunctionDeclaration'&&node.id?.name===name)found=node;for(const v of Object.values(node))if(Array.isArray(v))v.forEach(walk);else if(v&&typeof v==='object')walk(v);};walk(ast);assert.ok(found,name);return stripTypeScriptTypes(source.slice(found.start,found.end),{mode:'transform'});}
const deferred=()=>{let resolve,reject;const promise=new Promise((y,n)=>{resolve=y;reject=n;});return{promise,resolve,reject};};
const turn=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(){
 const item={id:'item',asset:{id:'source',kind:'video'},videoAnnotations:{revision:3}};
 const scene={id:'scene',items:[item]},grant=core.videoDrawingGrant();
 const target={sceneId:'scene',itemId:'item',sourceId:'source',expectedRangeRevision:0,expectedAnnotationRevision:3};
 const action={type:'add',content:{kind:'pen',version:1,sourcePoints:[{x:1,y:1},{x:2,y:2}],color:'#ffffff',strokeWidth:2}};
 const f={scene,grant,target,action,exiting:false,recording:false,calls:[],accepted:[],operations:0,receiptChecks:[],remaining:false};
 const scope={disposed:false,commands:Promise.resolve(),exitFailure:undefined,videoDrawingFlushDepth:0,videoDrawingCommits:new Set(),
   validVideoDrawingGrant:core.validVideoDrawingGrant,coreVideoDrawingTools:core.coreVideoDrawingTools,
   exitPreparing:()=>f.exiting,recording:()=>f.recording,scroll:()=>false,scene:()=>f.scene,
   pluginSnapshot:()=>({plugins:[{manifest:{id:grant.pluginId,contributions:[{id:grant.contributionId,kind:'artifact.video-drawing-tools',tools:['pen']}]},revision:grant.revision,state:f.disabled?'disabled':'enabled'}]}),
   videoAnnotationTarget:(sceneId,item)=>({sceneId,itemId:item.id,sourceId:item.asset.id,expectedRangeRevision:item.videoEdit?.revision??0,expectedAnnotationRevision:item.videoAnnotations?.revision??0}),
   sameVideoAnnotationTarget:(a,b)=>JSON.stringify(a)===JSON.stringify(b),
   spaceOperations:{begin(){f.operations++;let done=false;return()=>{assert.equal(done,false);done=true;f.operations--;};}},
   videoDrawingBridge:{async applyVideoDrawing(...args){f.calls.push(args);if(f.nativeWait)return f.nativeWait.promise;return{revision:8,scenes:[structuredClone(f.scene)]};}},
   acceptVideoDrawingReceipt:(receipt,sceneId,before,action)=>{f.receiptChecks.push({receipt,sceneId,before,action});if(f.invalidReceipt)throw Error('视频绘制回执不一致');},
   accept:receipt=>f.accepted.push(receipt),
   videoDrawingFlush:{async flush(active){assert.ok(active());if(f.flush)await f.flush(active);}},
   assertVideoDrawingDraftsSaved:sceneId=>{f.assertScope=sceneId;if(f.remaining)throw Error('还有未保存视频标注');},
   TrimCanceled:class TrimCanceled extends Error {},
 };
 vm.createContext(scope);vm.runInContext(`${productFunction('applyVideoDrawing')}\n${productFunction('flushVideoDrawing')}\n${productFunction('applyVideoDrawingDocument')}\n${productFunction('applyVideoDrawingText')}`,scope);
 return{f,scope};
}
test('actual App blocks new authoring during exit, permits only controlled RUNNING flush and releases its tracked operation',async()=>{
 const {f,scope}=fixture();f.exiting=true;
 await assert.rejects(scope.applyVideoDrawing('request',f.target,f.grant,f.action),/当前/);assert.equal(f.calls.length,0);
 scope.videoDrawingFlushDepth=1;const receipt=await scope.applyVideoDrawing('request',f.target,f.grant,f.action);
 assert.equal(f.accepted[0],receipt);assert.equal(f.receiptChecks[0].before,f.scene.items[0]);assert.equal(f.operations,0);assert.equal(scope.videoDrawingCommits.size,0);
});
test('queued source, freeze and exact core authority are rechecked before native authoring',async()=>{
 for(const change of [f=>{f.scene.items[0].asset.id='other';},f=>{f.scene.frozen=true;},f=>{f.grant.revision++;},f=>{f.grant.tools=['pen'];},f=>{f.grant.contributionId='foreign';}]){
  const {f,scope}=fixture(),wait=deferred();scope.commands=wait.promise;
  const pending=scope.applyVideoDrawing('request',f.target,f.grant,f.action);const rejected=assert.rejects(pending,/已变化/);
  change(f);wait.resolve();await rejected;assert.equal(f.calls.length,0);assert.equal(f.operations,0);
 }
});
test('actual App retains core video authoring when the legacy drawing plugin is absent or disabled',async()=>{
 for(const plugins of [[],[{manifest:{id:'mewu.drawing'},state:'disabled',revision:99}]]){
  const {f,scope}=fixture();scope.pluginSnapshot=()=>({plugins});
  await scope.applyVideoDrawing('request',f.target,f.grant,f.action);
  assert.equal(f.calls.length,1);assert.deepEqual(f.calls[0][2],core.videoDrawingGrant());assert.equal(f.operations,0);
 }
});
test('actual App validates the exact native receipt before publishing, with no guessed document or IDs',async()=>{
 const {f,scope}=fixture();f.invalidReceipt=true;
 await assert.rejects(scope.applyVideoDrawing('request',f.target,f.grant,f.action),/回执/);
 assert.equal(f.accepted.length,0);assert.equal(f.operations,0);assert.equal(scope.videoDrawingCommits.size,0);
});
test('authoring flush waits for the real command and checks unmounted dirty drafts before preparation',async()=>{
 const {f,scope}=fixture();f.exiting=true;f.nativeWait=deferred();f.flush=async()=>{void scope.applyVideoDrawing('request',f.target,f.grant,f.action);};
 let complete=false;const flush=scope.flushVideoDrawing(()=>true,'scene').then(()=>{complete=true;});
 await turn();assert.equal(complete,false);assert.equal(f.operations,1);assert.equal(scope.videoDrawingFlushDepth,1);
 const receipt={revision:9,scenes:[structuredClone(f.scene)]};f.nativeWait.resolve(receipt);await flush;
 assert.equal(f.accepted[0],receipt);assert.equal(f.assertScope,'scene');assert.equal(scope.videoDrawingFlushDepth,0);
 f.flush=undefined;f.remaining=true;await assert.rejects(scope.flushVideoDrawing(()=>true),/未保存/);assert.equal(scope.videoDrawingFlushDepth,0);
});
test('existing document and text adapters return their captured native Snapshot, never a later global snapshot',async()=>{
 const {f,scope}=fixture(),exact={revision:7,scenes:[structuredClone(f.scene)]};
 scope.videoDrawingFlushDepth=1;
 scope.applyVideoAnnotation=async(_target,_action,receive,controlled)=>{assert.equal(controlled,true);receive(exact);return{unrelated:'SpaceItem'};};
 scope.applyVideoAnnotationText=async(_id,_target,_object,_ref,_content,receive,controlled)=>{assert.equal(controlled,true);receive(exact);return{unrelated:'SpaceItem'};};
 assert.equal(await scope.applyVideoDrawingDocument(f.target,{type:'move'}),exact);
 assert.equal(await scope.applyVideoDrawingText('r',f.target,'id',{},{}),exact);
});
