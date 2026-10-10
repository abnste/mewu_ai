import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
import {parse} from '@babel/parser';
import test from 'node:test';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const raw=url=>readFile(new URL(url,import.meta.url),'utf8'),plain=x=>JSON.parse(JSON.stringify(x)),tick=()=>new Promise(r=>setImmediate(r));
const synth=values=>new SyntheticModule(Object.keys(values),function(){for(const[k,v]of Object.entries(values))this.setExport(k,v);});
const loaded=new Map();
async function pure(file){if(loaded.has(file))return loaded.get(file);const code=await raw(file.startsWith('@product/')?`./${file.slice(9)}.ts`:`${file}.ts`),m=new SourceTextModule(stripTypeScriptTypes(code,{mode:'transform'}));loaded.set(file,m);await m.link(name=>pure(name.startsWith('@product/')?name:name.replace(/^\.\//,'')));await m.evaluate();return m;}
const wire=(await pure('video-drawing-wire')).namespace,session=(await pure('video-drawing-session')).namespace,property=(await pure('@product/components/drawing-properties')).namespace;
const textSource=await raw('./video-manual-edit.ts'),textAst=parse(textSource,{sourceType:'module',plugins:['typescript']});
const funcs=['validateVideoTextContent','validateVideoTextRead'].map(name=>textAst.program.body.find(node=>node.type==='ExportNamedDeclaration'&&node.declaration?.id?.name===name).declaration).map(node=>textSource.slice(node.start,node.end));
const textModule=new SourceTextModule(stripTypeScriptTypes(`const equal=(a,b)=>JSON.stringify(a)===JSON.stringify(b);${funcs.map(value=>'export '+value).join('\n')}`,{mode:'transform'}));await textModule.link(()=>{throw Error('Unexpected import');});await textModule.evaluate();
const code=await raw('components/VideoDrawingEditor.tsx'),ast=parse(code,{sourceType:'module',plugins:['typescript','jsx']}),component=ast.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration;
const body=component.body.body.filter(node=>node.type!=='ReturnStatement');
let fixtureBody=body.map(node=>{let text=code.slice(node.start,node.end);if(node.type==='VariableDeclaration'&&node.declarations[0].id.name==='port'){const value=node.declarations[0].init.properties.find(property=>property.key.name==='render').value;text=text.slice(0,value.start-node.start)+'()=>undefined'+text.slice(value.end-node.start);}return text;}).join('\n');
const deps={...solid,...wire,...session,draftChanged:property.draftChanged,validateVideoTextRead:textModule.namespace.validateVideoTextRead,getVideoAnnotationVector:()=>{throw Error('No real IPC permitted');}};
const fixtureM=new SourceTextModule(stripTypeScriptTypes(`import {${Object.keys(deps).join(',')}} from 'deps';export function fixture(props){${fixtureBody};return{port,command,ensureEditable,register};}`,{mode:'transform'}));await fixtureM.link(()=>synth(deps));await fixtureM.evaluate();
const id=n=>`00000000-0000-0000-0000-${n.toString(16).padStart(12,'0')}`;
const ref={layoutId:id(10),layoutSha256:'a'.repeat(64),rasterSha256:'b'.repeat(64),width:24,height:26,geometryBounds:{x:2,y:3,width:20,height:20}};
const vector={kind:'line',version:1,localPoints:[{x:2,y:3},{x:22,y:23}],color:'#123456',strokeWidth:4};
function item(count=2){const objects=Array.from({length:count},(_,i)=>({id:id(100+i),interval:{startTicks:0,endTicks:10000000},primitive:{kind:'vector',topLeft:{x:18,y:27},layout:{...ref,layoutId:id(10+i)}}}));return{id:id(2),asset:{id:id(3),kind:'video',name:'synthetic',path:'synthetic-only.mp4'},x:0,y:0,width:1,height:1,videoAnnotations:{version:1,clock:'sourcePlaybackTicks',sourceId:id(3),sourceDurationTicks:10000000,sourceWidth:100,sourceHeight:100,revision:5,objects,undo:[],redo:[]}};}
const grant={pluginId:'mewu.core.drawing',revision:1,contributionId:'video-drawing-tools',tools:['pen','line','arrow','rect','ellipse','text','number']};
function setup(value=item(),overrides={}){
  const reads=[],documents=[],snapshots=[],errors=[];let dispose;
  const[result,setItem]=solid.createSignal(value),[authority,setGrant]=solid.createSignal(grant),[active,setActive]=solid.createSignal(true);
  const props={sceneId:id(1),get item(){return result();},info:{durationTicks:10000000,width:100,height:100,frameRateNumerator:30,frameRateDenominator:1,hasAudio:false},box:{x:0,y:0,width:200,height:70},get grant(){return authority();},busy:false,inputLocked:false,get active(){return active();},onPause(){},onClose(){},onError:error=>errors.push(error),onSnapshot:s=>{snapshots.push(s);setItem(s.scenes.find(v=>v.id===id(1)).items[0]);},renderStored(){},
    onReadVector:async(requestId,target,annotationId,reference)=>{reads.push(annotationId);return{requestId,target,annotationId,reference,content:vector};},
    onReadText:async(requestId,target,annotationId,reference)=>{reads.push(annotationId);return{requestId,target,annotationId,reference,content:{version:1,text:' 旧\t<&>\n正文 ',color:'#ff0000',fontSize:20}};},
    onDocument:async(target,action)=>{documents.push({target,action});return{scenes:[{id:id(1),items:[result()]}]};},onApplyDrawing:async()=>{throw Error('Missing synthetic native');},onEditText:async()=>{throw Error('Missing synthetic native');},...overrides};
  const actual=solid.createRoot(cleanup=>{dispose=cleanup;return fixtureM.namespace.fixture(props);});return{...actual,props,reads,documents,snapshots,errors,setItem,setGrant,setActive,dispose};
}
test('actual JSX adapter mounts 4096 compact refs without a getter; visibility/render/Move never infer control points',async()=>{
  const f=setup(item(4096),{visibleAnnotationIds:[id(100)]});await tick();assert.equal(f.reads.length,0);assert.equal(f.port.stored().length,1);assert.equal(f.port.drawings().length,0);
  const object=f.port.stored()[0],to=f.port.storedMove(object.id,object.bounds,{x:-20.5,y:3.5});assert.deepEqual(plain(to),{x:-2,y:31,width:24,height:26});
  await f.command({type:'move_stored',expectedRevision:5,drawingId:object.id,from:object.bounds,to});assert.equal(f.reads.length,0);assert.equal(f.documents[0].action.type,'move');assert.equal(f.documents[0].action.toTopLeft.x,-2);f.dispose();
});
test('selected exact-ref getter is lazy; unchanged-ref Move reuses contents and changed-ref/source cannot install a late result',async()=>{
  const f=setup();await tick();const value=await f.ensureEditable(id(100));assert.equal(f.reads.length,1);assert.deepEqual(plain(value.points),[{x:20,y:30},{x:40,y:50}]);
  const next=structuredClone(f.props.item);next.videoAnnotations.revision++;next.videoAnnotations.objects[0].primitive.topLeft.x=19;f.setItem(next);await tick();const moved=await f.ensureEditable(id(100));assert.equal(f.reads.length,1);assert.equal(moved.points[0].x,21);
  let release;f.props.onReadVector=async(requestId,target,annotationId,reference)=>new Promise(resolve=>{release=()=>resolve({requestId,target,annotationId,reference,content:vector});});
  const pending=f.ensureEditable(id(101));await tick();const changed=structuredClone(next);changed.videoAnnotations.objects[1].primitive.layout.rasterSha256='d'.repeat(64);f.setItem(changed);release();assert.equal(await pending,undefined);assert.equal(f.port.drawings().some(value=>value.id===id(101)),false);f.dispose();
});
test('existing Text loads literal old content, and persisted read/Move/Delete/Undo remain usable with no authoring grant',async()=>{
  const initial=item(1);initial.videoAnnotations.objects.push({id:id(101),interval:{startTicks:0,endTicks:10000000},primitive:{kind:'text',topLeft:{x:1,y:2},layout:{layoutId:id(90),layoutSha256:'c'.repeat(64),rasterSha256:'d'.repeat(64),width:20,height:30}}});initial.videoAnnotations.undo=[{id:id(91),before:[],after:initial.videoAnnotations.objects}];
  const f=setup(initial);f.setGrant(undefined);await tick();const text=await f.ensureEditable(id(101));assert.equal(text.text,' 旧\t<&>\n正文 ');assert.equal(f.port.canStyle(text),true);
  await f.command({type:'remove_drawing',expectedRevision:5,drawingId:id(100)});await f.command({type:'undo_drawing',expectedRevision:5});assert.deepEqual(f.documents.map(v=>v.action.type),['remove','undo']);assert.equal(f.port.allows({type:'add_drawing',drawing:text}),false);f.dispose();
});
test('Add waits real receipt; publish-before-receipt and late inactive view preserve successful authority without guessing generated ID',async()=>{
  let resolve;const f=setup(item(1),{onApplyDrawing:()=>new Promise(yes=>resolve=yes)});await tick();const drawing={id:id(200),kind:'line',points:[{x:20,y:30},{x:40,y:50}],color:'#123456',strokeWidth:4},before=structuredClone(f.props.item);
  const work=f.command({type:'add_drawing',expectedRevision:5,drawing});await tick();assert.equal(f.port.persistedId(id(200)),id(200));assert.equal(f.snapshots.length,0);
  const after=structuredClone(before),added={...after.videoAnnotations.objects[0],id:id(300)};after.videoAnnotations.objects.push(added);after.videoAnnotations.revision++;after.videoAnnotations.undo=[{id:id(400),before:before.videoAnnotations.objects,after:after.videoAnnotations.objects}];f.setItem(after);await tick();assert.equal(f.port.persistedId(id(200)),id(200));f.setActive(false);
  const receipt={scenes:[{id:id(1),items:[after]}]};resolve(receipt);await work;assert.equal(f.port.persistedId(id(200)),id(300));assert.equal(f.snapshots.length,1);f.dispose();
});
test('getter failure does not fabricate geometry; changed bounds/revision fail Move before native dispatch',async()=>{
  const f=setup(item(1),{onReadVector:async()=>{throw Error('read failed');}});await tick();await assert.rejects(f.ensureEditable(id(100)),/read failed/);assert.equal(f.port.drawings().length,0);
  const from=f.port.stored()[0].bounds;await assert.rejects(f.command({type:'move_stored',expectedRevision:4,drawingId:id(100),from,to:{...from,x:30}}));await assert.rejects(f.command({type:'move_stored',expectedRevision:5,drawingId:id(100),from:{...from,x:9},to:{...from,x:30}}));assert.equal(f.documents.length,0);f.dispose();
});
