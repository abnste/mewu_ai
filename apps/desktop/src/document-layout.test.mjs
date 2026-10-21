// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const source=stripTypeScriptTypes(await readFile(new URL('./document-layout.ts',import.meta.url),'utf8'));
const {documentBounds,documentDisplay}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
test('opaque frame geometry rejects nonfinite and unbounded reports',()=>{
  for(const v of [null,{}, {x:0,y:0,width:Infinity,height:90},{x:NaN,y:0,width:10,height:90},{x:32769,y:0,width:10,height:10},{x:0,y:0,width:0,height:10},{x:0,y:0,width:10,height:32769}])assert.equal(documentBounds(v),undefined);
  assert.deepEqual(documentBounds({x:-20,y:80,width:300,height:90,type:'layout',untrusted:true}),{x:-20,y:80,width:300,height:90});
});
test('authored sizes and full-screen dimensions are never forcibly scaled',()=>{
  for(const box of [{x:450,y:400,width:300,height:90},{x:0,y:0,width:1920,height:1080},{x:0,y:0,width:7680,height:4320}]){
    assert.deepEqual(documentDisplay(box),{width:box.width,height:box.height,scale:1});
  }
});
