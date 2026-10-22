// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {parse} from '@babel/parser';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const tick=()=>new Promise(setImmediate);
const deferred=()=>{let resolve;const promise=new Promise(done=>resolve=done);return {promise,resolve};};
const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const [name,value] of Object.entries(values))this.setExport(name,value);});

async function fixture(plugin){
  const source=await readFile(new URL('./components/ExtensionSurfaces.tsx',import.meta.url),'utf8');
  const ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
  const imports=ast.program.body.filter(node=>node.type==='ImportDeclaration');
  const fn=ast.program.body.find(node=>node.type==='FunctionDeclaration'&&node.id.name==='Surface');
  const returned=fn.body.body.find(node=>node.type==='ReturnStatement');
  const code=imports.map(node=>source.slice(node.start,node.end)).join('\n')+'\nconst innerWidth=1000,innerHeight=800;\n'+source.slice(fn.start,returned.start)+
    'return {close,mounted,open,setTrigger(value){trigger=value},publish(value){targets=value;notifyDropTargets()}};}\nexport default Surface;';
  const module=new SourceTextModule(stripTypeScriptTypes(code,{mode:'transform'}));
  const mount=deferred(),calls=[],errors=[];let receiver,flushed=0;
  const api={extensionNative:false,invokeExtension:async input=>{calls.push(input);return input.surface.action==='mount'?mount.promise:{saved:true};}};
  await module.link(name=>name==='solid-js'?synthetic(solid):name.endsWith('/extension-bridge')?synthetic(api):
    name.endsWith('/extension-object-drag')?synthetic({registerObjectDrop:value=>{receiver=value;return ()=>{receiver=undefined;};}}):
    synthetic(Object.fromEntries(imports.filter(node=>node.source.value===name).flatMap(node=>node.specifiers).map(spec=>[spec.type==='ImportDefaultSpecifier'?'default':spec.imported.name,()=>{}]))));
  await module.evaluate();
  let view,dispose;
  solid.createRoot(cleanup=>{dispose=cleanup;view=module.namespace.default({sceneId:'scene',disabled:false,left:16,onBeforeSave:async()=>{flushed++;},onWidth(){},onError:error=>errors.push(error),choice:{record:{manifest:{id:plugin},revision:4},service:{id:'view'}}});});
  view.setTrigger({getBoundingClientRect:()=>({left:10,top:10,right:42,bottom:42})});
  return {view,dispose,mount,calls,errors,get receiver(){return receiver;},get flushed(){return flushed;}};
}

test('the first launcher drop waits for an independently loaded plugin and its own declared default slot',async()=>{
  for(const plugin of ['independent.box','independent.library']){
    const f=await fixture(plugin);let removed=0;
    try{
      const drag=f.receiver.begin({source:{kind:'region',sceneId:'scene',regionId:'selected'},remove(){removed++;}});
      drag.move({x:20,y:20});assert.equal(drag.finish({x:20,y:20}),true);
      assert.equal(f.calls.length,1);
      f.mount.resolve({surfaceId:'surface',url:'http://127.0.0.1:1234/00000000-0000-4000-8000-000000000001',width:268,height:328});await tick();
      f.view.publish([{id:plugin+'-slot',x:0,y:0,width:80,height:80,default:true}]);await tick();
      assert.equal(f.calls.length,2);assert.equal(f.calls[1].input.target,plugin+'-slot');
      assert.equal(f.calls[1].pluginId,plugin);assert.equal(f.calls[1].objectSource.regionId,'selected');
      assert.equal(removed,0);assert.equal(f.flushed,1);assert.deepEqual(f.errors,[]);
    }finally{f.dispose();}
  }
});

test('closing an opening drop destination cancels its pending drop instead of writing to a later generation',async()=>{
  const f=await fixture('independent.closed');
  try{
    const drag=f.receiver.begin({source:{kind:'pin',id:'pin'},remove(){throw Error('must not remove');}});
    drag.move({x:20,y:20});assert.equal(drag.finish({x:20,y:20}),true);f.view.close();
    f.mount.resolve({surfaceId:'retired',url:'http://127.0.0.1:1234/00000000-0000-4000-8000-000000000001',width:268,height:328});await tick();
    assert.equal(f.calls.filter(value=>value.input?.type==='drop').length,0);assert.equal(f.flushed,0);
  }finally{f.dispose();}
});
