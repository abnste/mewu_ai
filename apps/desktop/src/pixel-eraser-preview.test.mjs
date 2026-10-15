// SPDX-License-Identifier: MPL-2.0
// Execute the production browser mutation, not a second implementation.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
import vm from 'node:vm';
import {randomUUID} from 'node:crypto';
const source=await readFile(new URL('./bridge.ts',import.meta.url),'utf8');
const node=parse(source,{sourceType:'module',plugins:['typescript']}).program.body.find(n=>n.type==='ExportNamedDeclaration'&&n.declaration?.id?.name==='previewDrawingCommand').declaration;
const actual=stripTypeScriptTypes(source.slice(node.start,node.end),{mode:'transform'});
const plain=value=>JSON.parse(JSON.stringify(value));
function fixture(){
  const pen={id:randomUUID(),kind:'pen',color:'#123456',strokeWidth:4,points:[{x:10,y:50},{x:90,y:50}]};
  const rect={...structuredClone(pen),id:randomUUID(),kind:'rect',points:[{x:10,y:10},{x:30,y:30}]};
  const region={id:'roi',drawings:[pen,rect],drawingRevision:7,drawingHistory:{undo:[],redo:[]}};
  const scene={id:'scene',background:{id:'source',width:100,height:100},regions:[region],frozen:false,closed:false};
  let published=0;
  const preview={activeSceneId:'scene',scenes:[scene]};
  const context=vm.createContext({structuredClone,native:false,preview,findScene:id=>{assert.equal(id,'scene');return scene;},clone:()=>structuredClone(preview),publish:()=>{published++;return structuredClone(preview);}});
  vm.runInContext(actual,context);
  const run=action=>context.previewDrawingCommand({sceneId:'scene',regionId:'roi',backgroundId:'source',expectedRevision:scene.regions[0].drawingRevision,...action});
  return {pen,rect,scene,run,revision:()=>scene.regions[0].drawingRevision,region:()=>scene.regions[0],published:()=>published,
    erase:()=>({drawingId:pen.id,fragments:[{id:pen.id,points:[{x:10,y:50},{x:43,y:50}]},{id:randomUUID(),points:[{x:57,y:50},{x:90,y:50}]}]})};
}
test('actual preview fragments preserve style and order and undo/redo the whole stroke in one revision',async()=>{
  const f=fixture(),cut=f.erase();await f.run({type:'erase_drawing_strokes',erasures:[cut]});
  assert.equal(f.revision(),8);assert.equal(f.region().drawings.length,3);assert.equal(f.region().drawingHistory.undo.length,1);
  assert.deepEqual(plain(f.region().drawings),[...cut.fragments.map(part=>({...f.pen,id:part.id,points:part.points})),f.rect]);
  await f.run({type:'undo_drawing'});assert.deepEqual(plain(f.region().drawings),[f.pen,f.rect]);
  await f.run({type:'redo_drawing'});assert.deepEqual(plain(f.region().drawings).map(d=>d.id),[...cut.fragments.map(p=>p.id),f.rect.id]);
});
test('actual preview no-op preserves revision, history and redo; malformed later input cannot partially publish',async()=>{
  const f=fixture();await f.run({type:'erase_drawing_strokes',erasures:[f.erase()]});await f.run({type:'undo_drawing'});
  const before=plain(f.region()),published=f.published();
  await f.run({type:'erase_drawing_strokes',erasures:[{drawingId:f.pen.id,fragments:[{id:f.pen.id,points:f.pen.points}]}]});
  assert.deepEqual(plain(f.region()),before);assert.equal(f.published(),published);
  for(const bad of [
    {drawingId:f.rect.id,fragments:[]},
    {drawingId:f.pen.id,fragments:[]},
    {drawingId:'missing',fragments:[]},
  ]){
    await assert.rejects(f.run({type:'erase_drawing_strokes',erasures:[f.erase(),bad]}));
    assert.deepEqual(plain(f.region()),before);assert.equal(f.published(),published);
  }
  for(const point of [{x:101,y:50},{x:-1,y:50},{x:NaN,y:50}]){
    const bad=f.erase();bad.fragments[1].points[0]=point;
    await assert.rejects(f.run({type:'erase_drawing_strokes',erasures:[bad]}));assert.deepEqual(plain(f.region()),before);
  }
  await assert.rejects(f.run({type:'erase_drawing_strokes',expectedRevision:0,erasures:[f.erase()]}));
  assert.deepEqual(plain(f.region()),before);
});
