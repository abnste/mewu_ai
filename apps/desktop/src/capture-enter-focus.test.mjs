// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {parse} from '@babel/parser';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';

test('finishing a selection keeps Enter with the screenshot; only explicit composer focus requests take focus',async()=>{
  const source=await readFile(new URL('./App.tsx',import.meta.url),'utf8');
  const nodes=[];
  function walk(node){if(!node||typeof node!=='object')return;if(node.type==='FunctionDeclaration')nodes.push(node);for(const [key,value] of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}}
  walk(parse(source,{sourceType:'module',plugins:['typescript','jsx']}));
  const handler=nodes.find(node=>node.id?.name==='selectionCompleted');assert.ok(handler);
  const completed=[],focus=[];let recording=false;
  const select=runInNewContext(stripTypeScriptTypes(source.slice(handler.start,handler.end),{mode:'transform'})+';selectionCompleted',{
    recordingSelection:{completed:id=>{completed.push(id);return recording;}},setFocusRequest:value=>focus.push(value),
  });
  select({id:'capture'});recording=true;select({id:'record'});
  assert.deepEqual(completed,['capture','record']);assert.deepEqual(focus,[]);
});
