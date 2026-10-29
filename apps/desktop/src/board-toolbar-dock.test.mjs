// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const {dockBoardToolbar,boardDockAt,boardDockPreview}=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./board-toolbar-dock.ts',import.meta.url),'utf8'))).toString('base64')}`);
test('release near each of four edges snaps, center detaches',()=>{
 const viewport={width:1000,height:700};
 for(const [point,expected] of [[{x:10,y:300},'left'],[{x:995,y:300},'right'],[{x:500,y:4},'top'],[{x:500,y:695},'bottom'],[{x:500,y:350},'free']])assert.equal(boardDockAt(point,viewport),expected);
});
test('dock placement accommodates orientation changes and viewport resize',()=>{
 const viewport={width:1000,height:700};
 assert.deepEqual(dockBoardToolbar({edge:'right',x:950,y:550},{width:64,height:500},viewport),{x:928,y:194});
 assert.deepEqual(dockBoardToolbar({edge:'bottom',x:500,y:600},{width:640,height:64},viewport),{x:354,y:620});
 const small=dockBoardToolbar({edge:'free',x:-50,y:2000},{width:100,height:64},{width:300,height:240});
 assert.deepEqual(small,{x:6,y:170});
});

test('drag preview rotates to the destination edge and stays inside the work area',()=>{
 const p={edge:'free',x:920,y:650},size={width:680,height:64},vp={width:1000,height:700};
 assert.equal(boardDockPreview(p,size,vp,'free',false),undefined);
 for(const edge of ['left','right','top','bottom']){
  const hint=boardDockPreview(p,size,vp,edge,false);
  assert.ok(hint.x>=6&&hint.y>=6&&hint.x+hint.width<=vp.width-6&&hint.y+hint.height<=vp.height-6);
  assert.equal(hint.width,edge==='left'||edge==='right'?64:680);
  assert.equal(hint.height,edge==='left'||edge==='right'?680:64);
 }
 const back=boardDockPreview(p,{width:64,height:680},vp,'bottom',true);
 assert.equal(back.width,680);assert.equal(back.height,64);
});
