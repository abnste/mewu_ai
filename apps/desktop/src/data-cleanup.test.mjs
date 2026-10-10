// Production client/controller with isolated receipts; no native user-data deletion.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
const read=path=>readFile(new URL(path,import.meta.url),'utf8');
const compile=text=>stripTypeScriptTypes(text,{mode:'transform'});
const reactive=await import(new URL('../../../node_modules/solid-js/dist/solid.js',import.meta.url));
const solid=new SyntheticModule(Object.keys(reactive),function(){for(const [key,value]of Object.entries(reactive))this.setExport(key,value);});
const contracts=new SourceTextModule(compile(await read('./settings-contracts.ts')));await contracts.link(()=>{throw Error('Unexpected import');});await contracts.evaluate();
const client=new SourceTextModule(compile(await read('./settings-client.ts')));await client.link(()=>contracts);await client.evaluate();
const controller=new SourceTextModule(compile(await read('./data-cleanup.ts')));await controller.link(()=>solid);await controller.evaluate();
const {createDataCleanup,formatStorageBytes}=controller.namespace;
const bucket=category=>({category,count:10,bytes:1000,cleanableCount:4,cleanableBytes:400,token:'a'.repeat(64)});
const usage=()=>({files:bucket('files'),screenshots:bucket('screenshots'),conversations:bucket('conversations'),databaseBytes:4096,canClean:true});
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{resolve,reject,promise};};
function mount(api){let data,dispose,setActive;reactive.createRoot(stop=>{dispose=stop;const [active,set]=reactive.createSignal(true);setActive=set;data=createDataCleanup(api,active);});return{data,dispose,setActive};}
test('usage rejects mixed categories, unsafe sizes and more cleanable bytes/items than exist',()=>{
  assert.equal(contracts.namespace.validDataUsage(usage()),true);
  for(const patch of [{bytes:NaN},{cleanableBytes:1001},{cleanableCount:11},{count:-1},{token:'path'},{category:'conversations'}]){const value=usage();Object.assign(value.files,patch);assert.equal(contracts.namespace.validDataUsage(value),false);}
  assert.equal(formatStorageBytes(0),'0 B');assert.equal(formatStorageBytes(1024),'1 KiB');assert.equal(formatStorageBytes(1536),'1.5 KiB');
});
test('cleanup IPC accepts only a category and exact opaque review token, never filesystem paths',async()=>{
  const calls=[],api=client.namespace.settingsClient({listen:async()=>()=>{},invoke:async(command,args)=>{calls.push({command,args});return command==='get_data_usage'?usage():{removedCount:4,removedBytes:400,failedCount:0,compacted:true,usage:usage()};}});
  await api.dataUsage();await api.cleanData('files','a'.repeat(64));await assert.rejects(api.cleanData('D:/user/data','a'.repeat(64)));await assert.rejects(api.cleanData('files','invalid'));
  assert.deepEqual(JSON.parse(JSON.stringify(calls)),[{command:'get_data_usage'},{command:'clean_data',args:{category:'files',expectedToken:'a'.repeat(64)}}]);
});
test('preview and cancellation never delete; confirmation uses the reviewed category and refreshes returned counts',async()=>{
  let deletes=0;const after=usage();after.files.count=6;after.files.bytes=600;after.files.cleanableCount=0;after.files.cleanableBytes=0;
  const m=mount({dataUsage:async()=>usage(),cleanData:async(category,token)=>{deletes++;assert.equal(category,'files');assert.equal(token,'a'.repeat(64));return{removedCount:4,removedBytes:400,failedCount:0,compacted:true,usage:after};}});
  await m.data.load();m.data.choose('files');assert.equal(deletes,0);m.data.cancel();await m.data.confirm();assert.equal(deletes,0);
  m.data.choose('files');await m.data.confirm();assert.equal(deletes,1);assert.equal(m.data.usage().files.count,6);assert.equal(m.data.review(),undefined);m.dispose();
});
test('pending cleanup serializes clicks; a stale or failed cleanup only reads fresh state and never retries deletion',async()=>{
  let deletes=0,reads=0;const pending=deferred();const m=mount({dataUsage:async()=>{reads++;return usage();},cleanData:()=>{deletes++;return pending.promise;}});
  await m.data.load();m.data.choose('conversations');const first=m.data.confirm();await m.data.confirm();m.data.choose('files');assert.equal(deletes,1);
  pending.reject(Error('清理范围已变化'));await first;assert.equal(deletes,1);assert.equal(reads,2);assert.match(m.data.error(),/已变化/);assert.equal(m.data.review(),undefined);m.dispose();
});
test('inactive page, busy host and unmounted late result cannot start a cleanup or replace visible state',async()=>{
  let deletes=0;const m=mount({dataUsage:async()=>({...usage(),canClean:false}),cleanData:async()=>deletes++});await m.data.load();m.data.choose('files');assert.equal(m.data.review(),undefined);m.dispose();
  const delayed=deferred(),n=mount({dataUsage:()=>delayed.promise,cleanData:async()=>deletes++});const read=n.data.load();n.dispose();delayed.resolve(usage());await read;assert.equal(n.data.usage(),undefined);
  const k=mount({dataUsage:async()=>usage(),cleanData:async()=>deletes++});await k.data.load();k.data.choose('files');k.setActive(false);await k.data.confirm();assert.equal(deletes,0);k.dispose();
});
test('destructive data commands are available only in first-party settings; TXT editing only in space',async()=>{
  const names=['settings','space','frozen-widget','pin','recording-controls','scroll'];
  for(const name of names){const value=JSON.parse(await read(`../src-tauri/capabilities/${name}.json`));for(const command of ['allow-get-data-usage','allow-clean-data'])assert.equal(value.permissions.includes(command),name==='settings');assert.equal(value.permissions.includes('allow-save-blackboard-text'),name==='space');}
});
