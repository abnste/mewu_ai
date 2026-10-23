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
    runtime:{describe:async()=>({capabilities:['secrets.network','secrets.settings','memory.archive']})},
    secrets:{status:async()=>({configured:true,revision:1}),read:async()=> 'fixture-16-keyxx'},
    ui:{register:(d,h)=>services.set(d.id,h)},memory:{register:(d,h)=>services.set(d.id,h)},
    tools:{register:(d,h)=>services.set('tool.'+d.id,{descriptor:d,invoke:h})},
    host:{call:async(method,params)=>{assert.equal(method,'network.fetch');requests.push(params);return await respond(params);}}
  };
  return {ctx,values,services,requests,invoke:async(input,agentId='alice')=>services.get('memory')(input,{context:{agentId}})};
}
const json=body=>({status:200,body:JSON.stringify(body)});

test('only the settings request refills the actual key; memory operations keep using the opaque vault authorization',async()=>{
  const f=fixture(p=>p.url.endsWith('/recall')?json({results:[]}):json({success:true}));
  let reads=0;f.ctx.secrets.read=async(key,revision)=>{assert.equal(key,'api');assert.equal(revision,1);reads++;return 'a123456789012345';};
  await activate(f.ctx);
  assert.equal(reads,0);
  const settings=await f.services.get('settings')({type:'get'});
  assert.equal(settings.key,'a123456789012345');assert.equal(settings.key.length,16);
  await f.invoke({operation:'recall',query:'own fact'});
  assert.equal(reads,1);
  assert.ok(f.requests.every(request=>request.authorization?.key==='api'));
  assert.doesNotMatch(JSON.stringify(f.requests),/a123456789012345/);
});

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
  const ctx={configuration:{get:async k=>values.get(k)??null,compareAndSet:async(k,expected,value)=>{assert.deepEqual(values.get(k)??null,expected);calls.push('configuration');values.set(k,value);return true;}},secrets:{status:async()=>({revision:0})},host:{call:async(method,params)=>{calls.push(method);if(method==='network.fetch'){assert.equal(params.method,'GET');return params.url.endsWith('/version')?json({api_version:'1.1.0'}):{status:200,body:''};}if(method==='memory.archive.list')return {entries:[row],nextCursor:null};if(method==='memory.archive.credential'){assert.deepEqual(params,{bindingId:'old-binding',expectedRevision:7,secretRevision:0});return {configured:true};}assert.equal(method,'memory.archive.activate');assert.equal(values.get('archive.alice').bankId,'original-bank');return {ok:true};}}};
  assert.deepEqual(await migrate(ctx,{bindingId:'old-binding',expectedRevision:7}),{ok:true});
  assert.deepEqual(calls,['memory.archive.list','memory.archive.credential','network.fetch','network.fetch','configuration','memory.archive.activate']);
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
  assert.match((await f.invoke({operation:'retain',text:'blocked'})).error,/未允许保存/);
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

test('explicit memory tools share the provider handler without trusting a model-supplied origin',async()=>{
  const f=fixture(()=>json({success:true,bank_id:'mewu-alice'}));await activate(f.ctx);
  for(const operation of ['retain','recall','forget']){
    const tool=f.services.get('tool.'+operation);
    assert.deepEqual(tool.descriptor.metadata,{requiresMemory:true,requiresProvider:'memory'});
    assert.equal(tool.descriptor.inputSchema.additionalProperties,false);
    assert.equal(tool.descriptor.inputSchema.properties.memoryOrigin,undefined);
  }
  const saved=await f.services.get('tool.retain').invoke({text:'explicit fact',memoryOrigin:'completedTurn'},{context:{agentId:'alice'}});
  assert.equal(saved.retained,true);assert.equal(f.requests.length,1);
});

test('automatic retention respects opt-in and preserves the independent legacy automatic policy',async()=>{
  const f=fixture(()=>json({success:true,bank_id:'original-bank'}));await activate(f.ctx);
  const automatic=input=>f.services.get('memory')(input,{context:{agentId:'alice',memoryOrigin:'completedTurn'}});
  assert.deepEqual(await automatic({operation:'retain',text:'no automatic sync by default'}),{retained:false,skipped:true});
  assert.equal(f.requests.length,0);assert.equal(f.values.size,0);
  const archive={bindingId:'old',endpoint:'https://old.example',bankId:'original-bank',policy:{recall:true,explicitRetain:false,completedTurnSync:true}};
  f.ctx.configuration.get=async key=>key==='archive.alice'?archive:null;
  assert.equal((await automatic({operation:'retain',text:'allowed automatic fact'})).retained,true);
  assert.equal(f.requests.length,1);
  const denied=await f.services.get('tool.retain').invoke({text:'explicit fact',memoryOrigin:'completedTurn'},{context:{agentId:'alice'}});
  assert.equal(denied.isError,true);assert.match(denied.error,/未允许保存/);assert.equal(f.requests.length,1);
  archive.policy.completedTurnSync=false;
  assert.deepEqual(await automatic({operation:'retain',text:'disabled again'}),{retained:false,skipped:true});
  assert.equal(f.requests.length,1);
});

test('provider failures return a safe tool result for the parent Agent to inspect',async()=>{
  const f=fixture();f.ctx.configuration.get=async()=>null;await activate(f.ctx);
  const result=await f.services.get('tool.retain').invoke({text:'fact'},{context:{agentId:'alice'}});
  assert.equal(result.isError,true);assert.equal(result.retryable,false);
  assert.equal(result.error,'请配置 Hindsight 服务');assert.equal(f.requests.length,0);
});

test('migration refuses to change configuration or provider when the archived service is not ready',async()=>{
  const calls=[];
  const row={binding:{id:'old',agentId:'alice',revision:3,enabled:true,status:'ready',providerId:'hindsight',endpoint:'https://old.example',policy:{recall:true}},bankId:'original-bank',bindingRevision:2};
  const ctx={configuration:{get:async()=>null,compareAndSet:async()=>{throw Error('must not change configuration');}},secrets:{status:async()=>({revision:1,configured:true})},host:{call:async(method,params)=>{
    calls.push(method);
    if(method==='memory.archive.list')return {entries:[row],nextCursor:null};
    if(method==='memory.archive.credential')return {configured:true};
    assert.equal(method,'network.fetch');assert.equal(params.method,'GET');
    assert.deepEqual(params.authorization,{key:'archive.old'});
    return params.url.endsWith('/version')?json({api_version:'1.1.0'}):{status:503,body:'private server error'};
  }}};
  await assert.rejects(migrate(ctx,{bindingId:'old',expectedRevision:3}),/HTTP 503/);
  assert.deepEqual(calls,['memory.archive.list','memory.archive.credential','network.fetch','network.fetch']);
});
