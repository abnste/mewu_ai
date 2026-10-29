// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import vm from 'node:vm';
import {parse} from '@babel/parser';
import test from 'node:test';
const code=await readFile(new URL('./components/SharedDrawingEditor.tsx',import.meta.url),'utf8');
const body=parse(code,{sourceType:'module',plugins:['typescript','jsx']}).program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration.body.body;
const method=body.find(n=>n.type==='FunctionDeclaration'&&n.id.name==='finishTextAt');
const actual=stripTypeScriptTypes(code.slice(method.start,method.end),{mode:'transform'});
test('inline canvas click waits for text receipt, opens next text only in the same source, never after failure or lock',async()=>{
  for(const mode of ['saved','failure','source','geometry','scene','disposed','locked','tool']){
    let resolve;const response=new Promise(r=>resolve=r);const events=[];
    const ctx=vm.createContext({JSON,disposed:false,locked:false,scene:'a',source:'a',mode:'text',
      props:{box:{x:40,y:30,width:800,height:500},port:{sourceIdentity:()=>ctx.source,frame:()=>({x:10,y:20,width:800,height:500})}},
      identity:()=>ctx.scene,tool:()=>ctx.mode,disabled:()=>ctx.locked,saveText:()=>response,begin:event=>events.push(event)});
    vm.runInContext(actual,ctx); const event={clientX:400,clientY:180};
    const work=ctx.finishTextAt(event); assert.equal(events.length,0);
    if(mode==='source')ctx.source='b';if(mode==='geometry')ctx.props.box.width=700;if(mode==='scene')ctx.scene='b';
    if(mode==='disposed')ctx.disposed=true;if(mode==='locked')ctx.locked=true;if(mode==='tool')ctx.mode='pen';
    resolve(mode!=='failure');await work;
    assert.deepEqual(events,mode==='saved'?[event]:[],mode);
  }
});
test('moving focus into text styling, including the color portal, never submits the draft early',()=>{
  class Node{};class Element extends Node {constructor(domain){super();this.domain=domain;}closest(){return this.domain==='palette'?this:null;}}
  const blur=body.find(n=>n.type==='FunctionDeclaration'&&n.id.name==='textBlur');let saves=0;
  const ctx=vm.createContext({Node,Element,toolbar:{contains:node=>node.domain==='toolbar'},saveText:()=>{saves++;return Promise.resolve(true);}});
  vm.runInContext(stripTypeScriptTypes(code.slice(blur.start,blur.end),{mode:'transform'}),ctx);
  for(const relatedTarget of [null,new Element('toolbar'),new Element('palette')])ctx.textBlur({relatedTarget});
  assert.equal(saves,0);ctx.textBlur({relatedTarget:new Element('outside')});assert.equal(saves,1);
});
const inputCode=await readFile(new URL('./components/InlineDrawingText.tsx',import.meta.url),'utf8');
const inputAst=parse(inputCode,{sourceType:'module',plugins:['typescript','jsx']});
let keydown;
function walk(node){if(!node||typeof node!=='object')return;if(node.type==='ObjectMethod'&&node.key.name==='handleKeyDown')keydown=node;for(const value of Object.values(node)){if(Array.isArray(value))value.forEach(walk);else walk(value);}}
walk(inputAst.program);
test('inline text keeps Enter for newlines and IME confirmation; Ctrl Enter commits once',()=>{
  let commits=0;const ctx=vm.createContext({props:{onConfirm:()=>commits++}});
  const handle=vm.runInContext(stripTypeScriptTypes('({'+inputCode.slice(keydown.start,keydown.end)+'}).handleKeyDown',{mode:'transform'}),ctx);
  for(const extra of [{},{shiftKey:true},{ctrlKey:true,isComposing:true},{ctrlKey:true,keyCode:229}]){
    let prevented=false;handle(null,{key:'Enter',preventDefault(){prevented=true;},stopPropagation(){},...extra});assert.equal(prevented,false);assert.equal(commits,0);
  }
  let prevented=false,stopped=false;
  handle(null,{key:'Enter',ctrlKey:true,preventDefault(){prevented=true;},stopPropagation(){stopped=true;}});
  assert.equal(commits,1);assert.ok(prevented&&stopped);
});
