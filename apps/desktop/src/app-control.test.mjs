// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';

// Execute the real adapter; only the surrounding native/UI actions are fakes.
const source = await readFile(new URL('./app-control.ts', import.meta.url), 'utf8');
const api = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source, { mode: 'transform' })).toString('base64')}`);
const turn = () => new Promise(resolve => setImmediate(resolve));
function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
function request(method = 'capture.open', params = {}, extra = {}) {
  return { id: 'request', nonce: 'private-nonce', sceneId: 'scene', revision: 8, method, params, ...extra };
}
function fixture(options = {}) {
  const calls = [], completed = [];
  let current = { sceneId: 'scene', revision: 8, backgroundId: 'background', draft: 'draft', refs: [{ kind: 'region', id: 'region' }], runId: 'run' };
  const actions = {
    current: () => current,
    flush: async () => { calls.push('flush'); },
    execute: async (method, params) => { calls.push([method, params]); return { accepted: true }; },
    complete: async value => { completed.push(value); },
  };
  const control = api.createAppControl(actions, options);
  return { control, actions, calls, completed, get current() { return current; }, set current(value) { current = value; } };
}
function notSent(value, code) { assert.equal(value.error.code, code); assert.equal(value.error.executed, false); assert.equal('result' in value, false); }

test('the finite catalog dispatches every supported method through the real action port', async () => {
  const drawing = { type: 'undo_drawing', sceneId: 'scene', regionId: 'region', backgroundId: 'background', expectedRevision: 3 };
  const selection = { pluginId: 'plugin', revision: 1, contributionId: 'selection', target: { sceneId: 'scene', regionId: 'region', backgroundId: 'background', drawingRevision: 3, x: 1, y: 2, width: 3, height: 4 } };
  const examples = {
    'capture.open': {}, 'capture.quick': {},
    'region.add': { backgroundId: 'background', region: { id: 'new-region', x: 1, y: 2, width: 3, height: 4 } },
    'chat.setDraft': { text: 'hello', expectedDraft: 'draft' },
    'chat.send': { text: 'hello', refs: [{ kind: 'region', id: 'region' }], expectedDraft: 'draft', expectedRefs: [{ kind: 'region', id: 'region' }], regionId: 'region' },
    'chat.cancel': { expectedRunId: 'run' },
    'blackboard.create': {}, 'blackboard.finish': {}, 'blackboard.open': { itemId: 'board' },
    'drawing.apply': { command: drawing }, 'recording.start': { regionId: 'region', backgroundId: 'background' },
    'recording.control': { id: 'recording', action: 'pause' },
    'scroll.start': { pluginId: 'scroll', revision: 1, contributionId: 'long', target: { sceneId: 'scene', regionId: 'region', backgroundId: 'background', drawingRevision: 3, x: 1, y: 2, width: 3, height: 4 }, snapshotNodeId: 2 },
    'scroll.control': { id: 'scrolling', action: 'stop' },
    'scene.activate': { sceneId: 'other' }, 'scene.new': {}, 'scene.newConversation': {}, 'scene.freeze': {}, 'scene.close': {},
    'scene.command': { command: { type: 'set_agent', sceneId: 'scene', agentId: 'agent' } },
    'references.set': { refs: [], expectedRefs: [{ kind: 'region', id: 'region' }] }, 'settings.open': {},
    'asset.import': { paths: ['D:/input.png'] }, 'image.export': { regionId: 'region', variant: 'original', path: 'D:/output.png' },
    'ocr.run': selection, 'translation.run': { ...selection, language: 'en' }, 'pin.create': { ...selection, translationId: null },
  };
  assert.deepEqual([...api.appControlMethods].sort(), Object.keys(examples).sort());
  for (const [method, params] of Object.entries(examples)) {
    const f = fixture(); await f.control.handle(request(method, params));
    assert.deepEqual(f.calls.at(-1), [method, params]);
    assert.deepEqual(f.completed, [{ id: 'request', nonce: 'private-nonce', result: { accepted: true } }]);
    assert.equal(f.calls.filter(value => value === 'flush').length, ['chat.cancel', 'scroll.control', 'recording.control'].includes(method) ? 0 : 1);
  }
});

test('invalid or arbitrary invoke methods and unknown fields never reach UI actions', async () => {
  const cases = [
    [request('tauri.invoke', { command: 'save_connection_profile' }), 'UNSUPPORTED_METHOD'],
    [request('capture.open', { shell: 'anything' }), 'INVALID_REQUEST'],
    [request('capture.open', {}, { permission: 'all' }), 'INVALID_REQUEST'],
    [request('chat.send', { text: 42 }), 'INVALID_REQUEST'],
    [request('chat.send', { refs: [{ kind: 'item', id: 'a', path: 'secret' }] }), 'INVALID_REQUEST'],
    [request('chat.send', { refs: [{ kind: 'item', id: 'a' }, { kind: 'item', id: 'a' }] }), 'INVALID_REQUEST'],
    [request('recording.control', { id: 'recording', action: 'restart' }), 'INVALID_REQUEST'],
    [request('scene.command', { command: { type: 'set_mcp_enabled', sceneId: 'scene', serverId: 'mcp', enabled: true, expectedRevision: 1 } }), 'INVALID_REQUEST'],
    [request('scene.command', { command: { type: 'set_agent', sceneId: 'scene', agentId: 'agent', apiKey: 'not-an-option' } }), 'INVALID_REQUEST'],
    [request('region.add', { backgroundId: 'background', region: { id: 'r', x: 0, y: 0, width: 2, height: 2, drawings: [] } }), 'INVALID_REQUEST'],
    [request('asset.import', { paths: [] }), 'INVALID_REQUEST'],
    [request('asset.import', { paths: ['D:/a', 'D:/a'] }), 'INVALID_REQUEST'],
    [request('image.export', { regionId: 'r', variant: 'all', path: 'D:/output.png' }), 'INVALID_REQUEST'],
  ];
  for (const [value, code] of cases) {
    const f = fixture(); await f.control.handle(value); assert.deepEqual(f.calls, []); notSent(f.completed[0], code);
  }
});

test('request budget is UTF-8 bytes, bounded depth, finite numbers and inert JSON only', async () => {
  const overUtf8 = request('chat.setDraft', { text: '中'.repeat(400_000) });
  assert.throws(() => api.parseAppControlRequest(overUtf8), error => error.code === 'INVALID_REQUEST');
  assert.throws(() => api.parseAppControlRequest(request('capture.open', {}, { revision: NaN })), error => error.code === 'INVALID_REQUEST');
  let deep = {}; for (let i = 0; i < 35; i++) deep = { nested: deep };
  assert.throws(() => api.parseAppControlRequest(request('capture.open', deep)), error => error.code === 'INVALID_REQUEST');
  let reads = 0;
  const getter = request(); Object.defineProperty(getter, 'method', { enumerable: true, get() { reads++; throw Error('do not evaluate'); } });
  const f = fixture(); await f.control.handle(getter); assert.equal(reads, 0); notSent(f.completed[0], 'INVALID_REQUEST');
  const idGetter = { get id() { reads++; throw Error('do not recover'); }, nonce: 'n' };
  await f.control.handle(idGetter); assert.equal(reads, 0); assert.equal(f.completed.length, 1);
  const poisoned = JSON.parse('{"id":"request","nonce":"n","method":"capture.open","params":{"__proto__":{}}}');
  assert.throws(() => api.parseAppControlRequest(poisoned), error => error.code === 'INVALID_REQUEST');
  const array = []; Object.defineProperty(array, 'toJSON', { value() { reads++; throw Error('do not serialize'); } });
  assert.throws(() => api.parseAppControlRequest(request('chat.send', { refs: array })), error => error.code === 'INVALID_REQUEST');
  assert.equal(reads, 0);
  const arrayGetter = []; Object.defineProperty(arrayGetter, '0', { enumerable: true, get() { reads++; throw Error('do not read'); } });
  assert.throws(() => api.parseAppControlRequest(request('chat.send', { refs: arrayGetter })), error => error.code === 'INVALID_REQUEST'); assert.equal(reads, 0);
});

test('preflush revision is strict while valid flush publications can advance it', async () => {
  const stale = fixture(); await stale.control.handle(request('capture.open', {}, { revision: 7 }));
  notSent(stale.completed[0], 'CONFLICT'); assert.deepEqual(stale.calls, []);
  const f = fixture();
  f.actions.flush = async () => { f.calls.push('flush'); f.current = { ...f.current, revision: 10 }; };
  await f.control.handle(request()); assert.deepEqual(f.calls, ['flush', ['capture.open', {}]]); assert.ok(f.completed[0].result);
});

test('a scene change, target background change or draft race during flush prevents dispatch', async () => {
  for (const [method, params, mutate] of [
    ['capture.open', {}, current => ({ ...current, sceneId: 'other' })],
    ['recording.start', { regionId: 'region', backgroundId: 'background' }, current => ({ ...current, backgroundId: 'new-background' })],
    ['chat.send', { text: 'replace', expectedDraft: 'draft' }, current => ({ ...current, draft: 'user typed meanwhile' })],
    ['references.set', { refs: [], expectedRefs: [{ kind: 'region', id: 'region' }] }, current => ({ ...current, refs: [] })],
  ]) {
    const f = fixture(); f.actions.flush = async () => { f.calls.push('flush'); f.current = mutate(f.current); };
    await f.control.handle(request(method, params)); notSent(f.completed[0], 'CONFLICT'); assert.deepEqual(f.calls, ['flush']);
  }
});

test('drawing and scroll targets cannot select a different scene or image through params', async () => {
  for (const command of [
    { type: 'undo_drawing', sceneId: 'other', regionId: 'region', backgroundId: 'background', expectedRevision: 3 },
    { type: 'undo_drawing', sceneId: 'scene', regionId: 'region', backgroundId: 'stale', expectedRevision: 3 },
  ]) {
    const f = fixture(); await f.control.handle(request('drawing.apply', { command }));
    notSent(f.completed[0], 'CONFLICT'); assert.deepEqual(f.calls, []);
  }
});

test('exclusive requests serialize through flush and execute and use receive-time deadline', async () => {
  let now = 0; const gate = deferred(), f = fixture({ now: () => now, requestTimeoutMs: 10 });
  f.actions.execute = async method => { f.calls.push(method); if (f.calls.filter(value => value === method).length === 1) await gate.promise; return method; };
  const first = f.control.handle(request()); await turn();
  const second = f.control.handle(request('scene.new', {}, { id: 'second' })); await turn();
  assert.deepEqual(f.calls, ['flush', 'capture.open']); now = 11; gate.resolve();
  await Promise.all([first, second]);
  assert.deepEqual(f.calls, ['flush', 'capture.open']); assert.equal(f.completed[0].result, 'capture.open'); notSent(f.completed[1], 'REQUEST_EXPIRED');
});

test('expired or disposed after flush is NotSent; disposal after execution does not falsify it', async () => {
  let now = 0; const expired = fixture({ now: () => now, requestTimeoutMs: 10 });
  expired.actions.flush = async () => { now = 10; };
  await expired.control.handle(request()); notSent(expired.completed[0], 'REQUEST_EXPIRED');
  const stopped = fixture(); stopped.actions.flush = async () => { stopped.control.dispose(); };
  await stopped.control.handle(request()); notSent(stopped.completed[0], 'NOT_READY');
  const accepted = fixture(); accepted.actions.execute = async () => { accepted.control.dispose(); return 'committed'; };
  await accepted.control.handle(request()); assert.equal(accepted.completed[0].result, 'committed');
});

test('stop cancel requests bypass a blocked lane while pause and resume remain serialized without authoring flush', async () => {
  const gate = deferred(), f = fixture(); f.actions.flush = async () => { f.calls.push('flush'); await gate.promise; };
  const blocked = f.control.handle(request()); await turn();
  for (const [method, params] of [
    ['chat.cancel', { expectedRunId: 'run' }], ['recording.control', { id: 'recording', action: 'stop' }],
    ['recording.control', { id: 'recording', action: 'cancel' }], ['scroll.control', { id: 'scroll', action: 'cancel' }],
  ]) await f.control.handle(request(method, params, { id: `${method}-${params.action ?? ''}` }));
  assert.equal(f.completed.length, 4); assert.equal(f.calls.filter(value => value === 'flush').length, 1);
  const pause = f.control.handle(request('recording.control', { id: 'recording', action: 'pause' }, { id: 'pause' }));
  await turn(); assert.equal(f.completed.length, 4); gate.resolve(); await Promise.all([blocked, pause]);
  assert.equal(f.calls.filter(value => value === 'flush').length, 1);
});

test('native claim occurs after successful flush and target check and before any business execution', async () => {
  const f = fixture(); f.actions.claim = async value => { f.calls.push(['claim', value.id, value.nonce]); };
  await f.control.handle(request());
  assert.deepEqual(f.calls, ['flush', ['claim', 'request', 'private-nonce'], ['capture.open', {}]]);
  const rejected = fixture(); rejected.actions.claim = async () => { throw new api.AppControlError('NOT_READY', '控制请求已撤销'); };
  await rejected.control.handle(request()); assert.deepEqual(rejected.calls, ['flush']); notSent(rejected.completed[0], 'NOT_READY');
  const changed = fixture(); changed.actions.claim = async () => { changed.current = { ...changed.current, sceneId: 'successor' }; };
  await changed.control.handle(request()); assert.deepEqual(changed.calls, ['flush']); notSent(changed.completed[0], 'CONFLICT');
});

test('cancellation has exact expected run identity and never stops a successor', async () => {
  const f = fixture(); await f.control.handle(request('chat.cancel', { expectedRunId: 'old-run' }));
  notSent(f.completed[0], 'CONFLICT'); assert.deepEqual(f.calls, []);
});

test('revision guard permits own flush but rejects edits made during native claim', async () => {
  const f = fixture();
  const revision = f.current.revision;
  f.actions.flush = async () => { f.current = { ...f.current, revision: revision + 1 }; };
  f.actions.claim = async () => { f.current = { ...f.current, revision: revision + 2 }; };
  await f.control.handle(request('capture.open', {}, { revision }));
  notSent(f.completed[0], 'CONFLICT');
  assert.deepEqual(f.calls, []);
});

test('queue limit is bounded without blocking a stop request', async () => {
  const gate = deferred(), f = fixture({ maxQueued: 1 }); f.actions.flush = async () => gate.promise;
  const first = f.control.handle(request()); await turn();
  await f.control.handle(request('scene.new', {}, { id: 'overflow' })); notSent(f.completed[0], 'BUSY');
  await f.control.handle(request('recording.control', { id: 'rec', action: 'stop' }, { id: 'urgent' }));
  assert.equal(f.completed[1].result.accepted, true); gate.resolve(); await first;
});

test('duplicate pending and completed events never execute twice; event mutation cannot change intent', async () => {
  const gate = deferred(), f = fixture(); f.actions.flush = async () => gate.promise;
  const value = request('chat.setDraft', { text: 'original' });
  const first = f.control.handle(value), duplicate = f.control.handle(value);
  value.params.text = 'mutated'; gate.resolve(); await Promise.all([first, duplicate]);
  assert.deepEqual(f.calls, [['chat.setDraft', { text: 'original' }]]); assert.equal(f.completed.length, 1);
  await f.control.handle(request('chat.setDraft', { text: 'original' })); assert.equal(f.calls.length, 1);
  await f.control.handle(request('chat.setDraft', { text: 'different' }, { nonce: 'different' })); notSent(f.completed[1], 'CONFLICT');
});

test('native failure is reported without claiming NotSent or leaking arbitrary exception text', async () => {
  const f = fixture(); f.actions.execute = async () => { throw Error('private key or user text must not be forwarded'); };
  await f.control.handle(request());
  assert.deepEqual(f.completed[0].error, { code: 'OPERATION_FAILED', message: '操作未完成' });
  const typed = fixture(); typed.actions.execute = async () => { throw new api.AppControlError('CONFLICT', '操作目标已变化'); };
  await typed.control.handle(request()); assert.deepEqual(typed.completed[0].error, { code: 'CONFLICT', message: '操作目标已变化' });
});

test('completion transport failure is not retried and does not poison later requests', async () => {
  const f = fixture(); let completions = 0;
  f.actions.complete = async value => { f.completed.push(value); if (++completions === 1) throw Error('transport gone'); };
  await assert.rejects(f.control.handle(request()), /transport gone/);
  await f.control.handle(request('scene.new', {}, { id: 'next' }));
  assert.equal(f.calls.filter(Array.isArray).length, 2); assert.equal(completions, 2);
});

test('attachment listens only on the first-party space and disposes queued callbacks', async () => {
  const f = fixture(); let listener, unlistens = 0;
  const detach = await api.attachAppControl(f.actions, async (name, handler, options) => {
    assert.equal(name, 'mewu-control-request'); assert.deepEqual(options, { target: 'space' }); listener = handler;
    return () => { unlistens++; };
  });
  listener({ payload: request() }); await turn(); assert.equal(f.completed[0].result.accepted, true);
  detach(); detach(); assert.equal(unlistens, 1);
  listener({ payload: request('scene.new', {}, { id: 'late' }) }); await turn(); notSent(f.completed[1], 'NOT_READY');
  await assert.rejects(api.attachAppControl(f.actions, async () => { throw Error('listen failed'); }), /listen failed/);
});
