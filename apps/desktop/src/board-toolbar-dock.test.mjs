// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const {dockBoardToolbar,boardDockAt}=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./board-toolbar-dock.ts',import.meta.url),'utf8'))).toString('base64')}`);
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
