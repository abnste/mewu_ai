// Real publication barrier, deterministic clocks, no native IPC or application.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule } from 'node:vm';
import { parse } from '@babel/parser';

const source = stripTypeScriptTypes(await readFile(new URL('./published-revision.ts', import.meta.url), 'utf8'), { mode: 'transform' });
const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const declarations = [];
function walk(value) {
  if (!value || typeof value !== 'object') return;
  if (value.type === 'VariableDeclarator' && value.id?.name === 'publishedRevision') declarations.push(value);
  for (const child of Object.values(value)) {
    if (Array.isArray(child)) child.forEach(walk);
    else if (child && typeof child === 'object' && child.type) walk(child);
  }
}
walk(parse(app, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
assert.equal(declarations.length, 1, 'exercise the unique App publication barrier wiring');
const constructor = app.slice(declarations[0].init.start, declarations[0].init.end);
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
async function fixture() {
  const timers = new Map(); let next = 0;
  const context = createContext({ setTimeout: callback => { const id = ++next; timers.set(id, callback); return id; }, clearTimeout: id => timers.delete(id) });
  const module = new SourceTextModule(source, { context });
  await module.link(() => { throw Error('Unexpected runtime import'); }); await module.evaluate();
  const wiring = new SourceTextModule(stripTypeScriptTypes(`export default function(PublishedRevision,snapshot,accept,bridge){return ${constructor};}`, { mode: 'transform' }), { context });
  await wiring.link(() => { throw Error('Unexpected App wiring import'); }); await wiring.evaluate();
  const state = { revision: 1, refreshes: 0, accepted: [], refresh: async () => {} };
  const bridge = { async getSnapshot() { state.refreshes++; await state.refresh(); return { revision: state.revision }; } };
  const barrier = wiring.namespace.default(module.namespace.PublishedRevision, () => ({ revision: state.revision }), next => {
    state.accepted.push(next); state.revision = next.revision;
  }, bridge);
  const takeTimer = () => { const [id, callback] = timers.entries().next().value; timers.delete(id); return callback; };
  return { state, barrier, timers, takeTimer, fire: () => takeTimer()() };
}

test('an event received before the receipt resolves immediately without a timer or full query', async () => {
  const f = await fixture(); f.state.revision = 7; f.barrier.changed();
  await f.barrier.wait(7); await f.barrier.wait(6);
  assert.equal(f.timers.size, 0); assert.equal(f.state.refreshes, 0); f.barrier.dispose();
});

test('an event after the receipt wakes only reached revisions and clears their timers', async () => {
  const f = await fixture(); let first = false, second = false;
  const p = f.barrier.wait(2).then(() => { first = true; });
  const q = f.barrier.wait(3).then(() => { second = true; });
  assert.equal(f.timers.size, 2); await tick(); assert.equal(first, false);
  f.state.revision = 2; f.barrier.changed(); await p;
  assert.equal(first, true); assert.equal(second, false); assert.equal(f.timers.size, 1);
  f.state.revision = 4; f.barrier.changed(); await q;
  assert.equal(second, true); assert.equal(f.timers.size, 0); assert.equal(f.state.refreshes, 0); f.barrier.dispose();
});

test('a missed event recovers through one ordinary snapshot query and validates its real revision', async () => {
  const f = await fixture(); f.state.refresh = async () => { f.state.revision = 9; };
  const result = f.barrier.wait(8); await f.fire(); await result;
  assert.equal(f.state.refreshes, 1); assert.deepEqual(f.state.accepted, [{ revision: 9 }]);
  assert.equal(f.timers.size, 0); f.barrier.dispose();
});

test('a full query that still lacks the saved revision fails without guessing a commit', async () => {
  const f = await fixture(); f.state.refresh = async () => { f.state.revision = 2; };
  const rejected = assert.rejects(f.barrier.wait(3), /未收到保存结果，请重试/);
  await f.fire(); await rejected;
  assert.equal(f.state.refreshes, 1); assert.equal(f.timers.size, 0); f.barrier.dispose();
});

test('snapshot query failure is propagated and removes the pending listener', async () => {
  const f = await fixture(), failure = Error('snapshot unavailable'); f.state.refresh = async () => { throw failure; };
  const rejected = assert.rejects(f.barrier.wait(3), error => error === failure);
  await f.fire(); await rejected; assert.equal(f.timers.size, 0);
  f.state.revision = 3; f.barrier.changed(); assert.equal(f.state.refreshes, 1); f.barrier.dispose();
});

test('dispose rejects current and future waiters without a recovery request', async () => {
  const f = await fixture(); const rejected = assert.rejects(f.barrier.wait(3), /窗口已关闭/);
  f.barrier.dispose(); await rejected; assert.equal(f.timers.size, 0);
  await assert.rejects(f.barrier.wait(3), /窗口已关闭/); assert.equal(f.state.refreshes, 0);
});

test('dispose while recovery is in flight rejects promptly and late success cannot revive the waiter', async () => {
  const f = await fixture(), refresh = deferred();
  f.state.refresh = async () => { await refresh.promise; f.state.revision = 3; };
  const rejected = assert.rejects(f.barrier.wait(3), /窗口已关闭/);
  const timeout = f.fire(); assert.equal(f.state.refreshes, 1);
  f.barrier.dispose(); await rejected; refresh.resolve(); await timeout;
  assert.equal(f.timers.size, 0); await assert.rejects(f.barrier.wait(3), /窗口已关闭/);
});

test('a queued timeout callback after disposal cannot start a full query', async () => {
  const f = await fixture(); const rejected = assert.rejects(f.barrier.wait(3), /窗口已关闭/);
  const callback = f.takeTimer(); f.barrier.dispose(); await rejected; await callback();
  assert.equal(f.state.refreshes, 0);
});

test('malformed revisions fail before querying or reserving a timer', async () => {
  const f = await fixture();
  for (const revision of [undefined, NaN, Infinity, -1, 0, 1.1, Number.MAX_SAFE_INTEGER + 1]) {
    await assert.rejects(f.barrier.wait(revision), /保存回执无效/);
  }
  assert.equal(f.timers.size, 0); assert.equal(f.state.refreshes, 0); f.barrier.dispose();
});

test('a later host revision without this operation witness cannot acknowledge the saved target', async () => {
  const f = await fixture(); let present = false, resolved = false;
  f.state.revision = 20;
  const waiting = f.barrier.wait(8, () => present).then(() => { resolved = true; });
  f.barrier.changed(); await tick(); assert.equal(resolved, false);
  present = true; f.barrier.changed(); await waiting;
  assert.equal(f.state.refreshes, 0); assert.equal(f.timers.size, 0); f.barrier.dispose();
});

test('a missing target witness uses the full query even when its host revision is already newer', async () => {
  const f = await fixture(); let present = false;
  f.state.revision = 20;
  f.state.refresh = async () => { present = true; f.state.revision = 20; };
  const waiting = f.barrier.wait(8, () => present);
  await f.fire(); await waiting;
  assert.equal(f.state.refreshes, 1); assert.equal(f.timers.size, 0); f.barrier.dispose();
});
