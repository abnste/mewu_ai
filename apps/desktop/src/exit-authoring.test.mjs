import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule} from 'node:vm';
const module=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./exit-preparation.ts',import.meta.url),'utf8'),{mode:'transform'}));
await module.link(()=>{throw Error('Unexpected runtime dependency');});await module.evaluate();
const {ExitPreparation}=module.namespace;
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
const plain=x=>JSON.parse(JSON.stringify(x));

test('quit locks new input while completed authoring drains in Running, then enters preparation and flushes',async()=>{
 const events=[],wait=deferred();let locked=false,phase='running',draft='completed drawing';
 const queue=new ExitPreparation({lock:value=>{locked=value;events.push(['locked',value]);},beforePrepare:async()=>{assert.equal(phase,'running');events.push(['authoring']);await wait.promise;draft='';},beginPreparation:async request=>{assert.equal(draft,'');events.push(['begin',request.requestId]);phase='preparing';},flush:async()=>{assert.equal(phase,'preparing');events.push(['flush']);},finish:async value=>events.push(['finish',plain(value)]),error:()=>assert.fail('Unexpected failure')});
 const pending=queue.prepare({requestId:'same-native-nonce'});
 assert.equal(locked,true);assert.equal(phase,'running');assert.equal(draft,'completed drawing');
 let lateInputs=0;if(!locked)lateInputs++;assert.equal(lateInputs,0);
 await queue.prepare({requestId:'same-native-nonce'});assert.equal(events.filter(x=>x[0]==='authoring').length,1);
 wait.resolve();await pending;
 assert.deepEqual(events.map(x=>x[0]),['locked','authoring','begin','flush','finish']);assert.equal(locked,true);assert.equal(events.at(-1)[1].success,true);
});
test('failed authoring retains the draft, never advances or flushes, and restores input',async()=>{
 const events=[],draft={text:'保留原文'};
 const queue=new ExitPreparation({lock:v=>events.push(['lock',v]),beforePrepare:async()=>{throw Error('authoring CAS rejected');},beginPreparation:async()=>assert.fail('Must remain Running'),flush:async()=>assert.fail('No later flush'),finish:async v=>events.push(['finish',plain(v)]),error:v=>events.push(['error',v])});
 await queue.prepare({requestId:'failed-drawing'});assert.deepEqual(draft,{text:'保留原文'});assert.deepEqual(events.filter(x=>x[0]==='lock'),[['lock',true],['lock',false]]);assert.equal(events.find(x=>x[0]==='finish')[1].success,false);
});
test('native cancellation during authoring restores input and cannot invoke late preparation or success',async()=>{
 const wait=deferred(),events=[];
 const queue=new ExitPreparation({lock:v=>events.push(['lock',v]),beforePrepare:async()=>wait.promise,beginPreparation:async()=>assert.fail('Canceled nonce cannot activate'),flush:async()=>assert.fail('Canceled nonce cannot flush'),finish:async v=>events.push(['finish',plain(v)]),error:()=>{}});
 const pending=queue.prepare({requestId:'timed-out'});queue.cancel({requestId:'timed-out'});wait.resolve();await pending;assert.deepEqual(events,[['lock',true],['lock',false]]);
});
test('native begin failure reports false, preserves editing, and does not flush',async()=>{
 const events=[];
 const queue=new ExitPreparation({lock:v=>events.push(['lock',v]),beforePrepare:async()=>{},beginPreparation:async()=>{throw Error('obsolete native nonce');},flush:async()=>assert.fail('No preparation'),finish:async v=>events.push(['finish',plain(v)]),error:()=>{}});
 await queue.prepare({requestId:'obsolete'});assert.deepEqual(events.filter(x=>x[0]==='lock'),[['lock',true],['lock',false]]);assert.equal(events.find(x=>x[0]==='finish')[1].success,false);
});
