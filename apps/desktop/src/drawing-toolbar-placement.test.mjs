// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const mod=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./drawing-toolbar-placement.ts',import.meta.url),'utf8')));
await mod.link(()=>{throw Error('Unexpected dependency');});await mod.evaluate();
const {placeDrawingToolbar:place,avoidDrawingToolbar:avoid}=mod.namespace;
const vp={width:1280,height:720},size={width:650,height:124};
const inside=r=>{assert.ok(r.x>=6&&r.y>=6);assert.ok(r.x+r.width<=vp.width-6&&r.y+r.height<=vp.height-6);};

test('a top-edge selection puts both bubbles below the region without overlap',()=>{
  const r=place({x:100,y:5,width:700,height:200},size,vp);inside(r);
  assert.equal(r.overlaps,false);assert.ok(r.y>=213);
});
test('a bottom-edge selection keeps both bubbles above the region',()=>{
  const r=place({x:900,y:500,width:300,height:210},size,vp,true);inside(r);
  assert.equal(r.overlaps,false);assert.ok(r.y+size.height<500);assert.equal(r.x,550);
});
test('tall regions use an available side rather than cover the annotation area',()=>{
  const r=place({x:6,y:6,width:300,height:708},size,vp);inside(r);
  assert.equal(r.overlaps,false);assert.ok(r.x>306);
});
test('a full-screen annotation starts docked at the bottom and moves away from an active stroke',()=>{
  const box={x:0,y:0,...vp};const bottom=place(box,size,vp);inside(bottom);
  assert.equal(bottom.overlaps,true);assert.ok(bottom.y>450);
  const edge=avoid(bottom,{x:300,y:650},box,vp);assert.equal(edge,'top');
  const top=place(box,size,vp,false,edge);inside(top);assert.equal(top.y,6);
  assert.equal(avoid(top,{x:300,y:400},box,vp),'top');
  assert.equal(avoid(top,{x:300,y:20},box,vp),'bottom');
});
test('no flip when the pointer is beside the toolbar or both choices are obstructed',()=>{
  const box={x:0,y:0,...vp};const r=place(box,size,vp);
  assert.equal(avoid(r,{x:1000,y:650},box,vp),'bottom');
  const tiny={width:800,height:190};const large=place({x:0,y:0,...tiny},size,tiny);
  assert.equal(avoid(large,{x:300,y:80},{x:0,y:0,...tiny},tiny),'bottom');
});
