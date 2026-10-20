// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {html} from '../../../extensions/examples/research-notes/view.mjs';

function editor(){
  const nodes=new Map(),events=new Map(),calls=[];
  let beforeClose,value={revision:0,state:{text:'saved',references:[]},objects:{}},fail=false;
  function element(){return {value:'',disabled:true,readOnly:false,textContent:'',listeners:new Map(),classList:{toggle(){}},replaceChildren(){},append(){},
    getBoundingClientRect(){return {x:0,y:200,width:300,height:100};},
    addEventListener(name,fn){this.listeners.set(name,fn);}};}
  for(const id of ['textarea','#refs','#error','#save'])nodes.set(id,element());
  const api={ready:Promise.resolve(),setDropTargets(){},on(name,fn){events.set(name,fn);},onBeforeClose(fn){beforeClose=fn;},
    async call(input){
      calls.push(structuredClone(input));
      if(input.type==='save'){
        if(fail)throw Error('disk unavailable');
        value={...value,revision:value.revision+1,state:{...value.state,text:input.text}};
      }
      return structuredClone(value);
    }};
  vm.runInNewContext(html.slice(html.indexOf('<script>')+8,html.indexOf('</script>')),{
    document:{querySelector:key=>nodes.get(key),createElement:element},mewu:api,setTimeout,clearTimeout,addEventListener(){}
  });
  return {nodes,calls,events,close:()=>beforeClose(),value:()=>value,setFailure:v=>{fail=v;}};
}
test('notes closes only after the latest typed text is persisted and never replaces a newer draft',async()=>{
  const app=editor();await new Promise(setImmediate);
  const text=app.nodes.get('textarea');assert.equal(text.value,'saved');assert.equal(text.disabled,false);
  text.value='new unsaved text';text.listeners.get('input')();
  await app.close();
  assert.equal(app.value().state.text,'new unsaved text');assert.equal(text.readOnly,true);
  assert.equal(app.calls.filter(c=>c.type==='save').length,1);
});
test('failed note close leaves the edited text available; retry persists it',async()=>{
  const app=editor();await new Promise(setImmediate);
  const text=app.nodes.get('textarea');text.value='must not disappear';app.setFailure(true);
  await assert.rejects(app.close(),/disk unavailable/);
  assert.equal(text.value,'must not disappear');assert.equal(text.readOnly,false);
  assert.equal(app.value().state.text,'saved');
  app.setFailure(false);await app.close();assert.equal(app.value().state.text,'must not disappear');
});
