// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {parse} from '@babel/parser';
import vm from 'node:vm';
const ts=code=>stripTypeScriptTypes(code,{mode:'transform'});
function find(ast,match){let found;const walk=n=>{if(!n||typeof n!=='object')return;if(match(n))found=n;for(const value of Object.values(n))if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);};walk(ast);assert.ok(found);return found;}
const app=await readFile(new URL('./App.tsx',import.meta.url),'utf8'),component=await readFile(new URL('./components/Composer.tsx',import.meta.url),'utf8');
const options={sourceType:'module',plugins:['typescript','jsx']};
const ack=find(parse(app,options),n=>n.type==='VariableDeclarator'&&n.id.name==='referenceInserted').init;
const effect=find(parse(component,options),n=>n.type==='CallExpression'&&n.callee.name==='createEffect'&&n.arguments[0]?.callee?.name==='on'&&n.arguments[0].arguments[0]?.body?.property?.name==='referenceInsertion');
function fixture(){
 let request,changed=0,focused=0;
 const input={value:'比较前后',selectionStart:2,selectionEnd:2,setRangeText(text,start,end){this.value=this.value.slice(0,start)+text+this.value.slice(end);this.selectionStart=this.selectionEnd=start+text.length;},focus(){focused++;}};
 const context=vm.createContext({input,setReferenceInsertion:update=>{request=update(request);},referenceLabel:ref=>ref.kind==='region'?'图片1':'录屏.mp4',setHidden:()=>{},queueMicrotask:fn=>fn(),on:(dep,fn)=>()=>fn(dep()),createEffect:fn=>fn(),props:{scene:{id:'scene'},get referenceInsertion(){return request;},onDraft:()=>changed++}});
 context.props.onReferenceInserted=vm.runInContext(ts(app.slice(ack.start,ack.end)),context);
 const mount=()=>vm.runInContext(ts(component.slice(effect.start,effect.end)),context);
 return {input,mount,context,get changed(){return changed;},get focused(){return focused;},get request(){return request;},set request(value){request=value;}};
}
test('actual Composer consumes the insertion once, preserving the caret and following text across remounts',()=>{
 const f=fixture();f.request={sceneId:'scene',serial:1,reference:{kind:'region',id:'image'}};
 f.mount();assert.equal(f.input.value,'比较@图片1 前后');assert.equal(f.input.selectionStart,7);assert.equal(f.request,undefined);
 f.mount();assert.equal(f.input.value,'比较@图片1 前后');assert.equal(f.changed,1);assert.equal(f.focused,1);
 f.request={sceneId:'scene',serial:2,reference:{kind:'item',id:'video'}};
 f.context.props.onReferenceInserted(1);assert.equal(f.request.serial,2,'stale acknowledgments cannot consume a newer request');
 f.mount();assert.equal(f.input.value,'比较@图片1 @录屏.mp4 前后');assert.equal(f.changed,2);assert.equal(f.request,undefined);
});
test('reference requests cannot insert into another conversation',()=>{
 const f=fixture();f.request={sceneId:'other',serial:1,reference:{kind:'region',id:'image'}};f.mount();assert.equal(f.input.value,'比较前后');assert.equal(f.changed,0);assert.equal(f.request.serial,1);
});
