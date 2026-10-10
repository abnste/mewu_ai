// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
const reasoning=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./reply-reasoning.ts',import.meta.url),'utf8'),{mode:'transform'}));await reasoning.link(()=>{throw Error('dependency');});await reasoning.evaluate();
const preview=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./frozen-preview.ts',import.meta.url),'utf8'),{mode:'transform'}));await preview.link(()=>new SyntheticModule(Object.keys(reasoning.namespace),function(){for(const key of Object.keys(reasoning.namespace))this.setExport(key,reasoning.namespace[key]);}));await preview.evaluate();
const {livePreview,liveSnippet,previewSnippet,mergeLivePreview}=preview.namespace;
test('live widget switches reasoning to answer, resets by run, and final preview starts at the beginning',()=>{
  let value=livePreview(undefined,{runId:'a',reasoning:'正在核对图中的坐标'});assert.equal(liveSnippet(value),'正在核对图中的坐标');
  value=livePreview(value,{runId:'a',text:'答案：'});value=livePreview(value,{runId:'a',text:'42'});assert.equal(liveSnippet(value),'答案：42');
  value=livePreview(value,{runId:'b',reasoning:'新问题'});assert.equal(liveSnippet(value),'新问题');
  assert.equal(previewSnippet('🙂'.repeat(100)),'🙂'.repeat(90)+'...');
  value=livePreview(undefined,{runId:'emoji',text:'🙂'.repeat(1700)});assert.equal(Array.from(value.text).length,1600);assert.ok(value.text.endsWith('🙂'));
});
test('transient widget preview cache is bounded and retains the most recently streamed scene',()=>{
  let values=new Map();for(let n=0;n<140;n++)values=mergeLivePreview(values,{sceneId:String(n),runId:'a',text:'x'});
  assert.equal(values.size,128);assert.equal(values.get('0'),undefined);
  values=mergeLivePreview(values,{sceneId:'12',runId:'a',text:'y'});values=mergeLivePreview(values,{sceneId:'new',runId:'a',text:'z'});
  assert.equal(values.get('12').text,'xy');assert.equal(values.get('13'),undefined);
});
test('long streaming answers continue following new chunks before returning to the final answer beginning',()=>{
  const finalAnswer='答案开头：'+'🙂'.repeat(1700)+'正在补充';
  let value=livePreview(undefined,{runId:'long',reasoning:'先检查来源'});
  value=livePreview(value,{runId:'long',text:finalAnswer});
  assert.equal(liveSnippet(value),'🙂'.repeat(86)+'正在补充');
  value=livePreview(value,{runId:'long',text:'，最后一段'});
  assert.equal(liveSnippet(value),'🙂'.repeat(81)+'正在补充，最后一段');
  assert.equal(Array.from(value.text).length,1600);
  assert.equal(previewSnippet(finalAnswer+'，最后一段'),'答案开头：'+'🙂'.repeat(85)+'...');
});
test('lower placement stays near the video and avoids the composer and object actions',async()=>{
  const module=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./video-placement.ts',import.meta.url),'utf8'),{mode:'transform'}));await module.link(()=>{throw Error('dependency');});await module.evaluate();
  const box={left:200,top:200,width:640,height:360},screen={width:1280,height:900};
  const controls={left:200,top:568,width:328,height:62},composer={left:320,top:800,width:640,height:100};
  const placed=module.namespace.placeVideoControls(box,screen,108,composer,controls);
  assert.equal(placed.top,controls.top+controls.height+6);assert.ok(placed.top+108<composer.top);
});
