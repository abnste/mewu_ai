// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createContext, definePlugin, hostApi } from '../index.mjs';

function harness(dispatch) {
  let sequence = 0;
  const calls = [];
  const scope = createContext((method, input) => {
    const id = String(++sequence);
    const params = JSON.parse(input);
    calls.push({ method, params });
    queueMicrotask(async () => {
      try { scope.settle(id, JSON.stringify(await dispatch(method, params)), false); }
      catch { scope.settle(id, 'null', true); }
    });
    return id;
  });
  return { scope, calls };
}
async function invoke(scope, id, input, context = {}) {
  scope.invoke(id, JSON.stringify(input), JSON.stringify(context));
  for (let i = 0; i < 200; i++) {
    const outcome = scope.takeOutcome();
    if (outcome !== null) return JSON.parse(outcome);
    await new Promise(resolve => setImmediate(resolve));
  }
  throw new Error('Isolated extension test did not settle');
}

test('published SDK is the exact embedded runtime, with no repository-relative import', async () => {
  const published = await readFile(new URL('../index.mjs', import.meta.url));
  const embedded = await readFile(new URL('../../../crates/mewu-extension/src/sdk.js', import.meta.url));
  assert.deepEqual(published, embedded);
  assert.equal(hostApi, 2);
});

test('registration closes, invocation receives host-attested context and disposal runs in reverse', async () => {
  const { scope } = harness(async () => null);
  const disposed = [];
  definePlugin(ctx => {
    ctx.commands.register({ id: 'third-party.command', title: 'Command' }, (input, { context }) => ({ input, agentId: context.agentId }));
    ctx.onDispose(() => { disposed.push('first'); });
    ctx.onDispose(async () => { disposed.push('second'); });
  })(scope.ctx);
  assert.deepEqual(JSON.parse(scope.finishRegistration()).map(r => r.kind), ['command']);
  assert.throws(() => scope.ctx.tools.register({ id: 'late', title: 'Late' }, () => null), /closed/);
  assert.deepEqual(await invoke(scope, 'third-party.command', { x: 1 }, { agentId: 'host-agent' }),
    { ok: true, result: { input: { x: 1 }, agentId: 'host-agent' } });
  await scope.deactivate();
  assert.deepEqual(disposed, ['second', 'first']);
  await assert.rejects(scope.ctx.host.call('model.invoke'), /closed/);
});

test('independent region agent consumes screenshots after each action; mock host only, no desktop or model request', async () => {
  const { default: activate } = await import('../../../extensions/examples/region-agent/index.mjs');
  let observations = 0;
  let decisions = 0;
  const { scope, calls } = harness(async (method, params) => {
    if (method === 'region.observe') {
      assert.equal(params.regionId, 'authorized-region');
      observations++;
      return { artifactId: `test-observation-${observations}`, width: 640, height: 480, sha256: String(observations).repeat(64) };
    }
    if (method === 'model.invoke') {
      assert.deepEqual(params.images, [`test-observation-${observations}`]);
      decisions++;
      return { content: JSON.stringify(decisions === 1 ? { action: { kind: 'click', x: 91, y: 42 } } : { done: true, message: '完成' }), toolCalls: [] };
    }
    if (method === 'region.action') {
      assert.deepEqual(params.action, { kind: 'click', x: 91, y: 42 });
      return { ok: true };
    }
    throw new Error('Unexpected host method');
  });
  activate(scope.ctx);
  assert.deepEqual(JSON.parse(scope.finishRegistration()).map(r => r.kind), ['command', 'agent']);
  const result = await invoke(scope, 'example.region-agent.operate', { task: '点击需要的项目' }, { regionId: 'authorized-region' });
  assert.equal(result.ok, true);
  assert.equal(result.result.completed, true);
  assert.equal(result.result.steps, 1);
  assert.deepEqual(calls.map(c => c.method), ['region.observe', 'model.invoke', 'region.action', 'region.observe', 'model.invoke']);
  await scope.deactivate();
});

