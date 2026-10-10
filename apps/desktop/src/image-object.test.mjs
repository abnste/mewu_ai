// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
async function load(path,deps={}){const m=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'}));await m.link(name=>new SyntheticModule(Object.keys(deps[name]),function(){for(const [k,v] of Object.entries(deps[name]))this.setExport(k,v);}));await m.evaluate();return m.namespace;}
const geometry=await load('./components/drawing-geometry.ts'),selection=await load('./components/drawing-selection.ts',{'./drawing-geometry':geometry}),image=await load('./image-object-geometry.ts');
const picture={id:'image',kind:'rich',points:[{x:100,y:100},{x:300,y:200}],rich:{kind:'extracted'}},ink={id:'ink',kind:'line',strokeWidth:4,points:[{x:200,y:200},{x:300,y:300}]};
test('marquee intersects selectable ink and images, skips repair, moves a group without altering shape or saved arrays',()=>{
  const box=selection.selectionBox({x:350,y:320},{x:80,y:80});
  const repair={...picture,id:'repair',rich:{kind:'repair',parentId:'image'}};
  assert.deepEqual([...selection.marqueeDrawings([picture,ink,repair],box,d=>d.rich?.kind!=='repair')],['image','ink']);
  const before=structuredClone([picture,ink]);const moved=selection.movedDrawings([picture,ink],20,-200,{x:0,y:0,width:800,height:600},{width:800,height:600});
  assert.ok(moved[0].points[0].y<0);assert.equal(geometry.drawingBounds(moved[1]).y,0);assert.equal(moved[1].points[0].y-moved[0].points[0].y,100);assert.deepEqual([picture,ink],before);
  assert.equal(moved[0].points[1].x-moved[0].points[0].x,200);
});
test('attached repair preview follows parent translation and zoom, unrelated ink leaves it untouched',()=>{
  const repair={id:'repair',kind:'rich',points:[{x:120,y:120},{x:140,y:140}],rich:{kind:'repair',parentId:'image'}};
  const parent={...picture,points:[{x:-10,y:-10},{x:390,y:190}]};
  const result=selection.linkedRepairPreview(repair,new Map([['image',picture]]),new Map([['image',parent]]));
  assert.deepEqual(JSON.parse(JSON.stringify(result.points)),[{x:30,y:30},{x:70,y:70}]);assert.equal(result.rich,repair.rich);
  assert.equal(selection.linkedRepairPreview(repair,new Map([['image',picture]]),new Map([['ink',ink]])),repair);
});
test('viewport zoom anchors the cursor and movement crosses all four edges symmetrically with a reachable sliver',()=>{
  const box={x:.2,y:.2,width:.3,height:.2},anchor={x:.3,y:.3};
  const zoom=image.zoomImageObject(box,anchor,-120,1000,800);assert.ok(zoom.width>box.width);assert.ok(Math.abs((anchor.x-zoom.x)/zoom.width-(anchor.x-box.x)/box.width)<1e-12);
  const moved=image.moveImageObject(box,-10,-10,1000,800);assert.equal(moved.x, .016-.3);assert.equal(moved.y,.02-.2);
  const bottom=image.moveImageObject(box,10,10,1000,800);assert.equal(bottom.x,.984);assert.equal(bottom.y,.98);
  assert.equal(image.zoomImageObject(box,anchor,NaN,1000,800),undefined);
});
