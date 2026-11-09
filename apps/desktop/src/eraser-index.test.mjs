// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const source=stripTypeScriptTypes(await readFile(new URL('./components/drawing-eraser.ts',import.meta.url),'utf8'),{mode:'transform'});
const {createStrokeHitTester,eraserStrokeHits}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));

test('candidate pruning agrees with exact capsule hits across dots, thick strokes, fast sweeps and negative coordinates',()=>{
  let seed=7153;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/4294967296;};
  const drawings=Array.from({length:256},(_,index)=>({id:String(index),kind:['pen','line','highlighter','rect'][index%4],strokeWidth:1+random()*63,
    points:Array.from({length:1+index%23},()=>({x:random()*1000-500,y:random()*1000-500}))}));
  const hit=createStrokeHitTester(drawings),allowed=new Set(drawings.filter((_,i)=>i%3).map(x=>x.id));
  for(let sample=0;sample<100;sample++){
    const a={x:random()*2000-1000,y:random()*2000-1000},b=sample%4?a:{x:random()*2000-1000,y:random()*2000-1000},diameter=1+random()*64;
    assert.deepEqual(hit(a,b,diameter),eraserStrokeHits(drawings,a,b,diameter));
    assert.deepEqual(hit(a,b,diameter,allowed),eraserStrokeHits(drawings.filter(x=>allowed.has(x.id)),a,b,diameter));
  }
});

test('repeated misses do not revisit thousands of stroke points',()=>{
  let reads=0;
  const points=Array.from({length:4096},(_,i)=>({get x(){reads++;return 500+i/10;},get y(){reads++;return 500;}}));
  const hit=createStrokeHitTester([{id:'far',kind:'pen',strokeWidth:4,points}]);
  reads=0;
  for(let i=0;i<100;i++)assert.deepEqual(hit({x:0,y:0},{x:50,y:50},8),[]);
  assert.equal(reads,0);
});

test('document replacement rebuilds bounds and permission changes apply without rebuilding',()=>{
  const stroke={id:'a',kind:'pen',strokeWidth:2,points:[{x:100,y:100}]};
  const old=createStrokeHitTester([stroke]);
  const changed=createStrokeHitTester([{...stroke,points:[{x:0,y:0}]}]);
  assert.deepEqual(old({x:0,y:0},{x:0,y:0},4),[]);
  assert.deepEqual(changed({x:0,y:0},{x:0,y:0},4),['a']);
  assert.deepEqual(changed({x:0,y:0},{x:0,y:0},4,new Set()),[]);
});
