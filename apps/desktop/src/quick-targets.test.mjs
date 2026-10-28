// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule} from 'node:vm';
import test from 'node:test';
const mod=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./quick-targets.ts',import.meta.url),'utf8'),{mode:'transform'}));await mod.link(()=>{throw Error('unexpected dependency')});await mod.evaluate();const{quickImageTarget}=mod.namespace;
test('native image selection uses the smallest actual image and never creates a native input witness',()=>{const large={x:5,y:7,width:200,height:100},small={x:20,y:30,width:40,height:25};const selected=quickImageTarget([large,small],300,200,32,42);assert.deepEqual(JSON.parse(JSON.stringify(selected.bounds)),small);assert.equal(selected.snapshot,false);assert.equal(selected.children.length,0);assert.equal(quickImageTarget([large],300,200,290,190),undefined);});
test('out of frame, malformed and over-budget metadata cannot turn into an image target',()=>{for(const bounds of [{x:-1,y:0,width:30,height:30},{x:0,y:0,width:Infinity,height:30},{x:0,y:0,width:301,height:30},{x:0,y:0,width:1.5,height:30}])assert.equal(quickImageTarget([bounds],300,200,0,0),undefined);assert.equal(quickImageTarget(Array(129).fill({x:0,y:0,width:30,height:30}),300,200,1,1),undefined);});
