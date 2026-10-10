// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import test from 'node:test';
const load = async path => import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64')}`);
const images=await load('./blackboard-image.ts'),colors=await load('./drawing-colors.ts');
const picture={id:'picture',kind:'rich',color:'#000000',strokeWidth:1,points:[{x:120,y:80},{x:320,y:180}],rich:{kind:'extracted',layoutId:'immutable',width:200,height:100}};
const frame={x:0,y:0,width:800,height:600};
test('wheel zoom preserves immutable source and aspect, anchors the cursor, and does not mutate persisted geometry',()=>{
  const before=structuredClone(picture),anchor={x:170,y:110};
  const next=images.zoomBlackboardImage(picture,frame,anchor,-120),[a,b]=next.points;
  assert.equal((b.x-a.x)/(b.y-a.y),2);
  assert.ok(Math.abs((anchor.x-a.x)/(b.x-a.x)-.25)<1e-12);
  assert.ok(Math.abs((anchor.y-a.y)/(b.y-a.y)-.3)<1e-12);
  assert.deepEqual(picture,before);assert.equal(next.rich,picture.rich);
  let bounded=next;
  for(let i=0;i<30;i++)bounded=images.zoomBlackboardImage(bounded,frame,anchor,-240);
  assert.ok(bounded.points[0].x>=0&&bounded.points[0].y>=0);
  assert.ok(bounded.points[1].x<=800&&bounded.points[1].y<=600);
  for(let i=0;i<50;i++)bounded=images.zoomBlackboardImage(bounded,frame,anchor,240);
  assert.ok(bounded.points[1].y-bounded.points[0].y>=24-1e-9);
  assert.equal(images.zoomBlackboardImage(picture,frame,anchor,NaN),undefined);
});
test('hover chooses the topmost image, ignores repair and ink, and respects its bounds',()=>{
  const second={...picture,id:'second'},repair={...picture,id:'repair',rich:{...picture.rich,kind:'repair'}};
  assert.equal(images.imageAt([picture,second,repair,{kind:'pen'}],{x:150,y:100}),second);
  assert.equal(images.imageAt([picture],{x:110,y:100}),undefined);
});
test('RGB and HSV conversion round trips presets, gray and custom colors with bounded channels',()=>{
  for(const color of [...colors.colorPresets,'#000000','#808080','#12ABCD','#E8ECF4']){
    const {h,s,v}=colors.colorHsv(color);assert.equal(colors.hsvColor(h,s,v),color);
  }
  assert.equal(colors.rgbColor([-10,256,127.5]),'#00FF80');
  assert.equal(colors.normalizeColor(' #abcdef '),'#ABCDEF');
  assert.equal(colors.normalizeColor('rgb(1,2,3)'),undefined);
});
test('recent colors persist three unique most recently used values and tolerate unavailable or malformed storage',()=>{
  let data;const storage={getItem:()=>data,setItem:(_,value)=>data=value};
  for(const color of ['#abcdef','#123456','#789abc','#ABCDEF','#12ABCD'])colors.rememberColor(color,storage);
  assert.deepEqual(colors.recentColors(storage),['#12ABCD','#ABCDEF','#789ABC']);
  assert.deepEqual(colors.rememberColor('invalid',storage),['#12ABCD','#ABCDEF','#789ABC']);
  data='{"unknown":true}';assert.deepEqual(colors.recentColors(storage),[]);
  data='["bad","#abcdef","#ABCDEF",42,"#123456"]';assert.deepEqual(colors.recentColors(storage),['#ABCDEF','#123456']);
  const blocked={getItem(){throw Error('disabled');},setItem(){throw Error('quota');}};
  assert.deepEqual(colors.rememberColor('#123456',blocked),['#123456']);
});
