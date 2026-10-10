// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule} from 'node:vm';
const module=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./components/drawing-geometry.ts',import.meta.url),'utf8'),{mode:'transform'}));
await module.link(()=>{throw Error('Unexpected runtime dependency');});await module.evaluate();
const {arrowWings,arrowHead,drawingBounds}=module.namespace;
const close=(actual,expected)=>assert.ok(Math.abs(actual-expected)<1e-9,`${actual} differs from ${expected}`);

test('original Mewu arrow wings grow with shaft length and stop at 24',()=>{
  for(const [shaft,wing] of [[4,1],[8,2],[32,8],[80,20],[96,24],[200,24],[1000,24]]){
    const end={x:shaft,y:0},[left,right]=arrowWings([{x:0,y:0},end]);
    close(Math.hypot(left.x-end.x,left.y),wing);close(Math.hypot(right.x-end.x,right.y),wing);
    close(left.y,wing/2);close(right.y,-wing/2);
    close(Math.atan2(left.y,end.x-left.x),Math.PI/6);
    assert.ok(left.x>0); // The short-arrow head must not reach the shaft's tail.
  }
});
test('adaptive arrow remains symmetric and finite in all directions and at large source offsets',()=>{
  for(const [dx,dy] of [[40,0],[0,40],[-40,0],[0,-40],[30,40],[-30,40],[-30,-40],[30,-40]]){
    const a={x:12000,y:13000},b={x:a.x+dx,y:a.y+dy},[left,right]=arrowWings([a,b]),size=Math.min(24,Math.hypot(dx,dy)/4);
    close(Math.hypot(left.x-b.x,left.y-b.y),size);close(Math.hypot(right.x-b.x,right.y-b.y),size);
    close((left.x-b.x)*dy-(left.y-b.y)*dx,-size*Math.hypot(dx,dy)/2);
    close((right.x-b.x)*dy-(right.y-b.y)*dx,size*Math.hypot(dx,dy)/2);
    assert.ok([left.x,left.y,right.x,right.y].every(Number.isFinite));
  }
});
test('arrow selection bounds include the painted wings and the stroke margin',()=>{
  for(const [a,b] of [[{x:20,y:50},{x:180,y:50}],[{x:50,y:20},{x:50,y:180}]]){
    const draw={kind:'arrow',points:[a,b],strokeWidth:6},box=drawingBounds(draw);
    for(const p of [a,b,...arrowWings(draw.points)])assert.ok(p.x-3>=box.x-1e-9&&p.x+3<=box.x+box.width+1e-9&&p.y-3>=box.y-1e-9&&p.y+3<=box.y+box.height+1e-9);
    close(a.y===b.y?box.height:box.width,30);
  }
});
test('incomplete, coincident and nonfinite arrows cannot paint invalid SVG coordinates',()=>{
  for(const points of [[],[{x:0,y:0}],[{x:0,y:0},{x:0,y:0}],[{x:NaN,y:0},{x:3,y:4}],[{x:0,y:0},{x:Infinity,y:0}]]){
    assert.equal(arrowWings(points),undefined);assert.equal(arrowHead(points),'');
  }
});
