// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import activate from '../../../extensions/official/hindsight/index.mjs';
import {api,probe} from '../../../extensions/official/hindsight/transport.mjs';

const connection={endpoint:'https://memory.example.test',bankPrefix:'mewu'};
function fixture(respond=()=>({status:200,body:'{}'})) {
  const values=new Map(),services=new Map(),requests=[];
  const storage={
    get:async key=>structuredClone(values.get(key)??null),
    set:async(key,value)=>{values.set(key,structuredClone(value));},
    compareAndSet:async(key,old,value)=>{
      if(JSON.stringify(values.get(key)??null)!==JSON.stringify(old))return false;
      values.set(key,structuredClone(value));return true;
    }
  };
  const ctx={storage,configuration:{get:async()=>connection},
    runtime:{describe:async()=>({capabilities:['secrets.network']})},
    secrets:{status:async()=>({configured:true,revision:1})},
    ui:{register:(d,h)=>services.set(d.id,h)},memory:{register:(d,h)=>services.set(d.id,h)},
    host:{call:async(method,params)=>{assert.equal(method,'network.fetch');requests.push(params);return await respond(params);}}
  };
  return {ctx,values,services,requests,invoke:async(input,agentId='alice')=>services.get('memory')(input,{context:{agentId}})};
}
const json=body=>({status:200,body:JSON.stringify(body)});

test('Hindsight probe uses semantic server version and readiness HTTP status',async()=>{
  const f=fixture(p=>p.url.endsWith('/version')?json({api_version:'0.10.3',features:{}}):{status:200,body:'null'});
  assert.deepEqual(await probe(f.ctx,connection),{ok:true,version:'0.10.3'});
  assert.equal(f.requests.length,2);
  assert.deepEqual(f.requests[0].authorization,{key:'api'});
  assert.equal(f.requests[0].headers.Authorization,undefined);
});
test('Hindsight never retries a failed mutation or exposes server error details',async()=>{
  const f=fixture(()=>{throw Error('server-secret-value');});await activate(f.ctx);
  const result=await f.invoke({operation:'retain',text:'remember'});
  assert.equal(result.state,'unknown');assert.equal(result.retryable,false);
  assert.equal(f.requests.length,1);assert.doesNotMatch(JSON.stringify(result),/server-secret/);
});
test('Hindsight only recalls committed documents owned by the current agent',async()=>{
  let id;
  const f=fixture(p=>p.url.endsWith('/recall')?json({results:[{document_id:id,text:'owned'},{document_id:'foreign',text:'other'}]}):json({success:true,bank_id:'mewu-alice'}));
  await activate(f.ctx);id=(await f.invoke({operation:'retain',text:'first'})).id;
  assert.deepEqual(await f.invoke({operation:'recall',query:'first'}),{memories:[{id,text:'owned'}]});
  assert.deepEqual(await f.invoke({operation:'recall',query:'first'},'bob'),{memories:[]});
  for(const [key,value] of f.values)if(value?.id===id)f.values.set(key,{...value,state:'unknown'});
  assert.deepEqual(await f.invoke({operation:'recall',query:'first'}),{memories:[]});
});
test('forget during retain cannot resurrect a late success or overwrite its tombstone',async()=>{
  let release,started;
  const arrived=new Promise(resolve=>{started=resolve;});
  const f=fixture(async p=>{
    if(p.method==='DELETE')return json({success:true,document_id:p.url.split('/').at(-1)});
    started();await new Promise(resolve=>{release=resolve;});return json({success:true,bank_id:'mewu-alice'});
  });
  await activate(f.ctx);const pending=f.invoke({operation:'retain',text:'forget while writing'});await arrived;
  const receipt=[...f.values.values()].find(v=>v?.state==='pending');
  await f.invoke({operation:'forget',id:receipt.id});release();
  assert.deepEqual(await pending,{id:receipt.id,retained:false,forgotten:true});
  assert.equal([...f.values.values()].find(v=>v?.id===receipt.id).forgotten,true);
});
test('readiness rejects non-success and credentials stay host-managed',async()=>{
  const f=fixture(()=>({status:503,body:'internal private traceback'}));
  await assert.rejects(api(f.ctx,connection)('GET',['health','ready'],undefined,{readiness:true}),/^Error: 服务返回 HTTP 503$/u);
  assert.deepEqual(f.requests[0].authorization,{key:'api'});
});
