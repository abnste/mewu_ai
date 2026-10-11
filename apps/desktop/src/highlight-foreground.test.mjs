// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const load = async path => { const m=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'}));await m.link(()=>{throw Error('Unexpected dependency');});await m.evaluate();return m.namespace; };
const {highlightForeground,straightForeground}=await load('./highlight-foreground.ts');
const {highlightMaskTable}=await load('./drawing-colors.ts');
function source(width,height,background,ink) { const data=new Uint8ClampedArray(width*height*4);for(let y=0;y<height;y++)for(let x=0;x<width;x++)data.set([...(x>=25&&x<28&&y>=9&&y<31?ink:background),255],(y*width+x)*4);return data; }
test('background separation protects black, white and colored text while leaving uniform background tintable',()=>{
  for(const [bg,ink] of [[[255,255,255],[0,0,0]],[[0,0,0],[255,255,255]],[[240,240,240],[255,0,120]],[[20,20,20],[15,110,180]]]){
    const data=highlightForeground(source(80,40,bg,ink),80,40);
    assert.equal(data[(20*80+26)*4+3],255);assert.equal(data[(20*80+60)*4+3],0);
  }
});
test('antialias foreground recovers black contribution instead of preserving the old pale background',()=>{
  const input=source(80,40,[255,255,255],[0,0,0]);for(let y=9;y<31;y++)input.set([128,128,128,255],(y*80+28)*4);
  const data=highlightForeground(input,80,40),p=Array.from(data.slice((20*80+28)*4,(20*80+28)*4+4));
  assert.equal(p[3],127);assert.deepEqual(p.slice(0,3),[0,0,0]);
  const rgb=[255,230,0].map((c,i)=>p[i]+Math.floor(((255-p[3])*c+127)/255));assert.deepEqual(rgb,[128,115,0]);
  const straight=straightForeground(data);assert.deepEqual(Array.from(straight.slice((20*80+28)*4,(20*80+28)*4+4)),[0,0,0,127]);
});
test('tile borders have identical foreground for repeating details and no writes touch the input',()=>{
  const width=1057,height=31,input=new Uint8ClampedArray(width*height*4);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){const v=x%41<3&&y>2&&y<29?30:230;input.set([v,Math.floor(v/2),v,255],(y*width+x)*4);}
  const before=input.slice(),data=highlightForeground(input,width,height);
  for(let y=14;y<=17;y++)for(let x=480;x<550;x++)assert.deepEqual(Array.from(data.slice((y*width+x)*4,(y*width+x)*4+4)),Array.from(data.slice((y*width+x-41)*4,(y*width+x-41)*4+4)));
  assert.deepEqual(input,before);
});
test('solid and translucent source pixels are excluded by the exact 256-level alpha mask; budgets reject malformed images',()=>{
  assert.deepEqual(highlightMaskTable.trim().split(/\s+/).map(Number),[...Array(255).fill(1),0]);
  const input=new Uint8ClampedArray([100,40,90,128]);assert.equal(highlightForeground(input,1,1)[3],255);
  for(const [w,h] of [[0,1],[1,0],[16385,1],[8192,8192],[1.2,1]])assert.throws(()=>highlightForeground(input,w,h));
  assert.throws(()=>highlightForeground(new Uint8ClampedArray(3),1,1));
});
