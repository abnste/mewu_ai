// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import activate from '../../../extensions/official/hindsight/index.mjs';
import {api,probe} from '../../../extensions/official/hindsight/transport.mjs';
import {migrate} from '../../../extensions/official/hindsight/migration.mjs';

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
  const ctx={storage,configuration:{get:async key=>key==='connection'?connection:null},
    runtime:{describe:async()=>({capabilities:['secrets.network','memory.archive']})},
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
test('settings probe tests the unsaved draft without persisting its key or using the saved server',async()=>{
  const f=fixture(p=>p.url.endsWith('/version')?json({api_version:'0.10.3'}):{status:200,body:''});
  await activate(f.ctx);
  const draft={endpoint:'https://draft.example.test',bankPrefix:'draft'};
  assert.equal((await f.services.get('settings')({type:'test',config:draft,key:'test-only-key'})).ok,true);
  assert.equal(f.values.size,0);
  assert.ok(f.requests.every(r=>r.url.startsWith(draft.endpoint+'/')&&r.headers.Authorization==='Bearer test-only-key'&&!r.authorization));
  f.requests.length=0;
  assert.equal((await f.services.get('settings')({type:'test',config:draft,key:''})).ok,true);
  assert.ok(f.requests.every(r=>!r.headers.Authorization&&!r.authorization));
  f.requests.length=0;
  assert.equal((await f.services.get('settings')({type:'test',config:draft,key:'bad\nkey'})).ok,false);
  assert.equal(f.requests.length,0);
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

test('archive migration keeps the exact bank, endpoint and policy; publishes binding last without replay',async()=>{
  const calls=[],values=new Map();
  const row={binding:{id:'old-binding',agentId:'alice',revision:7,enabled:true,status:'ready',providerId:'hindsight',endpoint:'https://old.example',policy:{recall:true,explicitRetain:false,completedTurnSync:false}},bankId:'original-bank',bindingRevision:3};
  const ctx={configuration:{get:async k=>values.get(k)??null,compareAndSet:async(k,expected,value)=>{assert.deepEqual(values.get(k)??null,expected);calls.push('configuration');values.set(k,value);return true;}},secrets:{status:async()=>({revision:0})},host:{call:async(method,params)=>{calls.push(method);if(method==='memory.archive.list')return {entries:[row],nextCursor:null};if(method==='memory.archive.credential'){assert.deepEqual(params,{bindingId:'old-binding',expectedRevision:7,secretRevision:0});return {configured:true};}assert.equal(method,'memory.archive.activate');assert.equal(values.get('archive.alice').bankId,'original-bank');return {ok:true};}}};
  assert.deepEqual(await migrate(ctx,{bindingId:'old-binding',expectedRevision:7}),{ok:true});
  assert.deepEqual(calls,['memory.archive.list','memory.archive.credential','configuration','memory.archive.activate']);
  assert.deepEqual(values.get('archive.alice'),{bindingId:'old-binding',endpoint:row.binding.endpoint,bankId:'original-bank',policy:row.binding.policy});
  await assert.rejects(migrate(ctx,{bindingId:'old-binding',expectedRevision:6}),/旧连接已变化/);
});

test('migrated recall filters old tombstones and uncertain writes while preserving original remote route',async()=>{
  const f=fixture(p=>json({results:[{document_id:'committed',text:'kept'},{document_id:'forgotten',text:'deleted'},{document_id:'unknown',text:'uncertain'},{document_id:'foreign',text:'foreign'}]}));
  f.ctx.configuration.get=async key=>key==='archive.alice'?{bindingId:'old',endpoint:'https://old.example',bankId:'original-bank',policy:{recall:true,explicitRetain:false}}:null;
  const fetch=f.ctx.host.call;
  f.ctx.host.call=async(method,p)=>method==='memory.archive.lookup'?[{documentId:'committed',evidence:{id:'e1',state:'committed'}},{documentId:'forgotten',evidence:{id:'e2',state:'suppressed',forgotten:{localSuppressed:true}}},{documentId:'unknown',evidence:{id:'e3',state:'unknown'}}]:fetch(method,p);
  await activate(f.ctx);
  assert.deepEqual(await f.invoke({operation:'recall',query:'old'}),{memories:[{id:'committed',text:'kept'}]});
  assert.equal(f.requests[0].url,'https://old.example/v1/default/banks/original-bank/memories/recall');
  assert.deepEqual(f.requests[0].authorization,{key:'archive.old'});
  await assert.rejects(f.invoke({operation:'retain',text:'blocked'}),/未允许保存/);
  assert.equal(f.requests.length,1);
});

test('forgetting an imported document durably suppresses its old ledger before remote deletion',async()=>{
  const calls=[],f=fixture(p=>{calls.push('network');return json({success:true,document_id:'old-doc'});});
  f.ctx.configuration.get=async key=>key==='archive.alice'?{bindingId:'old',endpoint:'https://old.example',bankId:'original-bank',policy:{recall:true,explicitRetain:true}}:null;
  const fetch=f.ctx.host.call;
  f.ctx.host.call=async(method,p)=>{if(method==='memory.archive.lookup')return [{documentId:'old-doc',evidence:{id:'e1',revision:4,state:'committed'}}];if(method==='memory.archive.forget'){calls.push('tombstone');assert.equal(p.evidenceRevision,4);return {};}return fetch(method,p);};
  await activate(f.ctx);assert.deepEqual(await f.invoke({operation:'forget',id:'old-doc'}),{forgotten:true,id:'old-doc'});
  assert.deepEqual(calls,['tombstone','network']);
  assert.equal([...f.values.values()].find(v=>v.id==='old-doc').forgotten,true);
});
