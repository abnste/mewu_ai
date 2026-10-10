// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const defer=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};};
const object={id:'12345678-1234-4234-9234-123456789abc',imageUrl:'http://127.0.0.1:4444/image',width:300,height:200,quarterTurns:0,opacity:1,topmost:true,attachmentAssetId:null,x:.1,y:.2,displayWidth:.3,displayHeight:.2};
async function load(native=true){
 const calls=[],queries=[],events=new Map();let removed=0;
 const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[key,value]of Object.entries(values))this.setExport(key,value);});
 const imports={'@tauri-apps/api/core':synthetic({invoke:async(name,args)=>{calls.push({name,args});if(name==='get_pin_objects'){const d=defer();queries.push(d);return d.promise;}return null;}}),'@tauri-apps/api/event':synthetic({listen:async(name,callback)=>{events.set(name,callback);return()=>{removed++;events.delete(name);};}}),'./bridge':synthetic({native})};
 const module=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./pin-objects.ts',import.meta.url),'utf8'),{mode:'transform'}));await module.link(name=>imports[name]);await module.evaluate();return{api:module.namespace,calls,queries,events,removed:()=>removed};
}
test('a newer close notification wins over a late initial object query',async()=>{
 const value=await load(),accepted=[],errors=[];const sub=value.api.subscribePinObjects(v=>accepted.push(JSON.parse(JSON.stringify(v))),e=>errors.push(e));await new Promise(setImmediate);
 value.events.get('pin-objects-changed')({payload:'untrusted'});value.queries[1].resolve([]);await new Promise(setImmediate);value.queries[0].resolve([object]);const handle=await sub;
 assert.deepEqual(accepted,[[]]);assert.deepEqual(errors,[]);handle.stop();assert.equal(value.removed(),1);
});
test('disposed object subscriptions reject late responses and unregister',async()=>{
 const value=await load(),accepted=[];const sub=value.api.subscribePinObjects(v=>accepted.push(v),()=>assert.fail());await new Promise(setImmediate);value.queries[0].resolve([]);const handle=await sub;
 const refresh=handle.refresh();handle.stop();value.queries[1].resolve([object]);await refresh;assert.equal(accepted.length,1);assert.equal(value.removed(),1);
});
test('object identity, geometry, and local media are validated',async()=>{
 const{api}=await load();assert.equal(api.validPinObject(object),true);for(const patch of[{imageUrl:'file:///private.png'},{imageUrl:'https://remote/image'},{x:NaN},{displayWidth:0},{width:1.5},{quarterTurns:4}])assert.equal(api.validPinObject({...object,...patch}),false);
});
test('reference and movement carry only host IDs, not renderer file paths',async()=>{
 const value=await load();await value.api.referencePinObject(object.id,'scene');await value.api.referencePinObject(null,null);await value.api.controlPinObject(object.id,{type:'close'});
 assert.deepEqual(JSON.parse(JSON.stringify(value.calls)),[{name:'reference_pin_object',args:{id:object.id,sceneId:'scene'}},{name:'reference_pin_object',args:{id:null,sceneId:null}},{name:'control_pin_object',args:{id:object.id,action:{type:'close'}}}]);
});
test('browser preview does not fabricate desktop pinned object success',async()=>{
 const value=await load(false);const handle=await value.api.subscribePinObjects(()=>assert.fail(),()=>assert.fail());await handle.refresh();handle.stop();await assert.rejects(value.api.referencePinObject(object.id,'scene'),/桌面版/);assert.deepEqual(value.calls,[]);
});
