// node --experimental-vm-modules apps/desktop/src/drawing-flush.test.mjs
// Synthetic handoffs: no native application or user drafts.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const load = async path => {
  const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path, import.meta.url), 'utf8'), {mode:'transform'}));
  await module.link(() => { throw Error('Unexpected runtime dependency'); });
  await module.evaluate(); return module.namespace;
};
const {DrawingFlushRegistry} = await load('./drawing-flush.ts');
const {ExitPreparation} = await load('./exit-preparation.ts');
const tick = () => new Promise(resolve => setImmediate(resolve));
const registry = new DrawingFlushRegistry(), events = [];
const oldCleanup = registry.register(async () => { events.push('old'); });
const newCleanup = registry.register(async () => { events.push('new'); });
oldCleanup(); await registry.flush(() => true); assert.deepEqual(events,['new']);
newCleanup(); await registry.flush(() => true); assert.deepEqual(events,['new']);
let release; const wait = new Promise(resolve => {release=resolve;});
registry.register(async active => { await wait; if(active())events.push('saved'); });
const receipts=[], locks=[];
const exit = new ExitPreparation({lock:value=>locks.push(value),flush:active=>registry.flush(active),finish:async value=>receipts.push(value),error:()=>{}});
const pending=exit.prepare({requestId:'pending'}); await tick();
assert.equal(receipts.length,0); release(); await pending;
assert.equal(receipts[0].success,true); assert.ok(events.includes('saved'));
exit.cancel({requestId:'pending'});
registry.register(async()=>{throw Error('冲突，草稿保留');});
await exit.prepare({requestId:'failed'});
assert.equal(receipts.at(-1).success,false); assert.equal(locks.at(-1),false);
let finishLate; const late=new Promise(resolve=>{finishLate=resolve;});
registry.register(async active=>{await late;if(active())events.push('must not save');});
const canceled=exit.prepare({requestId:'cancel'});await tick();exit.cancel({requestId:'cancel'});finishLate();await canceled;
assert.ok(!events.includes('must not save'));assert.equal(receipts.length,2);
let swap;const changing=new Promise(resolve=>{swap=resolve;});
registry.register(async()=>changing);const owner=registry.flush(()=>true);
registry.register(async()=>{});swap();await assert.rejects(owner,/绘制会话已变化/);
console.log(JSON.stringify({passed:5,checks:['stale cleanup preserves next editor','exit waits for actual draft save','save conflict aborts exit and unlocks','cancelled preparation cannot commit late draft','changed owner rejects transition']}));