test('independent memory retains more than 64 records, pages without omissions and rejects another agent ID', async () => {
  const { default: activate } = await import('../../../extensions/examples/journal-memory/index.mjs');
  const records = new Map();
  const { scope } = harness(async (method, params) => {
    if (method === 'storage.get') return records.get(params.key) ?? null;
    if (method === 'storage.set') { records.set(params.key, params.value); return null; }
    if (method === 'storage.compareAndSet') {
      const matches = params.expected === null ? !records.has(params.key) : JSON.stringify(records.get(params.key)) === JSON.stringify(params.expected);
      if (matches) records.set(params.key, params.value);
      return matches;
    }
    if (method === 'storage.delete') { records.delete(params.key); return null; }
    if (method === 'storage.list') {
      const matching = [...records.keys()].filter(k => k > (params.cursor || '') && k.startsWith(params.prefix || '')).sort();
      const keys = matching.slice(0, 100);
      return { keys, nextCursor: matching.length > 100 ? keys.at(-1) : null };
    }
    throw new Error('Unexpected host method');
  });
  activate(scope.ctx); scope.finishRegistration();
  for (let i = 0; i < 125; i++) {
    const result = await invoke(scope, 'example.journal-memory.provider', { operation: 'retain', text: `memory ${i}` }, { agentId: 'a' });
    assert.equal(result.ok, true);
  }
  const ids = new Set();
  let cursor = '';
  for (let i = 0; i < 20; i++) {
    const reply = await invoke(scope, 'example.journal-memory.provider', { operation: 'recall', limit: 20, cursor }, { agentId: 'a' });
    assert.equal(reply.ok, true);
    reply.result.memories.forEach(m => ids.add(m.id));
    cursor = reply.result.nextCursor;
    if (cursor === null) break;
  }
  assert.equal(ids.size, 125);
  const other = await invoke(scope, 'example.journal-memory.provider', { operation: 'recall' }, { agentId: 'b' });
  assert.deepEqual(other.result.memories, []);
  const rejected = await invoke(scope, 'example.journal-memory.provider', { operation: 'forget', id: [...ids][0] }, { agentId: 'b' });
  assert.equal(rejected.ok, false);
  await scope.deactivate();
});

test('independent memory uses host CAS for simultaneous provider runs; mock storage is explicitly atomic', async () => {
  const { default: activate } = await import('../../../extensions/examples/journal-memory/index.mjs');
  const records = new Map();
  const dispatch = async (method, params) => {
    if (method === 'storage.get') return records.get(params.key) ?? null;
    if (method === 'storage.set') { records.set(params.key, params.value); return null; }
    if (method === 'storage.compareAndSet') {
      const matches = params.expected === null ? !records.has(params.key) : records.get(params.key) === params.expected;
      if (matches) records.set(params.key, params.value);
      return matches;
    }
    throw new Error('Unexpected method');
  };
  const scopes = Array.from({ length: 8 }, () => {
    const { scope } = harness(dispatch); activate(scope.ctx); scope.finishRegistration(); return scope;
  });
  const results = await Promise.all(scopes.map((scope, i) => invoke(scope, 'example.journal-memory.provider',
    { operation: 'retain', text: `concurrent record ${i}` }, { agentId: 'shared' })));
  assert.equal(results.every(r => r.ok), true);
  assert.equal(new Set(results.map(r => r.result.id)).size, 8);
  assert.equal([...records.keys()].filter(k => k.includes('.entry.')).length, 8);
  await Promise.all(scopes.map(s => s.deactivate()));
});

test('independent renderer returns authored interactive HTML with transparent document', async () => {
  const { default: activate } = await import('../../../extensions/examples/interactive-widget/index.mjs');
  const { scope } = harness(async () => null);
  activate(scope.ctx);
  const registrations = JSON.parse(scope.finishRegistration());
  assert.deepEqual(registrations.map(r => r.kind), ['renderer', 'ui', 'command']);
  const reply = await invoke(scope, 'example.interactive-widget.ruler', {});
  assert.equal(reply.ok, true);
  assert.equal(reply.result.width, 440);
  assert.match(reply.result.html, /background:transparent/);
  assert.match(reply.result.html, /pointermove/);
  assert.match(reply.result.html, /<svg/);
  await scope.deactivate();
});
