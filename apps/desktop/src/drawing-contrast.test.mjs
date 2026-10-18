// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule} from 'node:vm';
import test from 'node:test';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./drawing-colors.ts',import.meta.url),'utf8'),{mode:'transform'}));
await module.link(()=>{throw Error('Unexpected import');}); await module.evaluate();
const {numberForeground} = module.namespace;
test('new screenshot and blackboard editors use red yellow blue without changing saved preferences',()=>{
  const {initialDrawingColor,initialColorPalette,colorPaletteKey,colorHistoryKey}=module.namespace;
  const saved=new Map(),writes=[];
  const storage={getItem:key=>saved.get(key),setItem:(...args)=>writes.push(args)};
  const palette=()=>[...initialColorPalette(initialDrawingColor(storage),storage)];
  assert.equal(initialDrawingColor(storage),'#EE4848');
  assert.deepEqual(palette(),['#EE4848','#F4C94D','#348BF1']);
  saved.set(colorPaletteKey,JSON.stringify(['#123456','#ABCDEF','#789ABC']));
  saved.set(colorHistoryKey,JSON.stringify(['#FFFFFF','#000000','#EE4848']));
  assert.equal(initialDrawingColor(storage),'#123456');
  assert.deepEqual(palette(),['#123456','#ABCDEF','#789ABC']);
  saved.delete(colorPaletteKey);
  assert.equal(initialDrawingColor(storage),'#FFFFFF');
  assert.deepEqual(palette(),['#FFFFFF','#000000','#EE4848']);
  saved.set(colorPaletteKey,'invalid json');saved.set(colorHistoryKey,'invalid json');
  assert.deepEqual(palette(),['#EE4848','#F4C94D','#348BF1']);
  assert.deepEqual(writes,[]);
});
test('toolbar keeps the current slot fixed and two distinct history slots without changing stored history',()=>{
  const {toolbarRecentColors,recentColors,rememberColor}=module.namespace;
  let stored='[]';const storage={getItem:()=>stored,setItem:(_key,value)=>stored=value};
  assert.deepEqual([...toolbarRecentColors([],'#EE4848')],['#EE4848','#F4C94D','#348BF1']);
  for(const color of ['#123456','#aabbcc','#FFFFFF','#123456'])rememberColor(color,storage);
  assert.deepEqual([...recentColors(storage)],['#123456','#FFFFFF','#AABBCC']);
  assert.deepEqual([...toolbarRecentColors(recentColors(storage),'#123456')],['#123456','#FFFFFF','#AABBCC']);
  assert.deepEqual([...toolbarRecentColors(['#bad','#348bf1','#348BF1'],'#F4C94D')],['#F4C94D','#348BF1','#EE4848']);
  assert.deepEqual([...toolbarRecentColors(recentColors(storage),'#E8ECF4')],['#E8ECF4','#123456','#FFFFFF']);
  assert.deepEqual([...recentColors(storage)],['#123456','#FFFFFF','#AABBCC']);
});
test('number labels remain readable on white, dark, saturated and threshold fills',()=>{
  for(const [fill,foreground] of [
    ['#FFFFFF','#000000'],['#000000','#FFFFFF'],['#242832','#FFFFFF'],
    ['#F4C94D','#000000'],['#00FF00','#000000'],['#0000FF','#FFFFFF'],
    ['#FF0000','#000000'],['#757575','#FFFFFF'],['#767676','#000000'],
  ]) assert.equal(numberForeground(fill),foreground,fill);
  // Black/white contrast cannot fall below 4.5 across the entire grey range.
  for(let channel=0;channel<=255;channel++){
    const fill='#'+channel.toString(16).padStart(2,'0').repeat(3);
    const s=channel/255, luminance=s<=.04045?s/12.92:((s+.055)/1.055)**2.4;
    const contrast=numberForeground(fill)==='#000000'?(luminance+.05)/.05:1.05/(luminance+.05);
    assert.ok(contrast>=4.5,fill);
  }
});
