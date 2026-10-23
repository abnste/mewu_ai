// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const load=async path=>import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64'));
const overlay=await load('./extension-transient-overlay.ts');
test('clamp keeps the overlay offset from the pointer and inside the viewport margin',()=>{
  const clamp=overlay.clampTransient;
  assert.deepEqual(clamp(400,300,260,260,1920,1080),{left:412,top:312});
  // Near the right/bottom edges the frame slides to the margin instead of flipping.
  assert.deepEqual(clamp(1900,1060,260,260,1920,1080),{left:1644,top:804});
  // Near the top-left the margin floors the placement, never negative.
  assert.deepEqual(clamp(-40,-40,260,260,1920,1080),{left:16,top:16});
});
test('a viewport that cannot fit the frame centers it instead of crossing zero',()=>{
  const clamp=overlay.clampTransient;
  assert.deepEqual(clamp(120,120,260,260,280,280),{left:10,top:10});
  assert.deepEqual(clamp(120,120,260,260,200,280),{left:0,top:10});
  assert.deepEqual(clamp(120,120,260,260,280,200),{left:10,top:0});
});
test('escape arbitration ignores composition and other keys',()=>{
  const escape=overlay.isTransientEscape;
  assert.equal(escape({key:'Escape'}),true);
  assert.equal(escape({key:'Escape',isComposing:true}),false);
  assert.equal(escape({key:'Escape',keyCode:229}),false);
  assert.equal(escape({key:'Enter'}),false);
});
test('pointer samples accept only finite coordinates with sane button masks',()=>{
  const sample=overlay.transientPointerSample;
  assert.deepEqual(sample({clientX:12.5,clientY:-8,buttons:1}),{x:12.5,y:-8,buttons:1});
  assert.deepEqual(sample({clientX:0,clientY:0,buttons:0}),{x:0,y:0,buttons:0});
  assert.equal(sample({clientX:Infinity,clientY:8,buttons:0}),undefined);
  assert.equal(sample({clientX:'8',clientY:8,buttons:0}),undefined);
  assert.equal(sample({clientX:8,clientY:8,buttons:-1}),undefined);
  assert.equal(sample({clientX:8,clientY:8,buttons:32}),undefined);
});
test('the transient bridge forwards pointer samples, destroys on escape and ignores the rest',()=>{
  let handler;const sent=[];let destroyed=0;
  const bridge=overlay.transientPortBridge({
    set onmessage(value){handler=value;},get onmessage(){return handler;},
    postMessage:value=>sent.push(value),close(){this.closed=true;}
  },()=>destroyed++);
  bridge.pointer({x:8,y:9,buttons:2});
  handler({data:{type:'response',id:1,ok:true}});
  handler({data:{type:'call',id:2,input:{}}});
  handler({data:'noise'});
  assert.equal(destroyed,0);
  assert.deepEqual(sent,[{type:'event',name:'pointer',value:{x:8,y:9,buttons:2}}]);
  handler({data:{type:'escape'}});
  assert.equal(destroyed,1);
  bridge.close();
});
