import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
import test from 'node:test';
const modules=new Map(),calls=[];let tauri=true,reply;
const shim=new SyntheticModule(['invoke','isTauri'],function(){this.setExport('isTauri',()=>tauri);this.setExport('invoke',async(name,args)=>{calls.push({name,args});return typeof reply==='function'?reply(name,args):reply;});});
async function load(file,parent=import.meta.url){const alias=file.startsWith('@product/'),name=alias?`./${file.slice(9)}`:file,url=new URL(name.endsWith('.ts')?name:`${name}.ts`,alias?import.meta.url:parent),key=url.href;if(modules.has(key))return modules.get(key);const code=stripTypeScriptTypes(await readFile(url,'utf8'),{mode:'transform'}),m=new SourceTextModule(code,{identifier:key});modules.set(key,m);await m.link((name,owner)=>name==='@tauri-apps/api/core'?shim:load(name,owner.identifier));return m;}
const wireM=await load('video-drawing-wire'),bridgeM=await load('video-drawing-bridge'),sessionM=await load('video-drawing-session');await wireM.evaluate();await bridgeM.evaluate();await sessionM.evaluate();
const wire=wireM.namespace,bridge=bridgeM.namespace,session=sessionM.namespace;
const uuid=n=>`00000000-0000-0000-0000-${n.toString(16).padStart(12,'0')}`;
const target={sceneId:uuid(1),itemId:uuid(2),sourceId:uuid(3),expectedRangeRevision:4,expectedAnnotationRevision:5};
const ref={layoutId:uuid(6),layoutSha256:'a'.repeat(64),rasterSha256:'b'.repeat(64),width:24,height:26,geometryBounds:{x:2,y:3,width:20,height:20}};
const vector={kind:'line',version:1,sourcePoints:[{x:20.25,y:30.125},{x:40.25,y:50.125}],color:'#123456',strokeWidth:4};
const grant={pluginId:'mewu.core.drawing',revision:1,contributionId:'video-drawing-tools',tools:['pen','line','arrow','rect','ellipse','text','number']};
const read={requestId:uuid(7),target,annotationId:uuid(8),reference:ref,content:{kind:'line',version:1,localPoints:[{x:2,y:3},{x:22,y:23}],color:'#123456',strokeWidth:4}};
const plain=x=>JSON.parse(JSON.stringify(x));
test('canonical UUID, closed source DTO and bounded controls reject foreign fields instead of forwarding PNG/path',()=>{
  assert.equal(wire.videoDrawingUuid(uuid(1)),true);assert.equal(wire.videoDrawingUuid('00000000-0000-0000-000000000000'),false);
  assert.equal(wire.validVideoSourceDrawing(vector),true);
  for(const extra of [{path:'x'},{png:'x'},{origin:{}},{localPoints:vector.sourcePoints},{interval:{startTicks:0,endTicks:10}}])assert.equal(wire.validVideoSourceDrawing({...vector,...extra}),false);
  for(const mutate of [x=>x.sourcePoints.push({x:NaN,y:0}),x=>x.strokeWidth=65,x=>x.sourcePoints=[x.sourcePoints[0],x.sourcePoints[0]],x=>x.kind='mosaic',x=>x.sourcePoints=Array.from({length:4097},(_,i)=>({x:i,y:i}))]){const value=structuredClone(vector);mutate(value);assert.equal(wire.validVideoSourceDrawing(value),false);}
});
test('Number is number/diameter and literal text retains spaces, TAB and line breaks',()=>{
  const number=wire.drawingToVideoSource({id:uuid(8),kind:'number',points:[{x:1.5,y:2.25}],color:'#ff0000',strokeWidth:4,text:'12',fontSize:28});
  assert.deepEqual(plain(number),{kind:'number',version:1,sourcePoints:[{x:1.5,y:2.25}],color:'#ff0000',number:12,diameter:28});
  const text=wire.drawingToVideoSource({id:uuid(8),kind:'text',points:[{x:1,y:2}],color:'#ff0000',strokeWidth:1,text:' 旧\t<&>\n公式 ',fontSize:20});assert.equal(text.content.text,' 旧\t<&>\n公式 ');
  assert.throws(()=>wire.drawingToVideoSource({id:uuid(8),kind:'number',points:[{x:1,y:2}],color:'#ff0000',strokeWidth:1,text:'01',fontSize:28}));
});
test('getter checks all reference identity, geometry, target and request independent of JSON key order',()=>{
  const reversed=Object.fromEntries(Object.entries(read).reverse());assert.deepEqual(plain(wire.validateVideoVectorRead(reversed,read)),read);
  for(const mutate of [x=>x.requestId=uuid(9),x=>x.target.expectedAnnotationRevision++,x=>x.reference.rasterSha256='c'.repeat(64),x=>x.reference.geometryBounds.width=19,x=>x.content.localPoints[1].x=100,x=>x.content.sourcePoints=x.content.localPoints]){const bad=structuredClone(read);mutate(bad);assert.throws(()=>wire.validateVideoVectorRead(bad,read));}
  assert.deepEqual(plain(wire.localVectorDrawing(read,{x:-2,y:10}).points),[{x:0,y:13},{x:20,y:33}]);
});
test('real bridge sends exact typed flattened authority; Add contains no ID or interval and getters deeply capture',async()=>{
  calls.length=0;reply={accepted:true};const action={type:'add',content:vector};await bridge.applyVideoDrawing(uuid(7),target,grant,action);
  assert.deepEqual(plain(calls[0]),{name:'apply_video_drawing',args:{requestId:uuid(7),target,pluginId:grant.pluginId,pluginRevision:1,contributionId:grant.contributionId,action}});
  action.content.sourcePoints[0].x=999;assert.equal(calls[0].args.action.content.sourcePoints[0].x,20.25);vector.sourcePoints[0].x=20.25;
  reply=read;const got=await bridge.getVideoAnnotationVector(uuid(7),target,uuid(8),ref);assert.deepEqual(plain(got),read);got.content.localPoints[0].x=999;assert.equal(read.content.localPoints[0].x,2);
});
test('real bridge rejects wrong authority, update Text, extra action identity, same-ID substituted ref, and browser fallback without invoking',async()=>{
  calls.length=0;
  for(const [g,a]of [[{...grant,tools:['pen']},{type:'add',content:vector}],[grant,{type:'add',id:uuid(9),content:vector}],[grant,{type:'update',annotationId:uuid(8),reference:ref,content:{kind:'text',topLeft:{x:1,y:2},content:{version:1,text:'a',color:'#ffffff',fontSize:20}}}]])await assert.rejects(bridge.applyVideoDrawing(uuid(7),target,g,a));
  assert.equal(calls.length,0);reply={...read,reference:{...ref,layoutSha256:'d'.repeat(64)}};await assert.rejects(bridge.getVideoAnnotationVector(uuid(7),target,uuid(8),ref));
  calls.length=0;tauri=false;await assert.rejects(bridge.applyVideoDrawing(uuid(7),target,grant,{type:'add',content:vector}));assert.equal(calls.length,0);tauri=true;
});
test('integer full-raster move preserves floating local controls, negative-half ties and edge ink',()=>{
  const drawing=wire.localVectorDrawing(read,{x:-2,y:0});
  assert.deepEqual(plain(session.movedVideoVector(drawing,{x:-2,y:0},ref,{x:0,y:0},{x:-.5,y:.5},{width:100,height:100}).points),[{x:0,y:4},{x:20,y:24}]);
  assert.deepEqual(plain(session.visibleVideoVectorBounds({x:-2,y:0},ref,{width:100,height:100})),{x:0,y:0,width:22,height:26});
  assert.throws(()=>session.visibleVideoVectorBounds({x:-.5,y:0},ref,{width:100,height:100}));
});
function fixture(){
  const object={id:uuid(8),interval:{startTicks:0,endTicks:10000000},primitive:{kind:'vector',topLeft:{x:18,y:27},layout:ref}};
  const doc={version:1,clock:'sourcePlaybackTicks',sourceId:uuid(3),sourceDurationTicks:10000000,sourceWidth:100,sourceHeight:100,revision:5,objects:[object],undo:[],redo:[]};
  const before={id:uuid(2),asset:{id:uuid(3),name:'synthetic',kind:'video',path:'synthetic-only.mp4'},x:0,y:0,width:1,height:1,videoEdit:{revision:4},videoAnnotations:doc};
  const added={...structuredClone(object),id:uuid(9)},after=structuredClone(before);after.videoAnnotations={...structuredClone(doc),revision:6,objects:[object,added],undo:[{id:uuid(10),before:[object],after:[object,added]}]};
  return{before,after,snapshot:{scenes:[{id:uuid(1),items:[after]}]},added};
}
test('actual receipt uniquely maps generated Add ID; early publish is not passed as evidence',()=>{
  const{before,snapshot}=fixture();const accepted=session.acceptVideoDrawingReceipt(snapshot,uuid(1),before,{type:'add',content:vector});assert.equal(accepted.annotationId,uuid(9));
  for(const mutate of [x=>x.scenes[0].items[0].videoAnnotations.revision=7,x=>x.scenes[0].items[0].asset.path='other.mp4',x=>x.scenes[0].items[0].videoAnnotations.objects.push(x.scenes[0].items[0].videoAnnotations.objects[1]),x=>x.scenes[0].items[0].videoAnnotations.objects[1].origin={runId:uuid(11)},x=>x.scenes[0].items[0].videoAnnotations.objects[1].interval.endTicks--,x=>x.scenes[0].items[0].videoAnnotations.objects[0].primitive.topLeft.x++]){const bad=structuredClone(snapshot);mutate(bad);assert.throws(()=>session.acceptVideoDrawingReceipt(bad,uuid(1),before,{type:'add',content:vector}));}
});
test('unmounted video drafts share the real cache and remain an exit barrier without blocking image drafts',async()=>{
  const cache=(await load('@product/components/drawing-properties')).namespace.drawingDrafts;
  const draft={mode:'text',drawing:{id:uuid(9),kind:'text',points:[{x:1,y:2}],color:'#ff0000',strokeWidth:1,text:'未保存',fontSize:20},expectedRevision:5};
  const imageKey=JSON.stringify([uuid(1),'background','region']),videoKey=JSON.stringify([uuid(1),'videoDrawing',uuid(2)]),otherKey=JSON.stringify([uuid(10),'videoDrawing',uuid(2)]);
  cache.set(imageKey,draft);assert.equal(session.listVideoDrawingDrafts(uuid(1)).length,0);session.assertVideoDrawingDraftsSaved(uuid(1));
  cache.set(videoKey,draft);cache.set(otherKey,draft);assert.equal(session.listVideoDrawingDrafts(uuid(1)).length,1);assert.equal(session.listVideoDrawingDrafts().length,2);assert.throws(()=>session.assertVideoDrawingDraftsSaved(uuid(1)));
  const newer={...draft,error:'CAS'};cache.set(videoKey,newer);assert.equal(cache.delete(videoKey,draft),false);assert.throws(()=>session.assertVideoDrawingDraftsSaved(uuid(1)));
  cache.delete(imageKey);cache.delete(videoKey);cache.delete(otherKey);
});
