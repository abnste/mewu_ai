import assert from 'node:assert/strict';import test from 'node:test';import {readFile}from'node:fs/promises';import {stripTypeScriptTypes}from'node:module';import {SourceTextModule,SyntheticModule,runInNewContext}from'node:vm';import{parse}from'@babel/parser';
const reactive=await import(new URL('../../../node_modules/solid-js/dist/solid.js',import.meta.url));
const solid=new SyntheticModule(Object.keys(reactive),function(){for(const[k,v]of Object.entries(reactive))this.setExport(k,v);});
const module=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./blackboard-text.ts',import.meta.url),'utf8'),{mode:'transform'}));await module.link(()=>solid);await module.evaluate();const {createBlackboardText,resizeBoardObject}=module.namespace;
function mount(read,write){let data,dispose;reactive.createRoot(stop=>{dispose=stop;data=createBlackboardText(read,write,()=>{});});return{data,dispose};}
test('TXT writes preserve new typing during an in-flight save and serialize aggregate/blur flushes',async()=>{
  let finish,active=0,maxActive=0;const writes=[];const m=mount(async()=>'',async(id,text)=>{active++;maxActive=Math.max(maxActive,active);writes.push({id,text});if(writes.length===1)await new Promise(resolve=>finish=resolve);active--;return`copy${writes.length}`;});
  await m.data.load('original');m.data.edit('first');const a=m.data.flush();await new Promise(resolve=>setImmediate(resolve));m.data.edit('second');const b=m.data.flush();finish();await Promise.all([a,b]);
  assert.deepEqual(writes,[{id:'original',text:'first'},{id:'copy1',text:'second'}]);assert.equal(maxActive,1);assert.equal(m.data.text(),'second');m.dispose();
});
test('failed text save keeps draft and original identity for explicit retry; invalid text never enters the writer',async()=>{
  let fail=true;const writes=[];const m=mount(async()=> 'source',async(id,text)=>{writes.push({id,text});if(fail)throw Error('disk full');return'copy';});await m.data.load('original');m.data.edit('draft');await assert.rejects(m.data.flush(),/disk full/);assert.equal(m.data.text(),'draft');fail=false;await m.data.flush();assert.equal(writes[1].id,'original');m.data.edit('\0');assert.equal(m.data.text(),'draft');m.dispose();
});
test('a synchronous bridge publication cannot reopen an unsaved draft during its own save',async()=>{
  let reads=0,m;const writes=[];m=mount(async()=>{reads++;return 'source';},async(id,text)=>{writes.push({id,text});await m.data.load('copy');return 'copy';});
  await m.data.load('original');m.data.edit('edited');await m.data.flush();await m.data.load('copy');assert.equal(reads,1);assert.deepEqual(writes,[{id:'original',text:'edited'}]);assert.equal(m.data.text(),'edited');m.dispose();
});
test('source changes cannot discard unsaved text and inactive ownership prevents a write',async()=>{
  let count=0;const m=mount(async()=> 'original',async()=>{count++;return'copy';});await m.data.load('source');m.data.edit('draft');await assert.rejects(m.data.load('other'),/仍保留/);await assert.rejects(m.data.flush(()=>false),/已切换/);assert.equal(count,0);assert.equal(m.data.text(),'draft');m.dispose();
});
test('the production quit hook saves the latest TXT draft before native preparation revokes authoring',async()=>{
  const source=await readFile(new URL('./App.tsx',import.meta.url),'utf8'),ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});let hook;
  const walk=n=>{if(!n||typeof n!=='object')return;if(n.type==='NewExpression'&&n.callee?.name==='ExitPreparation')hook=n.arguments[0].properties.find(p=>p.key?.name==='beforePrepare')?.value;for(const v of Object.values(n))if(Array.isArray(v))v.forEach(walk);else if(v&&typeof v==='object')walk(v);};walk(ast);assert.ok(hook);
  const exitModule=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./exit-preparation.ts',import.meta.url),'utf8'),{mode:'transform'}));await exitModule.link(()=>{throw Error('unexpected import');});await exitModule.evaluate();
  let phase='running',release,writes=0,receipt;const m=mount(async()=>'',async()=>{assert.equal(phase,'running');writes++;await new Promise(resolve=>release=resolve);return 'saved';});await m.data.load('original');m.data.edit('最后一段输入');
  const before=runInNewContext(`(${source.slice(hook.start,hook.end)})`,{flushVideoDrawing:async()=>{assert.equal(phase,'running');},flushDrawing:active=>m.data.flush(active)});
  const queue=new exitModule.namespace.ExitPreparation({lock:()=>{},beforePrepare:before,beginPreparation:async()=>{assert.equal(writes,1);phase='preparing';},flush:active=>m.data.flush(active),finish:r=>{receipt=r;},error:e=>assert.fail(e)});
  const pending=queue.prepare({requestId:'txt-exit'});await new Promise(resolve=>setImmediate(resolve));assert.equal(phase,'running');assert.equal(writes,1);release();await pending;assert.equal(receipt.success,true);assert.equal(m.data.text(),'最后一段输入');assert.equal(writes,1);m.dispose();
});
test('each corner keeps its opposite anchor while resizing and clamps bounds inside the board',()=>{
  const start={x:.2,y:.3,width:.4,height:.4};
  for(const corner of ['nw','ne','sw','se']){const next=resizeBoardObject(start,.05,.05,corner,1000,800);if(corner.endsWith('w'))assert.ok(Math.abs(next.x+next.width-.6)<1e-12);else assert.equal(next.x,.2);if(corner.startsWith('n'))assert.ok(Math.abs(next.y+next.height-.7)<1e-12);else assert.equal(next.y,.3);for(const delta of [-5,5]){const bounded=resizeBoardObject(start,delta,delta,corner,1000,800);assert.ok(bounded.x>=0&&bounded.y>=0&&bounded.x+bounded.width<=1.000001&&bounded.y+bounded.height<=1.000001);assert.ok(bounded.width>=.096-1e-12&&bounded.height>=.08-1e-12);}}
});
