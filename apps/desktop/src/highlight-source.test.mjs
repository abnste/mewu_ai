// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./highlight-source.ts', import.meta.url), 'utf8'), {mode:'transform'}));
await module.link(() => {throw Error('Unexpected IO');}); await module.evaluate();
const {highlightTarget,highlightCrop,highlightSourceIdentity:key}=module.namespace;
const background={id:'source',kind:'image',name:'owned',path:'owned.png',width:200,height:100};
const ink={id:'pen',kind:'pen',color:'#ff0000',strokeWidth:2,points:[{x:30,y:25}]};
const mosaic={id:'redaction',kind:'mosaic',color:'#ff0000',strokeWidth:6,points:[{x:30,y:25},{x:40,y:35}]};
const region={id:'r',x:20.4,y:10.6,width:120.3,height:60.2,drawings:[ink,mosaic],drawingRevision:2};
test('foreground footprint ignores ink, revision, undo history and translation; raster and geometry edits invalidate it',()=>{
  const initial=key('s',background,region);
  assert.equal(key('s',background,{...region,drawings:[{...ink,color:'#000000'},mosaic, {...ink,id:'next'}],drawingRevision:3,drawingHistory:{undo:[{batch:[]}],redo:[]},translation:{overlay:background}}),initial);
  for(const next of [{...region,x:21},{...region,drawings:[ink]},{...region,drawings:[ink,{...mosaic,points:[{x:30,y:25},{x:45,y:35}]}]},{...region,imageOverride:{...background,id:'long'}}])assert.notEqual(key('s',background,next),initial);
  assert.notEqual(key('other',background,region),initial);
  assert.notEqual(key('s',{...background,path:'changed.png'},region),initial);
  const target=highlightTarget(background,region);assert.deepEqual(target.rasterDrawings,[mosaic]);
  target.rasterDrawings[0].points[0].x=90;assert.equal(mosaic.points[0].x,30);
});
test('native integer crop mapping is reproduced for normal and replacement sources, and rich raster edits retain exact references',()=>{
  assert.deepEqual(highlightCrop(background,region),{x:20,y:11,width:120,height:60});
  const override={...background,id:'long',width:80,height:1500};
  const repair={...ink,id:'repair',kind:'rich',rich:{kind:'repair',layoutId:'layout',rasterSha256:'hash',parentId:'lift'}};
  const lifted={...repair,id:'lift',rich:{...repair.rich,kind:'extracted'}};
  const formula={...repair,rich:{...repair.rich,kind:'formula'}};
  const next={...region,imageOverride:override,drawings:[repair,lifted,formula,ink]};
  assert.deepEqual(highlightCrop(background,next),{x:0,y:0,width:80,height:1500});
  assert.deepEqual(highlightTarget(background,next).rasterDrawings,[repair,lifted]);
  assert.equal(highlightTarget(background,next).sourceId,'long');
});
