// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import test from 'node:test';
const source=stripTypeScriptTypes(await readFile(new URL('./object-move-grip.ts',import.meta.url),'utf8'),{mode:'transform'});
const {objectMoveGripPosition,OBJECT_MOVE_WIDTH,OBJECT_MOVE_HEIGHT}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const viewport={width:1000,height:800};
const absolute=(box)=>{const point=objectMoveGripPosition(box,viewport);return{x:box.x+parseFloat(point.left),y:box.y+parseFloat(point.top)};};
test('the grip follows live translation and the horizontal center of a resized object',()=>{
  const box={x:100,y:150,width:240},start=absolute(box),moved=absolute({...box,x:183,y:117});
  assert.equal(moved.x-start.x,83);assert.equal(moved.y-start.y,-33);
  const resized=absolute({...box,width:400});assert.equal(resized.x-start.x,80);assert.equal(resized.y,start.y);
});
test('partially offscreen objects retain a reachable grip on every viewport edge',()=>{
  for(const box of [{x:-500,y:-100,width:540},{x:990,y:790,width:200},{x:-30,y:20,width:36}]){
    const point=absolute(box);assert.ok(point.x-OBJECT_MOVE_WIDTH/2>=0&&point.x+OBJECT_MOVE_WIDTH/2<=viewport.width);assert.ok(point.y>=0&&point.y+OBJECT_MOVE_HEIGHT<=viewport.height);
  }
});
test('switching objects computes position from the new object without retaining old geometry',()=>{
  absolute({x:990,y:-400,width:80});
  assert.deepEqual(absolute({x:40,y:60,width:120}),{x:100,y:64});
});
