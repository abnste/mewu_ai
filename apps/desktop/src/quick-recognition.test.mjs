// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {createContext,runInContext} from 'node:vm';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
const source=await readFile(new URL('./quick-interaction.ts',import.meta.url),'utf8');
const ast=parse(source,{sourceType:'module',plugins:['typescript']});
const fn=ast.program.body.find(n=>n.type==='ExportNamedDeclaration'&&n.declaration?.id?.name==='recognizeQuickInteraction').declaration;
const code=stripTypeScriptTypes(source.slice(fn.start,fn.end),{mode:'transform'});

test('native OCR owns recovery without duplicate renderer dispatch',async()=>{
  const calls=[];const doc={width:100,height:100,lines:[{text:'TEXT COPY'}]};
  const context=createContext({invoke:async(...args)=>{calls.push(args);return doc;}});
  runInContext(code,context);
  assert.equal(await context.recognizeQuickInteraction(17,()=>true),doc);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)),[['quick_interaction_recognize',{generation:17}]]);
});

test('closed, superseded or revoked OCR owners do not trigger a replacement read',async()=>{
  let calls=0,finish;const failed=new Promise((_,reject)=>finish=()=>reject(Error('old request')));
  const context=createContext({invoke:()=>{calls++;return failed;}});runInContext(code,context);
  let current=true;const reading=context.recognizeQuickInteraction(3,()=>current);current=false;finish();
  await assert.rejects(reading,/old request/);assert.equal(calls,1);
});

test('native failure is reported once without resetting its original recognition budget',async()=>{
  let calls=0;const context=createContext({invoke:async()=>{calls++;throw Error('native failure');}});runInContext(code,context);
  await assert.rejects(context.recognizeQuickInteraction(8,()=>true),/native failure/);assert.equal(calls,1);
});

test('an already retired owner is rejected before native dispatch',async()=>{
  const context=createContext({invoke:()=>assert.fail('Retired owners cannot start OCR')});runInContext(code,context);
  await assert.rejects(context.recognizeQuickInteraction(8,()=>false),/快速交互已切换/);
});
